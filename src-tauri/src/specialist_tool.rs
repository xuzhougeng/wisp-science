//! `save_specialist` — lets the agent create a specialist from a chat
//! conversation ("Chat with Claude" creation flow). Create-only: editing and
//! deletion stay in the Settings UI, which keeps builtin rows unreachable.

use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct SaveSpecialistTool {
    pub store: Store,
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or_default().trim().to_string()
}

fn list_arg(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)?.as_array().map(|a| {
        a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect()
    })
}

#[async_trait]
impl Tool for SaveSpecialistTool {
    fn name(&self) -> &str {
        "save_specialist"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "save_specialist",
            "Create a new specialist (agent persona) from this conversation: a name, \
             instructions appended to the base prompt, an optional bound model id, and \
             optional skill/connector whitelists. Use after interviewing the user about \
             what the specialist is for. Creates only — never edits existing specialists.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name, e.g. 'Release notes writer'" },
                    "description": { "type": "string", "description": "One-line summary shown in settings (not in the prompt)" },
                    "instructions": { "type": "string", "description": "Persona instructions appended to the base system prompt" },
                    "model_id": { "type": "string", "description": "Model profile id to bind; omit to follow the active model" },
                    "skills": { "type": "array", "items": {"type": "string"}, "description": "Skill-name whitelist; omit to inherit project settings" },
                    "connectors": { "type": "array", "items": {"type": "string"}, "description": "Connector/MCP whitelist; omit to inherit" }
                },
                "required": ["name", "instructions"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        str_arg(args, "name")
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let name = str_arg(args, "name");
        if name.is_empty() {
            return ToolResult::fail("save_specialist error: 'name' is required");
        }
        let spec = crate::specialists::Specialist {
            id: String::new(), // create-only
            name,
            icon: "review".into(),
            color: "clay".into(),
            description: str_arg(args, "description"),
            instructions: str_arg(args, "instructions"),
            model_id: str_arg(args, "model_id"),
            skills: list_arg(args, "skills"),
            connectors: list_arg(args, "connectors"),
            builtin: false,
        };
        match crate::specialists::upsert(&self.store, spec).await {
            Ok(list) => {
                let created = list.iter().rev().find(|s| !s.builtin).cloned();
                ToolResult::ok(format!(
                    "Created specialist '{}' (id {}). The user can edit it under Settings → Specialists.",
                    created.as_ref().map(|s| s.name.as_str()).unwrap_or("?"),
                    created.as_ref().map(|s| s.id.as_str()).unwrap_or("?"),
                ))
            }
            Err(e) => ToolResult::fail(format!("save_specialist error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_tools::Tool;

    struct NoEnv(std::path::PathBuf);
    #[async_trait::async_trait]
    impl wisp_tools::ToolEnv for NoEnv {
        fn project_root(&self) -> &std::path::Path { &self.0 }
        async fn confirm(&self, _m: &str) -> bool { true }
        async fn emit(&self, _e: wisp_tools::ToolEvent) {}
    }

    #[tokio::test]
    async fn creates_a_specialist_and_never_touches_builtin() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool { store: store.clone() };
        let env = NoEnv(std::env::temp_dir());
        let r = tool
            .run(&serde_json::json!({"name": "Reviewer", "instructions": "custom"}), &env)
            .await;
        assert!(r.success, "{}", r.content);
        // Same display name is fine — it created sp1, not the builtin.
        let reviewer = crate::specialists::get(&store, "reviewer").await.unwrap();
        assert_eq!(reviewer.instructions, crate::review::REVIEWER_RUBRIC);
        assert!(crate::specialists::get(&store, "sp1").await.is_some());
        let _ = std::fs::remove_file(&tmp);
    }
}
