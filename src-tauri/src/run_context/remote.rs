use super::{
    RemoteRun, RemoteRunHandle, RunCommand, RunCommandOutput, RunCommandRunner, REMOTE_RPC_TIMEOUT,
    REMOTE_START_LEASE_SECS,
};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) fn resolve_input_paths(root: &Path, refs: &[String]) -> Result<Vec<PathBuf>, String> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("cannot resolve project root {}: {e}", root.display()))?;
    let mut names = HashSet::new();
    refs.iter()
        .map(|value| {
            let relative = Path::new(value);
            if relative.as_os_str().is_empty()
                || relative.is_absolute()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(format!("SSH input must be project-relative: {value}"));
            }
            let path = std::fs::canonicalize(canonical_root.join(relative))
                .map_err(|e| format!("cannot resolve SSH input {value}: {e}"))?;
            if !path.starts_with(&canonical_root) || !path.is_file() {
                return Err(format!("SSH input is not a project file: {value}"));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("SSH input filename is not UTF-8: {value}"))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            {
                return Err(format!(
                    "SSH input filename must use letters, numbers, '.', '_' or '-': {name}"
                ));
            }
            if !names.insert(name.to_string()) {
                return Err(format!("SSH inputs contain duplicate filename: {name}"));
            }
            Ok(path)
        })
        .collect()
}

pub(super) fn ssh_script_command(
    connection: &crate::ssh_hosts::SshConnection,
    label: &str,
    payload: String,
) -> Result<RunCommand, String> {
    let mut args = connection.ssh_args()?;
    args.push("sh -s".into());
    Ok(RunCommand {
        context_id: format!("ssh:{}", connection.alias),
        program: "ssh".into(),
        args,
        script: label.into(),
        cwd: None,
        stdin: Some(payload),
        envs: crate::ssh_hosts::auth_envs_for_connection(connection)?,
    })
}

pub(super) fn checked_output(
    label: &str,
    output: Result<RunCommandOutput, String>,
) -> Result<RunCommandOutput, String> {
    let output = output?;
    if output.exit_code == 0 {
        Ok(output)
    } else {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        Err(format!(
            "{label} failed with exit {}: {detail}",
            output.exit_code
        ))
    }
}

pub(super) enum PrepareRemote {
    Prepared,
    Existing(RemoteRunHandle),
}

pub(super) fn remote_parts(
    handle: &RemoteRunHandle,
) -> (
    &crate::ssh_hosts::SshConnection,
    &str,
    &str,
    Option<i64>,
    Option<u64>,
) {
    match handle {
        RemoteRunHandle::SshDirect {
            connection,
            workdir,
            token,
            pgid,
            start_time,
            ..
        } => (connection, workdir, token, *pgid, *start_time),
    }
}

pub(super) fn handle_from_ack(
    handle: &RemoteRunHandle,
    stdout: &str,
) -> Result<RemoteRunHandle, String> {
    const PREFIX: &str = "__WISP_HANDLE__:";
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(PREFIX))
        .ok_or_else(|| "SSH launcher did not return a remote handle".to_string())?;
    let mut fields = line.trim().split(':');
    let ack_token = fields.next().unwrap_or_default();
    let pgid = fields
        .next()
        .ok_or_else(|| "SSH launcher omitted PGID".to_string())?
        .parse::<i64>()
        .map_err(|_| "SSH launcher returned an invalid PGID".to_string())?;
    let start_time = fields
        .next()
        .ok_or_else(|| "SSH launcher omitted process start time".to_string())?
        .parse::<u64>()
        .map_err(|_| "SSH launcher returned an invalid process start time".to_string())?;
    if fields.next().is_some() || pgid <= 1 {
        return Err("SSH launcher returned a malformed remote handle".into());
    }
    match handle {
        RemoteRunHandle::SshDirect {
            connection,
            workdir,
            token,
            inputs_staged,
            ..
        } if token == ack_token => Ok(RemoteRunHandle::SshDirect {
            connection: connection.clone(),
            workdir: workdir.clone(),
            token: token.clone(),
            inputs_staged: *inputs_staged,
            pgid: Some(pgid),
            start_time: Some(start_time),
        }),
        _ => Err("SSH launcher token does not match this Run".into()),
    }
}

