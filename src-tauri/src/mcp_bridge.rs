//! Stdio MCP bridge exposed to external ACP agents.
//!
//! The bridge intentionally exposes Wisp's scientific capabilities (skills,
//! bundled bio MCP, custom MCP, run contexts) without forwarding Wisp's generic
//! shell/edit/read tools. Local runners already have their own filesystem tools;
//! this process is only for Wisp-native capabilities and policy/config reuse.

use crate::{
    bio_domains, connect_mcp, load_disabled_connectors, load_disabled_skills,
    load_enabled_skill_names, load_mcp_connections, run_context, skill_paths,
};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use wisp_skills::{list_resources, SkillIndex};
use wisp_store::Store;
use wisp_tools::{Approval, Tool, ToolEnv, ToolEvent, ToolResult};

#[derive(Debug, Clone)]
pub(crate) struct BridgeConfig {
    pub(crate) app_data: PathBuf,
    pub(crate) project_root: PathBuf,
    pub(crate) resource_root: Option<PathBuf>,
    pub(crate) project_id: String,
    pub(crate) frame_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcIn {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Clone)]
enum Route {
    Bio {
        client: Arc<wisp_mcp::McpClient>,
        remote_name: String,
        description: String,
        input_schema: Value,
    },
    Custom {
        client: Arc<wisp_mcp::McpClient>,
        remote_name: String,
        description: String,
        input_schema: Value,
    },
}

struct BridgeServer {
    cfg: BridgeConfig,
    store: Store,
    memory: wisp_core::MemoryManager,
    run_manager: run_context::RunManager,
    skills: Arc<SkillIndex>,
    routes: HashMap<String, Route>,
    bundled_bio_tools_loaded: bool,
    custom_mcp_tools_loaded: bool,
}

impl BridgeServer {
    async fn new(cfg: BridgeConfig) -> Result<Self> {
        if let Some(root) = &cfg.resource_root {
            wisp_paths::set_resource_root(root.clone());
        }
        std::fs::create_dir_all(&cfg.app_data).ok();
        let store = Store::open(&cfg.app_data.join("wisp.sqlite"))
            .await
            .context("open Wisp store for MCP bridge")?;
        let run_manager = run_context::RunManager::new();
        run_manager
            .recover(&store)
            .await
            .map_err(anyhow::Error::msg)?;
        let raw = SkillIndex::load(&skill_paths(&cfg.project_root));
        let skills = Arc::new(filter_skills(&store, &cfg.project_id, raw).await);
        let memory = wisp_core::MemoryManager::new(&cfg.project_root);
        Ok(Self {
            cfg,
            store,
            memory,
            run_manager,
            skills,
            routes: HashMap::new(),
            bundled_bio_tools_loaded: false,
            custom_mcp_tools_loaded: false,
        })
    }

