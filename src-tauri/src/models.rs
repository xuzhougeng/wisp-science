//! Model profiles: several named LLM configs (provider + API URL + model +
//! its own key), one of them active. The active profile drives every turn —
//! `load_settings` resolves through here — and the composer switches it.
//!
//! Legacy single-model installs are migrated into one "default" profile the
//! first time this is read, so nothing breaks and no key is lost.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::State;

pub const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

fn default_context_window() -> u64 {
    DEFAULT_CONTEXT_WINDOW
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub provider: String,
    /// Shared credential root for one API access. Protocol-specific paths
    /// live in `endpoint_suffix`. Profiles on the same root share a key only
    /// when they currently hold the same stored secret; a second access on
    /// this URL can keep a different key.
    pub api_url: String,
    #[serde(default)]
    pub endpoint_suffix: String,
    pub model: String,
    /// Computed on read from the keyring; never part of the persisted JSON.
    #[serde(default)]
    pub has_api_key: bool,
    /// Computed on read; true for the active profile.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub max_tokens: u64,
    /// Total input + output context capacity advertised for this model. Reader
    /// session splitting uses this value; it is not sent to the provider.
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    #[serde(default)]
    pub reasoning_effort: String,
    /// Capability marker: this API model can accept image input.
    #[serde(default)]
    pub supports_vision: bool,
    /// Computed on read / accepted on save; true when this profile is assigned
    /// to image analysis. Serialized so the UI can restore the checkbox.
    #[serde(default)]
    pub use_for_vision: bool,
    /// Computed on read / accepted on save; true when this profile is assigned
    /// to the Scientific Illustrator's raster image-generation tool.
    #[serde(default)]
    pub use_for_image_generation: bool,
    /// OpenAI image size (`auto`, `1024x1024`, …). Empty means the tool default.
    #[serde(default)]
    pub image_size: String,
    /// Image quality (`auto`/`low`/`medium`/`high`, or Grok `low`/`medium`).
    #[serde(default)]
    pub image_quality: String,
    /// Grok Imagine aspect ratio (`auto`, `1:1`, `16:9`, …).
    #[serde(default)]
    pub image_aspect_ratio: String,
    /// Grok Imagine resolution (`1k` or `2k`).
    #[serde(default)]
    pub image_resolution: String,
    /// Computed on read / accepted on save; true when this profile is assigned
    /// to the video-generation tool.
    #[serde(default)]
    pub use_for_video_generation: bool,
    /// Video length in seconds (1–15). None means the tool default.
    #[serde(default)]
    pub video_duration_secs: Option<u32>,
    /// Video aspect ratio (`16:9`, `9:16`, `1:1`, `4:3`, `3:4`).
    #[serde(default)]
    pub video_aspect_ratio: Option<String>,
    /// Video resolution (`480p`, `720p`, `1080p`).
    #[serde(default)]
    pub video_resolution: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ImageGenerationOptions {
    pub size: String,
    pub quality: String,
    pub aspect_ratio: String,
    pub resolution: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VideoGenerationOptions {
    pub duration_secs: u32,
    pub aspect_ratio: String,
    pub resolution: String,
}

impl Default for VideoGenerationOptions {
    fn default() -> Self {
        Self {
            duration_secs: 5,
            aspect_ratio: "16:9".into(),
            resolution: "720p".into(),
        }
    }
}

const PROFILES_KEY: &str = "model_profiles";
const ACTIVE_KEY: &str = "active_model_id";
const VISION_KEY: &str = "vision_model_id";
const IMAGE_GENERATION_KEY: &str = "image_generation_model_id";
const VIDEO_GENERATION_KEY: &str = "video_generation_model_id";
const LEGACY_KEY_SECRET: &str = "api_key";
const CUSTOM_CREDENTIALS_KEY: &str = "custom_credentials";
const CUSTOM_CREDENTIAL_SECRET_PREFIX: &str = "custom_credential:";

fn secret_name(id: &str) -> String {
    format!("model_key:{id}")
}

fn normalize_endpoint_suffix(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if value.contains("://")
        || value.starts_with("//")
        || value.contains('?')
        || value.contains('#')
    {
        return Err(
            "Endpoint suffix must be a URL path without a host, query, or fragment.".into(),
        );
    }
    let value = value.trim_matches('/');
    if value.is_empty() {
        return Ok(String::new());
    }
    if value.split('/').any(|segment| segment == "..") {
        return Err("Endpoint suffix cannot contain '..' path segments.".into());
    }
    Ok(format!("/{value}"))
}

pub(crate) fn join_api_url(base_url: &str, endpoint_suffix: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    let endpoint_suffix = endpoint_suffix.trim().trim_matches('/');
    if endpoint_suffix.is_empty() {
        base_url.to_string()
    } else {
        format!("{base_url}/{endpoint_suffix}")
    }
}

fn effective_api_url(profile: &ModelProfile) -> String {
    join_api_url(&profile.api_url, &profile.endpoint_suffix)
}

/// Identity of an LLM host: the API origin, not the provider enum and not the
/// model id. `openai` covers DeepSeek, official OpenAI, and local gateways;
/// those must not inherit a key from each other. OpenAI Chat and Responses on
/// the same host may share a key, but a second pasted key on that host stays
/// on its own batch.
pub(crate) fn normalize_endpoint(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http://", rest)
    } else {
        ("", url)
    };
    let rest = if scheme.is_empty() {
        rest.to_string()
    } else {
        match rest.split_once('/') {
            Some((host, path)) => format!("{}/{}", host.to_ascii_lowercase(), path),
            None => rest.to_ascii_lowercase(),
        }
    };
    let mut endpoint = format!("{scheme}{rest}");
    loop {
        while endpoint.ends_with('/') {
            endpoint.pop();
        }
        let Some(stripped) = [
            "/v1/messages",
            "/v1/chat/completions",
            "/chat/completions",
            "/responses",
            "/v1",
        ]
        .into_iter()
        .find_map(|suffix| endpoint.strip_suffix(suffix).map(str::to_string)) else {
            break;
        };
        endpoint = stripped;
    }
    endpoint
}

fn same_endpoint(left: &str, right: &str) -> bool {
    let left = normalize_endpoint(left);
    !left.is_empty() && left == normalize_endpoint(right)
}

fn sibling_key(profiles: &[ModelProfile], api_url: &str, exclude_id: &str) -> String {
    profiles
        .iter()
        .filter(|profile| profile.id != exclude_id && same_endpoint(&profile.api_url, api_url))
        .map(|profile| key_for(&profile.id))
        .find(|key| !key.is_empty())
        .unwrap_or_default()
}

/// Write a pasted key to this profile.
///
/// If this profile already had a key, also rotate every same-endpoint sibling
/// that currently shares that previous key. A pasted key on a new profile
/// (no previous key) stays on this profile, so a second credential can live
/// on the same Base URL without overwriting an existing batch.
///
/// If no key is pasted and this profile has none, copy a sibling's key so
/// adding another model on the same URL does not require pasting again.
fn store_profile_key(
    id: &str,
    key: Option<&str>,
    api_url: &str,
    profiles: &[ModelProfile],
) -> Result<(), String> {
    if let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) {
        let previous = key_for(id);
        secret_set(&secret_name(id), key)?;
        if !previous.is_empty() && previous != key {
            for profile in profiles {
                if profile.id != id
                    && same_endpoint(&profile.api_url, api_url)
                    && key_for(&profile.id) == previous
                {
                    secret_set(&secret_name(&profile.id), key)?;
                }
            }
        }
        return Ok(());
    }
    if key_for(id).is_empty() {
        let inherited = sibling_key(profiles, api_url, id);
        if !inherited.is_empty() {
            secret_set(&secret_name(id), &inherited)?;
        }
    }
    Ok(())
}

/// Process-lifetime cache of resolved secrets, keyed by keyring name.
///
/// On macOS the OS keyring pops a login-password prompt whenever the calling
/// app's code signature doesn't match the stored item's ACL (e.g. after the
/// unsigned→signed jump in v0.4.2). `decorated()` read the keyring once *per
/// profile on every UI refresh*, turning that into an endless prompt storm
/// (issue #85). Caching means the keyring is touched at most once per key per
/// launch; a denied prompt is remembered as empty so it stops nagging too.
/// Writes go through `secret_set`/`secret_del` so the cache never goes stale.
/// ponytail: holds keys in memory for the session (the process already does
/// while running a turn); values are dropped on process exit.
fn secret_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn secret_get(name: &str) -> String {
    if let Some(v) = secret_cache().lock().unwrap().get(name) {
        return v.clone();
    }
    let v = wisp_store::secrets::Secret::get(name)
        .ok()
        .unwrap_or_default();
    secret_cache()
        .lock()
        .unwrap()
        .insert(name.to_string(), v.clone());
    v
}

fn secret_set(name: &str, value: &str) -> Result<(), String> {
    wisp_store::secrets::Secret::set(name, value).map_err(|e| e.to_string())?;
    secret_cache()
        .lock()
        .unwrap()
        .insert(name.to_string(), value.to_string());
    Ok(())
}

fn secret_del(name: &str) -> Result<(), String> {
    let r = wisp_store::secrets::Secret::delete(name).map_err(|e| e.to_string());
    // Remember "absent" so existence checks don't re-hit (and re-prompt) the keyring.
    secret_cache()
        .lock()
        .unwrap()
        .insert(name.to_string(), String::new());
    r
}

/// Service credentials (#115): API keys/emails for external services that
/// skills and bundled MCP tools authenticate to. Each is stored in the OS
/// keyring (same cache as model keys, read at most once per launch) and
/// injected as an env var into spawned Python/MCP processes. `id` is the
/// stable UI/command identifier; `secret` is the keyring name; `env` is the
/// variable the consuming Python reads.
struct Credential {
    id: &'static str,
    secret: &'static str,
    env: &'static str,
}

