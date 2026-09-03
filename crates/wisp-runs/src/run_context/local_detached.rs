use super::remote::command_delimiter;
use super::{LocalTransport, RemoteRun, RemoteRunHandle, RunCommand};
use base64::Engine as _;
use std::sync::OnceLock;

/// Prefer PowerShell 7 (`pwsh`) when present so local Runs match modern Windows
/// shells; fall back to Windows PowerShell 5.1 which ships with the OS.
pub(super) fn windows_powershell_program() -> &'static str {
    static PROGRAM: OnceLock<&'static str> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        let Some(path) = std::env::var_os("PATH") else {
            return "powershell";
        };
        for dir in std::env::split_paths(&path) {
            if dir.join("pwsh.exe").is_file() || dir.join("pwsh").is_file() {
                return "pwsh";
            }
        }
        "powershell"
    })
}

pub(super) fn transport_script_command(
    handle: &RemoteRunHandle,
    label: &str,
    payload: String,
) -> Result<RunCommand, String> {
    match handle {
        RemoteRunHandle::LocalDetached { transport, .. } => match transport {
            LocalTransport::Posix {
                context_id,
                program,
                args,
            } => Ok(RunCommand {
                context_id: context_id.clone(),
                program: program.clone(),
                args: args.clone(),
                script: label.into(),
                cwd: None,
                stdin: Some(payload),
                envs: Vec::new(),
            }),
            // `-Command -` parses stdin line-by-line like an interactive
            // session in Windows PowerShell 5.1; read the whole payload and
            // execute it as one script instead. The same form works in pwsh.
            LocalTransport::Windows { context_id } => Ok(RunCommand {
                context_id: context_id.clone(),
                program: windows_powershell_program().into(),
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "[Console]::In.ReadToEnd() | Invoke-Expression".into(),
                ],
                script: label.into(),
                cwd: None,
                stdin: Some(payload),
                envs: Vec::new(),
            }),
        },
        RemoteRunHandle::SshDirect { .. } => {
            Err("local transport helper called for an SSH handle".into())
        }
    }
}

