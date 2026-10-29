//! Stdio MCP transport round-trip: concurrent `tools/call` must not steal
//! each other's JSON-RPC responses.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;
use wisp_mcp::McpClient;
use wisp_tools::{Registry, ToolEnv, ToolEvent};

struct TestEnv;

#[async_trait::async_trait]
impl ToolEnv for TestEnv {
    fn project_root(&self) -> &std::path::Path {
        std::path::Path::new(".")
    }
    async fn confirm(&self, _: &str) -> bool {
        true
    }
    async fn emit(&self, _: ToolEvent) {}
}

const ECHO_ARG: &str = "--fake-echo-mcp";

fn main() -> ExitCode {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some(ECHO_ARG) {
        return fake_echo_server();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    match runtime.block_on(run_stdio_transport_regressions()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stdio concurrent MCP transport failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fake_echo_server() -> ExitCode {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return ExitCode::FAILURE;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request.get("method").and_then(Value::as_str);
        let delay = request
            .pointer("/params/arguments/delay_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if request.pointer("/params/arguments/hold") == Some(&json!(true)) {
            continue;
        }
        if request.pointer("/params/arguments/crash") == Some(&json!(true)) {
            return ExitCode::FAILURE;
        }
        let pause_input = request.pointer("/params/arguments/pause_input") == Some(&json!(true));
        let result = match method {
            Some("initialize") => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-echo", "version": "1" }
            }),
            Some("tools/list") => json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo a token after an optional delay",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "token": { "type": "string" },
                            "delay_ms": { "type": "integer" }
                        },
                        "required": ["token"]
                    }
                }]
            }),
            Some("tools/call") => {
                let arguments = request
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                if arguments.get("rich") == Some(&json!(true)) {
                    json!({
                        "content": [
                            {"type": "text", "text": "TERMINAL: true\nNEXT_ACTION: ask_user"},
                            {"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC"},
                            {"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC"}
                        ],
                        "structuredContent": {"planDigest": "exact-token"},
                        "_meta": {"appOnly": "private-selection"},
                        "isError": true
                    })
                } else {
                    json!({
                        "content": [{ "type": "text", "text": "ok" }],
                        "structuredContent": {
                            "token": arguments.get("token").cloned().unwrap_or(Value::Null)
                        },
                        "isError": false
                    })
                }
            }
            _ => json!({}),
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        std::thread::spawn(move || {
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        });
        if pause_input {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    ExitCode::SUCCESS
}

async fn run_stdio_transport_regressions() -> Result<(), String> {
    agent_stop_cancels_wait_not_server().await?;
    deferred_tool_retains_rich_result().await?;
    concurrent_stdio_calls_keep_matching_ids().await?;
    cancelled_isolated_call_leaves_connection_usable().await?;
    unlimited_stdio_and_late_response().await?;
    cancellation_during_write_preserves_framing().await?;
    managed_stdio_reconnects_without_replay().await?;
    idle_managed_stdio_suspends_and_relaunches_on_demand().await
}

struct CancelEnv(std::sync::atomic::AtomicBool);
#[async_trait::async_trait]
impl ToolEnv for CancelEnv {
    fn project_root(&self) -> &std::path::Path {
        std::path::Path::new(".")
    }
    async fn confirm(&self, _: &str) -> bool {
        true
    }
    async fn emit(&self, _: ToolEvent) {}
    fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}