    async fn handle(&mut self, req: JsonRpcIn) -> Option<Value> {
        let id = req.id?;
        let result = match req.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "wisp_bridge", "version": env!("CARGO_PKG_VERSION") }
            })),
            "tools/list" => self.tools_list().await,
            "tools/call" => self.tools_call(req.params.unwrap_or_default()).await,
            _ => Err(anyhow!("unknown MCP method '{}'", req.method)),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": e.to_string() }
            }),
        })
    }

    async fn tools_list(&mut self) -> Result<Value> {
        let mut tools = vec![
            get_capabilities_tool_schema(),
            json!({
                "name": "wisp_list_skills",
                "description": "List skills currently available from the active Wisp project/profile.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "wisp_use_skill",
                "description": "Load a Wisp skill's SKILL.md guidance plus script/reference file paths.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "name": { "type": "string", "description": "Wisp skill name" } },
                    "required": ["name"]
                }
            }),
            search_memory_tool_schema(),
            list_artifacts_tool_schema(),
            get_research_graph_tool_schema(),
            list_execution_contexts_tool_schema(),
            run_in_context_tool_schema(),
            get_run_tool_schema(),
            monitor_run_tool_schema(),
            cancel_run_tool_schema(),
        ];
        if let Some(frame_id) = self.cfg.frame_id.as_deref() {
            if crate::delegation_runtime::session_delegation_enabled(&self.store, frame_id).await {
                tools.push(propose_delegation_tool_schema());
            }
        }
        let remote = self.ensure_remote_tools().await?;
        tools.extend(remote);
        Ok(json!({ "tools": tools }))
    }

    async fn tools_call(&mut self, params: Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("tools/call missing name"))?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let (text, is_error) = match name {
            "wisp_get_capabilities" => (self.capabilities_text().await?, false),
            "wisp_list_skills" => (self.list_skills_text(), false),
            "wisp_use_skill" => match self.use_skill_text(&args) {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_search_memory" => match self.search_memory_text(&args) {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_list_artifacts" => match self.list_artifacts_text(&args).await {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_get_research_graph" => match self.research_graph_text().await {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_list_execution_contexts" => match self.execution_contexts_text().await {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_run_in_context" => {
                let result = self.run_in_context(&args).await;
                (result.content, !result.success)
            }
            "wisp_get_run" => {
                let result = self.get_run(&args).await;
                (result.content, !result.success)
            }
            "wisp_monitor_run" => {
                let result = self.monitor_run(&args).await;
                (result.content, !result.success)
            }
            "wisp_cancel_run" => {
                let result = self.cancel_run(&args).await;
                (result.content, !result.success)
            }
            "wisp_propose_delegation" => {
                let Some(frame_id) = self.cfg.frame_id.as_deref() else {
                    return Err(anyhow!("delegation requires a conversation frame"));
                };
                let tool = crate::delegation_tool::ProposeDelegationTool::for_project(
                    self.store.clone(),
                    self.cfg.project_id.clone(),
                    self.cfg.project_root.clone(),
                    frame_id,
                );
                let result = tool
                    .run(
                        &args,
                        &BridgeToolEnv {
                            project_root: self.cfg.project_root.clone(),
                        },
                    )
                    .await;
                (result.content, !result.success)
            }
            other => {
                self.ensure_remote_tools().await?;
                let Some(route) = self.routes.get(other).cloned() else {
                    return Err(anyhow!("unknown Wisp bridge tool '{other}'"));
                };
                match route {
                    Route::Bio {
                        client,
                        remote_name,
                        ..
                    }
                    | Route::Custom {
                        client,
                        remote_name,
                        ..
                    } => match client.tool_call(&remote_name, &args).await {
                        Ok(s) => (s, false),
                        Err(e) => (e.to_string(), true),
                    },
                }
            }
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error
        }))
    }

    async fn ensure_remote_tools(&mut self) -> Result<Vec<Value>> {
        if !self.bundled_bio_tools_loaded {
            self.bundled_bio_tools_loaded = true;
            self.register_bundled_bio_tools().await;
        }
        if !self.custom_mcp_tools_loaded {
            self.custom_mcp_tools_loaded = true;
            self.register_custom_mcp_tools().await;
        }
        Ok(self.route_tools())
    }

    async fn register_bundled_bio_tools(&mut self) {
        let disabled = load_disabled_connectors(&self.store).await;
        let domains = bio_domains();
        let all_off = !domains.is_empty() && domains.iter().all(|d| disabled.contains(&d.slug));
        if all_off {
            return;
        }
        let skip: HashSet<String> = domains
            .iter()
            .filter(|d| disabled.contains(&d.slug))
            .flat_map(|d| d.tools.iter().cloned())
            .collect();
        let Ok(env) = wisp_runtime::PythonEnv::ensure(&self.cfg.app_data) else {
            return;
        };
        let pkg = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
        let Ok(client) = wisp_mcp::McpClient::launch_bio_tools(
            &env.python(),
            &pkg,
            &crate::models::service_env(),
        )
        .await
        else {
            return;
        };
        let client = Arc::new(client);
        let Ok(tools) = client.tools_list().await else {
            return;
        };
        for tool in tools {
            if tool.name.is_empty() || skip.contains(&tool.name) || self.is_reserved(&tool.name) {
                continue;
            }
            self.routes.insert(
                tool.name.clone(),
                Route::Bio {
                    client: client.clone(),
                    remote_name: tool.name.clone(),
                    description: tool.description,
                    input_schema: tool.input_schema,
                },
            );
        }
    }

    async fn register_custom_mcp_tools(&mut self) {
        let conns = load_mcp_connections(&self.store)
            .await
            .into_iter()
            .filter(|c| c.enabled)
            .collect::<Vec<_>>();
        for conn in conns {
            let Ok(client) = connect_mcp(&conn).await else {
                continue;
            };
            let client = Arc::new(client);
            let Ok(tools) = client.tools_list().await else {
                continue;
            };
            let prefix = format!("wisp_custom_{}__", sanitize_tool_part(&conn.id));
            for tool in tools {
                if tool.name.is_empty() {
                    continue;
                }
                let exposed = format!("{prefix}{}", sanitize_tool_part(&tool.name));
                if self.is_reserved(&exposed) {
                    continue;
                }
                self.routes.insert(
                    exposed,
                    Route::Custom {
                        client: client.clone(),
                        remote_name: tool.name,
                        description: tool.description,
                        input_schema: tool.input_schema,
                    },
                );
            }
        }
    }

    fn route_tools(&self) -> Vec<Value> {
        self.routes
            .iter()
            .map(|(name, route)| {
                let (desc, input_schema) = match route {
                    Route::Bio {
                        remote_name,
                        description,
                        input_schema,
                        ..
                    } => (
                        if description.trim().is_empty() {
                            format!("Bundled Wisp bio MCP tool `{remote_name}`.")
                        } else {
                            description.clone()
                        },
                        input_schema.clone(),
                    ),
                    Route::Custom {
                        remote_name,
                        description,
                        input_schema,
                        ..
                    } => (
                        if description.trim().is_empty() {
                            format!("Custom Wisp MCP tool `{remote_name}`.")
                        } else {
                            description.clone()
                        },
                        input_schema.clone(),
                    ),
                };
                json!({
                    "name": name,
                    "description": desc,
                    "inputSchema": input_schema
                })
            })
            .collect()
    }

    fn is_reserved(&self, name: &str) -> bool {
        is_builtin_tool(name) || self.routes.contains_key(name)
    }

    fn list_skills_text(&self) -> String {
        if self.skills.is_empty() {
            return "No Wisp skills are currently available. If this is a portable build, verify the skills/ resource directory is next to wisp-tauri.exe.".into();
        }
        self.skills
            .all()
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn use_skill_text(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required argument 'name'"))?;
        let Some(skill) = self.skills.get(name) else {
            return Err(anyhow!("skill '{name}' not found"));
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
        Ok(out)
    }

    async fn capabilities_text(&self) -> Result<String> {
        let delegation_enabled = match self.cfg.frame_id.as_deref() {
            Some(frame_id) => {
                crate::delegation_runtime::session_delegation_enabled(&self.store, frame_id).await
            }
            None => false,
        };
        pretty_json(&json!({
            "schemaVersion": 1,
            "projectId": self.cfg.project_id,
            "frameId": self.cfg.frame_id,
            "actor": "acp_agent",
            "scope": "active_project",
            "capabilities": [
                { "name": "skills.read", "allowed": true, "tools": ["wisp_list_skills", "wisp_use_skill"] },
                { "name": "memory.read", "allowed": true, "tools": ["wisp_search_memory"] },
                { "name": "artifacts.read", "allowed": true, "tools": ["wisp_list_artifacts"] },
                { "name": "research_graph.read", "allowed": true, "tools": ["wisp_get_research_graph"] },
                { "name": "execution_contexts.read", "allowed": true, "tools": ["wisp_list_execution_contexts"] },
                { "name": "runs.read", "allowed": true, "tools": ["wisp_get_run", "wisp_monitor_run"] },
                {
                    "name": "runs.execute",
                    "allowed": true,
                    "tools": ["wisp_run_in_context", "wisp_cancel_run"],
                    "policy": "non_interactive; dangerous commands requiring confirmation are denied"
                },
                { "name": "scientific_mcp", "allowed": true, "discovery": "tools/list" },
                {
                    "name": "harness.write",
                    "allowed": false,
                    "reason": "Memory, artifact, graph, and persistent runtime writes require an approval broker and are not exposed by this bridge."
                },
                {
                    "name": "delegation.propose",
                    "allowed": delegation_enabled,
                    "tools": if delegation_enabled { vec!["wisp_propose_delegation"] } else { vec![] },
                    "policy": "draft_only; explicit UI approval and run remain required"
                }
            ]
        }))
    }

    fn search_memory_text(&self, args: &Value) -> Result<String> {
        let query = required_string(args, "query")?;
        let top_k = bounded_i64(args, "top_k", 5, 1, 10) as usize;
        pretty_json(&json!({
            "query": query,
            "topK": top_k,
            "results": self.memory.search(query, top_k)
        }))
    }

    async fn list_artifacts_text(&self, args: &Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let limit = bounded_i64(args, "limit", 20, 1, 50);
        let rows = self
            .store
            .search_project_artifacts(&self.cfg.project_id, query, limit)
            .await?;
        let artifacts = rows
            .into_iter()
            .map(|(id, filename, content_type, storage_path, created_at)| {
                json!({
                    "id": id,
                    "filename": filename,
                    "contentType": content_type,
                    "storagePath": storage_path,
                    "createdAt": created_at
                })
            })
            .collect::<Vec<_>>();
        pretty_json(&json!({
            "projectId": self.cfg.project_id,
            "query": query,
            "artifacts": artifacts
        }))
    }

    async fn research_graph_text(&self) -> Result<String> {
        let graph = self.store.research_graph(&self.cfg.project_id).await?;
        pretty_json(&json!({
            "projectId": self.cfg.project_id,
            "graph": graph
        }))
    }

    async fn execution_contexts_text(&self) -> Result<String> {
        let contexts = self.store.list_execution_contexts().await?;
        let contexts = contexts
            .into_iter()
            .map(|context| {
                json!({
                    "id": context.id,
                    "kind": context.kind.as_str(),
                    "label": context.label,
                    "capabilities": parse_json_or_string(&context.capabilities_json),
                    "lastProbeAt": context.last_probe_at,
                    "lastProbeStatus": context.last_probe_status,
                    "lastProbeError": context.last_probe_error
                })
            })
            .collect::<Vec<_>>();
        pretty_json(&json!({ "contexts": contexts }))
    }

    async fn run_in_context(&self, args: &Value) -> ToolResult {
        let tool = run_context::RunInContextTool::new(
            self.store.clone(),
            self.run_manager.clone(),
            self.cfg.project_id.clone(),
            self.cfg.frame_id.clone(),
        );
        let env = BridgeToolEnv {
            project_root: self.cfg.project_root.clone(),
        };
        tool.run(args, &env).await
    }

    async fn get_run(&self, args: &Value) -> ToolResult {
        let tool = run_context::GetRunTool::new(self.store.clone(), self.cfg.project_id.clone());
        let env = BridgeToolEnv {
            project_root: self.cfg.project_root.clone(),
        };
        tool.run(args, &env).await
    }

    async fn monitor_run(&self, args: &Value) -> ToolResult {
        let tool =
            run_context::MonitorRunTool::new(self.store.clone(), self.cfg.project_id.clone());
        let env = BridgeToolEnv {
            project_root: self.cfg.project_root.clone(),
        };
        tool.run(args, &env).await
    }

    async fn cancel_run(&self, args: &Value) -> ToolResult {
        let tool = run_context::CancelRunTool::new(
            self.store.clone(),
            self.run_manager.clone(),
            self.cfg.project_id.clone(),
        );
        let env = BridgeToolEnv {
            project_root: self.cfg.project_root.clone(),
        };
        tool.run(args, &env).await
    }
}

async fn filter_skills(store: &Store, project_id: &str, raw: SkillIndex) -> SkillIndex {
    if let Some(enabled) = load_enabled_skill_names(store, project_id).await {
        return raw.filtered_by_names(Some(&enabled));
    }
    let disabled = load_disabled_skills(store).await;
    if disabled.is_empty() {
        raw
    } else {
        raw.filtered(&disabled)
    }
}

fn pretty_json(value: &Value) -> Result<String> {
    serde_json::to_string_pretty(value).context("serialize Wisp bridge response")
}

fn required_string<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing required argument '{name}'"))
}

