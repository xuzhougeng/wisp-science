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

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
    )
}

fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|ch| {
            if ch.is_alphanumeric() || is_cjk(ch) {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn search_terms(value: &str) -> Vec<String> {
    let normalized = normalize_search_text(value);
    let mut terms = normalized
        .split_whitespace()
        .filter(|term| term.chars().count() >= 2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let cjk = normalized
        .chars()
        .filter(|ch| is_cjk(*ch))
        .collect::<Vec<_>>();
    for size in [2, 3, 4] {
        for window in cjk.windows(size) {
            terms.push(window.iter().collect());
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

pub struct SearchSkillsTool {
    skills: Arc<SkillIndex>,
}

pub struct ListSkillCatalogTool {
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
    // Root-level runtime sidecars run inside the corresponding persistent
    // runtime, never as standalone CLI scripts. Rendering only supplies the
    // loading instruction; it does not execute or auto-inject either file.
    for (filename, language, tool) in [("runtime.py", "Python", "python"), ("runtime.r", "R", "r")]
    {
        let runtime_script = skill.dir.join(filename);
        if !runtime_script.is_file() {
            continue;
        }
        // JSON string escaping also produces a Python/R string literal,
        // including Windows separators and paths containing quotes.
        let path = serde_json::to_string(&runtime_script.to_string_lossy()).unwrap();
        let code = match tool {
            "python" => format!(
                "exec(compile(open({path}, encoding=\"utf-8\").read(), \"runtime.py\", \"exec\"))"
            ),
            // The R worker evaluates cells in its own persistent environment.
            // local = TRUE loads there, keeping helpers visible to inspection
            // and required_objects checks as well as subsequent cells.
            _ => format!("source({path}, local = TRUE, encoding = \"UTF-8\")"),
        };
        out.push_str(&format!(
            "\n## {language} Runtime Sidecar\n\
             `{filename}` is a special runtime script. Before using its helpers, \
             execute the following code through the `{tool}` tool in the selected \
             persistent {language} runtime. Do not launch this file as a standalone \
             process through shell or run_in_context. Load once per runtime; definitions \
             and state persist across calls. Load again after a runtime restart or when \
             switching to a different conversation or execution context.\n\
             ```{tool}\n{code}\n```\n\
             If the runtime cannot access this skill path (for example on SSH/WSL), \
             read the file with the read tool and submit its source as `code` to the \
             `{tool}` tool in that context. Loading the skill only returns these \
             instructions; it does not execute the sidecar automatically.\n"
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
        let browse = query == "*";
        let normalized_query = normalize_search_text(query);
        let terms = search_terms(query);
        let mut matches = vec![];
        for skill in self.skills.all() {
            let name = normalize_search_text(&skill.name);
            let description = normalize_search_text(&skill.description);
            let tags = normalize_search_text(&skill.tags.join(" "));
            let mut score = usize::from(browse);
            if name == normalized_query {
                score += 1_000;
            } else if !normalized_query.is_empty() && name.contains(&normalized_query) {
                score += 100;
            }
            if !normalized_query.is_empty() && description.contains(&normalized_query) {
                score += 50;
            }
            if !normalized_query.is_empty() && tags.contains(&normalized_query) {
                score += 60;
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
                    "scope": self.skills.source(&skill.name).map(|source| source.as_str()),
                    "path": skill.dir,
                })
            })
            .collect();
        ToolResult::ok(
            serde_json::to_string_pretty(&json!({
                "results": results,
                "matched_skills": matched_skills,
                "current_configured_enabled_count": self.skills.all().len(),
                "current_configured_enabled_by_source": self.skills.skill_counts_by_source(),
                "searchable_enabled_count": self.skills.all().len(),
                "catalog_counts": self.skills.catalog_audit(),
                "next": "Call 'use_skill' with the exact name of the relevant skill. Use query '*' to browse.",
            }))
            .unwrap_or_default(),
        )
    }
}

impl ListSkillCatalogTool {
    pub fn new(skills: Arc<SkillIndex>) -> Self {
        Self { skills }
    }

    fn list(&self, args: &serde_json::Value) -> ToolResult {
        let view = args
            .get("view")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("discovered");
        if !matches!(view, "discovered" | "effective") {
            return ToolResult::fail("view must be 'discovered' or 'effective'");
        }
        let offset = match args.get("cursor").and_then(serde_json::Value::as_str) {
            Some(cursor) => match cursor
                .strip_prefix("offset:")
                .and_then(|raw| raw.parse().ok())
            {
                Some(offset) => offset,
                None => return ToolResult::fail("invalid catalog cursor"),
            },
            None => 0,
        };
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(50)
            .clamp(1, 100);
        let records = self
            .skills
            .catalog_records()
            .iter()
            .filter(|record| view == "discovered" || record.effective)
            .collect::<Vec<_>>();
        if offset > records.len() {
            return ToolResult::fail("catalog cursor is past the end of this snapshot");
        }
        let page = records
            .iter()
            .skip(offset)
            .take(limit)
            .copied()
            .collect::<Vec<_>>();
        let next_offset = offset.saturating_add(page.len());
        let next_cursor = (next_offset < records.len()).then(|| format!("offset:{next_offset}"));
        ToolResult::ok(
            serde_json::to_string_pretty(&json!({
                "view": view,
                "records": page,
                "next_cursor": next_cursor,
                "audit": self.skills.catalog_audit(),
                "current_configured_enabled_count": self.skills.all().len(),
                "current_configured_enabled_by_source": self.skills.skill_counts_by_source(),
                "searchable_enabled_count": self.skills.all().len(),
            }))
            .unwrap_or_default(),
        )
    }
}

#[async_trait]
impl Tool for ListSkillCatalogTool {
    fn name(&self) -> &str {
        "list_skill_catalog"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "list_skill_catalog",
            "Read the authoritative current configured/enabled Skill count and page through the complete discovered or effective catalog, including source counts, shadowed records, and parse errors.",
            json!({
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["discovered", "effective"], "default": "discovered" },
                    "cursor": { "type": "string", "description": "Opaque cursor returned by the previous page" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }
                }
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("view")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("discovered")
            .to_string()
    }

    /// A pure catalog read: stays callable in plan mode and unattended turns.
    fn read_only(&self) -> bool {
        true
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        self.list(args)
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
            "Search installed skill names, descriptions, tags, scopes, and source paths without loading every skill body.",
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

    // Both languages are optional and independent. Ordinary scripts and
    // directories named like sidecars must not trigger runtime guidance.
    #[test]
    fn render_skill_appends_only_root_runtime_sidecar_loading_instructions() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-sidecar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (python, r) in [(false, false), (true, false), (false, true), (true, true)] {
            let dir = root.join(format!("研究er's skill {python}-{r}"));
            std::fs::create_dir_all(dir.join("scripts")).unwrap();
            // Legacy filenames no longer opt into runtime loading.
            for filename in ["kernel.py", "kernel.r"] {
                std::fs::write(dir.join(filename), "legacy sidecar sentinel").unwrap();
            }
            for filename in ["main.py", "main.r", "runtime.py", "runtime.r"] {
                std::fs::write(dir.join("scripts").join(filename), "ordinary script").unwrap();
            }
            for (filename, present) in [("runtime.py", python), ("runtime.r", r)] {
                if present {
                    // Rendering must not try to execute this content.
                    std::fs::write(dir.join(filename), "sidecar source sentinel").unwrap();
                } else {
                    std::fs::create_dir(dir.join(filename)).unwrap();
                }
            }
            let skill = Skill {
                name: "sidecar-test".into(),
                description: "d".into(),
                tags: vec![],
                body: "body".into(),
                dir: dir.clone(),
                declared_version: None,
                wisp: None,
            };
            let rendered = render_skill(&skill);
            assert_eq!(rendered.contains("## Python Runtime Sidecar"), python);
            assert_eq!(rendered.contains("## R Runtime Sidecar"), r);
            assert_eq!(rendered.contains("exec(compile(open("), python);
            assert_eq!(rendered.contains("local = TRUE"), r);
            assert!(!rendered.contains(".GlobalEnv"));
            assert!(rendered.contains("## Scripts"));
            for filename in ["main.py", "main.r", "runtime.py", "runtime.r"] {
                assert!(
                    rendered.contains(&dir.join("scripts").join(filename).display().to_string())
                );
            }
            for (filename, tool, present) in
                [("runtime.py", "python", python), ("runtime.r", "r", r)]
            {
                if present {
                    let path =
                        serde_json::to_string(&dir.join(filename).to_string_lossy()).unwrap();
                    assert!(rendered.contains(&path));
                    assert!(rendered.contains(&format!("through the `{tool}` tool")));
                    assert!(rendered.contains("Do not launch this file as a standalone"));
                    assert!(rendered.contains("read the file with the read tool"));
                    assert!(rendered.contains("does not execute the sidecar automatically"));
                }
            }
            assert!(!rendered.contains("sidecar source sentinel"));
        }
        std::fs::remove_dir_all(&root).unwrap();
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
        assert_eq!(output["results"][0]["scope"], "custom");
        assert_eq!(
            output["results"][0]["path"],
            root.join("pubmed-review").to_string_lossy().as_ref()
        );
        assert_eq!(output["results"].as_array().unwrap().len(), 1);
        assert!(!result.content.contains("publication-ready charts"));

        let overrides = std::collections::BTreeMap::from([(
            "pubmed-review".into(),
            vec!["systematic-evidence".into()],
        )]);
        let overridden = SearchSkillsTool::new(Arc::new(
            SkillIndex::load(&[root.clone()]).with_tag_overrides(&overrides),
        ));
        let result = overridden.search(&json!({ "query": "systematic-evidence" }));
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["results"][0]["name"], "pubmed-review");
        assert_eq!(output["results"][0]["tags"][0], "systematic-evidence");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn search_normalizes_separators_and_continuous_cjk_queries() {
        assert_eq!(normalize_search_text("Single-Cell_QC"), "single cell qc");
        let terms = search_terms("请检查论文图片是否重复");
        assert!(terms.contains(&"图片".to_string()));
        assert!(terms.contains(&"重复".to_string()));

        let root = std::env::temp_dir().join(format!(
            "wisp-skill-multilingual-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (name, description) in [
            (
                "singlecell-qc",
                "Design single cell RNA sequencing quality control.",
            ),
            ("figure-duplicate-audit", "审核论文图片重复和图片查重。"),
            ("literature-review", "Retrieve and synthesize papers."),
        ] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\nbody"),
            )
            .unwrap();
        }
        let search = SearchSkillsTool::new(Arc::new(SkillIndex::load(&[root.clone()])));

        let result = search.search(&json!({ "query": "single-cell_QC" }));
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["results"][0]["name"], "singlecell-qc");

        let result = search.search(&json!({ "query": "singlecell" }));
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["results"][0]["name"], "singlecell-qc");

        let result = search.search(&json!({ "query": "请检查论文图片是否重复" }));
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["results"][0]["name"], "figure-duplicate-audit");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn catalog_listing_pages_without_losing_records() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-listing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for name in ["alpha", "beta", "gamma"] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name}\n---\nbody"),
            )
            .unwrap();
        }
        let tool = ListSkillCatalogTool::new(Arc::new(SkillIndex::load(&[root.clone()])));
        let first = tool.list(&json!({"limit": 2}));
        let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
        assert_eq!(first["records"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_cursor"], "offset:2");
        assert_eq!(first["audit"]["discovered_count"], 3);
        assert_eq!(first["current_configured_enabled_count"], 3);
        assert_eq!(first["current_configured_enabled_by_source"]["custom"], 3);
        let second = tool.list(&json!({"limit": 2, "cursor": "offset:2"}));
        let second: serde_json::Value = serde_json::from_str(&second.content).unwrap();
        assert_eq!(second["records"].as_array().unwrap().len(), 1);
        assert!(second["next_cursor"].is_null());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn inventory_count_stays_authoritative_when_catalog_contains_disabled_skills() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-inventory-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for name in ["enabled", "disabled"] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name}\n---\nbody"),
            )
            .unwrap();
        }
        let enabled = std::collections::HashSet::from(["enabled".to_string()]);
        let filtered = SkillIndex::load(&[root.clone()]).filtered_by_names(Some(&enabled));
        let search = SearchSkillsTool::new(Arc::new(filtered));

        let result = search.search(&json!({ "query": "*" }));
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["current_configured_enabled_count"], 1);
        assert_eq!(output["current_configured_enabled_by_source"]["custom"], 1);
        assert_eq!(output["catalog_counts"]["effective_count"], 2);
        assert_eq!(output["results"].as_array().unwrap().len(), 1);

        std::fs::remove_dir_all(root).ok();
    }
}
