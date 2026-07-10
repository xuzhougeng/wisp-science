use super::{
    bio_domains, clear_idle_agents, connect_mcp, load_approval_scope, load_disabled_connectors,
    load_mcp_connections, load_skip_connectors, load_tool_approvals, refresh_approval_policy,
    save_json_setting, save_mcp_connections, AppState, ApprovalMode, McpConnection, McpTransport,
    Scope,
};
use serde::Serialize;
use tauri::State;

#[derive(Serialize, Clone)]
pub(super) struct McpConnectionsView {
    connections: Vec<McpConnection>,
}

#[tauri::command]
pub(super) async fn list_mcp_connections(
    state: State<'_, AppState>,
) -> Result<McpConnectionsView, String> {
    Ok(McpConnectionsView {
        connections: load_mcp_connections(&state.store).await,
    })
}

#[tauri::command]
pub(super) async fn add_mcp_connection(
    state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.push(conn);
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn update_mcp_connection(
    state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    match conns.iter_mut().find(|c| c.id == conn.id) {
        Some(slot) => *slot = conn,
        None => return Err("connection not found".into()),
    }
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn delete_mcp_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.retain(|c| c.id != id);
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn set_mcp_connection_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    if let Some(c) = conns.iter_mut().find(|c| c.id == id) {
        c.enabled = enabled;
    }
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

// ── Connectors tree (multi-level Connections UI) ────────────────────────────

#[derive(Serialize, Clone)]
struct ConnectorTool {
    name: String,
    /// Effective approval mode: "allow" | "ask" | "deny".
    mode: String,
}

#[derive(Serialize, Clone)]
struct ConnectorInfo {
    /// Domain slug (bundled) or connection id (custom).
    key: String,
    name: String,
    /// "bundled" | "custom".
    kind: String,
    enabled: bool,
    skip_approvals: bool,
    /// "stdio" | "http" for custom connectors; empty for bundled.
    transport: String,
    /// Command/URL line for custom connectors; empty for bundled.
    subtitle: String,
    /// Tools for bundled connectors (static from domains.json). Custom
    /// connector tools are loaded on demand through `test_mcp_connection`.
    tools: Vec<ConnectorTool>,
}

#[derive(Serialize, Clone)]
pub(super) struct ConnectorsView {
    connectors: Vec<ConnectorInfo>,
    /// Global approval scope ("full" | "auto" | "ask").
    scope: String,
}

#[tauri::command]
pub(super) async fn list_connectors(state: State<'_, AppState>) -> Result<ConnectorsView, String> {
    let store = &state.store;
    let disabled = load_disabled_connectors(store).await;
    let approvals = load_tool_approvals(store).await;
    let skip = load_skip_connectors(store).await;

    let mut connectors = vec![];
    for d in bio_domains() {
        let skip_on = skip.contains(&d.slug);
        let tools = d
            .tools
            .iter()
            .map(|t| ConnectorTool {
                mode: if skip_on {
                    "allow".into()
                } else {
                    approvals.get(t).cloned().unwrap_or_else(|| "allow".into())
                },
                name: t.clone(),
            })
            .collect();
        connectors.push(ConnectorInfo {
            enabled: !disabled.contains(&d.slug),
            key: d.slug,
            name: d.name,
            kind: "bundled".into(),
            skip_approvals: skip_on,
            transport: String::new(),
            subtitle: String::new(),
            tools,
        });
    }
    for c in load_mcp_connections(store).await {
        let (transport, subtitle) = match &c.transport {
            McpTransport::Stdio { command, .. } => ("stdio", command.clone()),
            McpTransport::Http { url, .. } => ("http", url.clone()),
        };
        connectors.push(ConnectorInfo {
            key: c.id,
            name: c.name,
            kind: "custom".into(),
            enabled: c.enabled,
            skip_approvals: false,
            transport: transport.into(),
            subtitle,
            tools: vec![],
        });
    }
    let scope = load_approval_scope(store).await.as_str().to_string();
    Ok(ConnectorsView { connectors, scope })
}

/// Enable/disable a bundled connector (domain). Custom connectors use
/// `set_mcp_connection_enabled` instead.
#[tauri::command]
pub(super) async fn set_connector_enabled(
    state: State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let mut disabled = load_disabled_connectors(&state.store).await;
    if enabled {
        disabled.remove(&key);
    } else {
        disabled.insert(key);
    }
    let list: Vec<String> = disabled.into_iter().collect();
    save_json_setting(&state.store, "disabled_connectors", &list).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

/// Set the approval mode ("allow" | "ask" | "deny") for a single tool. Enforced
/// live on the next tool call — no session rebuild needed.
#[tauri::command]
pub(super) async fn set_tool_approval(
    state: State<'_, AppState>,
    tool: String,
    mode: String,
) -> Result<(), String> {
    let mut approvals = load_tool_approvals(&state.store).await;
    // Store only overrides; "allow" is the default, so drop it to stay compact.
    if ApprovalMode::parse(&mode) == ApprovalMode::Allow {
        approvals.remove(&tool);
    } else {
        approvals.insert(tool, ApprovalMode::parse(&mode).as_str().into());
    }
    save_json_setting(&state.store, "tool_approvals", &approvals).await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

/// Set the global approval scope ("full" | "auto" | "ask"). Enforced live on
/// the next tool call — no session rebuild needed.
#[tauri::command]
pub(super) async fn set_approval_scope(
    state: State<'_, AppState>,
    scope: String,
) -> Result<(), String> {
    // Normalize through `Scope` so only the three valid values ever persist.
    save_json_setting(
        &state.store,
        "approval_scope",
        &Scope::parse(&scope).as_str(),
    )
    .await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

/// Toggle "Skip approvals" for a connector (force-allow all its tools).
#[tauri::command]
pub(super) async fn set_connector_skip_approvals(
    state: State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let mut skip = load_skip_connectors(&state.store).await;
    if enabled {
        skip.insert(key);
    } else {
        skip.remove(&key);
    }
    let list: Vec<String> = skip.into_iter().collect();
    save_json_setting(&state.store, "skip_approval_connectors", &list).await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn test_mcp_connection(
    _state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<Vec<wisp_mcp::RemoteTool>, String> {
    let client = connect_mcp(&conn).await.map_err(|e| format!("{e}"))?;
    let tools = client.tools_list().await.map_err(|e| format!("{e}"))?;
    Ok(tools)
}
