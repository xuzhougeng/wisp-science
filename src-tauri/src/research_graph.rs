use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct ResearchGraphTool {
    store: wisp_store::Store,
    project_id: String,
}

impl ResearchGraphTool {
    pub fn new(store: wisp_store::Store, project_id: String) -> Self {
        Self { store, project_id }
    }
}

#[async_trait::async_trait]
impl Tool for ResearchGraphTool {
    fn name(&self) -> &str {
        "research_graph"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "research_graph",
            "Record a data asset, paper, or decision in the current project's research graph, or link two existing graph nodes.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["record_data_asset", "record_paper", "record_decision", "link"]
                    },
                    "title": { "type": "string" },
                    "ref_id": { "type": "string" },
                    "metadata": { "type": "object" },
                    "source_id": { "type": "string" },
                    "target_id": { "type": "string" },
                    "relation": { "type": "string" }
                },
                "required": ["action"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("action")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .into()
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(action) = args.get("action").and_then(|value| value.as_str()) else {
            return ToolResult::fail("research_graph requires action");
        };
        if action == "link" {
            return self.link(args).await;
        }
        let Some(kind) = kind_for_action(action) else {
            return ToolResult::fail("Unknown research_graph action");
        };
        let Some(title) = args.get("title").and_then(|value| value.as_str()) else {
            return ToolResult::fail("Recording a research object requires title");
        };
        let mut node = match wisp_store::ResearchNode::new(
            uuid::Uuid::new_v4().to_string(),
            &self.project_id,
            kind,
            title,
        ) {
            Ok(node) => node,
            Err(error) => return ToolResult::fail(error.to_string()),
        };
        node.ref_id = args
            .get("ref_id")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        node.metadata_json = serde_json::to_string(
            &args
                .get("metadata")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .unwrap_or_else(|_| "{}".into());
        match self.store.save_research_node(&node).await {
            Ok(()) => ToolResult::ok(serde_json::json!({ "node_id": node.id }).to_string()),
            Err(error) => ToolResult::fail(format!("research_graph error: {error}")),
        }
    }
}

impl ResearchGraphTool {
    async fn link(&self, args: &serde_json::Value) -> ToolResult {
        let Some(source_id) = args.get("source_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("Linking research objects requires source_id");
        };
        let Some(target_id) = args.get("target_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("Linking research objects requires target_id");
        };
        let Some(relation) = args.get("relation").and_then(|value| value.as_str()) else {
            return ToolResult::fail("Linking research objects requires relation");
        };
        let mut edge = match wisp_store::ResearchEdge::new(
            uuid::Uuid::new_v4().to_string(),
            &self.project_id,
            source_id,
            target_id,
            relation,
        ) {
            Ok(edge) => edge,
            Err(error) => return ToolResult::fail(error.to_string()),
        };
        edge.metadata_json = serde_json::to_string(
            &args
                .get("metadata")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .unwrap_or_else(|_| "{}".into());
        match self.store.save_research_edge(&edge).await {
            Ok(()) => ToolResult::ok(serde_json::json!({ "edge_id": edge.id }).to_string()),
            Err(error) => ToolResult::fail(format!("research_graph error: {error}")),
        }
    }
}

fn kind_for_action(action: &str) -> Option<wisp_store::ResearchNodeKind> {
    match action {
        "record_data_asset" => Some(wisp_store::ResearchNodeKind::DataAsset),
        "record_paper" => Some(wisp_store::ResearchNodeKind::Paper),
        "record_decision" => Some(wisp_store::ResearchNodeKind::Decision),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoEnv(std::path::PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &std::path::Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    #[tokio::test]
    async fn records_research_objects_and_links_them() {
        let path = std::env::temp_dir().join(format!(
            "wisp_research_graph_tool_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&path).await.unwrap();
        store
            .create_project("project", "Project", "")
            .await
            .unwrap();
        let tool = ResearchGraphTool::new(store.clone(), "project".into());
        let env = NoEnv(std::env::temp_dir());

        let data = tool
            .run(
                &serde_json::json!({
                    "action": "record_data_asset",
                    "title": "Raw counts",
                    "ref_id": "data/raw-counts",
                }),
                &env,
            )
            .await;
        assert!(data.success, "{}", data.content);
        let data_id = serde_json::from_str::<serde_json::Value>(&data.content).unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();

        let decision = tool
            .run(
                &serde_json::json!({
                    "action": "record_decision",
                    "title": "Use normalized counts",
                }),
                &env,
            )
            .await;
        assert!(decision.success, "{}", decision.content);
        let decision_id = serde_json::from_str::<serde_json::Value>(&decision.content).unwrap()
            ["node_id"]
            .as_str()
            .unwrap()
            .to_string();

        let link = tool
            .run(
                &serde_json::json!({
                    "action": "link",
                    "source_id": decision_id,
                    "target_id": data_id,
                    "relation": "selects",
                }),
                &env,
            )
            .await;
        assert!(link.success, "{}", link.content);
        let graph = store.research_graph("project").await.unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].relation, "selects");

        drop(tool);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
