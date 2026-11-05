//! Shared project visibility for the desktop renderers and native readers.

pub use wisp_dto::project_browser::PrivacyMode;

pub async fn load(store: &wisp_store::Store) -> Result<PrivacyMode, String> {
    store.project_privacy().await.map_err(|e| e.to_string())
}

pub async fn save(
    store: &wisp_store::Store,
    active: bool,
    project_ids: &[String],
) -> Result<(), String> {
    store
        .write_project_privacy(
            PrivacyMode {
                active,
                project_ids: project_ids.to_vec(),
            },
            false,
        )
        .await
        .map_err(|e| e.to_string())
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
    legacy: Option<PrivacyMode>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<PrivacyMode, String> {
    if let Some(legacy) = legacy {
        state
            .store
            .write_project_privacy(legacy, true)
            .await
            .map_err(|e| e.to_string())?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    const ACTIVE_KEY: &str = "wisp-privacy-mode-active";
    const PROJECTS_KEY: &str = "wisp-privacy-mode-projects";

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
        store
            .write_project_privacy(
                PrivacyMode {
                    active: false,
                    project_ids: vec![],
                },
                true,
            )
            .await
            .unwrap();
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
