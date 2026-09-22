//! Native project creation. The desktop host owns the write; the read-only
//! project browser service is not involved. No WebView window is selected.

use wisp_dto::native_settings::Request;

pub(crate) async fn execute(
    store: &wisp_store::Store,
    request: &Request,
) -> Result<String, String> {
    if request
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("Creating a project does not use a project id".into());
    }
    if request.command != "native_project_create" {
        return Err("Unsupported native project command".into());
    }
    let input: wisp_dto::native_projects::CreateProjectRequest =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    crate::project_commands::create_project_record(store, input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_dto::native_settings::{Request, SCHEMA};

    fn request(project_id: Option<&str>, args: serde_json::Value) -> Request {
        Request {
            schema: SCHEMA.into(),
            id: "create-1".into(),
            project_id: project_id.map(str::to_owned),
            command: "native_project_create".into(),
            args,
        }
    }

    fn input(dir: &str, layout: bool) -> serde_json::Value {
        json!({
            "name": "RNA study",
            "workspace_dir": dir,
            "description": "counts",
            "agent_context": "",
            "standard_layout": layout,
        })
    }

    struct Fixture {
        root: std::path::PathBuf,
        store: wisp_store::Store,
    }

    impl Fixture {
        async fn open() -> Self {
            let root =
                std::env::temp_dir().join(format!("wisp-native-project-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let store = wisp_store::Store::open(&root.join("wisp.sqlite"))
                .await
                .unwrap();
            Self { root, store }
        }

        fn workspace(&self, name: &str) -> std::path::PathBuf {
            self.root.join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn create_rejects_a_project_id_and_writes_nothing() {
        let fixture = Fixture::open().await;
        let workspace = fixture.workspace("unused");
        let error = execute(
            &fixture.store,
            &request(
                Some("webview-project"),
                input(&workspace.to_string_lossy(), false),
            ),
        )
        .await
        .unwrap_err();
        assert!(error.contains("does not use a project id"));
        assert!(!workspace.exists());
        assert!(fixture.store.list_projects().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_name_or_directory_keeps_the_store_empty() {
        let fixture = Fixture::open().await;
        let workspace = fixture.workspace("named");
        for args in [
            json!({"name":"  ","workspace_dir":workspace,"description":"","agent_context":"","standard_layout":false}),
            json!({"name":"Study","workspace_dir":"  ","description":"","agent_context":"","standard_layout":false}),
            json!({"name":"Study","workspace_dir":workspace,"description":"","agent_context":"","standard_layout":false,"window":"main"}),
        ] {
            assert!(execute(&fixture.store, &request(None, args)).await.is_err());
        }
        assert!(!workspace.exists());
        assert!(fixture.store.list_projects().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_directory_and_unwritable_path_keep_the_draft_side_unchanged() {
        let fixture = Fixture::open().await;
        let workspace = fixture.workspace("study");
        let id = execute(
            &fixture.store,
            &request(None, input(&workspace.to_string_lossy(), false)),
        )
        .await
        .unwrap();
        let duplicate = execute(
            &fixture.store,
            &request(None, input(&workspace.to_string_lossy(), true)),
        )
        .await
        .unwrap_err();
        assert!(duplicate.contains("already registered"));
        assert!(!workspace.join("data/raw").exists());
        assert!(fixture.store.list_sessions(&id).await.unwrap().is_empty());
        assert_eq!(fixture.store.list_projects().await.unwrap().len(), 1);

        let blocked = fixture.workspace("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let error = execute(
            &fixture.store,
            &request(None, input(&blocked.to_string_lossy(), false)),
        )
        .await
        .unwrap_err();
        assert!(error.contains("Failed to create working directory"));
        assert_eq!(fixture.store.list_projects().await.unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unwritable_directory_is_rejected_without_a_project_row() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::open().await;
        let workspace = fixture.workspace("locked");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut permissions = std::fs::metadata(&workspace).unwrap().permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&workspace, permissions.clone()).unwrap();
        let probe = workspace.join(".wisp-write-test");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            permissions.set_mode(0o755);
            std::fs::set_permissions(&workspace, permissions).unwrap();
            panic!("directory stayed writable; the unwritable rejection was not exercised");
        }
        let result = execute(
            &fixture.store,
            &request(None, input(&workspace.to_string_lossy(), true)),
        )
        .await;
        permissions.set_mode(0o755);
        std::fs::set_permissions(&workspace, permissions).unwrap();
        let error = result.unwrap_err();
        assert!(error.contains("not writable"), "{error}");
        assert!(fixture.store.list_projects().await.unwrap().is_empty());
        assert!(!workspace.join("figures").exists());
    }

    #[tokio::test]
    async fn standard_layout_is_opt_in_and_no_session_is_created() {
        let fixture = Fixture::open().await;
        let plain = fixture.workspace("plain");
        let plain_id = execute(
            &fixture.store,
            &request(
                None,
                json!({
                    "name": "Plain",
                    "workspace_dir": plain,
                    "description": "notes",
                    "agent_context": "Own structure.",
                    "standard_layout": false,
                }),
            ),
        )
        .await
        .unwrap();
        assert!(plain.is_dir());
        assert!(!plain.join("data/raw").exists());
        assert!(!plain.join("figures").exists());
        assert_eq!(
            std::fs::read_to_string(plain.join(".wisp/WISP.md")).unwrap(),
            "Own structure."
        );
        let meta = fixture
            .store
            .get_project_meta(&plain_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.0, "Plain");
        assert_eq!(meta.1, "notes");
        assert!(fixture
            .store
            .list_sessions(&plain_id)
            .await
            .unwrap()
            .is_empty());

        let laid_out = fixture.workspace("layout");
        let laid_out_id = execute(
            &fixture.store,
            &request(None, input(&laid_out.to_string_lossy(), true)),
        )
        .await
        .unwrap();
        assert!(laid_out.join("data/raw").is_dir());
        assert!(laid_out.join("figures").is_dir());
        assert!(laid_out.join(".wisp/project.toml").is_file());
        assert!(!laid_out.join(".wisp/WISP.md").exists());
        assert!(fixture
            .store
            .list_sessions(&laid_out_id)
            .await
            .unwrap()
            .is_empty());
        assert_ne!(plain_id, laid_out_id);
    }
}
