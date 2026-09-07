use super::{
    bound_window_project_id, clear_idle_agents, connect_mcp, domain_display_name,
    load_approval_scope_for, load_disabled_connectors, load_mcp_connections,
    load_skip_connectors_for, load_tool_approvals_for, persist_approval_scope_overlay,
    persist_skip_connectors_overlay, persist_tool_approval_overlay, refresh_approval_policy_for,
    save_json_setting, save_mcp_connections, window_bound_project_id, AppState, McpConnection,
    McpHttpAuth, McpTransport,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use tauri::{State, WebviewWindow};

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
    let mut conn = conn;
    crate::mcp_secrets::persist_connection_secrets(&mut conn, None)?;
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
    let mut conn = conn;
    let previous = conns.iter().find(|c| c.id == conn.id).cloned();
    crate::mcp_secrets::persist_connection_secrets(&mut conn, previous.as_ref())?;
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
    if let Some(removed) = conns.iter().find(|connection| connection.id == id) {
        crate::mcp_secrets::forget_connection_secrets(removed);
    }
    conns.retain(|c| c.id != id);
    save_mcp_connections(&state.store, &conns).await?;
    crate::mcp_oauth::forget(&id);
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

pub(super) use wisp_dto::ConnectorsView;
use wisp_dto::{ConnectorInfo, ConnectorLink, ConnectorTool};

fn bundled_connector_infos(
    disabled: &HashSet<String>,
    approvals: &HashMap<String, String>,
    skip: &HashSet<String>,
) -> Vec<ConnectorInfo> {
    let mut domains = BTreeMap::<String, Vec<wisp_llm::ToolSchema>>::new();
    for (domain, schema) in wisp_bio::catalog() {
        domains.entry(domain.into()).or_default().push(schema);
    }
    domains
        .into_iter()
        .map(|(slug, schemas)| {
            let skip_on = skip.contains(&slug);
            let metadata = wisp_bio::domain_metadata(&slug);
            let tools = schemas
                .into_iter()
                .map(|schema| {
                    let function = schema.function;
                    ConnectorTool {
                        mode: if skip_on {
                            "allow".into()
                        } else {
                            approvals
                                .get(&function.name)
                                .cloned()
                                .unwrap_or_else(|| "allow".into())
                        },
                        name: function.name,
                        description: function.description,
                        input_schema: Some(function.parameters),
                        output_schema: None,
                    }
                })
                .collect();
            ConnectorInfo {
                enabled: !disabled.contains(&slug),
                name: domain_display_name(&slug),
                key: slug,
                kind: "bundled".into(),
                skip_approvals: skip_on,
                transport: String::new(),
                subtitle: String::new(),
                auth: String::new(),
                description: metadata.map(|m| m.description.clone()).unwrap_or_default(),
                description_zh: metadata
                    .map(|m| m.description_zh.clone())
                    .unwrap_or_default(),
                maintainer: "Wisp Science".into(),
                links: metadata
                    .map(|m| {
                        m.links
                            .iter()
                            .map(|link| ConnectorLink {
                                label: link.label.clone(),
                                url: link.url.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                tools,
            }
        })
        .collect()
}

#[tauri::command]
pub(super) async fn list_connectors(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<ConnectorsView, String> {
    let store = &state.store;
    // Unbound windows show the inherited global defaults without writing them.
    let project_id = window_bound_project_id(&state, window.label());
    let disabled = load_disabled_connectors(store).await;
    let approvals = load_tool_approvals_for(store, project_id.as_deref()).await;
    let skip = load_skip_connectors_for(store, project_id.as_deref()).await;

    let mut connectors = bundled_connector_infos(&disabled, &approvals, &skip);
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
            description: String::new(),
            description_zh: String::new(),
            maintainer: String::new(),
            links: vec![],
            tools: vec![],
        });
    }
    let scope = load_approval_scope_for(store, project_id.as_deref())
        .await
        .as_str()
        .to_string();
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
/// live on the next tool call — no session rebuild needed. Writes the overlay
/// for this window's project so a sibling window keeps its own policy.
#[tauri::command]
pub(super) async fn set_tool_approval(
    state: State<'_, AppState>,
    window: WebviewWindow,
    tool: String,
    mode: String,
) -> Result<(), String> {
    let project_id = persist_tool_approval_overlay(
        &state.store,
        bound_window_project_id(&state, window.label()),
        tool,
        mode,
    )
    .await?;
    refresh_approval_policy_for(&state, Some(&project_id)).await;
    Ok(())
}

/// Set the approval scope ("full" | "auto" | "ask") for this window's project.
#[tauri::command]
pub(super) async fn set_approval_scope(
    state: State<'_, AppState>,
    window: WebviewWindow,
    scope: String,
) -> Result<(), String> {
    let project_id = persist_approval_scope_overlay(
        &state.store,
        bound_window_project_id(&state, window.label()),
        &scope,
    )
    .await?;
    refresh_approval_policy_for(&state, Some(&project_id)).await;
    Ok(())
}

/// Toggle "Skip approvals" for a connector (force-allow all its tools).
#[tauri::command]
pub(super) async fn set_connector_skip_approvals(
    state: State<'_, AppState>,
    window: WebviewWindow,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let project_id = persist_skip_connectors_overlay(
        &state.store,
        bound_window_project_id(&state, window.label()),
        key,
        enabled,
    )
    .await?;
    refresh_approval_policy_for(&state, Some(&project_id)).await;
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
            auth: McpHttpAuth::OAuth,
            ..
        } if !url.trim().is_empty() => Ok((
            url.trim().to_string(),
            crate::mcp_secrets::hydrate_headers(connection),
        )),
        _ => Err("OAuth authorization requires a remote URL connection".into()),
    }
}

/// The saved connection's OAuth URL, if `id` names a stored OAuth connection.
fn saved_oauth_url(connections: &[McpConnection], id: &str) -> Option<String> {
    connections
        .iter()
        .find(|connection| connection.id == id)
        .and_then(|connection| oauth_http_config(connection).ok())
        .map(|(url, _)| url)
}

/// An existing credential is reused when the saved OAuth URL is unchanged;
/// metadata edits (name, headers, enabled) then skip the browser round-trip.
fn can_reuse_credential(connections: &[McpConnection], conn: &McpConnection, url: &str) -> bool {
    crate::mcp_oauth::has_credential(&conn.id)
        && saved_oauth_url(connections, &conn.id).as_deref() == Some(url)
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

/// List an OAuth URL's tools. Reuses the connection's stored credential when
/// its saved URL is unchanged; otherwise authorizes with an ephemeral
/// credential that is removed afterwards, without saving the connection.
#[tauri::command]
pub(super) async fn test_oauth_mcp_connection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<Vec<wisp_mcp::RemoteTool>, String> {
    let (resource_url, headers) = oauth_http_config(&conn)?;
    let connections = load_mcp_connections(&state.store).await;
    if can_reuse_credential(&connections, &conn, &resource_url) {
        let client = crate::mcp_oauth::connect(&conn.id, &resource_url, &headers)
            .await
            .map_err(|error| error.to_string())?;
        return client.tools_list().await.map_err(|error| error.to_string());
    }
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

    let mut connections = load_mcp_connections(&state.store).await;
    if !can_reuse_credential(&connections, &conn, &resource_url) {
        authorize_in_browser(&app, &resource_url, &conn.id).await?;
        // Authorization can take minutes; reload so concurrent edits survive.
        connections = load_mcp_connections(&state.store).await;
    }
    let mut conn = conn;
    let previous = connections.iter().find(|item| item.id == conn.id).cloned();
    crate::mcp_secrets::persist_connection_secrets(&mut conn, previous.as_ref())?;
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

/// Cancel the in-flight OAuth authorization started by Test or Save.
#[tauri::command]
pub(super) fn cancel_oauth_authorization() {
    crate::mcp_oauth::cancel_authorization();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_connector_details_preserve_catalog_contracts_and_approval_overrides() {
        let disabled = HashSet::from(["biomart".into()]);
        let approvals = HashMap::from([("list_marts".into(), "deny".into())]);
        let connectors = bundled_connector_infos(&disabled, &approvals, &HashSet::new());
        let serialized = serde_json::to_value(&connectors).unwrap();
        let ui: Vec<wisp_dto::ConnectorInfo> = serde_json::from_value(serialized).unwrap();
        let biomart = ui.iter().find(|c| c.key == "biomart").unwrap();
        assert!(!biomart.enabled);
        assert!(!biomart.description.is_empty());
        assert!(!biomart.description_zh.is_empty());
        assert_eq!(biomart.maintainer, "Wisp Science");
        assert!(biomart
            .links
            .iter()
            .any(|link| link.url.contains("ensembl.org")));
        assert_eq!(biomart.tools[0].name, "list_marts");
        assert_eq!(biomart.tools[0].mode, "deny");
        for (domain, schema) in wisp_bio::catalog() {
            let connector = ui.iter().find(|c| c.key == domain).unwrap();
            let tool = connector
                .tools
                .iter()
                .find(|t| t.name == schema.function.name)
                .unwrap();
            assert_eq!(tool.description, schema.function.description);
            assert_eq!(
                tool.input_schema.as_ref(),
                Some(&schema.function.parameters)
            );
            assert!(tool.output_schema.is_none());
        }
        let skipped =
            bundled_connector_infos(&disabled, &approvals, &HashSet::from(["biomart".into()]));
        assert!(skipped
            .iter()
            .find(|c| c.key == "biomart")
            .unwrap()
            .tools
            .iter()
            .all(|tool| tool.mode == "allow"));
    }

    #[test]
    fn saved_oauth_url_matches_only_oauth_connections() {
        let connections = vec![
            McpConnection {
                id: "oauth".into(),
                name: "Remote".into(),
                enabled: true,
                transport: McpTransport::Http {
                    url: " https://example.com/mcp ".into(),
                    headers: vec![],
                    auth: McpHttpAuth::OAuth,
                },
            },
            McpConnection {
                id: "plain".into(),
                name: "Plain".into(),
                enabled: true,
                transport: McpTransport::Http {
                    url: "https://example.com/mcp".into(),
                    headers: vec![],
                    auth: McpHttpAuth::None,
                },
            },
        ];
        assert_eq!(
            saved_oauth_url(&connections, "oauth").as_deref(),
            Some("https://example.com/mcp")
        );
        assert_eq!(saved_oauth_url(&connections, "plain"), None);
        assert_eq!(saved_oauth_url(&connections, "missing"), None);
    }

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
