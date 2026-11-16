//! Reading one file of an SSH host for a preview, shared by the WebView
//! desktop (`src-tauri/src/file_browser.rs`) and the GPUI host.
//!
//! The host is asked with a small POSIX script passed as the remote command
//! of `ssh`; the file's path goes into the script only single-quoted (or
//! after `"$HOME"/` for `~/…`), so a name with quotes, spaces, `;`, `$()`
//! or a leading `-` stays a name. A path with a NUL or a line break is
//! refused before anything runs. The answer starts with a protocol marker,
//! so a login banner before it is passed over, and its bytes are kept as
//! bytes: a picture is not run through UTF-8. Nothing is written on the
//! host or on this computer.

use crate::ssh_guard;
use crate::ssh_hosts::{self, SshConnection};

/// Whole-file answers: the marker, then the file's bytes.
pub const REMOTE_FILE_PROTOCOL: &[u8] = b"WISP_REMOTE_FILE_V1\0";
/// Text-oriented remote reads: size + truncated head so multi-GB logs never
/// cross the wire just for a UI preview.
pub const REMOTE_FILE_TEXT_PROTOCOL: &[u8] = b"WISP_REMOTE_FILE_TEXT_V1\0";
/// Remote previews stay capped at 32 MB: the bytes cross an SSH connection.
pub const REMOTE_FILE_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// One `ssh` invocation, as run on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommand {
    pub context_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

/// What it answered, its stdout kept as bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Runs a [`RemoteCommand`]; tests give a fake that answers what they say.
pub trait RemoteRunner: Send {
    fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String>;
}

/// Runs the command for real, over the host's shared SSH connection when
/// one can carry it. Blocking: call it off an async task's thread.
pub struct ProcessRemoteRunner;

impl RemoteRunner for ProcessRemoteRunner {
    fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String> {
        if let Some(payload) =
            crate::ssh_master::eligible_payload(&command.program, &command.args, None)
        {
            let ssh_args = command.args[..command.args.len() - 1].to_vec();
            let result = crate::ssh_master::run_blocking(
                &command.context_id,
                ssh_args,
                &command.envs,
                payload,
                std::time::Duration::from_secs(120),
            )
            .map(|output| RemoteOutput {
                status: output.exit_code as i32,
                stdout: output.stdout,
                stderr: output.stderr,
            });
            ssh_hosts::cleanup_password_auth_env(&command.envs);
            return result;
        }
        let mut process = std::process::Command::new(&command.program);
        process.args(&command.args);
        if !command.envs.is_empty() {
            process.envs(command.envs.iter().cloned());
        }
        wisp_tools::process::hide_console(&mut process);
        let output = process
            .output()
            .map_err(|e| format!("failed to run {}: {e}", command.program))?;
        ssh_hosts::cleanup_password_auth_env(&command.envs);
        Ok(RemoteOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

pub fn validate_remote_path(path: &str) -> Result<(), String> {
    if path.len() > 4096 {
        return Err("Remote path exceeds 4096 bytes".into());
    }
    if path.contains(['\0', '\n', '\r']) {
        return Err("Remote path must not contain NUL or line breaks".into());
    }
    Ok(())
}

pub fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// The path as the scripts name it: empty or `~` is the home folder, `~/x`
/// is under it, anything else is taken as written, quoted as data.
pub fn remote_path_expression(path: Option<&str>) -> Result<String, String> {
    let path = path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or("~");
    validate_remote_path(path)?;
    if path == "~" {
        Ok("\"$HOME\"".into())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("\"$HOME\"/{}", shell_single_quote(rest)))
    } else {
        Ok(shell_single_quote(path))
    }
}

/// POSIX-sh script that size-checks then streams one remote file in full.
/// Binary / office previews need an intact payload, so oversized files are
/// rejected on the remote before any bytes cross the wire.
pub fn remote_file_script(path: &str, cap: u64) -> Result<String, String> {
    let path = remote_path_expression(Some(path))?;
    Ok(format!(
        r#"LC_ALL=C
f={path}
case "$f" in -*) f="./$f" ;; esac
if [ ! -f "$f" ]; then
  printf 'Cannot read remote file: %s\n' "$f" >&2
  exit 66
fi
size=$(stat -c '%s' "$f" 2>/dev/null) ||
  size=$(stat -f '%z' "$f" 2>/dev/null) ||
  size=0
if [ "$size" -gt {cap} ]; then
  printf 'Remote file exceeds {cap} byte limit: %s\n' "$f" >&2
  exit 67
fi
printf 'WISP_REMOTE_FILE_V1\000'
cat "$f""#
    ))
}

/// POSIX-sh script that returns a bounded head of a remote text-ish file plus
/// the real size, so multi-GB MEDLINE dumps preview without a full transfer.
pub fn remote_text_file_script(path: &str, cap: u64) -> Result<String, String> {
    let path = remote_path_expression(Some(path))?;
    Ok(format!(
        r#"LC_ALL=C
f={path}
case "$f" in -*) f="./$f" ;; esac
if [ ! -f "$f" ]; then
  printf 'Cannot read remote file: %s\n' "$f" >&2
  exit 66
fi
size=$(stat -c '%s' "$f" 2>/dev/null) ||
  size=$(stat -f '%z' "$f" 2>/dev/null) ||
  size=0
printf 'WISP_REMOTE_FILE_TEXT_V1\000'
printf '%s\000' "$size"
if [ "$size" -eq 0 ] || [ {cap} -eq 0 ]; then
  :
elif command -v head >/dev/null 2>&1; then
  head -c {cap} "$f"
else
  dd if="$f" bs={cap} count=1 2>/dev/null
fi"#
    ))
}

