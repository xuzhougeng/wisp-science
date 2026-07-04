//! Model profiles: several named LLM configs (provider + API URL + model +
//! its own key), one of them active. The active profile drives every turn —
//! `load_settings` resolves through here — and the composer switches it.
//!
//! Legacy single-model installs are migrated into one "default" profile the
//! first time this is read, so nothing breaks and no key is lost.

use serde::{Deserialize, Serialize};
use tauri::State;

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
    #[serde(default)]
    pub reasoning_effort: String,
}

const PROFILES_KEY: &str = "model_profiles";
const ACTIVE_KEY: &str = "active_model_id";
const LEGACY_KEY_SECRET: &str = "api_key";

fn secret_name(id: &str) -> String {
    format!("model_key:{id}")
}

fn secret_get(name: &str) -> String {
    wisp_store::secrets::Secret::get(name).ok().unwrap_or_default()
}

async fn load_raw(store: &wisp_store::Store) -> Vec<ModelProfile> {
    store
        .get_setting(PROFILES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<ModelProfile>>(&s).ok())
        .unwrap_or_default()
}

async fn save_raw(store: &wisp_store::Store, profiles: &[ModelProfile]) -> Result<(), String> {
    let json = serde_json::to_string(profiles).map_err(|e| e.to_string())?;
    store.set_setting(PROFILES_KEY, &json).await.map_err(|e| e.to_string())
}

/// Ensure at least one profile exists. On the first read of a legacy install,
/// migrate the single `provider`/`api_url`/`model` settings + `api_key` secret
/// into a "default" profile so existing users keep working unchanged.
async fn ensure(store: &wisp_store::Store) -> Vec<ModelProfile> {
    let profiles = load_raw(store).await;
    if !profiles.is_empty() {
        return profiles;
    }
    let provider = store.get_setting("provider").await.ok().flatten().unwrap_or_default();
    let api_url = store.get_setting("api_url").await.ok().flatten().unwrap_or_default();
    let model = store.get_setting("model").await.ok().flatten().unwrap_or_default();
    let max_tokens = store
        .get_setting("max_tokens")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let reasoning_effort = store.get_setting("reasoning_effort").await.ok().flatten().unwrap_or_default();
    let default = ModelProfile {
        id: "default".into(),
        label: if model.trim().is_empty() { "Default".into() } else { model.clone() },
        provider,
        api_url,
        model,
        has_api_key: false,
        active: false,
        max_tokens,
        reasoning_effort,
    };
    let profiles = vec![default];
    let _ = save_raw(store, &profiles).await;
    let _ = store.set_setting(ACTIVE_KEY, "default").await;
    // Carry the legacy key into the default profile's slot so it isn't lost.
    let legacy = secret_get(LEGACY_KEY_SECRET);
    if !legacy.is_empty() {
        let _ = wisp_store::secrets::Secret::set(&secret_name("default"), &legacy);
    }
    profiles
}

async fn active_id(store: &wisp_store::Store, profiles: &[ModelProfile]) -> String {
    let want = store.get_setting(ACTIVE_KEY).await.ok().flatten().unwrap_or_default();
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
        p.label = if alias.is_empty() { model.to_string() } else { alias.to_string() };
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
        wisp_store::secrets::Secret::delete(&name).map_err(|e| e.to_string())
    } else {
        wisp_store::secrets::Secret::set(&name, key.trim()).map_err(|e| e.to_string())
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
            reasoning_effort = store.get_setting("reasoning_effort").await.ok().flatten().unwrap_or_default();
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
    let reasoning_effort = store.get_setting("reasoning_effort").await.ok().flatten().unwrap_or_default();
    (max_tokens, reasoning_effort)
}

/// Whether the active profile has a key stored (for `get_settings`).
pub async fn active_has_key(store: &wisp_store::Store) -> bool {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    !key_for(&id).is_empty()
}

/// Profiles with `has_api_key`/`active` filled in, for the UI.
async fn decorated(store: &wisp_store::Store) -> Vec<ModelProfile> {
    let profiles = ensure(store).await;
    let id = active_id(store, &profiles).await;
    profiles
        .into_iter()
        .map(|mut p| {
            p.has_api_key = !key_for(&p.id).is_empty();
            p.active = p.id == id;
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
) -> Result<Vec<ModelProfile>, String> {
    if profile.model.trim().is_empty() {
        return Err("Model is required.".into());
    }
    if profile.api_url.trim().is_empty() {
        return Err("API URL is required.".into());
    }
    let mut profiles = ensure(&state.store).await;
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
    if let Some(k) = key {
        let k = k.trim();
        if !k.is_empty() {
            wisp_store::secrets::Secret::set(&secret_name(&id), k).map_err(|e| e.to_string())?;
        }
    }
    // Land the user on a freshly added model so they can edit/use it right away.
    if is_new {
        let _ = state.store.set_setting(ACTIVE_KEY, &id).await;
    }
    let active = active_id(&state.store, &profiles).await;
    if id == active {
        crate::clear_idle_agents(&state).await;
    }
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn remove_model(state: State<'_, crate::AppState>, id: String) -> Result<Vec<ModelProfile>, String> {
    let mut profiles = ensure(&state.store).await;
    if profiles.len() <= 1 {
        return Err("At least one model is required.".into());
    }
    profiles.retain(|p| p.id != id);
    save_raw(&state.store, &profiles).await?;
    let _ = wisp_store::secrets::Secret::delete(&secret_name(&id));
    // If we removed the active profile, fall back to the first remaining one.
    let cur = state.store.get_setting(ACTIVE_KEY).await.ok().flatten().unwrap_or_default();
    if cur == id {
        if let Some(first) = profiles.first() {
            let _ = state.store.set_setting(ACTIVE_KEY, &first.id).await;
        }
    }
    Ok(decorated(&state.store).await)
}

#[tauri::command]
pub async fn set_active_model(state: State<'_, crate::AppState>, id: String) -> Result<Vec<ModelProfile>, String> {
    let profiles = ensure(&state.store).await;
    if !profiles.iter().any(|p| p.id == id) {
        return Err("Unknown model.".into());
    }
    state.store.set_setting(ACTIVE_KEY, &id).await.map_err(|e| e.to_string())?;
    crate::clear_idle_agents(&state).await;
    Ok(decorated(&state.store).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_id_skips_taken() {
        let existing = vec![
            ModelProfile { id: "m1".into(), label: "a".into(), provider: "openai".into(), api_url: "u".into(), model: "x".into(), has_api_key: false, active: false, max_tokens: 0, reasoning_effort: String::new() },
            ModelProfile { id: "m2".into(), label: "b".into(), provider: "openai".into(), api_url: "u".into(), model: "y".into(), has_api_key: false, active: false, max_tokens: 0, reasoning_effort: String::new() },
        ];
        assert_eq!(fresh_id(&existing), "m3");
        assert_eq!(fresh_id(&[]), "m1");
    }
}
