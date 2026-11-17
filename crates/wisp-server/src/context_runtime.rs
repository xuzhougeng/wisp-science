//! Where a conversation's Python and R start: on this computer through the
//! runtime crate's own launcher, and on a saved SSH host as the WebView's
//! `TauriRuntimeLauncher` starts them there: the host must have passed its
//! check, the interpreter is the path kept on the host or the one its check
//! found, the worker is copied there once per checksum, and it is started
//! over the host's own SSH settings in its working folder. The paths, the
//! copy and the command are `wisp_runs::runtime_launch`, shared with the
//! WebView. WSL distros are not among this host's places to run.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use wisp_runs::{
    runtime_launch::{self, AttachedCommand, InterpreterLanguage},
    ssh_guard, ssh_hosts, RunCommandRunner,
};
use wisp_runtime::{
    KernelClient, LaunchedRuntime, RuntimeKey, RuntimeLanguage, RuntimeLauncher, RuntimeMetadata,
    LOCAL_CONTEXT_ID, PROTOCOL_VERSION,
};

/// Starts a worker from the command built for it; tests give one that
/// starts nothing.
#[async_trait]
pub(crate) trait SpawnWorker: Send + Sync {
    async fn spawn(
        &self,
        command: &AttachedCommand,
        envs: &[(String, String)],
        language: &'static str,
        interpreter: String,
    ) -> Result<LaunchedRuntime>;
}

/// Starts the worker as a process of this computer: `ssh` for a host.
struct ProcessSpawn;

