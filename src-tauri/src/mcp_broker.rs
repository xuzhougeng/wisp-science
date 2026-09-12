//! Private ACP-to-Host transport. Grants are memory-only and scoped by the Host,
//! never by request-provided project paths. Plugin processes remain Host-owned.
use crate::*;
use axum::{
    extract::{DefaultBodyLimit, Query, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::oneshot;

pub(crate) struct ProxyConfig {
    pub(crate) base: String,
    pub(crate) token: String,
}
static PROXY_CONFIG: std::sync::OnceLock<ProxyConfig> = std::sync::OnceLock::new();

/// Called by the single-threaded re-exec entrypoint, before building a runtime.
/// Retain the capability in memory, not in the environment inherited by Runs.
pub(crate) fn capture_proxy_environment() {
    let pair = (
        std::env::var("WISP_MCP_HOST_URL"),
        std::env::var("WISP_MCP_HOST_TOKEN"),
    );
    // This entrypoint has not started worker threads or the desktop shell.
    unsafe {
        std::env::remove_var("WISP_MCP_HOST_URL");
        std::env::remove_var("WISP_MCP_HOST_TOKEN");
    }
    if let (Ok(base), Ok(token)) = pair {
        let _ = PROXY_CONFIG.set(ProxyConfig { base, token });
    }
}
pub(crate) fn proxy_config() -> Option<&'static ProxyConfig> {
    PROXY_CONFIG.get()
}

struct Grant {
    app_data: PathBuf,
    project: String,
    frame: String,
    allowed: Option<HashSet<String>>,
    revoked: AtomicBool,
    pending: StdMutex<HashMap<(String, String, String), oneshot::Sender<()>>>,
    catalogs: StdMutex<HashMap<(String, String), Vec<wisp_mcp::RemoteTool>>>,
}
impl Grant {
    fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        for (_, cancel) in self.pending.lock().unwrap().drain() {
            let _ = cancel.send(());
        }
    }
}
type Grants = Arc<StdMutex<HashMap<String, Arc<Grant>>>>;
struct Broker {
    address: String,
    grants: Grants,
    closed: bool,
}
fn slot() -> &'static StdMutex<Option<Broker>> {
    static BROKER: std::sync::OnceLock<StdMutex<Option<Broker>>> = std::sync::OnceLock::new();
    BROKER.get_or_init(Default::default)
}

pub(crate) fn launch_env(
    app_data: &Path,
    project: &ActiveProject,
    frame: &str,
    allowed: Option<&[String]>,
) -> Result<Vec<wisp_acp::acp::schema::v1::EnvVariable>, String> {
    let mut slot = slot().lock().unwrap();
    if slot.is_none() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| e.to_string())?;
        let address = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let grants: Grants = Default::default();
        let router = Router::new()
            .route("/mcp", post(request))
            .route("/lease", get(lease))
            .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
            .with_state(grants.clone());
        tauri::async_runtime::spawn(async move {
            if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
                let _ = axum::serve(listener, router).await;
            }
        });
        *slot = Some(Broker {
            address,
            grants,
            closed: false,
        });
    }
    let broker = slot.as_ref().unwrap();
    if broker.closed {
        return Err("Wisp MCP Host is shutting down".into());
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    broker.grants.lock().unwrap().insert(
        token.clone(),
        Arc::new(Grant {
            app_data: app_data.into(),
            project: project.id.clone(),
            frame: frame.into(),
            allowed: allowed.map(|a| {
                a.iter()
                    .filter_map(|s| crate::delegation_resources::connector_from_token(s))
                    .map(str::to_string)
                    .collect()
            }),
            revoked: AtomicBool::new(false),
            pending: Default::default(),
            catalogs: Default::default(),
        }),
    );
    use wisp_acp::acp::schema::v1::EnvVariable;
    Ok(vec![
        EnvVariable::new("WISP_MCP_HOST_URL", &broker.address),
        EnvVariable::new("WISP_MCP_HOST_TOKEN", token),
    ])
}

pub(crate) fn cancel_frame(frame: &str) {
    if let Some(broker) = slot().lock().unwrap().as_ref() {
        broker.grants.lock().unwrap().retain(|_, grant| {
            if grant.frame == frame {
                grant.revoke();
                false
            } else {
                true
            }
        });
    }
}

