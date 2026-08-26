//! Built-in agent tools for Wisp, Windows-first.
//!
//! Tools implement [`tool::Tool`] and run against a [`env::ToolEnv`] the host
//! supplies. [`Registry`] bundles the built-ins, exposes their JSON schemas to
//! the LLM, and dispatches tool calls. Extra tools (Python `repl`, MCP) are
//! added with [`Registry::add`].

pub mod ask_user;
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

pub use env::{
    Approval, ConfirmDecision, ImageData, McpAppServer, ToolControl, ToolEnv, ToolEvent,
    ToolResourceLease, ToolResult,
};
pub use tool::Tool;

use serde_json::Value;
use std::collections::HashSet;
use wisp_llm::ToolSchema;

/// Where a schema in the model request comes from. This is intentionally a
/// request-time view rather than tool metadata: deferred MCP tools collapse
/// into the two dynamic search/dispatch schemas below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSchemaOrigin {
    BuiltIn,
    Dynamic,
    Subagent,
}

const BUILT_IN_SCHEMA_NAMES: &[&str] = &[
    "read",
    "write",
    "edit",
    "search",
    "grep",
    "shell",
    "view_image",
    "update_plan",
    "attempt_completion",
    "list_skill_catalog",
    "search_skills",
    "search_models",
    "use_skill",
    "search_memory",
];

const SUBAGENT_SCHEMA_NAMES: &[&str] = &["explore", "delegate_tasks", "get_delegated_result"];

const SEARCH_MCP_TOOLS: &str = "search_mcp_tools";
const USE_MCP_TOOL: &str = "use_mcp_tool";
/// Prefix on tool *event* names (not schemas) marking MCP-backed tools, so
/// the UI can highlight external calls (#451). Call and result rows must
/// agree: `run_registered_tool` prefixes the call event, `Registry::event_name`
/// derives the matching result name.
pub const MCP_EVENT_PREFIX: &str = "mcp:";
const DEFAULT_MCP_SEARCH_LIMIT: usize = 5;

/// Tools that stay callable while a session is in plan mode (see
/// [`ToolEnv::plan_mode`]), on top of any tool that reports
/// [`Tool::read_only`]. Everything else is refused at the registry gate:
/// writes, shell/python/R, delegation, and state changes.
/// `search_mcp_tools` is a schema lookup and is allowed by its own path.
///
/// ponytail: the host override half of the gate — one list to read, naming
/// host tools this crate never sees. The other half is `Tool::read_only()`,
/// which covers the tools a list cannot name: MCP retrieval tools, whose
/// server declares `readOnlyHint`. Both fail closed.
pub const PLAN_MODE_READ_ONLY: &[&str] = &[
    // wisp-tools built-ins
    "read",
    "search",
    "grep",
    "view_image",
    "update_plan",
    "attempt_completion",
    // wisp-core / wisp-skills
    "search_memory",
    "search_skills",
    "search_models",
    "use_skill",
    // Desktop host tools that only read or retrieve
    "web_scan",
    "web_open_tab",
    "web_screenshot",
    "get_run",
    "monitor_run",
    "get_delegated_result",
    // The plan proposal tool: plan mode is exactly when it has to run.
    plan::PROPOSE_PLAN,
    // Questions are read-only; planning is exactly when forks surface.
    ask_user::ASK_USER,
];

fn plan_mode_blocks(name: &str) -> bool {
    !PLAN_MODE_READ_ONLY.contains(&name)
}

/// Refusal text for a blocked call. Written at the agent, not the user: it has
/// to steer the model back to planning instead of hunting for a workaround.
fn plan_mode_refusal(name: &str) -> String {
    format!(
        "tool '{name}' is unavailable: this conversation is in plan mode, so it investigates and \
         writes a plan instead of executing one. Keep researching with read-only tools and finish \
         the plan; the user approves it before anything runs."
    )
}

