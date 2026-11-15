//! Wisp's science tools for an ACP agent: skills, the literature and
//! biology connectors, the user's MCP servers, memory and runs, offered
//! over MCP so the agent can call them next to its own file and shell
//! tools, which it already has.
//!
//! The WebView desktop starts a second copy of itself as a stdio MCP
//! server (`src-tauri/src/mcp_bridge.rs`). Here the host serves the tools
//! itself on a loopback address, one unguessable path per conversation:
//! an agent that speaks MCP over HTTP is given the address, and one that
//! only starts programs is given a relay (the desktop's own executable in
//! relay mode) that carries stdio to that address.

use anyhow::{Context, Result};
use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use wisp_tools::{Tool, ToolEnv};

/// What the agent is told the server is called.
pub(crate) const NAME: &str = "wisp-science";
/// The MCP revision answered when the agent asks for none.
const PROTOCOL: &str = "2025-03-26";
/// The argument that puts the desktop's executable in relay mode.
pub const RELAY_FLAG: &str = "--wisp-mcp-relay";

static RELAY: OnceLock<PathBuf> = OnceLock::new();

/// Names the executable that relays stdio to the bridge (the desktop
/// itself, started with [`RELAY_FLAG`] and the address). Without one,
/// only agents that speak MCP over HTTP get Wisp's tools.
pub fn set_relay(program: PathBuf) {
    let _ = RELAY.set(program);
}

pub(crate) fn relay() -> Option<&'static PathBuf> {
    RELAY.get()
}

/// The tools offered to one conversation's agent, run in its folder.
struct Offer {
    root: PathBuf,
    tools: Vec<Box<dyn Tool>>,
}

/// The loopback MCP server and what it offers to whom.
pub(crate) struct Bridge {
    url: String,
    offers: Mutex<HashMap<String, Arc<Offer>>>,
}

impl Bridge {
    /// Starts listening on a free loopback port.
    pub(crate) async fn start() -> Result<Arc<Self>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("cannot listen on the loopback address for the ACP bridge")?;
        let bridge = Arc::new(Self {
            url: format!("http://{}", listener.local_addr()?),
            offers: Mutex::default(),
        });
        let app = Router::new()
            .route("/mcp/{token}", post(answer).get(no_stream))
            .with_state(bridge.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(bridge)
    }

    /// Offers `tools` under `token`, in place of what it offered before.
    pub(crate) fn offer(&self, token: &str, root: &Path, tools: Vec<Box<dyn Tool>>) {
        self.offers.lock().unwrap().insert(
            token.to_owned(),
            Arc::new(Offer {
                root: root.to_path_buf(),
                tools,
            }),
        );
    }

    pub(crate) fn withdraw(&self, token: &str) {
        self.offers.lock().unwrap().remove(token);
    }

    pub(crate) fn url(&self, token: &str) -> String {
        format!("{}/mcp/{token}", self.url)
    }
}

/// Runs a tool in the conversation's folder. It is the agent's own
/// permission that lets a call through; nothing here asks again, and
/// what would need a confirmation of Wisp's is refused.
struct Env {
    root: PathBuf,
}

#[async_trait::async_trait]
impl ToolEnv for Env {
    fn project_root(&self) -> &Path {
        &self.root
    }
    async fn confirm(&self, _message: &str) -> bool {
        false
    }
    async fn emit(&self, _event: wisp_tools::ToolEvent) {}
}

/// A plain GET would open a stream of server messages; there are none.
async fn no_stream() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn answer(
    State(bridge): State<Arc<Bridge>>,
    UrlPath(token): UrlPath<String>,
    Json(request): Json<Value>,
) -> Response {
    let Some(offer) = bridge.offers.lock().unwrap().get(&token).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match reply(&offer, &request).await {
        Some(reply) => Json(reply).into_response(),
        // A notification or a response wants no answer.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The answer to one JSON-RPC message, or nothing for a notification.
async fn reply(offer: &Offer, request: &Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => result(
            &id,
            json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Wisp Science's skills, literature and biology connectors, \
                    memory and runs for this project. Use your own tools for files and the shell.",
            }),
        ),
        "ping" => result(&id, json!({})),
        "tools/list" => result(
            &id,
            json!({
                "tools": offer.tools.iter().map(|tool| {
                    let schema = tool.schema().function;
                    json!({
                        "name": schema.name,
                        "description": schema.description,
                        "inputSchema": schema.parameters,
                    })
                }).collect::<Vec<_>>(),
            }),
        ),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(tool) = offer.tools.iter().find(|tool| tool.name() == name) else {
                return Some(failure(&id, -32602, &format!("Unknown tool: {name}")));
            };
            let args = params
                .get("arguments")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let env = Env {
                root: offer.root.clone(),
            };
            tool.before(&args, &env).await;
            let done = tool.run(&args, &env).await;
            let mut content = vec![json!({ "type": "text", "text": done.content })];
            for image in &done.images {
                if let Some((head, data)) = image.data_url.split_once(";base64,") {
                    content.push(json!({
                        "type": "image",
                        "mimeType": head.trim_start_matches("data:"),
                        "data": data,
                    }));
                }
            }
            result(&id, json!({ "content": content, "isError": !done.success }))
        }
        _ => failure(&id, -32601, &format!("Method not found: {method}")),
    })
}

/// Carries newline-delimited JSON-RPC from `input` to the bridge at
/// `url` and each answer back to `output`, until `input` ends.
pub(crate) async fn carry<R, W>(url: &str, input: R, mut output: W) -> Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    // Loopback only: no proxy may stand in between.
    let client = reqwest::Client::builder().no_proxy().build()?;
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = client
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            continue;
        }
        let body = response.error_for_status()?.text().await?;
        if !body.trim().is_empty() {
            output.write_all(body.trim().as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

/// Relay mode of the desktop's executable: stdio to the bridge at `url`.
pub fn relay_main(url: &str) -> std::process::ExitCode {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return std::process::ExitCode::FAILURE;
    };
    let done = runtime.block_on(carry(
        url,
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    ));
    match done {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wisp-science MCP relay: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