/// `ssh` to the context's host with `script` as its remote command.
pub fn build_remote_script_command(
    context: &wisp_store::ExecutionContext,
    script: String,
) -> Result<RemoteCommand, String> {
    let connection = SshConnection::from_execution_context(context)?;
    let mut args = connection.ssh_args()?;
    args.push(script);
    Ok(RemoteCommand {
        context_id: context.id.clone(),
        program: "ssh".into(),
        args,
        envs: ssh_hosts::auth_envs_for_connection(&connection)?,
    })
}

pub fn build_remote_file_command(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
) -> Result<RemoteCommand, String> {
    build_remote_script_command(context, remote_file_script(path, cap)?)
}

pub fn build_remote_text_file_command(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
) -> Result<RemoteCommand, String> {
    build_remote_script_command(context, remote_text_file_script(path, cap)?)
}

/// Slice off everything before (and including) the protocol marker, so login
/// banners and motd noise never reach the parser.
pub fn protocol_payload<'a>(stdout: &'a [u8], marker: &[u8]) -> Result<&'a [u8], String> {
    stdout
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|start| &stdout[start + marker.len()..])
        .ok_or_else(|| "Remote response did not contain the expected protocol marker".into())
}

/// Payload shape: `total_size\0<body>` after the text-protocol marker.
pub fn parse_remote_text_payload(stdout: &[u8]) -> Result<(Vec<u8>, u64), String> {
    let payload = protocol_payload(stdout, REMOTE_FILE_TEXT_PROTOCOL)?;
    let zero = payload
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| "Remote text response omitted its size field".to_string())?;
    let total = std::str::from_utf8(&payload[..zero])
        .map_err(|_| "Remote text response size was not UTF-8".to_string())?
        .parse::<u64>()
        .map_err(|_| "Remote text response size was not a number".to_string())?;
    Ok((payload[zero + 1..].to_vec(), total))
}

/// Runs `command`, noting a sign-in failure against the host.
pub fn run_remote_command(
    context: &wisp_store::ExecutionContext,
    command: &RemoteCommand,
    runner: &mut dyn RemoteRunner,
) -> Result<RemoteOutput, String> {
    match runner.run(command) {
        Ok(output) => Ok(output),
        Err(error) => {
            if ssh_guard::is_authentication_failure(&error) {
                ssh_guard::record_failure(&context.id, &error);
            }
            Err(error)
        }
    }
}

