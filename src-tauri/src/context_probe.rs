use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

const PROBE_SKILL_NAME: &str = "probe-compute-environment";
const PROBE_SKILL: &str = include_str!("../../skills/probe-compute-environment/SKILL.md");
const PROBE_VALUE_BEGIN: &str = "__WISP_PROBE_VALUE_BEGIN__";
const PROBE_VALUE_END: &str = "__WISP_PROBE_VALUE_END__";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub probe_skill: String,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub hostname: Option<String>,
    pub cpu_count: Option<u32>,
    pub gpu_summary: Option<String>,
    pub scheduler: Option<String>,
    /// Legacy display string retained for existing UI/data compatibility.
    pub python: Option<String>,
    pub python_executable: Option<String>,
    pub python_version: Option<String>,
    pub rscript_executable: Option<String>,
    pub r_version: Option<String>,
    pub r_jsonlite: Option<bool>,
    pub conda: Option<String>,
    pub mamba: Option<String>,
    pub modulecmd: Option<String>,
    pub home: Option<String>,
    pub pwd: Option<String>,
    pub privilege: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCommand {
    pub context_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub script: String,
    pub envs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

pub trait ProbeRunner: Send {
    fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String>;
}

pub(crate) struct ProcessProbeRunner;

impl ProbeRunner for ProcessProbeRunner {
    fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
        if let Some(payload) =
            crate::ssh_master::eligible_payload(&command.program, &command.args, None)
        {
            let ssh_args = command.args[..command.args.len() - 1].to_vec();
            let result = crate::ssh_master::run_blocking(
                &command.context_id,
                ssh_args,
                &command.envs,
                payload,
                // Probes previously ran without any timeout; a hung command
                // must not poison the shared master connection.
                std::time::Duration::from_secs(120),
            )
            .map(|output| ProbeCommandOutput {
                status: output.exit_code as i32,
                stdout: output.stdout,
                stderr: output.stderr,
            });
            crate::ssh_hosts::cleanup_password_auth_env(&command.envs);
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
        crate::ssh_hosts::cleanup_password_auth_env(&command.envs);
        Ok(ProbeCommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

pub fn build_probe_command(
    ctx: &wisp_store::ExecutionContext,
    script: &str,
) -> Result<ProbeCommand, String> {
    let cfg: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap_or_default();
    match ctx.kind {
        wisp_store::ExecutionContextKind::Local => Ok(local_command(&ctx.id, script)),
        wisp_store::ExecutionContextKind::Ssh => {
            let connection = crate::ssh_hosts::SshConnection::from_execution_context(ctx)?;
            let mut args = connection.ssh_args()?;
            args.push(script.into());
            Ok(ProbeCommand {
                context_id: ctx.id.clone(),
                program: "ssh".into(),
                args,
                script: script.into(),
                envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
            })
        }
        wisp_store::ExecutionContextKind::Wsl => {
            let distro = cfg
                .get("distro")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| ctx.id.strip_prefix("wsl:").unwrap_or(&ctx.id));
            Ok(ProbeCommand {
                context_id: ctx.id.clone(),
                program: "wsl.exe".into(),
                args: vec![
                    "-d".into(),
                    distro.into(),
                    "--exec".into(),
                    "sh".into(),
                    "-lc".into(),
                    script.into(),
                ],
                script: script.into(),
                envs: Vec::new(),
            })
        }
    }
}

#[cfg(target_os = "windows")]
fn local_command(context_id: &str, script: &str) -> ProbeCommand {
    ProbeCommand {
        context_id: context_id.into(),
        program: "powershell".into(),
        args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
        script: script.into(),
        envs: Vec::new(),
    }
}

#[cfg(not(target_os = "windows"))]
fn local_command(context_id: &str, script: &str) -> ProbeCommand {
    ProbeCommand {
        context_id: context_id.into(),
        program: "sh".into(),
        args: vec!["-lc".into(), script.into()],
        script: script.into(),
        envs: Vec::new(),
    }
}

pub fn probe_context_with_runner(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
) -> Result<ProbeResult, String> {
    let (specs, mut values) = probe_specs(ctx)?;
    let discovered = if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
        run_bundled_ssh_probe(ctx, runner, &specs)?
    } else {
        run_sequential_probe(ctx, runner, &specs)?
    };
    values.extend(discovered);
    probe_result(values)
}

struct ProbeSpec {
    key: &'static str,
    script: String,
    required: bool,
}

fn probe_specs(
    ctx: &wisp_store::ExecutionContext,
) -> Result<(Vec<ProbeSpec>, HashMap<&'static str, String>), String> {
    let configured_python = configured_interpreter(ctx, "python_executable", "python_path")?;
    let configured_rscript = configured_interpreter(ctx, "rscript_executable", "rscript_path")?;
    let mut values = HashMap::new();
    if let Some(executable) = configured_python.as_ref() {
        values.insert("python_executable", executable.clone());
    }
    if let Some(executable) = configured_rscript.as_ref() {
        values.insert("rscript_executable", executable.clone());
    }

    let mut specs = vec![
        ProbeSpec {
            key: "os",
            script: platform_script(
                ctx,
                "uname -s",
                "[System.Runtime.InteropServices.RuntimeInformation]::OSDescription",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "arch",
            script: platform_script(
                ctx,
                "uname -m",
                "[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "hostname",
            script: platform_script(ctx, "hostname", "$env:COMPUTERNAME").into(),
            required: false,
        },
        ProbeSpec {
            key: "cpu_count",
            script: platform_script(
                ctx,
                "getconf _NPROCESSORS_ONLN",
                "$env:NUMBER_OF_PROCESSORS",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "gpu_summary",
            script: "nvidia-smi -L".into(),
            required: false,
        },
        ProbeSpec {
            key: "scheduler",
            script: platform_script(
                ctx,
                "command -v sbatch || command -v qsub || command -v bsub",
                "Get-Command sbatch,qsub,bsub -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source",
            )
            .into(),
            required: false,
        },
    ];
    if configured_python.is_none() {
        specs.push(ProbeSpec {
            key: "python_executable",
            script: platform_owned(
                ctx,
                posix_python_discovery(),
                format!(
                    "{}; if ($wispPython) {{ $wispPython }}",
                    windows_python_discovery()
                ),
            ),
            required: false,
        });
    }
    specs.push(ProbeSpec {
        key: "python_version",
        script: configured_python.as_deref().map_or_else(
            || {
                platform_owned(
                    ctx,
                    posix_with_discovered("PY", &posix_python_discovery(), "--version 2>&1"),
                    format!(
                        "{}; if ($wispPython) {{ & $wispPython --version 2>&1 }}",
                        windows_python_discovery()
                    ),
                )
            },
            |executable| interpreter_command(ctx, executable, "--version 2>&1"),
        ),
        required: false,
    });
    if configured_rscript.is_none() {
        specs.push(ProbeSpec {
            key: "rscript_executable",
            script: platform_owned(
                ctx,
                posix_rscript_discovery(),
                format!(
                    "{}; if ($wispRscript) {{ $wispRscript }}",
                    windows_rscript_discovery()
                ),
            ),
            required: false,
        });
    }
    specs.extend([
        ProbeSpec {
            key: "r_version",
            script: configured_rscript.as_deref().map_or_else(
                || {
                    platform_owned(
                        ctx,
                        posix_with_discovered("RSCRIPT", &posix_rscript_discovery(), "--version 2>&1"),
                        format!(
                            "{}; if ($wispRscript) {{ & $wispRscript --version 2>&1 }}",
                            windows_rscript_discovery()
                        ),
                    )
                },
                |executable| interpreter_command(ctx, executable, "--version 2>&1"),
            ),
            required: false,
        },
        ProbeSpec {
            key: "r_jsonlite",
            script: configured_rscript.as_deref().map_or_else(
                || {
                    platform_owned(
                        ctx,
                        posix_with_discovered(
                            "RSCRIPT",
                            &posix_rscript_discovery(),
                            "--vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                        ),
                        format!(
                            "{}; if ($wispRscript) {{ & $wispRscript --vanilla -e \"cat(requireNamespace('jsonlite', quietly=TRUE))\" 2>$null }}",
                            windows_rscript_discovery()
                        ),
                    )
                },
                |executable| {
                    interpreter_command(
                        ctx,
                        executable,
                        platform_script(
                            ctx,
                            "--vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                            "--vanilla -e \"cat(requireNamespace('jsonlite', quietly=TRUE))\" 2>$null",
                        ),
                    )
                },
            ),
            required: false,
        },
        ProbeSpec {
            key: "conda",
            script: platform_script(
                ctx,
                "command -v conda",
                "(Get-Command conda -ErrorAction SilentlyContinue).Source",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "mamba",
            script: platform_script(
                ctx,
                "command -v mamba",
                "(Get-Command mamba -ErrorAction SilentlyContinue).Source",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "modulecmd",
            script: platform_script(
                ctx,
                "command -v modulecmd",
                "(Get-Command modulecmd -ErrorAction SilentlyContinue).Source",
            )
            .into(),
            required: false,
        },
        ProbeSpec {
            key: "home",
            script: platform_script(ctx, "printf '%s' \"$HOME\"", "$HOME").into(),
            required: false,
        },
        ProbeSpec {
            key: "pwd",
            script: platform_script(ctx, "pwd", "(Get-Location).Path").into(),
            required: false,
        },
        ProbeSpec {
            key: "privilege",
            script: platform_script(
                ctx,
                "if [ \"$(id -u)\" = 0 ]; then printf root; elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then printf sudo; else printf unprivileged; fi",
                "if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { 'root' } else { 'unprivileged' }",
            )
            .into(),
            required: false,
        },
    ]);
    Ok((specs, values))
}

fn run_sequential_probe(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
    specs: &[ProbeSpec],
) -> Result<HashMap<&'static str, String>, String> {
    let mut values = HashMap::new();
    for spec in specs {
        match run_probe_command(ctx, runner, &spec.script) {
            Ok(Some(value)) => {
                values.insert(spec.key, value);
            }
            Ok(None) if spec.required => {
                return Err(required_probe_no_output(spec, false));
            }
            Err(error) if spec.required => {
                return Err(format!("probe command failed for {}: {error}", spec.script));
            }
            Ok(None) | Err(_) => {}
        }
    }
    Ok(values)
}

fn bundled_probe_script(specs: &[ProbeSpec]) -> String {
    let mut script = String::from("set +e\n");
    for spec in specs {
        script.push_str(&format!(
            "printf '%s\\n' '{PROBE_VALUE_BEGIN}:{}'\n{{\n{}\n}} 2>/dev/null\nprintf '\\n%s\\n' '{PROBE_VALUE_END}:{}'\n",
            spec.key, spec.script, spec.key
        ));
    }
    script.push_str("exit 0\n");
    script
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Ask a known POSIX shell to interpret the probe instead of assuming the
/// account's login shell understands sh syntax (it may be fish, csh, etc.).
fn remote_probe_command(script: &str) -> String {
    format!("sh -c {}", shell_single_quote(script))
}

fn parse_bundled_value(stdout: &str, key: &str) -> Option<String> {
    let stdout = stdout.replace("\r\n", "\n");
    let begin = format!("{PROBE_VALUE_BEGIN}:{key}\n");
    let end = format!("\n{PROBE_VALUE_END}:{key}");
    let (_, value) = stdout.split_once(&begin)?;
    let (value, _) = value.split_once(&end)?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn bundled_probe_protocol_observed(stdout: &str, specs: &[ProbeSpec]) -> bool {
    let stdout = stdout.replace("\r\n", "\n");
    specs.iter().any(|spec| {
        stdout.contains(&format!("{PROBE_VALUE_BEGIN}:{}\n", spec.key))
            && stdout.contains(&format!("\n{PROBE_VALUE_END}:{}", spec.key))
    })
}

fn run_bundled_ssh_probe(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
    specs: &[ProbeSpec],
) -> Result<HashMap<&'static str, String>, String> {
    let connection = crate::ssh_hosts::SshConnection::from_execution_context(ctx)?;
    connection.assert_ready_to_connect()?;
    let script = bundled_probe_script(specs);
    let command = build_probe_command(ctx, &remote_probe_command(&script))?;
    let output = runner.run(&command)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status != 0 {
        let detail = if output.stderr.trim().is_empty() {
            stdout.trim()
        } else {
            output.stderr.trim()
        };
        return Err(format_ssh_probe_failure(&connection, output.status, detail));
    }
    if !bundled_probe_protocol_observed(&stdout, specs) {
        return Err(
            "SSH authentication succeeded, but the remote account did not execute Wisp's non-interactive probe commands. Check for a restricted shell, forced command, or a login startup script that exits early."
                .into(),
        );
    }
    let mut values = HashMap::new();
    for spec in specs {
        match parse_bundled_value(&stdout, spec.key) {
            Some(value) => {
                values.insert(spec.key, value);
            }
            None if spec.required => {
                return Err(required_probe_no_output(spec, true));
            }
            None => {}
        }
    }
    Ok(values)
}

fn format_ssh_probe_failure(
    connection: &crate::ssh_hosts::SshConnection,
    status: i32,
    detail: &str,
) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("authentication failed") {
        return match connection.auth_method {
            crate::ssh_hosts::SshAuthMethod::Password => format!(
                "SSH password authentication failed for `{}`: the server rejected the saved password. Check the password, user name, and whether the server allows password login. OpenSSH: {detail}",
                connection.alias
            ),
            crate::ssh_hosts::SshAuthMethod::Key => format!(
                "SSH key authentication failed for `{}`: the server rejected the configured key or agent identity. Check the user name, IdentityFile, and authorized_keys. OpenSSH: {detail}",
                connection.alias
            ),
        };
    }
    format!("SSH probe failed with exit {status}: {detail}")
}

fn required_probe_no_output(spec: &ProbeSpec, ssh_connected: bool) -> String {
    let field = match spec.key {
        "os" => "operating system information",
        "arch" => "CPU architecture",
        "hostname" => "host name",
        _ => "a required environment field",
    };
    if ssh_connected {
        format!(
            "SSH connection succeeded, but the environment probe could not read {field} (`{}` returned no output). The account may block non-interactive POSIX shell commands.",
            spec.script
        )
    } else {
        format!(
            "Environment probe could not read {field}: `{}` returned no output.",
            spec.script
        )
    }
}

fn probe_result(values: HashMap<&'static str, String>) -> Result<ProbeResult, String> {
    let optional = |key: &'static str| values.get(key).cloned();
    let python_version = optional("python_version");
    Ok(ProbeResult {
        probe_skill: PROBE_SKILL_NAME.into(),
        os: optional("os"),
        arch: optional("arch"),
        hostname: optional("hostname"),
        cpu_count: optional("cpu_count").and_then(|value| value.parse::<u32>().ok()),
        gpu_summary: optional("gpu_summary"),
        scheduler: optional("scheduler").and_then(|value| scheduler_from_command(&value)),
        python: python_version.clone(),
        python_executable: optional("python_executable"),
        python_version,
        rscript_executable: optional("rscript_executable"),
        r_version: optional("r_version"),
        r_jsonlite: optional("r_jsonlite").map(|value| value.eq_ignore_ascii_case("true")),
        conda: optional("conda"),
        mamba: optional("mamba"),
        modulecmd: optional("modulecmd"),
        home: optional("home"),
        pwd: optional("pwd"),
        privilege: optional("privilege"),
    })
}

fn platform_script<'a>(
    ctx: &wisp_store::ExecutionContext,
    posix: &'a str,
    windows: &'a str,
) -> &'a str {
    if cfg!(target_os = "windows") && ctx.kind == wisp_store::ExecutionContextKind::Local {
        windows
    } else {
        posix
    }
}

fn platform_owned(ctx: &wisp_store::ExecutionContext, posix: String, windows: String) -> String {
    if cfg!(target_os = "windows") && ctx.kind == wisp_store::ExecutionContextKind::Local {
        windows
    } else {
        posix
    }
}

/// Interpreter lookup order is: explicit configuration (handled by the caller),
/// PATH, then well-known install locations — so an interpreter installed
/// outside PATH (e.g. `D:\R-4.5.2\bin\Rscript.exe` or a conda base env) is
/// still discovered instead of being reported as missing (issue #651).
fn posix_rscript_discovery() -> String {
    "command -v Rscript || for p in /usr/local/bin/Rscript /opt/homebrew/bin/Rscript /usr/bin/Rscript \"$HOME/miniconda3/bin/Rscript\" \"$HOME/anaconda3/bin/Rscript\" \"$HOME/miniforge3/bin/Rscript\" \"$HOME/mambaforge/bin/Rscript\" /opt/conda/bin/Rscript; do [ -x \"$p\" ] && { printf '%s\\n' \"$p\"; break; }; done".into()
}

fn posix_python_discovery() -> String {
    "command -v python3 || command -v python || for p in \"$HOME/miniconda3/bin/python3\" \"$HOME/anaconda3/bin/python3\" \"$HOME/miniforge3/bin/python3\" \"$HOME/mambaforge/bin/python3\" /opt/conda/bin/python3 /usr/local/bin/python3; do [ -x \"$p\" ] && { printf '%s\\n' \"$p\"; break; }; done".into()
}

/// Run `args` with the interpreter resolved by `discovery` (PATH + common
/// install dirs), printing nothing when no interpreter exists.
fn posix_with_discovered(var: &str, discovery: &str, args: &str) -> String {
    format!("{var}=$({discovery}); if [ -n \"${var}\" ]; then \"${var}\" {args}; fi")
}

fn windows_rscript_discovery() -> String {
    // Newest C:\Program Files\R\R-* install wins when PATH lookup fails.
    "$wispRscript = (Get-Command Rscript -ErrorAction SilentlyContinue).Source; if (-not $wispRscript) { $wispRscript = Get-ChildItem (Join-Path $env:ProgramFiles 'R') -Directory -ErrorAction SilentlyContinue | Sort-Object { try { [version]($_.Name -replace '^R-', '') } catch { [version]'0.0.0' } } -Descending | ForEach-Object { Join-Path $_.FullName 'bin\\Rscript.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1 }".into()
}

fn windows_python_discovery() -> String {
    // Per-user installs live in %LOCALAPPDATA%\Programs\Python\PythonXY(Z).
    "$wispPython = (Get-Command python -ErrorAction SilentlyContinue).Source; if (-not $wispPython) { $wispPython = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Programs\\Python') -Directory -ErrorAction SilentlyContinue | Sort-Object { try { [int]($_.Name -replace '[^0-9]', '') } catch { 0 } } -Descending | ForEach-Object { Join-Path $_.FullName 'python.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1 }".into()
}

fn configured_interpreter(
    ctx: &wisp_store::ExecutionContext,
    key: &str,
    legacy_key: &str,
) -> Result<Option<String>, String> {
    let config: serde_json::Value =
        serde_json::from_str(&ctx.config_json).map_err(|error| error.to_string())?;
    let object = config
        .as_object()
        .ok_or_else(|| "execution context config must be a JSON object".to_string())?;
    for name in [key, legacy_key] {
        match object.get(name) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(value)) if value.trim().is_empty() => {}
            Some(serde_json::Value::String(value)) if !value.contains(['\0', '\n', '\r']) => {
                return Ok(Some(value.trim().to_string()));
            }
            Some(serde_json::Value::String(_)) => {
                return Err(format!(
                    "execution context field '{name}' contains a line break"
                ));
            }
            Some(_) => {
                return Err(format!("execution context field '{name}' must be a string"));
            }
        }
    }
    Ok(None)
}

fn interpreter_command(
    ctx: &wisp_store::ExecutionContext,
    executable: &str,
    arguments: &str,
) -> String {
    if cfg!(target_os = "windows") && ctx.kind == wisp_store::ExecutionContextKind::Local {
        format!("& '{}' {arguments}", executable.replace('\'', "''"))
    } else {
        format!("'{}' {arguments}", executable.replace('\'', "'\"'\"'"))
    }
}

fn run_probe_command(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
    script: &str,
) -> Result<Option<String>, String> {
    let command = build_probe_command(ctx, script)?;
    let output = runner.run(&command)?;
    if output.status != 0 {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value = stdout.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.into()))
    }
}

fn scheduler_from_command(path: &str) -> Option<String> {
    if path.contains("sbatch") {
        Some("slurm".into())
    } else if path.contains("qsub") {
        Some("pbs".into())
    } else if path.contains("bsub") {
        Some("lsf".into())
    } else {
        None
    }
}

pub async fn probe_and_store_with_runner(
    store: &wisp_store::Store,
    context_id: &str,
    runner: &mut dyn ProbeRunner,
) -> Result<wisp_store::ExecutionContext, String> {
    let mut ctx = store
        .get_execution_context(context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    // User-driven probe is an intentional reconnect: clear any AI-loop cooldown.
    if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
        crate::ssh_guard::clear(context_id);
    }
    let now = chrono::Utc::now().timestamp();
    match probe_context_with_runner(&ctx, runner) {
        Ok(probe) => {
            ctx.capabilities_json = serde_json::to_string(&probe).map_err(|e| e.to_string())?;
            ctx.last_probe_status = Some("ok".into());
            ctx.last_probe_error = None;
            if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
                crate::ssh_guard::record_success(context_id);
            }
        }
        Err(e) => {
            if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
                crate::ssh_guard::record_failure(context_id, &e);
            }
            ctx.last_probe_status = Some("error".into());
            ctx.last_probe_error = Some(e);
        }
    }
    ctx.last_probe_at = Some(now);
    ctx.updated_at = now;
    store
        .upsert_execution_context(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ctx)
}

