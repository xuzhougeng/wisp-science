//! SSH host registry Tauri commands. Core types live in [`wisp_runs::ssh_hosts`].

pub use wisp_runs::ssh_hosts::*;

use tauri::State;

#[tauri::command]
pub async fn set_default_execution_context(
    state: State<'_, crate::AppState>,
    context_id: Option<String>,
) -> Result<Option<String>, String> {
    match context_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        Some(id) => {
            match state.store.get_execution_context(&id).await {
                Ok(Some(ctx)) if ctx.kind != wisp_store::ExecutionContextKind::Local => {}
                Ok(Some(_)) => {
                    return Err("Local compute is always available; no default needed".into())
                }
                Ok(None) => return Err(format!("Execution context not found: {id}")),
                Err(e) => return Err(e.to_string()),
            }
            state
                .store
                .set_setting(DEFAULT_EXECUTION_CONTEXT_KEY, &id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Some(id))
        }
        None => {
            state
                .store
                .set_setting(DEFAULT_EXECUTION_CONTEXT_KEY, "")
                .await
                .map_err(|e| e.to_string())?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn get_default_execution_context(
    state: State<'_, crate::AppState>,
) -> Result<Option<String>, String> {
    Ok(stored_default_execution_context(&state.store).await)
}

#[tauri::command]
pub async fn get_session_default_execution_context(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    Ok(
        stored_session_default_execution_context(&state.store, &session_id)
            .await
            .stored_value()
            .map(str::to_string),
    )
}

#[tauri::command]
pub async fn set_session_default_execution_context(
    state: State<'_, crate::AppState>,
    session_id: String,
    context_id: Option<String>,
) -> Result<Option<String>, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, &session_id).await?;
    let _activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let value = match context_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "local")
    {
        Some(id) => {
            match state.store.get_execution_context(id).await {
                Ok(Some(ctx)) if ctx.kind != wisp_store::ExecutionContextKind::Local => {}
                Ok(Some(_)) => {
                    return Err("Local compute is always available; no default needed".into())
                }
                Ok(None) => return Err(format!("Execution context not found: {id}")),
                Err(e) => return Err(e.to_string()),
            }
            SessionDefaultExecutionContext::Remote(id.to_string())
        }
        None => SessionDefaultExecutionContext::Local,
    };
    persist_session_default_execution_context(&state.store, &session_id, value)
        .await
        .map(|saved| saved.stored_value().map(str::to_string))
}

#[tauri::command]
pub async fn list_ssh_hosts(state: State<'_, crate::AppState>) -> Result<Vec<SshHost>, String> {
    Ok(load(&state.store)
        .await
        .into_iter()
        .map(decorate_host)
        .collect())
}

/// Trust edges are created by the agent's `configure_ssh_trust` tool; these
/// two commands are the user's window into them: see every persisted edge and
/// revoke one (record removal plus best-effort managed-key cleanup).
#[tauri::command]
pub async fn list_ssh_trust_edges(
    state: State<'_, crate::AppState>,
) -> Result<Vec<crate::run_context::SshTrustEdge>, String> {
    Ok(crate::run_context::load_trust_edges(&state.store).await)
}

#[tauri::command]
pub async fn revoke_ssh_trust_edge(
    state: State<'_, crate::AppState>,
    source_context_id: String,
    destination_context_id: String,
) -> Result<crate::run_context::RevokeTrustResponse, String> {
    crate::run_context::revoke_trust_edge(
        &state.store,
        &state.run_manager,
        &source_context_id,
        &destination_context_id,
    )
    .await
}

#[tauri::command]
pub async fn list_session_execution_context_ids(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Vec<String>, String> {
    state
        .store
        .list_session_execution_context_ids(&session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_session_execution_context_enabled(
    state: State<'_, crate::AppState>,
    session_id: String,
    context_id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, &session_id).await?;
    let _activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    state
        .store
        .set_session_execution_context_enabled(&session_id, &context_id, enabled)
        .await
        .map_err(|error| error.to_string())?;
    state
        .store
        .list_session_execution_context_ids(&session_id)
        .await
        .map_err(|error| error.to_string())
}

/// One-shot connectivity check for the host editor, using the form's current
/// (possibly unsaved) values. Deliberately bypasses the ssh_guard gate (this
/// is the user's diagnostic tool) and the master pool (a fresh connection is
/// the point); a success clears any guard block for the alias.
#[tauri::command]
pub async fn test_ssh_connection(host: SshHost) -> Result<(), String> {
    let connection = SshConnection::from_host(&host)?;
    let envs = if connection.uses_password() {
        match host
            .password
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(password) => build_password_askpass_env(password)?,
            None => connection.password_auth_env()?,
        }
    } else {
        connection.assert_ready_to_connect()?;
        Vec::new()
    };
    let mut args = connection.ssh_args()?;
    args.push("echo __WISP_SSH_OK__".into());
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if !envs.is_empty() {
        cmd.envs(envs.iter().cloned());
    }
    wisp_tools::process::hide_console_async(&mut cmd);
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;
    cleanup_password_auth_env(&envs);
    let output = result
        .map_err(|_| "SSH connection test timed out after 30s".to_string())?
        .map_err(|e| format!("failed to run ssh: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && stdout.contains("__WISP_SSH_OK__") {
        crate::ssh_guard::record_success(&format!("ssh:{}", connection.alias));
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        Err(if detail.is_empty() {
            format!(
                "ssh exited with status {}",
                output.status.code().unwrap_or(-1)
            )
        } else {
            detail
        })
    }
}

#[tauri::command]
pub async fn add_ssh_host(
    state: State<'_, crate::AppState>,
    host: SshHost,
) -> Result<Vec<SshHost>, String> {
    SshConnection::from_host(&host)?;
    apply_host_password(&host)?;
    let host = persistable_host(&host);
    let hosts = upsert_host(load(&state.store).await, host);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts.into_iter().map(decorate_host).collect())
}

#[tauri::command]
pub async fn remove_ssh_host(
    state: State<'_, crate::AppState>,
    alias: String,
) -> Result<Vec<SshHost>, String> {
    let hosts = remove_host(load(&state.store).await, &alias);
    save(&state.store, &hosts).await?;
    if let Err(error) = state
        .run_manager
        .wind_down_context(&state.store, &format!("ssh:{alias}"))
        .await
    {
        tracing::warn!(alias = %alias, "host wind-down failed: {error}");
    }
    crate::run_context::remote_files::abandon_context_sources(&state.store, &alias).await?;
    remove_context_for_alias(&state.store, &alias).await?;
    let _ = password_delete(&alias);
    Ok(hosts.into_iter().map(decorate_host).collect())
}

#[tauri::command]
pub async fn import_ssh_config_hosts(
    state: State<'_, crate::AppState>,
) -> Result<Vec<SshHost>, String> {
    let aliases = list_ssh_config_aliases();
    let hosts = merge_config_aliases(load(&state.store).await, aliases);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts.into_iter().map(decorate_host).collect())
}