pub(super) fn command_delimiter(token: &str, command: &str) -> String {
    let mut delimiter = format!("__WISP_COMMAND_{}__", token.replace('-', "_"));
    while command.lines().any(|line| line == delimiter) {
        delimiter.push('X');
    }
    delimiter
}

pub(super) fn prepare_payload(remote: &RemoteRun) -> String {
    let (_, workdir, token, _, _) = remote_parts(&remote.handle);
    let delimiter = command_delimiter(token, &remote.command);
    format!(
        r#"set -eu
umask 077
workdir="$HOME/{workdir}"
mkdir -p "$workdir"
mkdir -p "$workdir/inputs"
if [ -f "$workdir/token" ]; then
  [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
else
  printf '%s\n' '{token}' > "$workdir/token.tmp"
  mv "$workdir/token.tmp" "$workdir/token"
fi
if [ -f "$workdir/_submitted" ]; then
  printf '__WISP_HANDLE__:'
  cat "$workdir/_submitted"
  exit 0
fi
cat > "$workdir/command.sh" <<'{delimiter}'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/inputs"
{command}
{delimiter}
cat > "$workdir/supervisor.sh" <<'__WISP_SUPERVISOR__'
#!/bin/sh
set +e
umask 077
cd "$(dirname "$0")" || exit 125
write_state() {{
  path=$1
  value=$2
  tmp="$path.tmp.$$"
  printf '%s\n' "$value" > "$tmp" && mv "$tmp" "$path"
}}
if ! command -v setsid >/dev/null 2>&1 || ! command -v timeout >/dev/null 2>&1 || ! command -v bash >/dev/null 2>&1; then
  write_state _status 'lost:ssh direct Run requires setsid, timeout, and bash'
  exit 69
fi
rm -f _command_exit
setsid timeout -k 10 {timeout_secs} sh -c 'bash -l "$1"; rc=$?; tmp="$2.tmp.$$"; printf "%s\\n" "$rc" > "$tmp" && mv "$tmp" "$2"; exit "$rc"' sh "$PWD/command.sh" "$PWD/_command_exit" >stdout.log 2>stderr.log &
pgid=$!
i=0
start_time=''
while [ "$i" -lt 5 ]; do
  start_time=$(awk '{{print $22}}' "/proc/$pgid/stat" 2>/dev/null || true)
  process_group=$(awk '{{print $5}}' "/proc/$pgid/stat" 2>/dev/null || true)
  if [ -n "$start_time" ] && [ "$process_group" = "$pgid" ]; then
    break
  fi
  sleep 1
  i=$((i + 1))
done
if [ -z "$start_time" ] || [ "$process_group" != "$pgid" ]; then
  write_state _status 'lost:command process group did not start'
  exit 69
fi
write_state _submitted '{token}:'"$pgid:$start_time"
write_state _status running
wait "$pgid"
rc=$?
if [ -f _cancel_requested ]; then
  write_state _status cancelled
elif [ -f _command_exit ]; then
  command_rc=$(cat _command_exit 2>/dev/null || printf '%s' "$rc")
  write_state _status "done:$command_rc"
elif [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
  write_state _status 'timed_out:124'
else
  write_state _status "done:$rc"
fi
exit "$rc"
__WISP_SUPERVISOR__
chmod 700 "$workdir/command.sh" "$workdir/supervisor.sh"
printf '__WISP_PREPARED__\n'
"#,
        command = remote.command,
        timeout_secs = remote.timeout.as_secs(),
    )
}

pub(super) async fn prepare_remote(
    runner: &dyn RunCommandRunner,
    remote: &RemoteRun,
) -> Result<PrepareRemote, String> {
    let (connection, _, _, _, _) = remote_parts(&remote.handle);
    let output = checked_output(
        "SSH prepare",
        runner
            .run(
                ssh_script_command(connection, "prepare SSH Run", prepare_payload(remote))?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    if output
        .stdout
        .lines()
        .any(|line| line == "__WISP_PREPARED__")
    {
        Ok(PrepareRemote::Prepared)
    } else {
        Ok(PrepareRemote::Existing(handle_from_ack(
            &remote.handle,
            &output.stdout,
        )?))
    }
}

pub(super) async fn stage_remote_inputs(
    store: &wisp_store::Store,
    owner_id: &str,
    runner: &dyn RunCommandRunner,
    remote: &RemoteRun,
) -> Result<(), String> {
    if remote.input_refs.is_empty() {
        return Ok(());
    }
    let root = remote
        .harvest_root
        .as_deref()
        .ok_or_else(|| "SSH input staging requires its project workspace".to_string())?;
    let input_paths = resolve_input_paths(root, &remote.input_refs)?;
    let inputs = input_paths
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("SSH input filename is not UTF-8: {}", path.display()))?;
            let size = std::fs::metadata(path)
                .map_err(|error| format!("cannot read SSH input {}: {error}", path.display()))?
                .len();
            Ok((name.to_string(), size))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let total_bytes = inputs.iter().map(|(_, size)| size).sum();
    let started = Instant::now();
    let initial = super::transfer_progress(
        "upload",
        "uploading",
        0,
        total_bytes,
        0,
        inputs.len() as u64,
        inputs.first().map(|(name, _)| name.clone()),
        started,
    );
    if !store
        .update_run_progress_owned(&remote.run_id, owner_id, &initial)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("SSH lifecycle lease expired before input staging".into());
    }
    let (connection, workdir, _, _, _) = remote_parts(&remote.handle);
    let mut args = connection.scp_option_args()?;
    args.extend(input_paths.iter().map(|path| scp_local_path(path)));
    args.push(format!("{}:{workdir}/inputs/", connection.target()?));
    let transfer = runner.run(
        RunCommand {
            context_id: format!("ssh:{}", connection.alias),
            program: "scp".into(),
            args,
            script: format!("stage {} input file(s)", input_paths.len()),
            cwd: remote.harvest_root.clone(),
            stdin: None,
            envs: crate::ssh_hosts::auth_envs_for_connection(connection)?,
        },
        remote.timeout.max(Duration::from_secs(300)),
    );
    tokio::pin!(transfer);
    let mut interval = tokio::time::interval(transfer_poll_interval());
    interval.tick().await;
    let mut last_completed_bytes = 0;
    let mut last_files_completed = 0;
    let mut last_current_file = inputs.first().map(|(name, _)| name.clone());
    let output = loop {
        tokio::select! {
            output = &mut transfer => break output,
            _ = interval.tick() => {
                let Ok(remote_sizes) = poll_remote_input_sizes(runner, connection, workdir, &inputs).await else {
                    continue;
                };
                let completed_bytes = inputs.iter().map(|(name, expected)| {
                    remote_sizes.get(name).copied().unwrap_or(0).min(*expected)
                }).sum();
                let files_completed = inputs.iter().filter(|(name, expected)| {
                    remote_sizes.get(name).is_some_and(|size| size >= expected)
                }).count() as u64;
                let current_file = inputs.iter().find(|(name, expected)| {
                    !remote_sizes.get(name).is_some_and(|size| size >= expected)
                }).map(|(name, _)| name.clone());
                last_completed_bytes = completed_bytes;
                last_files_completed = files_completed;
                last_current_file = current_file.clone();
                let progress = super::transfer_progress(
                    "upload", "uploading", completed_bytes, total_bytes,
                    files_completed, inputs.len() as u64, current_file, started,
                );
                if !store.update_run_progress_owned(&remote.run_id, owner_id, &progress)
                    .await.map_err(|error| error.to_string())? {
                    return Err("SSH lifecycle lease expired during input staging".into());
                }
            }
        }
    };
    let result = checked_output("SSH input staging", output).map(|_| ());
    let final_progress = super::transfer_progress(
        "upload",
        if result.is_ok() { "uploaded" } else { "failed" },
        if result.is_ok() {
            total_bytes
        } else {
            last_completed_bytes
        },
        total_bytes,
        if result.is_ok() {
            inputs.len() as u64
        } else {
            last_files_completed
        },
        inputs.len() as u64,
        if result.is_ok() {
            None
        } else {
            last_current_file
        },
        started,
    );
    let _ = store
        .update_run_progress_owned(&remote.run_id, owner_id, &final_progress)
        .await;
    result
}

fn transfer_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(1)
    }
}