#[async_trait]
impl SpawnWorker for ProcessSpawn {
    async fn spawn(
        &self,
        command: &AttachedCommand,
        envs: &[(String, String)],
        language: &'static str,
        interpreter: String,
    ) -> Result<LaunchedRuntime> {
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

/// Whether a host may be used now; the real gate is the shared one every
/// SSH use passes (OpenSSH present, a passed check, no blocked login).
type ReadyGate = fn(&wisp_store::ExecutionContext) -> Result<(), String>;

pub(crate) struct ContextRuntimeLauncher {
    local: Arc<dyn RuntimeLauncher>,
    store: wisp_store::Store,
    python_worker: PathBuf,
    r_worker: PathBuf,
    runner: Arc<dyn RunCommandRunner>,
    ready: ReadyGate,
    spawn: Arc<dyn SpawnWorker>,
}

impl ContextRuntimeLauncher {
    pub(crate) fn new(
        local: Arc<dyn RuntimeLauncher>,
        store: wisp_store::Store,
        python_worker: PathBuf,
        r_worker: PathBuf,
        runner: Arc<dyn RunCommandRunner>,
    ) -> Self {
        Self {
            local,
            store,
            python_worker,
            r_worker,
            runner,
            ready: ssh_hosts::require_managed_ssh_ready,
            spawn: Arc::new(ProcessSpawn),
        }
    }

    #[cfg(test)]
    fn with_seams(mut self, ready: ReadyGate, spawn: Arc<dyn SpawnWorker>) -> Self {
        self.ready = ready;
        self.spawn = spawn;
        self
    }
}

#[async_trait]
impl RuntimeLauncher for ContextRuntimeLauncher {
    async fn launch(&self, key: &RuntimeKey, project_root: &Path) -> Result<LaunchedRuntime> {
        if key.context_id == LOCAL_CONTEXT_ID {
            return self.local.launch(key, project_root).await;
        }
        let context = self
            .store
            .get_execution_context(&key.context_id)
            .await?
            .ok_or_else(|| anyhow!("Execution context not found: {}", key.context_id))?;
        if context.kind != wisp_store::ExecutionContextKind::Ssh {
            bail!(
                "Interpreters start on this computer or on a saved SSH host; {} is neither",
                context.id
            );
        }
        (self.ready)(&context).map_err(anyhow::Error::msg)?;
        // A host has no interpreter of this computer to fall back on.
        let (interpreter, worker, language, shared) = match key.language {
            RuntimeLanguage::Python => (
                runtime_launch::resolve_python_interpreter(&context, || None)?,
                &self.python_worker,
                "python",
                InterpreterLanguage::Python,
            ),
            RuntimeLanguage::R => (
                runtime_launch::resolve_r_interpreter(&context, || None)?,
                &self.r_worker,
                "r",
                InterpreterLanguage::R,
            ),
        };
        if !worker.is_file() {
            bail!(
                "{} runtime worker not found at {}",
                language,
                worker.display()
            );
        }
        let source = tokio::fs::read_to_string(worker).await.map_err(|error| {
            anyhow!(
                "read {language} runtime worker {}: {error}",
                worker.display()
            )
        })?;
        let remote_worker = runtime_launch::ensure_remote_worker(
            &context,
            shared,
            PROTOCOL_VERSION,
            &source,
            self.runner.as_ref(),
        )
        .await
        .map_err(|error| {
            if ssh_guard::is_authentication_failure(&error) {
                ssh_guard::record_failure(&context.id, &error);
            }
            anyhow::Error::msg(error)
        })?;
        let command = runtime_launch::build_attached_command(
            &context,
            shared,
            &interpreter,
            worker,
            Some(&remote_worker),
            project_root,
        )
        .map_err(anyhow::Error::msg)?;
        // The host's own environment is its shell's; only a password
        // sign-in is given, and taken away once the worker answered.
        let connection = ssh_hosts::SshConnection::from_execution_context(&context)
            .map_err(anyhow::Error::msg)?;
        let auth = ssh_hosts::auth_envs_for_connection(&connection).map_err(anyhow::Error::msg)?;
        let launched = self
            .spawn
            .spawn(&command, &auth, language, interpreter)
            .await;
        ssh_hosts::cleanup_password_auth_env(&auth);
        match &launched {
            Ok(_) => ssh_guard::record_success(&context.id),
            Err(error) => {
                let detail = error.to_string();
                if ssh_guard::is_authentication_failure(&detail) {
                    ssh_guard::record_failure(&context.id, &detail);
                }
            }
        }
        launched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex, time::Duration};
    use wisp_runs::{RunCommand, RunCommandOutput};

    /// Answers each command with the next exit code and keeps what it was
    /// asked; nothing is run.
    struct FakeRunner {
        exits: Mutex<VecDeque<i64>>,
        commands: Mutex<Vec<RunCommand>>,
    }

    #[async_trait]
    impl RunCommandRunner for FakeRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            self.commands.lock().unwrap().push(command);
            let exit_code = self
                .exits
                .lock()
                .unwrap()
                .pop_front()
                .ok_or("unexpected command")?;
            Ok(RunCommandOutput {
                exit_code,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    struct Idle;

    #[async_trait]
    impl wisp_runtime::RuntimeKernel for Idle {
        async fn execute(
            &mut self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &[String],
            _: &wisp_runtime::RuntimeOutput,
        ) -> Result<wisp_runtime::KernelResp> {
            Ok(Default::default())
        }
        async fn inspect(&mut self, _: &str) -> Result<wisp_runtime::RuntimeObjectList> {
            Ok(Default::default())
        }
        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Keeps the command it was given and starts nothing.
    #[derive(Default)]
    struct Recorded(Mutex<Vec<(AttachedCommand, &'static str, String)>>);

    #[async_trait]
    impl SpawnWorker for Recorded {
        async fn spawn(
            &self,
            command: &AttachedCommand,
            _: &[(String, String)],
            language: &'static str,
            interpreter: String,
        ) -> Result<LaunchedRuntime> {
            self.0
                .lock()
                .unwrap()
                .push((command.clone(), language, interpreter.clone()));
            Ok(LaunchedRuntime::new(
                Box::new(Idle),
                RuntimeMetadata {
                    interpreter: Some(interpreter),
                    ..Default::default()
                },
            ))
        }
    }

    /// Stands for this computer's launcher.
    struct Local(Mutex<Vec<String>>);

    #[async_trait]
    impl RuntimeLauncher for Local {
        async fn launch(&self, key: &RuntimeKey, _: &Path) -> Result<LaunchedRuntime> {
            self.0.lock().unwrap().push(key.context_id.clone());
            Ok(LaunchedRuntime::new(Box::new(Idle), Default::default()))
        }
    }

    fn ready(_: &wisp_store::ExecutionContext) -> Result<(), String> {
        Ok(())
    }

    fn not_checked(context: &wisp_store::ExecutionContext) -> Result<(), String> {
        Err(format!("no successful probe yet for {}", context.id))
    }

    #[tokio::test]
    async fn a_host_interpreter_starts_over_ssh_and_this_computer_keeps_its_own() {
        let temp = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&temp.path().join("wisp.sqlite"))
            .await
            .unwrap();
        let worker = temp.path().join("kernel_worker.py");
        std::fs::write(&worker, "print('worker')\n").unwrap();
        let r_worker = temp.path().join("kernel_worker.R");
        std::fs::write(&r_worker, "cat('worker')\n").unwrap();
        let mut host = wisp_store::ExecutionContext::new("ssh:launch-test-gpu", "gpu").unwrap();
        host.kind = wisp_store::ExecutionContextKind::Ssh;
        host.config_json = serde_json::json!({
            "user": "alice",
            "host_name": "gpu.example",
            "workdir": "/scratch/my project",
            "python_executable": "/opt/conda/bin/python"
        })
        .to_string();
        host.capabilities_json = serde_json::json!({
            "rscript_executable": "/usr/bin/Rscript",
            "r_jsonlite": true
        })
        .to_string();
        store.upsert_execution_context(&host).await.unwrap();
        let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        wsl.kind = wisp_store::ExecutionContextKind::Wsl;
        store.upsert_execution_context(&wsl).await.unwrap();

        // The worker is not there yet: checked, then copied.
        let runner = Arc::new(FakeRunner {
            exits: Mutex::new([1, 0, 0].into()),
            commands: Mutex::default(),
        });
        let local = Arc::new(Local(Mutex::default()));
        let spawned = Arc::new(Recorded::default());
        let launcher = ContextRuntimeLauncher::new(
            local.clone(),
            store.clone(),
            worker,
            r_worker,
            runner.clone(),
        )
        .with_seams(ready, spawned.clone());
        let root = temp.path().join("project");

        let python = RuntimeKey::python("project", "ssh:launch-test-gpu");
        let launched = launcher.launch(&python, &root).await.unwrap();
        drop(launched);
        {
            let commands = runner.commands.lock().unwrap();
            assert_eq!(commands.len(), 2);
            assert_eq!(commands[0].program, "ssh");
            assert_eq!(commands[0].script, "check python runtime worker");
            assert_eq!(commands[1].script, "deploy python runtime worker");
            assert_eq!(commands[1].stdin.as_deref(), Some("print('worker')\n"));
            assert!(commands[1].args.contains(&"alice@gpu.example".to_owned()));
        }
        {
            let spawned = spawned.0.lock().unwrap();
            let (command, language, interpreter) = &spawned[0];
            assert_eq!(*language, "python");
            // The path kept on the host wins over what a check found.
            assert_eq!(interpreter, "/opt/conda/bin/python");
            assert_eq!(command.program, PathBuf::from("ssh"));
            let script = command.args.last().unwrap().to_string_lossy().into_owned();
            assert!(
                script.starts_with("cd '/scratch/my project' && exec '/opt/conda/bin/python' "),
                "{script}"
            );
            assert!(script.contains("\"$HOME\"/'.wisp-science/runtime/python-v"));
        }

        // R uses what the check found; its worker is already there.
        let r = RuntimeKey::r("project", "ssh:launch-test-gpu");
        drop(launcher.launch(&r, &root).await.unwrap());
        assert_eq!(runner.commands.lock().unwrap().len(), 3);
        {
            let spawned = spawned.0.lock().unwrap();
            let (command, language, interpreter) = &spawned[1];
            assert_eq!((*language, interpreter.as_str()), ("r", "/usr/bin/Rscript"));
            let script = command.args.last().unwrap().to_string_lossy().into_owned();
            assert!(script.contains("--vanilla"), "{script}");
        }

        // This computer goes to its own launcher; WSL and unknown places
        // are refused before anything runs.
        drop(
            launcher
                .launch(&RuntimeKey::python("project", LOCAL_CONTEXT_ID), &root)
                .await
                .unwrap(),
        );
        assert_eq!(*local.0.lock().unwrap(), vec!["local".to_owned()]);
        for (id, said) in [
            ("wsl:Ubuntu", "is neither"),
            ("ssh:launch-test-missing", "not found"),
        ] {
            let error = match launcher
                .launch(&RuntimeKey::python("project", id), &root)
                .await
            {
                Ok(_) => panic!("{id} started"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains(said), "{error}");
        }
        assert_eq!(runner.commands.lock().unwrap().len(), 3);
        assert_eq!(spawned.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_host_that_has_not_passed_a_check_or_has_no_python_starts_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&temp.path().join("wisp.sqlite"))
            .await
            .unwrap();
        let worker = temp.path().join("kernel_worker.py");
        std::fs::write(&worker, "print('worker')\n").unwrap();
        let mut host = wisp_store::ExecutionContext::new("ssh:launch-test-bare", "bare").unwrap();
        host.kind = wisp_store::ExecutionContextKind::Ssh;
        store.upsert_execution_context(&host).await.unwrap();
        let runner = Arc::new(FakeRunner {
            exits: Mutex::default(),
            commands: Mutex::default(),
        });
        let spawned = Arc::new(Recorded::default());
        let key = RuntimeKey::python("project", "ssh:launch-test-bare");
        let unchecked = ContextRuntimeLauncher::new(
            Arc::new(Local(Mutex::default())),
            store.clone(),
            worker.clone(),
            temp.path().join("missing.R"),
            runner.clone(),
        )
        .with_seams(not_checked, spawned.clone());
        let error = match unchecked.launch(&key, temp.path()).await {
            Ok(_) => panic!("started without a check"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("no successful probe"), "{error}");

        let checked = ContextRuntimeLauncher::new(
            Arc::new(Local(Mutex::default())),
            store,
            worker,
            temp.path().join("missing.R"),
            runner.clone(),
        )
        .with_seams(ready, spawned.clone());
        let error = match checked.launch(&key, temp.path()).await {
            Ok(_) => panic!("started without a Python"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("Python interpreter is unknown"), "{error}");
        assert!(runner.commands.lock().unwrap().is_empty());
        assert!(spawned.0.lock().unwrap().is_empty());
    }
}
