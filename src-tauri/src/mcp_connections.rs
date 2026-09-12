//! Host-owned, scope-isolated MCP connections. Agent/view lifetimes are not owners.
use crate::*;
use serde_json::json;
use wisp_mcp::{
    connection::{ClientFactory, ManagedConnection},
    McpClient,
};

#[derive(Clone)]
pub(crate) enum Spec {
    Custom(McpConnection),
    Plugin(plugins::PluginMcpLaunch),
    Development(Vec<String>),
}
impl Spec {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Custom(c) => &c.id,
            Self::Plugin(p) => &p.connector_id,
            Self::Development(_) => BUNDLED_DEV_MCP_CONNECTOR_ID,
        }
    }
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Custom(c) => &c.name,
            Self::Plugin(p) => &p.display_name,
            Self::Development(_) => "Development MCP",
        }
    }
    pub(crate) fn plugin_id(&self) -> Option<&str> {
        match self {
            Self::Plugin(p) => Some(&p.plugin_id),
            _ => None,
        }
    }
    fn descriptor(&self) -> String {
        // Memory only; never log a descriptor (it may contain user-supplied env).
        let value = match self {
            Self::Custom(c) => serde_json::to_value(c),
            Self::Plugin(p) => serde_json::to_value(p),
            Self::Development(parts) => serde_json::to_value(parts),
        }
        .unwrap();
        json!([value, network::mcp_proxy()]).to_string()
    }
    fn factory(&self) -> ClientFactory {
        let spec = self.clone();
        Arc::new(move || {
            let spec = spec.clone();
            Box::pin(async move {
                match spec {
                    Self::Custom(c) => connect_mcp(&c).await,
                    Self::Plugin(p) => connect_plugin_mcp(&p).await,
                    Self::Development(parts) => McpClient::launch(&parts[0], &parts[1..]).await,
                }
            })
        })
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    project: String,
    frame: String,
    scope: String,
    context: String,
    connector: String,
}
struct Entry {
    descriptor: String,
    client: Arc<McpClient>,
}
#[derive(Default)]
pub(crate) struct Connections {
    entries: tokio::sync::Mutex<HashMap<Key, Entry>>,
    // Access only while holding entries, so retirement and registration are atomic.
    // Frame IDs are never reused; retain tombstones until the Host exits.
    retired_frames: StdMutex<HashSet<String>>,
    closing: Arc<AtomicBool>,
}
pub(crate) fn host() -> &'static Connections {
    static HOST: std::sync::OnceLock<Connections> = std::sync::OnceLock::new();
    HOST.get_or_init(Connections::default)
}

pub(crate) async fn configured(
    store: &Store,
    project: &str,
) -> (Vec<Spec>, Vec<plugins::PluginRuntimeError>) {
    let (plugins, errors) = plugins::enabled_plugin_mcp_launches(store, project).await;
    let mut specs: Vec<_> = plugins.into_iter().map(Spec::Plugin).collect();
    specs.extend(
        load_mcp_connections(store)
            .await
            .into_iter()
            .filter(|c| c.enabled)
            .map(Spec::Custom),
    );
    if let Ok(command) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<String> = command
            .split_whitespace()
            .map(|s| {
                if s.ends_with(".py") {
                    wisp_runtime::resolve_bundled_script(s)
                        .to_string_lossy()
                        .into_owned()
                } else {
                    s.to_string()
                }
            })
            .collect();
        if !parts.is_empty() {
            specs.push(Spec::Development(parts));
        }
    }
    (specs, errors)
}

