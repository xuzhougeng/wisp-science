//! Persistent SSH master connections.
//!
//! Instead of opening a new TCP connection + auth handshake for every remote
//! script (probes, Run polling, file browsing), each execution context keeps
//! one long-lived `ssh <opts> <target> sh` process and multiplexes script
//! RPCs over its stdin/stdout, framed by one-shot nonce markers. Servers see
//! a single authenticated session instead of a login flood, on every platform
//! OpenSSH ships on (unlike ControlMaster, which Win32-OpenSSH lacks).
//!
//! Auth, `~/.ssh/config`, known_hosts, and askpass all still belong to the
//! OpenSSH client; this module only changes how often it connects.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Everything the remote wrote before the exit marker, stderr merged into
/// stdout by the frame (`2>&1`); `stderr` carries local ssh transport noise.
pub struct SshRpcOutput {
    pub exit_code: i64,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

pub enum SshPayload {
    /// Script fed to a fresh remote `sh -s` child, exactly like today's
    /// `ssh host "sh -s"` with the script on stdin.
    Script(String),
    /// Remote command line run via the login shell, exactly like today's
    /// `ssh host <command>`.
    Command(String),
}

/// Classify an ssh invocation built by our own arg builders (options + target
/// + one trailing remote-command arg). Returns None for shapes that need a
/// real per-call session, e.g. stdin used as data for the remote command.
pub fn eligible_payload(program: &str, args: &[String], stdin: Option<&str>) -> Option<SshPayload> {
    if program != "ssh" || args.len() < 2 {
        return None;
    }
    let last = args.last().expect("checked len");
    match stdin {
        Some(payload) if last == "sh -s" => Some(SshPayload::Script(payload.into())),
        None => Some(SshPayload::Command(last.clone())),
        Some(_) => None,
    }
}

/// 32 MiB remote-file cap plus framing headroom.
const MAX_RPC_OUTPUT_BYTES: usize = 40 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;

fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Marker printed by the master shell after each RPC child exits. The leading
/// newline lets us find it even when the child's output has no trailing one.
fn exit_marker(nonce: &str) -> String {
    format!("\n__WISP_RPC_EXIT_{nonce}__:")
}

fn frame(payload: &SshPayload, nonce: &str) -> String {
    let exit_line = format!("printf '\\n__WISP_RPC_EXIT_{nonce}__:%s\\n' \"$?\"\n");
    match payload {
        SshPayload::Script(script) => {
            let mut delimiter = format!("__WISP_MASTER_{nonce}__");
            while script.lines().any(|line| line == delimiter) {
                delimiter.push('X');
            }
            let newline = if script.ends_with('\n') { "" } else { "\n" };
            format!("sh -s <<'{delimiter}' 2>&1\n{script}{newline}{delimiter}\n{exit_line}")
        }
        SshPayload::Command(command) => {
            format!(
                "\"${{SHELL:-sh}}\" -c {} </dev/null 2>&1\n{exit_line}",
                single_quoted(command)
            )
        }
    }
}

/// How long a dying master gets to exit and flush its stderr before we give up
/// on quoting OpenSSH's own diagnostic.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

struct Master {
    // Held for kill_on_drop: dropping a Master tears the connection down.
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr_buf: Arc<StdMutex<Vec<u8>>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl Master {
    fn spawn(program: &str, args: &[String], envs: &[(String, String)]) -> Result<Self, String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if !envs.is_empty() {
            cmd.envs(envs.iter().cloned());
        }
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn ssh master: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open ssh master stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open ssh master stdout".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to open ssh master stderr".to_string())?;
        let stderr_buf = Arc::new(StdMutex::new(Vec::new()));
        let buf = stderr_buf.clone();
        let stderr_task = tokio::spawn(async move {
            let mut chunk = [0_u8; 4096];
            while let Ok(read) = stderr.read(&mut chunk).await {
                if read == 0 {
                    break;
                }
                let mut buf = buf.lock().expect("ssh master stderr lock");
                buf.extend_from_slice(&chunk[..read]);
                let overflow = buf.len().saturating_sub(MAX_STDERR_BYTES);
                if overflow > 0 {
                    buf.drain(..overflow);
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr_buf,
            stderr_task: Some(stderr_task),
        })
    }

    fn take_stderr(&self) -> String {
        let mut buf = self.stderr_buf.lock().expect("ssh master stderr lock");
        String::from_utf8_lossy(&std::mem::take(&mut *buf))
            .trim()
            .to_string()
    }

    /// A failing `ssh` writes its diagnostic ("Permission denied (publickey)",
    /// "Host key verification failed", …) to stderr and exits; we notice the
    /// stdout EOF first, so reporting immediately loses the only line that says
    /// what went wrong — and hides auth failures from `ssh_guard`. Wait for the
    /// child and its stderr reader before formatting.
    async fn transport_error(&mut self, action: &str, detail: String) -> String {
        let status = match tokio::time::timeout(DRAIN_TIMEOUT, self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => None,
        };
        if let Some(task) = self.stderr_task.take() {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, task).await;
        }
        let stderr = self.take_stderr();
        let exit = match status {
            Some(code) => format!(" (ssh exited with status {code})"),
            None => String::new(),
        };
        if stderr.is_empty() {
            format!("ssh master {action}: {detail}{exit}")
        } else {
            format!("ssh master {action}: {detail}{exit}: {stderr}")
        }
    }