/// The error for a script that exited with a failure.
pub fn remote_command_failed(
    context: &wisp_store::ExecutionContext,
    output: &RemoteOutput,
) -> String {
    let detail = if output.stderr.is_empty() {
        "no error details returned".to_string()
    } else {
        output.stderr.clone()
    };
    let error = format!(
        "Remote file request failed (exit {}): {detail}",
        output.status
    );
    if ssh_guard::is_authentication_failure(&error) {
        ssh_guard::record_failure(&context.id, &error);
    }
    error
}

/// At most `cap` bytes from the head of `path` on the host, and the file's
/// whole size. The caller gates the host (`require_managed_ssh_ready`).
pub fn read_remote_text_head(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
    runner: &mut dyn RemoteRunner,
) -> Result<(Vec<u8>, u64), String> {
    let cap = cap.min(REMOTE_FILE_MAX_BYTES);
    let command = build_remote_text_file_command(context, path, cap)?;
    let output = run_remote_command(context, &command, runner)?;
    if output.status != 0 {
        return Err(remote_command_failed(context, &output));
    }
    ssh_guard::record_success(&context.id);
    let (bytes, total) = parse_remote_text_payload(&output.stdout)?;
    if bytes.len() as u64 > cap {
        return Err(format!("remote file exceeds {cap} byte limit"));
    }
    Ok((bytes, total))
}

