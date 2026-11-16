//! A conversation's interpreters from the window, a port of the WebView's
//! runtime cards (`src-tauri/src/runtime_commands.rs`): start one, see the
//! variables it holds, restart it, and run code in it from a console with
//! its printed output and plots. They are the same interpreters the
//! model's `python` and `r` tools use in that conversation, keyed the same
//! way, so what one defines the other sees. Only interpreters on this
//! computer are started here.

use anyhow::{bail, ensure, Context as _, Result};
use wisp_runtime::{RuntimeEvent, RuntimeInfo, RuntimeKey, RuntimeManager, LOCAL_CONTEXT_ID};

use crate::desktop::RuntimeLanguage;
use crate::Server;

pub use wisp_runtime::{RuntimeObject, RuntimeObjectList};

/// What running code in an interpreter printed, and the plots it drew as
/// PNG bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeOutput {
    pub text: String,
    pub plots: Vec<Vec<u8>>,
}

impl Server {
    /// The conversation's interpreters and the folder they work in: the
    /// exploration's own copy for an exploration's conversation.
    async fn conversation_runtimes(
        &self,
        project: &str,
        session: &str,
    ) -> Result<(RuntimeManager, String)> {
        self.own_conversation(project, session).await?;
        let (working, scope) = crate::explorations::service::working_project_in(
            &self.0.store,
            &self.0.config.data,
            session,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let root = match scope {
            wisp_store::StateScope::Exploration { .. } => {
                working.root.to_string_lossy().into_owned()
            }
            _ => self.project_root(project).await?,
        };
        let manager = crate::runtime::session_runtimes(
            &self.0.config,
            &self.session(session),
            self.service_keys().await,
        );
        Ok((manager, root))
    }

    async fn conversation_runtime(
        &self,
        project: &str,
        session: &str,
        id: &str,
    ) -> Result<(RuntimeManager, RuntimeInfo, String)> {
        let (manager, root) = self.conversation_runtimes(project, session).await?;
        let info = manager
            .list()
            .into_iter()
            .find(|info| info.runtime_id == id)
            .context("That interpreter is no longer there")?;
        Ok((manager, info, root))
    }

    /// Starts the conversation's Python or R on this computer, the one the
    /// model's tool would use, unless it is already there.
    pub async fn start_runtime(
        &self,
        project: &str,
        session: &str,
        language: RuntimeLanguage,
    ) -> Result<String> {
        let (manager, root) = self.conversation_runtimes(project, session).await?;
        let (worker, r_worker) = crate::runtime::workers();
        let key = match language {
            RuntimeLanguage::Python => {
                ensure!(worker.is_file(), "Python is not set up for this app");
                RuntimeKey::python(root.clone(), LOCAL_CONTEXT_ID)
            }
            RuntimeLanguage::R => {
                ensure!(r_worker.is_file(), "R is not set up for this app");
                RuntimeKey::r(root.clone(), LOCAL_CONTEXT_ID)
            }
        };
        match manager.list().into_iter().find(|info| info.key == key) {
            Some(info) if info.status != wisp_runtime::RuntimeStatus::Dead => Ok(info.runtime_id),
            // One that stopped, or could not start (say Python was not
            // there yet), is started afresh rather than reported dead.
            Some(_) => Ok(manager.restart(key, root.into()).await?.runtime_id),
            None => Ok(manager.start(key, root.into()).await?.runtime_id),
        }
    }

    /// The variables an interpreter holds, with their type and a summary.
    pub async fn runtime_objects(
        &self,
        project: &str,
        session: &str,
        id: &str,
    ) -> Result<RuntimeObjectList> {
        let (manager, info, _) = self.conversation_runtime(project, session, id).await?;
        manager.inspect(&info.key).await
    }

    /// Starts an interpreter afresh: what it held in memory is lost, files
    /// it wrote stay. Returns the new interpreter's id.
    pub async fn restart_runtime(&self, project: &str, session: &str, id: &str) -> Result<String> {
        let (manager, info, root) = self.conversation_runtime(project, session, id).await?;
        Ok(manager.restart(info.key, root.into()).await?.runtime_id)
    }

    /// Runs code in an interpreter, as the researcher's own; returns what
    /// it printed (an error included) and its plots.
    pub async fn run_in_runtime(
        &self,
        project: &str,
        session: &str,
        id: &str,
        code: &str,
    ) -> Result<RuntimeOutput> {
        ensure!(!code.trim().is_empty(), "Write some code first");
        ensure!(
            code.len() <= wisp_runtime::MAX_CODE_BYTES,
            "The code is longer than the {} bytes an interpreter takes at once",
            wisp_runtime::MAX_CODE_BYTES
        );
        let (manager, info, root) = self.conversation_runtime(project, session, id).await?;
        let store = &self.0.store;
        let (_, scope) =
            crate::explorations::service::working_project_in(store, &self.0.config.data, session)
                .await
                .map_err(anyhow::Error::msg)?;
        ensure!(
            !crate::explorations::service::conversation_project_write_locked(
                store,
                &scope,
                Some(session)
            )
            .await
            .map_err(anyhow::Error::msg)?,
            "The project is frozen while an exploration runs"
        );
        if let Some(boundary) = wisp_runs::exploration_isolation::boundary_for_scope(store, &scope)
            .await
            .map_err(anyhow::Error::msg)?
        {
            boundary
                .check_local_source(code)
                .map_err(anyhow::Error::msg)?;
        }
        let mut execution = manager
            .execute(&info.key, std::path::Path::new(&root), code.to_owned())
            .await?;
        loop {
            match execution.recv().await {
                Some(RuntimeEvent::Stdout(_)) => {}
                Some(RuntimeEvent::Finished(result)) => {
                    let response = result.map_err(anyhow::Error::msg)?;
                    use base64::Engine as _;
                    // A plot that cannot be read is left out.
                    let plots = response
                        .plots
                        .iter()
                        .filter_map(|plot| {
                            base64::engine::general_purpose::STANDARD
                                .decode(plot.trim())
                                .ok()
                        })
                        .collect();
                    return Ok(RuntimeOutput {
                        text: wisp_runtime::format_response(&response),
                        plots,
                    });
                }
                None => bail!("The interpreter ended before returning a result"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn an_interpreter_that_is_not_there_is_said_and_code_is_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let server = Server::open_with_models(
            crate::tests::config(temp.path()),
            crate::tests::models(Arc::default(), false),
        )
        .await
        .unwrap();
        let session = server.create_conversation(crate::PROJECT).await.unwrap();
        let gone = server
            .runtime_objects(crate::PROJECT, &session, "missing")
            .await
            .unwrap_err()
            .to_string();
        assert!(gone.contains("no longer there"), "{gone}");
        assert!(server
            .restart_runtime(crate::PROJECT, &session, "missing")
            .await
            .is_err());
        assert!(server
            .run_in_runtime(crate::PROJECT, &session, "missing", "  ")
            .await
            .unwrap_err()
            .to_string()
            .contains("Write some code"));
        let long = "x".repeat(wisp_runtime::MAX_CODE_BYTES + 1);
        assert!(server
            .run_in_runtime(crate::PROJECT, &session, "missing", &long)
            .await
            .unwrap_err()
            .to_string()
            .contains("bytes"));
        assert!(server
            .runtime_objects("other", &session, "missing")
            .await
            .is_err());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn an_interpreter_that_could_not_start_starts_once_it_can() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Idle;
        #[async_trait::async_trait]
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
            async fn inspect(&mut self, _: &str) -> Result<RuntimeObjectList> {
                Ok(Default::default())
            }
            async fn shutdown(&mut self) -> Result<()> {
                Ok(())
            }
        }
        /// Fails until Python is "installed".
        struct Launcher(Arc<AtomicBool>);
        #[async_trait::async_trait]
        impl wisp_runtime::RuntimeLauncher for Launcher {
            async fn launch(
                &self,
                _: &RuntimeKey,
                _: &std::path::Path,
            ) -> Result<wisp_runtime::LaunchedRuntime> {
                ensure!(
                    self.0.load(Ordering::SeqCst),
                    "Python interpreter not found"
                );
                Ok(wisp_runtime::LaunchedRuntime::new(
                    Box::new(Idle),
                    wisp_runtime::RuntimeMetadata::default(),
                ))
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let server = Server::open_with_models(
            crate::tests::config(temp.path()),
            crate::tests::models(Arc::default(), false),
        )
        .await
        .unwrap();
        let session = server.create_conversation(crate::PROJECT).await.unwrap();
        let installed = Arc::new(AtomicBool::new(false));
        let manager = RuntimeManager::new(Arc::new(Launcher(installed.clone())));
        *server.session(&session).runtime.lock().unwrap() = Some(manager);
        let missing = server
            .start_runtime(crate::PROJECT, &session, RuntimeLanguage::Python)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            missing.contains("Python interpreter not found"),
            "{missing}"
        );
        // Once Python is there, asking again starts it rather than saying
        // the first try died.
        installed.store(true, Ordering::SeqCst);
        let id = server
            .start_runtime(crate::PROJECT, &session, RuntimeLanguage::Python)
            .await
            .unwrap();
        // And asking while it runs gives the same one.
        assert_eq!(
            server
                .start_runtime(crate::PROJECT, &session, RuntimeLanguage::Python)
                .await
                .unwrap(),
            id
        );
        // Restarted, it is a new interpreter, and the old one is gone.
        let restarted = server
            .restart_runtime(crate::PROJECT, &session, &id)
            .await
            .unwrap();
        assert_ne!(restarted, id);
        assert!(server
            .runtime_objects(crate::PROJECT, &session, &id)
            .await
            .is_err());
        assert!(server
            .runtime_objects(crate::PROJECT, &session, &restarted)
            .await
            .is_ok());
        server.shutdown().await;
    }
}
