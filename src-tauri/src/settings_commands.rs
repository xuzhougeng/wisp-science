use super::{
    build_provider_config, clear_idle_agents, desktop_lifecycle, effective_api_key, load_locale,
    load_settings, models, normalized_provider, pet_commands::load_pet_asset, AppState, Settings,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::State;
use wisp_llm::Message;
use wisp_store::secrets::Secret;

const SYNC_RELAY_TOKEN: &str = "sync_relay_token";

#[derive(serde::Serialize)]
pub(super) struct TokenUsageOverview {
    workspaces: Vec<wisp_store::ProjectTokenUsage>,
    days: Vec<wisp_store::TokenUsageDay>,
    models: Vec<wisp_store::ModelTokenUsage>,
    tools: Vec<wisp_store::ToolCallUsage>,
}

fn annotate_provider_error(error: impl ToString, proxy: Option<&str>) -> String {
    wisp_llm::annotate_transport_error(&error.to_string(), proxy, &wisp_llm::ambient_proxy_env())
}

async fn validate_provider_config(
    provider_name: &str,
    mut cfg: wisp_llm::ProviderConfig,
    supports_vision: bool,
) -> Result<(), String> {
    let proxy = cfg.proxy.clone();
    if models::is_image_generation_model(&cfg.model) {
        if !models::supports_image_generation(provider_name, &cfg.model) {
            return Err(models::IMAGE_GENERATION_UNSUPPORTED.into());
        }
        return super::image_generation_tool::GenerateImageTool::new(
            cfg.base_url,
            cfg.api_key,
            cfg.model,
            cfg.proxy,
        )
        .validate_model_access()
        .await
        .map_err(|error| annotate_provider_error(error, proxy.as_deref()));
    }
    if models::is_video_generation_model(&cfg.model) {
        if !models::supports_video_generation(provider_name, &cfg.model) {
            return Err(models::VIDEO_GENERATION_UNSUPPORTED.into());
        }
        return super::video_generation_tool::GenerateVideoTool::new(
            cfg.base_url,
            cfg.api_key,
            cfg.model,
            cfg.proxy,
        )
        .validate_model_access()
        .await;
    }

    // Keep the ping cheap but respect API minimum (Responses API needs >= 16).
    cfg.max_tokens = cfg.max_tokens.min(64).max(16);
    // "Supports images" is checked by hand, so probe with a real image rather
    // than trusting the box — otherwise the first pasted screenshot is what
    // discovers the model can't take one.
    let probe = if supports_vision {
        vision_probe_message()
    } else {
        Message::user("Reply with OK.")
    };
    wisp_llm::build(cfg)
        .complete(&[probe], &[])
        .await
        .map(|_| ())
        .map_err(|error| annotate_provider_error(error, proxy.as_deref()))
}

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
    let max_iter = state
        .store
        .get_setting("max_iter")
        .await
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value >= 0)
        .unwrap_or_else(super::default_max_iter_setting);
    let (max_tokens, reasoning_effort, service_tier) =
        models::active_llm_advanced(&state.store).await;
    let has_api_key = models::active_has_key(&state.store).await;
    let supports_vision = models::active_supports_vision(&state.store).await;
    let label = models::active_label(&state.store).await;
    let sync_backend = state
        .store
        .get_setting("sync_backend")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "relay".into());
    let sync_relay_url = state
        .store
        .get_setting("sync_relay_url")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let sync_folder = state
        .store
        .get_setting("sync_folder")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let has_sync_relay_token =
        tokio::task::spawn_blocking(|| Secret::get(SYNC_RELAY_TOKEN).is_ok())
            .await
            .unwrap_or(false);
    let pet_enabled = state
        .store
        .get_setting("pet_enabled")
        .await
        .ok()
        .flatten()
        .map(|value| value == "true")
        .unwrap_or(false);
    let pet_directory = state
        .store
        .get_setting("pet_directory")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let notifications_enabled = super::load_notifications_enabled(&state.store).await;
    let auto_compact = super::load_auto_compact_enabled(&state.store).await;
    let (auto_continue, auto_continue_limit) =
        super::load_auto_continue_settings(&state.store).await;
    let follow_up_questions = state
        .store
        .get_setting("follow_up_questions")
        .await
        .ok()
        .flatten()
        .map(|value| value == "true")
        .unwrap_or(true);
    let resume_last_session = state
        .store
        .get_setting("resume_last_session")
        .await
        .ok()
        .flatten()
        .map(|value| value == "true")
        .unwrap_or(true);
    let proxy_url = state
        .store
        .get_setting("proxy_url")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Settings {
        provider,
        api_url,
        model,
        label,
        has_api_key,
        locale,
        workspace_dir,
        max_iter,
        auto_compact,
        auto_continue,
        auto_continue_limit: auto_continue_limit as u64,
        follow_up_questions,
        resume_last_session,
        max_tokens,
        reasoning_effort,
        service_tier,
        proxy_url,
        supports_vision,
        sync_backend,
        sync_relay_url,
        sync_folder,
        sync_relay_token: String::new(),
        has_sync_relay_token,
        pet_enabled,
        pet_directory,
        notifications_enabled,
    })
}

