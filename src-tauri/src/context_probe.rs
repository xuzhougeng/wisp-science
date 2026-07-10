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
    pub python: Option<String>,
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
        let output = std::process::Command::new(&command.program)
            .args(&command.args)
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
    let os = run_required(ctx, runner, "uname -s")?;
    let arch = run_required(ctx, runner, "uname -m")?;
    let hostname = run_required(ctx, runner, "hostname")?;
    let cpu_count =
        run_optional(ctx, runner, "getconf _NPROCESSORS_ONLN").and_then(|s| s.parse::<u32>().ok());
    let gpu_summary = run_optional(ctx, runner, "nvidia-smi -L");
    let scheduler = run_optional(
        ctx,
        runner,
        "command -v sbatch || command -v qsub || command -v bsub",
    )
    .and_then(|s| scheduler_from_command(&s));
    let python = run_optional(ctx, runner, "python --version 2>&1");
    let conda = run_optional(ctx, runner, "command -v conda");
    let mamba = run_optional(ctx, runner, "command -v mamba");
    let modulecmd = run_optional(ctx, runner, "command -v modulecmd");
    let home = run_optional(ctx, runner, "printf '%s' \"$HOME\"");
    let pwd = run_optional(ctx, runner, "pwd");

    Ok(ProbeResult {
        os: Some(os),
        arch: Some(arch),
        hostname: Some(hostname),
        cpu_count,
        gpu_summary,
        scheduler,
        python,
        conda,
        mamba,
        modulecmd,
        home,
        pwd,
    })
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
            ("python --version 2>&1", "Python 3.11.8"),
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
        assert_eq!(probe.conda.as_deref(), Some("/opt/conda/bin/conda"));
        assert_eq!(probe.mamba, None);
        assert_eq!(probe.modulecmd.as_deref(), Some("/usr/bin/modulecmd"));
        assert_eq!(probe.home.as_deref(), Some("/home/alice"));
        assert_eq!(probe.pwd.as_deref(), Some("/scratch/proj"));
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