impl Connections {
    pub(crate) async fn needs_catalog_refresh(&self, frame: &str) -> bool {
        self.entries
            .lock()
            .await
            .iter()
            .any(|(key, entry)| key.frame == frame && entry.client.needs_catalog_refresh())
    }
    pub(crate) async fn invalidate_connector(&self, connector: &str) {
        tracing::info!(target: "wisp", connector, reason="configuration-replaced", "mcp.scope.close");
        let mut entries = self.entries.lock().await;
        let keys: Vec<_> = entries
            .keys()
            .filter(|k| k.connector == connector)
            .cloned()
            .collect();
        let old: Vec<_> = keys
            .into_iter()
            .filter_map(|k| entries.remove(&k))
            .collect();
        drop(entries);
        for entry in old {
            let _ = entry.client.shutdown().await;
        }
    }
    pub(crate) async fn restart_frame(&self, frame: &str) {
        tracing::info!(target: "wisp", frame, reason="explicit-restart", "mcp.scope.close");
        let mut entries = self.entries.lock().await;
        let keys: Vec<_> = entries
            .keys()
            .filter(|k| k.frame == frame)
            .cloned()
            .collect();
        let old: Vec<_> = keys
            .into_iter()
            .filter_map(|k| entries.remove(&k))
            .collect();
        drop(entries);
        for entry in old {
            let _ = entry.client.shutdown().await;
        }
    }
    pub(crate) async fn retire_frame(&self, frame: &str) {
        let mut entries = self.entries.lock().await;
        self.retired_frames.lock().unwrap().insert(frame.into());
        let keys: Vec<_> = entries
            .keys()
            .filter(|k| k.frame == frame)
            .cloned()
            .collect();
        let old: Vec<_> = keys
            .into_iter()
            .filter_map(|k| entries.remove(&k))
            .collect();
        drop(entries);
        tracing::info!(target: "wisp", frame, reason="conversation-deleted", "mcp.scope.close");
        // Dropping a window's delete future must not abort cleanup after the
        // entries have been removed. Dropped JoinHandles let shutdown finish.
        let tasks: Vec<_> = old
            .into_iter()
            .map(|entry| {
                tokio::spawn(async move {
                    let _ = entry.client.shutdown().await;
                })
            })
            .collect();
        for task in tasks {
            let _ = task.await;
        }
    }
    pub(crate) async fn acquire(
        &self,
        store: &Store,
        project: &str,
        frame: &str,
        scope: &str,
        spec: &Spec,
    ) -> Result<Arc<McpClient>, String> {
        if store
            .frame_project_id(frame)
            .await
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(project)
        {
            return Err("MCP conversation was deleted or moved".into());
        }
        let context = format!(
            "{:?}",
            ssh_hosts::stored_session_default_execution_context(store, frame).await
        );
        let key = Key {
            project: project.into(),
            frame: frame.into(),
            scope: scope.into(),
            context,
            connector: spec.id().into(),
        };
        let descriptor = spec.descriptor();
        let (client, old) = {
            let mut entries = self.entries.lock().await;
            if self.closing.load(Ordering::SeqCst)
                || self.retired_frames.lock().unwrap().contains(frame)
            {
                return Err("MCP conversation or Host was closed; request not sent".into());
            }
            if let Some(entry) = entries.get(&key) {
                if entry.descriptor == descriptor {
                    return Ok(entry.client.clone());
                }
            }
            let factory = spec.factory();
            let store = store.clone();
            let project_owned = project.to_string();
            let frame_owned = frame.to_string();
            let connector = spec.id().to_string();
            let expected = descriptor.clone();
            let closing = self.closing.clone();
            let checked: ClientFactory = Arc::new(move || {
                let (store, project, frame, connector, expected, factory) = (
                    store.clone(),
                    project_owned.clone(),
                    frame_owned.clone(),
                    connector.clone(),
                    expected.clone(),
                    factory.clone(),
                );
                let closing = closing.clone();
                Box::pin(async move {
                    let valid = || async {
                        if closing.load(Ordering::SeqCst) {
                            return false;
                        }
                        if store
                            .frame_project_id(&frame)
                            .await
                            .ok()
                            .flatten()
                            .as_deref()
                            != Some(&project)
                        {
                            return false;
                        }
                        configured(&store, &project)
                            .await
                            .0
                            .iter()
                            .any(|s| s.id() == connector && s.descriptor() == expected)
                    };
                    if !valid().await {
                        anyhow::bail!("MCP connector disabled or replaced; request not sent");
                    }
                    let client = factory().await?;
                    if !valid().await {
                        let _ = client.shutdown().await;
                        anyhow::bail!("MCP configuration changed while connecting");
                    }
                    Ok(client)
                })
            });
            let client = Arc::new(McpClient::managed(
                ManagedConnection::new(checked).with_identity(project, frame, spec.id()),
            ));
            let old = entries.insert(
                key,
                Entry {
                    descriptor,
                    client: client.clone(),
                },
            );
            (client, old)
        };
        if let Some(old) = old {
            let _ = old.client.shutdown().await;
        }
        Ok(client)
    }

