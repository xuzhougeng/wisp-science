//! Agent runtime for Wisp: context compaction, system-prompt assembly, the
//! agent loop, markdown memory, and memory tools.

pub mod agent;
pub mod context;
pub mod memory;
pub mod output;
pub mod provenance;
pub mod system_prompt;

pub use agent::{agent_loop, agent_loop_continue};
pub use context::ContextManager;
pub use memory::MemoryManager;
pub use output::{NullOutput, Output, StreamSinkAdapter, ToolEnvAdapter};
pub use provenance::ProvenanceRecord;
pub use system_prompt::SystemPrompt;

use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use wisp_llm::{Provider, ProviderConfig, ToolSchema};
use wisp_skills::SkillIndex;
use wisp_tools::{Registry, Tool, ToolEnv, ToolResult};

/// Build the working tool registry: built-ins + `use_skill` + optional memory tools.
pub fn build_registry(
    skills: Arc<SkillIndex>,
    memory: Arc<MemoryManager>,
    memory_enabled: bool,
) -> Registry {
    let mut reg = Registry::builtins();
    reg.add(Box::new(wisp_skills::UseSkillTool::new(skills)));
    if memory_enabled {
        reg.add(Box::new(SearchMemoryTool::new(memory.clone())));
        reg.add(Box::new(AppendMemoryTool::new(memory)));
    }
    reg
}

/// A ready-to-run agent: provider, tools, context, project root, session file.
pub struct Agent {
    pub provider: Box<dyn Provider>,
    pub vision_provider: Option<Box<dyn Provider>>,
    pub tools: Registry,
    pub ctx: ContextManager,
    pub root: PathBuf,
    pub max_iter: usize,
    pub session_path: PathBuf,
}

impl Agent {
    pub fn new(
        cfg: ProviderConfig,
        skills: Arc<SkillIndex>,
        memory: Arc<MemoryManager>,
        root: PathBuf,
        max_context: usize,
        max_iter: usize,
        memory_enabled: bool,
        vision_cfg: Option<ProviderConfig>,
    ) -> Self {
        let provider = wisp_llm::build(cfg);
        let vision_provider = vision_cfg.map(wisp_llm::build);
        let tools = build_registry(skills, memory, memory_enabled);
        let session_path = root.join(".wisp").join("session.json");
        let mut ctx = ContextManager::new(max_context);
        ctx.load(&session_path);
        Self {
            provider,
            vision_provider,
            tools,
            ctx,
            root,
            max_iter,
            session_path,
        }
    }

    /// Seed the system prompt once when the session is fresh.
    pub fn seed_system_prompt(&mut self, skills: &SkillIndex, compute_hosts: Option<String>) {
        if self.ctx.is_empty() {
            let prompt = SystemPrompt::new(&self.root, skills, compute_hosts).assemble();
            self.ctx.append_system(prompt);
        }
    }

    pub async fn run(
        &mut self,
        user_input: &str,
        output: &dyn Output,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<()> {
        agent_loop(
            &mut self.ctx,
            self.provider.as_ref(),
            self.vision_provider.as_deref(),
            &self.tools,
            &self.root,
            output,
            user_input,
            self.max_iter,
            cancel,
        )
        .await
    }

    /// Resume a failed turn without appending another user message.
    pub async fn run_resume(
        &mut self,
        output: &dyn Output,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<()> {
        agent_loop_continue(
            &mut self.ctx,
            self.provider.as_ref(),
            self.vision_provider.as_deref(),
            &self.tools,
            &self.root,
            output,
            self.max_iter,
            cancel,
        )
        .await
    }

    /// Register an extra tool (e.g. the Python `repl` tool or MCP tools).
    pub fn add_tool(&mut self, tool: Box<dyn wisp_tools::Tool>) {
        self.tools.add(tool);
    }

    pub fn save(&self) {
        self.ctx.save(&self.session_path);
    }
}

// --- memory tools ---

pub struct SearchMemoryTool {
    memory: Arc<MemoryManager>,
}
impl SearchMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "search_memory",
            "Search YOUR long-term memory — notes saved in past sessions. Call when the user references past work, before recommending architecture/patterns, or when asked about preferences/conventions.",
            json!({ "type": "object", "properties": { "query": { "type": "string", "description": "Search query (space-separated keywords, EN or ZH)" } }, "required": ["query"] }),
        )
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let q = match args.get("query").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::fail("missing 'query'"),
        };
        ToolResult::ok(self.memory.search(&q, 10))
    }
}

pub struct AppendMemoryTool {
    memory: Arc<MemoryManager>,
}
impl AppendMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for AppendMemoryTool {
    fn name(&self) -> &str {
        "append_memory"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "append_memory",
            "Save a note to YOUR long-term memory. Persists across sessions. Use for preferences, architecture decisions, non-obvious bug fixes, project conventions. Not for ephemeral context.",
            json!({ "type": "object", "properties": { "content": { "type": "string", "description": "Concise 5-10 sentence note. Prefix tag: [PREFERENCE]/[DECISION]/[BUG-FIX]/[CONVENTION]" } }, "required": ["content"] }),
        )
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let c = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::fail("missing 'content'"),
        };
        match self.memory.append(&c) {
            Ok(_) => ToolResult::ok("memory appended"),
            Err(e) => ToolResult::fail(format!("append_memory error: {e}")),
        }
    }
}
