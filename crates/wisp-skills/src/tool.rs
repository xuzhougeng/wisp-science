//! `use_skill` — load an installed skill's guidance, scripts, and references.

use crate::index::{list_resources, SkillIndex};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct UseSkillTool {
    skills: Arc<SkillIndex>,
}

impl UseSkillTool {
    pub fn new(skills: Arc<SkillIndex>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl Tool for UseSkillTool {
    fn name(&self) -> &str {
        "use_skill"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "use_skill",
            "Load an installed skill with guidance, scripts and references.",
            json!({ "type": "object", "properties": { "name": { "type": "string", "description": "Skill name" } }, "required": ["name"] }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => return ToolResult::fail("missing required argument 'name'"),
        };
        let Some(skill) = self.skills.get(&name) else {
            return ToolResult::fail(format!("skill '{name}' not found"));
        };
        let mut out = format!("# Skill: {}\n{}\n", skill.name, skill.body);
        let (scripts, refs) = list_resources(skill);
        if !scripts.is_empty() {
            out.push_str("\n## Scripts\n");
            for p in &scripts {
                out.push_str(p);
                out.push('\n');
            }
        }
        if !refs.is_empty() {
            out.push_str("\n## References\n");
            for p in &refs {
                out.push_str(p);
                out.push('\n');
            }
        }
        ToolResult::ok(out)
    }
}
