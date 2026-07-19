//! `propose_delegation` — an opt-in main-Agent tool that creates a persisted
//! draft workflow. Approval and execution remain explicit UI actions.

use crate::{delegation_runtime, ActiveProject};
use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub(crate) fn propose_delegation_schema() -> ToolSchema {
    ToolSchema::new(
        "propose_delegation",
        "Create a persisted draft plan for controlled sub-Agent delegation when the user's task materially benefits from parallel code, biology, visualization, or independent review. This only proposes a plan: it never approves or runs Agents. After creating it, tell the user to review the Agents panel.",
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Concrete delegated outcome, including the evidence or artifact the user expects"
                },
                "mode": {
                    "type": "string",
                    "enum": ["manual", "assisted", "automatic"],
                    "description": "Planning mode; defaults to assisted"
                },
                "agents": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["code_execution", "biology_interpreter", "visualization", "reviewer"]
                    },
                    "description": "Required ordered Agent template ids in manual mode; ignored for assisted and automatic planning"
                }
            },
            "required": ["goal"]
        }),
    )
}

pub(crate) struct ProposeDelegationTool {
    store: Store,
    project_id: String,
    project_root: std::path::PathBuf,
    frame_id: String,
}

impl ProposeDelegationTool {
    pub(crate) fn new(store: Store, project: ActiveProject, frame_id: impl Into<String>) -> Self {
        Self::for_project(store, project.id, project.root, frame_id)
    }

    pub(crate) fn for_project(
        store: Store,
        project_id: impl Into<String>,
        project_root: impl Into<std::path::PathBuf>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            project_root: project_root.into(),
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for ProposeDelegationTool {
    fn name(&self) -> &str {
        "propose_delegation"
    }

    fn schema(&self) -> ToolSchema {
        propose_delegation_schema()
    }

    fn preview(&self, args: &Value) -> String {
        args.get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if goal.is_empty() {
            return ToolResult::fail("propose_delegation error: 'goal' is required");
        }
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("assisted");
        let template_ids = args
            .get("agents")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        match delegation_runtime::create_agent_workflow_draft(
            &self.store,
            &self.project_id,
            &self.project_root,
            self.frame_id.clone(),
            goal,
            mode,
            &template_ids,
        )
        .await
        {
            Ok(snapshot) => ToolResult::ok(format!(
                "Created draft Agent workflow '{}' with {} controlled steps. No Agent has started. Ask the user to review and approve it in the Agents panel.",
                snapshot.workflow.id,
                snapshot.steps.len(),
            )),
            Err(error) => ToolResult::fail(format!("propose_delegation error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoEnv(std::path::PathBuf);

    #[async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &std::path::Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    async fn fixture() -> (Store, ActiveProject, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("wisp_delegation_tool_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("store.sqlite");
        let store = Store::open(&database).await.unwrap();
        store
            .create_project("p", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("f", "p", "OPERON", "wisp")
            .await
            .unwrap();
        let project = ActiveProject {
            id: "p".into(),
            root: root.clone(),
            skills: std::sync::Arc::new(wisp_skills::SkillIndex::load(&[])),
            memory: std::sync::Arc::new(wisp_core::MemoryManager::new(&root)),
        };
        (store, project, root)
    }

    #[tokio::test]
    async fn disabled_session_cannot_propose_a_workflow() {
        let (store, project, root) = fixture().await;
        let tool = ProposeDelegationTool::new(store.clone(), project, "f");
        let result = tool
            .run(
                &json!({
                    "goal": "analyze code and create a visualization",
                    "mode": "manual",
                    "agents": ["code_execution", "reviewer"]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("delegation is off"));
        assert!(store.list_agent_workflows("p").await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn enabled_session_can_only_create_a_draft() {
        let (store, project, root) = fixture().await;
        delegation_runtime::save_session_delegation_enabled(&store, "p", "f", true)
            .await
            .unwrap();
        let tool = ProposeDelegationTool::new(store.clone(), project, "f");
        let result = tool
            .run(
                &json!({
                    "goal": "analyze code and create a visualization",
                    "mode": "manual",
                    "agents": ["code_execution", "reviewer"]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let workflows = store.list_agent_workflows("p").await.unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].status, wisp_store::AgentWorkflowStatus::Draft);
        assert_eq!(workflows[0].frame_id.as_deref(), Some("f"));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