fn input_progress_payload(workdir: &str, inputs: &[(String, u64)]) -> String {
    let mut payload = format!("set -u\nbase=\"$HOME/{workdir}/inputs\"\n");
    for (name, _) in inputs {
        payload.push_str(&format!(
            "if [ -f \"$base/{name}\" ]; then bytes=$(wc -c < \"$base/{name}\" 2>/dev/null || printf 0); printf '__WISP_TRANSFER_FILE__:{name}:%s\\n' \"$bytes\"; fi\n"
        ));
    }
    payload
}

pub(super) fn parse_input_progress(stdout: &str) -> HashMap<String, u64> {
    stdout
        .lines()
        .filter_map(|line| {
            let value = line.strip_prefix("__WISP_TRANSFER_FILE__:")?;
            let (name, bytes) = value.rsplit_once(':')?;
            Some((name.to_string(), bytes.parse().ok()?))
        })
        .collect()
}

async fn poll_remote_input_sizes(
    runner: &dyn RunCommandRunner,
    connection: &crate::ssh_hosts::SshConnection,
    workdir: &str,
    inputs: &[(String, u64)],
) -> Result<HashMap<String, u64>, String> {
    let output = checked_output(
        "SSH input progress",
        runner
            .run(
                ssh_script_command(
                    connection,
                    "poll SSH input progress",
                    input_progress_payload(workdir, inputs),
                )?,
                super::REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    Ok(parse_input_progress(&output.stdout))
}

pub(super) fn scp_local_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }
        if let Some(path) = path.strip_prefix(r"\\?\") {
            return path.to_string();
        }
    }
    path.into_owned()
}

