//! Stdio MCP bridge exposed to local runners (Codex CLI / Claude Code).
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
    skills: Arc<SkillIndex>,
    routes: HashMap<String, Route>,
    remote_tools_loaded: bool,
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
        let raw = SkillIndex::load(&skill_paths(&cfg.project_root));
        let skills = Arc::new(filter_skills(&store, &cfg.project_id, raw).await);
        Ok(Self {
            cfg,
            store,
            skills,
            routes: HashMap::new(),
            remote_tools_loaded: false,
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
            run_in_context_tool_schema(),
        ];
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
            "wisp_list_skills" => (self.list_skills_text(), false),
            "wisp_use_skill" => match self.use_skill_text(&args) {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
            "wisp_run_in_context" => {
                let result = self.run_in_context(&args).await;
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
        if self.remote_tools_loaded {
            return Ok(self.route_tools());
        }
        self.remote_tools_loaded = true;

        self.register_bundled_bio_tools().await;
        self.register_custom_mcp_tools().await;
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
        let Ok(env) = wisp_python::PythonEnv::ensure(&self.cfg.app_data) else {
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
        matches!(
            name,
            "wisp_list_skills" | "wisp_use_skill" | "wisp_run_in_context"
        ) || self.routes.contains_key(name)
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

    async fn run_in_context(&self, args: &Value) -> ToolResult {
        let tool = run_context::RunInContextTool::new(
            self.store.clone(),
            self.cfg.project_id.clone(),
            self.cfg.frame_id.clone(),
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

fn run_in_context_tool_schema() -> Value {
    json!({
        "name": "wisp_run_in_context",
        "description": "Create a persisted Wisp Run and execute a bounded command in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). Dangerous commands require approval and are rejected in this non-interactive bridge.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                "command": { "type": "string", "description": "Command to execute in that context" },
                "title": { "type": "string", "description": "Short run title" },
                "timeout_secs": { "type": "integer", "description": "Bounded timeout in seconds, clamped to 1..300" },
                "output_specs": {
                    "type": "array",
                    "description": "Optional harvest specs for files produced by the run",
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

pub(crate) async fn run_oneshot(
    cfg: BridgeConfig,
    tool_name: String,
    arguments: Value,
) -> Result<String> {
    let mut server = BridgeServer::new(cfg).await?;
    let result = if tool_name == "tools/list" || tool_name == "wisp_tools_list" {
        server.tools_list().await?
    } else {
        server
            .tools_call(json!({ "name": tool_name, "arguments": arguments }))
            .await?
    };
    Ok(extract_text_result(&result))
}

fn extract_text_result(result: &Value) -> String {
    let Some(content) = result.get("content").and_then(|v| v.as_array()) else {
        return serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    };
    content
        .iter()
        .filter_map(|block| {
            block
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_custom_tool_parts() {
        assert_eq!(sanitize_tool_part("abc.Def/g"), "abc_Def_g");
        assert_eq!(sanitize_tool_part(""), "tool");
    }
}