#[tauri::command]
pub async fn probe_execution_context(
    state: State<'_, crate::AppState>,
    context_id: String,
) -> Result<wisp_store::ExecutionContext, String> {
    debug_assert!(PROBE_SKILL.contains("name: probe-compute-environment"));
    tracing::info!(
        skill = PROBE_SKILL_NAME,
        context_id,
        "probing execution context"
    );
    let mut runner = ProcessProbeRunner;
    probe_and_store_with_runner(&state.store, &context_id, &mut runner).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_probe_commands_for_local_ssh_and_wsl() {
        let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "Ubuntu").unwrap();

        let local_cmd = build_probe_command(&local, "uname -s").unwrap();
        assert_eq!(local_cmd.script, "uname -s");
        assert!(!local_cmd.program.is_empty());

        let ssh_cmd = build_probe_command(&ssh, "uname -s").unwrap();
        assert_eq!(ssh_cmd.program, "ssh");
        assert_eq!(
            ssh_cmd.args,
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-T",
                "-o",
                "IdentitiesOnly=yes",
                "gpu-box",
                "uname -s",
            ]
        );
        assert_eq!(ssh_cmd.script, "uname -s");

        let wsl_cmd = build_probe_command(&wsl, "uname -s").unwrap();
        assert_eq!(wsl_cmd.program, "wsl.exe");
        assert_eq!(
            wsl_cmd.args,
            ["-d", "Ubuntu-22.04", "--exec", "sh", "-lc", "uname -s"]
        );
    }

    #[test]
    fn ssh_probe_uses_user_port_and_identity_file() {
        let mut ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        ssh.config_json = serde_json::json!({
            "alias": "gpu-box",
            "user": "alice",
            "port": 2222,
            "identity_file": "/home/alice/.ssh/lab key"
        })
        .to_string();

        let command = build_probe_command(&ssh, "uname -s").unwrap();
        assert_eq!(
            command.args,
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-T",
                "-p",
                "2222",
                "-o",
                "IdentitiesOnly=yes",
                "-i",
                "/home/alice/.ssh/lab key",
                "alice@gpu-box",
                "uname -s",
            ]
        );
    }

    #[test]
    fn fake_runner_collects_probe_capabilities() {
        let ctx = wisp_store::ExecutionContext::new("wsl:Ubuntu", "GPU").unwrap();
        let mut runner = FakeRunner::new(vec![
            ("uname -s".into(), "Linux".into()),
            ("uname -m".into(), "x86_64".into()),
            ("hostname".into(), "gpu01".into()),
            ("getconf _NPROCESSORS_ONLN".into(), "64".into()),
            ("nvidia-smi -L".into(), "GPU 0: NVIDIA A100-SXM4-80GB".into()),
            (
                "command -v sbatch || command -v qsub || command -v bsub".into(),
                "/usr/bin/sbatch".into(),
            ),
            (posix_python_discovery(), "/opt/conda/bin/python".into()),
            (
                posix_with_discovered("PY", &posix_python_discovery(), "--version 2>&1"),
                "Python 3.11.8".into(),
            ),
            (posix_rscript_discovery(), "/opt/R/bin/Rscript".into()),
            (
                posix_with_discovered("RSCRIPT", &posix_rscript_discovery(), "--version 2>&1"),
                "R scripting front-end version 4.4.1".into(),
            ),
            (
                posix_with_discovered(
                    "RSCRIPT",
                    &posix_rscript_discovery(),
                    "--vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                ),
                "TRUE".into(),
            ),
            ("command -v conda".into(), "/opt/conda/bin/conda".into()),
            ("command -v mamba".into(), "".into()),
            ("command -v modulecmd".into(), "/usr/bin/modulecmd".into()),
            ("printf '%s' \"$HOME\"".into(), "/home/alice".into()),
            ("pwd".into(), "/scratch/proj".into()),
            (
                "if [ \"$(id -u)\" = 0 ]; then printf root; elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then printf sudo; else printf unprivileged; fi".into(),
                "unprivileged".into(),
            ),
        ]);

        let probe = probe_context_with_runner(&ctx, &mut runner).unwrap();
        assert_eq!(probe.os.as_deref(), Some("Linux"));
        assert_eq!(probe.arch.as_deref(), Some("x86_64"));
        assert_eq!(probe.hostname.as_deref(), Some("gpu01"));
        assert_eq!(probe.cpu_count, Some(64));
        assert_eq!(
            probe.gpu_summary.as_deref(),
            Some("GPU 0: NVIDIA A100-SXM4-80GB")
        );
        assert_eq!(probe.scheduler.as_deref(), Some("slurm"));
        assert_eq!(probe.python.as_deref(), Some("Python 3.11.8"));
        assert_eq!(
            probe.python_executable.as_deref(),
            Some("/opt/conda/bin/python")
        );
        assert_eq!(probe.python_version.as_deref(), Some("Python 3.11.8"));
        assert_eq!(
            probe.rscript_executable.as_deref(),
            Some("/opt/R/bin/Rscript")
        );
        assert_eq!(
            probe.r_version.as_deref(),
            Some("R scripting front-end version 4.4.1")
        );
        assert_eq!(probe.r_jsonlite, Some(true));
        assert_eq!(probe.conda.as_deref(), Some("/opt/conda/bin/conda"));
        assert_eq!(probe.mamba, None);
        assert_eq!(probe.modulecmd.as_deref(), Some("/usr/bin/modulecmd"));
        assert_eq!(probe.home.as_deref(), Some("/home/alice"));
        assert_eq!(probe.pwd.as_deref(), Some("/scratch/proj"));
        assert_eq!(probe.privilege.as_deref(), Some("unprivileged"));
        assert_eq!(probe.probe_skill, "probe-compute-environment");
    }

    #[test]
    fn probe_uses_persisted_interpreters_instead_of_path_discovery() {
        let mut ctx = wisp_store::ExecutionContext::new("wsl:Ubuntu", "CPU2").unwrap();
        ctx.config_json = serde_json::json!({
            "python_executable": "/opt/conda env/bin/python",
            "rscript_executable": "/opt/R 4.5/bin/Rscript"
        })
        .to_string();
        let mut runner = FakeRunner::new(vec![
            ("uname -s".into(), "Linux".into()),
            ("uname -m".into(), "x86_64".into()),
            ("hostname".into(), "cpu2".into()),
            (
                "'/opt/conda env/bin/python' --version 2>&1".into(),
                "Python 3.12.2".into(),
            ),
            (
                "'/opt/R 4.5/bin/Rscript' --version 2>&1".into(),
                "R scripting front-end version 4.5.2".into(),
            ),
            (
                "'/opt/R 4.5/bin/Rscript' --vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null".into(),
                "TRUE".into(),
            ),
            // Capabilities this host genuinely lacks: explicit empty output.
            ("getconf _NPROCESSORS_ONLN".into(), String::new()),
            ("nvidia-smi -L".into(), String::new()),
            (
                "command -v sbatch || command -v qsub || command -v bsub".into(),
                String::new(),
            ),
            ("command -v conda".into(), String::new()),
            ("command -v mamba".into(), String::new()),
            ("command -v modulecmd".into(), String::new()),
            ("printf '%s' \"$HOME\"".into(), String::new()),
            ("pwd".into(), String::new()),
            (
                "if [ \"$(id -u)\" = 0 ]; then printf root; elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then printf sudo; else printf unprivileged; fi".into(),
                String::new(),
            ),
        ]);

        let probe = probe_context_with_runner(&ctx, &mut runner).unwrap();
        assert_eq!(
            probe.python_executable.as_deref(),
            Some("/opt/conda env/bin/python")
        );
        assert_eq!(probe.python_version.as_deref(), Some("Python 3.12.2"));
        assert_eq!(
            probe.rscript_executable.as_deref(),
            Some("/opt/R 4.5/bin/Rscript")
        );
        assert_eq!(probe.r_jsonlite, Some(true));
    }

    #[test]
    fn empty_probe_output_marks_capabilities_absent_instead_of_failing() {
        let ctx = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Empty").unwrap();
        let (specs, _) = probe_specs(&ctx).unwrap();
        let mut runner = FakeRunner::new(
            specs
                .iter()
                .map(|spec| (spec.script.clone(), String::new()))
                .collect(),
        );

        let probe = probe_context_with_runner(&ctx, &mut runner).unwrap();
        assert_eq!(probe.os, None);
        assert_eq!(probe.cpu_count, None);
        assert_eq!(probe.gpu_summary, None);
        assert_eq!(probe.scheduler, None);
        assert_eq!(probe.python_executable, None);
        assert_eq!(probe.rscript_executable, None);
    }

    #[test]
    fn interpreter_discovery_prefers_path_then_common_install_dirs() {
        let ctx = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let (specs, values) = probe_specs(&ctx).unwrap();
        assert!(!values.contains_key("rscript_executable"));

        let exe = specs
            .iter()
            .find(|spec| spec.key == "rscript_executable")
            .unwrap();
        assert!(exe.script.starts_with("command -v Rscript"));
        assert!(exe.script.contains("/opt/homebrew/bin/Rscript"));
        assert!(exe.script.contains("/opt/conda/bin/Rscript"));

        // The version/jsonlite probes must reuse the discovered interpreter,
        // otherwise an Rscript found outside PATH would still probe as missing.
        let version = specs.iter().find(|spec| spec.key == "r_version").unwrap();
        assert!(version.script.contains("RSCRIPT=$("));
        assert!(version.script.contains("\"$RSCRIPT\" --version"));
        let jsonlite = specs.iter().find(|spec| spec.key == "r_jsonlite").unwrap();
        assert!(jsonlite.script.contains("RSCRIPT=$("));

        let python = specs
            .iter()
            .find(|spec| spec.key == "python_executable")
            .unwrap();
        assert!(python
            .script
            .starts_with("command -v python3 || command -v python"));
        assert!(python.script.contains("/opt/conda/bin/python3"));
    }

    #[test]
    fn windows_discovery_scans_program_files_and_localappdata() {
        let rscript = windows_rscript_discovery();
        assert!(rscript.contains("Get-Command Rscript"));
        assert!(rscript.contains("ProgramFiles"));
        assert!(rscript.contains("bin\\Rscript.exe"));
        let python = windows_python_discovery();
        assert!(python.contains("Get-Command python"));
        assert!(python.contains("Programs\\Python"));
    }

    #[test]
    fn ssh_probe_collects_every_field_through_one_connection() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let mut runner = OneShotRunner {
            output: ProbeCommandOutput {
                status: 0,
                stdout: bundled_output(&[
                    ("os", "Linux"),
                    ("arch", "x86_64"),
                    ("hostname", "gpu01"),
                    ("cpu_count", "64"),
                    ("gpu_summary", "GPU 0: NVIDIA A100"),
                ])
                .into_bytes(),
                stderr: String::new(),
            },
            commands: Vec::new(),
        };

        let probe = probe_context_with_runner(&ctx, &mut runner).unwrap();

        assert_eq!(runner.commands.len(), 1);
        assert_eq!(runner.commands[0].program, "ssh");
        assert!(runner.commands[0].script.starts_with("sh -c 'set +e"));
        assert!(runner.commands[0].script.contains("uname -s"));
        assert!(runner.commands[0].script.contains("nvidia-smi -L"));
        assert_eq!(probe.os.as_deref(), Some("Linux"));
        assert_eq!(probe.cpu_count, Some(64));
        assert_eq!(probe.gpu_summary.as_deref(), Some("GPU 0: NVIDIA A100"));
    }

    #[test]
    fn bundled_probe_accepts_crlf_output() {
        let output = bundled_output(&[("os", "Linux"), ("arch", "x86_64"), ("hostname", "gpu01")])
            .replace('\n', "\r\n");
        assert_eq!(parse_bundled_value(&output, "os").as_deref(), Some("Linux"));
    }

    #[test]
    fn ssh_probe_allows_missing_uname_output_after_command_execution() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let mut runner = OneShotRunner {
            output: ProbeCommandOutput {
                status: 0,
                stdout: bundled_output(&[("os", ""), ("arch", "x86_64"), ("hostname", "gpu01")])
                    .into_bytes(),
                stderr: String::new(),
            },
            commands: Vec::new(),
        };

        let probe = probe_context_with_runner(&ctx, &mut runner).unwrap();

        assert_eq!(probe.os, None);
        assert_eq!(probe.arch.as_deref(), Some("x86_64"));
        assert_eq!(probe.hostname.as_deref(), Some("gpu01"));
    }

    #[test]
    fn ssh_probe_rejects_accounts_that_do_not_execute_remote_commands() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let mut runner = OneShotRunner {
            output: ProbeCommandOutput {
                status: 0,
                stdout: b"Welcome to the restricted service\n".to_vec(),
                stderr: String::new(),
            },
            commands: Vec::new(),
        };

        let error = probe_context_with_runner(&ctx, &mut runner).unwrap_err();

        assert!(error.contains("SSH authentication succeeded"));
        assert!(error.contains("did not execute Wisp's non-interactive probe commands"));
    }

    #[test]
    fn ssh_probe_surfaces_authentication_failure_from_its_only_connection() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let mut runner = OneShotRunner {
            output: ProbeCommandOutput {
                status: 255,
                stdout: Vec::new(),
                stderr: "Permission denied (publickey).".into(),
            },
            commands: Vec::new(),
        };

        let error = probe_context_with_runner(&ctx, &mut runner).unwrap_err();

        assert_eq!(runner.commands.len(), 1);
        assert!(error.contains("Permission denied (publickey)"));
        assert!(error.contains("SSH key authentication failed"));
    }

    #[test]
    fn ssh_probe_labels_password_rejection() {
        let connection = crate::ssh_hosts::SshConnection {
            alias: "gpu-box".into(),
            host_name: None,
            user: Some("alice".into()),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            auth_method: crate::ssh_hosts::SshAuthMethod::Password,
        };

        let error = format_ssh_probe_failure(
            &connection,
            255,
            "Permission denied (password,keyboard-interactive).",
        );

        assert!(error.contains("SSH password authentication failed"));
        assert!(error.contains("server rejected the saved password"));
    }

    #[tokio::test]
    async fn failed_probe_keeps_previous_capabilities() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_probe_context_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let mut ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        ctx.capabilities_json = r#"{"os":"Linux","gpu_summary":"A100"}"#.into();
        store.upsert_execution_context(&ctx).await.unwrap();

        let mut runner = FailingRunner;
        let updated = probe_and_store_with_runner(&store, "ssh:gpu-box", &mut runner)
            .await
            .unwrap();

        assert_eq!(
            updated.capabilities_json,
            r#"{"os":"Linux","gpu_summary":"A100"}"#
        );
        assert_eq!(updated.last_probe_status.as_deref(), Some("error"));
        assert!(updated
            .last_probe_error
            .as_deref()
            .unwrap()
            .contains("boom"));

        let _ = std::fs::remove_file(&tmp);
    }

    struct FakeRunner {
        outputs: HashMap<String, String>,
    }

    impl FakeRunner {
        fn new(pairs: Vec<(String, String)>) -> Self {
            Self {
                outputs: pairs.into_iter().collect(),
            }
        }
    }

    impl ProbeRunner for FakeRunner {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            // A forgotten probe must fail loudly instead of silently reading
            // as "capability absent" (empty stdout).
            let stdout = self.outputs.get(&command.script).unwrap_or_else(|| {
                panic!(
                    "FakeRunner received an unscripted probe command: {}",
                    command.script
                )
            });
            Ok(ProbeCommandOutput {
                status: 0,
                stdout: stdout.clone().into_bytes(),
                stderr: String::new(),
            })
        }
    }

    struct FailingRunner;

    impl ProbeRunner for FailingRunner {
        fn run(&mut self, _command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            Err("boom".into())
        }
    }

    struct OneShotRunner {
        output: ProbeCommandOutput,
        commands: Vec<ProbeCommand>,
    }

    impl ProbeRunner for OneShotRunner {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            self.commands.push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn bundled_output(values: &[(&str, &str)]) -> String {
        values
            .iter()
            .map(|(key, value)| {
                format!("{PROBE_VALUE_BEGIN}:{key}\n{value}\n{PROBE_VALUE_END}:{key}\n")
            })
            .collect()
    }
}