    /// Config changes close only changed/disabled connections, never every idle agent.
    pub(crate) async fn reconcile(&self, store: &Store, project: Option<&str>) {
        let keys: Vec<_> = self
            .entries
            .lock()
            .await
            .keys()
            .filter(|k| project.is_none_or(|p| p == k.project))
            .cloned()
            .collect();
        let mut specs = HashMap::new();
        for key in keys {
            let owner = match store.frame_project_id(&key.frame).await {
                Ok(owner) => owner,
                Err(error) => {
                    tracing::warn!(target: "wisp", frame=%key.frame, %error, "MCP ownership check failed");
                    continue;
                }
            };
            if owner.as_deref() != Some(&key.project) {
                self.retire_frame(&key.frame).await;
                continue;
            }
            if !specs.contains_key(&key.project) {
                let (current, _) = configured(store, &key.project).await;
                specs.insert(
                    key.project.clone(),
                    current
                        .into_iter()
                        .map(|s| (s.id().to_string(), s.descriptor()))
                        .collect::<HashMap<_, _>>(),
                );
            }
            let context = format!(
                "{:?}",
                ssh_hosts::stored_session_default_execution_context(store, &key.frame).await
            );
            let mut entries = self.entries.lock().await;
            let stale = entries.get(&key).is_some_and(|e| {
                specs[&key.project].get(&key.connector) != Some(&e.descriptor)
                    || key.context != context
            });
            let old = if stale { entries.remove(&key) } else { None };
            drop(entries);
            if let Some(old) = old {
                tracing::info!(target: "wisp", connector=%key.connector, frame=%key.frame, reason="disabled-or-config-changed", "mcp.scope.close");
                let _ = old.client.shutdown().await;
            }
        }
    }
    pub(crate) async fn shutdown_all(&self) {
        tracing::info!(target: "wisp", reason="host-exit", "mcp.scope.close_all");
        self.closing.store(true, Ordering::SeqCst);
        let entries = std::mem::take(&mut *self.entries.lock().await);
        let mut tasks = tokio::task::JoinSet::new();
        for (_, entry) in entries {
            tasks.spawn(async move {
                let _ = entry.client.shutdown().await;
            });
        }
        while tasks.join_next().await.is_some() {}
    }
}

