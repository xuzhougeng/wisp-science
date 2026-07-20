//! Stdio MCP bridge exposed to external ACP agents.
//!
//! The bridge intentionally exposes Wisp's scientific capabilities (skills,
//! bundled bio MCP, custom MCP, run contexts) without forwarding Wisp's generic
//! shell/edit/read tools. Local runners already have their own filesystem tools;
//! this process is only for Wisp-native capabilities and policy/config reuse.

use crate::{
    bio_domains, connect_mcp, load_disabled_connectors, load_disabled_skills,
    load_enabled_skill_names, load_mcp_connections, run_context, skill_paths, ActiveProject,
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

const DEFAULT_TOOL_SEARCH_LIMIT: usize = 5;
const MAX_TOOL_SEARCH_LIMIT: usize = 10;
const MAX_TOOL_DESCRIPTION_CHARS: usize = 2_048;

#[derive(Debug, Clone)]
pub(crate) struct BridgeConfig {
    pub(crate) app_data: PathBuf,
    pub(crate) project_root: PathBuf,
    pub(crate) resource_root: Option<PathBuf>,
    pub(crate) project_id: String,
    pub(crate) frame_id: Option<String>,
    pub(crate) allowed_tools: Option<HashSet<String>>,
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
        connector_id: String,
        client: Arc<wisp_mcp::McpClient>,
        remote_name: String,
        description: String,
        input_schema: Value,
    },
    Custom {
        connector_id: String,
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
    runtime_manager: wisp_runtime::RuntimeManager,
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
        let runtime_manager = wisp_runtime::RuntimeManager::new(Arc::new(
            crate::runtime_launcher::TauriRuntimeLauncher::new(
                store.clone(),
                cfg.app_data.clone(),
                crate::kernel_worker_path(),
                crate::r_kernel_worker_path(),
                vec![],
            ),
        ));
        let raw = SkillIndex::load(&skill_paths(&cfg.project_root));
        let project_skills = filter_skills(&store, &cfg.project_id, raw).await;
        let skills = match &cfg.allowed_tools {
            Some(allowed) => {
                let names = allowed
                    .iter()
                    .filter_map(|token| crate::delegation_resources::skill_from_token(token))
                    .map(str::to_string)
                    .collect::<HashSet<_>>();
                Arc::new(project_skills.filtered_by_names(Some(&names)))
            }
            None => Arc::new(project_skills),
        };
        let memory = wisp_core::MemoryManager::new(&cfg.project_root);
        Ok(Self {
            cfg,
            store,
            memory,
            run_manager,
            runtime_manager,
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
            search_tools_tool_schema(),
            use_tool_tool_schema(),
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
            let session_enabled =
                crate::delegation_runtime::session_delegation_enabled(&self.store, frame_id).await;
            let nested_enabled =
                crate::delegation_tool::nested_delegation_available(&self.store, frame_id).await;
            let nested_result_access =
                crate::delegation_tool::nested_result_access_available(&self.store, frame_id).await;
            if session_enabled || nested_enabled {
                tools.push(
                    delegate_tasks_tool_schema(
                        &self.store,
                        &self.active_project(),
                        frame_id,
                        &self.cfg.app_data,
                    )
                    .await?,
                );
            }
            if session_enabled || nested_result_access {
                tools.push(get_delegated_result_tool_schema());
            }
        }
        if !self.allowed_connectors().is_empty() {
            self.ensure_remote_tools().await?;
            tools.extend(self.route_tools());
        }
        if self.cfg.allowed_tools.is_some() {
            tools.retain(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| self.tool_authorized(name))
            });
        }
        Ok(json!({ "tools": tools }))
    }

    fn active_project(&self) -> ActiveProject {
        ActiveProject {
            id: self.cfg.project_id.clone(),
            root: self.cfg.project_root.clone(),
            skills: self.skills.clone(),
            memory: Arc::new(wisp_core::MemoryManager::new(&self.cfg.project_root)),
        }
    }

    fn allowed_connectors(&self) -> HashSet<String> {
        self.cfg
            .allowed_tools
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|token| crate::delegation_resources::connector_from_token(token))
            .map(str::to_string)
            .collect()
    }

    fn has_skill_grant(&self) -> bool {
        self.cfg.allowed_tools.as_ref().is_some_and(|allowed| {
            allowed
                .iter()
                .any(|token| crate::delegation_resources::skill_from_token(token).is_some())
        })
    }

    fn tool_authorized(&self, name: &str) -> bool {
        self.cfg.allowed_tools.as_ref().is_none_or(|allowed| {
            allowed.contains(name)
                || (matches!(name, "wisp_list_skills" | "wisp_use_skill") && self.has_skill_grant())
                || self.routes.get(name).is_some_and(|route| {
                    let connector_id = match route {
                        Route::Bio { connector_id, .. } | Route::Custom { connector_id, .. } => {
                            connector_id
                        }
                    };
                    self.allowed_connectors().contains(connector_id)
                })
        })
    }

    async fn tools_call(&mut self, params: Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("tools/call missing name"))?;
        if !is_builtin_tool(name) && !self.allowed_connectors().is_empty() {
            self.ensure_remote_tools().await?;
        }
        if !self.tool_authorized(name) {
            return Err(anyhow!(
                "MCP tool '{name}' is outside this Agent's capability grant"
            ));
        }
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
            "wisp_search_tools" => match self.search_tools_text(&args).await {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_use_tool" => {
                let Some(tool_name) = args.get("tool_name").and_then(Value::as_str) else {
                    return Ok(tool_call_result(
                        "missing required argument 'tool_name'",
                        true,
                    ));
                };
                let Some(tool_input) = args.get("tool_input").filter(|value| value.is_object())
                else {
                    return Ok(tool_call_result("'tool_input' must be a JSON object", true));
                };
                match self.call_remote_tool(tool_name, tool_input).await {
                    Ok(result) => result,
                    Err(error) => (error.to_string(), true),
                }
            }
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
            "wisp_delegate_tasks" => {
                let Some(frame_id) = self.cfg.frame_id.as_deref() else {
                    return Err(anyhow!("delegation requires a conversation frame"));
                };
                let project = self.active_project();
                let tool = crate::delegation_tool::DelegateTasksTool::new(
                    self.store.clone(),
                    project,
                    frame_id,
                    self.run_manager.clone(),
                    self.runtime_manager.clone(),
                    self.cfg.app_data.clone(),
                )
                .await
                .map_err(anyhow::Error::msg)?;
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
            "wisp_get_delegated_result" => {
                let Some(frame_id) = self.cfg.frame_id.as_deref() else {
                    return Err(anyhow!("delegation requires a conversation frame"));
                };
                let tool = crate::delegation_tool::GetDelegatedResultTool::new(
                    self.store.clone(),
                    self.cfg.project_id.clone(),
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
            other => self.call_remote_tool(other, &args).await?,
        };
        Ok(tool_call_result(text, is_error))
    }

    async fn ensure_remote_tools(&mut self) -> Result<()> {
        if !self.bundled_bio_tools_loaded {
            self.bundled_bio_tools_loaded = true;
            self.register_bundled_bio_tools().await;
        }
        if !self.custom_mcp_tools_loaded {
            self.custom_mcp_tools_loaded = true;
            self.register_custom_mcp_tools().await;
        }
        Ok(())
    }

    async fn register_bundled_bio_tools(&mut self) {
        if let Ok(command) = std::env::var("WISP_MCP_COMMAND") {
            let allowed = self.allowed_connectors();
            if self.cfg.allowed_tools.is_some() && !allowed.contains("dev-mcp") {
                return;
            }
            let parts = command.split_whitespace().collect::<Vec<_>>();
            let Some((program, args)) = parts.split_first() else {
                return;
            };
            let args = args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            let Ok(client) = wisp_mcp::McpClient::launch(program, &args).await else {
                return;
            };
            let client = Arc::new(client);
            let Ok(tools) = client.tools_list().await else {
                return;
            };
            for tool in tools {
                if tool.name.is_empty() {
                    continue;
                }
                let exposed = format!("wisp_custom_dev_mcp__{}", sanitize_tool_part(&tool.name));
                if self.is_reserved(&exposed) {
                    continue;
                }
                self.routes.insert(
                    exposed,
                    Route::Custom {
                        connector_id: "dev-mcp".into(),
                        client: client.clone(),
                        remote_name: tool.name,
                        description: tool.description,
                        input_schema: tool.input_schema,
                    },
                );
            }
            return;
        }
        let disabled = load_disabled_connectors(&self.store).await;
        let domains = bio_domains();
        let allowed = self.allowed_connectors();
        let blocked = |slug: &str| {
            disabled.contains(slug) || (self.cfg.allowed_tools.is_some() && !allowed.contains(slug))
        };
        let all_off = if self.cfg.allowed_tools.is_some() {
            domains.is_empty() || domains.iter().all(|domain| blocked(&domain.slug))
        } else {
            !domains.is_empty() && domains.iter().all(|domain| blocked(&domain.slug))
        };
        if all_off {
            return;
        }
        let skip: HashSet<String> = domains
            .iter()
            .filter(|d| blocked(&d.slug))
            .flat_map(|d| d.tools.iter().cloned())
            .collect();
        let tool_connectors = domains
            .iter()
            .flat_map(|domain| {
                domain
                    .tools
                    .iter()
                    .map(|tool| (tool.clone(), domain.slug.clone()))
            })
            .collect::<HashMap<_, _>>();
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
                    connector_id: tool_connectors.get(&tool.name).cloned().unwrap_or_default(),
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
            .filter(|connection| {
                let allowed = self.allowed_connectors();
                self.cfg.allowed_tools.is_none() || allowed.contains(&connection.id)
            })
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
                        connector_id: conn.id.clone(),
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

    async fn search_tools_text(&mut self, args: &Value) -> Result<String> {
        self.ensure_remote_tools().await?;
        search_tool_catalog(self.route_tools(), args)
    }

    async fn call_remote_tool(&mut self, name: &str, args: &Value) -> Result<(String, bool)> {
        self.ensure_remote_tools().await?;
        let route = self
            .routes
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown Wisp bridge tool '{name}'"))?;
        let (client, remote_name) = match route {
            Route::Bio {
                client,
                remote_name,
                ..
            }
            | Route::Custom {
                client,
                remote_name,
                ..
            } => (client, remote_name),
        };
        Ok(match client.tool_call(&remote_name, args).await {
            Ok(text) => (text, false),
            Err(error) => (error.to_string(), true),
        })
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
        let (delegation_enabled, result_access) = match self.cfg.frame_id.as_deref() {
            Some(frame_id) => {
                let session =
                    crate::delegation_runtime::session_delegation_enabled(&self.store, frame_id)
                        .await;
                let nested =
                    crate::delegation_tool::nested_delegation_available(&self.store, frame_id)
                        .await;
                let nested_result =
                    crate::delegation_tool::nested_result_access_available(&self.store, frame_id)
                        .await;
                (session || nested, session || nested_result)
            }
            None => (false, false),
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
                {
                    "name": "scientific_mcp",
                    "allowed": true,
                    "tools": ["wisp_search_tools", "wisp_use_tool"],
                    "discovery": "wisp_search_tools"
                },
                {
                    "name": "harness.write",
                    "allowed": false,
                    "reason": "Memory, artifact, graph, and persistent runtime writes require an approval broker and are not exposed by this bridge."
                },
                {
                    "name": "delegation.inline",
                    "allowed": delegation_enabled || result_access,
                    "tools": match (delegation_enabled, result_access) {
                        (true, true) => vec!["wisp_delegate_tasks", "wisp_get_delegated_result"],
                        (true, false) => vec!["wisp_delegate_tasks"],
                        (false, true) => vec!["wisp_get_delegated_result"],
                        (false, false) => vec![],
                    },
                    "policy": "bounded Native child Agents; read-only batches run inline; any requested approval is denied by this non-interactive bridge"
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
        let mut contexts = self.store.list_execution_contexts().await?;
        if self.cfg.allowed_tools.is_some() {
            let selected = match self.cfg.frame_id.as_deref() {
                Some(frame_id) => self
                    .store
                    .list_session_execution_context_ids(frame_id)
                    .await?
                    .into_iter()
                    .collect::<HashSet<_>>(),
                None => HashSet::new(),
            };
            contexts.retain(|context| {
                context.kind == wisp_store::ExecutionContextKind::Local
                    || selected.contains(&context.id)
            });
        }
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

fn tool_call_result(text: impl Into<String>, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text.into() }],
        "isError": is_error
    })
}

fn search_tool_catalog(tools: Vec<Value>, args: &Value) -> Result<String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| anyhow!("missing required argument 'query'"))?
        .to_lowercase();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .unwrap_or(DEFAULT_TOOL_SEARCH_LIMIT)
        .clamp(1, MAX_TOOL_SEARCH_LIMIT);
    let browse = query == "*";
    let terms: Vec<_> = query.split_whitespace().collect();
    let total_hidden_tools = tools.len();
    let mut matches = vec![];
    for mut tool in tools {
        let name = tool["name"].as_str().unwrap_or_default().to_lowercase();
        let description = tool["description"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let parameters = tool["inputSchema"].to_string().to_lowercase();
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
            tool["description"] = Value::String(truncate_catalog_description(
                tool["description"].as_str().unwrap_or_default(),
            ));
            matches.push((score, name, tool));
        }
    }
    matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let matched_tools = matches.len();
    let results: Vec<_> = matches
        .into_iter()
        .take(limit)
        .map(|(_, _, tool)| {
            json!({
                "tool_name": tool["name"],
                "description": tool["description"],
                "input_schema": tool["inputSchema"],
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json!({
        "results": results,
        "matched_tools": matched_tools,
        "total_hidden_tools": total_hidden_tools,
        "next": "Call 'wisp_use_tool' with a returned tool_name and matching tool_input. Use query '*' to browse.",
    }))?)
}

fn truncate_catalog_description(value: &str) -> String {
    if value.chars().count() <= MAX_TOOL_DESCRIPTION_CHARS {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(MAX_TOOL_DESCRIPTION_CHARS).collect();
    truncated.push_str("… [truncated]");
    truncated
}

fn search_tools_tool_schema() -> Value {
    json!({
        "name": "wisp_search_tools",
        "description": "Search enabled Wisp scientific and custom MCP tools. Returns only matching input schemas instead of exposing the full catalog on every request.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Capability, connector, action, known tool name, or '*' to browse" },
                "limit": { "type": "integer", "description": "Maximum matches to return (default 5, maximum 10)" }
            },
            "required": ["query"],
            "additionalProperties": false
        }
    })
}

