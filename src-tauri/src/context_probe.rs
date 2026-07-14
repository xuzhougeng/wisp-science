use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCommand {
    pub context_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub script: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait ProbeRunner: Send {
    fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String>;
}

struct ProcessProbeRunner;

impl ProbeRunner for ProcessProbeRunner {
    fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
        let mut process = std::process::Command::new(&command.program);
        process.args(&command.args);
        wisp_tools::process::hide_console(&mut process);
        let output = process
            .output()
            .map_err(|e| format!("failed to run {}: {e}", command.program))?;
        Ok(ProbeCommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
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
                    "--".into(),
                    "sh".into(),
                    "-lc".into(),
                    script.into(),
                ],
                script: script.into(),
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
    }
}

#[cfg(not(target_os = "windows"))]
fn local_command(context_id: &str, script: &str) -> ProbeCommand {
    ProbeCommand {
        context_id: context_id.into(),
        program: "sh".into(),
        args: vec!["-lc".into(), script.into()],
        script: script.into(),
    }
}

pub fn probe_context_with_runner(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
) -> Result<ProbeResult, String> {
    let os = run_required(
        ctx,
        runner,
        platform_script(
            ctx,
            "uname -s",
            "[System.Runtime.InteropServices.RuntimeInformation]::OSDescription",
        ),
    )?;
    let arch = run_required(
        ctx,
        runner,
        platform_script(
            ctx,
            "uname -m",
            "[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture",
        ),
    )?;
    let hostname = run_required(
        ctx,
        runner,
        platform_script(ctx, "hostname", "$env:COMPUTERNAME"),
    )?;
    let cpu_count = run_optional(
        ctx,
        runner,
        platform_script(
            ctx,
            "getconf _NPROCESSORS_ONLN",
            "$env:NUMBER_OF_PROCESSORS",
        ),
    )
    .and_then(|s| s.parse::<u32>().ok());
    let gpu_summary = run_optional(ctx, runner, "nvidia-smi -L");
    let scheduler = run_optional(
        ctx,
        runner,
        platform_script(
            ctx,
            "command -v sbatch || command -v qsub || command -v bsub",
            "Get-Command sbatch,qsub,bsub -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source",
        ),
    )
    .and_then(|s| scheduler_from_command(&s));
    let configured_python = configured_interpreter(ctx, "python_executable", "python_path")?;
    let python_executable = configured_python.clone().or_else(|| {
        run_optional(
            ctx,
            runner,
            platform_script(
                ctx,
                "command -v python3 || command -v python",
                "(Get-Command python -ErrorAction SilentlyContinue).Source",
            ),
        )
    });
    let python_version_script = configured_python.as_deref().map_or_else(
        || {
            platform_script(
                ctx,
                "python3 --version 2>&1 || python --version 2>&1",
                "python --version 2>&1",
            )
            .to_string()
        },
        |executable| interpreter_command(ctx, executable, "--version 2>&1"),
    );
    let python_version = run_optional(ctx, runner, &python_version_script);
    let configured_rscript = configured_interpreter(ctx, "rscript_executable", "rscript_path")?;
    let rscript_executable = configured_rscript.clone().or_else(|| {
        run_optional(
            ctx,
            runner,
            platform_script(
                ctx,
                "command -v Rscript",
                "(Get-Command Rscript -ErrorAction SilentlyContinue).Source",
            ),
        )
    });
    let r_version_script = configured_rscript.as_deref().map_or_else(
        || platform_script(ctx, "Rscript --version 2>&1", "Rscript --version 2>&1").to_string(),
        |executable| interpreter_command(ctx, executable, "--version 2>&1"),
    );
    let r_version = run_optional(ctx, runner, &r_version_script);
    let r_jsonlite_script = configured_rscript.as_deref().map_or_else(
        || {
            platform_script(
                ctx,
                "Rscript --vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                "Rscript --vanilla -e \"cat(requireNamespace('jsonlite', quietly=TRUE))\" 2>$null",
            )
            .to_string()
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
    );
    let r_jsonlite = run_optional(ctx, runner, &r_jsonlite_script)
        .map(|value| value.eq_ignore_ascii_case("true"));
    let conda = run_optional(
        ctx,
        runner,
        platform_script(
            ctx,
            "command -v conda",
            "(Get-Command conda -ErrorAction SilentlyContinue).Source",
        ),
    );
    let mamba = run_optional(
        ctx,
        runner,
        platform_script(
            ctx,
            "command -v mamba",
            "(Get-Command mamba -ErrorAction SilentlyContinue).Source",
        ),
    );
    let modulecmd = run_optional(
        ctx,
        runner,
        platform_script(
            ctx,
            "command -v modulecmd",
            "(Get-Command modulecmd -ErrorAction SilentlyContinue).Source",
        ),
    );
    let home = run_optional(
        ctx,
        runner,
        platform_script(ctx, "printf '%s' \"$HOME\"", "$HOME"),
    );
    let pwd = run_optional(
        ctx,
        runner,
        platform_script(ctx, "pwd", "(Get-Location).Path"),
    );

    Ok(ProbeResult {
        os: Some(os),
        arch: Some(arch),
        hostname: Some(hostname),
        cpu_count,
        gpu_summary,
        scheduler,
        python: python_version.clone(),
        python_executable,
        python_version,
        rscript_executable,
        r_version,
        r_jsonlite,
        conda,
        mamba,
        modulecmd,
        home,
        pwd,
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

fn run_required(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
    script: &str,
) -> Result<String, String> {
    match run_probe_command(ctx, runner, script) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(format!("probe command returned no output: {script}")),
        Err(e) => Err(format!("probe command failed for {script}: {e}")),
    }
}

fn run_optional(
    ctx: &wisp_store::ExecutionContext,
    runner: &mut dyn ProbeRunner,
    script: &str,
) -> Option<String> {
    run_probe_command(ctx, runner, script).ok().flatten()
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
    let value = output.stdout.trim();
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
    let now = chrono::Utc::now().timestamp();
    match probe_context_with_runner(&ctx, runner) {
        Ok(probe) => {
            ctx.capabilities_json = serde_json::to_string(&probe).map_err(|e| e.to_string())?;
            ctx.last_probe_status = Some("ok".into());
            ctx.last_probe_error = None;
        }
        Err(e) => {
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
    let mut runner = ProcessProbeRunner;
    probe_and_store_with_runner(&state.store, &context_id, &mut runner).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
                "gpu-box",
                "uname -s",
            ]
        );
        assert_eq!(ssh_cmd.script, "uname -s");

        let wsl_cmd = build_probe_command(&wsl, "uname -s").unwrap();
        assert_eq!(wsl_cmd.program, "wsl.exe");
        assert!(wsl_cmd.args.contains(&"-d".to_string()));
        assert!(wsl_cmd.args.contains(&"Ubuntu-22.04".to_string()));
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
                "-i",
                "/home/alice/.ssh/lab key",
                "alice@gpu-box",
                "uname -s",
            ]
        );
    }

    #[test]
    fn fake_runner_collects_probe_capabilities() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let mut runner = FakeRunner::new([
            ("uname -s", "Linux"),
            ("uname -m", "x86_64"),
            ("hostname", "gpu01"),
            ("getconf _NPROCESSORS_ONLN", "64"),
            ("nvidia-smi -L", "GPU 0: NVIDIA A100-SXM4-80GB"),
            (
                "command -v sbatch || command -v qsub || command -v bsub",
                "/usr/bin/sbatch",
            ),
            (
                "command -v python3 || command -v python",
                "/opt/conda/bin/python",
            ),
            (
                "python3 --version 2>&1 || python --version 2>&1",
                "Python 3.11.8",
            ),
            ("command -v Rscript", "/opt/R/bin/Rscript"),
            (
                "Rscript --version 2>&1",
                "R scripting front-end version 4.4.1",
            ),
            (
                "Rscript --vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                "TRUE",
            ),
            ("command -v conda", "/opt/conda/bin/conda"),
            ("command -v mamba", ""),
            ("command -v modulecmd", "/usr/bin/modulecmd"),
            ("printf '%s' \"$HOME\"", "/home/alice"),
            ("pwd", "/scratch/proj"),
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
    }

    #[test]
    fn probe_uses_persisted_interpreters_instead_of_path_discovery() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:cpu2", "CPU2").unwrap();
        ctx.config_json = serde_json::json!({
            "python_executable": "/opt/conda env/bin/python",
            "rscript_executable": "/opt/R 4.5/bin/Rscript"
        })
        .to_string();
        let mut runner = FakeRunner::new([
            ("uname -s", "Linux"),
            ("uname -m", "x86_64"),
            ("hostname", "cpu2"),
            (
                "'/opt/conda env/bin/python' --version 2>&1",
                "Python 3.12.2",
            ),
            (
                "'/opt/R 4.5/bin/Rscript' --version 2>&1",
                "R scripting front-end version 4.5.2",
            ),
            (
                "'/opt/R 4.5/bin/Rscript' --vanilla -e 'cat(requireNamespace(\"jsonlite\", quietly=TRUE))' 2>/dev/null",
                "TRUE",
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
        fn new<const N: usize>(pairs: [(&str, &str); N]) -> Self {
            Self {
                outputs: pairs
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }
        }
    }

    impl ProbeRunner for FakeRunner {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            Ok(ProbeCommandOutput {
                status: 0,
                stdout: self
                    .outputs
                    .get(&command.script)
                    .cloned()
                    .unwrap_or_default(),
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
}