#[tauri::command]
pub(super) async fn set_settings(
    app: tauri::AppHandle,
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
    validate_max_iter(settings.max_iter)?;
    if !matches!(settings.sync_backend.as_str(), "relay" | "folder") {
        return Err("Sync backend must be relay or shared folder.".into());
    }
    let sync_relay_url = settings.sync_relay_url.trim();
    if !sync_relay_url.is_empty() {
        let url = url::Url::parse(sync_relay_url)
            .map_err(|_| "Sync relay URL is invalid.".to_string())?;
        let local_http = url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if url.scheme() != "https" && !local_http {
            return Err(
                "Sync relay URL must use HTTPS (HTTP is allowed only for localhost).".into(),
            );
        }
    }
    let sync_folder = settings.sync_folder.trim();
    if !sync_folder.is_empty() && !Path::new(sync_folder).is_absolute() {
        return Err("Shared sync folder must be an absolute path.".into());
    }
    let workspace_dir = settings.workspace_dir.trim();
    if !workspace_dir.is_empty() && !Path::new(workspace_dir).is_absolute() {
        return Err("Workspace directory must be an absolute path.".into());
    }
    let pet_directory = settings.pet_directory.trim();
    if !pet_directory.is_empty() && !Path::new(pet_directory).is_absolute() {
        return Err("Pet directory must be an absolute path.".into());
    }
    let proxy_url = settings.proxy_url.trim();
    if !proxy_url.is_empty() && proxy_url != "none" && reqwest::Proxy::all(proxy_url).is_err() {
        return Err(
            "Proxy must be empty, `none`, or a URL like http://127.0.0.1:7890 / socks5://127.0.0.1:1080.".into(),
        );
    }
    if settings.pet_enabled {
        if pet_directory.is_empty() {
            return Err("Choose a pet directory before enabling the pet.".into());
        }
        load_pet_asset(Path::new(pet_directory))?;
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
    #[cfg(target_os = "macos")]
    super::install_macos_app_menu(&app, locale)?;
    #[cfg(target_os = "windows")]
    desktop_lifecycle::apply_windows_tray_locale(&app, locale)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = app;
    state
        .store
        .set_setting("sync_backend", &settings.sync_backend)
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("sync_relay_url", sync_relay_url)
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("sync_folder", sync_folder)
        .await
        .map_err(|e| e.to_string())?;
    if !settings.sync_relay_token.trim().is_empty() {
        let token = settings.sync_relay_token.trim().to_string();
        tokio::task::spawn_blocking(move || Secret::set(SYNC_RELAY_TOKEN, &token))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    state
        .store
        .set_setting(
            "pet_enabled",
            if settings.pet_enabled {
                "true"
            } else {
                "false"
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("pet_directory", pet_directory)
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(
            "notifications_enabled",
            if settings.notifications_enabled {
                "true"
            } else {
                "false"
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("max_iter", &settings.max_iter.to_string())
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("auto_compact", &settings.auto_compact.to_string())
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("auto_continue", &settings.auto_continue.to_string())
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(
            "auto_continue_limit",
            &settings.auto_continue_limit.max(1).to_string(),
        )
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(
            "follow_up_questions",
            &settings.follow_up_questions.to_string(),
        )
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(
            "resume_last_session",
            &settings.resume_last_session.to_string(),
        )
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("proxy_url", proxy_url)
        .await
        .map_err(|e| e.to_string())?;
    super::set_llm_proxy(proxy_url);
    desktop_lifecycle::sync_pet_window(&app, settings.pet_enabled)?;

    // Workspace directory: persist an absolute, creatable path. Takes effect on
    // next launch (AppState.root is fixed at startup — restart, not hot-swap).
    if workspace_dir.is_empty() {
        // Empty clears the override → back to the platform default next launch.
        state
            .store
            .set_setting("workspace_dir", "")
            .await
            .map_err(|e| format!("{e}"))?;
    } else {
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

fn validate_max_iter(max_iter: i64) -> Result<(), String> {
    if max_iter < 0 {
        return Err("Maximum agent iterations cannot be negative.".into());
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn credential_status(
    state: State<'_, AppState>,
) -> Result<Vec<(String, bool)>, String> {
    models::load_custom_credentials(&state.store).await?;
    Ok(models::credential_status())
}

#[tauri::command]
pub(super) async fn list_custom_credentials(
    state: State<'_, AppState>,
) -> Result<Vec<models::CustomCredentialStatus>, String> {
    models::custom_credential_status(&state.store).await
}

#[tauri::command]
pub(super) async fn add_custom_credential(
    state: State<'_, AppState>,
    name: String,
    env_var: String,
    value: String,
) -> Result<models::CustomCredentialStatus, String> {
    let credential = models::add_custom_credential(&state.store, &name, &env_var, &value).await?;
    tracing::info!(
        target: "wisp",
        id = %credential.id,
        env_var = %credential.env_var,
        "added custom credential"
    );
    clear_idle_agents(&state).await;
    Ok(credential)
}

#[tauri::command]
pub(super) async fn remove_custom_credential(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    models::remove_custom_credential(&state.store, &id).await?;
    tracing::info!(target: "wisp", id = %id, "removed custom credential");
    clear_idle_agents(&state).await;
    Ok(())
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
    let mut command = tokio::process::Command::new(&bin);
    command.arg("init").arg("--api-key").arg(api_key);
    wisp_tools::process::hide_console_async(&mut command);
    let out = command
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

fn scimaster_config_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".scimaster").join("config.json"))
        .ok_or_else(|| "Could not resolve the home directory for SciMaster config.".into())
}

fn merged_scimaster_config(raw: Option<&str>, api_key: Option<&str>) -> Result<String, String> {
    let mut root = match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => serde_json::from_str::<serde_json::Value>(text).unwrap_or_else(|_| json!({})),
        None => json!({}),
    };
    if !root.is_object() {
        root = json!({});
    }
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "SciMaster config must be a JSON object.".to_string())?;
    obj.entry("version").or_insert_with(|| json!(1));
    obj.entry("apiBaseUrl")
        .or_insert_with(|| json!("https://scimaster.bohrium.com"));
    let defaults = obj.entry("defaults").or_insert_with(|| json!({}));
    if !defaults.is_object() {
        *defaults = json!({});
    }
    if let Some(defaults_obj) = defaults.as_object_mut() {
        defaults_obj.entry("limit").or_insert_with(|| json!(10));
        defaults_obj.entry("mode").or_insert_with(|| json!("low"));
    }
    match api_key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(key) => {
            obj.insert("apiKey".into(), serde_json::Value::String(key.to_string()));
        }
        None => {
            obj.remove("apiKey");
        }
    }
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn sync_scimaster_config_at(path: &Path, api_key: &str) -> Result<(), String> {
    let api_key = api_key.trim();
    if api_key.is_empty() && !path.is_file() {
        return Ok(());
    }
    let current = if path.is_file() {
        Some(
            fs::read_to_string(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?,
        )
    } else {
        None
    };
    let merged =
        merged_scimaster_config(current.as_deref(), (!api_key.is_empty()).then_some(api_key))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid SciMaster config path: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    fs::write(path, merged).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to secure {}: {e}", path.display()))?;
    }
    Ok(())
}

fn sync_scimaster_config(api_key: &str) -> Result<(), String> {
    let path = scimaster_config_path()?;
    sync_scimaster_config_at(&path, api_key)
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
    if id == "scimaster_api_key" {
        sync_scimaster_config(&value)?;
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
    profile_id: Option<String>,
) -> Result<String, String> {
    let provider_name = normalized_provider(&settings.provider);
    let stored_key = match profile_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => models::profile_key(&state.store, id)
            .await
            .unwrap_or_default(),
        None => {
            let (_, _, _, stored_key) = load_settings(&state.store).await;
            stored_key
        }
    };
    let api_key = effective_api_key(key, stored_key);
    let mut cfg = build_provider_config(
        &settings.provider,
        &settings.api_url,
        &api_key,
        &settings.model,
        settings.max_tokens,
        &settings.reasoning_effort,
        &settings.service_tier,
    )?;
    let form_proxy = settings.proxy_url.trim();
    if !form_proxy.is_empty() {
        cfg.proxy = Some(form_proxy.to_string());
    }

    tracing::info!(
        target: "wisp",
        provider = %provider_name,
        api_url = %settings.api_url,
        model = %settings.model,
        "validating settings"
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        validate_provider_config(&provider_name, cfg, settings.supports_vision),
    )
    .await
    .map_err(|_| {
        tracing::warn!(target: "wisp", "settings validation timed out");
        "Validation timed out after 30s".to_string()
    })?;
    if let Err(e) = result {
        tracing::warn!(target: "wisp", error = %e, vision = settings.supports_vision, "settings validation failed");
        return Err(e);
    }

    tracing::info!(target: "wisp", "settings validation succeeded");
    Ok(format!(
        "Validated {} with {}",
        provider_name, settings.model
    ))
}

/// 16x16 PNG — small enough to be free, large enough that vision APIs with a
/// minimum-dimension rule don't reject it for the wrong reason.
fn vision_probe_message() -> Message {
    use wisp_llm::message::{Content, ImageUrl, Part};
    const PROBE_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAFklEQVR42mM4EWBDEmIY1TCqYfhqAABNl1QQCkyLAQAAAABJRU5ErkJggg==";
    let mut msg = Message::user("");
    msg.content = Content::Parts(vec![
        Part::Text {
            kind: "text".into(),
            text: "Reply with OK.".into(),
        },
        Part::Image {
            kind: "image_url".into(),
            image_url: ImageUrl {
                url: format!("data:image/png;base64,{PROBE_PNG}"),
            },
        },
    ]);
    msg
}

#[cfg(test)]
mod tests {
    use super::{
        collect_storage_usage, merged_scimaster_config, sync_scimaster_config_at,
        validate_max_iter, vision_probe_message,
    };
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn vision_probe_sends_a_decodable_png_part() {
        let v = serde_json::to_value(vision_probe_message()).unwrap();
        let parts = v["content"].as_array().expect("multipart content");
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        let b64 = url
            .strip_prefix("data:image/png;base64,")
            .expect("data URI");
        // A probe that isn't a real image would fail on vision models too, and
        // that false negative is exactly what this check is meant to prevent.
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn scimaster_config_merge_sets_key_and_defaults() {
        let json = merged_scimaster_config(None, Some("sk-sci")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["apiKey"], "sk-sci");
        assert_eq!(v["apiBaseUrl"], "https://scimaster.bohrium.com");
        assert_eq!(v["defaults"]["limit"], 10);
        assert_eq!(v["defaults"]["mode"], "low");
    }

    #[test]
    fn max_iter_cannot_be_negative() {
        assert!(validate_max_iter(0).is_ok());
        assert!(validate_max_iter(1).is_ok());
        assert_eq!(
            validate_max_iter(-1).unwrap_err(),
            "Maximum agent iterations cannot be negative."
        );
    }

    #[test]
    fn scimaster_config_sync_preserves_existing_settings_when_clearing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "wisp-scimaster-config-test-{}-{unique}",
            std::process::id()
        ));
        let path = dir.join("config.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            r#"{"version":1,"apiKey":"old-key","apiBaseUrl":"https://custom.example","defaults":{"limit":25,"mode":"mid"}}"#,
        )
        .unwrap();

        sync_scimaster_config_at(&path, "").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v.get("apiKey").is_none());
        assert_eq!(v["apiBaseUrl"], "https://custom.example");
        assert_eq!(v["defaults"]["limit"], 25);
        assert_eq!(v["defaults"]["mode"], "mid");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_usage_keeps_project_workspaces_separate() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wisp-storage-usage-test-{}-{unique}",
            std::process::id()
        ));
        let app_data = root.join("app");
        let workspace = root.join("workspace");
        fs::create_dir_all(&app_data).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(app_data.join("wisp.sqlite"), [0u8; 3]).unwrap();
        fs::write(workspace.join("results.csv"), [0u8; 7]).unwrap();

        let usage = collect_storage_usage(
            app_data,
            vec![
                ("a".into(), "Alpha".into(), workspace.clone()),
                ("b".into(), "Beta".into(), workspace.clone()),
            ],
        );

        assert_eq!(usage["projects"][0]["name"], "Alpha");
        assert_eq!(usage["projects"][0]["bytes"], 7);
        assert_eq!(usage["projects"][1]["name"], "Beta");
        assert_eq!(usage["projects"][1]["bytes"], 7);
        assert_eq!(
            usage["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["key"] == "workspace")
                .unwrap()["bytes"],
            7
        );

        let _ = fs::remove_dir_all(&root);
    }
}

use super::*;

#[tauri::command]
pub(super) async fn get_auto_review_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(load_auto_review_enabled(&state.store).await)
}

#[tauri::command]
pub(super) async fn set_auto_review_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    save_auto_review_enabled(&state.store, enabled).await?;
    Ok(enabled)
}