fn use_tool_tool_schema() -> Value {
    json!({
        "name": "wisp_use_tool",
        "description": "Call a Wisp MCP tool found by wisp_search_tools. tool_input must match the returned input_schema.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string", "description": "Exact tool_name returned by wisp_search_tools" },
                "tool_input": { "type": "object", "description": "Arguments matching the selected tool's input_schema", "additionalProperties": true }
            },
            "required": ["tool_name", "tool_input"],
            "additionalProperties": false
        }
    })
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

async fn delegate_tasks_tool_schema(
    store: &Store,
    project: &ActiveProject,
    frame_id: &str,
    app_data: &Path,
) -> Result<Value> {
    let schema = crate::delegation_tool::delegate_tasks_schema(store, project, frame_id, app_data)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(json!({
        "name": "wisp_delegate_tasks",
        "description": schema.function.description,
        "inputSchema": schema.function.parameters,
    }))
}

fn get_delegated_result_tool_schema() -> Value {
    let schema = crate::delegation_tool::get_delegated_result_schema();
    json!({
        "name": "wisp_get_delegated_result",
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
            | "wisp_search_tools"
            | "wisp_use_tool"
            | "wisp_search_memory"
            | "wisp_list_artifacts"
            | "wisp_get_research_graph"
            | "wisp_list_execution_contexts"
            | "wisp_run_in_context"
            | "wisp_get_run"
            | "wisp_monitor_run"
            | "wisp_cancel_run"
            | "wisp_delegate_tasks"
            | "wisp_get_delegated_result"
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
    let mut allowed_tools = HashSet::new();
    let mut tool_filter_present = false;
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
            "--allow-tool" => {
                tool_filter_present = true;
                i += 1;
                if let Some(value) = args.get(i).filter(|value| !value.trim().is_empty()) {
                    allowed_tools.insert(value.clone());
                }
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
        allowed_tools: tool_filter_present.then_some(allowed_tools),
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
        assert_eq!(search_tools_tool_schema()["name"], "wisp_search_tools");
        assert_eq!(use_tool_tool_schema()["name"], "wisp_use_tool");
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
        let delegated_result = get_delegated_result_tool_schema();
        assert_eq!(delegated_result["name"], "wisp_get_delegated_result");
        assert_eq!(
            delegated_result["inputSchema"]["required"],
            json!(["workflow_id", "task_id"])
        );
    }

    #[test]
    fn run_control_plane_names_are_reserved() {
        for name in [
            "wisp_get_capabilities",
            "wisp_list_skills",
            "wisp_use_skill",
            "wisp_search_tools",
            "wisp_use_tool",
            "wisp_search_memory",
            "wisp_list_artifacts",
            "wisp_get_research_graph",
            "wisp_list_execution_contexts",
            "wisp_run_in_context",
            "wisp_get_run",
            "wisp_monitor_run",
            "wisp_cancel_run",
            "wisp_delegate_tasks",
            "wisp_get_delegated_result",
        ] {
            assert!(is_builtin_tool(name), "{name} must be reserved");
        }
        assert!(!is_builtin_tool("third_party_run"));
    }

    #[test]
    fn bridge_mcp_catalog_is_searched_on_demand() {
        let catalog = vec![
            json!({
                "name": "pubmed_search",
                "description": "Search biomedical literature.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } }
                }
            }),
            json!({
                "name": "notion_create_page",
                "description": "Create a Notion page.",
                "inputSchema": { "type": "object", "properties": {} }
            }),
        ];

        let result = search_tool_catalog(catalog, &json!({ "query": "biomedical" })).unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["total_hidden_tools"], 2);
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["results"][0]["tool_name"], "pubmed_search");
        assert_eq!(
            result["results"][0]["input_schema"]["properties"]["query"]["type"],
            "string"
        );
    }

    #[tokio::test]
    async fn delegated_bridge_lists_and_calls_only_granted_tools() {
        let base = std::env::temp_dir().join(format!("wisp_mcp_filtered_{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let mut server = BridgeServer::new(BridgeConfig {
            app_data: base.join("app-data"),
            project_root,
            resource_root: None,
            project_id: "project-a".into(),
            frame_id: None,
            allowed_tools: Some(HashSet::from([
                "wisp_list_execution_contexts".into(),
                "wisp_get_run".into(),
            ])),
        })
        .await
        .unwrap();
        server
            .store
            .upsert_execution_context(
                &wisp_store::ExecutionContext::new("ssh:not-selected", "Private host").unwrap(),
            )
            .await
            .unwrap();

        let listed = server.tools_list().await.unwrap();
        let names = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            names,
            HashSet::from(["wisp_list_execution_contexts", "wisp_get_run"])
        );
        let contexts = server.execution_contexts_text().await.unwrap();
        assert!(contexts.contains("local"));
        assert!(!contexts.contains("ssh:not-selected"));
        assert!(server
            .tools_call(json!({"name":"wisp_search_memory","arguments":{}}))
            .await
            .unwrap_err()
            .to_string()
            .contains("outside this Agent's capability grant"));

        drop(server);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn delegated_bridge_exposes_only_explicitly_granted_skills() {
        let base =
            std::env::temp_dir().join(format!("wisp_mcp_skill_filter_{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project");
        for (name, body) in [
            ("papers", "paper guidance"),
            ("private", "private guidance"),
        ] {
            let dir = project_root.join(".wisp/skills").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: test\n---\n{body}"),
            )
            .unwrap();
        }
        let mut server = BridgeServer::new(BridgeConfig {
            app_data: base.join("app-data"),
            project_root,
            resource_root: None,
            project_id: "project-a".into(),
            frame_id: None,
            allowed_tools: Some(HashSet::from([crate::delegation_resources::skill_token(
                "papers",
            )])),
        })
        .await
        .unwrap();

        let listed = server.tools_list().await.unwrap();
        let names = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<HashSet<_>>();
        assert_eq!(names, HashSet::from(["wisp_list_skills", "wisp_use_skill"]));
        let skills = server
            .tools_call(json!({"name":"wisp_list_skills","arguments":{}}))
            .await
            .unwrap()
            .to_string();
        assert!(skills.contains("papers"));
        assert!(!skills.contains("private"));
        let denied = server
            .tools_call(json!({
                "name":"wisp_use_skill",
                "arguments":{"name":"private"}
            }))
            .await
            .unwrap();
        assert_eq!(denied["isError"], true);

        drop(server);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn dynamic_delegation_tools_are_listed_only_when_the_session_opted_in() {
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
            allowed_tools: None,
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
        assert!(!disabled.to_string().contains("wisp_delegate_tasks"));
        assert!(!disabled.to_string().contains("wisp_get_delegated_result"));
        crate::delegation_runtime::save_session_delegation_enabled(
            &server.store,
            "project-a",
            "frame-a",
            true,
        )
        .await
        .unwrap();
        let enabled = server.tools_list().await.unwrap();
        assert!(enabled.to_string().contains("wisp_delegate_tasks"));
        assert!(enabled.to_string().contains("wisp_get_delegated_result"));
        let inline_schema = enabled["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "wisp_delegate_tasks")
            .unwrap();
        assert_eq!(
            inline_schema["inputSchema"]["required"],
            json!(["goal", "tasks"])
        );
        let capabilities: Value =
            serde_json::from_str(&server.capabilities_text().await.unwrap()).unwrap();
        let inline = capabilities["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "delegation.inline")
            .unwrap();
        assert_eq!(inline["allowed"], true);
        assert!(inline["tools"]
            .as_array()
            .unwrap()
            .contains(&json!("wisp_delegate_tasks")));

        let malformed_inline = server
            .tools_call(json!({
                "name": "wisp_delegate_tasks",
                "arguments": {}
            }))
            .await
            .unwrap();
        assert_eq!(malformed_inline["isError"], true);
        assert!(malformed_inline.to_string().contains("invalid task batch"));
        assert!(server
            .store
            .list_agent_workflows("project-a")
            .await
            .unwrap()
            .is_empty());

        drop(server);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn delegated_bridge_exposes_bounded_nested_tools_without_session_toggle() {
        let base = std::env::temp_dir().join(format!("wisp_mcp_nested_{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project_root).unwrap();
        let mut server = BridgeServer::new(BridgeConfig {
            app_data: app_data.clone(),
            project_root: project_root.clone(),
            resource_root: None,
            project_id: "project-a".into(),
            frame_id: Some("child-frame".into()),
            allowed_tools: Some(HashSet::from([
                "wisp_delegate_tasks".into(),
                "wisp_get_delegated_result".into(),
            ])),
        })
        .await
        .unwrap();
        server.bundled_bio_tools_loaded = true;
        server.custom_mcp_tools_loaded = true;
        server
            .store
            .create_project("project-a", "A", &project_root.to_string_lossy())
            .await
            .unwrap();
        for frame_id in ["root-frame", "child-frame"] {
            server
                .store
                .create_frame(frame_id, "project-a", "Agent", "model")
                .await
                .unwrap();
        }
        server
            .store
            .set_setting(
                "acp_agent_profiles",
                &json!([{
                    "id": "nested-test",
                    "label": "Nested test ACP",
                    "command": std::env::current_exe().unwrap().to_string_lossy(),
                    "args": []
                }])
                .to_string(),
            )
            .await
            .unwrap();
        let policy = crate::delegation_runtime::dynamic_delegation_policy_for_project(
            &server.store,
            &server.active_project(),
            Some("child-frame"),
            &app_data,
        )
        .await
        .unwrap();
        assert!(policy.host.executors.iter().any(|executor| {
            executor.enabled
                && executor.executor
                    == wisp_core::AgentExecutorRef::Acp {
                        profile_id: "nested-test".into(),
                    }
        }));
        let parent_spec: wisp_core::AgentSpec = serde_json::from_value(json!({
            "agent_id": "parent",
            "name": "Parent",
            "goal": "Delegate one bounded batch",
            "role": "temporary",
            "backend": "acp",
            "prompt_template": "Use only approved delegation.",
            "permissions": {
                "tools": ["delegate_tasks", "get_delegated_result"]
            },
            "budget": {
                "max_tokens": 8000,
                "max_tool_calls": 16,
                "max_cost_microunits": 100000
            },
            "allow_delegation": true,
            "origin": {"kind": "temporary"},
            "capabilities": ["reasoning", "delegation"],
            "executor": {"kind": "acp", "profile_id": "nested-test"}
        }))
        .unwrap();
        let limits = wisp_store::AgentDelegationRootLimits {
            max_depth: 2,
            max_tasks: 2,
            max_parallel: 2,
            max_tokens: 32_000,
            max_tool_calls: 64,
            max_cost_microunits: 1_000_000,
            wall_time_secs: 300,
        };
        let mut root = wisp_store::AgentWorkflow::new(
            "root-workflow",
            "project-a",
            project_root.to_string_lossy().into_owned(),
            "Root",
        )
        .unwrap();
        root.frame_id = Some("root-frame".into());
        root.root_limits_json = serde_json::to_string(&limits).unwrap();
        let mut root_step = wisp_store::AgentWorkflowStep::new(
            "root-step",
            &root.id,
            0,
            "parent",
            "temporary",
            "acp",
            "Use only approved delegation.",
        )
        .unwrap();
        root_step.spec_json = serde_json::to_string(&parent_spec).unwrap();
        root_step.budget_json = serde_json::to_string(&parent_spec.budget).unwrap();
        server
            .store
            .create_agent_workflow_plan(&root, &[root_step])
            .await
            .unwrap();
        assert!(server
            .store
            .approve_agent_workflow_plan(&root.id, root.version)
            .await
            .unwrap());
        assert!(server
            .store
            .transition_agent_workflow_status(
                &root.id,
                wisp_store::AgentWorkflowStatus::Approved,
                wisp_store::AgentWorkflowStatus::Running,
            )
            .await
            .unwrap());
        let mut parent_attempt = wisp_store::AgentWorkflowAttempt::queued(
            "parent-attempt",
            &root.id,
            "root-step",
            1,
            "parent-request",
            "acp",
            "{}",
        )
        .unwrap();
        parent_attempt.allow_delegation = true;
        let wisp_store::AgentWorkflowAttemptStart::Started(parent_attempt) = server
            .store
            .try_create_started_agent_workflow_attempt(parent_attempt)
            .await
            .unwrap()
        else {
            panic!("parent attempt should start");
        };
        assert!(server
            .store
            .set_running_agent_workflow_attempt_provenance("parent-request", None, "child-frame",)
            .await
            .unwrap());

        let listed = server.tools_list().await.unwrap().to_string();
        assert!(listed.contains("wisp_delegate_tasks"));
        assert!(listed.contains("wisp_get_delegated_result"));

        let mut nested = wisp_store::AgentWorkflow::new(
            "nested-workflow",
            "project-a",
            project_root.to_string_lossy().into_owned(),
            "Nested",
        )
        .unwrap();
        nested.frame_id = Some("child-frame".into());
        nested.root_workflow_id = root.id.clone();
        nested.parent_attempt_id = Some(parent_attempt.id);
        nested.depth = 1;
        nested.root_limits_json = serde_json::to_string(&limits).unwrap();
        nested.max_parallel = 1;
        let mut leaf = wisp_store::AgentWorkflowStep::new(
            "leaf-step",
            &nested.id,
            0,
            "leaf",
            "temporary",
            "local",
            "Return evidence.",
        )
        .unwrap();
        leaf.spec_json = json!({"allow_delegation": false}).to_string();
        leaf.budget_json = json!({
            "max_tokens": 1000,
            "max_tool_calls": 1,
            "max_cost_microunits": 1
        })
        .to_string();
        server
            .store
            .create_agent_workflow_plan(&nested, &[leaf])
            .await
            .unwrap();
        let exhausted = server.tools_list().await.unwrap().to_string();
        assert!(!exhausted.contains("wisp_delegate_tasks"));
        assert!(exhausted.contains("wisp_get_delegated_result"));

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
            allowed_tools: None,
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
