//! Specialists (专家): user-definable agent personas — instructions plus a
//! skill/MCP subset and a directly-bound model, selectable per session.
//! Stored as a JSON array under the `specialists` settings key (same pattern
//! as `model_profiles`). The builtin Reviewer is materialized into the list on
//! first read so user edits to its model binding persist like any other row.

use serde::{Deserialize, Serialize};
use tauri::State;
use wisp_store::Store;

pub const SPECIALISTS_KEY: &str = "specialists";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Specialist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    /// "" = follow the active model; dangling ids fall back to active too.
    #[serde(default)]
    pub model_id: String,
    /// None = inherit the project skill config; Some = whitelist of skill names.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// None = inherit; Some = whitelist of connector slugs / MCP connection ids.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
    #[serde(default)]
    pub builtin: bool,
}

pub fn builtin_reviewer() -> Specialist {
    Specialist {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        icon: "review".into(),
        color: "clay".into(),
        description: "Traces a session transcript and reports fabrication, hallucination, or plan deviation.".into(),
        instructions: crate::review::REVIEWER_RUBRIC.into(),
        model_id: String::new(),
        skills: Some(vec![]), // reviewer runs one-shot; skills are irrelevant
        connectors: Some(vec![]),
        builtin: true,
    }
}

async fn load_raw(store: &Store) -> Vec<Specialist> {
    store
        .get_setting(SPECIALISTS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<Specialist>>(&s).ok())
        .unwrap_or_default()
}

async fn save_raw(store: &Store, list: &[Specialist]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    store
        .set_setting(SPECIALISTS_KEY, &json)
        .await
        .map_err(|e| e.to_string())
}

/// Load the list, materializing the builtin Reviewer if absent. The builtin's
/// `instructions` are always re-pinned to the compiled rubric so rubric
/// improvements ship without a migration.
pub async fn ensure(store: &Store) -> Vec<Specialist> {
    let mut list = load_raw(store).await;
    match list.iter_mut().find(|s| s.id == "reviewer") {
        Some(r) => {
            r.builtin = true;
            r.instructions = crate::review::REVIEWER_RUBRIC.into();
        }
        None => list.insert(0, builtin_reviewer()),
    }
    list
}

pub async fn get(store: &Store, id: &str) -> Option<Specialist> {
    ensure(store).await.into_iter().find(|s| s.id == id)
}

fn fresh_id(existing: &[Specialist]) -> String {
    for n in 1..10_000 {
        let id = format!("sp{n}");
        if !existing.iter().any(|s| s.id == id) {
            return id;
        }
    }
    "sp".into()
}

/// Create (empty id) or update (existing id). Builtin rows keep their
/// compiled instructions and can never lose `builtin`.
pub async fn upsert(store: &Store, mut spec: Specialist) -> Result<Vec<Specialist>, String> {
    if spec.name.trim().is_empty() {
        return Err("Specialist name is required.".into());
    }
    let mut list = ensure(store).await;
    if spec.id.trim().is_empty() {
        spec.id = fresh_id(&list);
    }
    if let Some(existing) = list.iter_mut().find(|s| s.id == spec.id) {
        if existing.builtin {
            spec.builtin = true;
            spec.instructions = existing.instructions.clone();
        }
        *existing = spec;
    } else {
        spec.builtin = false;
        list.push(spec);
    }
    save_raw(store, &list).await?;
    Ok(ensure(store).await)
}

pub async fn remove(store: &Store, id: &str) -> Result<Vec<Specialist>, String> {
    let mut list = ensure(store).await;
    if list.iter().any(|s| s.id == id && s.builtin) {
        return Err("Built-in specialists cannot be removed.".into());
    }
    list.retain(|s| s.id != id);
    save_raw(store, &list).await?;
    Ok(ensure(store).await)
}

#[tauri::command]
pub async fn list_specialists(state: State<'_, crate::AppState>) -> Result<Vec<Specialist>, String> {
    Ok(ensure(&state.store).await)
}

#[tauri::command]
pub async fn save_specialist_cmd(
    state: State<'_, crate::AppState>,
    spec: Specialist,
) -> Result<Vec<Specialist>, String> {
    upsert(&state.store, spec).await
}

