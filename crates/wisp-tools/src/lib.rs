//! Built-in agent tools for Wisp, Windows-first.
//!
//! Tools implement [`tool::Tool`] and run against a [`env::ToolEnv`] the host
//! supplies. [`Registry`] bundles the built-ins, exposes their JSON schemas to
//! the LLM, and dispatches tool calls. Extra tools (Python `repl`, MCP) are
//! added with [`Registry::add`].

pub mod attempt_completion;
pub mod edit;
pub mod env;
pub mod grep;
pub mod image;
pub mod plan;
pub mod process;
pub mod read;
pub mod safety;
pub mod search;
pub mod shell;
pub mod tool;
pub mod write;

pub use env::{Approval, ConfirmDecision, ImageData, ToolEnv, ToolEvent, ToolResult};
pub use tool::Tool;

use serde_json::Value;
use wisp_llm::ToolSchema;

const SEARCH_MCP_TOOLS: &str = "search_mcp_tools";
const USE_MCP_TOOL: &str = "use_mcp_tool";
const DEFAULT_MCP_SEARCH_LIMIT: usize = 5;
const MAX_MCP_SEARCH_LIMIT: usize = 10;
const MAX_MCP_DESCRIPTION_CHARS: usize = 2_048;

/// The built-in tool set plus any extras (repl, MCP) registered later.
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    /// The mangopi-compatible built-ins: read/write/edit/search/grep/shell/
    /// attempt_completion. `view_image` is reached via `read` on image files
    /// (and exposed here too for explicit calls).
    pub fn builtins() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(read::ReadTool),
            Box::new(write::WriteTool),
            Box::new(edit::EditTool),
            Box::new(search::SearchTool),
            Box::new(grep::GrepTool),
            Box::new(shell::ShellTool),
            image_view_tool(),
            Box::new(plan::UpdatePlanTool),
            Box::new(attempt_completion::AttemptCompletionTool),
        ];
        Self { tools }
    }

    pub fn add(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Keep only tools named by a host-resolved capability grant.
    pub fn filtered(mut self, allowed: &[String]) -> Self {
        self.tools
            .retain(|tool| allowed.iter().any(|name| name == tool.name()));
        self
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        let mut schemas: Vec<_> = self
            .tools
            .iter()
            .filter(|tool| !tool.defer_schema())
            .map(|tool| tool.schema())
            .collect();
        if self.tools.iter().any(|tool| tool.defer_schema()) {
            schemas.push(search_mcp_tools_schema());
            schemas.push(use_mcp_tool_schema());
        }
        schemas
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Dispatch a tool call: enforce the approval policy, emit the call card,
    /// run `before`, then `run`.
    pub async fn run(&self, name: &str, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        if name == SEARCH_MCP_TOOLS {
            return self.run_mcp_search(args, env).await;
        }
        if name == USE_MCP_TOOL {
            let Some(tool_name) = args.get("tool_name").and_then(Value::as_str) else {
                return ToolResult::fail("missing required argument 'tool_name'");
            };
            let Some(tool_input) = args.get("tool_input").filter(|value| value.is_object()) else {
                return ToolResult::fail("'tool_input' must be a JSON object");
            };
            let Some(tool) = self
                .tools
                .iter()
                .find(|tool| tool.defer_schema() && tool.name() == tool_name)
            else {
                return ToolResult::fail(format!(
                    "deferred MCP tool '{tool_name}' not found; call '{SEARCH_MCP_TOOLS}' first"
                ));
            };
            return run_registered_tool(tool.as_ref(), tool_input, env).await;
        }
        let Some(tool) = self.get(name) else {
            return ToolResult::fail(format!("unknown tool '{name}'"));
        };
        run_registered_tool(tool, args, env).await
    }

    async fn run_mcp_search(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let approval = env.approval_mode(SEARCH_MCP_TOOLS).await;
        if approval == env::Approval::Deny {
            return ToolResult::fail(format!(
                "tool '{SEARCH_MCP_TOOLS}' is blocked by the approval policy"
            ));
        }
        let preview = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        env.emit(ToolEvent::Call {
            name: SEARCH_MCP_TOOLS.to_string(),
            preview,
        })
        .await;
        if approval == env::Approval::Ask
            && !env
                .confirm(&format!("Run tool '{SEARCH_MCP_TOOLS}'?"))
                .await
        {
            env.emit(ToolEvent::Result { ok: false }).await;
            return ToolResult::fail(format!("tool '{SEARCH_MCP_TOOLS}' was denied by the user"));
        }
        let result = self.search_mcp_tools(args);
        env.emit(ToolEvent::Result { ok: result.success }).await;
        result
    }

    fn search_mcp_tools(&self, args: &Value) -> ToolResult {
        let Some(query) = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
        else {
            return ToolResult::fail("missing required argument 'query'");
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_MCP_SEARCH_LIMIT)
            .clamp(1, MAX_MCP_SEARCH_LIMIT);
        let query = query.to_lowercase();
        let browse = query == "*";
        let terms: Vec<_> = query.split_whitespace().collect();
        let total_hidden_tools = self.tools.iter().filter(|tool| tool.defer_schema()).count();
        let mut matches = vec![];
        for tool in self.tools.iter().filter(|tool| tool.defer_schema()) {
            let schema = tool.schema();
            let name = schema.function.name.to_lowercase();
            let description = schema.function.description.to_lowercase();
            let parameters = schema.function.parameters.to_string().to_lowercase();
            let mut score = usize::from(browse);
            if name == query {
                score += 1_000;
            } else if name.contains(&query) {
                score += 100;
            }
            if description.contains(&query) {
                score += 50;
            }
            for term in &terms {
                if name.contains(term) {
                    score += 20;
                }
                if description.contains(term) {
                    score += 5;
                }
                if parameters.contains(term) {
                    score += 1;
                }
            }
            if score > 0 {
                matches.push((score, schema));
            }
        }
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.function.name.cmp(&right.function.name))
        });
        let matched_tools = matches.len();
        let results: Vec<_> = matches
            .into_iter()
            .take(limit)
            .map(|(_, schema)| {
                serde_json::json!({
                    "tool_name": schema.function.name,
                    "description": truncate_chars(
                        &schema.function.description,
                        MAX_MCP_DESCRIPTION_CHARS,
                    ),
                    "input_schema": schema.function.parameters,
                })
            })
            .collect();
        ToolResult::ok(
            serde_json::to_string_pretty(&serde_json::json!({
                "results": results,
                "matched_tools": matched_tools,
                "total_hidden_tools": total_hidden_tools,
                "next": format!(
                    "Call '{USE_MCP_TOOL}' with a returned tool_name and matching tool_input. Use query '*' to browse."
                ),
            }))
            .unwrap_or_default(),
        )
    }
}