    async fn rpc(&mut self, payload: &SshPayload) -> Result<SshRpcOutput, String> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let marker = exit_marker(&nonce);
        let marker = marker.as_bytes();
        if let Err(e) = self
            .stdin
            .write_all(frame(payload, &nonce).as_bytes())
            .await
        {
            return Err(self.transport_error("write failed", e.to_string()).await);
        }
        if let Err(e) = self.stdin.flush().await {
            return Err(self.transport_error("write failed", e.to_string()).await);
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            if let Some(start) = buf
                .windows(marker.len())
                .position(|window| window == marker)
            {
                let tail = &buf[start + marker.len()..];
                if let Some(end) = tail.iter().position(|byte| *byte == b'\n') {
                    let exit_code = std::str::from_utf8(&tail[..end])
                        .ok()
                        .and_then(|code| code.trim().parse::<i64>().ok())
                        .ok_or_else(|| "ssh master returned a malformed exit code".to_string())?;
                    return Ok(SshRpcOutput {
                        exit_code,
                        stdout: buf[..start].to_vec(),
                        stderr: self.take_stderr(),
                    });
                }
            }
            if buf.len() > MAX_RPC_OUTPUT_BYTES {
                return Err(format!(
                    "ssh command output exceeded {MAX_RPC_OUTPUT_BYTES} bytes"
                ));
            }
            let read = match self.stdout.read(&mut chunk).await {
                Ok(read) => read,
                Err(e) => return Err(self.transport_error("read failed", e.to_string()).await),
            };
            if read == 0 {
                return Err(self
                    .transport_error("connection closed", "unexpected EOF".into())
                    .await);
            }
            buf.extend_from_slice(&chunk[..read]);
        }
    }
}

#[derive(Default)]
struct Slot {
    args: Vec<String>,
    master: Option<Master>,
}

fn pool() -> &'static Mutex<HashMap<String, Arc<Mutex<Slot>>>> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<Mutex<Slot>>>>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}

/// Run one payload over the persistent master for `key` (the execution
/// context id), spawning or replacing the master as needed. `ssh_args` are
/// the client options + target, without any remote command.
pub async fn run(
    key: &str,
    ssh_args: Vec<String>,
    envs: &[(String, String)],
    payload: SshPayload,
    timeout: Duration,
) -> Result<SshRpcOutput, String> {
    let slot = {
        let mut pool = pool().lock().await;
        pool.entry(key.to_string()).or_default().clone()
    };
    // ponytail: RPCs serialize per host; fine while each RPC is a short script
    let mut slot = slot.lock().await;
    if slot.master.is_some() && slot.args != ssh_args {
        slot.master = None;
    }
    let reused = slot.master.is_some();
    let mut spawn_args = ssh_args.clone();
    spawn_args.push("sh".into());
    if !reused {
        slot.master = Some(Master::spawn("ssh", &spawn_args, envs)?);
        slot.args = ssh_args;
    }
    let master = slot.master.as_mut().expect("master just ensured");
    match tokio::time::timeout(timeout, master.rpc(&payload)).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(_stale)) if reused => {
            // An idle master may have died (sleep, network change, server
            // idle timeout); retry once on a fresh connection.
            slot.master = None;
            let master = slot.master.insert(Master::spawn("ssh", &spawn_args, envs)?);
            match tokio::time::timeout(timeout, master.rpc(&payload)).await {
                Ok(result) => {
                    if result.is_err() {
                        slot.master = None;
                    }
                    result
                }
                Err(_) => {
                    slot.master = None;
                    Err(format!(
                        "SSH command timed out after {}s",
                        timeout.as_secs()
                    ))
                }
            }
        }
        Ok(Err(error)) => {
            slot.master = None;
            Err(error)
        }
        Err(_) => {
            // The remote side may still be mid-RPC; the stream is desynced,
            // so the master cannot be reused.
            slot.master = None;
            Err(format!(
                "SSH command timed out after {}s",
                timeout.as_secs()
            ))
        }
    }
}

