//! Search installed skill metadata, then load one skill's full guidance on demand.

use crate::index::{list_resources, Skill, SkillIndex};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

const DEFAULT_SEARCH_LIMIT: usize = 5;
const MAX_SEARCH_LIMIT: usize = 10;
const MAX_DESCRIPTION_CHARS: usize = 512;

pub struct SearchSkillsTool {
    skills: Arc<SkillIndex>,
}

pub struct UseSkillTool {
    skills: Arc<SkillIndex>,
}

/// Render the same guidance a `use_skill` tool call would return. The desktop
/// composer uses this for an explicitly selected skill, so manual selection
/// and tool-driven selection never drift apart.
pub fn render_skill(skill: &Skill) -> String {
    let mut out = format!("# Skill: {}\n{}\n", skill.name, skill.body);
    // Skills may ship a `kernel.py` sidecar of python helpers. Wisp has no
    // host-side auto-injection into the REPL, so the loading instruction IS
    // the mechanism: one exec in the persistent kernel and the definitions
    // survive across cells.
    let kernel = skill.dir.join("kernel.py");
    if kernel.is_file() {
        out.push_str(&format!(
            "\n## Python Kernel Sidecar\n\
             Before calling this skill's python helpers, load them into the persistent \
             python kernel once (definitions persist across cells; re-run only after a \
             kernel restart):\n```python\nexec(compile(open(r\"{}\").read(), \"kernel.py\", \"exec\"))\n```\n",
            kernel.display()
        ));
    }
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
    out
}

impl SearchSkillsTool {
    pub fn new(skills: Arc<SkillIndex>) -> Self {
        Self { skills }
    }

    fn search(&self, args: &serde_json::Value) -> ToolResult {
        let Some(query) = args
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
        else {
            return ToolResult::fail("missing required argument 'query'");
        };
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        let query = query.to_lowercase();
        let browse = query == "*";
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches = vec![];
        for skill in self.skills.all() {
            let name = skill.name.to_lowercase();
            let description = skill.description.to_lowercase();
            let tags = skill.tags.join(" ").to_lowercase();
            let mut score = usize::from(browse);
            if name == query {
                score += 1_000;
            } else if name.contains(&query) {
                score += 100;
            }
            if description.contains(&query) || tags.contains(&query) {
                score += 50;
            }
            for term in &terms {
                if name.contains(term) {
                    score += 20;
                }
                if description.contains(term) {
                    score += 5;
                }
                if tags.contains(term) {
                    score += 10;
                }
            }
            if score > 0 {
                matches.push((score, skill));
            }
        }
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        let matched_skills = matches.len();
        let results: Vec<_> = matches
            .into_iter()
            .take(limit)
            .map(|(_, skill)| {
                json!({
                    "name": skill.name,
                    "description": truncate_chars(&skill.description, MAX_DESCRIPTION_CHARS),
                    "tags": skill.tags,
                })
            })
            .collect();
        ToolResult::ok(
            serde_json::to_string_pretty(&json!({
                "results": results,
                "matched_skills": matched_skills,
                "total_skills": self.skills.all().len(),
                "next": "Call 'use_skill' with the exact name of the relevant skill. Use query '*' to browse.",
            }))
            .unwrap_or_default(),
        )
    }
}

#[async_trait]
impl Tool for SearchSkillsTool {
    fn name(&self) -> &str {
        "search_skills"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "search_skills",
            "Search installed skill names, descriptions, and tags without loading every skill body.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Task, workflow, domain keywords, or '*' to browse" },
                    "limit": { "type": "integer", "description": "Maximum matches to return (default 5, maximum 10)" }
                },
                "required": ["query"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("query")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        self.search(args)
    }
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
        ToolResult::ok(render_skill(skill))
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("… [truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    // A kernel.py sidecar has no host auto-injection in wisp; the rendered
    // skill must carry the one-time exec loading instruction with the
    // sidecar's absolute path.
    #[test]
    fn render_skill_appends_kernel_sidecar_loading_instruction() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-sidecar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("kernel.py"), "def pdf_pages():\n    pass\n").unwrap();
        let skill = Skill {
            name: "pdf-explore".into(),
            description: "d".into(),
            tags: vec![],
            body: "body".into(),
            dir: root.clone(),
        };
        let rendered = render_skill(&skill);
        assert!(rendered.contains("## Python Kernel Sidecar"));
        assert!(rendered.contains(&root.join("kernel.py").display().to_string()));
        assert!(rendered.contains("exec(compile(open("));

        let plain = Skill {
            dir: root.join("no-kernel-here"),
            ..skill
        };
        assert!(!render_skill(&plain).contains("Kernel Sidecar"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn skill_search_returns_only_relevant_metadata() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (name, description, tags) in [
            (
                "pubmed-review",
                "Find biomedical papers and synthesize evidence.",
                "literature, medicine",
            ),
            (
                "plot-figure",
                "Create publication-ready charts.",
                "visualization",
            ),
        ] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\ntags: [{tags}]\n---\nbody"),
            )
            .unwrap();
        }
        let search = SearchSkillsTool::new(Arc::new(SkillIndex::load(&[root.clone()])));

        let result = search.search(&json!({ "query": "biomedical literature" }));
        assert!(result.success, "search failed: {}", result.content);
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["results"][0]["name"], "pubmed-review");
        assert_eq!(output["results"].as_array().unwrap().len(), 1);
        assert!(!result.content.contains("publication-ready charts"));

        std::fs::remove_dir_all(root).ok();
    }
}