pub(super) fn launch_payload(handle: &RemoteRunHandle) -> String {
    let (_, workdir, token, _, _) = remote_parts(handle);
    format!(
        r#"set -eu
workdir="$HOME/{workdir}"
[ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
lock="$workdir/_launch_lock"
if [ -d "$lock" ] && [ ! -f "$workdir/_submitted" ]; then
  owner=$(cat "$lock/owner" 2>/dev/null || true)
  lock_pid=${{owner%%:*}}
  lock_start=${{owner#*:}}
  current=$(awk '{{print $22}}' "/proc/$lock_pid/stat" 2>/dev/null || true)
  if [ -z "$lock_pid" ] || [ "$current" != "$lock_start" ]; then
    rm -f "$lock/owner"
    rmdir "$lock" 2>/dev/null || true
  fi
fi
if [ ! -f "$workdir/_submitted" ] && mkdir "$lock" 2>/dev/null; then
  trap 'rm -f "$lock/owner"; rmdir "$lock" 2>/dev/null || true' EXIT HUP INT TERM
  lock_start=$(awk '{{print $22}}' "/proc/$$/stat" 2>/dev/null || true)
  printf '%s:%s\n' "$$" "$lock_start" > "$lock/owner"
  command -v setsid >/dev/null 2>&1 || {{ echo 'SSH direct Runs require setsid' >&2; exit 69; }}
  command -v timeout >/dev/null 2>&1 || {{ echo 'SSH direct Runs require timeout' >&2; exit 69; }}
  command -v bash >/dev/null 2>&1 || {{ echo 'SSH direct Runs require bash' >&2; exit 69; }}
  nohup setsid sh "$workdir/supervisor.sh" </dev/null >/dev/null 2>&1 &
fi
if [ ! -f "$workdir/_submitted" ]; then
  i=0
  while [ ! -f "$workdir/_submitted" ] && [ "$i" -lt 10 ]; do
    sleep 1
    i=$((i + 1))
  done
fi
[ -f "$workdir/_submitted" ] || {{ echo 'remote supervisor did not acknowledge launch' >&2; exit 70; }}
printf '__WISP_HANDLE__:'
cat "$workdir/_submitted"
"#,
    )
}

pub(super) async fn launch_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemoteRunHandle, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH launch",
        runner
            .run(
                ssh_script_command(connection, "launch SSH Run", launch_payload(handle))?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    handle_from_ack(handle, &output.stdout)
}

pub(super) async fn ensure_remote_started(
    store: &wisp_store::Store,
    owner_id: &str,
    runner: &dyn RunCommandRunner,
    remote: &mut RemoteRun,
) -> Result<RemoteRunHandle, String> {
    if remote.handle.is_confirmed() {
        return Ok(remote.handle.clone());
    }
    match prepare_remote(runner, remote).await? {
        PrepareRemote::Existing(handle) => {
            remote.handle = handle.clone();
            Ok(handle)
        }
        PrepareRemote::Prepared => {
            if !remote.input_refs.is_empty() && !remote.handle.inputs_staged() {
                stage_remote_inputs(store, owner_id, runner, remote).await?;
                remote.handle.mark_inputs_staged();
                let handle_json =
                    serde_json::to_string(&remote.handle).map_err(|e| e.to_string())?;
                if !store
                    .set_run_remote_handle_owned(
                        &remote.run_id,
                        owner_id,
                        &handle_json,
                        &remote.handle.display_workdir(),
                    )
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Err("SSH lifecycle lease expired after input staging".into());
                }
            }
            let run = store
                .get_run(&remote.run_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Run not found: {}", remote.run_id))?;
            if run.status == wisp_store::RunStatus::Cancelling {
                return Err("SSH Run was cancelled before launch".into());
            }
            if !store
                .renew_run_lifecycle(&remote.run_id, owner_id, REMOTE_START_LEASE_SECS)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("SSH lifecycle lease expired before launch".into());
            }
            launch_remote(runner, &remote.handle).await
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RemotePollState {
    Running,
    Finished(i64),
    TimedOut(i64),
    Cancelled,
    Lost(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemotePoll {
    pub(super) state: RemotePollState,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) fn poll_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let (_, workdir, token, pgid, start_time) = remote_parts(handle);
    let (pgid, start_time) = pgid
        .zip(start_time)
        .ok_or_else(|| "SSH Run handle has not been confirmed".to_string())?;
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
state='lost:control directory missing'
same_identity() {{
  current=$(awk '{{print $22}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  group=$(awk '{{print $5}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  [ "$current" = "{start_time}" ] && [ "$group" = "{pgid}" ] && kill -0 "-{pgid}" 2>/dev/null
}}
read_status() {{
  status=$(cat "$workdir/_status" 2>/dev/null || true)
  case "$status" in
    done:*) state="finished:${{status#done:}}"; return 0 ;;
    timed_out:*) state="$status"; return 0 ;;
    cancelled) state='cancelled'; return 0 ;;
    lost:*) state="$status"; return 0 ;;
  esac
  return 1
}}
if [ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ]; then
  if ! read_status; then
    if same_identity; then
      state='running'
    else
      # A supervisor writes _status immediately after its child exits. Re-read
      # once before declaring the process lost at that boundary.
      sleep 1
      if read_status; then
        :
      elif same_identity; then
        state='running'
      else
        state='lost:remote process handle no longer exists'
      fi
    fi
  fi
fi
printf '__WISP_RUN_STATUS__:%s\n' "$state"
printf '__WISP_STDOUT__\n'
tail -c 4000 "$workdir/stdout.log" 2>/dev/null || true
printf '\n__WISP_STDERR__\n'
tail -c 4000 "$workdir/stderr.log" 2>/dev/null || true
"#,
    ))
}

pub(super) fn parse_remote_poll(stdout: &str) -> Result<RemotePoll, String> {
    const STATUS: &str = "__WISP_RUN_STATUS__:";
    const STDOUT: &str = "__WISP_STDOUT__\n";
    const STDERR: &str = "\n__WISP_STDERR__\n";
    let start = stdout
        .find(STATUS)
        .ok_or_else(|| "SSH poll response omitted status".to_string())?;
    let after = &stdout[start + STATUS.len()..];
    let (status, body) = after
        .split_once('\n')
        .ok_or_else(|| "SSH poll response has a malformed status".to_string())?;
    let body = body
        .strip_prefix(STDOUT)
        .ok_or_else(|| "SSH poll response omitted stdout marker".to_string())?;
    let (stdout_tail, stderr_tail) = body
        .split_once(STDERR)
        .ok_or_else(|| "SSH poll response omitted stderr marker".to_string())?;
    let state = if status == "running" {
        RemotePollState::Running
    } else if status == "cancelled" {
        RemotePollState::Cancelled
    } else if let Some(code) = status.strip_prefix("finished:") {
        RemotePollState::Finished(
            code.parse::<i64>()
                .map_err(|_| "SSH poll returned an invalid exit code".to_string())?,
        )
    } else if let Some(code) = status.strip_prefix("timed_out:") {
        RemotePollState::TimedOut(
            code.parse::<i64>()
                .map_err(|_| "SSH poll returned an invalid timeout code".to_string())?,
        )
    } else if let Some(reason) = status.strip_prefix("lost:") {
        RemotePollState::Lost(reason.into())
    } else {
        return Err(format!("SSH poll returned unknown state: {status}"));
    };
    Ok(RemotePoll {
        state,
        stdout: stdout_tail.trim_end_matches('\n').into(),
        stderr: stderr_tail.trim_end_matches('\n').into(),
    })
}

pub(super) async fn poll_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemotePoll, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH poll",
        runner
            .run(
                ssh_script_command(connection, "poll SSH Run", poll_payload(handle)?)?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    parse_remote_poll(&output.stdout)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RemoteCancel {
    Cancelled,
    Finished(i64),
    TimedOut(i64),
    Lost(String),
}

pub(super) fn cancel_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let (_, workdir, token, pgid, start_time) = remote_parts(handle);
    let (pgid, start_time) = pgid
        .zip(start_time)
        .ok_or_else(|| "SSH Run handle has not been confirmed".to_string())?;
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
same_identity() {{
  current=$(awk '{{print $22}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  group=$(awk '{{print $5}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  [ "$current" = "{start_time}" ] && [ "$group" = "{pgid}" ] && kill -0 "-{pgid}" 2>/dev/null
}}
terminal_status() {{
  status=$(cat "$workdir/_status" 2>/dev/null || true)
  case "$status" in
    done:*) printf '__WISP_CANCEL__:finished:%s\n' "${{status#done:}}"; return 0 ;;
    timed_out:*) printf '__WISP_CANCEL__:timed_out:%s\n' "${{status#timed_out:}}"; return 0 ;;
    cancelled) printf '__WISP_CANCEL__:cancelled\n'; return 0 ;;
  esac
  return 1
}}
if [ ! -f "$workdir/token" ] || [ "$(cat "$workdir/token")" != "{token}" ]; then
  printf '__WISP_CANCEL__:lost:token mismatch\n'
  exit 0
fi
terminal_status && exit 0 || true
if ! same_identity; then
  sleep 1
  terminal_status && exit 0 || true
  printf '__WISP_CANCEL__:retry:process identity changed\n'
  exit 0
fi
if ! kill -TERM "-{pgid}" 2>/dev/null; then
  printf '__WISP_CANCEL__:retry:TERM was not confirmed\n'
  exit 0
fi
tmp="$workdir/_cancel_requested.tmp.$$"
printf 'requested\n' > "$tmp" && mv "$tmp" "$workdir/_cancel_requested"
i=0
while [ "$i" -lt 10 ]; do
  terminal_status && exit 0 || true
  kill -0 "-{pgid}" 2>/dev/null || break
  sleep 1
  i=$((i + 1))
done
if same_identity; then
  kill -KILL "-{pgid}" 2>/dev/null || true
fi
i=0
while kill -0 "-{pgid}" 2>/dev/null && [ "$i" -lt 5 ]; do
  sleep 1
  i=$((i + 1))
done
terminal_status && exit 0 || true
if kill -0 "-{pgid}" 2>/dev/null; then
  printf '__WISP_CANCEL__:retry:process group survived cancellation\n'
  exit 0
fi
tmp="$workdir/_status.tmp.$$"
printf 'cancelled\n' > "$tmp" && mv "$tmp" "$workdir/_status"
printf '__WISP_CANCEL__:cancelled\n'
"#,
    ))
}