async fn agent_stop_cancels_wait_not_server() -> Result<(), String> {
    use wisp_tools::Tool;
    let client = echo().await?;
    let remote = client
        .tools_list()
        .await
        .map_err(|e| e.to_string())?
        .remove(0);
    let tool = wisp_mcp::McpTool::new(remote, client.clone());
    let env = CancelEnv(std::sync::atomic::AtomicBool::new(false));
    let args = json!({"token":"held","hold":true});
    let (result, ()) = tokio::join!(tool.run(&args, &env), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        env.0.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert!(!result.success);
    assert!(result.content.contains("outcome may be unknown"));
    assert!(client.is_connected());
    assert_eq!(
        client
            .tool_call("echo", &json!({"token":"next"}))
            .await
            .map_err(|e| e.to_string())?,
        "ok"
    );
    client.shutdown().await.map_err(|e| e.to_string())
}

async fn deferred_tool_retains_rich_result() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let client = std::sync::Arc::new(
        McpClient::launch(&executable.to_string_lossy(), &[ECHO_ARG.into()])
            .await
            .map_err(|e| e.to_string())?,
    );
    let remote = client
        .tools_list()
        .await
        .map_err(|e| e.to_string())?
        .remove(0);
    let mut registry = Registry::builtins();
    registry.add(Box::new(wisp_mcp::McpTool::new(remote, client.clone())));
    let result = registry
        .run(
            "use_mcp_tool",
            &json!({
                "tool_name": "echo", "tool_input": {"token": "rich", "rich": true}
            }),
            &TestEnv,
        )
        .await;
    assert!(!result.success);
    assert_eq!(result.images.len(), 2);
    assert!(result.content.contains("exact-token"));
    assert!(result.content.contains("NEXT_ACTION: ask_user"));
    assert!(!result.content.contains("private-selection"));
    drop(registry);
    client.shutdown().await.map_err(|e| e.to_string())
}

async fn concurrent_stdio_calls_keep_matching_ids() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![ECHO_ARG.to_string()];
    let client = McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .map_err(|error| error.to_string())?;
    let slow_args = json!({ "token": "slow", "delay_ms": 180 });
    let fast_args = json!({ "token": "fast", "delay_ms": 20 });
    let slow = client.tool_call_rich("echo", &slow_args);
    let fast = client.tool_call_rich("echo", &fast_args);
    let (slow, fast) = tokio::join!(slow, fast);
    let slow = slow.map_err(|error| error.to_string())?;
    let fast = fast.map_err(|error| error.to_string())?;
    if slow
        .structured_content
        .as_ref()
        .and_then(|v| v.get("token"))
        != Some(&json!("slow"))
    {
        return Err(format!("slow call received {:?}", slow.structured_content));
    }
    if fast
        .structured_content
        .as_ref()
        .and_then(|v| v.get("token"))
        != Some(&json!("fast"))
    {
        return Err(format!("fast call received {:?}", fast.structured_content));
    }
    client
        .shutdown()
        .await
        .map_err(|error| format!("shutdown after successful concurrent calls: {error}"))?;
    // EOF-only servers exit before the tree's signal grace period. Repeated
    // shutdown must also succeed once that unreaped zombie has been handled.
    client
        .shutdown()
        .await
        .map_err(|error| format!("repeated shutdown: {error}"))?;
    Ok(())
}

