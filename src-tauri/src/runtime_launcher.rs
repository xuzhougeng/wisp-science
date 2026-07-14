//! ExecutionContext-aware launcher for attached interactive runtimes.

use crate::{
    run_context::{ProcessRunRunner, RunCommand, RunCommandOutput, RunCommandRunner},
    ssh_hosts::SshConnection,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use wisp_runtime::{
    find_rscript, KernelClient, LaunchedRuntime, PythonEnv, RuntimeKey, RuntimeLanguage,
    RuntimeLauncher, RuntimeMetadata, PROTOCOL_VERSION,
};

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TauriRuntimeLauncher {
    store: wisp_store::Store,
    app_data: PathBuf,
    python_worker: PathBuf,
    r_worker: PathBuf,
    envs: Vec<(String, String)>,
    runner: Arc<dyn RunCommandRunner>,
}

impl TauriRuntimeLauncher {
    pub fn new(
        store: wisp_store::Store,
        app_data: PathBuf,
        python_worker: PathBuf,
        r_worker: PathBuf,
        envs: Vec<(String, String)>,
    ) -> Self {
        Self {
            store,
            app_data,
            python_worker,
            r_worker,
            envs,
            runner: Arc::new(ProcessRunRunner),
        }
    }

    #[cfg(test)]
    fn with_runner(mut self, runner: Arc<dyn RunCommandRunner>) -> Self {
        self.runner = runner;
        self
    }
}

#[async_trait]
impl RuntimeLauncher for TauriRuntimeLauncher {
    async fn launch(&self, key: &RuntimeKey, project_root: &Path) -> Result<LaunchedRuntime> {
        let context = self
            .store
            .get_execution_context(&key.context_id)
            .await?
            .ok_or_else(|| anyhow!("Execution context not found: {}", key.context_id))?;
        let (interpreter, worker, language) = match key.language {
            RuntimeLanguage::Python => (
                resolve_python_interpreter(&context, &self.app_data)?,
                &self.python_worker,
                "python",
            ),
            RuntimeLanguage::R => (resolve_r_interpreter(&context)?, &self.r_worker, "r"),
        };
        if !worker.is_file() {
            return Err(anyhow!(
                "{} runtime worker not found at {}",
                language,
                worker.display()
            ));
        }
        let remote_worker = if context.kind == wisp_store::ExecutionContextKind::Local {
            None
        } else {
            let source = tokio::fs::read_to_string(worker).await.map_err(|error| {
                anyhow!(
                    "read {language} runtime worker {}: {error}",
                    worker.display()
                )
            })?;
            Some(
                ensure_remote_worker(&context, key.language, &source, self.runner.as_ref())
                    .await
                    .map_err(anyhow::Error::msg)?,
            )
        };
        let command = build_attached_command(
            &context,
            key.language,
            &interpreter,
            worker,
            remote_worker.as_deref(),
            project_root,
        )
        .map_err(anyhow::Error::msg)?;
        let envs = if context.kind == wisp_store::ExecutionContextKind::Local
            && key.language == RuntimeLanguage::Python
        {
            self.envs.as_slice()
        } else {
            &[]
        };
        let client = KernelClient::spawn_command(
            &command.program,
            &command.args,
            envs,
            command.cwd.as_deref(),
            language,
        )
        .await?;
        let ready = client.ready().clone();
        Ok(LaunchedRuntime::new(
            Box::new(client),
            RuntimeMetadata {
                interpreter: Some(interpreter),
                version: Some(ready.version),
                process_id: Some(ready.pid),
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachedCommand {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
}

fn resolve_python_interpreter(
    context: &wisp_store::ExecutionContext,
    app_data: &Path,
) -> Result<String> {
    let config = json_object(&context.config_json, "execution context config")?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")?;
    if let Some(interpreter) = first_string(&config, &["python_executable", "python_path"])? {
        return Ok(interpreter);
    }
    if let Some(interpreter) = first_string(&capabilities, &["python_executable"])? {
        return Ok(interpreter);
    }
    if context.kind == wisp_store::ExecutionContextKind::Local {
        return Ok(PythonEnv::managed(app_data)
            .python()
            .to_string_lossy()
            .into_owned());
    }
    Err(anyhow!(
        "Python interpreter is unknown for {}; probe the context or configure python_executable",
        context.id
    ))
}

fn resolve_r_interpreter(context: &wisp_store::ExecutionContext) -> Result<String> {
    let config = json_object(&context.config_json, "execution context config")?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")?;
    if let Some(interpreter) = first_string(&config, &["rscript_executable", "rscript_path"])? {
        return Ok(interpreter);
    }
    if let Some(interpreter) = first_string(&capabilities, &["rscript_executable"])? {
        if capabilities
            .get("r_jsonlite")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            return Err(anyhow!(
                "R package 'jsonlite' is not available in {}; install it in the selected R environment",
                context.id
            ));
        }
        return Ok(interpreter);
    }
    if context.kind == wisp_store::ExecutionContextKind::Local {
        return find_rscript()
            .map(|path| path.to_string_lossy().into_owned())
            .ok_or_else(|| {
                anyhow!(
                    "Rscript not found on PATH; install R or set WISP_RSCRIPT to the selected interpreter"
                )
            });
    }
    Err(anyhow!(
        "Rscript interpreter is unknown for {}; probe the context or configure rscript_executable",
        context.id
    ))
}

fn build_attached_command(
    context: &wisp_store::ExecutionContext,
    language: RuntimeLanguage,
    interpreter: &str,
    local_worker: &Path,
    remote_worker: Option<&str>,
    project_root: &Path,
) -> Result<AttachedCommand, String> {
    validate_context_value("runtime interpreter", interpreter)?;
    match context.kind {
        wisp_store::ExecutionContextKind::Local => Ok(AttachedCommand {
            program: PathBuf::from(interpreter),
            args: match language {
                RuntimeLanguage::Python => vec![local_worker.as_os_str().to_os_string()],
                RuntimeLanguage::R => vec![
                    OsString::from("--vanilla"),
                    local_worker.as_os_str().to_os_string(),
                ],
            },
            cwd: Some(project_root.to_path_buf()),
        }),
        wisp_store::ExecutionContextKind::Wsl | wisp_store::ExecutionContextKind::Ssh => {
            let worker =
                remote_worker.ok_or_else(|| "remote worker path is required".to_string())?;
            let workdir = runtime_workdir(context)?;
            let interpreter = shell_single_quote(interpreter);
            let worker = remote_path_expression(worker)?;
            let script = match language {
                RuntimeLanguage::Python => format!(
                    "cd {} && exec {interpreter} {worker}",
                    remote_path_expression(&workdir)?,
                ),
                RuntimeLanguage::R => format!(
                    "cd {} && exec {interpreter} --vanilla {worker}",
                    remote_path_expression(&workdir)?,
                ),
            };
            match context.kind {
                wisp_store::ExecutionContextKind::Wsl => {
                    let distro = wsl_distro(context)?;
                    Ok(AttachedCommand {
                        program: PathBuf::from("wsl.exe"),
                        args: ["-d", &distro, "--", "sh", "-lc", &script]
                            .into_iter()
                            .map(OsString::from)
                            .collect(),
                        cwd: None,
                    })
                }
                wisp_store::ExecutionContextKind::Ssh => {
                    let mut args = SshConnection::from_execution_context(context)?.ssh_args()?;
                    args.push(script);
                    Ok(AttachedCommand {
                        program: PathBuf::from("ssh"),
                        args: args.into_iter().map(OsString::from).collect(),
                        cwd: None,
                    })
                }
                wisp_store::ExecutionContextKind::Local => unreachable!(),
            }
        }
    }
}

async fn ensure_remote_worker(
    context: &wisp_store::ExecutionContext,
    language: RuntimeLanguage,
    source: &str,
    runner: &dyn RunCommandRunner,
) -> Result<String, String> {
    let checksum = wisp_sync::sha256_hex(source.as_bytes());
    let (name, extension) = match language {
        RuntimeLanguage::Python => ("python", "py"),
        RuntimeLanguage::R => ("r", "R"),
    };
    let remote_path = format!(
        "~/.wisp-science/runtime/{name}-v{}-{checksum}.{extension}",
        PROTOCOL_VERSION
    );
    let check = runner
        .run(
            remote_command(
                context,
                &format!("check {name} runtime worker"),
                checksum_script(&remote_path, &checksum),
                None,
            )?,
            DEPLOY_TIMEOUT,
        )
        .await?;
    if check.exit_code == 0 {
        return Ok(remote_path);
    }
    let deploy = runner
        .run(
            remote_command(
                context,
                &format!("deploy {name} runtime worker"),
                deploy_script(&remote_path, &checksum),
                Some(source.to_string()),
            )?,
            DEPLOY_TIMEOUT,
        )
        .await?;
    checked_command(&format!("{name} runtime worker deployment"), deploy)?;
    Ok(remote_path)
}

fn remote_command(
    context: &wisp_store::ExecutionContext,
    label: &str,
    script: String,
    stdin: Option<String>,
) -> Result<RunCommand, String> {
    match context.kind {
        wisp_store::ExecutionContextKind::Wsl => Ok(RunCommand {
            context_id: context.id.clone(),
            program: "wsl.exe".into(),
            args: vec![
                "-d".into(),
                wsl_distro(context)?,
                "--".into(),
                "sh".into(),
                "-lc".into(),
                script,
            ],
            script: label.into(),
            cwd: None,
            stdin,
        }),
        wisp_store::ExecutionContextKind::Ssh => {
            let mut args = SshConnection::from_execution_context(context)?.ssh_args()?;
            args.push(format!("sh -lc {}", shell_single_quote(&script)));
            Ok(RunCommand {
                context_id: context.id.clone(),
                program: "ssh".into(),
                args,
                script: label.into(),
                cwd: None,
                stdin,
            })
        }
        wisp_store::ExecutionContextKind::Local => {
            Err("remote deployment requires WSL or SSH".into())
        }
    }
}

fn checksum_script(remote_path: &str, checksum: &str) -> String {
    let path = remote_path_expression(remote_path).expect("generated runtime path is valid");
    format!(
        "hash_file() {{ if command -v sha256sum >/dev/null 2>&1; then sha256sum \"$1\" | cut -d' ' -f1; else shasum -a 256 \"$1\" | cut -d' ' -f1; fi; }}; test -f {path} && test \"$(hash_file {path})\" = {}",
        shell_single_quote(checksum)
    )
}

fn deploy_script(remote_path: &str, checksum: &str) -> String {
    let path = remote_path_expression(remote_path).expect("generated runtime path is valid");
    format!(
        "set -eu; dir=\"$HOME/.wisp-science/runtime\"; mkdir -p \"$dir\"; tmp={path}.tmp.$$; cat > \"$tmp\"; if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum \"$tmp\" | cut -d' ' -f1); else actual=$(shasum -a 256 \"$tmp\" | cut -d' ' -f1); fi; if test \"$actual\" != {}; then rm -f \"$tmp\"; exit 1; fi; chmod 600 \"$tmp\"; mv -f \"$tmp\" {path}",
        shell_single_quote(checksum)
    )
}

fn checked_command(label: &str, output: RunCommandOutput) -> Result<(), String> {
    if output.exit_code == 0 {
        return Ok(());
    }
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

fn runtime_workdir(context: &wisp_store::ExecutionContext) -> Result<String, String> {
    let config = json_object(&context.config_json, "execution context config")
        .map_err(|error| error.to_string())?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")
        .map_err(|error| error.to_string())?;
    first_string(&config, &["workdir", "default_workdir"])
        .map_err(|error| error.to_string())?
        .or(first_string(&capabilities, &["pwd", "home"]).map_err(|error| error.to_string())?)
        .map_or_else(
            || Ok("~".into()),
            |value| {
                validate_context_value("runtime workdir", &value)?;
                Ok(value)
            },
        )
}

fn wsl_distro(context: &wisp_store::ExecutionContext) -> Result<String, String> {
    let config = json_object(&context.config_json, "WSL context config")
        .map_err(|error| error.to_string())?;
    let distro = first_string(&config, &["distro"])
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            context
                .id
                .strip_prefix("wsl:")
                .unwrap_or(&context.id)
                .to_string()
        });
    validate_context_value("WSL distro", &distro)?;
    Ok(distro)
}

fn json_object(value: &str, label: &str) -> Result<serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|error| anyhow!("Invalid {label}: {error}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(anyhow!("{label} must be a JSON object"))
    }
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Result<Option<String>> {
    for key in keys {
        match value.get(*key) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
                validate_context_value(key, value).map_err(anyhow::Error::msg)?;
                return Ok(Some(value.clone()));
            }
            Some(serde_json::Value::String(_)) => {}
            Some(_) => return Err(anyhow!("execution context field '{key}' must be a string")),
        }
    }
    Ok(None)
}

fn validate_context_value(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.contains(['\0', '\n', '\r']) {
        Err(format!(
            "{label} must be non-empty and contain no line breaks"
        ))
    } else {
        Ok(())
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_path_expression(path: &str) -> Result<String, String> {
    validate_context_value("remote path", path)?;
    if path == "~" {
        Ok("\"$HOME\"".into())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("\"$HOME\"/{}", shell_single_quote(rest)))
    } else {
        Ok(shell_single_quote(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    struct FakeRunner {
        outputs: Mutex<VecDeque<RunCommandOutput>>,
        commands: Mutex<Vec<RunCommand>>,
    }

    impl FakeRunner {
        fn new(exit_codes: impl IntoIterator<Item = i64>) -> Self {
            Self {
                outputs: Mutex::new(
                    exit_codes
                        .into_iter()
                        .map(|exit_code| RunCommandOutput {
                            exit_code,
                            stdout: String::new(),
                            stderr: String::new(),
                        })
                        .collect(),
                ),
                commands: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl RunCommandRunner for FakeRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            self.commands.lock().unwrap().push(command);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "unexpected command".into())
        }
    }

    #[test]
    fn local_launch_keeps_windows_paths_with_spaces_as_single_arguments() {
        let mut context = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        context.config_json = serde_json::json!({
            "python_executable": r"C:\Program Files\Python\python.exe"
        })
        .to_string();
        let interpreter = resolve_python_interpreter(&context, Path::new("unused")).unwrap();
        let command = build_attached_command(
            &context,
            RuntimeLanguage::Python,
            &interpreter,
            Path::new(r"C:\Program Files\Wisp\kernel_worker.py"),
            None,
            Path::new(r"C:\Research Project"),
        )
        .unwrap();
        assert_eq!(
            command.program,
            PathBuf::from(r"C:\Program Files\Python\python.exe")
        );
        assert_eq!(command.args.len(), 1);
        assert_eq!(
            command.args[0],
            OsString::from(r"C:\Program Files\Wisp\kernel_worker.py")
        );

        context.config_json = serde_json::json!({
            "rscript_executable": r"C:\Program Files\R\bin\Rscript.exe"
        })
        .to_string();
        let rscript = resolve_r_interpreter(&context).unwrap();
        let command = build_attached_command(
            &context,
            RuntimeLanguage::R,
            &rscript,
            Path::new(r"C:\Program Files\Wisp\kernel_worker.R"),
            None,
            Path::new(r"C:\Research Project"),
        )
        .unwrap();
        assert_eq!(command.args[0], OsString::from("--vanilla"));
        assert_eq!(
            command.args[1],
            OsString::from(r"C:\Program Files\Wisp\kernel_worker.R")
        );
    }

    #[test]
    fn wsl_and_ssh_launches_preserve_context_configuration() {
        let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-24.04", "WSL").unwrap();
        wsl.config_json = serde_json::json!({
            "distro": "Ubuntu 24.04",
            "workdir": "/scratch/project one"
        })
        .to_string();
        wsl.capabilities_json = serde_json::json!({
            "python_executable": "/opt/conda env/bin/python"
        })
        .to_string();
        let wsl_python = resolve_python_interpreter(&wsl, Path::new("unused")).unwrap();
        let wsl_command = build_attached_command(
            &wsl,
            RuntimeLanguage::Python,
            &wsl_python,
            Path::new("unused"),
            Some("~/.wisp-science/runtime/python.py"),
            Path::new("unused"),
        )
        .unwrap();
        let wsl_args = wsl_command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(wsl_command.program, PathBuf::from("wsl.exe"));
        assert_eq!(&wsl_args[..2], ["-d", "Ubuntu 24.04"]);
        assert!(wsl_args.last().unwrap().contains("/scratch/project one"));

        let mut ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        ssh.config_json = serde_json::json!({
            "user": "alice",
            "port": 2222,
            "identity_file": "/home/alice/.ssh/lab key",
            "python_executable": "/opt/python/bin/python"
        })
        .to_string();
        let ssh_command = build_attached_command(
            &ssh,
            RuntimeLanguage::Python,
            "/opt/python/bin/python",
            Path::new("unused"),
            Some("~/.wisp-science/runtime/python.py"),
            Path::new("unused"),
        )
        .unwrap();
        let ssh_args = ssh_command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(ssh_command.program, PathBuf::from("ssh"));
        assert!(ssh_args.windows(2).any(|args| args == ["-p", "2222"]));
        assert!(ssh_args
            .windows(2)
            .any(|args| args == ["-i", "/home/alice/.ssh/lab key"]));
        assert!(ssh_args.contains(&"alice@gpu-box".to_string()));

        wsl.capabilities_json = serde_json::json!({
            "rscript_executable": "/opt/R/bin/Rscript",
            "r_jsonlite": true
        })
        .to_string();
        let rscript = resolve_r_interpreter(&wsl).unwrap();
        let r_command = build_attached_command(
            &wsl,
            RuntimeLanguage::R,
            &rscript,
            Path::new("unused"),
            Some("~/.wisp-science/runtime/r.R"),
            Path::new("unused"),
        )
        .unwrap();
        assert!(r_command
            .args
            .last()
            .unwrap()
            .to_string_lossy()
            .contains("--vanilla"));
    }

    #[test]
    fn known_missing_jsonlite_is_an_actionable_r_capability_error() {
        let mut context = wisp_store::ExecutionContext::new("ssh:r-box", "R").unwrap();
        context.capabilities_json = serde_json::json!({
            "rscript_executable": "/usr/bin/Rscript",
            "r_jsonlite": false
        })
        .to_string();
        let error = resolve_r_interpreter(&context).unwrap_err();
        assert!(error.to_string().contains("jsonlite"));
        assert!(error.to_string().contains("install"));

        context.config_json = serde_json::json!({
            "rscript_executable": "/opt/project-R/bin/Rscript"
        })
        .to_string();
        assert_eq!(
            resolve_r_interpreter(&context).unwrap(),
            "/opt/project-R/bin/Rscript"
        );
    }

    #[tokio::test]
    async fn remote_deployment_skips_checksum_hits_and_uploads_misses() {
        let context = wisp_store::ExecutionContext::new("wsl:Ubuntu", "WSL").unwrap();
        let hit = FakeRunner::new([0]);
        let path = ensure_remote_worker(&context, RuntimeLanguage::Python, "print('worker')", &hit)
            .await
            .unwrap();
        assert!(path.contains(&format!("python-v{}-", PROTOCOL_VERSION)));
        assert_eq!(hit.commands.lock().unwrap().len(), 1);

        let miss = FakeRunner::new([1, 0]);
        ensure_remote_worker(&context, RuntimeLanguage::R, "print('worker')", &miss)
            .await
            .unwrap();
        let commands = miss.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].script, "check r runtime worker");
        assert_eq!(commands[1].script, "deploy r runtime worker");
        assert_eq!(commands[1].stdin.as_deref(), Some("print('worker')"));
    }

    #[tokio::test]
    async fn launcher_uses_the_persisted_context_registry() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_launcher_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&db).await.unwrap();
        let launcher = TauriRuntimeLauncher::new(
            store,
            PathBuf::from("app-data"),
            PathBuf::from("worker.py"),
            PathBuf::from("worker.R"),
            vec![],
        )
        .with_runner(Arc::new(FakeRunner::new([])));
        let result = launcher
            .launch(
                &RuntimeKey::python("project", "ssh:missing"),
                Path::new("project"),
            )
            .await;
        let error = match result {
            Ok(_) => panic!("missing context unexpectedly launched"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Execution context not found"));
        let _ = std::fs::remove_file(db);
    }
}
