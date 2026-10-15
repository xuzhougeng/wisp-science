//! `save_specialist` — create a specialist from chat, or update one by id.
//! Builtin instruction text stays pinned by `specialists::upsert`.

use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct SaveSpecialistTool {
    pub store: Store,
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn list_arg(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)?.as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect()
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
            "Create or update a specialist (agent persona): a name, instructions \
             appended to the base prompt, an optional bound model id, and optional \
             skill/connector whitelists. Interview the user before creating. \
             Omit `id` to create. Pass `id` from configure get specialists to update \
             an existing custom specialist. Builtin instruction text cannot be replaced.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Existing specialist id to update; omit to create" },
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
        let instructions = str_arg(args, "instructions");
        if instructions.is_empty() {
            return ToolResult::fail("save_specialist error: 'instructions' is required");
        }
        let id = str_arg(args, "id");
        let existing = if id.is_empty() {
            None
        } else {
            match crate::specialists::get(&self.store, &id).await {
                Some(existing) => Some(existing),
                None => {
                    return ToolResult::fail(format!(
                        "save_specialist error: no specialist with id '{id}'"
                    ))
                }
            }
        };
        let spec = if let Some(existing) = existing.clone() {
            crate::specialists::Specialist {
                id: existing.id,
                name,
                icon: existing.icon,
                color: existing.color,
                description: {
                    let description = str_arg(args, "description");
                    if description.is_empty() {
                        existing.description
                    } else {
                        description
                    }
                },
                instructions,
                model_id: if args.get("model_id").is_some() {
                    str_arg(args, "model_id")
                } else {
                    existing.model_id
                },
                review_backend: existing.review_backend,
                skills: list_arg(args, "skills").or(existing.skills),
                connectors: list_arg(args, "connectors").or(existing.connectors),
                builtin: existing.builtin,
            }
        } else {
            crate::specialists::Specialist {
                id: String::new(),
                name,
                icon: "review".into(),
                color: "clay".into(),
                description: str_arg(args, "description"),
                instructions,
                model_id: str_arg(args, "model_id"),
                review_backend: None,
                skills: list_arg(args, "skills"),
                connectors: list_arg(args, "connectors"),
                builtin: false,
            }
        };
        let updating = existing.is_some();
        let target_id = spec.id.clone();
        let before: std::collections::HashSet<String> = crate::specialists::ensure(&self.store)
            .await
            .into_iter()
            .map(|s| s.id)
            .collect();
        match crate::specialists::upsert(&self.store, spec).await {
            Ok(list) => {
                if updating {
                    let updated = list.iter().find(|s| s.id == target_id);
                    ToolResult::ok(format!(
                        "Updated specialist '{}' (id {}).",
                        updated.map(|s| s.name.as_str()).unwrap_or("?"),
                        updated.map(|s| s.id.as_str()).unwrap_or(&target_id),
                    ))
                } else {
                    let created = list.iter().find(|s| !before.contains(&s.id)).cloned();
                    ToolResult::ok(format!(
                        "Created specialist '{}' (id {}). Select it from the session specialist menu, or edit it later with save_specialist and this id.",
                        created.as_ref().map(|s| s.name.as_str()).unwrap_or("?"),
                        created.as_ref().map(|s| s.id.as_str()).unwrap_or("?"),
                    ))
                }
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
        fn project_root(&self) -> &std::path::Path {
            &self.0
        }
        async fn confirm(&self, _m: &str) -> bool {
            true
        }
        async fn emit(&self, _e: wisp_tools::ToolEvent) {}
    }

    /// The id from a "Created/Updated specialist '…' (id …)." result.
    fn reported_id(content: &str) -> String {
        let rest = content.split("(id ").nth(1).expect(content);
        rest.split(')').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn creates_a_specialist_and_never_touches_builtin() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool {
            store: store.clone(),
        };
        let env = NoEnv(std::env::temp_dir());
        let r = tool
            .run(
                &serde_json::json!({"name": "Reviewer", "instructions": "custom"}),
                &env,
            )
            .await;
        assert!(r.success, "{}", r.content);
        // Same display name is fine — it created a new custom row, not the builtin.
        let reviewer = crate::specialists::get(&store, "reviewer").await.unwrap();
        assert_eq!(reviewer.instructions, crate::review::REVIEWER_RUBRIC);
        let created = crate::specialists::get(&store, &reported_id(&r.content))
            .await
            .unwrap();
        assert!(!created.builtin);
        assert_eq!(created.instructions, "custom");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn requires_instructions() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool {
            store: store.clone(),
        };
        let env = NoEnv(std::env::temp_dir());
        let r = tool
            .run(&serde_json::json!({"name": "Reviewer"}), &env)
            .await;
        assert!(!r.success);
        assert_eq!(
            r.content,
            "save_specialist error: 'instructions' is required"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn sequential_creates_report_correct_distinct_ids() {
        // Verifies sequential creates report the right id/name pair. NOTE: this
        // does NOT discriminate the id-snapshot lookup from the old
        // last-non-builtin one — upsert appends in-process, so both coincide
        // here; the snapshot lookup is defensive hardening against future
        // ordering changes, not a behavior this test can force apart.
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool {
            store: store.clone(),
        };
        let env = NoEnv(std::env::temp_dir());

        let r1 = tool
            .run(
                &serde_json::json!({"name": "First", "instructions": "one"}),
                &env,
            )
            .await;
        assert!(r1.success, "{}", r1.content);

        let r2 = tool
            .run(
                &serde_json::json!({"name": "Second", "instructions": "two"}),
                &env,
            )
            .await;
        assert!(r2.success, "{}", r2.content);

        let (id1, id2) = (reported_id(&r1.content), reported_id(&r2.content));
        assert_ne!(id1, id2);

        let first = crate::specialists::get(&store, &id1).await.unwrap();
        assert_eq!(first.name, "First");
        let second = crate::specialists::get(&store, &id2).await.unwrap();
        assert_eq!(second.name, "Second");

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn updates_an_existing_custom_specialist_by_id() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool {
            store: store.clone(),
        };
        let env = NoEnv(std::env::temp_dir());
        let created = tool
            .run(
                &serde_json::json!({"name": "Paper hunter", "instructions": "find papers"}),
                &env,
            )
            .await;
        assert!(created.success, "{}", created.content);
        let id = reported_id(&created.content);

        let updated = tool
            .run(
                &serde_json::json!({
                    "id": id,
                    "name": "Paper hunter 2",
                    "instructions": "find newer papers"
                }),
                &env,
            )
            .await;
        assert!(updated.success, "{}", updated.content);
        assert!(
            updated.content.contains("Updated specialist"),
            "{}",
            updated.content
        );
        let spec = crate::specialists::get(&store, &id).await.unwrap();
        assert_eq!(spec.name, "Paper hunter 2");
        assert_eq!(spec.instructions, "find newer papers");
        assert_eq!(
            crate::specialists::ensure(&store)
                .await
                .iter()
                .filter(|s| !s.builtin)
                .count(),
            1
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn unknown_id_fails_without_creating() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool {
            store: store.clone(),
        };
        let env = NoEnv(std::env::temp_dir());
        let result = tool
            .run(
                &serde_json::json!({
                    "id": "missing",
                    "name": "Ghost",
                    "instructions": "nope"
                }),
                &env,
            )
            .await;
        assert!(!result.success);
        assert!(
            result.content.contains("no specialist"),
            "{}",
            result.content
        );
        assert!(crate::specialists::get(&store, "missing").await.is_none());
        let _ = std::fs::remove_file(&tmp);
    }
}
