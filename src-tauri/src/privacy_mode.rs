//! Privacy mode shared by the WebView and the native calendar.
//!
//! The WebView writes `set_privacy_mode`. The calendar reads `get_privacy_mode`
//! before it builds a project list. Both use these settings rows.

use serde::{Deserialize, Serialize};

pub const ACTIVE_KEY: &str = "wisp-privacy-mode-active";
pub const PROJECTS_KEY: &str = "wisp-privacy-mode-projects";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyMode {
    pub active: bool,
    pub project_ids: Vec<String>,
}

pub async fn load(store: &wisp_store::Store) -> Result<PrivacyMode, String> {
    let raw = store
        .get_setting(PROJECTS_KEY)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "[]".into());
    let mut project_ids = serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default();
    project_ids = normalize_ids(project_ids);
    let active = store
        .get_setting(ACTIVE_KEY)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        == Some("1")
        && !project_ids.is_empty();
    Ok(PrivacyMode {
        active,
        project_ids,
    })
}

pub async fn save(
    store: &wisp_store::Store,
    active: bool,
    project_ids: &[String],
) -> Result<(), String> {
    let project_ids = normalize_ids(project_ids.iter().cloned());
    let encoded = serde_json::to_string(&project_ids).map_err(|error| error.to_string())?;
    store
        .set_setting(PROJECTS_KEY, &encoded)
        .await
        .map_err(|error| error.to_string())?;
    if active && !project_ids.is_empty() {
        store
            .set_setting(ACTIVE_KEY, "1")
            .await
            .map_err(|error| error.to_string())?;
    } else {
        store
            .delete_setting(ACTIVE_KEY)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Native calendar read. A project id is rejected and nothing is changed.
pub async fn read(
    store: &wisp_store::Store,
    project_id: Option<&str>,
    args: &serde_json::Value,
) -> Result<PrivacyMode, String> {
    if project_id.is_some_and(|id| !id.trim().is_empty()) {
        return Err("Reading privacy mode does not use a project id".into());
    }
    if !args.as_object().is_some_and(|object| object.is_empty()) {
        return Err("Privacy mode takes no arguments".into());
    }
    load(store).await
}

#[tauri::command]
pub async fn get_privacy_mode(
    state: tauri::State<'_, crate::AppState>,
) -> Result<PrivacyMode, String> {
    load(&state.store).await
}

#[tauri::command]
pub async fn set_privacy_mode(
    state: tauri::State<'_, crate::AppState>,
    active: bool,
    project_ids: Vec<String>,
) -> Result<(), String> {
    save(&state.store, active, &project_ids).await
}

fn normalize_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut project_ids = ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    project_ids.sort();
    project_ids.dedup();
    project_ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn webview_privacy_write_is_what_the_calendar_reads() {
        let directory = std::env::temp_dir().join(format!("wisp_privacy_{}", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&directory.join("test.sqlite"))
            .await
            .unwrap();
        let empty = load(&store).await.unwrap();
        assert!(!empty.active);
        assert!(empty.project_ids.is_empty());
        save(
            &store,
            true,
            &[
                "hidden".into(),
                " hidden ".into(),
                "research-1".into(),
                "".into(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            store.get_setting(PROJECTS_KEY).await.unwrap().as_deref(),
            Some("[\"hidden\",\"research-1\"]")
        );
        assert_eq!(
            store.get_setting(ACTIVE_KEY).await.unwrap().as_deref(),
            Some("1")
        );
        let loaded = read(&store, None, &serde_json::json!({})).await.unwrap();
        assert!(loaded.active);
        assert_eq!(loaded.project_ids, vec!["hidden", "research-1"]);
        let rejected = read(&store, Some("research-1"), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(rejected.contains("does not use a project id"));
        assert_eq!(
            store.get_setting(ACTIVE_KEY).await.unwrap().as_deref(),
            Some("1")
        );
        save(&store, false, &["hidden".into()]).await.unwrap();
        let off = load(&store).await.unwrap();
        assert!(!off.active);
        assert_eq!(off.project_ids, vec!["hidden".to_string()]);
        assert!(store.get_setting(ACTIVE_KEY).await.unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_dir_all(directory);
    }
}