/// User-defined credential metadata. The value is deliberately absent: only
/// this non-secret name/environment mapping is persisted in SQLite, while the
/// value stays in the OS keyring under an id-derived entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomCredential {
    pub id: String,
    pub name: String,
    pub env_var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomCredentialStatus {
    pub id: String,
    pub name: String,
    pub env_var: String,
    pub present: bool,
}

const CREDENTIALS: &[Credential] = &[
    Credential {
        id: "openalex_api_key",
        secret: "openalex_api_key",
        env: "OPENALEX_API_KEY",
    },
    Credential {
        id: "infinisynapse_api_key",
        secret: "infinisynapse_api_key",
        env: "INFINISYNAPSE_API_KEY",
    },
    Credential {
        id: "scimaster_api_key",
        secret: "scimaster_api_key",
        env: "SCIMASTER_API_KEY",
    },
    Credential {
        id: "ncbi_api_key",
        secret: "ncbi_api_key",
        env: "NCBI_API_KEY",
    },
    Credential {
        id: "ncbi_email",
        secret: "ncbi_email",
        env: "NCBI_EMAIL",
    },
];

fn credential(id: &str) -> Option<&'static Credential> {
    CREDENTIALS.iter().find(|c| c.id == id)
}

fn custom_credentials_cache() -> &'static Mutex<Vec<CustomCredential>> {
    static CACHE: OnceLock<Mutex<Vec<CustomCredential>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn custom_secret_name(id: &str) -> String {
    format!("{CUSTOM_CREDENTIAL_SECRET_PREFIX}{id}")
}

fn valid_env_var(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && chars.all(|ch| matches!(ch, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

fn validate_custom_credential(name: &str, env_var: &str, value: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Credential name is required.".into());
    }
    if name.len() > 80 {
        return Err("Credential name must be 80 characters or fewer.".into());
    }
    if env_var.is_empty() {
        return Err("Environment variable is required.".into());
    }
    if env_var.len() > 128 || !valid_env_var(env_var) {
        return Err(
            "Environment variable must start with a letter or underscore and contain only letters, numbers, and underscores."
                .into(),
        );
    }
    if value.is_empty() {
        return Err("Credential value is required.".into());
    }
    Ok(())
}

fn sanitized_custom_credentials(raw: &str) -> Vec<CustomCredential> {
    let mut ids = std::collections::HashSet::new();
    let mut env_vars = std::collections::HashSet::new();
    serde_json::from_str::<Vec<CustomCredential>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|credential| {
            let env_key = credential.env_var.to_ascii_uppercase();
            uuid::Uuid::parse_str(&credential.id).is_ok()
                && !credential.name.trim().is_empty()
                && valid_env_var(&credential.env_var)
                && ids.insert(credential.id.clone())
                && env_vars.insert(env_key)
        })
        .collect()
}

/// Load user-defined credential metadata from SQLite into the synchronous
/// process cache used by runtime/MCP launch paths.
pub async fn load_custom_credentials(
    store: &wisp_store::Store,
) -> Result<Vec<CustomCredential>, String> {
    let raw = store
        .get_setting(CUSTOM_CREDENTIALS_KEY)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let credentials = sanitized_custom_credentials(&raw);
    *custom_credentials_cache().lock().unwrap() = credentials.clone();
    Ok(credentials)
}

async fn save_custom_credentials(
    store: &wisp_store::Store,
    credentials: &[CustomCredential],
) -> Result<(), String> {
    let raw = serde_json::to_string(credentials).map_err(|error| error.to_string())?;
    store
        .set_setting(CUSTOM_CREDENTIALS_KEY, &raw)
        .await
        .map_err(|error| error.to_string())?;
    *custom_credentials_cache().lock().unwrap() = credentials.to_vec();
    Ok(())
}

pub async fn custom_credential_status(
    store: &wisp_store::Store,
) -> Result<Vec<CustomCredentialStatus>, String> {
    Ok(load_custom_credentials(store)
        .await?
        .into_iter()
        .map(|credential| CustomCredentialStatus {
            present: !secret_get(&custom_secret_name(&credential.id)).is_empty(),
            id: credential.id,
            name: credential.name,
            env_var: credential.env_var,
        })
        .collect())
}

pub async fn add_custom_credential(
    store: &wisp_store::Store,
    name: &str,
    env_var: &str,
    value: &str,
) -> Result<CustomCredentialStatus, String> {
    let name = name.trim();
    let env_var = env_var.trim();
    let value = value.trim();
    validate_custom_credential(name, env_var, value)?;

    let mut credentials = load_custom_credentials(store).await?;
    if CREDENTIALS
        .iter()
        .any(|credential| credential.env.eq_ignore_ascii_case(env_var))
    {
        return Err(format!(
            "A credential already uses environment variable {env_var}."
        ));
    }

    // Re-adding an env var that already has a row overwrites it in place, so a
    // cleared or lost value never blocks reconfiguration (#335).
    if let Some(existing) = credentials
        .iter_mut()
        .find(|credential| credential.env_var.eq_ignore_ascii_case(env_var))
    {
        existing.name = name.to_string();
        let credential = existing.clone();
        secret_set(&custom_secret_name(&credential.id), value)?;
        save_custom_credentials(store, &credentials).await?;
        return Ok(CustomCredentialStatus {
            id: credential.id,
            name: credential.name,
            env_var: credential.env_var,
            present: true,
        });
    }

    let credential = CustomCredential {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        env_var: env_var.to_string(),
    };
    let secret_name = custom_secret_name(&credential.id);
    secret_set(&secret_name, value)?;
    credentials.push(credential.clone());
    if let Err(error) = save_custom_credentials(store, &credentials).await {
        let _ = secret_del(&secret_name);
        return Err(error);
    }
    Ok(CustomCredentialStatus {
        id: credential.id,
        name: credential.name,
        env_var: credential.env_var,
        present: true,
    })
}

pub async fn remove_custom_credential(store: &wisp_store::Store, id: &str) -> Result<(), String> {
    let mut credentials = load_custom_credentials(store).await?;
    let index = credentials
        .iter()
        .position(|credential| credential.id == id)
        .ok_or_else(|| format!("unknown custom credential: {id}"))?;
    let secret_name = custom_secret_name(id);
    if !secret_get(&secret_name).is_empty() {
        secret_del(&secret_name)?;
    }
    credentials.remove(index);
    save_custom_credentials(store, &credentials).await
}

/// `(id, present)` for every known credential, for the Settings UI.
pub fn credential_status() -> Vec<(String, bool)> {
    let mut status = CREDENTIALS
        .iter()
        .map(|c| (c.id.to_string(), !secret_get(c.secret).is_empty()))
        .collect::<Vec<_>>();
    status.extend(custom_credentials_cache().lock().unwrap().iter().map(|c| {
        (
            c.id.clone(),
            !secret_get(&custom_secret_name(&c.id)).is_empty(),
        )
    }));
    status
}

/// Store (or clear, when `value` is blank) a credential by id. Returns an
/// error for an unknown id.
pub fn store_credential(id: &str, value: &str) -> Result<(), String> {
    let secret = credential(id)
        .map(|credential| credential.secret.to_string())
        .or_else(|| {
            custom_credentials_cache()
                .lock()
                .unwrap()
                .iter()
                .find(|credential| credential.id == id)
                .map(|credential| custom_secret_name(&credential.id))
        })
        .ok_or_else(|| format!("unknown credential: {id}"))?;
    let value = value.trim();
    if value.is_empty() {
        // Clearing a never-stored key is fine — cache records "absent".
        let _ = secret_del(&secret);
        Ok(())
    } else {
        secret_set(&secret, value)
    }
}

/// Extra env vars for spawned service processes (Python REPL kernel and the
/// bundled bio-tools MCP server), so skills and literature tools can
/// authenticate to external APIs. Only set credentials are included.
pub fn service_env() -> Vec<(String, String)> {
    let mut env = CREDENTIALS
        .iter()
        .filter_map(|c| {
            let v = secret_get(c.secret);
            (!v.is_empty()).then(|| (c.env.to_string(), v))
        })
        .collect::<Vec<_>>();
    env.extend(
        custom_credentials_cache()
            .lock()
            .unwrap()
            .iter()
            .filter_map(|credential| {
                let value = secret_get(&custom_secret_name(&credential.id));
                (!value.is_empty()).then(|| (credential.env_var.clone(), value))
            }),
    );
    env
}

async fn load_raw(store: &wisp_store::Store) -> Vec<ModelProfile> {
    let Some(raw) = store.get_setting(PROFILES_KEY).await.ok().flatten() else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ModelProfile>>(&raw).unwrap_or_default()
}

async fn save_raw(store: &wisp_store::Store, profiles: &[ModelProfile]) -> Result<(), String> {
    let json = serde_json::to_string(profiles).map_err(|e| e.to_string())?;
    store
        .set_setting(PROFILES_KEY, &json)
        .await
        .map_err(|e| e.to_string())
}

/// Ensure at least one profile exists. On the first read of a legacy install,
/// migrate the single `provider`/`api_url`/`model` settings + `api_key` secret
/// into a "default" profile so existing users keep working unchanged.
async fn ensure(store: &wisp_store::Store) -> Vec<ModelProfile> {
    let profiles = load_raw(store).await;
    if !profiles.is_empty() {
        return profiles;
    }
    let provider = store
        .get_setting("provider")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let api_url = store
        .get_setting("api_url")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let model = store
        .get_setting("model")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let max_tokens = store
        .get_setting("max_tokens")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let reasoning_effort = store
        .get_setting("reasoning_effort")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let default = ModelProfile {
        id: "default".into(),
        label: if model.trim().is_empty() {
            "Default".into()
        } else {
            model.clone()
        },
        provider,
        api_url,
        endpoint_suffix: String::new(),
        model,
        has_api_key: false,
        active: false,
        max_tokens,
        context_window: DEFAULT_CONTEXT_WINDOW,
        reasoning_effort,
        supports_vision: false,
        use_for_vision: false,
        use_for_image_generation: false,
        image_size: String::new(),
        image_quality: String::new(),
        image_aspect_ratio: String::new(),
        image_resolution: String::new(),
        use_for_video_generation: false,
        video_duration_secs: None,
        video_aspect_ratio: None,
        video_resolution: None,
    };
    let profiles = vec![default];
    let _ = save_raw(store, &profiles).await;
    let _ = store.set_setting(ACTIVE_KEY, "default").await;
    // Carry the legacy key into the default profile's slot so it isn't lost.
    let legacy = secret_get(LEGACY_KEY_SECRET);
    if !legacy.is_empty() {
        let _ = secret_set(&secret_name("default"), &legacy);
    }
    profiles
}

async fn active_id(store: &wisp_store::Store, profiles: &[ModelProfile]) -> String {
    let want = store
        .get_setting(ACTIVE_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if profiles.iter().any(|p| p.id == want && is_chat_model(p)) {
        want
    } else {
        profiles
            .iter()
            .find(|p| is_chat_model(p))
            .or_else(|| profiles.first())
            .map(|p| p.id.clone())
            .unwrap_or_default()
    }
}

pub async fn active_profile_id(store: &wisp_store::Store) -> String {
    let profiles = ensure(store).await;
    active_id(store, &profiles).await
}

pub async fn session_profile_id(store: &wisp_store::Store, frame_id: &str) -> String {
    let profiles = ensure(store).await;
    let bound = store
        .frame_model(frame_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if profiles
        .iter()
        .any(|profile| profile.id == bound && is_chat_model(profile))
    {
        bound
    } else {
        active_id(store, &profiles).await
    }
}

pub async fn session_label(store: &wisp_store::Store, frame_id: &str) -> String {
    let profiles = ensure(store).await;
    let id = session_profile_id(store, frame_id).await;
    profiles
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| profile.label.clone())
        .unwrap_or_default()
}

/// Effective reasoning effort for a conversation: an explicit frame override
/// wins, otherwise the bound model profile supplies its configured default.
/// An empty stored override (written by older builds for "provider default")
/// counts as no override, so the profile default applies again.
pub async fn session_reasoning_effort(
    store: &wisp_store::Store,
    frame_id: &str,
    profile_default: &str,
) -> String {
    if let Ok(Some(effort)) = store.frame_reasoning_effort(frame_id).await {
        if !effort.is_empty() {
            return effort;
        }
    }
    profile_default.to_string()
}

/// Key for a profile, falling back to the legacy `api_key` secret for the
/// migrated "default" profile (so a not-yet-re-saved default still works).
fn key_for(id: &str) -> String {
    let k = secret_get(&secret_name(id));
    if k.is_empty() && id == "default" {
        secret_get(LEGACY_KEY_SECRET)
    } else {
        k
    }
}

/// The active profile's `(provider, api_url, model, api_key)` for a turn.
pub async fn active_config(store: &wisp_store::Store) -> (String, String, String, String) {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    let p = profiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .unwrap_or_else(|| profiles[0].clone());
    let api_url = effective_api_url(&p);
    (p.provider, api_url, p.model, key_for(&p.id))
}

pub(crate) const IMAGE_GENERATION_UNSUPPORTED: &str =
    "Image generation currently supports OpenAI gpt-image-2 and xAI grok-imagine-image-2.0.";

pub(crate) fn model_id_tail(model: &str) -> &str {
    let model = model.trim();
    model.rsplit('/').next().unwrap_or(model)
}

/// Raster image-generation model IDs. Gateway `vendor/model` ids match on the
/// last path segment. Exact IDs only.
pub(crate) fn is_image_generation_model(model: &str) -> bool {
    let tail = model_id_tail(model);
    tail.eq_ignore_ascii_case("gpt-image-2") || tail.eq_ignore_ascii_case("grok-imagine-image-2.0")
}

pub(crate) fn is_grok_imagine_model(model: &str) -> bool {
    model_id_tail(model).eq_ignore_ascii_case("grok-imagine-image-2.0")
}

pub(crate) const VIDEO_GENERATION_UNSUPPORTED: &str = "Video generation currently supports xAI grok-imagine-video, grok-imagine-video-1.5, and grok-imagine-video-1.5-preview.";

pub(crate) const VIDEO_ASPECT_RATIOS: &[&str] = &["16:9", "9:16", "1:1", "4:3", "3:4"];
pub(crate) const VIDEO_RESOLUTIONS: &[&str] = &["480p", "720p", "1080p"];
pub(crate) const VIDEO_DURATION_MIN_SECS: u32 = 1;
pub(crate) const VIDEO_DURATION_MAX_SECS: u32 = 15;

/// Video-generation model IDs. Gateway `vendor/model` ids match on the last
/// path segment. Exact IDs only — `grok-imagine-video` must not absorb
/// `grok-imagine-video-1.5-preview` or a future sibling.
pub(crate) fn is_video_generation_model(model: &str) -> bool {
    let tail = model_id_tail(model);
    tail.eq_ignore_ascii_case("grok-imagine-video")
        || tail.eq_ignore_ascii_case("grok-imagine-video-1.5")
        || tail.eq_ignore_ascii_case("grok-imagine-video-1.5-preview")
}

fn normalize_image_options(profile: &mut ModelProfile) -> Result<(), String> {
    if !is_image_generation_model(&profile.model) {
        profile.image_size.clear();
        profile.image_quality.clear();
        profile.image_aspect_ratio.clear();
        profile.image_resolution.clear();
        return Ok(());
    }
    let size = profile.image_size.trim();
    let quality = profile.image_quality.trim();
    let aspect = profile.image_aspect_ratio.trim();
    let resolution = profile.image_resolution.trim();
    if is_grok_imagine_model(&profile.model) {
        if !matches!(
            aspect,
            "" | "auto"
                | "1:1"
                | "16:9"
                | "9:16"
                | "4:3"
                | "3:4"
                | "3:2"
                | "2:3"
                | "2:1"
                | "1:2"
                | "19.5:9"
                | "9:19.5"
                | "20:9"
                | "9:20"
        ) {
            return Err("Unsupported image aspect ratio.".into());
        }
        if !matches!(resolution, "" | "1k" | "2k") {
            return Err("Unsupported image resolution.".into());
        }
        if !matches!(quality, "" | "low" | "medium") {
            return Err("Unsupported image quality.".into());
        }
        profile.image_size.clear();
        profile.image_aspect_ratio = aspect.to_string();
        profile.image_resolution = resolution.to_string();
        profile.image_quality = quality.to_string();
    } else {
        if !matches!(size, "" | "auto" | "1024x1024" | "1536x1024" | "1024x1536") {
            return Err("Unsupported image size.".into());
        }
        if !matches!(quality, "" | "auto" | "low" | "medium" | "high") {
            return Err("Unsupported image quality.".into());
        }
        profile.image_size = size.to_string();
        profile.image_quality = quality.to_string();
        profile.image_aspect_ratio.clear();
        profile.image_resolution.clear();
    }
    Ok(())
}

pub(crate) fn supports_image_generation(provider: &str, model: &str) -> bool {
    matches!(
        provider.trim(),
        "openai" | "openai_compatible" | "openai_responses" | "openai-responses" | "responses"
    ) && is_image_generation_model(model)
}

/// Out-of-range or unknown video options are dropped back to the tool
/// defaults rather than rejected, so a stale form value cannot wedge a save.
fn normalize_video_options(profile: &mut ModelProfile) {
    if !is_video_generation_model(&profile.model) {
        profile.video_duration_secs = None;
        profile.video_aspect_ratio = None;
        profile.video_resolution = None;
        return;
    }
    profile.video_duration_secs = profile
        .video_duration_secs
        .map(|value| value.clamp(VIDEO_DURATION_MIN_SECS, VIDEO_DURATION_MAX_SECS));
    profile.video_aspect_ratio = profile
        .video_aspect_ratio
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| VIDEO_ASPECT_RATIOS.contains(&value.as_str()));
    profile.video_resolution = profile
        .video_resolution
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| VIDEO_RESOLUTIONS.contains(&value.as_str()));
}