/// The whole of `path` on the host, refused there when it is larger than
/// `cap`. The caller gates the host (`require_managed_ssh_ready`).
pub fn read_remote_file_whole(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
    runner: &mut dyn RemoteRunner,
) -> Result<Vec<u8>, String> {
    let cap = cap.min(REMOTE_FILE_MAX_BYTES);
    let command = build_remote_file_command(context, path, cap)?;
    let output = run_remote_command(context, &command, runner)?;
    if output.status != 0 {
        return Err(remote_command_failed(context, &output));
    }
    ssh_guard::record_success(&context.id);
    let bytes = protocol_payload(&output.stdout, REMOTE_FILE_PROTOCOL)?.to_vec();
    if bytes.len() as u64 > cap {
        return Err(format!("remote file exceeds {cap} byte limit"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers in order what it was given, keeping what it was asked.
    struct FakeRemoteRunner {
        outputs: Vec<Result<RemoteOutput, String>>,
        commands: Vec<RemoteCommand>,
    }

    impl FakeRemoteRunner {
        fn answering(outputs: Vec<RemoteOutput>) -> Self {
            Self {
                outputs: outputs.into_iter().map(Ok).collect(),
                commands: Vec::new(),
            }
        }
    }

    impl RemoteRunner for FakeRemoteRunner {
        fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String> {
            self.commands.push(command.clone());
            assert!(
                !self.outputs.is_empty(),
                "the fake runner ran out of answers"
            );
            self.outputs.remove(0)
        }
    }

    /// A key-signed host context; the key file is a temporary one, so
    /// nothing of the user's is read and nothing connects.
    fn host(key: &std::path::Path) -> wisp_store::ExecutionContext {
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        context.config_json = serde_json::json!({
            "alias": "gpu.example",
            "user": "researcher",
            "port": 2222,
            "identity_file": key.to_string_lossy(),
        })
        .to_string();
        context.last_probe_status = Some("ok".into());
        context
    }

    fn key() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("wisp-remote-file-key-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"test-key\n").unwrap();
        path
    }

    #[test]
    fn a_hostile_name_reaches_the_host_only_as_quoted_data() {
        for (path, quoted) in [
            ("/data/it's; rm -rf ~", "f='/data/it'\"'\"'s; rm -rf ~'\n"),
            ("/data/$(reboot) `id`.txt", "f='/data/$(reboot) `id`.txt'\n"),
            ("-rf", "f='-rf'\n"),
            ("~/run 1/a b.csv", "f=\"$HOME\"/'run 1/a b.csv'\n"),
            ("~", "f=\"$HOME\"\n"),
        ] {
            for script in [
                remote_text_file_script(path, 1024).unwrap(),
                remote_file_script(path, 1024).unwrap(),
            ] {
                assert!(script.contains(quoted), "{path}: {script}");
                // A name that starts with `-` is never read as an option.
                assert!(script.contains("case \"$f\" in -*) f=\"./$f\" ;; esac"));
            }
        }
        for broken in ["/a\nrm -rf /", "/a\rb", "/a\0b"] {
            assert!(remote_text_file_script(broken, 1).is_err(), "{broken:?}");
            assert!(remote_file_script(broken, 1).is_err(), "{broken:?}");
        }
        assert!(remote_file_script(&"x".repeat(5000), 1).is_err());
        // The scripts only read.
        for script in [
            remote_text_file_script("/d", 1).unwrap(),
            remote_file_script("/d", 1).unwrap(),
        ] {
            for word in ["rm ", "mv ", "> \"", "mkdir", "touch"] {
                assert!(!script.contains(word), "{word}");
            }
        }
    }

    #[test]
    fn a_head_is_read_past_a_banner_with_the_whole_size() {
        let key = key();
        let context = host(&key);
        let mut runner = FakeRemoteRunner::answering(vec![RemoteOutput {
            status: 0,
            stdout: b"Welcome\nWISP_REMOTE_FILE_TEXT_V1\0999\0gene,count\n".to_vec(),
            stderr: String::new(),
        }]);
        let (head, total) =
            read_remote_text_head(&context, "/data/it's.csv", 1024, &mut runner).unwrap();
        assert_eq!(head, b"gene,count\n");
        assert_eq!(total, 999);
        let command = &runner.commands[0];
        assert_eq!(command.program, "ssh");
        assert_eq!(command.context_id, "ssh:gpu");
        assert!(command
            .args
            .iter()
            .any(|arg| arg == "researcher@gpu.example"));
        assert_eq!(
            command.args.last().unwrap(),
            &remote_text_file_script("/data/it's.csv", 1024).unwrap()
        );
        // More than was asked for is refused, not shown.
        let mut over = FakeRemoteRunner::answering(vec![RemoteOutput {
            status: 0,
            stdout: b"WISP_REMOTE_FILE_TEXT_V1\x005\0abcdef".to_vec(),
            stderr: String::new(),
        }]);
        assert!(read_remote_text_head(&context, "/a", 3, &mut over)
            .unwrap_err()
            .contains("byte limit"));
        let _ = std::fs::remove_file(key);
    }

    #[test]
    fn a_whole_file_keeps_its_bytes_and_its_cap() {
        let key = key();
        let context = host(&key);
        let picture = b"\x89PNG\r\n\x1a\n\0\xff\xfe";
        let mut stdout = b"motd\nWISP_REMOTE_FILE_V1\0".to_vec();
        stdout.extend_from_slice(picture);
        let mut runner = FakeRemoteRunner::answering(vec![RemoteOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        }]);
        let bytes = read_remote_file_whole(&context, "/p/a.png", u64::MAX, &mut runner).unwrap();
        assert_eq!(bytes, picture);
        // The cap is never more than a remote preview may fetch.
        assert!(runner.commands[0]
            .args
            .last()
            .unwrap()
            .contains(&format!("-gt {REMOTE_FILE_MAX_BYTES} ]")));
        // The host refusing a larger file is said, with its reason.
        let mut refused = FakeRemoteRunner::answering(vec![RemoteOutput {
            status: 67,
            stdout: Vec::new(),
            stderr: "Remote file exceeds 33554432 byte limit: /p/big.png".into(),
        }]);
        let error = read_remote_file_whole(&context, "/p/big.png", 1, &mut refused).unwrap_err();
        assert!(
            error.contains("exit 67") && error.contains("byte limit"),
            "{error}"
        );
        // An answer without the marker is not a file.
        let mut noise = FakeRemoteRunner::answering(vec![RemoteOutput {
            status: 0,
            stdout: b"Permission denied".to_vec(),
            stderr: String::new(),
        }]);
        assert!(read_remote_file_whole(&context, "/p/a.png", 1, &mut noise)
            .unwrap_err()
            .contains("protocol marker"));
        let _ = std::fs::remove_file(key);
    }
}
