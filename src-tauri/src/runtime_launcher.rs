//! ExecutionContext-aware launcher for attached interactive runtimes.
//!
//! Interpreter paths, worker deployment and the launch command are shared
//! with the embedded host in [`wisp_runs::runtime_launch`]; this file keeps
//! the Tauri command, local-only environment and the spawn.

use crate::{
    run_context::{ProcessRunRunner, RunCommandRunner},
    ssh_hosts::SshConnection,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::State;
use wisp_runs::runtime_launch::{
    build_attached_command, ensure_remote_worker, InterpreterLanguage,
};
pub use wisp_runs::runtime_launch::{preserve_interpreter_config, save_interpreter_config};
use wisp_runtime::{
    find_rscript, KernelClient, KernelWriteScope, LaunchedRuntime, PythonEnv, RuntimeKey,
    RuntimeLanguage, RuntimeLauncher, RuntimeMetadata, PROTOCOL_VERSION,
};

#[tauri::command]
pub async fn update_execution_context_interpreters(
    state: State<'_, crate::AppState>,
    context_id: String,
    python_executable: String,
    rscript_executable: String,
) -> Result<wisp_store::ExecutionContext, String> {
    save_interpreter_config(
        &state.store,
        &context_id,
        &python_executable,
        &rscript_executable,
    )
    .await
    .map_err(|error| error.to_string())
}

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
        if context.kind == wisp_store::ExecutionContextKind::Ssh {
            crate::ssh_hosts::require_managed_ssh_ready(&context).map_err(anyhow::Error::msg)?;
        }
        let (interpreter, worker, language) = match key.language {
            RuntimeLanguage::Python => (
                resolve_python_interpreter(&context, &self.app_data)?,
                &self.python_worker,
                "python",
            ),
            RuntimeLanguage::R => (local_direct_rscript(&context)?, &self.r_worker, "r"),
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
                ensure_remote_worker(
                    &context,
                    interpreter_language(key.language),
                    PROTOCOL_VERSION,
                    &source,
                    self.runner.as_ref(),
                )
                .await
                .map_err(|error| {
                    if context.kind == wisp_store::ExecutionContextKind::Ssh
                        && crate::ssh_guard::is_authentication_failure(&error)
                    {
                        crate::ssh_guard::record_failure(&context.id, &error);
                    }
                    anyhow::Error::msg(error)
                })?,
            )
        };
        let command = build_attached_command(
            &context,
            interpreter_language(key.language),
            &interpreter,
            worker,
            remote_worker.as_deref(),
            project_root,
        )
        .map_err(anyhow::Error::msg)?;
        let mut envs = launch_envs(
            &context,
            key.language,
            &interpreter,
            &self.envs,
            crate::models::service_env(),
        );
        let ssh_auth_envs = if context.kind == wisp_store::ExecutionContextKind::Ssh {
            let connection =
                SshConnection::from_execution_context(&context).map_err(anyhow::Error::msg)?;
            crate::ssh_hosts::auth_envs_for_connection(&connection).map_err(anyhow::Error::msg)?
        } else {
            Vec::new()
        };
        envs.extend(ssh_auth_envs.iter().cloned());
        // Desktop loopback addresses are not valid on remote SSH/WSL hosts.
        // Remote contexts retain their own environment configuration.
        if context.kind == wisp_store::ExecutionContextKind::Local {
            let settings = crate::network::load(&self.store)
                .await
                .map_err(anyhow::Error::msg)?;
            envs.extend(wisp_tools::network::proxy_env(&settings.command_proxy_url));
        }
        let mut client = match KernelClient::spawn_command(
            &command.program,
            &command.args,
            envs.as_slice(),
            command.cwd.as_deref(),
            language,
        )
        .await
        {
            Ok(client) => {
                crate::ssh_hosts::cleanup_password_auth_env(&ssh_auth_envs);
                if context.kind == wisp_store::ExecutionContextKind::Ssh {
                    crate::ssh_guard::record_success(&context.id);
                }
                client
            }
            Err(error) => {
                crate::ssh_hosts::cleanup_password_auth_env(&ssh_auth_envs);
                if context.kind == wisp_store::ExecutionContextKind::Ssh {
                    let detail = error.to_string();
                    if crate::ssh_guard::is_authentication_failure(&detail) {
                        crate::ssh_guard::record_failure(&context.id, &detail);
                    }
                }
                return Err(error);
            }
        };
        if let Some(scope) = kernel_write_scope(&context, key.language, project_root) {
            client.configure_write_scope(&scope).await?;
        }
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

fn kernel_write_scope(
    context: &wisp_store::ExecutionContext,
    language: RuntimeLanguage,
    project_root: &Path,
) -> Option<KernelWriteScope> {
    if language != RuntimeLanguage::Python {
        return None;
    }
    let root = match context.kind {
        wisp_store::ExecutionContextKind::Local => project_root.to_string_lossy().into_owned(),
        // The WSL launch command enters the translated project root before
        // starting the worker, so `.` is already the correct in-distro path.
        wisp_store::ExecutionContextKind::Wsl => ".".into(),
        wisp_store::ExecutionContextKind::Ssh => return None,
    };
    Some(KernelWriteScope {
        root,
        skip_dirs: wisp_core::provenance::SNAPSHOT_SKIP_DIRS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    })
}

/// The shared launch code's name for a runtime language.
fn interpreter_language(language: RuntimeLanguage) -> InterpreterLanguage {
    match language {
        RuntimeLanguage::Python => InterpreterLanguage::Python,
        RuntimeLanguage::R => InterpreterLanguage::R,
    }
}