/// Read the trusted persisted presentation, never a binding sent by a guest.
pub(crate) async fn restore_app(
    state: &AppState,
    instance: &str,
) -> Result<Option<serde_json::Value>, String> {
    if state
        .mcp_app_bridge(instance)
        .is_some_and(|b| b.server.is_connected())
    {
        return Ok(None);
    }
    let frame = mcp_app_frame_id(instance)?;
    let project = state
        .store
        .frame_project_id(frame)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("MCP conversation was deleted")?;
    let events = state
        .store
        .load_session_ui_events(frame)
        .await
        .map_err(|e| e.to_string())?;
    let mut payload = events
        .into_iter()
        .rev()
        .filter_map(|s| serde_json::from_str::<AgentEvent>(&s).ok())
        .find_map(|e| {
            if let AgentEvent::ToolPresentation {
                payload,
                presentation_kind,
                ..
            } = e
            {
                if presentation_kind == "mcp_app"
                    && crate::mcp_app_instance_id(frame, &payload) == instance
                {
                    return Some(payload);
                }
            }
            None
        });
    let Some(mut payload) = payload.take() else {
        return Ok(None);
    };
    let Some(raw_binding) = payload.get("_wispMcpBinding").cloned() else {
        return Ok(None);
    };
    let binding: wisp_dto::McpAppBinding =
        serde_json::from_value(raw_binding).map_err(|_| "Invalid Host binding")?;
    if binding.version != 1 || binding.frame_id != frame || binding.project_id != project {
        return Err("MCP App binding does not match this conversation".into());
    }
    let allow = specialists::session_specialist(&state.store, frame)
        .await
        .and_then(|s| s.connectors);
    if allow
        .as_ref()
        .is_some_and(|a| !a.contains(&binding.connector_id))
    {
        return Err("MCP connector not granted to this conversation".into());
    }
    let (specs, _) = configured(&state.store, &project).await;
    let spec = specs
        .iter()
        .find(|s| s.id() == binding.connector_id)
        .ok_or("MCP connector is disabled or removed")?;
    let scope = state
        .store
        .frame_state_scope(frame)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("MCP conversation scope missing")?;
    let client = host()
        .acquire(&state.store, &project, frame, scope.scope_key(), spec)
        .await?;
    let catalog = Arc::new(client.tools_list().await.map_err(|e| e.to_string())?);
    let name = payload
        .pointer("/tool/name")
        .and_then(serde_json::Value::as_str)
        .ok_or("Historical App tool missing")?;
    let tool = catalog
        .iter()
        .find(|t| t.name == name && t.visible_to_model())
        .ok_or("MCP App tool is no longer available")?;
    let uri = tool
        .ui_resource_uri()
        .ok_or("MCP tool no longer provides an App")?;
    if payload
        .pointer("/resource/uri")
        .and_then(serde_json::Value::as_str)
        != Some(uri)
    {
        return Err("MCP App resource changed; open the updated plugin App".into());
    }
    let resource = client.resource_read(uri).await.map_err(|e| e.to_string())?;
    let resource = resource["contents"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|r| {
                r["uri"].as_str() == Some(uri)
                    && r["mimeType"]
                        .as_str()
                        .is_some_and(|m| m.starts_with("text/html"))
            })
        })
        .cloned()
        .ok_or("MCP App resource is unavailable")?;
    if resource["text"]
        .as_str()
        .is_none_or(|s| s.len() > 32 * 1024 * 1024)
    {
        return Err("MCP App resource is invalid or oversized".into());
    }
    payload["tool"] = serde_json::to_value(tool).map_err(|e| e.to_string())?;
    payload["resource"] = resource;
    payload["_wispHistoricalResult"] = true.into();
    let server = wisp_mcp::tool::McpAppServerHandle::new(
        binding.connector_id,
        tool.display_title(),
        catalog.clone(),
        Arc::downgrade(&client),
        spec.plugin_id().is_some(),
    );
    state.register_mcp_app_bridge(
        instance.to_string(),
        McpAppToolBridge {
            generation: 0,
            frame_id: frame.into(),
            server: Arc::new(server),
            limiter: McpAppCallLimiter::new(),
        },
    );
    Ok(Some(payload))
}