fn bounded_i64(args: &Value, name: &str, default: i64, min: i64, max: i64) -> i64 {
    args.get(name)
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(min, max)
}

fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn get_capabilities_tool_schema() -> Value {
    json!({
        "name": "wisp_get_capabilities",
        "description": "Describe the project-scoped Wisp Harness capabilities granted to this ACP session, including intentionally unavailable write operations.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn search_memory_tool_schema() -> Value {
    json!({
        "name": "wisp_search_memory",
        "description": "Search the active project's durable Wisp memory. Read-only; does not append or alter memory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keywords to find in project memory" },
                "top_k": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
            },
            "required": ["query"],
            "additionalProperties": false
        }
    })
}

fn list_artifacts_tool_schema() -> Value {
    json!({
        "name": "wisp_list_artifacts",
        "description": "List persisted artifacts owned by the active Wisp project, optionally filtering by filename.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Optional filename substring" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
            },
            "additionalProperties": false
        }
    })
}

fn get_research_graph_tool_schema() -> Value {
    json!({
        "name": "wisp_get_research_graph",
        "description": "Read the active project's Wisp research graph (nodes and edges).",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn list_execution_contexts_tool_schema() -> Value {
    json!({
        "name": "wisp_list_execution_contexts",
        "description": "List Wisp execution contexts and probe/capability summaries without exposing stored connection configuration.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn run_in_context_tool_schema() -> Value {
    json!({
        "name": "wisp_run_in_context",
        "description": "Submit a persisted background Wisp Run in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). Set wait_for_completion=true for direct model-free waiting, or call wisp_monitor_run exactly once with the returned id. Never poll with wisp_get_run. Dangerous commands require approval and are rejected in this non-interactive bridge.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                "command": { "type": "string", "description": "Command to execute in that context" },
                "title": { "type": "string", "description": "Short run title" },
                "timeout_secs": { "type": "integer", "description": "Job wall timeout. SSH: 1s..7d (default 4h); local/WSL: 1s..300s" },
                "wait_for_completion": { "type": "boolean", "description": "Suspend the tool until the Run is terminal without repeatedly calling wisp_get_run (default false)" },
                "input_paths": {
                    "type": "array",
                    "description": "Optional project-relative files staged flat into an SSH Run workdir",
                    "items": { "type": "string" }
                },
                "output_specs": {
                    "type": "array",
                    "description": "Optional output specs. SSH direct currently accepts explicit ssh:// references only",
                    "items": {
                        "type": "object",
                        "properties": {
                            "glob": { "type": "string" },
                            "kind": { "type": "string" },
                            "residency": { "type": "string", "enum": ["local", "remote", "auto"] },
                            "max_file_mb": { "type": "integer" },
                            "max_total_mb": { "type": "integer" }
                        },
                        "required": ["glob", "kind", "residency"]
                    }
                }
            },
            "required": ["context_id", "command"]
        }
    })
}

fn get_run_tool_schema() -> Value {
    json!({
        "name": "wisp_get_run",
        "description": "Read one immediate Run status snapshot. Do not call repeatedly to wait; call wisp_monitor_run exactly once for live monitoring.",
        "inputSchema": {
            "type": "object",
            "properties": { "run_id": { "type": "string" } },
            "required": ["run_id"],
            "additionalProperties": false
        }
    })
}

fn monitor_run_tool_schema() -> Value {
    json!({
        "name": "wisp_monitor_run",
        "description": "Monitor one existing long-running Run until it finishes. Call once instead of repeatedly calling wisp_get_run; Wisp waits without repeated model calls or token use.",
        "inputSchema": {
            "type": "object",
            "properties": { "run_id": { "type": "string" } },
            "required": ["run_id"],
            "additionalProperties": false
        }
    })
}

fn cancel_run_tool_schema() -> Value {
    json!({
        "name": "wisp_cancel_run",
        "description": "Request cancellation of a submitted or running Run. SSH Runs remain `cancelling` until the remote process group confirms termination.",
        "inputSchema": {
            "type": "object",
            "properties": { "run_id": { "type": "string" } },
            "required": ["run_id"],
            "additionalProperties": false
        }
    })
}

fn propose_delegation_tool_schema() -> Value {
    let schema = crate::delegation_tool::propose_delegation_schema();
    json!({
        "name": "wisp_propose_delegation",
        "description": schema.function.description,
        "inputSchema": schema.function.parameters,
    })
}

fn is_builtin_tool(name: &str) -> bool {
    matches!(
        name,
        "wisp_get_capabilities"
            | "wisp_list_skills"
            | "wisp_use_skill"
            | "wisp_search_memory"
            | "wisp_list_artifacts"
            | "wisp_get_research_graph"
            | "wisp_list_execution_contexts"
            | "wisp_run_in_context"
            | "wisp_get_run"
            | "wisp_monitor_run"
            | "wisp_cancel_run"
            | "wisp_propose_delegation"
    )
}

struct BridgeToolEnv {
    project_root: PathBuf,
}

#[async_trait::async_trait]
impl ToolEnv for BridgeToolEnv {
    fn project_root(&self) -> &Path {
        &self.project_root
    }
    async fn confirm(&self, _message: &str) -> bool {
        false
    }
    async fn approval_mode(&self, _tool: &str) -> Approval {
        Approval::Allow
    }
    fn danger_auto_approve(&self) -> bool {
        false
    }
    async fn emit(&self, _event: ToolEvent) {}
}

fn sanitize_tool_part(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "tool".into()
    } else {
        out
    }
}

pub(crate) async fn run_stdio(cfg: BridgeConfig) -> Result<()> {
    let mut server = BridgeServer::new(cfg).await?;
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<JsonRpcIn>(raw) else {
            continue;
        };
        if let Some(resp) = server.handle(req).await {
            stdout.write_all(resp.to_string().as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

/// CLI args for the `--wisp-mcp-bridge` re-exec mode used by ACP agents.
fn parse_mcp_bridge_cli_args() -> BridgeConfig {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut app_data: Option<PathBuf> = None;
    let mut project_root: Option<PathBuf> = None;
    let mut resource_root: Option<PathBuf> = None;
    let mut project_id = "default".to_string();
    let mut frame_id: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--app-data" => {
                i += 1;
                app_data = args.get(i).map(PathBuf::from);
            }
            "--project-root" => {
                i += 1;
                project_root = args.get(i).map(PathBuf::from);
            }
            "--resource-root" => {
                i += 1;
                resource_root = args.get(i).map(PathBuf::from);
            }
            "--project-id" => {
                i += 1;
                if let Some(v) = args.get(i).filter(|s| !s.trim().is_empty()) {
                    project_id = v.clone();
                }
            }
            "--frame-id" => {
                i += 1;
                frame_id = args.get(i).filter(|s| !s.trim().is_empty()).cloned();
            }
            _ => {}
        }
        i += 1;
    }
    let app_data = app_data.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".wisp"))
            .join("wisp-science")
    });
    let project_root = project_root.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let resource_root = resource_root.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(PathBuf::from))
    });
    BridgeConfig {
        app_data,
        project_root,
        resource_root,
        project_id,
        frame_id,
    }
}