fn resolve_python_interpreter(
    context: &wisp_store::ExecutionContext,
    app_data: &Path,
) -> Result<String> {
    wisp_runs::runtime_launch::resolve_python_interpreter(context, || {
        PythonEnv::find_python(app_data).map(|path| path.to_string_lossy().into_owned())
    })
}

fn resolve_r_interpreter(context: &wisp_store::ExecutionContext) -> Result<String> {
    wisp_runs::runtime_launch::resolve_r_interpreter(context, || {
        find_rscript().map(|path| path.to_string_lossy().into_owned())
    })
}

/// Environment for one launched worker, over and above what the child inherits.
///
/// A local interpreter that lives inside a conda/pixi prefix needs that prefix
/// on the child's `PATH` to load its own shared libraries; the host environment
/// is never touched. API credentials stay limited to local Python, and remote
/// contexts get neither: their environment is the remote shell's business.
fn launch_envs(
    context: &wisp_store::ExecutionContext,
    language: RuntimeLanguage,
    interpreter: &str,
    base: &[(String, String)],
    service: Vec<(String, String)>,
) -> Vec<(String, String)> {
    if context.kind != wisp_store::ExecutionContextKind::Local {
        return Vec::new();
    }
    let mut envs = wisp_runtime::conda_prefix_envs(Path::new(interpreter));
    if language == RuntimeLanguage::Python {
        for (name, value) in base.iter().cloned().chain(service) {
            upsert_env(&mut envs, name, value);
        }
    }
    envs
}

fn upsert_env(envs: &mut Vec<(String, String)>, name: String, value: String) {
    match envs.iter_mut().find(|(current, _)| current == &name) {
        Some((_, current)) => *current = value,
        None => envs.push((name, value)),
    }
}

/// Resolve the configured `Rscript`, then on a local Windows context launch the
/// real binary behind the `bin\Rscript.exe` architecture shim. Remote contexts
/// keep the configured path: their filesystem layout is not ours to inspect.
fn local_direct_rscript(context: &wisp_store::ExecutionContext) -> Result<String> {
    let configured = resolve_r_interpreter(context)?;
    if context.kind != wisp_store::ExecutionContextKind::Local {
        return Ok(configured);
    }
    Ok(wisp_runtime::direct_rscript(Path::new(&configured))
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_context::{RunCommand, RunCommandOutput};
    use std::{collections::VecDeque, sync::Mutex, time::Duration};

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
    fn write_scope_is_project_local_for_local_and_wsl_but_disabled_for_ssh_and_r() {
        let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu", "WSL").unwrap();
        let ssh = wisp_store::ExecutionContext::new("ssh:gpu", "SSH").unwrap();
        let project = Path::new(r"C:\Users\me\project one");

        let local_scope = kernel_write_scope(&local, RuntimeLanguage::Python, project).unwrap();
        assert_eq!(local_scope.root, project.to_string_lossy());
        assert_eq!(
            local_scope.skip_dirs,
            wisp_core::provenance::SNAPSHOT_SKIP_DIRS
        );
        assert_eq!(
            kernel_write_scope(&wsl, RuntimeLanguage::Python, project)
                .unwrap()
                .root,
            "."
        );
        assert!(kernel_write_scope(&ssh, RuntimeLanguage::Python, project).is_none());
        assert!(kernel_write_scope(&wsl, RuntimeLanguage::R, project).is_none());
    }

    /// A pixi/conda interpreter cannot find its own shared libraries without
    /// its prefix on PATH; on Windows that is the difference between a working
    /// R kernel and an immediate 0xC0000135 exit (#941).
    #[test]
    fn local_launches_add_the_interpreter_prefix_to_the_child_path() {
        let prefix = std::env::temp_dir().join(format!("wisp-launch-env-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        let rscript = prefix
            .join("lib")
            .join("R")
            .join("bin")
            .join("Rscript")
            .to_string_lossy()
            .into_owned();
        let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let credentials = vec![("OPENAI_API_KEY".to_string(), "secret".to_string())];

        let r_envs = launch_envs(
            &local,
            RuntimeLanguage::R,
            &rscript,
            &[],
            credentials.clone(),
        );
        assert_eq!(r_envs.len(), 1, "{r_envs:?}");
        assert_eq!(r_envs[0].0, "PATH");
        assert!(r_envs[0].1.contains(&prefix.to_string_lossy().into_owned()));

        // Python keeps its credentials and gains the same prefix PATH.
        let python_envs = launch_envs(
            &local,
            RuntimeLanguage::Python,
            &prefix.join("python").to_string_lossy(),
            &[],
            credentials.clone(),
        );
        assert!(python_envs.iter().any(|(name, _)| name == "PATH"));
        assert!(python_envs.contains(&credentials[0]));

        // A remote context's environment belongs to the remote shell.
        let mut ssh = wisp_store::ExecutionContext::new("ssh:cpu2", "CPU2").unwrap();
        ssh.kind = wisp_store::ExecutionContextKind::Ssh;
        assert!(launch_envs(
            &ssh,
            RuntimeLanguage::Python,
            &rscript,
            &[],
            credentials.clone()
        )
        .is_empty());

        // An interpreter outside any prefix must not gain a PATH override.
        let system = launch_envs(
            &local,
            RuntimeLanguage::Python,
            "/usr/bin/python3",
            &[],
            credentials.clone(),
        );
        assert_eq!(system, credentials);
        let _ = std::fs::remove_dir_all(&prefix);
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