pub(crate) fn shutdown() {
    if let Some(broker) = slot().lock().unwrap().as_mut() {
        broker.closed = true;
        for (_, grant) in broker.grants.lock().unwrap().drain() {
            grant.revoke();
        }
    }
}
fn authorize(grants: &Grants, headers: &HeaderMap) -> Result<(String, Arc<Grant>), StatusCode> {
    if headers.contains_key("origin") {
        return Err(StatusCode::FORBIDDEN);
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let grant = grants
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .filter(|g| !g.revoked.load(Ordering::SeqCst))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok((token.into(), grant))
}
struct Lease {
    token: String,
    grant: Arc<Grant>,
    grants: Grants,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.grant.revoke();
        self.grants.lock().unwrap().remove(&self.token);
    }
}
async fn lease(
    AxumState(grants): AxumState<Grants>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let (token, grant) = authorize(&grants, &headers)?;
    let stream = futures_util::stream::unfold(
        Lease {
            token,
            grant,
            grants,
        },
        |lease| async move {
            if lease.grant.revoked.load(Ordering::SeqCst) {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Some((Ok::<_, std::io::Error>(": alive\n\n"), lease))
        },
    );
    Ok((
        [("content-type", "text/event-stream")],
        axum::body::Body::from_stream(stream),
    )
        .into_response())
}
struct Pending {
    grant: Arc<Grant>,
    key: (String, String, String),
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.grant.pending.lock().unwrap().remove(&self.key);
    }
}

async fn request(
    AxumState(grants): AxumState<Grants>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Json(rpc): Json<Value>,
) -> Result<Response, StatusCode> {
    let (_, grant) = authorize(&grants, &headers)?;
    let connector = query
        .get("connector")
        .cloned()
        .ok_or(StatusCode::BAD_REQUEST)?;
    if grant
        .allowed
        .as_ref()
        .is_some_and(|a| !a.contains(&connector))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let session = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let method = rpc["method"].as_str().unwrap_or("");
    if method == "notifications/cancelled" {
        let key = (connector, session, rpc["params"]["requestId"].to_string());
        if let Some(cancel) = grant.pending.lock().unwrap().remove(&key) {
            let _ = cancel.send(());
        }
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    if method == "notifications/initialized" {
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    let id = rpc.get("id").cloned().ok_or(StatusCode::BAD_REQUEST)?;
    // Carry identity on the event itself as well as transport spans. This stays
    // useful across the HTTP/stdio task boundary and with span-less log sinks.
    let started = std::time::Instant::now();
    tracing::info!(target: "wisp", frame=%grant.frame, connector=%connector, method, request_id=id.as_u64().unwrap_or_default(), "mcp.broker.request.started");
    let (cancel, cancelled) = oneshot::channel();
    let key = (connector.clone(), session.clone(), id.to_string());
    {
        let mut pending = grant.pending.lock().unwrap();
        if pending.len() >= 64 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        if pending.contains_key(&key) {
            return Err(StatusCode::CONFLICT);
        }
        pending.insert(key.clone(), cancel);
    }
    let _pending = Pending {
        grant: grant.clone(),
        key,
    };
    let new_session = if method == "initialize" {
        Uuid::new_v4().to_string()
    } else {
        session.clone()
    };
    let work = async {
        let store = Store::open(&grant.app_data.join("wisp.sqlite")).await?;
        if store.frame_project_id(&grant.frame).await?.as_deref() != Some(&grant.project) {
            anyhow::bail!("MCP conversation no longer exists");
        }
        let scope = store
            .frame_state_scope(&grant.frame)
            .await?
            .ok_or_else(|| anyhow::anyhow!("MCP conversation scope missing"))?;
        let (specs, _) = mcp_connections::configured(&store, &grant.project).await;
        let spec = specs
            .iter()
            .find(|s| s.id() == connector)
            .ok_or_else(|| anyhow::anyhow!("MCP connector disabled or removed"))?;
        let client = mcp_connections::host()
            .acquire(
                &store,
                &grant.project,
                &grant.frame,
                scope.scope_key(),
                spec,
            )
            .await
            .map_err(anyhow::Error::msg)?;
        match method {
            "initialize" | "tools/list" => {
                let tools: Vec<_> = client
                    .tools_list()
                    .await?
                    .into_iter()
                    .filter(|t| t.visible_to_model())
                    .collect();
                grant
                    .catalogs
                    .lock()
                    .unwrap()
                    .insert((connector.clone(), new_session.clone()), tools.clone());
                if method == "initialize" {
                    Ok(
                        json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"wisp-host","version":env!("CARGO_PKG_VERSION")}}),
                    )
                } else {
                    Ok(json!({"tools":tools}))
                }
            }
            "tools/call" => {
                let name = rpc["params"]["name"].as_str().unwrap_or("");
                let expected = grant
                    .catalogs
                    .lock()
                    .unwrap()
                    .get(&(connector.clone(), session.clone()))
                    .and_then(|ts| ts.iter().find(|t| t.name == name))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("MCP tool not in this bridge's catalog"))?;
                if plan_mode::session_plan_mode(&store, &grant.frame).await && !expected.read_only()
                {
                    anyhow::bail!("MCP write blocked in Plan mode");
                }
                if grant.revoked.load(Ordering::SeqCst) {
                    anyhow::bail!("MCP bridge closed; request not sent");
                }
                serde_json::to_value(
                    client
                        .tool_call_checked(&expected, &rpc["params"]["arguments"])
                        .await?,
                )
                .map_err(Into::into)
            }
            _ => Err(anyhow::anyhow!("Unsupported private MCP broker method")),
        }
    };
    let result: anyhow::Result<Value> = tokio::select! { result = work => result, _ = cancelled => Err(anyhow::anyhow!("MCP wait cancelled; external operation outcome may be unknown; do not replay")) };
    tracing::info!(target: "wisp", frame=%grant.frame, connector=%connector, method, request_id=id.as_u64().unwrap_or_default(), elapsed_ms=started.elapsed().as_millis() as u64, success=result.is_ok(), "mcp.broker.request.finished");
    let response = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
        }
    };
    Ok(([("mcp-session-id", new_session)], Json(response)).into_response())
}