fn project_write_lock_refusal(name: &str) -> String {
    format!(
        "tool '{name}' is unavailable: an active isolated exploration has frozen project writes. \
         This conversation can still inspect the project and answer questions with read-only tools. \
         Promote an exploration, or archive or discard every active candidate, before changing project state."
    )
}
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
        self.schemas_with_origins().0
    }

    /// Return the exact schema list sent to the provider plus one aligned
    /// origin per schema, so context accounting can explain the fixed payload
    /// without rebuilding or guessing at it in the UI.
    pub fn schemas_with_origins(&self) -> (Vec<ToolSchema>, Vec<ToolSchemaOrigin>) {
        let mut schemas = Vec::new();
        let mut origins = Vec::new();
        for tool in self.tools.iter().filter(|tool| !tool.defer_schema()) {
            schemas.push(tool.schema());
            origins.push(if SUBAGENT_SCHEMA_NAMES.contains(&tool.name()) {
                ToolSchemaOrigin::Subagent
            } else if BUILT_IN_SCHEMA_NAMES.contains(&tool.name()) {
                ToolSchemaOrigin::BuiltIn
            } else {
                ToolSchemaOrigin::Dynamic
            });
        }
        if self.tools.iter().any(|tool| tool.defer_schema()) {
            schemas.push(search_mcp_tools_schema());
            origins.push(ToolSchemaOrigin::Dynamic);
            schemas.push(use_mcp_tool_schema());
            origins.push(ToolSchemaOrigin::Dynamic);
        }
        (schemas, origins)
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    /// Every name the approval gate may see: registered tool targets plus
    /// virtual schemas such as the deferred MCP search/dispatch gateway.
    pub fn approval_names(&self) -> HashSet<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .chain(
                self.schemas()
                    .into_iter()
                    .map(|schema| schema.function.name),
            )
            .collect()
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

    /// The event name for a model-requested tool call: MCP-backed tools
    /// (called directly or through `use_mcp_tool`) get [`MCP_EVENT_PREFIX`]
    /// so call and result rows match in the UI transcript.
    pub fn event_name(&self, name: &str, args: &Value) -> String {
        let target = if name == USE_MCP_TOOL {
            args.get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or(name)
        } else {
            name
        };
        if self.get(target).is_some_and(|t| t.defer_schema()) {
            format!("{MCP_EVENT_PREFIX}{target}")
        } else {
            target.to_string()
        }
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
            return ToolResult::fail(format!("tool '{SEARCH_MCP_TOOLS}' was denied by the user"))
                .stop_batch();
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
    // Project freeze is stronger than approval/full-permission settings. Keep
    // ordinary conversations usable for research, but fail closed for every
    // tool that is not known to be retrieval-only.
    if env.project_write_locked() && plan_mode_blocks(name) && !tool.read_only() {
        return ToolResult::fail(project_write_lock_refusal(name));
    }
    // Plan-mode gate, ahead of approvals: a session that is only allowed to
    // plan never reaches the approval prompt for a tool that would execute.
    if env.plan_mode() && plan_mode_blocks(name) && !tool.read_only() {
        return ToolResult::fail(plan_mode_refusal(name));
    }
    // Per-tool approval gate. `Deny` blocks before the call card even shows;
    // `Ask` shows the card then routes through `confirm`; `Allow` runs as before.
    let host_approval = env.approval_mode(name).await;
    let approval = if host_approval == env::Approval::Deny {
        // An explicit block remains a hard policy even in Full Permission.
        env::Approval::Deny
    } else if env.approval_bypass() {
        // Full Permission suppresses both host prompts and a tool's built-in
        // minimum approval requirement. Plan mode already gated mutations
        // above, before this branch.
        env::Approval::Allow
    } else if host_approval == env::Approval::Ask || tool.minimum_approval() == env::Approval::Ask {
        env::Approval::Ask
    } else {
        env::Approval::Allow
    };
    if approval == env::Approval::Deny {
        return ToolResult::fail(format!("tool '{name}' is blocked by the approval policy"));
    }
    let preview = tool.preview(args);
    let event_name = if tool.defer_schema() {
        format!("{MCP_EVENT_PREFIX}{name}")
    } else {
        name.to_string()
    };
    env.emit(ToolEvent::Call {
        name: event_name,
        preview,
    })
    .await;
    if approval == env::Approval::Ask && !env.confirm(&format!("Run tool '{name}'?")).await {
        env.emit(ToolEvent::Result { ok: false }).await;
        return ToolResult::fail(format!("tool '{name}' was denied by the user")).stop_batch();
    }
    let _resource_lease = match env.acquire_tool_resources(name, args).await {
        Ok(lease) => lease,
        Err(error) => {
            env.emit(ToolEvent::Result { ok: false }).await;
            return ToolResult::fail(error).stop_batch();
        }
    };
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
        if image::needs_resize(&path).unwrap_or(false)
            && !env
                .confirm(&format!(
                    "{}Resize large image for model input? The original file will not be changed: {}",
                    image::RESIZE_CONFIRM_PREFIX,
                    path.display()
                ))
                .await
        {
            return ToolResult::fail("view_image cancelled: image resize was not approved");
        }
        image::view_image_resized(&path.to_string_lossy())
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::env::{Approval, ToolEnv, ToolEvent};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn only_audited_builtin_retrievals_opt_into_parallel_batches() {
        let tools = Registry::builtins();
        for name in ["read", "search", "grep"] {
            assert!(
                tools.get(name).is_some_and(|tool| tool.parallel_safe()),
                "{name} should be parallel-safe"
            );
        }
        for name in ["write", "edit", "shell", "view_image"] {
            assert!(
                !tools.get(name).is_some_and(|tool| tool.parallel_safe()),
                "{name} must stay sequential"
            );
        }
    }

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
        /// What the bundled bio/literature servers declare via `readOnlyHint`.
        fn read_only(&self) -> bool {
            true
        }
        async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            ToolResult::ok(format!("searched {}", args["query"]))
        }
    }

    /// An MCP tool whose server says nothing about writing — the unclassified
    /// case the gate has to keep refusing.
    struct DeferredWriteTool;
    #[async_trait::async_trait]
    impl Tool for DeferredWriteTool {
        fn name(&self) -> &str {
            "zenodo_upload_record"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new(self.name(), "Upload a record.", serde_json::json!({}))
        }
        fn defer_schema(&self) -> bool {
            true
        }
        async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            ToolResult::ok("uploaded")
        }
    }

    struct PolicyEnv {
        root: PathBuf,
        mode: Approval,
        confirm_ok: bool,
        bypass: bool,
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
        fn approval_bypass(&self) -> bool {
            self.bypass
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    struct EventEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    struct LeaseEnv {
        root: PathBuf,
        held: Arc<AtomicBool>,
        released: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for LeaseEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn acquire_tool_resources(
            &self,
            _tool: &str,
            _args: &Value,
        ) -> Result<Option<ToolResourceLease>, String> {
            self.held.store(true, Ordering::SeqCst);
            let held = self.held.clone();
            let released = self.released.clone();
            Ok(Some(ToolResourceLease::new(move || {
                held.store(false, Ordering::SeqCst);
                released.store(true, Ordering::SeqCst);
            })))
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    struct LeaseAwareTool {
        held: Arc<AtomicBool>,
        before_ran: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl Tool for LeaseAwareTool {
        fn name(&self) -> &str {
            "lease_aware"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new(self.name(), "test", serde_json::json!({}))
        }
        async fn before(&self, _args: &Value, _env: &dyn ToolEnv) {
            assert!(self.held.load(Ordering::SeqCst));
            self.before_ran.store(true, Ordering::SeqCst);
        }
        async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            assert!(self.before_ran.load(Ordering::SeqCst));
            assert!(self.held.load(Ordering::SeqCst));
            ToolResult::ok("ran while leased")
        }
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
            bypass: false,
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
            bypass: false,
        };
        let result = registry
            .run("third_party", &serde_json::json!({}), &env)
            .await;
        assert!(!result.success);
        assert_eq!(result.control, ToolControl::StopBatch);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn approval_bypass_skips_minimum_prompt_but_not_explicit_deny() {
        static RAN: AtomicBool = AtomicBool::new(false);
        let mut registry = Registry { tools: vec![] };
        registry.add(Box::new(AskSpy(&RAN)));

        RAN.store(false, Ordering::SeqCst);
        let bypass = PolicyEnv {
            root: PathBuf::from("."),
            mode: Approval::Allow,
            confirm_ok: false,
            bypass: true,
        };
        let allowed = registry
            .run("third_party", &serde_json::json!({}), &bypass)
            .await;
        assert!(allowed.success);
        assert!(RAN.load(Ordering::SeqCst));

        RAN.store(false, Ordering::SeqCst);
        let denied = PolicyEnv {
            root: PathBuf::from("."),
            mode: Approval::Deny,
            confirm_ok: true,
            bypass: true,
        };
        let blocked = registry
            .run("third_party", &serde_json::json!({}), &denied)
            .await;
        assert!(!blocked.success);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn approval_gate() {
        // Deny: never runs, fails.
        let (ran, res) = run_with(Approval::Deny, true).await;
        assert!(!ran && !res.success, "deny must block the tool");
        assert_eq!(res.control, ToolControl::Continue);
        // Ask + confirm no: never runs, fails.
        let (ran, res) = run_with(Approval::Ask, false).await;
        assert!(!ran && !res.success, "ask+deny must block the tool");
        assert_eq!(res.control, ToolControl::StopBatch);
        // Ask + confirm yes: runs.
        let (ran, res) = run_with(Approval::Ask, true).await;
        assert!(ran && res.success, "ask+approve must run the tool");
        // Allow: runs without asking.
        let (ran, res) = run_with(Approval::Allow, false).await;
        assert!(ran && res.success, "allow must run the tool");
    }

    #[tokio::test]
    async fn resource_lease_covers_before_and_run_then_releases() {
        let held = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let before_ran = Arc::new(AtomicBool::new(false));
        let mut registry = Registry { tools: vec![] };
        registry.add(Box::new(LeaseAwareTool {
            held: held.clone(),
            before_ran,
        }));
        let env = LeaseEnv {
            root: PathBuf::from("."),
            held: held.clone(),
            released: released.clone(),
        };

        let result = registry
            .run("lease_aware", &serde_json::json!({}), &env)
            .await;

        assert!(result.success, "{}", result.content);
        assert!(!held.load(Ordering::SeqCst));
        assert!(released.load(Ordering::SeqCst));
    }

    struct PlanEnv {
        root: PathBuf,
        plan: bool,
        project_locked: bool,
    }
    #[async_trait::async_trait]
    impl ToolEnv for PlanEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        fn plan_mode(&self) -> bool {
            self.plan
        }
        fn project_write_locked(&self) -> bool {
            self.project_locked
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    #[tokio::test]
    async fn plan_mode_blocks_writers_and_lets_readers_through() {
        let dir = std::env::temp_dir().join(format!(
            "wisp-plan-mode-gate-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        let reg = Registry::builtins();
        let write = serde_json::json!({ "path": "gated.txt", "content": "no" });
        let read = serde_json::json!({ "path": dir.join("note.txt").to_string_lossy() });

        let planning = PlanEnv {
            root: dir.clone(),
            plan: true,
            project_locked: false,
        };
        let blocked = reg.run("write", &write, &planning).await;
        assert!(!blocked.success);
        assert!(blocked.content.contains("plan mode"), "{}", blocked.content);
        assert!(!dir.join("gated.txt").exists(), "the write must not happen");
        assert!(reg.run("read", &read, &planning).await.success);

        // Same calls with plan mode off: the gate is invisible.
        let executing = PlanEnv {
            root: dir.clone(),
            plan: false,
            project_locked: false,
        };
        assert!(reg.run("write", &write, &executing).await.success);
        assert!(reg.run("read", &read, &executing).await.success);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn project_write_lock_blocks_mutations_but_keeps_conversation_retrieval_available() {
        let dir = std::env::temp_dir().join(format!(
            "wisp-project-write-lock-gate-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        let reg = Registry::builtins();
        let locked = PlanEnv {
            root: dir.clone(),
            plan: false,
            project_locked: true,
        };

        let blocked = reg
            .run(
                "write",
                &serde_json::json!({ "path": "gated.txt", "content": "no" }),
                &locked,
            )
            .await;
        assert!(!blocked.success);
        assert!(
            blocked.content.contains("active isolated exploration"),
            "{}",
            blocked.content
        );
        assert!(!dir.join("gated.txt").exists(), "the write must not happen");
        assert!(
            reg.run(
                "read",
                &serde_json::json!({ "path": dir.join("note.txt").to_string_lossy() }),
                &locked,
            )
            .await
            .success
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn plan_mode_lets_read_only_retrieval_through() {
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(DeferredTool));
        reg.add(Box::new(DeferredWriteTool));
        let planning = PlanEnv {
            root: PathBuf::from("."),
            plan: true,
            project_locked: false,
        };

        // Dispatched the way the model reaches a deferred MCP tool.
        let searched = reg
            .run(
                USE_MCP_TOOL,
                &serde_json::json!({
                    "tool_name": "pubmed_search_articles",
                    "tool_input": { "query": "tp53" },
                }),
                &planning,
            )
            .await;
        assert!(searched.success, "{}", searched.content);
        assert!(searched.content.contains("tp53"));

        let blocked = reg
            .run(
                USE_MCP_TOOL,
                &serde_json::json!({
                    "tool_name": "zenodo_upload_record",
                    "tool_input": {},
                }),
                &planning,
            )
            .await;
        assert!(!blocked.success, "an unhinted MCP tool must stay blocked");
        assert!(blocked.content.contains("plan mode"), "{}", blocked.content);
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

        let (schemas, origins) = reg.schemas_with_origins();
        let names: Vec<_> = schemas
            .into_iter()
            .map(|schema| schema.function.name)
            .collect();

        assert_eq!(names, ["spy", SEARCH_MCP_TOOLS, USE_MCP_TOOL]);
        assert_eq!(origins, vec![ToolSchemaOrigin::Dynamic; 3]);
        assert!(!names.contains(&"pubmed_search_articles".to_string()));
    }

    #[test]
    fn built_in_schemas_are_marked_for_context_accounting() {
        let (_, origins) = Registry::builtins().schemas_with_origins();
        assert!(!origins.is_empty());
        assert!(origins
            .iter()
            .all(|origin| *origin == ToolSchemaOrigin::BuiltIn));
    }

    #[test]
    fn deferred_gateway_and_target_names_share_one_approval_set() {
        let mut registry = Registry { tools: vec![] };
        registry.add(Box::new(DeferredTool));

        assert_eq!(
            registry.approval_names(),
            HashSet::from([
                "pubmed_search_articles".to_string(),
                SEARCH_MCP_TOOLS.to_string(),
                USE_MCP_TOOL.to_string(),
            ])
        );
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
            ToolEvent::Call { name, .. } if name == "mcp:pubmed_search_articles"
        )));
    }

    #[test]
    fn event_name_prefixes_mcp_backed_tools() {
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(DeferredTool));
        let via_dispatch = serde_json::json!({ "tool_name": "pubmed_search_articles" });
        assert_eq!(
            reg.event_name(USE_MCP_TOOL, &via_dispatch),
            "mcp:pubmed_search_articles"
        );
        assert_eq!(
            reg.event_name("pubmed_search_articles", &Value::Null),
            "mcp:pubmed_search_articles"
        );
        assert_eq!(reg.event_name("shell", &Value::Null), "shell");
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
