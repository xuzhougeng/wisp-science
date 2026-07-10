use super::{
    build_provider_config, clear_idle_agents, effective_api_key, load_locale, load_settings,
    models, normalized_provider, AppState, Settings,
};
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_llm::Message;

#[tauri::command]
pub(super) async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let (provider, api_url, model, _api_key) = load_settings(&state.store).await;
    let locale = load_locale(&state.store).await;
    let workspace_dir = state
        .store
        .get_setting("workspace_dir")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let (max_tokens, reasoning_effort) = models::active_llm_advanced(&state.store).await;
    let has_api_key = models::active_has_key(&state.store).await;
    let supports_vision = models::active_supports_vision(&state.store).await;
    let label = models::active_label(&state.store).await;
    Ok(Settings {
        provider,
        api_url,
        model,
        label,
        has_api_key,
        locale,
        workspace_dir,
        max_tokens,
        reasoning_effort,
        supports_vision,
    })
}

#[tauri::command]
pub(super) async fn set_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    let provider = normalized_provider(&settings.provider);
    let api_url = settings.api_url.trim();
    let model = settings.model.trim();
    if api_url.is_empty() {
        return Err("API URL is required.".into());
    }
    if model.is_empty() {
        return Err("Model is required.".into());
    }
    tracing::info!(
        target: "wisp",
        provider = %provider,
        api_url = %api_url,
        model = %model,
        "saving settings"
    );
    // provider/api_url/model belong to the *active* model profile now, not a
    // single global config — the classic form edits whichever model is active.
    models::set_active_fields(
        &state.store,
        &provider,
        api_url,
        model,
        settings.label.trim(),
    )
    .await?;
    let locale = match settings.locale.trim() {
        "zh" | "zh-CN" | "zh-TW" => "zh",
        other if !other.is_empty() => other,
        _ => "en",
    };
    state
        .store
        .set_setting("locale", locale)
        .await
        .map_err(|e| format!("{e}"))?;

    // Workspace directory: persist an absolute, creatable path. Takes effect on
    // next launch (AppState.root is fixed at startup — restart, not hot-swap).
    let workspace_dir = settings.workspace_dir.trim();
    if workspace_dir.is_empty() {
        // Empty clears the override → back to the platform default next launch.
        state
            .store
            .set_setting("workspace_dir", "")
            .await
            .map_err(|e| format!("{e}"))?;
    } else {
        let ws = Path::new(workspace_dir);
        if !ws.is_absolute() {
            return Err("Workspace directory must be an absolute path.".into());
        }
        // Don't create the dir here. It only takes effect next launch, where
        // `ensure_writable` creates it (with a fallback). Creating it eagerly
        // during save can block the whole command on a bad/removable path —
        // e.g. Windows pops a modal "insert a disk in drive D:" — wedging the
        // UI at "Saving…" forever (#40). Just persist the string.
        state
            .store
            .set_setting("workspace_dir", workspace_dir)
            .await
            .map_err(|e| format!("{e}"))?;
    }

    // Reset cached agents so the next turn picks up the new provider.
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn set_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    tracing::info!(target: "wisp", has_api_key = !key.is_empty(), "saving api key");
    // The key belongs to the active model profile.
    models::set_active_key(&state.store, &key).await
}

#[tauri::command]
pub(super) async fn credential_status() -> Result<Vec<(String, bool)>, String> {
    Ok(models::credential_status())
}

fn agent_infini_binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "agent_infini.exe"
    } else {
        "agent_infini"
    };
    let path_bins = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|p| p.join(exe))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    path_bins
        .into_iter()
        .chain(dirs::home_dir().map(|home| home.join(".infini").join("bin").join(exe)))
        .find(|p| p.is_file())
}

async fn init_agent_infini(api_key: &str) -> Result<(), String> {
    let bin = agent_infini_binary().ok_or_else(|| {
        let install = if cfg!(windows) {
            "irm https://infinisynapse.cn/cli-install/install.ps1 | iex"
        } else {
            "curl -fsSL https://infinisynapse.cn/cli-install/install.sh | bash"
        };
        format!("agent_infini not found. Install it with: {install}")
    })?;
    let out = tokio::process::Command::new(&bin)
        .arg("init")
        .arg("--api-key")
        .arg(api_key)
        .output()
        .await
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let mut detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stdout.is_empty() {
        if !detail.is_empty() {
            detail.push('\n');
        }
        detail.push_str(&stdout);
    }
    let detail = detail.replace(api_key, "<redacted>");
    if detail.is_empty() {
        Err(format!(
            "agent_infini init failed with status {}",
            out.status
        ))
    } else {
        Err(format!("agent_infini init failed: {detail}"))
    }
}

#[tauri::command]
pub(super) async fn set_credential(
    state: State<'_, AppState>,
    id: String,
    value: String,
) -> Result<(), String> {
    let value = value.trim().to_string();
    // OpenAlex is the one service with a cheap online key probe: GET
    // /rate-limit carrying only api_key. 2xx or 429 (= authenticated but over
    // budget) means the key works; any other 4xx means OpenAlex rejected it.
    // Network trouble is treated like success (soft-degrade) — don't block
    // saving a key offline. Other credentials (NCBI key/email) have no cheap
    // standalone probe, so they're stored as-is.
    if id == "openalex_api_key" && !value.is_empty() {
        let resp = reqwest::Client::builder()
            .user_agent("wisp-science")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?
            .get("https://api.openalex.org/rate-limit")
            .query(&[("api_key", value.as_str())])
            .send()
            .await;
        if let Ok(r) = resp {
            let s = r.status();
            if s.is_client_error() && s.as_u16() != 429 {
                return Err("OpenAlex rejected this API key.".into());
            }
        }
    }
    if id == "infinisynapse_api_key" && !value.is_empty() {
        init_agent_infini(&value).await?;
    }
    tracing::info!(target: "wisp", id = %id, present = !value.is_empty(), "saving credential");
    models::store_credential(&id, &value)?;
    // Respawn kernels/MCP on the next turn so they inherit the new env.
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn validate_settings(
    state: State<'_, AppState>,
    settings: Settings,
    key: Option<String>,
) -> Result<String, String> {
    let provider_name = normalized_provider(&settings.provider);
    let (_, _, _, stored_key) = load_settings(&state.store).await;
    let api_key = effective_api_key(key, stored_key);
    let mut cfg = build_provider_config(
        &settings.provider,
        &settings.api_url,
        &api_key,
        &settings.model,
        settings.max_tokens,
        &settings.reasoning_effort,
    )?;
    // Keep the ping cheap but respect API minimum (Responses API needs >= 16).
    cfg.max_tokens = cfg.max_tokens.min(64).max(16);

    tracing::info!(
        target: "wisp",
        provider = %provider_name,
        api_url = %settings.api_url,
        model = %settings.model,
        "validating settings"
    );
    let provider = wisp_llm::build(cfg);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.complete(&[Message::user("Reply with OK.")], &[]),
    )
    .await
    .map_err(|_| {
        tracing::warn!(target: "wisp", "settings validation timed out");
        "Validation timed out after 30s".to_string()
    })?;
    if let Err(e) = result {
        tracing::warn!(target: "wisp", error = %e, "settings validation failed");
        return Err(format!("{e}"));
    }

    tracing::info!(target: "wisp", "settings validation succeeded");
    Ok(format!(
        "Validated {} with {}",
        provider_name, settings.model
    ))
}