#[tauri::command]
pub async fn remove_specialist(
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<Vec<Specialist>, String> {
    remove(&state.store, &id).await
}

/// LLM config for a specialist: its bound profile, or the active-model chain
/// when unbound/dangling (soft fallback — personas are not hard capabilities).
pub async fn specialist_llm(
    store: &Store,
    spec: &Specialist,
) -> (String, String, String, String, u64, String) {
    if !spec.model_id.trim().is_empty() {
        if let Some(cfg) = crate::models::profile_llm(store, &spec.model_id).await {
            return cfg;
        }
    }
    let (provider, api_url, model, api_key) = crate::load_settings(store).await;
    let (max_tokens, reasoning_effort) = crate::models::active_llm_advanced(store).await;
    (provider, api_url, model, api_key, max_tokens, reasoning_effort)
}

fn frame_key(frame_id: &str) -> String {
    format!("frame_specialist:{frame_id}")
}

pub async fn set_frame_specialist(store: &Store, frame_id: &str, id: &str) -> Result<(), String> {
    store
        .set_setting(&frame_key(frame_id), id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn session_specialist(store: &Store, frame_id: &str) -> Option<Specialist> {
    let id = store.get_setting(&frame_key(frame_id)).await.ok().flatten()?;
    if id.trim().is_empty() {
        return None;
    }
    get(store, &id).await
}

/// The UI disables the picker once a session has messages; this backend guard
/// enforces the same rule for any other caller.
#[tauri::command]
pub async fn set_session_specialist(
    state: State<'_, crate::AppState>,
    frame_id: String,
    id: String,
) -> Result<(), String> {
    let msgs = state
        .store
        .load_messages(&frame_id)
        .await
        .map_err(|e| format!("{e}"))?;
    if msgs.iter().any(|m| m.role != wisp_llm::Role::System) {
        return Err("Specialist is locked once the session has messages.".into());
    }
    if !id.is_empty() && get(&state.store, &id).await.is_none() {
        return Err(format!("Unknown specialist '{id}'."));
    }
    set_frame_specialist(&state.store, &frame_id, &id).await
}

#[tauri::command]
pub async fn get_session_specialist(
    state: State<'_, crate::AppState>,
    frame_id: String,
) -> Result<Option<Specialist>, String> {
    Ok(session_specialist(&state.store, &frame_id).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> (wisp_store::Store, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!("wisp_spec_{}.sqlite", uuid::Uuid::new_v4()));
        (wisp_store::Store::open(&tmp).await.unwrap(), tmp)
    }

    #[tokio::test]
    async fn ensure_materializes_builtin_reviewer_once() {
        let (store, tmp) = test_store().await;
        let list = ensure(&store).await;
        assert_eq!(list.len(), 1);
        let r = &list[0];
        assert_eq!(r.id, "reviewer");
        assert!(r.builtin);
        assert_eq!(r.instructions, crate::review::REVIEWER_RUBRIC);
        // Second read does not duplicate it.
        assert_eq!(ensure(&store).await.len(), 1);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn upsert_roundtrip_and_fresh_id() {
        let (store, tmp) = test_store().await;
        let spec = Specialist {
            id: String::new(),
            name: "Paper hunter".into(),
            icon: "search".into(),
            color: "clay".into(),
            description: "finds papers".into(),
            instructions: "You hunt papers.".into(),
            model_id: "m1".into(),
            skills: Some(vec!["bear-support".into()]),
            connectors: None,
            builtin: false,
        };
        let list = upsert(&store, spec).await.unwrap();
        let created = list.iter().find(|s| !s.builtin).unwrap();
        assert_eq!(created.id, "sp1");
        assert_eq!(created.skills.as_deref(), Some(&["bear-support".to_string()][..]));
        // Edit by id keeps the id.
        let mut edited = created.clone();
        edited.name = "Paper hunter 2".into();
        let list = upsert(&store, edited).await.unwrap();
        assert_eq!(list.iter().filter(|s| !s.builtin).count(), 1);
        assert_eq!(list.iter().find(|s| s.id == "sp1").unwrap().name, "Paper hunter 2");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn builtin_reviewer_guards() {
        let (store, tmp) = test_store().await;
        ensure(&store).await;
        assert!(remove(&store, "reviewer").await.is_err());
        // Editing the builtin keeps instructions but accepts a model change.
        let mut r = get(&store, "reviewer").await.unwrap();
        r.instructions = "haha".into();
        r.model_id = "m2".into();
        let list = upsert(&store, r).await.unwrap();
        let r = list.iter().find(|s| s.id == "reviewer").unwrap();
        assert_eq!(r.instructions, crate::review::REVIEWER_RUBRIC);
        assert_eq!(r.model_id, "m2");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn specialist_llm_falls_back_to_active_for_empty_or_dangling() {
        let (store, tmp) = test_store().await;
        // No model profiles configured: active resolution still returns the
        // env/default fallback chain from load_settings.
        let spec = Specialist { model_id: "no-such".into(), ..builtin_reviewer() };
        let (provider, api_url, model, _key, _mt, _re) = specialist_llm(&store, &spec).await;
        assert!(!provider.is_empty());
        assert!(!api_url.is_empty());
        assert!(!model.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn session_specialist_set_get_and_lock() {
        let (store, tmp) = test_store().await;
        ensure(&store).await;
        store.create_project("p1", "proj", "").await.unwrap();
        store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
        set_frame_specialist(&store, "f1", "reviewer").await.unwrap();
        assert_eq!(session_specialist(&store, "f1").await.unwrap().id, "reviewer");
        // Clearing works.
        set_frame_specialist(&store, "f1", "").await.unwrap();
        assert!(session_specialist(&store, "f1").await.is_none());
        let _ = std::fs::remove_file(&tmp);
    }
}