pub fn run_mcp_bridge_cli() {
    let cfg = parse_mcp_bridge_cli_args();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create Wisp MCP bridge runtime");
    if let Err(e) = rt.block_on(run_stdio(cfg)) {
        eprintln!("Wisp MCP bridge error: {e:?}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_custom_tool_parts() {
        assert_eq!(sanitize_tool_part("abc.Def/g"), "abc_Def_g");
        assert_eq!(sanitize_tool_part(""), "tool");
    }

    #[test]
    fn run_control_plane_schemas_match_current_contract() {
        assert_eq!(
            get_capabilities_tool_schema()["name"],
            "wisp_get_capabilities"
        );
        assert_eq!(search_memory_tool_schema()["name"], "wisp_search_memory");
        assert_eq!(list_artifacts_tool_schema()["name"], "wisp_list_artifacts");
        assert_eq!(
            get_research_graph_tool_schema()["name"],
            "wisp_get_research_graph"
        );
        assert_eq!(
            list_execution_contexts_tool_schema()["name"],
            "wisp_list_execution_contexts"
        );
        let run = run_in_context_tool_schema();
        assert_eq!(run["name"], "wisp_run_in_context");
        assert!(run["description"]
            .as_str()
            .unwrap()
            .contains("wait_for_completion"));
        let properties = &run["inputSchema"]["properties"];
        assert!(properties["timeout_secs"]["description"]
            .as_str()
            .unwrap()
            .contains("7d"));
        assert_eq!(properties["input_paths"]["items"]["type"], "string");
        assert_eq!(properties["wait_for_completion"]["type"], "boolean");
        assert!(properties["output_specs"]["description"]
            .as_str()
            .unwrap()
            .contains("ssh://"));

        assert_eq!(get_run_tool_schema()["name"], "wisp_get_run");
        assert_eq!(monitor_run_tool_schema()["name"], "wisp_monitor_run");
        assert_eq!(cancel_run_tool_schema()["name"], "wisp_cancel_run");
        let delegation = propose_delegation_tool_schema();
        assert_eq!(delegation["name"], "wisp_propose_delegation");
        assert_eq!(delegation["inputSchema"]["required"], json!(["goal"]));
    }

    #[test]
    fn run_control_plane_names_are_reserved() {
        for name in [
            "wisp_get_capabilities",
            "wisp_list_skills",
            "wisp_use_skill",
            "wisp_search_memory",
            "wisp_list_artifacts",
            "wisp_get_research_graph",
            "wisp_list_execution_contexts",
            "wisp_run_in_context",
            "wisp_get_run",
            "wisp_monitor_run",
            "wisp_cancel_run",
            "wisp_propose_delegation",
        ] {
            assert!(is_builtin_tool(name), "{name} must be reserved");
        }
        assert!(!is_builtin_tool("third_party_run"));
    }

    #[tokio::test]
    async fn delegation_tool_is_listed_only_when_the_session_opted_in() {
        let base =
            std::env::temp_dir().join(format!("wisp_mcp_delegation_{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project_root).unwrap();
        let cfg = BridgeConfig {
            app_data,
            project_root: project_root.clone(),
            resource_root: None,
            project_id: "project-a".into(),
            frame_id: Some("frame-a".into()),
        };
        let mut server = BridgeServer::new(cfg).await.unwrap();
        server.bundled_bio_tools_loaded = true;
        server.custom_mcp_tools_loaded = true;
        server
            .store
            .create_project("project-a", "A", &project_root.to_string_lossy())
            .await
            .unwrap();
        server
            .store
            .create_frame("frame-a", "project-a", "Agent", "model")
            .await
            .unwrap();

        let disabled = server.tools_list().await.unwrap();
        assert!(!disabled.to_string().contains("wisp_propose_delegation"));
        crate::delegation_runtime::save_session_delegation_enabled(
            &server.store,
            "project-a",
            "frame-a",
            true,
        )
        .await
        .unwrap();
        let enabled = server.tools_list().await.unwrap();
        assert!(enabled.to_string().contains("wisp_propose_delegation"));
        let capabilities: Value =
            serde_json::from_str(&server.capabilities_text().await.unwrap()).unwrap();
        let delegation = capabilities["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "delegation.propose")
            .unwrap();
        assert_eq!(delegation["allowed"], true);

        let result = server
            .tools_call(json!({
                "name": "wisp_propose_delegation",
                "arguments": {
                    "goal": "analyze code and create a visualization",
                    "mode": "manual",
                    "agents": ["code_execution", "reviewer"]
                }
            }))
            .await
            .unwrap();
        assert_eq!(result["isError"], false);
        let workflows = server
            .store
            .list_agent_workflows("project-a")
            .await
            .unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].status, wisp_store::AgentWorkflowStatus::Draft);

        drop(server);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn read_gateway_is_project_scoped_and_redacts_context_config() {
        let base =
            std::env::temp_dir().join(format!("wisp_mcp_read_gateway_{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project_root).unwrap();
        let cfg = BridgeConfig {
            app_data,
            project_root: project_root.clone(),
            resource_root: None,
            project_id: "project-a".into(),
            frame_id: Some("frame-a".into()),
        };
        let mut server = BridgeServer::new(cfg).await.unwrap();
        server
            .store
            .create_project("project-a", "A", &project_root.display().to_string())
            .await
            .unwrap();
        server
            .store
            .create_project("project-b", "B", &project_root.display().to_string())
            .await
            .unwrap();
        server
            .store
            .create_frame("frame-a", "project-a", "Agent", "model")
            .await
            .unwrap();
        server
            .store
            .create_frame("frame-b", "project-b", "Agent", "model")
            .await
            .unwrap();
        server
            .store
            .save_artifact(
                "artifact-a",
                "project-a",
                "frame-a",
                "visible.csv",
                "text/csv",
                &project_root.join("visible.csv").display().to_string(),
            )
            .await
            .unwrap();
        server
            .store
            .save_artifact(
                "artifact-b",
                "project-b",
                "frame-b",
                "hidden.csv",
                "text/csv",
                &project_root.join("hidden.csv").display().to_string(),
            )
            .await
            .unwrap();

        let memory_dir = project_root.join(".wisp").join("memory");
        std::fs::write(
            memory_dir.join("2026-07-15.md"),
            "The validated cohort contains forty-two samples.",
        )
        .unwrap();
        let memory: Value = serde_json::from_str(
            &server
                .search_memory_text(&json!({ "query": "cohort", "top_k": 99 }))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(memory["topK"], 10);
        assert!(memory["results"].as_str().unwrap().contains("forty-two"));

        let artifacts: Value = serde_json::from_str(
            &server
                .list_artifacts_text(&json!({ "limit": 100 }))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(artifacts["projectId"], "project-a");
        let rendered = artifacts.to_string();
        assert!(rendered.contains("visible.csv"));
        assert!(!rendered.contains("hidden.csv"));

        let graph: Value =
            serde_json::from_str(&server.research_graph_text().await.unwrap()).unwrap();
        assert_eq!(graph["projectId"], "project-a");
        assert_eq!(graph["graph"]["nodes"].as_array().unwrap().len(), 1);

        let mut ssh = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        ssh.config_json = r#"{"token":"must-not-leak"}"#.into();
        server.store.upsert_execution_context(&ssh).await.unwrap();
        let contexts = server.execution_contexts_text().await.unwrap();
        assert!(contexts.contains("ssh:gpu"));
        assert!(!contexts.contains("must-not-leak"));

        let capabilities: Value =
            serde_json::from_str(&server.capabilities_text().await.unwrap()).unwrap();
        assert_eq!(capabilities["scope"], "active_project");
        let harness_write = capabilities["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "harness.write")
            .unwrap();
        assert_eq!(harness_write["allowed"], false);
        let routed = server
            .tools_call(json!({
                "name": "wisp_get_capabilities",
                "arguments": {}
            }))
            .await
            .unwrap();
        assert_eq!(routed["isError"], false);
        assert!(routed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("active_project"));

        drop(server);
        let _ = std::fs::remove_dir_all(base);
    }
}
