use super::{
    bio_domains, clear_idle_agents, connect_mcp, load_approval_scope, load_disabled_connectors,
    load_mcp_connections, load_skip_connectors, load_tool_approvals, refresh_approval_policy,
    save_json_setting, save_mcp_connections, AppState, ApprovalMode, McpConnection, McpHttpAuth,
    McpTransport, Scope,
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
    if is_oauth_http(&conn) {
        return Err("OAuth connections must be authorized before saving".into());
    }
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
    if is_oauth_http(&conn) {
        return Err("OAuth connections must be authorized before saving".into());
    }
    let connection_id = conn.id.clone();
    let removed_oauth = match conns.iter_mut().find(|c| c.id == conn.id) {
        Some(slot) => {
            let removed_oauth = is_oauth_http(slot);
            *slot = conn;
            removed_oauth
        }
        None => return Err("connection not found".into()),
    };
    save_mcp_connections(&state.store, &conns).await?;
    if removed_oauth {
        crate::mcp_oauth::forget(&connection_id);
    }
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn delete_mcp_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    let removed_oauth = conns
        .iter()
        .any(|connection| connection.id == id && is_oauth_http(connection));
    conns.retain(|c| c.id != id);
    save_mcp_connections(&state.store, &conns).await?;
    if removed_oauth {
        crate::mcp_oauth::forget(&id);
    }
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
    /// "none" | "oauth" for remote HTTP connectors; empty otherwise.
    auth: String,
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
            auth: String::new(),
            tools,
        });
    }
    for c in load_mcp_connections(store).await {
        let (transport, subtitle, auth) = match &c.transport {
            McpTransport::Stdio { command, .. } => ("stdio", command.clone(), String::new()),
            McpTransport::Http { url, auth, .. } => ("http", url.clone(), auth.as_str().into()),
        };
        connectors.push(ConnectorInfo {
            key: c.id,
            name: c.name,
            kind: "custom".into(),
            enabled: c.enabled,
            skip_approvals: false,
            transport: transport.into(),
            subtitle,
            auth,
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

fn is_oauth_http(connection: &McpConnection) -> bool {
    matches!(
        &connection.transport,
        McpTransport::Http {
            auth: McpHttpAuth::OAuth,
            ..
        }
    )
}

fn oauth_http_config(
    connection: &McpConnection,
) -> Result<(String, Vec<(String, String)>), String> {
    match &connection.transport {
        McpTransport::Http {
            url,
            headers,
            auth: McpHttpAuth::OAuth,
        } if !url.trim().is_empty() => Ok((url.trim().to_string(), headers.clone())),
        _ => Err("OAuth authorization requires a remote URL connection".into()),
    }
}

async fn authorize_in_browser(
    app: &tauri::AppHandle,
    resource_url: &str,
    credential_id: &str,
) -> Result<(), String> {
    let (listener, pending) = crate::mcp_oauth::begin_authorization(resource_url)
        .await
        .map_err(|error| error.to_string())?;
    let authorization_url = pending.authorization_url().to_string();
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(&authorization_url, None::<&str>)
            .map_err(|error| format!("open MCP authorization page: {error}"))?;
    }
    crate::mcp_oauth::finish_authorization(listener, pending, credential_id)
        .await
        .map_err(|error| error.to_string())
}

/// Authorize an OAuth URL with an ephemeral credential, list its tools, then
/// remove the credential without saving the connection.
#[tauri::command]
pub(super) async fn test_oauth_mcp_connection(
    app: tauri::AppHandle,
    conn: McpConnection,
) -> Result<Vec<wisp_mcp::RemoteTool>, String> {
    let (resource_url, headers) = oauth_http_config(&conn)?;
    let credential_id = format!("oauth-test-{}", uuid::Uuid::new_v4());
    let result = async {
        authorize_in_browser(&app, &resource_url, &credential_id).await?;
        let client = crate::mcp_oauth::connect(&credential_id, &resource_url, &headers)
            .await
            .map_err(|error| error.to_string())?;
        client.tools_list().await.map_err(|error| error.to_string())
    }
    .await;
    crate::mcp_oauth::forget(&credential_id);
    result
}

/// Authorize and save an OAuth-backed remote URL connection.
#[tauri::command]
pub(super) async fn authorize_http_connection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<(), String> {
    let (resource_url, _) = oauth_http_config(&conn)?;
    let connection_id = conn.id.clone();
    let had_credential = crate::mcp_oauth::has_credential(&conn.id);

    authorize_in_browser(&app, &resource_url, &conn.id).await?;

    let mut connections = load_mcp_connections(&state.store).await;
    if let Some(existing) = connections.iter().position(|item| item.id == conn.id) {
        connections[existing] = conn;
    } else {
        connections.push(conn);
    }
    if let Err(error) = save_mcp_connections(&state.store, &connections).await {
        if !had_credential {
            crate::mcp_oauth::forget(&connection_id);
        }
        return Err(error);
    }
    clear_idle_agents(&state).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_oauth_http_connections() {
        let oauth = McpConnection {
            id: "remote".into(),
            name: "Remote".into(),
            enabled: true,
            transport: McpTransport::Http {
                url: "https://example.com/mcp".into(),
                headers: vec![],
                auth: McpHttpAuth::OAuth,
            },
        };
        assert!(is_oauth_http(&oauth));
        let (url, headers) = oauth_http_config(&oauth).unwrap();
        assert_eq!(url, "https://example.com/mcp");
        assert!(headers.is_empty());

        let plain = McpConnection {
            transport: McpTransport::Http {
                url: "https://example.com/mcp".into(),
                headers: vec![],
                auth: McpHttpAuth::None,
            },
            ..oauth
        };
        assert!(!is_oauth_http(&plain));
        assert!(oauth_http_config(&plain).is_err());
    }
}
