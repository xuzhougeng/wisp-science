//! Agent tool for saving an interpreter on an existing ExecutionContext.
//!
//! The tool is `wisp_runs::runtime_config_tool`, shared with the embedded
//! host; this file hands it the desktop's runtime manager.

use std::path::PathBuf;

use wisp_runs::{
    runtime_config_tool::{InterpreterKey, InterpreterRuntimes},
    runtime_launch::InterpreterLanguage,
};
use wisp_runtime::{RuntimeKey, RuntimeLanguage, RuntimeManager};
use wisp_store::Store;

pub use wisp_runs::runtime_config_tool::SetRuntimeInterpreterTool;

/// The desktop's runtime manager, as the shared tool asks for it.
struct ManagedRuntimes(RuntimeManager);

fn runtime_key(key: &InterpreterKey) -> RuntimeKey {
    RuntimeKey {
        project_id: key.project_id.clone(),
        scope_key: key.scope_key.clone(),
        session_id: key.session_id.clone(),
        context_id: key.context_id.clone(),
        language: match key.language {
            InterpreterLanguage::Python => RuntimeLanguage::Python,
            InterpreterLanguage::R => RuntimeLanguage::R,
        },
    }
}

#[async_trait::async_trait]
impl InterpreterRuntimes for ManagedRuntimes {
    fn has_runtime(&self, key: &InterpreterKey) -> bool {
        let key = runtime_key(key);
        self.0.list().iter().any(|runtime| runtime.key == key)
    }

    async fn restart(&self, key: &InterpreterKey, project_root: PathBuf) -> anyhow::Result<()> {
        self.0
            .restart(runtime_key(key), project_root)
            .await
            .map(drop)
    }
}

pub fn new_in_session(
    store: Store,
    runtime_manager: RuntimeManager,
    project_id: impl Into<String>,
    scope_key: impl Into<String>,
    session_id: impl Into<String>,
) -> SetRuntimeInterpreterTool {
    SetRuntimeInterpreterTool::new_in_session(
        store,
        Box::new(ManagedRuntimes(runtime_manager)),
        project_id,
        scope_key,
        session_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_tool_keys_the_desktops_runtimes_as_its_repl_does() {
        // The shared tool tells the mainline by the store's key, the
        // runtime manager by its own; they must be one.
        assert_eq!(
            wisp_store::MAINLINE_SCOPE_KEY,
            wisp_runtime::MAINLINE_RUNTIME_SCOPE
        );
        let key = runtime_key(&InterpreterKey {
            project_id: "project-1".into(),
            scope_key: wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
            session_id: "frame-1".into(),
            context_id: "ssh:CPU2".into(),
            language: InterpreterLanguage::R,
        });
        assert_eq!(
            key,
            RuntimeKey {
                project_id: "project-1".into(),
                scope_key: wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
                session_id: "frame-1".into(),
                context_id: "ssh:CPU2".into(),
                language: RuntimeLanguage::R,
            }
        );
    }
}
