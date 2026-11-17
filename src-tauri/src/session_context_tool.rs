//! Live per-session authorization for agent tools that accept `context_id`.
//! The wrapper lives in [`wisp_runs::session_context_tool`], shared with the
//! embedded host; the subagent test stays here with `subagent_tool`.

pub use wisp_runs::session_context_tool::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use wisp_llm::ToolSchema;
    use wisp_tools::{Tool, ToolEnv, ToolResult};

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
    async fn subagent_dispatch_inherits_compute_and_follow_up_refreshes_access() {
        let dir = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&dir.path().join("wisp.sqlite"))
            .await
            .unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        for frame in ["parent", "child", "other"] {
            store.create_frame(frame, "p", "OPERON", "m").await.unwrap();
        }
        for context in ["ssh:CPU3", "ssh:gpu", "ssh:unrelated"] {
            store
                .upsert_execution_context(
                    &wisp_store::ExecutionContext::new(context, context).unwrap(),
                )
                .await
                .unwrap();
        }
        store
            .set_session_dispatched_from("child", "parent")
            .await
            .unwrap();
        store
            .set_session_execution_context_enabled("parent", "ssh:CPU3", true)
            .await
            .unwrap();
        crate::ssh_hosts::persist_session_default_execution_context(
            &store,
            "parent",
            crate::ssh_hosts::SessionDefaultExecutionContext::Remote("ssh:gpu".into()),
        )
        .await
        .unwrap();
        // A fresh child initially snapshots the global server. That snapshot
        // must not survive when its owning parent uses a different server.
        store
            .set_setting(
                crate::ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY,
                "ssh:unrelated",
            )
            .await
            .unwrap();
        crate::ssh_hosts::snapshot_session_default_from_global(&store, "child")
            .await
            .unwrap();
        let echo = EchoTool::default();
        let tool = SessionExecutionContextTool::new(Box::new(echo.clone()), store.clone(), "child");
        let env = TestEnv(dir.path().into());
        let cpu3 = serde_json::json!({ "context_id": "ssh:CPU3" });
        assert!(!tool.run(&cpu3, &env).await.success);
        assert!(
            crate::subagent_tool::prepare_execution_contexts(&store, "other", "child")
                .await
                .is_err()
        );
        assert!(!tool.run(&cpu3, &env).await.success);

        crate::subagent_tool::prepare_execution_contexts(&store, "parent", "child")
            .await
            .unwrap();
        assert!(tool.run(&cpu3, &env).await.success);
        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert_eq!(echo.last_args().unwrap()["context_id"], "ssh:gpu");
        assert!(
            !tool
                .run(&serde_json::json!({ "context_id": "ssh:unrelated" }), &env)
                .await
                .success
        );

        // The same child and tool instance pick up revocation and a new local
        // default on follow-up; unrelated conversations remain unchanged.
        store
            .set_session_execution_context_enabled("parent", "ssh:CPU3", false)
            .await
            .unwrap();
        crate::ssh_hosts::persist_session_default_execution_context(
            &store,
            "parent",
            crate::ssh_hosts::SessionDefaultExecutionContext::Local,
        )
        .await
        .unwrap();
        crate::subagent_tool::prepare_execution_contexts(&store, "parent", "child")
            .await
            .unwrap();
        assert!(!tool.run(&cpu3, &env).await.success);
        assert!(tool.run(&serde_json::json!({}), &env).await.success);
        assert!(echo.last_args().unwrap().get("context_id").is_none());
        assert!(store
            .list_session_execution_context_ids("other")
            .await
            .unwrap()
            .is_empty());

        // Legacy parents without a pinned default use the resolved global
        // target; make that available for explicit context_id calls too.
        store
            .set_session_default_execution_context("parent", None)
            .await
            .unwrap();
        crate::subagent_tool::prepare_execution_contexts(&store, "parent", "child")
            .await
            .unwrap();
        assert!(
            tool.run(&serde_json::json!({ "context_id": "ssh:unrelated" }), &env)
                .await
                .success
        );
        assert_eq!(
            store
                .session_default_execution_context("child")
                .await
                .unwrap()
                .as_deref(),
            Some("ssh:unrelated")
        );
    }
}
