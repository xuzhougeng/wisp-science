//! Post-run server workspace cleanup. Deletes exactly one run workdir after
//! the run is terminal and its declared outputs were harvested (or the user
//! explicitly confirmed data loss), so servers never accumulate garbage.

use super::{checked_output, ssh_script_command, RemoteRunHandle, RunCommandRunner};
use base64::Engine;
use std::time::Duration;

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DONE_MARKER: &str = "__WISP_CLEANUP__:done";

/// Per-stream cap on how many trailing log bytes are pulled back before
/// cleanup. Big enough for real diagnostics, small enough to move over one
/// script RPC.
pub(super) const LOG_PULL_CAP_BYTES: u64 = 4 * 1024 * 1024;
const LOG_MARKER: &str = "__WISP_LOGPULL__";

/// One pulled log stream: the total size on the server and the (possibly
/// truncated) trailing bytes.
pub(super) struct PulledLog {
    pub total_size: u64,
    pub bytes: Vec<u8>,
}

pub(super) enum LogPull {
    /// The workdir no longer exists on the server; nothing to save.
    Absent,
    /// The server has no base64 encoder; caller falls back to stored tails.
    EncoderMissing,
    Logs {
        stdout: Option<PulledLog>,
        stderr: Option<PulledLog>,
    },
}

/// The only path cleanup will ever delete: a HOME-relative workdir that ends
/// with this run's id, with no traversal or expansion tricks. Never trusts a
/// string that could resolve to `~`, `/`, or another run's directory.
pub(super) fn validate_cleanup_workdir(workdir: &str, run_id: &str) -> Result<(), String> {
    if workdir.trim().is_empty() || workdir.len() > 512 {
        return Err("run workdir path is empty or too long".into());
    }
    if let Some(bad) = workdir
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || "._-/".contains(*c)))
    {
        return Err(format!(
            "run workdir contains an unsupported character '{bad}'"
        ));
    }
    if workdir.starts_with('/') {
        return Err("run workdir must be HOME-relative".into());
    }
    let segments: Vec<&str> = workdir.split('/').collect();
    if segments.len() < 2
        || segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err("run workdir must be a nested HOME-relative path".into());
    }
    if segments.last() != Some(&run_id) {
        return Err("run workdir does not belong to this run".into());
    }
    Ok(())
}

fn posix_logs_payload(workdir: &str) -> String {
    format!(
        r#"set -eu
workdir="$HOME/{workdir}"
if [ ! -d "$workdir" ]; then
  printf '{LOG_MARKER}:absent\n{LOG_MARKER}:done\n'
  exit 0
fi
if ! command -v base64 >/dev/null 2>&1; then
  printf '{LOG_MARKER}:noencoder\n{LOG_MARKER}:done\n'
  exit 0
fi
for stream in stdout stderr; do
  f="$workdir/$stream.log"
  if [ -f "$f" ]; then
    size=$(wc -c < "$f" | tr -d '[:space:]')
    printf '{LOG_MARKER}:%s:%s\n' "$stream" "$size"
    tail -c {LOG_PULL_CAP_BYTES} "$f" | base64
    printf '{LOG_MARKER}:end\n'
  else
    printf '{LOG_MARKER}:%s:missing\n' "$stream"
  fi
done
printf '{LOG_MARKER}:done\n'
"#
    )
}

fn windows_logs_payload(workdir: &str) -> String {
    let windows_workdir = workdir.replace('/', "\\");
    format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $HOME '{windows_workdir}'
if (-not (Test-Path -LiteralPath $workdir)) {{
  Write-Output '{LOG_MARKER}:absent'
  Write-Output '{LOG_MARKER}:done'
  exit 0
}}
foreach ($stream in @('stdout','stderr')) {{
  $f = Join-Path $workdir "$stream.log"
  if (Test-Path -LiteralPath $f) {{
    $fs = [System.IO.File]::Open($f, 'Open', 'Read', 'ReadWrite')
    try {{
      $size = $fs.Length
      Write-Output ('{LOG_MARKER}:{{0}}:{{1}}' -f $stream, $size)
      $take = [int][Math]::Min($size, {LOG_PULL_CAP_BYTES})
      if ($take -gt 0) {{
        $fs.Seek(-$take, 'End') | Out-Null
        $buf = New-Object byte[] $take
        $read = $fs.Read($buf, 0, $take)
        Write-Output ([Convert]::ToBase64String($buf, 0, $read))
      }}
      Write-Output '{LOG_MARKER}:end'
    }} finally {{
      $fs.Dispose()
    }}
  }} else {{
    Write-Output ('{LOG_MARKER}:{{0}}:missing' -f $stream)
  }}
}}
Write-Output '{LOG_MARKER}:done'
"#
    )
}