pub(super) fn rpc_action_label(handle: &RemoteRunHandle, action: &str) -> String {
    match handle {
        RemoteRunHandle::LocalDetached {
            transport: LocalTransport::Posix { context_id, .. },
            ..
        } if context_id.starts_with("wsl:") => format!("{action} WSL Run"),
        RemoteRunHandle::LocalDetached { .. } => format!("{action} local Run"),
        RemoteRunHandle::SshDirect { .. } => format!("{action} SSH Run"),
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn b64(value: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}

pub(super) fn posix_prepare_payload(remote: &RemoteRun) -> String {
    let RemoteRunHandle::LocalDetached {
        transport,
        workdir,
        token,
        command_cwd,
        ..
    } = &remote.handle
    else {
        unreachable!("posix prepare requires LocalDetached");
    };
    let is_wsl = matches!(
        transport,
        LocalTransport::Posix { context_id, .. } if context_id.starts_with("wsl:")
    );
    let delimiter = command_delimiter(token, &remote.command);
    let cd_line = match command_cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
        // WSL stores the Windows project root; translate it inside the distro
        // so project-relative commands and output harvest keep working.
        Some(cwd) if is_wsl => format!(
            "if command -v wslpath >/dev/null 2>&1; then\n  cd \"$(wslpath {})\" || exit 125\nfi\n",
            shell_single_quote(cwd)
        ),
        Some(cwd) => format!("cd {} || exit 125\n", shell_single_quote(cwd)),
        None => String::new(),
    };
    format!(
        r#"set -eu
umask 077
workdir="$HOME/{workdir}"
mkdir -p "$workdir"
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
{cd_line}{command}
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
process_start() {{
  pid=$1
  if [ -r "/proc/$pid/stat" ]; then
    awk '{{print $22}}' "/proc/$pid/stat" 2>/dev/null || true
  else
    ps -p "$pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' || true
  fi
}}
stop_tree() {{
  signal=$1
  pid=$2
  kill "-$signal" "-$pid" 2>/dev/null || true
  kill "-$signal" "$pid" 2>/dev/null || true
}}
if [ -f _submitted ]; then
  # A supervisor already ran for this workdir; never launch the command twice.
  exit 0
fi
if ! command -v bash >/dev/null 2>&1; then
  write_state _status 'lost:local detached Run requires bash'
  exit 69
fi
if ! command -v nohup >/dev/null 2>&1; then
  write_state _status 'lost:local detached Run requires nohup'
  exit 69
fi
rm -f _command_exit _cancel_requested
# Prefer setsid so the command owns an independent process group. macOS lacks
# setsid; enable job control there so the background job still gets its own
# process group and group kill reaches the whole command tree.
if command -v setsid >/dev/null 2>&1; then
  setsid sh -c 'bash -l "$1"; rc=$?; tmp="$2.tmp.$$"; printf "%s\n" "$rc" > "$tmp" && mv "$tmp" "$2"; exit "$rc"' sh "$PWD/command.sh" "$PWD/_command_exit" >stdout.log 2>stderr.log &
else
  set -m 2>/dev/null || true
  (
    bash -l "$PWD/command.sh"
    rc=$?
    tmp="$PWD/_command_exit.tmp.$$"
    printf '%s\n' "$rc" > "$tmp" && mv "$tmp" "$PWD/_command_exit"
    exit "$rc"
  ) >stdout.log 2>stderr.log &
  set +m 2>/dev/null || true
fi
pgid=$!
i=0
start_identity=''
while [ "$i" -lt 5 ]; do
  start_identity=$(process_start "$pgid")
  if [ -n "$start_identity" ] && kill -0 "$pgid" 2>/dev/null; then
    break
  fi
  # Fast commands can exit before the first identity sample; accept a completed
  # command that already wrote _command_exit.
  if [ -f _command_exit ]; then
    start_identity=$(printf 'exited:%s' "$(cat _command_exit 2>/dev/null || printf 0)")
    break
  fi
  sleep 1
  i=$((i + 1))
done
if [ -z "$start_identity" ]; then
  write_state _status 'lost:command process group did not start'
  exit 69
fi
write_state _submitted '{token}:'"$pgid:$start_identity"
if [ -f _command_exit ]; then
  command_rc=$(cat _command_exit 2>/dev/null || printf 0)
  write_state _status "done:$command_rc"
  exit "$command_rc"
fi
write_state _status running
(
  sleep {timeout_secs}
  if kill -0 "$pgid" 2>/dev/null || kill -0 "-$pgid" 2>/dev/null; then
    write_state _status 'timed_out:124'
    stop_tree TERM "$pgid"
    sleep 10
    stop_tree KILL "$pgid"
  fi
) &
watchdog_pid=$!
wait "$pgid"
rc=$?
kill "$watchdog_pid" 2>/dev/null || true
wait "$watchdog_pid" 2>/dev/null || true
if [ -f _cancel_requested ]; then
  write_state _status cancelled
elif [ -f _command_exit ]; then
  command_rc=$(cat _command_exit 2>/dev/null || printf '%s' "$rc")
  write_state _status "done:$command_rc"
elif grep -q '^timed_out:' _status 2>/dev/null; then
  :
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
        cd_line = cd_line,
        workdir = workdir,
        token = token,
        delimiter = delimiter,
    )
}

pub(super) fn posix_launch_payload(handle: &RemoteRunHandle) -> String {
    let RemoteRunHandle::LocalDetached { workdir, token, .. } = handle else {
        unreachable!("posix launch requires LocalDetached");
    };
    format!(
        r#"set -eu
workdir="$HOME/{workdir}"
[ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
process_start() {{
  pid=$1
  if [ -r "/proc/$pid/stat" ]; then
    awk '{{print $22}}' "/proc/$pid/stat" 2>/dev/null || true
  else
    ps -p "$pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' || true
  fi
}}
lock="$workdir/_launch_lock"
if [ -d "$lock" ] && [ ! -f "$workdir/_submitted" ]; then
  owner=$(cat "$lock/owner" 2>/dev/null || true)
  lock_pid=${{owner%%:*}}
  lock_start=${{owner#*:}}
  current=$(process_start "$lock_pid")
  if [ -z "$lock_pid" ] || [ "$current" != "$lock_start" ]; then
    rm -f "$lock/owner"
    rmdir "$lock" 2>/dev/null || true
  fi
fi
if [ ! -f "$workdir/_submitted" ] && mkdir "$lock" 2>/dev/null; then
  trap 'rm -f "$lock/owner"; rmdir "$lock" 2>/dev/null || true' EXIT HUP INT TERM
  lock_start=$(process_start "$$")
  printf '%s:%s\n' "$$" "$lock_start" > "$lock/owner"
  command -v nohup >/dev/null 2>&1 || {{ echo 'local detached Runs require nohup' >&2; exit 69; }}
  command -v bash >/dev/null 2>&1 || {{ echo 'local detached Runs require bash' >&2; exit 69; }}
  # Detach the supervisor into its own session when possible so signals sent
  # to the app's process group cannot kill it while the command keeps running.
  if command -v setsid >/dev/null 2>&1; then
    nohup setsid sh "$workdir/supervisor.sh" </dev/null >/dev/null 2>&1 &
  else
    nohup sh "$workdir/supervisor.sh" </dev/null >/dev/null 2>&1 &
  fi
fi
if [ ! -f "$workdir/_submitted" ]; then
  i=0
  while [ ! -f "$workdir/_submitted" ] && [ "$i" -lt 10 ]; do
    sleep 1
    i=$((i + 1))
  done
fi
[ -f "$workdir/_submitted" ] || {{ echo 'local supervisor did not acknowledge launch' >&2; exit 70; }}
printf '__WISP_HANDLE__:'
cat "$workdir/_submitted"
"#,
    )
}

pub(super) fn posix_poll_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let RemoteRunHandle::LocalDetached {
        workdir,
        token,
        pgid,
        start_identity,
        ..
    } = handle
    else {
        return Err("posix poll requires LocalDetached".into());
    };
    let pgid = pgid.ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity = start_identity
        .as_deref()
        .ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity_q = shell_single_quote(start_identity);
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
state='lost:control directory missing'
process_start() {{
  pid=$1
  if [ -r "/proc/$pid/stat" ]; then
    awk '{{print $22}}' "/proc/$pid/stat" 2>/dev/null || true
  else
    ps -p "$pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' || true
  fi
}}
same_identity() {{
  case {start_identity_q} in
    exited:*) return 1 ;;
  esac
  current=$(process_start "{pgid}")
  [ -n "$current" ] && [ "$current" = {start_identity_q} ] && {{
    kill -0 "{pgid}" 2>/dev/null || kill -0 "-{pgid}" 2>/dev/null
  }}
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
      sleep 1
      if read_status; then
        :
      elif same_identity; then
        state='running'
      else
        state='lost:local process handle no longer exists'
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

pub(super) fn posix_cancel_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let RemoteRunHandle::LocalDetached {
        workdir,
        token,
        pgid,
        start_identity,
        ..
    } = handle
    else {
        return Err("posix cancel requires LocalDetached".into());
    };
    let pgid = pgid.ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity = start_identity
        .as_deref()
        .ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity_q = shell_single_quote(start_identity);
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
process_start() {{
  pid=$1
  if [ -r "/proc/$pid/stat" ]; then
    awk '{{print $22}}' "/proc/$pid/stat" 2>/dev/null || true
  else
    ps -p "$pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' || true
  fi
}}
same_identity() {{
  case {start_identity_q} in
    exited:*) return 1 ;;
  esac
  current=$(process_start "{pgid}")
  [ -n "$current" ] && [ "$current" = {start_identity_q} ] && {{
    kill -0 "{pgid}" 2>/dev/null || kill -0 "-{pgid}" 2>/dev/null
  }}
}}
stop_tree() {{
  signal=$1
  kill "-$signal" "-{pgid}" 2>/dev/null || true
  kill "-$signal" "{pgid}" 2>/dev/null || true
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
stop_tree TERM
tmp="$workdir/_cancel_requested.tmp.$$"
printf 'requested\n' > "$tmp" && mv "$tmp" "$workdir/_cancel_requested"
i=0
while [ "$i" -lt 10 ]; do
  terminal_status && exit 0 || true
  same_identity || break
  sleep 1
  i=$((i + 1))
done
if same_identity; then
  stop_tree KILL
fi
i=0
while same_identity && [ "$i" -lt 5 ]; do
  sleep 1
  i=$((i + 1))
done
terminal_status && exit 0 || true
if same_identity; then
  printf '__WISP_CANCEL__:retry:process group survived cancellation\n'
  exit 0
fi
tmp="$workdir/_status.tmp.$$"
printf 'cancelled\n' > "$tmp" && mv "$tmp" "$workdir/_status"
printf '__WISP_CANCEL__:cancelled\n'
"#,
    ))
}

fn windows_supervisor_script(
    workdir_win: &str,
    token: &str,
    timeout_secs: u64,
    command_cwd: Option<&str>,
) -> String {
    let cwd_assign = match command_cwd.filter(|cwd| !cwd.is_empty()) {
        Some(cwd) => format!(
            "$workingDirectory = {}",
            powershell_single_quote(&cwd.replace('/', "\\"))
        ),
        None => "$workingDirectory = $null".into(),
    };
    format!(
        r#"$ErrorActionPreference = 'Continue'
$workdir = Join-Path $env:USERPROFILE '{workdir_win}'
Set-Location -LiteralPath $workdir
{cwd_assign}
function Write-State([string]$Path, [string]$Value) {{
  $tmp = $Path + '.tmp.' + $PID
  Set-Content -LiteralPath $tmp -Value $Value -Encoding ascii
  Move-Item -LiteralPath $tmp -Destination $Path -Force
}}
if (Test-Path -LiteralPath (Join-Path $workdir '_submitted')) {{
  exit 0
}}
Remove-Item -LiteralPath (Join-Path $workdir '_command_exit') -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $workdir '_cancel_requested') -ErrorAction SilentlyContinue
$stdout = Join-Path $workdir 'stdout.log'
$stderr = Join-Path $workdir 'stderr.log'
$commandPath = Join-Path $workdir 'command.ps1'
# Prefer the same engine running this supervisor (pwsh or Windows PowerShell).
# Start-Process -PassThru with RedirectStandard* returns a null ExitCode on
# Windows PowerShell 5.1; System.Diagnostics.Process is reliable on 5.1 and 7.
$shell = $null
try {{ $shell = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName }} catch {{ }}
if (-not $shell) {{
  $shell = Join-Path $PSHOME $(if ($PSVersionTable.PSEdition -eq 'Core') {{ 'pwsh.exe' }} else {{ 'powershell.exe' }})
}}
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $shell
$psi.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + ($commandPath.Replace('"', '\"')) + '"'
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
if ($null -ne $workingDirectory -and $workingDirectory -ne '') {{
  $psi.WorkingDirectory = $workingDirectory
}}
$proc = New-Object System.Diagnostics.Process
$proc.StartInfo = $psi
$outFs = $null
$errFs = $null
$stdoutTask = $null
$stderrTask = $null
try {{
  if (-not $proc.Start()) {{
    Write-State (Join-Path $workdir '_status') 'lost:command process did not start'
    exit 69
  }}
  $outFs = [System.IO.File]::Create($stdout)
  $errFs = [System.IO.File]::Create($stderr)
  $stdoutTask = $proc.StandardOutput.BaseStream.CopyToAsync($outFs)
  $stderrTask = $proc.StandardError.BaseStream.CopyToAsync($errFs)
}} catch {{
  Write-State (Join-Path $workdir '_status') 'lost:command process did not start'
  exit 69
}}
$identity = $null
for ($i = 0; $i -lt 5; $i++) {{
  try {{
    if ($proc.StartTime) {{
      $identity = [string]$proc.StartTime.ToUniversalTime().Ticks
      break
    }}
  }} catch {{ }}
  Start-Sleep -Seconds 1
}}
if (-not $identity) {{
  try {{ if (-not $proc.HasExited) {{ $proc.Kill() }} }} catch {{ }}
  Write-State (Join-Path $workdir '_status') 'lost:command process did not start'
  exit 69
}}
Write-State (Join-Path $workdir '_submitted') ('{token}:' + $proc.Id + ':' + $identity)
Write-State (Join-Path $workdir '_status') 'running'
$deadline = [datetime]::UtcNow.AddSeconds({timeout_secs})
while (-not $proc.HasExited) {{
  if (Test-Path -LiteralPath (Join-Path $workdir '_cancel_requested')) {{ break }}
  if ([datetime]::UtcNow -ge $deadline) {{
    Write-State (Join-Path $workdir '_status') 'timed_out:124'
    try {{ $proc.Kill() }} catch {{ }}
    try {{ & taskkill.exe /PID $proc.Id /T /F | Out-Null }} catch {{ }}
    break
  }}
  Start-Sleep -Seconds 1
}}
if (-not $proc.HasExited) {{
  try {{ [void]$proc.WaitForExit(15000) }} catch {{ }}
  if (-not $proc.HasExited) {{
    try {{ $proc.Kill() }} catch {{ }}
    try {{ & taskkill.exe /PID $proc.Id /T /F | Out-Null }} catch {{ }}
    try {{ [void]$proc.WaitForExit(5000) }} catch {{ }}
  }}
}}
try {{
  if ($null -ne $stdoutTask) {{ [void]$stdoutTask.Wait(30000) }}
  if ($null -ne $stderrTask) {{ [void]$stderrTask.Wait(30000) }}
}} catch {{ }}
if ($null -ne $outFs) {{ try {{ $outFs.Dispose() }} catch {{ }} }}
if ($null -ne $errFs) {{ try {{ $errFs.Dispose() }} catch {{ }} }}
$rc = 1
try {{
  if ($proc.HasExited) {{ $rc = [int]$proc.ExitCode }}
}} catch {{ }}
Write-State (Join-Path $workdir '_command_exit') ([string]$rc)
if (Test-Path -LiteralPath (Join-Path $workdir '_cancel_requested')) {{
  Write-State (Join-Path $workdir '_status') 'cancelled'
}} elseif ((Test-Path -LiteralPath (Join-Path $workdir '_status')) -and ((Get-Content -LiteralPath (Join-Path $workdir '_status') -Raw) -match '^timed_out:')) {{
}} else {{
  Write-State (Join-Path $workdir '_status') ('done:' + [string]$rc)
}}
exit $rc
"#,
        workdir_win = workdir_win.replace('\'', "''"),
        token = token,
        timeout_secs = timeout_secs,
        cwd_assign = cwd_assign,
    )
}

pub(super) fn windows_prepare_payload(remote: &RemoteRun) -> String {
    let RemoteRunHandle::LocalDetached {
        workdir,
        token,
        command_cwd,
        ..
    } = &remote.handle
    else {
        unreachable!("windows prepare requires LocalDetached");
    };
    let workdir_win = workdir.replace('/', "\\");
    let command_b64 = b64(&remote.command);
    let supervisor_b64 = b64(&windows_supervisor_script(
        &workdir_win,
        token,
        remote.timeout.as_secs(),
        command_cwd.as_deref(),
    ));
    format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $env:USERPROFILE '{workdir_win}'
New-Item -ItemType Directory -Force -Path $workdir | Out-Null
$tokenPath = Join-Path $workdir 'token'
if (Test-Path -LiteralPath $tokenPath) {{
  if ((Get-Content -LiteralPath $tokenPath -Raw).Trim() -ne '{token}') {{
    Write-Error 'wisp token mismatch'
    exit 73
  }}
}} else {{
  Set-Content -LiteralPath ($tokenPath + '.tmp') -Value '{token}' -Encoding ascii
  Move-Item -LiteralPath ($tokenPath + '.tmp') -Destination $tokenPath -Force
}}
$submitted = Join-Path $workdir '_submitted'
if (Test-Path -LiteralPath $submitted) {{
  Write-Output ('__WISP_HANDLE__:' + (Get-Content -LiteralPath $submitted -Raw).Trim())
  exit 0
}}
$commandPath = Join-Path $workdir 'command.ps1'
$supervisorPath = Join-Path $workdir 'supervisor.ps1'
[System.IO.File]::WriteAllText($commandPath, [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{command_b64}')))
[System.IO.File]::WriteAllText($supervisorPath, [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{supervisor_b64}')))
Write-Output '__WISP_PREPARED__'
"#,
        workdir_win = workdir_win.replace('\'', "''"),
        token = token,
        command_b64 = command_b64,
        supervisor_b64 = supervisor_b64,
    )
}

pub(super) fn windows_launch_payload(handle: &RemoteRunHandle) -> String {
    let RemoteRunHandle::LocalDetached { workdir, token, .. } = handle else {
        unreachable!("windows launch requires LocalDetached");
    };
    let workdir_win = workdir.replace('/', "\\");
    format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $env:USERPROFILE '{workdir_win}'
$tokenPath = Join-Path $workdir 'token'
if (-not (Test-Path -LiteralPath $tokenPath) -or ((Get-Content -LiteralPath $tokenPath -Raw).Trim() -ne '{token}')) {{
  Write-Error 'wisp token mismatch'
  exit 73
}}
$submitted = Join-Path $workdir '_submitted'
$lock = Join-Path $workdir '_launch_lock'
$ownerPath = Join-Path $lock 'owner'
if ((Test-Path -LiteralPath $lock) -and -not (Test-Path -LiteralPath $submitted)) {{
  # Only clear the lock when its recorded owner process is gone; a live owner
  # is still mid-launch and must not be raced.
  $ownerAlive = $false
  try {{
    $ownerId = [int]((Get-Content -LiteralPath $ownerPath -Raw -ErrorAction Stop).Trim())
    if ($ownerId -gt 0 -and (Get-Process -Id $ownerId -ErrorAction SilentlyContinue)) {{
      $ownerAlive = $true
    }}
  }} catch {{ }}
  if (-not $ownerAlive) {{
    Remove-Item -LiteralPath $lock -Recurse -Force -ErrorAction SilentlyContinue
  }}
}}
if (-not (Test-Path -LiteralPath $submitted)) {{
  $acquired = $false
  try {{
    New-Item -ItemType Directory -Path $lock -ErrorAction Stop | Out-Null
    $acquired = $true
  }} catch {{ }}
  if ($acquired) {{
    Set-Content -LiteralPath $ownerPath -Value ([string]$PID) -Encoding ascii
    $supervisor = Join-Path $workdir 'supervisor.ps1'
    $supervisorStdout = Join-Path $workdir 'supervisor.stdout.log'
    $supervisorStderr = Join-Path $workdir 'supervisor.stderr.log'
    # Keep the supervisor on the same engine as this launch host (pwsh or
    # Windows PowerShell). -File still needs an explicit process-scope bypass.
    $shell = $null
    try {{ $shell = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName }} catch {{ }}
    if (-not $shell) {{
      $shell = Join-Path $PSHOME $(if ($PSVersionTable.PSEdition -eq 'Core') {{ 'pwsh.exe' }} else {{ 'powershell.exe' }})
    }}
    Start-Process -FilePath $shell -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File', $supervisor) -WindowStyle Hidden -RedirectStandardOutput $supervisorStdout -RedirectStandardError $supervisorStderr | Out-Null
  }}
}}
for ($i = 0; $i -lt 10 -and -not (Test-Path -LiteralPath $submitted); $i++) {{
  Start-Sleep -Seconds 1
}}
if (-not (Test-Path -LiteralPath $submitted)) {{
  $detail = ''
  $statusPath = Join-Path $workdir '_status'
  $supervisorStderr = Join-Path $workdir 'supervisor.stderr.log'
  if (Test-Path -LiteralPath $statusPath) {{
    $detail = (Get-Content -LiteralPath $statusPath -Raw -ErrorAction SilentlyContinue).Trim()
  }}
  if (-not $detail -and (Test-Path -LiteralPath $supervisorStderr)) {{
    $detail = (Get-Content -LiteralPath $supervisorStderr -Raw -ErrorAction SilentlyContinue).Trim()
  }}
  if ($detail) {{ Write-Error ('local supervisor did not acknowledge launch: ' + $detail) }}
  else {{ Write-Error 'local supervisor did not acknowledge launch' }}
  exit 70
}}
Write-Output ('__WISP_HANDLE__:' + (Get-Content -LiteralPath $submitted -Raw).Trim())
"#,
    )
}

pub(super) fn windows_poll_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let RemoteRunHandle::LocalDetached {
        workdir,
        token,
        pgid,
        start_identity,
        ..
    } = handle
    else {
        return Err("windows poll requires LocalDetached".into());
    };
    let pgid = pgid.ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity = start_identity
        .as_deref()
        .ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let workdir_win = workdir.replace('/', "\\");
    let identity_q = powershell_single_quote(start_identity);
    Ok(format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $env:USERPROFILE '{workdir_win}'
$state = 'lost:control directory missing'
function Same-Identity {{
  try {{
    $process = Get-Process -Id {pgid} -ErrorAction Stop
    return ([string]$process.StartTime.ToUniversalTime().Ticks) -eq {identity_q}
  }} catch {{ return $false }}
}}
function Read-Status {{
  $statusPath = Join-Path $workdir '_status'
  if (-not (Test-Path -LiteralPath $statusPath)) {{ return $false }}
  $status = (Get-Content -LiteralPath $statusPath -Raw).Trim()
  if ($status -like 'done:*') {{
    $code = $status.Substring(5).Trim()
    if ([string]::IsNullOrWhiteSpace($code)) {{ $code = '0' }}
    $script:state = 'finished:' + $code
    return $true
  }}
  if ($status -like 'timed_out:*') {{ $script:state = $status; return $true }}
  if ($status -eq 'cancelled') {{ $script:state = 'cancelled'; return $true }}
  if ($status -like 'lost:*') {{ $script:state = $status; return $true }}
  return $false
}}
function Read-Tail([string]$Path) {{
  # The command still holds the log open for writing, so share read+write and
  # only pull the last 4000 bytes instead of loading the whole file.
  if (-not (Test-Path -LiteralPath $Path)) {{ return '' }}
  try {{
    $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    try {{
      $take = [int][Math]::Min($fs.Length, 4000)
      if ($take -le 0) {{ return '' }}
      $fs.Seek(-$take, [System.IO.SeekOrigin]::End) | Out-Null
      $buffer = New-Object byte[] $take
      $read = $fs.Read($buffer, 0, $take)
      return [System.Text.Encoding]::UTF8.GetString($buffer, 0, $read)
    }} finally {{
      $fs.Dispose()
    }}
  }} catch {{ return '' }}
}}
$tokenPath = Join-Path $workdir 'token'
if ((Test-Path -LiteralPath $tokenPath) -and ((Get-Content -LiteralPath $tokenPath -Raw).Trim() -eq '{token}')) {{
  if (-not (Read-Status)) {{
    if (Same-Identity) {{
      $state = 'running'
    }} else {{
      Start-Sleep -Seconds 1
      if (Read-Status) {{
      }} elseif (Same-Identity) {{
        $state = 'running'
      }} else {{
        $state = 'lost:local process handle no longer exists'
      }}
    }}
  }}
}}
Write-Output ('__WISP_RUN_STATUS__:' + $state)
Write-Output '__WISP_STDOUT__'
[Console]::Out.Write((Read-Tail (Join-Path $workdir 'stdout.log')))
Write-Output ''
Write-Output '__WISP_STDERR__'
[Console]::Out.Write((Read-Tail (Join-Path $workdir 'stderr.log')))
"#,
    ))
}

pub(super) fn windows_cancel_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let RemoteRunHandle::LocalDetached {
        workdir,
        token,
        pgid,
        start_identity,
        ..
    } = handle
    else {
        return Err("windows cancel requires LocalDetached".into());
    };
    let pgid = pgid.ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let start_identity = start_identity
        .as_deref()
        .ok_or_else(|| "local Run handle has not been confirmed".to_string())?;
    let workdir_win = workdir.replace('/', "\\");
    let identity_q = powershell_single_quote(start_identity);
    Ok(format!(
        r#"$ErrorActionPreference = 'Stop'
$workdir = Join-Path $env:USERPROFILE '{workdir_win}'
function Same-Identity {{
  try {{
    $process = Get-Process -Id {pgid} -ErrorAction Stop
    return ([string]$process.StartTime.ToUniversalTime().Ticks) -eq {identity_q}
  }} catch {{ return $false }}
}}
function Terminal-Status {{
  $statusPath = Join-Path $workdir '_status'
  if (-not (Test-Path -LiteralPath $statusPath)) {{ return $false }}
  $status = (Get-Content -LiteralPath $statusPath -Raw).Trim()
  if ($status -like 'done:*') {{
    $code = $status.Substring(5).Trim()
    if ([string]::IsNullOrWhiteSpace($code)) {{ $code = '0' }}
    Write-Output ('__WISP_CANCEL__:finished:' + $code)
    return $true
  }}
  if ($status -like 'timed_out:*') {{ Write-Output ('__WISP_CANCEL__:timed_out:' + $status.Substring(10)); return $true }}
  if ($status -eq 'cancelled') {{ Write-Output '__WISP_CANCEL__:cancelled'; return $true }}
  return $false
}}
$tokenPath = Join-Path $workdir 'token'
if (-not (Test-Path -LiteralPath $tokenPath) -or ((Get-Content -LiteralPath $tokenPath -Raw).Trim() -ne '{token}')) {{
  Write-Output '__WISP_CANCEL__:lost:token mismatch'
  exit 0
}}
if (Terminal-Status) {{ exit 0 }}
if (-not (Same-Identity)) {{
  Start-Sleep -Seconds 1
  if (Terminal-Status) {{ exit 0 }}
  Write-Output '__WISP_CANCEL__:retry:process identity changed'
  exit 0
}}
$cancelPath = Join-Path $workdir '_cancel_requested'
Set-Content -LiteralPath ($cancelPath + '.tmp') -Value 'requested' -Encoding ascii
Move-Item -LiteralPath ($cancelPath + '.tmp') -Destination $cancelPath -Force
try {{ Stop-Process -Id {pgid} -Force -ErrorAction SilentlyContinue }} catch {{ }}
try {{ & taskkill.exe /PID {pgid} /T /F | Out-Null }} catch {{ }}
for ($i = 0; $i -lt 10; $i++) {{
  if (Terminal-Status) {{ exit 0 }}
  if (-not (Same-Identity)) {{ break }}
  Start-Sleep -Seconds 1
}}
if (Terminal-Status) {{ exit 0 }}
if (Same-Identity) {{
  Write-Output '__WISP_CANCEL__:retry:process group survived cancellation'
  exit 0
}}
$statusPath = Join-Path $workdir '_status'
Set-Content -LiteralPath ($statusPath + '.tmp') -Value 'cancelled' -Encoding ascii
Move-Item -LiteralPath ($statusPath + '.tmp') -Destination $statusPath -Force
Write-Output '__WISP_CANCEL__:cancelled'
"#,
    ))
}