pub(crate) fn supports_video_generation(provider: &str, model: &str) -> bool {
    matches!(
        provider.trim(),
        "openai" | "openai_compatible" | "openai_responses" | "openai-responses" | "responses"
    ) && is_video_generation_model(model)
}

fn is_chat_model(p: &ModelProfile) -> bool {
    !is_image_generation_model(&p.model) && !is_video_generation_model(&p.model)
}

fn can_describe_images(p: &ModelProfile) -> bool {
    is_chat_model(p) && p.supports_vision
}

fn can_generate_images(p: &ModelProfile) -> bool {
    supports_image_generation(&p.provider, &p.model)
}

fn can_generate_videos(p: &ModelProfile) -> bool {
    supports_video_generation(&p.provider, &p.model)
}

async fn vision_id(store: &wisp_store::Store, profiles: &[ModelProfile]) -> Option<String> {
    let want = store
        .get_setting(VISION_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    profiles
        .iter()
        .find(|p| p.id == want && can_describe_images(p))
        .or_else(|| profiles.iter().find(|p| can_describe_images(p)))
        .map(|p| p.id.clone())
}

async fn image_generation_id(
    store: &wisp_store::Store,
    profiles: &[ModelProfile],
) -> Option<String> {
    let want = store
        .get_setting(IMAGE_GENERATION_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    profiles
        .iter()
        .find(|p| p.id == want && can_generate_images(p))
        .map(|p| p.id.clone())
}

/// The assigned vision profile's `(provider, api_url, model, api_key,
/// max_tokens, reasoning_effort)`, if the user configured one.
pub async fn vision_config(
    store: &wisp_store::Store,
) -> Option<(String, String, String, String, u64, String)> {
    let profiles = ensure(store).await;
    let id = vision_id(store, &profiles).await?;
    let p = profiles.iter().find(|p| p.id == id)?.clone();
    let api_url = effective_api_url(&p);
    Some((
        p.provider,
        api_url,
        p.model,
        key_for(&p.id),
        p.max_tokens,
        p.reasoning_effort,
    ))
}

/// The explicitly assigned image-generation profile.
/// Unlike vision, image generation has no implicit fallback: no assignment
/// means the Scientific Illustrator deliberately uses SVG.
pub async fn image_generation_config(
    store: &wisp_store::Store,
) -> Option<(String, String, String, ImageGenerationOptions)> {
    let profiles = ensure(store).await;
    let id = image_generation_id(store, &profiles).await?;
    let p = profiles.iter().find(|p| p.id == id)?;
    Some((
        effective_api_url(p),
        p.model.clone(),
        key_for(&p.id),
        ImageGenerationOptions {
            size: p.image_size.clone(),
            quality: p.image_quality.clone(),
            aspect_ratio: p.image_aspect_ratio.clone(),
            resolution: p.image_resolution.clone(),
        },
    ))
}

async fn video_generation_id(
    store: &wisp_store::Store,
    profiles: &[ModelProfile],
) -> Option<String> {
    let want = store
        .get_setting(VIDEO_GENERATION_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    profiles
        .iter()
        .find(|p| p.id == want && can_generate_videos(p))
        .map(|p| p.id.clone())
}

/// The explicitly assigned video-generation profile. Like image generation,
/// there is no implicit fallback: no assignment means no `generate_video`
/// tool is injected into the turn.
pub async fn video_generation_config(
    store: &wisp_store::Store,
) -> Option<(String, String, String, VideoGenerationOptions)> {
    let profiles = ensure(store).await;
    let id = video_generation_id(store, &profiles).await?;
    let p = profiles.iter().find(|p| p.id == id)?;
    let defaults = VideoGenerationOptions::default();
    Some((
        effective_api_url(p),
        p.model.clone(),
        key_for(&p.id),
        VideoGenerationOptions {
            duration_secs: p.video_duration_secs.unwrap_or(defaults.duration_secs),
            aspect_ratio: p
                .video_aspect_ratio
                .clone()
                .unwrap_or(defaults.aspect_ratio),
            resolution: p.video_resolution.clone().unwrap_or(defaults.resolution),
        },
    ))
}

/// Update the active profile's provider/api_url/model/label. The classic Settings
/// form now edits whichever model is active, rather than a single global config.
pub async fn set_active_fields(
    store: &wisp_store::Store,
    provider: &str,
    api_url: &str,
    model: &str,
    label: &str,
) -> Result<(), String> {
    let mut profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    if let Some(p) = profiles.iter_mut().find(|p| p.id == id) {
        let current_api_url = effective_api_url(p);
        p.provider = provider.to_string();
        if current_api_url != api_url.trim() {
            p.api_url = api_url.to_string();
            p.endpoint_suffix.clear();
        }
        p.model = model.to_string();
        let alias = label.trim();
        p.label = if alias.is_empty() {
            model.to_string()
        } else {
            alias.to_string()
        };
    }
    save_raw(store, &profiles).await
}

/// Display alias for the active profile (shown in the composer picker).
pub async fn active_label(store: &wisp_store::Store) -> String {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    profiles
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.label.clone())
        .unwrap_or_default()
}

/// Per-model advanced LLM options for the active profile, falling back to
/// legacy global store keys when a profile has no values yet.
pub async fn active_llm_advanced(store: &wisp_store::Store) -> (u64, String) {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    if let Some(p) = profiles.iter().find(|p| p.id == id) {
        let mut max_tokens = p.max_tokens;
        let mut reasoning_effort = p.reasoning_effort.clone();
        if max_tokens == 0 {
            max_tokens = store
                .get_setting("max_tokens")
                .await
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
        if reasoning_effort.is_empty() {
            reasoning_effort = store
                .get_setting("reasoning_effort")
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
        }
        return (max_tokens, reasoning_effort);
    }
    let max_tokens = store
        .get_setting("max_tokens")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let reasoning_effort = store
        .get_setting("reasoning_effort")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    (max_tokens, reasoning_effort)
}

fn effective_context_window(profile: &ModelProfile) -> u64 {
    let value = if profile.context_window >= 4_096 {
        profile.context_window
    } else {
        DEFAULT_CONTEXT_WINDOW
    };
    // Catalog ceiling: an over-declared window defeats compaction, so clamp to
    // the model's documented limit (exact id match; unknown models untouched).
    match crate::model_catalog::lookup(&profile.provider, &profile.api_url, &profile.model) {
        Some(entry) => value.min(entry.c),
        None => value,
    }
}

/// Clamp `context_window`/`max_tokens` to the model's catalog ceilings.
/// `max_tokens = 0` means "unset" and is left alone.
fn clamp_to_catalog(profile: &mut ModelProfile) {
    if let Some(entry) =
        crate::model_catalog::lookup(&profile.provider, &profile.api_url, &profile.model)
    {
        profile.context_window = profile.context_window.min(entry.c);
        profile.max_tokens = profile.max_tokens.min(entry.o);
    }
}

/// Context capacity for the active HTTP model.
pub async fn active_context_window(store: &wisp_store::Store) -> u64 {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    profiles
        .iter()
        .find(|profile| profile.id == id)
        .map(effective_context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Context capacity for a concrete HTTP model profile.
pub async fn profile_context_window(store: &wisp_store::Store, id: &str) -> Option<u64> {
    ensure(store)
        .await
        .iter()
        .find(|profile| profile.id == id && is_chat_model(profile))
        .map(effective_context_window)
}

/// Full LLM config for one profile id: (provider, api_url, model, api_key,
/// max_tokens, reasoning_effort). None when the id doesn't exist.
pub async fn profile_llm(
    store: &wisp_store::Store,
    id: &str,
) -> Option<(String, String, String, String, u64, String)> {
    let profiles = ensure(store).await;
    let p = profiles.iter().find(|p| p.id == id)?;
    if !is_chat_model(p) {
        return None;
    }
    Some((
        p.provider.clone(),
        effective_api_url(p),
        p.model.clone(),
        key_for(&p.id),
        p.max_tokens,
        p.reasoning_effort.clone(),
    ))
}

/// Stored key for a specific profile id, or None when the profile does not
/// exist. The returned string may still be empty when the profile has no key.
pub async fn profile_key(store: &wisp_store::Store, id: &str) -> Option<String> {
    let profiles = ensure(store).await;
    profiles.iter().any(|p| p.id == id).then(|| key_for(id))
}

/// Whether the active profile has a key stored (for `get_settings`).
pub async fn active_has_key(store: &wisp_store::Store) -> bool {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    !key_for(&id).is_empty()
}

pub async fn active_supports_vision(store: &wisp_store::Store) -> bool {
    supports_vision(store, None).await
}

pub async fn supports_vision(store: &wisp_store::Store, profile_id: Option<&str>) -> bool {
    let profiles = ensure(store).await;
    let id = match profile_id.filter(|id| profiles.iter().any(|profile| profile.id == *id)) {
        Some(id) => id.to_string(),
        None => active_id(store, &profiles).await,
    };
    profiles
        .iter()
        .find(|p| p.id == id)
        .is_some_and(can_describe_images)
}

/// Profiles with `has_api_key`/`active` filled in, for the UI.
async fn decorated(store: &wisp_store::Store) -> Vec<ModelProfile> {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    let vision = vision_id(store, &profiles).await;
    let image_generation = image_generation_id(store, &profiles).await;
    let video_generation = video_generation_id(store, &profiles).await;
    profiles
        .into_iter()
        .map(|mut p| {
            p.has_api_key = !key_for(&p.id).is_empty();
            p.active = p.id == id;
            p.use_for_vision = vision.as_deref() == Some(p.id.as_str());
            p.use_for_image_generation = image_generation.as_deref() == Some(p.id.as_str());
            p.use_for_video_generation = video_generation.as_deref() == Some(p.id.as_str());
            p
        })
        .collect()
}

pub(crate) async fn delegation_profiles(store: &wisp_store::Store) -> Vec<ModelProfile> {
    decorated(store)
        .await
        .into_iter()
        .filter(is_chat_model)
        .collect()
}

/// A short unique id derived from the label (or a counter) that isn't taken.
fn fresh_id(existing: &[ModelProfile]) -> String {
    for n in 1..10_000 {
        let id = format!("m{n}");
        if !existing.iter().any(|p| p.id == id) {
            return id;
        }
    }
    "m".into()
}

#[tauri::command]
pub async fn list_models(state: State<'_, crate::AppState>) -> Result<Vec<ModelProfile>, String> {
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn get_session_model(
    state: State<'_, crate::AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<String, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&session_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session not found".into());
    }
    // ACP-bound frames run through the agent, not an HTTP model. Return the
    // agent's label under an `acp:` marker so message badges don't fall back
    // to the active HTTP model.
    if let Ok(Some(binding)) = state.store.get_acp_session(&session_id).await {
        let label = crate::acp::profile_label(&state.store, &binding.agent_profile_id)
            .await
            .unwrap_or_else(|| "ACP Agent".into());
        return Ok(format!("acp:{label}"));
    }
    Ok(session_profile_id(&state.store, &session_id).await)
}

#[tauri::command]
pub async fn get_session_reasoning_effort(
    state: State<'_, crate::AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<Option<String>, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&session_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session not found".into());
    }
    state
        .store
        .frame_reasoning_effort(&session_id)
        .await
        .map_err(|error| error.to_string())
}

/// Upsert a profile. An empty `id` creates a new one; a non-empty `key` updates
/// the keyring (a blank key leaves the stored one untouched).
#[tauri::command]
pub async fn save_model(
    state: State<'_, crate::AppState>,
    mut profile: ModelProfile,
    key: Option<String>,
    use_for_vision: Option<bool>,
    use_for_image_generation: Option<bool>,
    use_for_video_generation: Option<bool>,
) -> Result<Vec<ModelProfile>, String> {
    // Explicit top-level param: the flag nested inside `profile` was observed
    // arriving as false through the webview IPC boundary, losing the
    // assignment on save (#131 follow-up).
    let assign_vision = use_for_vision.unwrap_or(profile.use_for_vision);
    let assign_image_generation =
        use_for_image_generation.unwrap_or(profile.use_for_image_generation);
    let assign_video_generation =
        use_for_video_generation.unwrap_or(profile.use_for_video_generation);
    profile.use_for_vision = assign_vision;
    profile.use_for_image_generation = assign_image_generation;
    profile.use_for_video_generation = assign_video_generation;
    let mut profiles = ensure(&state.store).await;
    if profile.model.trim().is_empty() {
        return Err("Model is required.".into());
    }
    if profile.api_url.trim().is_empty() {
        return Err("API URL is required.".into());
    }
    profile.api_url = profile.api_url.trim().trim_end_matches('/').to_string();
    profile.endpoint_suffix = normalize_endpoint_suffix(&profile.endpoint_suffix)?;
    if assign_vision && !can_describe_images(&profile) {
        return Err("Image analysis requires an API model marked as vision-capable.".into());
    }
    if assign_image_generation && !can_generate_images(&profile) {
        return Err(IMAGE_GENERATION_UNSUPPORTED.into());
    }
    if assign_video_generation && !can_generate_videos(&profile) {
        return Err(VIDEO_GENERATION_UNSUPPORTED.into());
    }
    normalize_image_options(&mut profile)?;
    normalize_video_options(&mut profile);
    clamp_to_catalog(&mut profile);
    if profile.label.trim().is_empty() {
        profile.label = profile.model.clone();
    }
    if profile.id.trim().is_empty() {
        profile.id = fresh_id(&profiles);
    }
    let id = profile.id.clone();
    let api_url = profile.api_url.clone();
    let is_new = !profiles.iter().any(|p| p.id == id);
    if !is_chat_model(&profile) && !profiles.iter().any(|p| p.id != id && is_chat_model(p)) {
        return Err("At least one chat model is required.".into());
    }
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == id) {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    save_raw(&state.store, &profiles).await?;
    store_profile_key(&id, key.as_deref(), &api_url, &profiles)?;
    if assign_vision {
        let _ = state.store.set_setting(VISION_KEY, &id).await;
    } else {
        let cur = state
            .store
            .get_setting(VISION_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if cur == id
            && !profiles
                .iter()
                .any(|p| can_describe_images(p) && p.id != id)
        {
            let _ = state.store.set_setting(VISION_KEY, "").await;
        }
    }
    if assign_image_generation {
        let _ = state.store.set_setting(IMAGE_GENERATION_KEY, &id).await;
    } else {
        let current = state
            .store
            .get_setting(IMAGE_GENERATION_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if current == id {
            let _ = state.store.set_setting(IMAGE_GENERATION_KEY, "").await;
        }
    }
    if assign_video_generation {
        let _ = state.store.set_setting(VIDEO_GENERATION_KEY, &id).await;
    } else {
        let current = state
            .store
            .get_setting(VIDEO_GENERATION_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if current == id {
            let _ = state.store.set_setting(VIDEO_GENERATION_KEY, "").await;
        }
    }
    // Land the user on a freshly added model so they can edit/use it right away.
    if is_new && profiles.iter().any(|p| p.id == id && is_chat_model(p)) {
        let _ = state.store.set_setting(ACTIVE_KEY, &id).await;
    } else if !profiles.iter().any(|p| p.id == id && is_chat_model(p)) {
        let active = state
            .store
            .get_setting(ACTIVE_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if active == id {
            if let Some(first) = profiles.iter().find(|p| is_chat_model(p)) {
                let _ = state.store.set_setting(ACTIVE_KEY, &first.id).await;
            }
        }
    }
    crate::clear_idle_agents(&state).await;
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn remove_model(
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<Vec<ModelProfile>, String> {
    let mut profiles = ensure(&state.store).await;
    if profiles
        .iter()
        .filter(|p| p.id != id && is_chat_model(p))
        .count()
        == 0
    {
        return Err("At least one chat model is required.".into());
    }
    profiles.retain(|p| p.id != id);
    save_raw(&state.store, &profiles).await?;
    let _ = secret_del(&secret_name(&id));
    // If we removed the active profile, fall back to the first remaining one.
    let cur = state
        .store
        .get_setting(ACTIVE_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if cur == id {
        if let Some(first) = profiles.iter().find(|p| is_chat_model(p)) {
            let _ = state.store.set_setting(ACTIVE_KEY, &first.id).await;
        }
    }
    let image_generation = state
        .store
        .get_setting(IMAGE_GENERATION_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if image_generation == id {
        let _ = state.store.set_setting(IMAGE_GENERATION_KEY, "").await;
    }
    let video_generation = state
        .store
        .get_setting(VIDEO_GENERATION_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if video_generation == id {
        let _ = state.store.set_setting(VIDEO_GENERATION_KEY, "").await;
    }
    crate::clear_idle_agents(&state).await;
    Ok(decorated(&state.store).await)
}

/// Reorder `profiles` to match `ids`. Profiles missing from `ids` keep their
/// existing relative order at the end, so a stale client list can never drop a
/// model — it just falls through unmoved. sort_by_key is stable, which is what
/// makes the usize::MAX tail preserve order.
fn reordered(mut profiles: Vec<ModelProfile>, ids: &[String]) -> Vec<ModelProfile> {
    profiles.sort_by_key(|p| ids.iter().position(|id| id == &p.id).unwrap_or(usize::MAX));
    profiles
}

#[tauri::command]
pub async fn reorder_models(
    state: State<'_, crate::AppState>,
    ids: Vec<String>,
) -> Result<Vec<ModelProfile>, String> {
    let profiles = reordered(ensure(&state.store).await, &ids);
    save_raw(&state.store, &profiles).await?;
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn set_active_model(
    state: State<'_, crate::AppState>,
    _window: tauri::WebviewWindow,
    id: String,
    session_id: Option<String>,
) -> Result<Vec<ModelProfile>, String> {
    let profiles = ensure(&state.store).await;
    if !profiles.iter().any(|p| p.id == id) {
        return Err("Unknown model.".into());
    }
    if profiles
        .iter()
        .find(|p| p.id == id)
        .is_some_and(|p| !is_chat_model(p))
    {
        return Err("Image or video generation models cannot be used for chat.".into());
    }
    if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
        let (project, scope) =
            crate::exploration_commands::working_project_for_frame(&state, &session_id).await?;
        let _activity = state.begin_project_activity(&project.id)?;
        let _project_write_locked = crate::exploration_commands::conversation_project_write_locked(
            &state.store,
            &scope,
            Some(&session_id),
        )
        .await?;
        state
            .store
            .set_frame_model(&session_id, &project.id, &id)
            .await
            .map_err(|error| error.to_string())?;
        crate::clear_session_agent(&state, &session_id).await;
    } else {
        state
            .store
            .set_setting(ACTIVE_KEY, &id)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn set_session_reasoning_effort(
    state: State<'_, crate::AppState>,
    effort: String,
    session_id: String,
) -> Result<(), String> {
    let effort = effort.trim();
    if !effort.is_empty()
        && !matches!(
            effort,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        )
    {
        return Err("Unknown reasoning effort.".into());
    }
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, &session_id).await?;
    let _activity = state.begin_project_activity(&project.id)?;
    let _project_write_locked = crate::exploration_commands::conversation_project_write_locked(
        &state.store,
        &scope,
        Some(&session_id),
    )
    .await?;
    // Empty effort clears the override: the session inherits the bound model
    // profile again instead of pinning "provider default" forever.
    let override_value = if effort.is_empty() {
        None
    } else {
        Some(effort)
    };
    state
        .store
        .set_frame_reasoning_effort(&session_id, &project.id, override_value)
        .await
        .map_err(|error| error.to_string())?;
    crate::clear_session_agent(&state, &session_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile(id: &str, label: &str, model: &str) -> ModelProfile {
        ModelProfile {
            id: id.into(),
            label: label.into(),
            provider: "openai".into(),
            api_url: "u".into(),
            endpoint_suffix: String::new(),
            model: model.into(),
            has_api_key: false,
            active: false,
            max_tokens: 0,
            context_window: DEFAULT_CONTEXT_WINDOW,
            reasoning_effort: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
        }
    }

    #[tokio::test]
    async fn save_then_reload_keeps_vision_assignment() {
        // repro for "checkbox lost after save+reopen": full backend round-trip
        // through save_raw + VISION_KEY + decorated.
        let tmp = std::env::temp_dir().join(format!("wisp_vision_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let mut p = test_profile("m1", "claude", "claude-opus-4-8");
        p.supports_vision = true;
        save_raw(&store, &[test_profile("m0", "text", "deepseek"), p])
            .await
            .unwrap();
        store.set_setting(VISION_KEY, "m1").await.unwrap();
        let out = decorated(&store).await;
        let m1 = out.iter().find(|p| p.id == "m1").unwrap();
        assert!(m1.supports_vision, "capability lost in persistence");
        assert!(m1.use_for_vision, "vision assignment lost after reload");
        assert!(!out.iter().find(|p| p.id == "m0").unwrap().use_for_vision);
        let json = serde_json::to_value(out).unwrap();
        assert_eq!(
            json[1]["use_for_vision"], true,
            "IPC response lost vision assignment"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn reordered_moves_named_and_keeps_unlisted_at_end() {
        let ids = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let src = vec![
            test_profile("a", "a", "a"),
            test_profile("b", "b", "b"),
            test_profile("c", "c", "c"),
        ];
        // Full reversal.
        let out = reordered(src.clone(), &ids(&["c", "b", "a"]));
        assert_eq!(
            out.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["c", "b", "a"]
        );
        // Only "c" named: it leads, unlisted a/b keep their original order.
        let out = reordered(src.clone(), &ids(&["c"]));
        assert_eq!(
            out.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
        // Unknown id in the list is ignored, real ids still reorder.
        let out = reordered(src, &ids(&["ghost", "b", "a"]));
        assert_eq!(
            out.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
    }

    #[tokio::test]
    async fn session_profile_binding_does_not_change_other_sessions() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_session_models_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "project", "").await.unwrap();
        save_raw(
            &store,
            &[
                test_profile("m1", "first", "model-1"),
                test_profile("m2", "second", "model-2"),
            ],
        )
        .await
        .unwrap();
        store.set_setting(ACTIVE_KEY, "m1").await.unwrap();
        store.create_frame("a", "p", "OPERON", "m1").await.unwrap();
        store.create_frame("b", "p", "OPERON", "m1").await.unwrap();

        store.set_frame_model("a", "p", "m2").await.unwrap();

        assert_eq!(session_profile_id(&store, "a").await, "m2");
        assert_eq!(session_profile_id(&store, "b").await, "m1");
        drop(store);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn session_reasoning_override_does_not_change_profile_or_sibling() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_session_reasoning_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "project", "").await.unwrap();
        let mut profile = test_profile("m1", "reasoner", "model-1");
        profile.reasoning_effort = "max".into();
        save_raw(&store, &[profile]).await.unwrap();
        store.set_setting(ACTIVE_KEY, "m1").await.unwrap();
        store.create_frame("a", "p", "OPERON", "m1").await.unwrap();
        store.create_frame("b", "p", "OPERON", "m1").await.unwrap();

        store
            .set_frame_reasoning_effort("a", "p", Some("high"))
            .await
            .unwrap();

        assert_eq!(session_reasoning_effort(&store, "a", "max").await, "high");
        assert_eq!(session_reasoning_effort(&store, "b", "max").await, "max");
        assert_eq!(profile_llm(&store, "m1").await.unwrap().5, "max");
        drop(store);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn cleared_or_empty_session_override_inherits_profile() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_session_reasoning_clear_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "project", "").await.unwrap();
        store.create_frame("a", "p", "OPERON", "m1").await.unwrap();
        store.create_frame("b", "p", "OPERON", "m1").await.unwrap();

        store
            .set_frame_reasoning_effort("a", "p", Some("high"))
            .await
            .unwrap();
        assert_eq!(session_reasoning_effort(&store, "a", "max").await, "high");
        // Clearing the override (selecting "default" in the composer) makes
        // the session follow the profile default again.
        store
            .set_frame_reasoning_effort("a", "p", None)
            .await
            .unwrap();
        assert_eq!(session_reasoning_effort(&store, "a", "max").await, "max");
        // Legacy rows hold Some("") for "provider default"; they must not pin
        // the session away from the profile either.
        store
            .set_frame_reasoning_effort("b", "p", Some(""))
            .await
            .unwrap();
        assert_eq!(session_reasoning_effort(&store, "b", "max").await, "max");
        drop(store);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn vision_capability_follows_the_input_profile() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_input_vision_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let text = test_profile("m0", "text", "text-model");
        let mut vision = test_profile("m1", "vision", "vision-model");
        vision.supports_vision = true;
        save_raw(&store, &[text, vision]).await.unwrap();
        store.set_setting(ACTIVE_KEY, "m0").await.unwrap();

        assert!(!supports_vision(&store, None).await);
        assert!(supports_vision(&store, Some("m1")).await);
        assert!(!supports_vision(&store, Some("missing")).await);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn use_for_vision_survives_deserialization() {
        // Repro for the "checkbox lost after save" report: the incoming
        // command payload must keep both role assignments.
        let p: ModelProfile = serde_json::from_str(
            r#"{"id":"m1","label":"l","provider":"anthropic","api_url":"u","model":"m",
                "max_tokens":8192,"reasoning_effort":"medium",
                "supports_vision":true,"use_for_vision":true,
                "use_for_image_generation":true}"#,
        )
        .unwrap();
        assert!(p.supports_vision);
        assert!(p.use_for_vision, "use_for_vision dropped on deserialize");
        assert!(
            p.use_for_image_generation,
            "use_for_image_generation dropped on deserialize"
        );
        assert_eq!(p.context_window, DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn older_profiles_without_label_still_load() {
        let profiles: Vec<ModelProfile> = serde_json::from_str(
            r#"[{"id":"m1","provider":"openai","api_url":"https://api.deepseek.com","model":"deepseek-v4-flash"}]"#,
        )
        .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "m1");
        assert!(profiles[0].label.is_empty());
    }

    #[test]
    fn context_window_survives_profile_roundtrip() {
        let mut profile = test_profile("m1", "reader", "cheap-reader");
        profile.context_window = 32_768;
        let json = serde_json::to_string(&profile).unwrap();
        let restored: ModelProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.context_window, 32_768);
    }

    fn kimi_coding_profile(model: &str) -> ModelProfile {
        let mut profile = test_profile("m1", "kimi", model);
        profile.api_url = "https://api.kimi.com/coding/v1".into();
        profile
    }

    #[test]
    fn clamp_to_catalog_caps_over_declared_limits() {
        let mut profile = kimi_coding_profile("k3-256k");
        profile.context_window = 1_000_000;
        profile.max_tokens = 999_999;
        clamp_to_catalog(&mut profile);
        assert_eq!(profile.context_window, 262_144);
        assert_eq!(profile.max_tokens, 131_072);
    }

    #[test]
    fn clamp_to_catalog_keeps_unset_and_unknown_values() {
        // max_tokens = 0 means "unset" and stays.
        let mut profile = kimi_coding_profile("k3-256k");
        profile.max_tokens = 0;
        clamp_to_catalog(&mut profile);
        assert_eq!(profile.max_tokens, 0);
        // Unknown models are left alone.
        let mut unknown = test_profile("m2", "x", "totally-unknown");
        unknown.context_window = 500_000;
        unknown.max_tokens = 90_000;
        clamp_to_catalog(&mut unknown);
        assert_eq!(unknown.context_window, 500_000);
        assert_eq!(unknown.max_tokens, 90_000);
    }

    #[test]
    fn effective_context_window_respects_catalog_ceiling() {
        let mut over = kimi_coding_profile("k3-256k");
        over.context_window = 1_000_000;
        assert_eq!(effective_context_window(&over), 262_144);
        // Unknown models keep their declared value.
        let mut unknown = test_profile("m2", "x", "totally-unknown");
        unknown.context_window = 500_000;
        assert_eq!(effective_context_window(&unknown), 500_000);
        // Degenerate values still fall back to the default window.
        unknown.context_window = 100;
        assert_eq!(effective_context_window(&unknown), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn fresh_id_skips_taken() {
        let existing = vec![test_profile("m1", "a", "x"), test_profile("m2", "b", "y")];
        assert_eq!(fresh_id(&existing), "m3");
        assert_eq!(fresh_id(&[]), "m1");
    }

    #[test]
    fn vision_assignment_marker_is_serialized_for_ui() {
        let mut profile = test_profile("m1", "vision", "v");
        profile.supports_vision = true;
        profile.use_for_vision = true;
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("supports_vision"));
        assert!(json.contains("\"use_for_vision\":true"));
    }

    #[test]
    fn vision_capability_uses_marker() {
        let mut profile = test_profile("m1", "vision", "v");
        profile.supports_vision = true;
        assert!(can_describe_images(&profile));
        profile.supports_vision = false;
        assert!(!can_describe_images(&profile));
        profile.supports_vision = true;
        profile.model = "gpt-image-2".into();
        assert!(!can_describe_images(&profile));
    }

    #[test]
    fn image_generation_accepts_openai_and_xai_models() {
        let mut profile = test_profile("image", "image", "gpt-image-2");
        profile.provider = "openai_responses".into();
        assert!(can_generate_images(&profile));

        profile.model = "grok-imagine-image-2.0".into();
        profile.provider = "openai".into();
        assert!(can_generate_images(&profile));
        profile.model = "xai/grok-imagine-image-2.0".into();
        assert!(can_generate_images(&profile));

        profile.provider = "anthropic".into();
        assert!(!can_generate_images(&profile));
        profile.provider = "openai".into();
        profile.model = "gpt-image-1".into();
        assert!(!can_generate_images(&profile));
        profile.model = "grok-imagine-image".into();
        assert!(!can_generate_images(&profile));
        assert!(!is_chat_model(&test_profile(
            "image",
            "image",
            "grok-imagine-image-2.0"
        )));
    }

    #[test]
    fn image_generation_options_are_normalized_per_family() {
        let mut grok = test_profile("image", "image", "grok-imagine-image-2.0");
        grok.image_aspect_ratio = "16:9".into();
        grok.image_resolution = "2k".into();
        grok.image_quality = "low".into();
        grok.image_size = "1024x1024".into();
        normalize_image_options(&mut grok).unwrap();
        assert!(grok.image_size.is_empty());
        assert_eq!(grok.image_aspect_ratio, "16:9");
        assert_eq!(grok.image_resolution, "2k");

        grok.image_aspect_ratio = "square".into();
        assert!(normalize_image_options(&mut grok).is_err());

        let mut openai = test_profile("image", "image", "gpt-image-2");
        openai.image_size = "1536x1024".into();
        openai.image_quality = "high".into();
        openai.image_aspect_ratio = "16:9".into();
        normalize_image_options(&mut openai).unwrap();
        assert_eq!(openai.image_size, "1536x1024");
        assert!(openai.image_aspect_ratio.is_empty());
    }

    #[tokio::test]
    async fn image_generation_requires_an_explicit_gpt_image_2_assignment() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_image_gen_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let chat = test_profile("chat", "chat", "gpt-5.5");
        let mut image = test_profile("image", "image", "gpt-image-2");
        image.provider = "openai_responses".into();
        save_raw(&store, &[chat, image]).await.unwrap();

        assert!(image_generation_config(&store).await.is_none());
        store
            .set_setting(IMAGE_GENERATION_KEY, "image")
            .await
            .unwrap();
        let (url, model, _key, options) = image_generation_config(&store).await.unwrap();
        assert_eq!(options, ImageGenerationOptions::default());
        assert_eq!(url, "u");
        assert_eq!(model, "gpt-image-2");

        let decorated = decorated(&store).await;
        assert!(
            decorated
                .iter()
                .find(|profile| profile.id == "image")
                .unwrap()
                .use_for_image_generation
        );
        assert_eq!(
            delegation_profiles(&store)
                .await
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            ["chat"]
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn video_generation_matches_exact_model_ids() {
        for model in [
            "grok-imagine-video",
            "Grok-Imagine-Video",
            "grok-imagine-video-1.5",
            "grok-imagine-video-1.5-preview",
            "xai/grok-imagine-video-1.5-preview",
        ] {
            assert!(is_video_generation_model(model), "{model}");
        }
        // Exact ids only: the base id must not absorb longer siblings, and
        // unrelated models (chat, image) must not match.
        for model in [
            "grok-imagine-video-2.0",
            "grok-imagine-video-1.5-preview-2",
            "grok-imagine-image-2.0",
            "gpt-image-2",
            "gpt-5.5",
        ] {
            assert!(!is_video_generation_model(model), "{model}");
        }
        assert!(!is_chat_model(&test_profile(
            "video",
            "video",
            "grok-imagine-video-1.5-preview"
        )));
    }

    #[test]
    fn video_generation_requires_a_compatible_provider() {
        let mut profile = test_profile("video", "video", "grok-imagine-video");
        assert!(can_generate_videos(&profile));
        profile.provider = "openai_compatible".into();
        assert!(can_generate_videos(&profile));
        profile.provider = "anthropic".into();
        assert!(!can_generate_videos(&profile));
        profile.provider = "openai".into();
        profile.model = "grok-imagine-video-2.0".into();
        assert!(!can_generate_videos(&profile));
    }

    #[test]
    fn video_options_are_normalized() {
        let mut video = test_profile("video", "video", "grok-imagine-video");
        video.video_duration_secs = Some(42);
        video.video_aspect_ratio = Some(" 9:16 ".into());
        video.video_resolution = Some("4k".into());
        normalize_video_options(&mut video);
        assert_eq!(video.video_duration_secs, Some(15));
        assert_eq!(video.video_aspect_ratio.as_deref(), Some("9:16"));
        assert_eq!(video.video_resolution, None);

        video.video_duration_secs = Some(0);
        normalize_video_options(&mut video);
        assert_eq!(video.video_duration_secs, Some(1));

        // Non-video profiles drop any leftover video options.
        let mut chat = test_profile("chat", "chat", "gpt-5.5");
        chat.video_duration_secs = Some(5);
        chat.video_aspect_ratio = Some("16:9".into());
        normalize_video_options(&mut chat);
        assert_eq!(chat.video_duration_secs, None);
        assert_eq!(chat.video_aspect_ratio, None);
    }

    #[tokio::test]
    async fn video_generation_requires_an_explicit_assignment() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_video_gen_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let chat = test_profile("chat", "chat", "gpt-5.5");
        let mut video = test_profile("video", "video", "grok-imagine-video-1.5");
        video.video_duration_secs = Some(10);
        video.video_aspect_ratio = Some("9:16".into());
        save_raw(&store, &[chat, video]).await.unwrap();

        assert!(video_generation_config(&store).await.is_none());
        store
            .set_setting(VIDEO_GENERATION_KEY, "video")
            .await
            .unwrap();
        let (url, model, _key, options) = video_generation_config(&store).await.unwrap();
        assert_eq!(url, "u");
        assert_eq!(model, "grok-imagine-video-1.5");
        assert_eq!(options.duration_secs, 10);
        assert_eq!(options.aspect_ratio, "9:16");
        assert_eq!(options.resolution, "720p");

        let decorated = decorated(&store).await;
        assert!(
            decorated
                .iter()
                .find(|profile| profile.id == "video")
                .unwrap()
                .use_for_video_generation
        );
        assert_eq!(
            delegation_profiles(&store)
                .await
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            ["chat"]
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn video_generation_config_falls_back_to_option_defaults() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_video_gen_defaults_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        save_raw(
            &store,
            &[
                test_profile("chat", "chat", "gpt-5.5"),
                test_profile("video", "video", "grok-imagine-video"),
            ],
        )
        .await
        .unwrap();
        store
            .set_setting(VIDEO_GENERATION_KEY, "video")
            .await
            .unwrap();

        let (_url, _model, _key, options) = video_generation_config(&store).await.unwrap();
        assert_eq!(options, VideoGenerationOptions::default());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn use_for_video_generation_survives_deserialization() {
        let p: ModelProfile = serde_json::from_str(
            r#"{"id":"m1","label":"l","provider":"openai","api_url":"u","model":"grok-imagine-video",
                "use_for_video_generation":true,"video_duration_secs":8,
                "video_aspect_ratio":"9:16","video_resolution":"1080p"}"#,
        )
        .unwrap();
        assert!(p.use_for_video_generation);
        assert_eq!(p.video_duration_secs, Some(8));
        assert_eq!(p.video_aspect_ratio.as_deref(), Some("9:16"));
        assert_eq!(p.video_resolution.as_deref(), Some("1080p"));
    }

    #[tokio::test]
    async fn image_generation_profile_is_never_the_active_chat_model() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_image_gen_active_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let chat = test_profile("chat", "chat", "gpt-5.5");
        let image = test_profile("image", "image", "gpt-image-2");
        save_raw(&store, &[chat, image]).await.unwrap();
        store.set_setting(ACTIVE_KEY, "image").await.unwrap();

        assert_eq!(active_profile_id(&store).await, "chat");
        assert_eq!(profile_context_window(&store, "image").await, None);
        let decorated = decorated(&store).await;
        assert!(
            decorated
                .iter()
                .find(|profile| profile.id == "chat")
                .unwrap()
                .active
        );
        assert!(
            !decorated
                .iter()
                .find(|profile| profile.id == "image")
                .unwrap()
                .active
        );
        let _ = std::fs::remove_file(&tmp);
    }

    // The write-through cache must stay coherent: a set is readable without a
    // fresh keyring hit, and a delete reads back as absent (not the old value).
    // `cargo test` builds with debug_assertions, so the secret backend is the
    // dev secrets file in $HOME, never a real OS keyring; the UUID-scoped name
    // keeps concurrent or leftover runs sharing $HOME from colliding.
    #[test]
    fn secret_cache_write_through() {
        let name = format!(
            "model_key:__cache_coherence_test_{}__",
            uuid::Uuid::new_v4()
        );
        secret_set(&name, "sk-abc").unwrap();
        assert_eq!(secret_get(&name), "sk-abc");
        secret_del(&name).unwrap();
        assert_eq!(secret_get(&name), "");
    }

    #[test]
    fn normalize_endpoint_strips_version_and_api_suffixes() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_endpoint("https://API.OpenAI.com/v1/"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1/responses"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            normalize_endpoint("https://api.kimi.com/coding/v1"),
            "https://api.kimi.com/coding"
        );
        assert!(same_endpoint(
            "https://api.openai.com",
            "https://api.openai.com/v1"
        ));
        assert!(!same_endpoint(
            "https://api.deepseek.com",
            "https://api.openai.com"
        ));
    }

    #[test]
    fn endpoint_suffix_is_normalized_and_joined_to_the_shared_root() {
        assert_eq!(
            normalize_endpoint_suffix(" anthropic/ ").unwrap(),
            "/anthropic"
        );
        assert_eq!(normalize_endpoint_suffix("/").unwrap(), "");
        assert_eq!(
            join_api_url("https://api.deepseek.com/", "/anthropic"),
            "https://api.deepseek.com/anthropic"
        );
        assert!(normalize_endpoint_suffix("https://other.example/v1").is_err());
        assert!(normalize_endpoint_suffix("/anthropic?version=1").is_err());
        assert!(normalize_endpoint_suffix("/../anthropic").is_err());
    }

    #[tokio::test]
    async fn runtime_configs_use_the_per_model_endpoint_suffix() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_model_endpoint_suffix_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();

        let mut anthropic = test_profile("anthropic", "deepseek-anthropic", "deepseek-chat");
        anthropic.provider = "anthropic".into();
        anthropic.api_url = "https://api.deepseek.com".into();
        anthropic.endpoint_suffix = "/anthropic".into();

        let mut image = test_profile("image", "image", "gpt-image-2");
        image.provider = "openai_responses".into();
        image.api_url = "https://api.openai.com".into();
        image.endpoint_suffix = "/v1/images/generations".into();

        save_raw(&store, &[anthropic, image]).await.unwrap();
        store.set_setting(ACTIVE_KEY, "anthropic").await.unwrap();
        store
            .set_setting(IMAGE_GENERATION_KEY, "image")
            .await
            .unwrap();

        assert_eq!(
            active_config(&store).await.1,
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            profile_llm(&store, "anthropic").await.unwrap().1,
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            image_generation_config(&store).await.unwrap().0,
            "https://api.openai.com/v1/images/generations"
        );

        drop(store);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn new_profile_inherits_sibling_key_on_the_same_endpoint() {
        let prefix = uuid::Uuid::new_v4();
        let existing_id = format!("{prefix}-a");
        let new_id = format!("{prefix}-b");
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
        secret_set(&secret_name(&existing_id), "sk-shared").unwrap();
        let mut existing = test_profile(&existing_id, "flash", "deepseek-v4-flash");
        existing.api_url = "https://api.deepseek.com".into();
        let mut added = test_profile(&new_id, "pro", "deepseek-v4-pro");
        added.api_url = "https://api.deepseek.com/".into();
        let added_url = added.api_url.clone();
        store_profile_key(&new_id, None, &added_url, &[existing, added]).unwrap();
        assert_eq!(key_for(&new_id), "sk-shared");
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
    }

    #[test]
    fn new_profile_does_not_inherit_a_key_from_another_endpoint() {
        let prefix = uuid::Uuid::new_v4();
        let existing_id = format!("{prefix}-a");
        let new_id = format!("{prefix}-b");
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
        secret_set(&secret_name(&existing_id), "sk-deepseek").unwrap();
        let mut existing = test_profile(&existing_id, "flash", "deepseek-v4-flash");
        existing.api_url = "https://api.deepseek.com".into();
        let mut added = test_profile(&new_id, "gpt", "gpt-5.5");
        added.api_url = "https://api.openai.com/v1".into();
        let added_url = added.api_url.clone();
        store_profile_key(&new_id, None, &added_url, &[existing, added]).unwrap();
        assert!(key_for(&new_id).is_empty());
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
    }

    #[test]
    fn pasted_key_on_existing_profile_rotates_siblings_that_share_that_key() {
        let prefix = uuid::Uuid::new_v4();
        let first_id = format!("{prefix}-a");
        let second_id = format!("{prefix}-b");
        let _ = secret_del(&secret_name(&first_id));
        let _ = secret_del(&secret_name(&second_id));
        secret_set(&secret_name(&first_id), "sk-old").unwrap();
        secret_set(&secret_name(&second_id), "sk-old").unwrap();
        let mut first = test_profile(&first_id, "flash", "deepseek-v4-flash");
        first.api_url = "https://api.deepseek.com".into();
        let mut second = test_profile(&second_id, "pro", "deepseek-v4-pro");
        second.api_url = "https://api.deepseek.com/v1".into();
        let second_url = second.api_url.clone();
        store_profile_key(&second_id, Some("sk-new"), &second_url, &[first, second]).unwrap();
        assert_eq!(key_for(&first_id), "sk-new");
        assert_eq!(key_for(&second_id), "sk-new");
        let _ = secret_del(&secret_name(&first_id));
        let _ = secret_del(&secret_name(&second_id));
    }

    #[test]
    fn pasted_key_on_new_profile_does_not_overwrite_an_existing_batch() {
        let prefix = uuid::Uuid::new_v4();
        let existing_id = format!("{prefix}-a");
        let new_id = format!("{prefix}-b");
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
        secret_set(&secret_name(&existing_id), "sk-one").unwrap();
        let mut existing = test_profile(&existing_id, "flash", "deepseek-v4-flash");
        existing.api_url = "https://api.deepseek.com".into();
        let mut added = test_profile(&new_id, "pro", "deepseek-v4-pro");
        added.api_url = "https://api.deepseek.com".into();
        let added_url = added.api_url.clone();
        store_profile_key(&new_id, Some("sk-two"), &added_url, &[existing, added]).unwrap();
        assert_eq!(key_for(&existing_id), "sk-one");
        assert_eq!(key_for(&new_id), "sk-two");
        let _ = secret_del(&secret_name(&existing_id));
        let _ = secret_del(&secret_name(&new_id));
    }

    #[test]
    fn rotating_one_key_leaves_a_different_key_on_the_same_endpoint() {
        let prefix = uuid::Uuid::new_v4();
        let one_a = format!("{prefix}-a");
        let one_b = format!("{prefix}-b");
        let two_id = format!("{prefix}-c");
        for id in [&one_a, &one_b, &two_id] {
            let _ = secret_del(&secret_name(id));
        }
        secret_set(&secret_name(&one_a), "sk-one").unwrap();
        secret_set(&secret_name(&one_b), "sk-one").unwrap();
        secret_set(&secret_name(&two_id), "sk-two").unwrap();
        let mut first = test_profile(&one_a, "flash", "deepseek-v4-flash");
        first.api_url = "https://api.deepseek.com".into();
        let mut second = test_profile(&one_b, "pro", "deepseek-v4-pro");
        second.api_url = "https://api.deepseek.com".into();
        let mut other = test_profile(&two_id, "other", "deepseek-v4-flash");
        other.api_url = "https://api.deepseek.com".into();
        let first_url = first.api_url.clone();
        store_profile_key(
            &one_a,
            Some("sk-one-rotated"),
            &first_url,
            &[first, second, other],
        )
        .unwrap();
        assert_eq!(key_for(&one_a), "sk-one-rotated");
        assert_eq!(key_for(&one_b), "sk-one-rotated");
        assert_eq!(key_for(&two_id), "sk-two");
        for id in [&one_a, &one_b, &two_id] {
            let _ = secret_del(&secret_name(id));
        }
    }

    /// Restores each captured secret on drop (even when an assert panics), so
    /// the test never clobbers values a developer keeps in the shared dev
    /// secrets file.
    struct RestoreSecrets(Vec<(String, String)>);

    impl RestoreSecrets {
        fn capture(ids: &[&str]) -> Self {
            Self(
                ids.iter()
                    .map(|id| {
                        let secret = credential(id).unwrap().secret.to_string();
                        let prior = secret_get(&secret);
                        (secret, prior)
                    })
                    .collect(),
            )
        }
    }

    impl Drop for RestoreSecrets {
        fn drop(&mut self) {
            for (secret, prior) in &self.0 {
                if prior.is_empty() {
                    let _ = secret_del(secret);
                } else {
                    let _ = secret_set(secret, prior);
                }
            }
        }
    }

    // Storing a credential surfaces it in service_env under its env var;
    // clearing removes it; an unknown id is rejected. Registry ids are fixed
    // production names, so prior values are captured and restored on exit.
    #[test]
    fn credential_registry_roundtrip() {
        let _restore =
            RestoreSecrets::capture(&["ncbi_email", "infinisynapse_api_key", "scimaster_api_key"]);

        store_credential("ncbi_email", "me@lab.org").unwrap();
        assert!(credential_status()
            .iter()
            .any(|(id, ok)| id == "ncbi_email" && *ok));
        assert!(service_env()
            .iter()
            .any(|(k, v)| k == "NCBI_EMAIL" && v == "me@lab.org"));

        store_credential("infinisynapse_api_key", "sk-infini").unwrap();
        assert!(service_env()
            .iter()
            .any(|(k, v)| k == "INFINISYNAPSE_API_KEY" && v == "sk-infini"));
        store_credential("infinisynapse_api_key", "").unwrap();

        store_credential("scimaster_api_key", "sk-sci").unwrap();
        assert!(service_env()
            .iter()
            .any(|(k, v)| k == "SCIMASTER_API_KEY" && v == "sk-sci"));
        store_credential("scimaster_api_key", "").unwrap();

        store_credential("ncbi_email", "  ").unwrap(); // blank clears
        assert!(!service_env().iter().any(|(k, _)| k == "NCBI_EMAIL"));

        assert!(store_credential("nonexistent", "x").is_err());
    }

    #[tokio::test]
    async fn custom_credentials_keep_values_out_of_sqlite_and_join_service_env() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_custom_credentials_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let suffix = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .to_ascii_uppercase();
        let env_var = format!("WISP_CUSTOM_TEST_{suffix}");
        let secret = format!("custom-secret-{suffix}");

        assert!(
            add_custom_credential(&store, "MetaSo", "BAD-NAME", "secret")
                .await
                .unwrap_err()
                .contains("Environment variable")
        );
        assert!(
            add_custom_credential(&store, "Duplicate", "OPENALEX_API_KEY", "secret")
                .await
                .unwrap_err()
                .contains("already uses")
        );

        let saved = add_custom_credential(&store, "MetaSo", &env_var, &secret)
            .await
            .unwrap();
        assert!(saved.present);
        assert_eq!(saved.env_var, env_var);
        assert!(custom_credential_status(&store)
            .await
            .unwrap()
            .iter()
            .any(|credential| credential.id == saved.id && credential.present));
        assert!(service_env()
            .iter()
            .any(|(name, value)| name == &env_var && value == &secret));

        let raw = store
            .get_setting(CUSTOM_CREDENTIALS_KEY)
            .await
            .unwrap()
            .unwrap();
        assert!(raw.contains("MetaSo"));
        assert!(raw.contains(&env_var));
        assert!(!raw.contains(&secret));

        store_credential(&saved.id, "replacement").unwrap();
        assert!(service_env()
            .iter()
            .any(|(name, value)| name == &env_var && value == "replacement"));

        // Re-adding the same env var upserts the existing row instead of
        // erroring, even after its value was cleared (#335).
        store_credential(&saved.id, "").unwrap();
        let updated = add_custom_credential(&store, "MetaSo v2", &env_var, "second")
            .await
            .unwrap();
        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.name, "MetaSo v2");
        assert!(updated.present);
        assert!(service_env()
            .iter()
            .any(|(name, value)| name == &env_var && value == "second"));
        assert_eq!(custom_credential_status(&store).await.unwrap().len(), 1);

        remove_custom_credential(&store, &saved.id).await.unwrap();
        assert!(custom_credential_status(&store).await.unwrap().is_empty());
        assert!(!service_env().iter().any(|(name, _)| name == &env_var));
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn profile_key_reads_the_requested_profile() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_profile_key_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        save_raw(
            &store,
            &[
                test_profile("default", "deepseek", "deepseek-v4-pro"),
                test_profile("glm", "glm", "glm-5.2"),
            ],
        )
        .await
        .unwrap();
        secret_set(&secret_name("default"), "sk-default").unwrap();
        secret_set(&secret_name("glm"), "sk-glm").unwrap();

        assert_eq!(profile_key(&store, "glm").await.as_deref(), Some("sk-glm"));
        assert_eq!(
            profile_key(&store, "default").await.as_deref(),
            Some("sk-default")
        );
        assert_eq!(profile_key(&store, "missing").await, None);

        let _ = secret_del(&secret_name("default"));
        let _ = secret_del(&secret_name("glm"));
        let _ = std::fs::remove_file(&tmp);
    }
}