pub(super) fn parse_log_pull(stdout: &str) -> Result<LogPull, String> {
    let normalized = stdout.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let mut streams: [Option<PulledLog>; 2] = [None, None];
    let mut done = false;
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix(&format!("{LOG_MARKER}:")) else {
            continue;
        };
        match rest {
            "done" => {
                done = true;
                break;
            }
            "absent" => return Ok(LogPull::Absent),
            "noencoder" => return Ok(LogPull::EncoderMissing),
            _ => {}
        }
        let (stream, size) = rest
            .split_once(':')
            .ok_or_else(|| format!("unexpected log pull marker: {line}"))?;
        let slot = match stream {
            "stdout" => 0,
            "stderr" => 1,
            _ => return Err(format!("unexpected log pull stream: {stream}")),
        };
        if size == "missing" {
            continue;
        }
        let total_size: u64 = size
            .parse()
            .map_err(|_| format!("unreadable log size for {stream}: {size}"))?;
        let mut encoded = String::new();
        let mut closed = false;
        for line in lines.by_ref() {
            if line == format!("{LOG_MARKER}:end") {
                closed = true;
                break;
            }
            encoded.push_str(line.trim());
        }
        if !closed {
            return Err(format!("log pull for {stream} ended mid-stream"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(|error| format!("log pull for {stream} is not valid base64: {error}"))?;
        streams[slot] = Some(PulledLog { total_size, bytes });
    }
    if !done {
        return Err("log pull did not confirm completion".into());
    }
    let [stdout, stderr] = streams;
    Ok(LogPull::Logs { stdout, stderr })
}

/// Read the trailing bytes of the run's full stdout/stderr logs from its
/// workspace, so cleanup never destroys the only copy of the logs.
pub(super) async fn fetch_run_logs(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
    run_id: &str,
) -> Result<LogPull, String> {
    let workdir = match handle {
        RemoteRunHandle::SshDirect { workdir, .. }
        | RemoteRunHandle::LocalDetached { workdir, .. } => workdir.as_str(),
    };
    validate_cleanup_workdir(workdir, run_id)?;
    let command = match handle {
        RemoteRunHandle::SshDirect { connection, .. } => ssh_script_command(
            connection,
            "save run logs before cleanup",
            posix_logs_payload(workdir),
        )?,
        RemoteRunHandle::LocalDetached { transport, .. } => {
            let payload = match transport {
                super::LocalTransport::Posix { .. } => posix_logs_payload(workdir),
                super::LocalTransport::Windows { .. } => windows_logs_payload(workdir),
            };
            super::local_detached::transport_script_command(
                handle,
                "save run logs before cleanup",
                payload,
            )?
        }
    };
    let output = checked_output(
        "run log retrieval",
        runner.run(command, CLEANUP_TIMEOUT).await,
    )?;
    parse_log_pull(&output.stdout)
}

fn posix_cleanup_payload(workdir: &str, token: &str) -> String {
    format!(
        r#"set -eu
workdir="$HOME/{workdir}"
if [ ! -d "$workdir" ]; then
  printf '{DONE_MARKER}\n'
  exit 0
fi
[ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
if [ -f "$workdir/_submitted" ]; then
  handle=$(cat "$workdir/_submitted")
  rest=${{handle#*:}}
  pgid=${{rest%%:*}}
  start=${{handle##*:}}
  current=$(awk '{{print $22}}' "/proc/$pgid/stat" 2>/dev/null || true)
  group=$(awk '{{print $5}}' "/proc/$pgid/stat" 2>/dev/null || true)
  if [ -n "$pgid" ] && [ "$current" = "$start" ] && [ "$group" = "$pgid" ]; then
    kill -KILL "-$pgid" 2>/dev/null || true
    sleep 1
  fi
fi
rm -rf "$workdir"
printf '{DONE_MARKER}\n'
"#
    )
}

fn windows_cleanup_payload(workdir: &str, token: &str) -> String {
    let windows_workdir = workdir.replace('/', "\\");
    format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $HOME '{windows_workdir}'
if (-not (Test-Path -LiteralPath $workdir)) {{
  Write-Output '{DONE_MARKER}'
  exit 0
}}
$tokenPath = Join-Path $workdir 'token'
if (-not (Test-Path -LiteralPath $tokenPath) -or (Get-Content -LiteralPath $tokenPath -Raw).Trim() -ne '{token}') {{
  Write-Error 'wisp token mismatch'
  exit 73
}}
Remove-Item -LiteralPath $workdir -Recurse -Force
Write-Output '{DONE_MARKER}'
"#
    )
}

fn cleanup_command(
    handle: &RemoteRunHandle,
    workdir: &str,
    token: &str,
) -> Result<super::RunCommand, String> {
    match handle {
        RemoteRunHandle::SshDirect { connection, .. } => ssh_script_command(
            connection,
            "clean up run workspace",
            posix_cleanup_payload(workdir, token),
        ),
        RemoteRunHandle::LocalDetached { transport, .. } => {
            let payload = match transport {
                super::LocalTransport::Posix { .. } => posix_cleanup_payload(workdir, token),
                super::LocalTransport::Windows { .. } => windows_cleanup_payload(workdir, token),
            };
            super::local_detached::transport_script_command(
                handle,
                "clean up run workspace",
                payload,
            )
        }
    }
}

/// Delete the run's workdir on its execution context. The caller has already
/// enforced lifecycle preconditions; this function only guards the path.
pub(super) async fn delete_run_workspace(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
    run_id: &str,
) -> Result<(), String> {
    let (workdir, token) = match handle {
        RemoteRunHandle::SshDirect { workdir, token, .. }
        | RemoteRunHandle::LocalDetached { workdir, token, .. } => {
            (workdir.as_str(), token.as_str())
        }
    };
    validate_cleanup_workdir(workdir, run_id)?;
    let output = checked_output(
        "run workspace cleanup",
        runner
            .run(cleanup_command(handle, workdir, token)?, CLEANUP_TIMEOUT)
            .await,
    )?;
    let normalized = output.stdout.replace("\r\n", "\n");
    if !normalized.lines().any(|line| line == DONE_MARKER) {
        return Err("run workspace cleanup did not confirm deletion".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_validation_rejects_escapes_and_foreign_runs() {
        assert!(validate_cleanup_workdir(".wisp-science/runs/run-1", "run-1").is_ok());
        assert!(validate_cleanup_workdir("scratch/wisp-runs/run-1", "run-1").is_ok());
        for (workdir, run_id) in [
            ("", "run-1"),
            ("/etc", "etc"),
            ("run-1", "run-1"),
            ("~/runs/run-1", "run-1"),
            ("runs/../run-1", "run-1"),
            ("runs/run-2", "run-1"),
            ("runs/$HOME/run-1", "run-1"),
            ("runs/a b/run-1", "run-1"),
        ] {
            assert!(
                validate_cleanup_workdir(workdir, run_id).is_err(),
                "{workdir}"
            );
        }
    }

    #[test]
    fn posix_payload_kills_the_confirmed_group_then_removes_only_the_workdir() {
        let payload = posix_cleanup_payload(".wisp-science/runs/run-1", "tok");
        assert!(payload.contains("workdir=\"$HOME/.wisp-science/runs/run-1\""));
        assert!(payload.contains("wisp token mismatch"));
        assert!(payload.contains("kill -KILL \"-$pgid\""));
        assert!(payload.contains("rm -rf \"$workdir\""));
        assert!(!payload.contains("rm -rf \"$HOME\""));
    }

    #[test]
    fn windows_payload_uses_native_removal_under_home() {
        let payload = windows_cleanup_payload(".wisp-science/runs/run-1", "tok");
        assert!(payload.contains("Join-Path $HOME '.wisp-science\\runs\\run-1'"));
        assert!(payload.contains("Remove-Item -LiteralPath $workdir -Recurse -Force"));
        assert!(payload.contains("wisp token mismatch"));
    }

    #[test]
    fn log_payloads_read_capped_tails_and_never_delete() {
        let posix = posix_logs_payload(".wisp-science/runs/run-1");
        assert!(posix.contains("workdir=\"$HOME/.wisp-science/runs/run-1\""));
        assert!(posix.contains(&format!("tail -c {LOG_PULL_CAP_BYTES}")));
        assert!(posix.contains("base64"));
        assert!(!posix.contains("rm "));
        let windows = windows_logs_payload(".wisp-science/runs/run-1");
        assert!(windows.contains("Join-Path $HOME '.wisp-science\\runs\\run-1'"));
        assert!(windows.contains("ToBase64String"));
        assert!(!windows.contains("Remove-Item"));
    }

    #[test]
    fn log_pull_parser_decodes_streams_and_detects_truncation() {
        let stdout_b64 = base64::engine::general_purpose::STANDARD.encode(b"hello stdout\n");
        let output = format!(
            "__WISP_LOGPULL__:stdout:999\n{stdout_b64}\n__WISP_LOGPULL__:end\n\
             __WISP_LOGPULL__:stderr:missing\n__WISP_LOGPULL__:done\n"
        );
        let LogPull::Logs { stdout, stderr } = parse_log_pull(&output).unwrap() else {
            panic!("expected logs");
        };
        let stdout = stdout.unwrap();
        assert_eq!(stdout.bytes, b"hello stdout\n");
        assert_eq!(stdout.total_size, 999);
        assert!(stderr.is_none());

        assert!(matches!(
            parse_log_pull("__WISP_LOGPULL__:absent\n__WISP_LOGPULL__:done\n").unwrap(),
            LogPull::Absent
        ));
        assert!(matches!(
            parse_log_pull("__WISP_LOGPULL__:noencoder\n__WISP_LOGPULL__:done\n").unwrap(),
            LogPull::EncoderMissing
        ));
        // A dropped connection mid-stream or a missing completion sentinel
        // must not silently pass as "no logs".
        assert!(parse_log_pull("__WISP_LOGPULL__:stdout:9\nAAAA\n").is_err());
        assert!(parse_log_pull("").is_err());
    }
}
