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
    pub label: String,
    pub provider: String,
    pub api_url: String,
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
}

const PROFILES_KEY: &str = "model_profiles";
const ACTIVE_KEY: &str = "active_model_id";
const VISION_KEY: &str = "vision_model_id";
const LEGACY_KEY_SECRET: &str = "api_key";
const CUSTOM_CREDENTIALS_KEY: &str = "custom_credentials";
const CUSTOM_CREDENTIAL_SECRET_PREFIX: &str = "custom_credential:";

fn secret_name(id: &str) -> String {
    format!("model_key:{id}")
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
        model,
        has_api_key: false,
        active: false,
        max_tokens,
        context_window: DEFAULT_CONTEXT_WINDOW,
        reasoning_effort,
        supports_vision: false,
        use_for_vision: false,
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
    if profiles.iter().any(|p| p.id == want) {
        want
    } else {
        profiles.first().map(|p| p.id.clone()).unwrap_or_default()
    }
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
    (p.provider, p.api_url, p.model, key_for(&p.id))
}

fn can_describe_images(p: &ModelProfile) -> bool {
    p.supports_vision
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

/// The assigned vision profile's `(provider, api_url, model, api_key,
/// max_tokens, reasoning_effort)`, if the user configured one.
pub async fn vision_config(
    store: &wisp_store::Store,
) -> Option<(String, String, String, String, u64, String)> {
    let profiles = ensure(store).await;
    let id = vision_id(store, &profiles).await?;
    let p = profiles.iter().find(|p| p.id == id)?.clone();
    Some((
        p.provider,
        p.api_url,
        p.model,
        key_for(&p.id),
        p.max_tokens,
        p.reasoning_effort,
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
        p.provider = provider.to_string();
        p.api_url = api_url.to_string();
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

/// Set (or clear, when empty) the active profile's key in the keyring.
pub async fn set_active_key(store: &wisp_store::Store, key: &str) -> Result<(), String> {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    let name = secret_name(&id);
    if key.trim().is_empty() {
        secret_del(&name)
    } else {
        secret_set(&name, key.trim())
    }
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

fn effective_context_window(value: u64) -> u64 {
    if value >= 4_096 {
        value
    } else {
        DEFAULT_CONTEXT_WINDOW
    }
}

/// Context capacity for the active HTTP model.
pub async fn active_context_window(store: &wisp_store::Store) -> u64 {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    profiles
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| effective_context_window(profile.context_window))
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Context capacity for a concrete HTTP model profile.
pub async fn profile_context_window(store: &wisp_store::Store, id: &str) -> Option<u64> {
    ensure(store)
        .await
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| effective_context_window(profile.context_window))
}

/// Full LLM config for one profile id: (provider, api_url, model, api_key,
/// max_tokens, reasoning_effort). None when the id doesn't exist.
pub async fn profile_llm(
    store: &wisp_store::Store,
    id: &str,
) -> Option<(String, String, String, String, u64, String)> {
    let profiles = ensure(store).await;
    let p = profiles.iter().find(|p| p.id == id)?;
    Some((
        p.provider.clone(),
        p.api_url.clone(),
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
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
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
    profiles
        .into_iter()
        .map(|mut p| {
            p.has_api_key = !key_for(&p.id).is_empty();
            p.active = p.id == id;
            p.use_for_vision = vision.as_deref() == Some(p.id.as_str());
            p
        })
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

/// Upsert a profile. An empty `id` creates a new one; a non-empty `key` updates
/// the keyring (a blank key leaves the stored one untouched).
#[tauri::command]
pub async fn save_model(
    state: State<'_, crate::AppState>,
    mut profile: ModelProfile,
    key: Option<String>,
    use_for_vision: Option<bool>,
) -> Result<Vec<ModelProfile>, String> {
    // Explicit top-level param: the flag nested inside `profile` was observed
    // arriving as false through the webview IPC boundary, losing the
    // assignment on save (#131 follow-up).
    let assign_vision = use_for_vision.unwrap_or(profile.use_for_vision);
    profile.use_for_vision = assign_vision;
    let mut profiles = ensure(&state.store).await;
    if profile.model.trim().is_empty() {
        return Err("Model is required.".into());
    }
    if profile.api_url.trim().is_empty() {
        return Err("API URL is required.".into());
    }
    if assign_vision && !can_describe_images(&profile) {
        return Err("Image analysis requires an API model marked as vision-capable.".into());
    }
    if profile.label.trim().is_empty() {
        profile.label = profile.model.clone();
    }
    if profile.id.trim().is_empty() {
        profile.id = fresh_id(&profiles);
    }
    let id = profile.id.clone();
    let is_new = !profiles.iter().any(|p| p.id == id);
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == id) {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    save_raw(&state.store, &profiles).await?;
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
    if let Some(k) = key {
        let k = k.trim();
        if !k.is_empty() {
            secret_set(&secret_name(&id), k)?;
        }
    }
    // Land the user on a freshly added model so they can edit/use it right away.
    if is_new {
        let _ = state.store.set_setting(ACTIVE_KEY, &id).await;
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
    if profiles.len() <= 1 {
        return Err("At least one model is required.".into());
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
        if let Some(first) = profiles.first() {
            let _ = state.store.set_setting(ACTIVE_KEY, &first.id).await;
        }
    }
    crate::clear_idle_agents(&state).await;
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn set_active_model(
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<Vec<ModelProfile>, String> {
    let profiles = ensure(&state.store).await;
    if !profiles.iter().any(|p| p.id == id) {
        return Err("Unknown model.".into());
    }
    state
        .store
        .set_setting(ACTIVE_KEY, &id)
        .await
        .map_err(|e| e.to_string())?;
    crate::clear_idle_agents(&state).await;
    Ok(decorated(&state.store).await)
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
            model: model.into(),
            has_api_key: false,
            active: false,
            max_tokens: 0,
            context_window: DEFAULT_CONTEXT_WINDOW,
            reasoning_effort: String::new(),
            supports_vision: false,
            use_for_vision: false,
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
    fn use_for_vision_survives_deserialization() {
        // Repro for the "checkbox lost after save" report: the incoming
        // command payload must keep use_for_vision.
        let p: ModelProfile = serde_json::from_str(
            r#"{"id":"m1","label":"l","provider":"anthropic","api_url":"u","model":"m",
                "max_tokens":8192,"reasoning_effort":"medium",
                "supports_vision":true,"use_for_vision":true}"#,
        )
        .unwrap();
        assert!(p.supports_vision);
        assert!(p.use_for_vision, "use_for_vision dropped on deserialize");
        assert_eq!(p.context_window, DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn context_window_survives_profile_roundtrip() {
        let mut profile = test_profile("m1", "reader", "cheap-reader");
        profile.context_window = 32_768;
        let json = serde_json::to_string(&profile).unwrap();
        let restored: ModelProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.context_window, 32_768);
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
    }

    // The write-through cache must stay coherent: a set is readable without a
    // fresh keyring hit, and a delete reads back as absent (not the old value).
    #[test]
    fn secret_cache_write_through() {
        let name = "model_key:__cache_coherence_test__";
        secret_set(name, "sk-abc").unwrap();
        assert_eq!(secret_get(name), "sk-abc");
        secret_del(name).unwrap();
        assert_eq!(secret_get(name), "");
    }

    // Storing a credential surfaces it in service_env under its env var;
    // clearing removes it; an unknown id is rejected.
    #[test]
    fn credential_registry_roundtrip() {
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