#[tauri::command]
pub(super) async fn get_update_check_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(load_update_check_enabled(&state.store).await)
}

#[tauri::command]
pub(super) async fn set_update_check_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    save_update_check_enabled(&state.store, enabled).await?;
    Ok(enabled)
}

/// Sum of file sizes under `path`; 0 for a missing path.
fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}

pub(crate) fn collect_storage_usage(
    app_data: PathBuf,
    projects: Vec<(String, String, PathBuf)>,
) -> serde_json::Value {
    let project_dirs = projects
        .iter()
        .map(|(_, _, root)| root.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let project_sizes = project_dirs
        .iter()
        .map(|root| (root.clone(), dir_size(root)))
        .collect::<std::collections::BTreeMap<_, _>>();

    let (mut database, mut python, mut plugins, mut other) = (0u64, 0u64, 0u64, 0u64);
    for entry in fs::read_dir(&app_data).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = if entry.path().is_dir() {
            dir_size(&entry.path())
        } else {
            entry.metadata().map(|meta| meta.len()).unwrap_or(0)
        };
        match name.as_str() {
            name if name.contains(".sqlite") => database += bytes,
            "python" => python += bytes,
            "plugins" | "plugin-staging" | "plugin-downloads" => plugins += bytes,
            _ => other += bytes,
        }
    }
    let workspace = project_dirs
        .iter()
        .filter(|root| !root.starts_with(&app_data))
        .map(|root| project_sizes.get(root).copied().unwrap_or_default())
        .sum();
    let entries = [
        ("database", database),
        ("python", python),
        ("plugins", plugins),
        ("workspace", workspace),
        ("other", other),
    ];

    json!({
        "data_dir": app_data.to_string_lossy(),
        "projects": projects
            .into_iter()
            .map(|(id, name, path)| json!({
                "id": id,
                "name": name,
                "path": path.to_string_lossy(),
                "bytes": project_sizes.get(&path).copied().unwrap_or_default(),
            }))
            .collect::<Vec<_>>(),
        "entries": entries
            .iter()
            .map(|(key, bytes)| json!({ "key": key, "bytes": bytes }))
            .collect::<Vec<_>>(),
        "total_bytes": entries.iter().map(|(_, bytes)| bytes).sum::<u64>(),
    })
}