async fn run_registered_tool(tool: &dyn Tool, args: &Value, env: &dyn ToolEnv) -> ToolResult {
    let name = tool.name();
    // Per-tool approval gate. `Deny` blocks before the call card even shows;
    // `Ask` shows the card then routes through `confirm`; `Allow` runs as before.
    let approval = match (env.approval_mode(name).await, tool.minimum_approval()) {
        (env::Approval::Deny, _) => env::Approval::Deny,
        (env::Approval::Ask, _) | (_, env::Approval::Ask) => env::Approval::Ask,
        _ => env::Approval::Allow,
    };
    if approval == env::Approval::Deny {
        return ToolResult::fail(format!("tool '{name}' is blocked by the approval policy"));
    }
    let preview = tool.preview(args);
    env.emit(ToolEvent::Call {
        name: name.to_string(),
        preview,
    })
    .await;
    if approval == env::Approval::Ask && !env.confirm(&format!("Run tool '{name}'?")).await {
        env.emit(ToolEvent::Result { ok: false }).await;
        return ToolResult::fail(format!("tool '{name}' was denied by the user"));
    }
    tool.before(args, env).await;
    let result = tool.run(args, env).await;
    env.emit(ToolEvent::Result { ok: result.success }).await;
    result
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("… [truncated]");
    truncated
}