/// The bridge receives only its own ephemeral grant, never a global credential.
pub(crate) async fn proxy_client(connector: &str) -> anyhow::Result<wisp_mcp::McpClient> {
    let config =
        proxy_config().ok_or_else(|| anyhow::anyhow!("Missing private Host capability"))?;
    let (base, token) = (&config.base, &config.token);
    let mut url = reqwest::Url::parse(&format!("{base}/mcp"))?;
    if url.host_str() != Some("127.0.0.1") || url.scheme() != "http" {
        anyhow::bail!("Invalid private MCP broker address");
    }
    url.query_pairs_mut().append_pair("connector", connector);
    wisp_mcp::McpClient::connect_http_with_proxy(
        url.as_str(),
        &[("Authorization".into(), format!("Bearer {token}"))],
        "none",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct AuditBuffer(Arc<StdMutex<Vec<u8>>>);
    impl std::io::Write for AuditBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn fixture() -> (Grants, Arc<Grant>, HeaderMap) {
        let grant = Arc::new(Grant {
            app_data: PathBuf::new(),
            project: "p".into(),
            frame: "f".into(),
            allowed: Some(HashSet::from(["allowed".into()])),
            revoked: AtomicBool::new(false),
            pending: Default::default(),
            catalogs: Default::default(),
        });
        let grants = Arc::new(StdMutex::new(HashMap::from([(
            "test-capability".into(),
            grant.clone(),
        )])));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-capability".parse().unwrap());
        (grants, grant, headers)
    }
    #[tokio::test]
    async fn broker_long_call_survives_deadlines_and_lease_loss_retains_host_connection() {
        let audit = AuditBuffer(Default::default());
        let captured = audit.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || captured.clone())
            .finish();
        use tracing::instrument::WithSubscriber;
        // Attach the dispatcher to each async operation. A thread-local guard
        // is not a reliable assertion boundary when the suite spawns tasks.
        let dispatch = tracing::Dispatch::new(subscriber);
        let entered = Arc::new(tokio::sync::Notify::new());
        let observed = entered.clone();
        let initializations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inits = initializations.clone();
        let router = Router::new().route("/", post(move |Json(rpc): Json<Value>| {
            let (entered, initializations) = (observed.clone(), inits.clone());
            async move {
                let result = match rpc["method"].as_str().unwrap_or("") {
                    "initialize" => { initializations.fetch_add(1, Ordering::SeqCst); json!({"capabilities":{"tools":{}}}) }
                    "tools/list" => json!({"tools":[{"name":"echo","description":"fixture","inputSchema":{"type":"object"}}]}),
                    "tools/call" => {
                        if rpc["params"]["arguments"]["hold"] == true {
                            entered.notify_one();
                            std::future::pending::<()>().await;
                        }
                        json!({"content":[{"type":"text","text":"ok"}]})
                    }
                    _ => json!({}),
                };
                Json(json!({"jsonrpc":"2.0","id":rpc["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let root = std::env::temp_dir().join(format!("wisp-broker-long-{}", Uuid::new_v4()));
        let store = Store::open(&root.join("wisp.sqlite")).await.unwrap();
        let frame = Uuid::new_v4().to_string();
        store
            .create_project("p", "test", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame(&frame, "p", "test", "model")
            .await
            .unwrap();
        let connection = McpConnection {
            id: "allowed".into(),
            name: "fixture".into(),
            enabled: true,
            transport: McpTransport::Http {
                url,
                headers: vec![],
                auth: McpHttpAuth::None,
            },
        };
        save_mcp_connections(&store, &[connection]).await.unwrap();
        let grant = Arc::new(Grant {
            app_data: root.clone(),
            project: "p".into(),
            frame: frame.clone(),
            allowed: None,
            revoked: AtomicBool::new(false),
            pending: Default::default(),
            catalogs: Default::default(),
        });
        let grants = Arc::new(StdMutex::new(HashMap::from([(
            "cap".into(),
            grant.clone(),
        )])));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer cap".parse().unwrap());
        let query = HashMap::from([("connector".into(), "allowed".into())]);
        let initialized = request(
            AxumState(grants.clone()),
            headers.clone(),
            Query(query.clone()),
            Json(json!({"id":1,"method":"initialize"})),
        )
        .with_subscriber(dispatch.clone())
        .await
        .unwrap();
        assert!(initialized.status().is_success());
        headers.insert(
            "mcp-session-id",
            initialized.headers()["mcp-session-id"].clone(),
        );
        let task = {
            let (grants, headers, query) = (grants.clone(), headers.clone(), query.clone());
            let dispatch = dispatch.clone();
            tokio::spawn(async move {
                request(AxumState(grants),headers,Query(query),Json(json!({"id":2,"method":"tools/call","params":{"name":"echo","arguments":{"hold":true,"secret":"DO_NOT_LOG_MCP_ARGUMENTS_4817"}}}))).with_subscriber(dispatch).await
            })
        };
        entered.notified().await;
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(121)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        tokio::time::resume();
        grant.revoke();
        let result = task.await.unwrap().unwrap();
        let bytes = axum::body::to_bytes(result.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("outcome may be unknown"));
        let (specs, _) = mcp_connections::configured(&store, "p").await;
        let scope = store.frame_state_scope(&frame).await.unwrap().unwrap();
        let client = mcp_connections::host()
            .acquire(&store, "p", &frame, scope.scope_key(), &specs[0])
            .await
            .unwrap();
        assert!(client.is_connected());
        assert_eq!(
            client
                .tool_call("echo", &json!({}))
                .with_subscriber(dispatch.clone())
                .await
                .unwrap(),
            "ok"
        );
        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        let log = String::from_utf8(audit.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("mcp.request.started"), "audit: {log}");
        assert!(
            log.contains(&frame) && log.contains("connector=allowed"),
            "audit: {log}"
        );
        assert!(!log.contains("DO_NOT_LOG_MCP_ARGUMENTS_4817"));
        mcp_connections::host().restart_frame(&frame).await;
        server.abort();
        drop((client, store, grant, grants));
        let _ = std::fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn broker_rejects_cross_connector_and_browser_requests() {
        let (grants, grant, mut headers) = fixture();
        assert!(authorize(&grants, &headers).is_ok());
        assert!(authorize(&grants, &HeaderMap::new()).is_err());
        headers.insert("origin", "http://untrusted.invalid".parse().unwrap());
        assert_eq!(
            authorize(&grants, &headers).err(),
            Some(StatusCode::FORBIDDEN)
        );
        headers.remove("origin");
        assert_eq!(
            request(
                AxumState(grants.clone()),
                headers.clone(),
                Query(HashMap::from([("connector".into(), "other".into())])),
                Json(json!({"id":1,"method":"tools/list"}))
            )
            .await
            .err(),
            Some(StatusCode::FORBIDDEN)
        );
        grant.revoke();
        assert_eq!(
            authorize(&grants, &headers).err(),
            Some(StatusCode::UNAUTHORIZED)
        );
    }
    #[tokio::test]
    async fn cancellation_and_lease_loss_cancel_only_this_grant() {
        let (grants, grant, headers) = fixture();
        let (cancel, cancelled) = oneshot::channel();
        grant
            .pending
            .lock()
            .unwrap()
            .insert(("allowed".into(), "".into(), "7".into()), cancel);
        request(
            AxumState(grants.clone()),
            headers,
            Query(HashMap::from([("connector".into(), "allowed".into())])),
            Json(json!({"method":"notifications/cancelled","params":{"requestId":7}})),
        )
        .await
        .unwrap();
        cancelled.await.unwrap();
        assert!(!grant.revoked.load(Ordering::SeqCst));
        let (cancel, cancelled) = oneshot::channel();
        grant
            .pending
            .lock()
            .unwrap()
            .insert(("allowed".into(), "".into(), "8".into()), cancel);
        drop(Lease {
            token: "test-capability".into(),
            grant: grant.clone(),
            grants: grants.clone(),
        });
        cancelled.await.unwrap();
        assert!(grant.revoked.load(Ordering::SeqCst));
        assert!(grants.lock().unwrap().is_empty());
    }
}
