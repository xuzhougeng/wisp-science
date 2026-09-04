//! Live per-session authorization for agent tools that accept `context_id`.

use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct SessionExecutionContextTool {
    inner: Box<dyn Tool>,
    store: wisp_store::Store,
    frame_id: String,
}

impl SessionExecutionContextTool {
    pub fn new(
        inner: Box<dyn Tool>,
        store: wisp_store::Store,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            store,
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait::async_trait]
impl Tool for SessionExecutionContextTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn schema(&self) -> ToolSchema {
        self.inner.schema()
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        self.inner.preview(args)
    }

    async fn before(&self, args: &serde_json::Value, env: &dyn ToolEnv) {
        self.inner.before(args, env).await;
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let explicit = args
            .get("context_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty());
        // With no explicit `context_id`, use this conversation's resolved
        // default (session snapshot > global > local).
        let default = if explicit.is_none() {
            crate::ssh_hosts::resolved_session_execution_context_id(&self.store, &self.frame_id)
                .await
        } else {
            None
        };
        let injected = explicit.is_none() && default.is_some();
        let context_id = explicit
            .map(str::to_string)
            .or(default)
            .unwrap_or_else(|| "local".to_string());
        if context_id != "local" {
            match self
                .store
                .session_execution_context_enabled(&self.frame_id, &context_id)
                .await
            {
                Ok(true) => {}
                Ok(false) if injected => {
                    // First use of the default in this session: select it so
                    // subsequent checks and the compute UI reflect it.
                    if let Err(error) = self
                        .store
                        .set_session_execution_context_enabled(&self.frame_id, &context_id, true)
                        .await
                    {
                        return ToolResult::fail(format!(
                            "Unable to select default execution context {context_id}: {error}"
                        ));
                    }
                }
                Ok(false) => {
                    return ToolResult::fail(format!(
                        "Execution context {context_id} is not selected for this session"
                    ))
                }
                Err(error) => {
                    return ToolResult::fail(format!(
                        "Unable to check execution context access: {error}"
                    ))
                }
            }
        }
        if injected {
            let mut injected_args = args.as_object().cloned().unwrap_or_default();
            injected_args.insert(
                "context_id".to_string(),
                serde_json::Value::String(context_id),
            );
            self.inner
                .run(&serde_json::Value::Object(injected_args), env)
                .await
        } else {
            self.inner.run(args, env).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[derive(Clone, Default)]
    struct EchoTool(std::sync::Arc<std::sync::Mutex<Option<serde_json::Value>>>);

    impl EchoTool {
        fn last_args(&self) -> Option<serde_json::Value> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo_context"
        }

        fn schema(&self) -> ToolSchema {
            ToolSchema::new(
                "echo_context",
                "test",
                serde_json::json!({ "type": "object" }),
            )
        }

        async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
            *self.0.lock().unwrap() = Some(args.clone());
            ToolResult::ok("ran")
        }
    }

    struct TestEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for TestEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    #[tokio::test]
    async fn remote_context_requires_selection_in_the_same_session() {
        let path =
            std::env::temp_dir().join(format!("wisp_session_tool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
            .await
            .unwrap();
        let tool =
            SessionExecutionContextTool::new(Box::new(EchoTool::default()), store.clone(), "f");
        let env = TestEnv(PathBuf::from("."));

        let denied = tool
            .run(&serde_json::json!({ "context_id": "ssh:gpu" }), &env)
            .await;
        assert!(!denied.success);
        assert!(denied.content.contains("not selected for this session"));

        store
            .set_session_execution_context_enabled("f", "ssh:gpu", true)
            .await
            .unwrap();
        assert!(
            tool.run(&serde_json::json!({ "context_id": "ssh:gpu" }), &env)
                .await
                .success
        );
        assert!(tool.run(&serde_json::json!({}), &env).await.success);

        let _ = std::fs::remove_file(path);
    }

    async fn default_test_store() -> (wisp_store::Store, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("wisp_session_tool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
            .await
            .unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn omitted_context_id_uses_default_and_selects_it_for_the_session() {
        let (store, path) = default_test_store().await;
        store
            .set_setting(crate::ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY, "ssh:gpu")
            .await
            .unwrap();
        let echo = EchoTool::default();
        let tool = SessionExecutionContextTool::new(Box::new(echo.clone()), store.clone(), "f");
        let env = TestEnv(PathBuf::from("."));

        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert_eq!(
            echo.last_args().unwrap().get("context_id").unwrap(),
            "ssh:gpu"
        );
        assert!(store
            .session_execution_context_enabled("f", "ssh:gpu")
            .await
            .unwrap());

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn omitted_context_id_prefers_session_snapshot_over_global() {
        let (store, path) = default_test_store().await;
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:cpu", "CPU").unwrap())
            .await
            .unwrap();
        store
            .set_setting(crate::ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY, "ssh:gpu")
            .await
            .unwrap();
        crate::ssh_hosts::persist_session_default_execution_context(
            &store,
            "f",
            crate::ssh_hosts::SessionDefaultExecutionContext::Remote("ssh:cpu".into()),
        )
        .await
        .unwrap();
        let echo = EchoTool::default();
        let tool = SessionExecutionContextTool::new(Box::new(echo.clone()), store.clone(), "f");
        let env = TestEnv(PathBuf::from("."));

        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert_eq!(
            echo.last_args().unwrap().get("context_id").unwrap(),
            "ssh:cpu"
        );

        crate::ssh_hosts::persist_session_default_execution_context(
            &store,
            "f",
            crate::ssh_hosts::SessionDefaultExecutionContext::Local,
        )
        .await
        .unwrap();
        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert!(echo.last_args().unwrap().get("context_id").is_none());

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn stale_default_falls_back_to_local() {
        let (store, path) = default_test_store().await;
        store
            .set_setting(crate::ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY, "ssh:gpu")
            .await
            .unwrap();
        store.delete_execution_context("ssh:gpu").await.unwrap();
        let echo = EchoTool::default();
        let tool = SessionExecutionContextTool::new(Box::new(echo.clone()), store.clone(), "f");
        let env = TestEnv(PathBuf::from("."));

        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert!(echo.last_args().unwrap().get("context_id").is_none());

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn explicit_context_id_is_unaffected_by_default() {
        let (store, path) = default_test_store().await;
        store
            .set_setting(crate::ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY, "ssh:gpu")
            .await
            .unwrap();
        let echo = EchoTool::default();
        let tool = SessionExecutionContextTool::new(Box::new(echo.clone()), store.clone(), "f");
        let env = TestEnv(PathBuf::from("."));

        assert!(
            tool.run(&serde_json::json!({ "context_id": "local" }), &env)
                .await
                .success
        );
        assert_eq!(
            echo.last_args().unwrap().get("context_id").unwrap(),
            "local"
        );
        // The default being configured does not authorize an explicit,
        // unselected remote context.
        let denied = tool
            .run(&serde_json::json!({ "context_id": "ssh:gpu" }), &env)
            .await;
        assert!(!denied.success);
        assert!(denied.content.contains("not selected for this session"));

        let _ = std::fs::remove_file(path);
    }
}