fn search_mcp_tools_schema() -> ToolSchema {
    ToolSchema::new(
        SEARCH_MCP_TOOLS,
        "Search deferred MCP tools by name, description, and input fields. Returns only matching schemas so the full MCP catalog does not consume every request.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Capability, server, action, known tool name, or '*' to browse" },
                "limit": { "type": "integer", "description": "Maximum matches to return (default 5, maximum 10)" }
            },
            "required": ["query"]
        }),
    )
}

fn use_mcp_tool_schema() -> ToolSchema {
    ToolSchema::new(
        USE_MCP_TOOL,
        "Call an MCP tool found by search_mcp_tools. tool_input must match the returned input_schema.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": { "type": "string", "description": "Exact tool_name returned by search_mcp_tools" },
                "tool_input": { "type": "object", "description": "Arguments matching the selected tool's input_schema", "additionalProperties": true }
            },
            "required": ["tool_name", "tool_input"]
        }),
    )
}

/// A thin `view_image` tool wrapper around the shared image helper.
struct ViewImageTool;
fn image_view_tool() -> Box<dyn Tool> {
    Box::new(ViewImageTool)
}

#[async_trait::async_trait]
impl Tool for ViewImageTool {
    fn name(&self) -> &str {
        "view_image"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "view_image",
            "Analyze a local image (screenshot, UI mockup, diagram, figure) with the configured vision model. Accepts an absolute path to a file on disk; URLs are not supported.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to a local image file (png/jpg/jpeg/gif/webp)" },
                    "question": { "type": "string", "description": "Optional specific question or extraction goal for the vision model" }
                },
                "required": ["path"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        args.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::fail("view_image error: 'path' is required"),
        };
        let path = match env.resolve_read_path(&path, false) {
            Ok(path) => path,
            Err(error) => return ToolResult::fail(format!("view_image error: {error}")),
        };
        image::view_image(&path.to_string_lossy())
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::env::{Approval, ToolEnv, ToolEvent};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// A tool that flips a flag when it actually runs, so we can assert whether
    /// the approval gate let it through.
    struct SpyTool(&'static AtomicBool);
    #[async_trait::async_trait]
    impl Tool for SpyTool {
        fn name(&self) -> &str {
            "spy"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new("spy", "test", serde_json::json!({"type": "object"}))
        }
        async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            self.0.store(true, Ordering::SeqCst);
            ToolResult::ok("ran")
        }
    }

    struct AskSpy(&'static AtomicBool);
    #[async_trait::async_trait]
    impl Tool for AskSpy {
        fn name(&self) -> &str {
            "third_party"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new(self.name(), "test", serde_json::json!({"type": "object"}))
        }
        fn minimum_approval(&self) -> Approval {
            Approval::Ask
        }
        async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            self.0.store(true, Ordering::SeqCst);
            ToolResult::ok("ran")
        }
    }

    struct DeferredTool;
    #[async_trait::async_trait]
    impl Tool for DeferredTool {
        fn name(&self) -> &str {
            "pubmed_search_articles"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new(
                self.name(),
                "Search PubMed articles by biomedical keywords.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }),
            )
        }
        fn defer_schema(&self) -> bool {
            true
        }
        async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            ToolResult::ok(format!("searched {}", args["query"]))
        }
    }

    struct PolicyEnv {
        root: PathBuf,
        mode: Approval,
        confirm_ok: bool,
    }
    #[async_trait::async_trait]
    impl ToolEnv for PolicyEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            self.confirm_ok
        }
        async fn approval_mode(&self, _tool: &str) -> Approval {
            self.mode
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    struct EventEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for EventEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    async fn run_with(mode: Approval, confirm_ok: bool) -> (bool, ToolResult) {
        static RAN: AtomicBool = AtomicBool::new(false);
        RAN.store(false, Ordering::SeqCst);
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(SpyTool(&RAN)));
        let env = PolicyEnv {
            root: PathBuf::from("."),
            mode,
            confirm_ok,
        };
        let res = reg.run("spy", &serde_json::json!({}), &env).await;
        (RAN.load(Ordering::SeqCst), res)
    }

    #[tokio::test]
    async fn tool_minimum_approval_upgrades_host_allow_to_ask() {
        static RAN: AtomicBool = AtomicBool::new(false);
        RAN.store(false, Ordering::SeqCst);
        let mut registry = Registry { tools: vec![] };
        registry.add(Box::new(AskSpy(&RAN)));
        let env = PolicyEnv {
            root: PathBuf::from("."),
            mode: Approval::Allow,
            confirm_ok: false,
        };
        let result = registry
            .run("third_party", &serde_json::json!({}), &env)
            .await;
        assert!(!result.success);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn approval_gate() {
        // Deny: never runs, fails.
        let (ran, res) = run_with(Approval::Deny, true).await;
        assert!(!ran && !res.success, "deny must block the tool");
        // Ask + confirm no: never runs, fails.
        let (ran, res) = run_with(Approval::Ask, false).await;
        assert!(!ran && !res.success, "ask+deny must block the tool");
        // Ask + confirm yes: runs.
        let (ran, res) = run_with(Approval::Ask, true).await;
        assert!(ran && res.success, "ask+approve must run the tool");
        // Allow: runs without asking.
        let (ran, res) = run_with(Approval::Allow, false).await;
        assert!(ran && res.success, "allow must run the tool");
    }

    #[tokio::test]
    async fn shell_tool_emits_single_call_event() {
        let reg = Registry::builtins();
        let env = EventEnv {
            root: std::env::current_dir().unwrap(),
            events: Mutex::new(vec![]),
        };
        let cmd = if cfg!(target_os = "windows") {
            "Write-Output ok"
        } else {
            "printf ok"
        };

        let res = reg
            .run("shell", &serde_json::json!({ "cmd": cmd }), &env)
            .await;

        assert!(res.success, "shell command should succeed: {}", res.content);
        let calls = env
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|ev| matches!(ev, ToolEvent::Call { .. }))
            .count();
        assert_eq!(calls, 1, "registry should emit the only tool call card");
    }

    #[test]
    fn deferred_schemas_are_replaced_by_search_and_dispatch_tools() {
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(SpyTool(&SPY_FOR_SCHEMA_TEST)));
        reg.add(Box::new(DeferredTool));

        let names: Vec<_> = reg
            .schemas()
            .into_iter()
            .map(|schema| schema.function.name)
            .collect();

        assert_eq!(names, ["spy", SEARCH_MCP_TOOLS, USE_MCP_TOOL]);
        assert!(!names.contains(&"pubmed_search_articles".to_string()));
    }

    static SPY_FOR_SCHEMA_TEST: AtomicBool = AtomicBool::new(false);

    #[tokio::test]
    async fn deferred_tool_is_searched_then_dispatched() {
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(DeferredTool));
        let env = EventEnv {
            root: PathBuf::from("."),
            events: Mutex::new(vec![]),
        };

        let found = reg
            .run(
                SEARCH_MCP_TOOLS,
                &serde_json::json!({ "query": "biomedical articles" }),
                &env,
            )
            .await;
        assert!(found.success, "search failed: {}", found.content);
        let catalog: Value = serde_json::from_str(&found.content).unwrap();
        assert_eq!(catalog["results"][0]["tool_name"], "pubmed_search_articles");
        assert_eq!(
            catalog["results"][0]["input_schema"]["required"][0],
            "query"
        );

        let called = reg
            .run(
                USE_MCP_TOOL,
                &serde_json::json!({
                    "tool_name": "pubmed_search_articles",
                    "tool_input": { "query": "cancer" }
                }),
                &env,
            )
            .await;
        assert!(called.success, "dispatch failed: {}", called.content);
        assert_eq!(called.content, "searched \"cancer\"");
        assert!(env.events.lock().unwrap().iter().any(|event| matches!(
            event,
            ToolEvent::Call { name, .. } if name == "pubmed_search_articles"
        )));
    }

    #[test]
    fn filtered_registry_exposes_only_host_approved_tools() {
        let allowed = vec!["read".to_string(), "grep".to_string()];
        let registry = Registry::builtins().filtered(&allowed);
        assert_eq!(registry.names(), vec!["read", "grep"]);
        assert!(registry.get("write").is_none());
        assert!(registry.get("shell").is_none());
    }
}