/// Bridge for the synchronous probe / file-browser runners.
pub fn run_blocking(
    key: &str,
    ssh_args: Vec<String>,
    envs: &[(String, String)],
    payload: SshPayload,
    timeout: Duration,
) -> Result<SshRpcOutput, String> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| {
            handle.block_on(run(key, ssh_args, envs, payload, timeout))
        }),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?
            .block_on(run(key, ssh_args, envs, payload, timeout)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_payload_classifies_command_shapes() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            eligible_payload("ssh", &args(&["-T", "host", "sh -s"]), Some("echo hi")),
            Some(SshPayload::Script(script)) if script == "echo hi"
        ));
        assert!(matches!(
            eligible_payload("ssh", &args(&["-T", "host", "uname -a"]), None),
            Some(SshPayload::Command(command)) if command == "uname -a"
        ));
        // stdin as data for the remote command needs a dedicated session
        assert!(eligible_payload("ssh", &args(&["-T", "host", "cat > f"]), Some("data")).is_none());
        // Harvest collect uses `sh -s --` so it does not take the shared slot.
        assert!(
            eligible_payload("ssh", &args(&["-T", "host", "sh -s --"]), Some("collect")).is_none()
        );
        assert!(eligible_payload("scp", &args(&["a", "b"]), None).is_none());
        assert!(eligible_payload("ssh", &args(&["host"]), None).is_none());
    }

    #[test]
    fn script_frame_avoids_heredoc_delimiter_collision() {
        let script = "echo one\n__WISP_MASTER_abc__\necho two";
        let framed = frame(&SshPayload::Script(script.into()), "abc");
        assert!(framed.starts_with("sh -s <<'__WISP_MASTER_abc__X' 2>&1\n"));
        assert!(framed.contains("\n__WISP_MASTER_abc__X\nprintf"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dying_master_reports_stderr_and_exit_status() {
        // Stands in for `ssh` refusing to connect: a diagnostic on stderr, then
        // exit. The error must quote it instead of a bare "unexpected EOF".
        let mut master = Master::spawn(
            "sh",
            &[
                "-c".into(),
                "echo 'Permission denied (publickey).' >&2; exit 255".into(),
            ],
            &[],
        )
        .unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(10),
            master.rpc(&SshPayload::Command("true".into())),
        )
        .await
        .expect("rpc timed out")
        .err()
        .expect("master exited, rpc must fail");
        assert!(error.contains("Permission denied (publickey)"), "{error}");
        assert!(error.contains("status 255"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_sh_master_round_trips_sequential_rpcs() {
        // Drive the framing protocol against a local `sh` standing in for
        // `ssh host sh`; two RPCs prove the stream stays in sync.
        async fn rpc(master: &mut Master, payload: SshPayload) -> SshRpcOutput {
            tokio::time::timeout(Duration::from_secs(10), master.rpc(&payload))
                .await
                .expect("rpc timed out")
                .expect("rpc failed")
        }
        let mut master = Master::spawn("sh", &[], &[]).unwrap();
        let out = rpc(
            &mut master,
            SshPayload::Script("echo out\necho err >&2\nexit 3".into()),
        )
        .await;
        assert_eq!(out.exit_code, 3);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "out\nerr\n");
        let out = rpc(
            &mut master,
            SshPayload::Command("printf 'no newline'".into()),
        )
        .await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "no newline");
        // NUL bytes and inner heredocs survive framing
        let out = rpc(
            &mut master,
            SshPayload::Script("cat <<'INNER'\ninner heredoc\nINNER\nprintf 'a\\000b'".into()),
        )
        .await;
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, b"inner heredoc\na\0b");
    }
}