pub(super) fn parse_remote_cancel(stdout: &str) -> Result<RemoteCancel, String> {
    const PREFIX: &str = "__WISP_CANCEL__:";
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix(PREFIX))
        .ok_or_else(|| "SSH cancel response omitted status".to_string())?;
    if value == "cancelled" {
        Ok(RemoteCancel::Cancelled)
    } else if let Some(code) = value.strip_prefix("finished:") {
        Ok(RemoteCancel::Finished(code.parse::<i64>().map_err(
            |_| "SSH cancel returned an invalid exit code".to_string(),
        )?))
    } else if let Some(code) = value.strip_prefix("timed_out:") {
        Ok(RemoteCancel::TimedOut(code.parse::<i64>().map_err(
            |_| "SSH cancel returned an invalid timeout code".to_string(),
        )?))
    } else if let Some(reason) = value.strip_prefix("lost:") {
        Ok(RemoteCancel::Lost(reason.into()))
    } else {
        Err(format!("SSH cancel returned unknown state: {value}"))
    }
}

pub(super) async fn cancel_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemoteCancel, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH cancel",
        runner
            .run(
                ssh_script_command(connection, "cancel SSH Run", cancel_payload(handle)?)?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    parse_remote_cancel(&output.stdout)
}

pub(super) fn remote_terminal_status(exit_code: i64) -> wisp_store::RunStatus {
    match exit_code {
        0 => wisp_store::RunStatus::Succeeded,
        _ => wisp_store::RunStatus::Failed,
    }
}

pub(super) fn remote_poll_delay_secs(consecutive_transport_errors: u32) -> u64 {
    match consecutive_transport_errors {
        0 | 1 => 5,
        2 => 10,
        _ => 20,
    }
}

pub(super) fn remote_poll_interval(consecutive_transport_errors: u32) -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(remote_poll_delay_secs(consecutive_transport_errors))
    }
}

pub(super) fn permanent_remote_start_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "requires setsid",
        "requires timeout",
        "requires bash",
        "process group did not start",
        "permission denied",
        "too many authentication failures",
        "host key verification failed",
        "could not resolve hostname",
        "could not resolve host",
        "name or service not known",
        "no such identity",
        "identity file is not accessible",
        "bad configuration option",
        "connection reset",
        "connection closed",
        "connection timed out",
        "connect timed out",
        "no route to host",
        "network is unreachable",
        "kex_exchange_identification",
        "ssh connectivity gate blocked",
    ]
    .iter()
    .any(|message| error.contains(message))
}
