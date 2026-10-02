//! Native scratch chat. Creates a hidden project and session in a sandbox
//! directory. It does not select a WebView window or restore a previous project.

use std::path::{Path, PathBuf};

use wisp_dto::native_settings::Request;
use wisp_store::{is_scratch_project_id, Store, SCRATCH_PROJECT_PREFIX};

pub(crate) async fn execute(
    store: &Store,
    app_data: &Path,
    request: &Request,
) -> Result<serde_json::Value, String> {
    match request.command.as_str() {
        "native_scratch_open" => open(store, app_data, request).await,
        "native_scratch_close" => close(store, app_data, request).await,
        _ => Err("Unsupported native scratch command".into()),
    }
}

async fn open(
    store: &Store,
    app_data: &Path,
    request: &Request,
) -> Result<serde_json::Value, String> {
    if request
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("Opening a scratch chat does not use a project id".into());
    }
    let _: wisp_dto::native_scratch::OpenRequest =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    let uuid = uuid::Uuid::new_v4().to_string();
    let sandbox = scratch_root(app_data).join(&uuid);
    std::fs::create_dir_all(&sandbox)
        .map_err(|error| format!("Failed to create scratch sandbox: {error}"))?;
    let marker = sandbox.join(".wisp-write-test");
    if let Err(error) = std::fs::write(&marker, b"") {
        let _ = std::fs::remove_dir_all(&sandbox);
        return Err(format!("Scratch sandbox is not writable: {error}"));
    }
    let _ = std::fs::remove_file(&marker);
    let project_id = format!("{SCRATCH_PROJECT_PREFIX}{uuid}");
    let workspace = sandbox.to_string_lossy().into_owned();
    if let Err(error) = store
        .create_project(&project_id, "Scratch", &workspace)
        .await
    {
        let _ = std::fs::remove_dir_all(&sandbox);
        return Err(error.to_string());
    }
    let session_id = match crate::create_session_frame(store, &project_id).await {
        Ok(id) => id,
        Err(error) => {
            let _ = store.delete_project(&project_id).await;
            let _ = std::fs::remove_dir_all(&sandbox);
            return Err(error);
        }
    };
    serde_json::to_value(wisp_dto::native_scratch::ScratchSession {
        project_id,
        session_id,
    })
    .map_err(|error| error.to_string())
}

async fn close(
    store: &Store,
    app_data: &Path,
    request: &Request,
) -> Result<serde_json::Value, String> {
    let Some(project_id) = request
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Err("A scratch project id is required".into());
    };
    if !is_scratch_project_id(project_id) {
        return Err("Only a scratch project can be closed".into());
    }
    let Some((_, workspace)) = store
        .get_project(project_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(serde_json::Value::Bool(false));
    };
    let sandbox = PathBuf::from(&workspace);
    if !sandbox_is_under_scratch_root(app_data, &sandbox) {
        return Err("Scratch sandbox is outside the scratch directory".into());
    }
    store
        .delete_project(project_id)
        .await
        .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_dir_all(&sandbox);
    Ok(serde_json::Value::Bool(true))
}

fn scratch_root(app_data: &Path) -> PathBuf {
    app_data.join("scratch")
}

fn sandbox_is_under_scratch_root(app_data: &Path, sandbox: &Path) -> bool {
    let root = scratch_root(app_data);
    let Ok(root) = std::fs::canonicalize(&root) else {
        return false;
    };
    let Ok(sandbox) = std::fs::canonicalize(sandbox) else {
        return false;
    };
    sandbox.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_dto::native_settings::{Request, SCHEMA};

    fn request(project_id: Option<&str>, command: &str, args: serde_json::Value) -> Request {
        Request {
            schema: SCHEMA.into(),
            id: "scratch-1".into(),
            project_id: project_id.map(str::to_owned),
            command: command.into(),
            args,
        }
    }

    #[tokio::test]
    async fn open_creates_a_hidden_project_and_close_removes_only_that_sandbox() {
        let app_data =
            std::env::temp_dir().join(format!("wisp-native-scratch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&app_data).unwrap();
        let store = Store::open(&app_data.join("wisp.sqlite")).await.unwrap();
        store
            .create_project(
                "research-1",
                "Research",
                &app_data.join("research").to_string_lossy(),
            )
            .await
            .unwrap();
        let opened = execute(
            &store,
            &app_data,
            &request(None, "native_scratch_open", json!({})),
        )
        .await
        .unwrap();
        let opened: wisp_dto::native_scratch::ScratchSession =
            serde_json::from_value(opened).unwrap();
        assert!(opened.project_id.starts_with("scratch:"));
        assert_eq!(
            store
                .frame_project_id(&opened.session_id)
                .await
                .unwrap()
                .as_deref(),
            Some(opened.project_id.as_str())
        );
        let (_, workspace) = store
            .get_project(&opened.project_id)
            .await
            .unwrap()
            .unwrap();
        let sandbox = PathBuf::from(&workspace);
        assert!(sandbox_is_under_scratch_root(&app_data, &sandbox));
        assert!(sandbox.join(".wisp-write-test").exists() == false);
        assert!(store.get_project("research-1").await.unwrap().is_some());
        let closed = execute(
            &store,
            &app_data,
            &request(Some(&opened.project_id), "native_scratch_close", json!({})),
        )
        .await
        .unwrap();
        assert_eq!(closed, true);
        assert!(store
            .get_project(&opened.project_id)
            .await
            .unwrap()
            .is_none());
        assert!(!sandbox.exists());
        assert!(store.get_project("research-1").await.unwrap().is_some());
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[tokio::test]
    async fn close_refuses_a_normal_project_and_a_sandbox_outside_the_scratch_root() {
        let app_data = std::env::temp_dir().join(format!(
            "wisp-native-scratch-guard-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&app_data).unwrap();
        let store = Store::open(&app_data.join("wisp.sqlite")).await.unwrap();
        let kept = app_data.join("kept");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("note.txt"), b"keep").unwrap();
        store
            .create_project("research-1", "Research", &kept.to_string_lossy())
            .await
            .unwrap();
        let rejected = execute(
            &store,
            &app_data,
            &request(Some("research-1"), "native_scratch_close", json!({})),
        )
        .await
        .unwrap_err();
        assert!(rejected.contains("Only a scratch project"));
        assert!(kept.join("note.txt").exists());
        let outside = app_data.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("note.txt"), b"keep").unwrap();
        let disguised = format!("{SCRATCH_PROJECT_PREFIX}disguised");
        store
            .create_project(&disguised, "Scratch", &outside.to_string_lossy())
            .await
            .unwrap();
        let unsafe_close = execute(
            &store,
            &app_data,
            &request(Some(&disguised), "native_scratch_close", json!({})),
        )
        .await
        .unwrap_err();
        assert!(unsafe_close.contains("outside the scratch directory"));
        assert!(outside.join("note.txt").exists());
        assert!(store.get_project(&disguised).await.unwrap().is_some());
        let supplied = execute(
            &store,
            &app_data,
            &request(Some("research-1"), "native_scratch_open", json!({})),
        )
        .await
        .unwrap_err();
        assert!(supplied.contains("does not use a project id"));
        assert!(store.get_project("research-1").await.unwrap().is_some());
        let _ = std::fs::remove_dir_all(&app_data);
    }
}