#[tauri::command]
pub(crate) async fn prepare_mcp_app(
    state: State<'_, AppState>,
    window: workspace_surface::WorkspaceSurface,
    instance_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let frame = mcp_app_frame_id(&instance_id)?;
    if state.active_frame(window.label()).as_deref() != Some(frame) {
        return Err("stale-instance".into());
    }
    let result = restore_app(&state, &instance_id).await?;
    if state.active_frame(window.label()).as_deref() != Some(frame) {
        return Err("stale-instance".into());
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn restart_session_mcp(
    state: State<'_, AppState>,
    window: workspace_surface::WorkspaceSurface,
    instance_id: String,
    confirm_outcome_unknown: bool,
) -> Result<(), String> {
    let frame = mcp_app_frame_id(&instance_id)?;
    if !confirm_outcome_unknown || state.active_frame(window.label()).as_deref() != Some(frame) {
        return Err("Explicit confirmation in the active conversation is required".into());
    }
    state.remove_mcp_app_bridges_for_frame(frame);
    crate::mcp_broker::cancel_frame(frame);
    host().restart_frame(frame).await;
    crate::clear_session_agent(&state, frame).await;
    Ok(())
}

pub(crate) async fn restore(store: &Store, project: &str, frame: &str) {
    let Ok(Some(scope)) = store.frame_state_scope(frame).await else {
        return;
    };
    let (specs, _) = configured(store, project).await;
    let mut tasks = tokio::task::JoinSet::new();
    let allow = specialists::session_specialist(store, frame)
        .await
        .and_then(|s| s.connectors);
    for spec in specs {
        if allow
            .as_ref()
            .is_some_and(|a| !a.iter().any(|id| id == spec.id()))
        {
            continue;
        }
        let Ok(client) = host()
            .acquire(store, project, frame, scope.scope_key(), &spec)
            .await
        else {
            continue;
        };
        tasks.spawn(async move {
            let _ = client.tools_list().await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State as FixtureState, http::StatusCode, response::IntoResponse, routing::post,
        Json, Router,
    };

    #[derive(Default)]
    struct LifecycleFixture {
        starts: std::sync::atomic::AtomicUsize,
        stops: std::sync::atomic::AtomicUsize,
        hold_initialize: AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    async fn lifecycle_fixture() -> (
        PathBuf,
        Store,
        Spec,
        Arc<LifecycleFixture>,
        tokio::task::JoinHandle<()>,
    ) {
        let root = std::env::temp_dir().join(format!("wisp-mcp-lifecycle-{}", Uuid::new_v4()));
        let store = Store::open(&root.join("wisp.sqlite")).await.unwrap();
        store
            .create_project("p", "test", &root.to_string_lossy())
            .await
            .unwrap();
        for frame in ["a", "b"] {
            store
                .create_frame(frame, "p", "test", "model")
                .await
                .unwrap();
        }
        let fixture = Arc::new(LifecycleFixture::default());
        let router = Router::new()
            .route(
                "/",
                post(
                    |FixtureState(f): FixtureState<Arc<LifecycleFixture>>,
                     Json(rpc): Json<serde_json::Value>| async move {
                        let result = match rpc["method"].as_str().unwrap_or("") {
                            "initialize" => {
                                f.starts.fetch_add(1, Ordering::SeqCst);
                                f.entered.notify_one();
                                if f.hold_initialize.load(Ordering::SeqCst) {
                                    f.release.notified().await;
                                }
                                json!({"capabilities":{"tools":{}}})
                            }
                            "tools/list" => json!({"tools":[]}),
                            "notifications/initialized" | "notifications/cancelled" => {
                                return StatusCode::ACCEPTED.into_response()
                            }
                            _ => json!({"content":[{"type":"text","text":"ok"}]}),
                        };
                        (
                            [("mcp-session-id", "fixture")],
                            Json(json!({"jsonrpc":"2.0","id":rpc["id"],"result":result})),
                        )
                            .into_response()
                    },
                )
                .delete(
                    |FixtureState(f): FixtureState<Arc<LifecycleFixture>>| async move {
                        f.stops.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    },
                ),
            )
            .with_state(fixture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let conn = McpConnection {
            id: "fixture".into(),
            name: "test".into(),
            enabled: true,
            transport: McpTransport::Http {
                url: format!("http://{}", listener.local_addr().unwrap()),
                headers: vec![],
                auth: McpHttpAuth::None,
            },
        };
        save_mcp_connections(&store, &[conn.clone()]).await.unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (root, store, Spec::Custom(conn), fixture, server)
    }

    #[tokio::test]
    async fn deleting_frame_closes_connections_and_fences_late_restore_without_affecting_peers() {
        let (root, store, spec, fixture, server) = lifecycle_fixture().await;
        let owner = Connections::default();
        let a = owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .unwrap();
        let b = owner
            .acquire(&store, "p", "b", "main", &spec)
            .await
            .unwrap();
        a.tools_list().await.unwrap();
        b.tools_list().await.unwrap();
        owner.retire_frame("a").await;
        assert!(!a.is_connected());
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 1);
        assert_eq!(b.tool_call("echo", &json!({})).await.unwrap(), "ok");
        // Retirement precedes the SQLite delete: even an already-read restore
        // snapshot must be refused while the frame still exists in the store.
        assert!(owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .is_err());
        assert!(a.tools_list().await.is_err());
        store.delete_session("a", "p").await.unwrap();
        assert!(owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .is_err());
        assert_eq!(owner.entries.lock().await.len(), 1);
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 2);
        owner.shutdown_all().await;
        server.abort();
        drop((a, b, owner, store));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn deletion_interrupts_in_progress_initialization() {
        let (root, store, spec, fixture, server) = lifecycle_fixture().await;
        fixture.hold_initialize.store(true, Ordering::SeqCst);
        let owner = Connections::default();
        let client = owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .unwrap();
        let connecting = tokio::spawn(async move { client.tools_list().await });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.entered.notified(),
        )
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), owner.retire_frame("a"))
            .await
            .unwrap();
        assert!(connecting.await.unwrap().is_err());
        fixture.release.notify_one();
        assert!(owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .is_err());
        assert!(owner.entries.lock().await.is_empty());
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 1);
        server.abort();
        drop((owner, store));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reconcile_removes_deleted_frames_and_unstarted_handles_cannot_reconnect() {
        let (root, store, spec, fixture, server) = lifecycle_fixture().await;
        let owner = Connections::default();
        let a = owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .unwrap();
        let b = owner
            .acquire(&store, "p", "b", "main", &spec)
            .await
            .unwrap();
        a.tools_list().await.unwrap();
        store.delete_session("a", "p").await.unwrap();
        store.delete_session("b", "p").await.unwrap();
        // A previously acquired handle must check durable ownership on launch.
        assert!(b.tools_list().await.is_err());
        owner.reconcile(&store, None).await;
        assert!(!a.is_connected());
        assert!(owner.entries.lock().await.is_empty());
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 1);
        server.abort();
        drop((a, b, owner, store));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn owner_reuses_only_same_scope_and_disable_revokes_old_handles() {
        let root = std::env::temp_dir().join(format!("wisp-mcp-owner-{}", Uuid::new_v4()));
        let store = Store::open(&root.join("wisp.sqlite")).await.unwrap();
        store
            .create_project("p", "test", &root.to_string_lossy())
            .await
            .unwrap();
        for frame in ["a", "b"] {
            store
                .create_frame(frame, "p", "test", "model")
                .await
                .unwrap();
        }
        let spec = Spec::Custom(McpConnection {
            id: "fixture".into(),
            name: "test".into(),
            enabled: true,
            transport: McpTransport::Http {
                url: "http://127.0.0.1:1".into(),
                headers: vec![],
                auth: McpHttpAuth::None,
            },
        });
        let Spec::Custom(conn) = &spec else {
            unreachable!()
        };
        save_mcp_connections(&store, &[conn.clone()]).await.unwrap();
        let owner = Connections::default();
        let a = owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .unwrap();
        let b = owner
            .acquire(&store, "p", "b", "main", &spec)
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(
            &a,
            &owner
                .acquire(&store, "p", "a", "main", &spec)
                .await
                .unwrap()
        ));
        owner.reconcile(&store, None).await;
        assert!(
            Arc::ptr_eq(
                &a,
                &owner
                    .acquire(&store, "p", "a", "main", &spec)
                    .await
                    .unwrap()
            ),
            "agent rebuild must retain unchanged connections"
        );
        let mut disabled = conn.clone();
        disabled.enabled = false;
        save_mcp_connections(&store, &[disabled]).await.unwrap();
        owner.reconcile(&store, None).await;
        assert!(owner.entries.lock().await.is_empty());
        assert!(a
            .tools_list()
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled"));
        // A racing stale wiring snapshot cannot relaunch an explicitly disabled plugin.
        let stale = owner
            .acquire(&store, "p", "a", "main", &spec)
            .await
            .unwrap();
        assert!(stale
            .tools_list()
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled"));
        owner.shutdown_all().await;
        drop((a, b, stale, owner, store));
        let _ = std::fs::remove_dir_all(root);
    }
}