#[tauri::command]
pub(super) async fn get_storage_usage(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let app_data = state.app_data.clone();
    let projects = state
        .store
        .list_projects()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, name, root, ..)| (id, name, PathBuf::from(root)))
        .filter(|(_, _, root)| !root.as_os_str().is_empty())
        .collect();
    tokio::task::spawn_blocking(move || collect_storage_usage(app_data, projects))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn get_token_usage(
    state: State<'_, AppState>,
) -> Result<TokenUsageOverview, String> {
    let workspaces = state
        .store
        .token_usage_by_project()
        .await
        .map_err(|error| error.to_string())?;
    let days = state
        .store
        .token_usage_activity()
        .await
        .map_err(|error| error.to_string())?;
    let models = state
        .store
        .token_usage_by_model()
        .await
        .map_err(|error| error.to_string())?;
    let tools = state
        .store
        .tool_call_usage_ranking()
        .await
        .map_err(|error| error.to_string())?;
    Ok(TokenUsageOverview {
        workspaces,
        days,
        models,
        tools,
    })
}

#[tauri::command]
pub(super) async fn get_session_token_usage(
    state: State<'_, AppState>,
    project_id: String,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<wisp_store::SessionTokenUsagePage, String> {
    state
        .store
        .token_usage_by_session(&project_id, offset.unwrap_or(0), limit.unwrap_or(20))
        .await
        .map_err(|error| error.to_string())
}