async fn cancelled_isolated_call_leaves_connection_usable() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![ECHO_ARG.to_string()];
    let client = McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .map_err(|error| error.to_string())?;
    let hang_args = json!({ "token": "hang", "delay_ms": 400 });
    let hang = client.tool_call_rich_isolated("echo", &hang_args);
    if tokio::time::timeout(Duration::from_millis(60), hang)
        .await
        .is_ok()
    {
        let _ = client.shutdown().await;
        return Err("isolated call returned before the host timeout".into());
    }
    let recovered_args = json!({ "token": "recovered" });
    let recovered = client
        .tool_call_rich_isolated("echo", &recovered_args)
        .await
        .map_err(|error| error.to_string())?;
    if recovered
        .structured_content
        .as_ref()
        .and_then(|value| value.get("token"))
        != Some(&json!("recovered"))
    {
        let _ = client.shutdown().await;
        return Err(format!(
            "connection unusable after isolated cancel: {:?}",
            recovered.structured_content
        ));
    }
    client.shutdown().await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn echo() -> Result<std::sync::Arc<McpClient>, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    McpClient::launch(&executable.to_string_lossy(), &[ECHO_ARG.into()])
        .await
        .map(std::sync::Arc::new)
        .map_err(|e| e.to_string())
}
async fn unlimited_stdio_and_late_response() -> Result<(), String> {
    let client = echo().await?;
    let call = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .tool_call("echo", &json!({"token":"hold","hold":true}))
                .await
        })
    };
    // An ordered, independent round trip establishes that the held call was written.
    tokio::time::sleep(Duration::from_millis(30)).await;
    client.tools_list().await.map_err(|e| e.to_string())?;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(121)).await;
    tokio::task::yield_now().await;
    assert!(
        !call.is_finished(),
        "stdio tools/call must have no execution deadline"
    );
    tokio::time::resume();
    call.abort();
    let _ = call.await;
    assert!(client.is_connected());
    assert_eq!(
        client
            .tool_call("echo", &json!({"token":"next"}))
            .await
            .map_err(|e| e.to_string())?,
        "ok"
    );
    client.shutdown().await.map_err(|e| e.to_string())
}
async fn cancellation_during_write_preserves_framing() -> Result<(), String> {
    let client = echo().await?;
    client
        .tool_call("echo", &json!({"token":"pause","pause_input":true}))
        .await
        .map_err(|e| e.to_string())?;
    let call = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .tool_call("echo", &json!({"token":"x".repeat(8*1024*1024)}))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    call.abort();
    let _ = call.await;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.tool_call("echo", &json!({"token":"after-partial-write"})),
    )
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    assert_eq!(result, "ok");
    assert!(client.is_connected());
    client.shutdown().await.map_err(|e| e.to_string())
}
async fn managed_stdio_reconnects_without_replay() -> Result<(), String> {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wisp_mcp::connection::{ClientFactory, ManagedConnection};
    let launches = Arc::new(AtomicUsize::new(0));
    let observed = launches.clone();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let factory: ClientFactory = Arc::new(move || {
        let executable = executable.clone();
        observed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            McpClient::launch(&executable.to_string_lossy(), &[ECHO_ARG.into()]).await
        })
    });
    let client = Arc::new(McpClient::managed(ManagedConnection::new(factory)));
    client.tools_list().await.map_err(|e| e.to_string())?;
    let error = client
        .tool_call("echo", &json!({"token":"write","crash":true}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outcome unknown"));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let client = client.clone();
        tasks.spawn(async move { client.tool_call("echo", &json!({"token":"new"})).await });
    }
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap().unwrap(), "ok");
    }
    assert_eq!(launches.load(Ordering::SeqCst), 2);
    client.shutdown().await.map_err(|e| e.to_string())
}
async fn idle_managed_stdio_suspends_and_relaunches_on_demand() -> Result<(), String> {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wisp_mcp::connection::{ClientFactory, ManagedConnection};
    let launches = Arc::new(AtomicUsize::new(0));
    let observed = launches.clone();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let factory: ClientFactory = Arc::new(move || {
        let executable = executable.clone();
        observed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            McpClient::launch(&executable.to_string_lossy(), &[ECHO_ARG.into()]).await
        })
    });
    let client = Arc::new(McpClient::managed(ManagedConnection::new(factory)));
    // Never launched: nothing to suspend.
    assert!(!client.suspend_if_idle(Duration::ZERO).await);
    client.tools_list().await.map_err(|e| e.to_string())?;
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    // Recently used.
    assert!(!client.suspend_if_idle(Duration::from_secs(3600)).await);
    // A running call keeps its process even when the threshold has passed.
    let call = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .tool_call("echo", &json!({"token":"busy","delay_ms":300}))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!client.suspend_if_idle(Duration::ZERO).await);
    assert_eq!(call.await.unwrap().map_err(|e| e.to_string())?, "ok");
    assert!(client.is_connected());

    assert!(client.suspend_if_idle(Duration::ZERO).await);
    assert!(!client.is_connected());
    assert!(client.needs_catalog_refresh());
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    // The next request relaunches as a new generation instead of failing.
    assert_eq!(
        client
            .tool_call("echo", &json!({"token":"again"}))
            .await
            .map_err(|e| e.to_string())?,
        "ok"
    );
    assert_eq!(launches.load(Ordering::SeqCst), 2);
    assert_eq!(client.generation(), 2);
    client.shutdown().await.map_err(|e| e.to_string())?;
    // A closed connection is never relaunched by a later sweep.
    assert!(!client.suspend_if_idle(Duration::ZERO).await);
    Ok(())
}
