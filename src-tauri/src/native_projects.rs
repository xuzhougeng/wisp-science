//! Native project creation. The desktop host owns the write; the read-only
//! project browser service is not involved. No WebView window is selected.

use wisp_dto::native_settings::Request;

pub(crate) async fn execute(
    store: &wisp_store::Store,
    app_data: &std::path::Path,
    request: &Request,
) -> Result<String, String> {
    if request
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("This project command does not use a project id".into());
    }
    match request.command.as_str() {
        "native_project_create" => {
            let input: wisp_dto::native_projects::CreateProjectRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            crate::project_commands::create_project_record(store, input).await
        }
        "native_project_import" => {
            let input: wisp_dto::native_projects::ImportProjectRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let archive = std::path::PathBuf::from(input.archive_path.trim());
            if archive.as_os_str().is_empty() {
                return Err("A project archive is required".into());
            }
            let parent = archive
                .parent()
                .filter(|parent| parent.is_dir())
                .ok_or_else(|| "The archive's folder is not an import destination".to_string())?;
            crate::project_transfer::import_archived_project(store, app_data, &archive, parent)
                .await
        }
        _ => Err("Unsupported native project command".into()),
    }
}

pub(crate) async fn execute_folders(
    store: &wisp_store::Store,
    request: &Request,
) -> Result<serde_json::Value, String> {
    let project_id = request
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or("A project is required")?;
    if store
        .get_project(project_id)
        .await
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("Project not found".into());
    }
    match request.command.as_str() {
        "native_project_folders" => {
            if request
                .args
                .as_object()
                .is_some_and(|args| !args.is_empty())
            {
                return Err("Listing folders takes no arguments".into());
            }
            let folders = store
                .list_folders(project_id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|(id, name, _)| wisp_dto::native_projects::ProjectFolder { id, name })
                .collect::<Vec<_>>();
            serde_json::to_value(folders).map_err(|error| error.to_string())
        }
        "native_project_folder_create" => {
            let input: wisp_dto::native_projects::FolderCreateRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let id = uuid::Uuid::new_v4().to_string();
            store
                .create_folder(&id, project_id, &input.name)
                .await
                .map_err(|error| error.to_string())?;
            let name = input.name.trim().to_owned();
            serde_json::to_value(wisp_dto::native_projects::ProjectFolder { id, name })
                .map_err(|error| error.to_string())
        }
        "native_project_folder_rename" => {
            let input: wisp_dto::native_projects::FolderRenameRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            store
                .rename_folder(&input.folder_id, project_id, &input.name)
                .await
                .map_err(|error| error.to_string())?;
            Ok(serde_json::Value::Bool(true))
        }
        "native_project_session_move" => {
            let input: wisp_dto::native_projects::SessionMoveRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let folder = input
                .folder_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty());
            store
                .move_session_to_folder(&input.session_id, project_id, folder)
                .await
                .map_err(|error| error.to_string())?;
            Ok(serde_json::Value::Bool(true))
        }
        _ => Err("Unsupported native project command".into()),
    }
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
            &fixture.root,
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
            assert!(execute(&fixture.store, &fixture.root, &request(None, args))
                .await
                .is_err());
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
            &fixture.root,
            &request(None, input(&workspace.to_string_lossy(), false)),
        )
        .await
        .unwrap();
        let duplicate = execute(
            &fixture.store,
            &fixture.root,
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
            &fixture.root,
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
            &fixture.root,
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
            &fixture.root,
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
            &fixture.root,
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

    fn import_request(project_id: Option<&str>, archive: &std::path::Path) -> Request {
        let mut value = request(
            project_id,
            json!({ "archive_path": archive.to_string_lossy() }),
        );
        value.command = "native_project_import".into();
        value
    }

    #[tokio::test]
    async fn import_rejects_a_project_id_or_invalid_archive_without_writing() {
        let fixture = Fixture::open().await;
        let archive = fixture.workspace("missing.zip");
        let error = execute(
            &fixture.store,
            &fixture.root,
            &import_request(Some("webview-project"), &archive),
        )
        .await
        .unwrap_err();
        assert!(error.contains("does not use a project id"));
        assert!(!archive.exists());

        let junk = fixture.workspace("notes.zip");
        std::fs::write(&junk, b"not a zip").unwrap();
        let error = execute(&fixture.store, &fixture.root, &import_request(None, &junk))
            .await
            .unwrap_err();
        assert!(
            error.contains("not a valid project archive") || error.contains("cannot open"),
            "{error}"
        );
        assert!(fixture.store.list_projects().await.unwrap().is_empty());
        let again = execute(&fixture.store, &fixture.root, &import_request(None, &junk))
            .await
            .unwrap_err();
        assert_eq!(error, again);
    }

    #[tokio::test]
    async fn import_round_trip_opens_the_archived_project_once() {
        let source = Fixture::open().await;
        let workspace = source.workspace("study");
        let created = crate::project_commands::create_project_record(
            &source.store,
            wisp_dto::native_projects::CreateProjectRequest {
                name: "Imported study".into(),
                workspace_dir: workspace.to_string_lossy().into_owned(),
                description: "from archive".into(),
                agent_context: String::new(),
                standard_layout: false,
            },
        )
        .await
        .unwrap();
        std::fs::write(workspace.join("notes.txt"), b"hello").unwrap();
        let archive = source.root.join("study.zip");
        crate::project_transfer::export_project_archive(
            &source.store,
            &source.root,
            &created,
            &archive,
        )
        .await
        .unwrap();

        let destination = Fixture::open().await;
        let imported = execute(
            &destination.store,
            &destination.root,
            &import_request(None, &archive),
        )
        .await
        .unwrap();
        assert_eq!(imported, created);
        let meta = destination
            .store
            .get_project_meta(&imported)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.0, "Imported study");
        assert_eq!(meta.1, "from archive");
        assert_eq!(
            std::fs::read(std::path::Path::new(&meta.2).join("notes.txt")).unwrap(),
            b"hello"
        );
        assert!(destination
            .store
            .list_sessions(&imported)
            .await
            .unwrap()
            .is_empty());
        let duplicate = execute(
            &destination.store,
            &destination.root,
            &import_request(None, &archive),
        )
        .await
        .unwrap_err();
        assert!(duplicate.contains("already present"), "{duplicate}");
        assert_eq!(
            std::fs::read(std::path::Path::new(&meta.2).join("notes.txt")).unwrap(),
            b"hello"
        );
    }

    fn folder_request(project_id: Option<&str>, command: &str, args: serde_json::Value) -> Request {
        let mut value = request(project_id, args);
        value.command = command.into();
        value
    }

    #[tokio::test]
    async fn folders_require_an_explicit_project_and_do_not_cross_projects() {
        let fixture = Fixture::open().await;
        let workspace = fixture.workspace("grouped");
        let project = crate::project_commands::create_project_record(
            &fixture.store,
            wisp_dto::native_projects::CreateProjectRequest {
                name: "Grouped".into(),
                workspace_dir: workspace.to_string_lossy().into_owned(),
                description: String::new(),
                agent_context: String::new(),
                standard_layout: false,
            },
        )
        .await
        .unwrap();
        let missing = execute_folders(
            &fixture.store,
            &folder_request(
                None,
                "native_project_folder_create",
                json!({"name": "Week"}),
            ),
        )
        .await
        .unwrap_err();
        assert!(missing.contains("project is required"), "{missing}");
        assert!(fixture
            .store
            .list_folders(&project)
            .await
            .unwrap()
            .is_empty());

        let empty = execute_folders(
            &fixture.store,
            &folder_request(
                Some(&project),
                "native_project_folder_create",
                json!({"name": "  "}),
            ),
        )
        .await
        .unwrap_err();
        assert!(empty.contains("cannot be empty"), "{empty}");
        assert!(fixture
            .store
            .list_folders(&project)
            .await
            .unwrap()
            .is_empty());

        let created = execute_folders(
            &fixture.store,
            &folder_request(
                Some(&project),
                "native_project_folder_create",
                json!({"name": "Week"}),
            ),
        )
        .await
        .unwrap();
        let folder_id = created["id"].as_str().unwrap().to_owned();
        assert_eq!(created["name"], "Week");
        execute_folders(
            &fixture.store,
            &folder_request(
                Some(&project),
                "native_project_folder_rename",
                json!({"folder_id": folder_id, "name": "Week 1"}),
            ),
        )
        .await
        .unwrap();
        fixture
            .store
            .create_frame("session-a", &project, "OPERON", "model")
            .await
            .unwrap();
        fixture
            .store
            .rename_session("session-a", &project, "Named draft")
            .await
            .unwrap();
        execute_folders(
            &fixture.store,
            &folder_request(
                Some(&project),
                "native_project_session_move",
                json!({"session_id": "session-a", "folder_id": folder_id}),
            ),
        )
        .await
        .unwrap();
        let sessions = fixture.store.list_sessions(&project).await.unwrap();
        assert_eq!(sessions[0].3.as_deref(), Some(folder_id.as_str()));
        let other = crate::project_commands::create_project_record(
            &fixture.store,
            wisp_dto::native_projects::CreateProjectRequest {
                name: "Other".into(),
                workspace_dir: fixture.workspace("other").to_string_lossy().into_owned(),
                description: String::new(),
                agent_context: String::new(),
                standard_layout: false,
            },
        )
        .await
        .unwrap();
        let crossed = execute_folders(
            &fixture.store,
            &folder_request(
                Some(&other),
                "native_project_session_move",
                json!({"session_id": "session-a", "folder_id": folder_id}),
            ),
        )
        .await
        .unwrap_err();
        assert!(
            crossed.contains("not found") || crossed.contains("Folder"),
            "{crossed}"
        );
        assert_eq!(
            fixture.store.list_sessions(&project).await.unwrap()[0]
                .3
                .as_deref(),
            Some(folder_id.as_str())
        );
        let listed = execute_folders(
            &fixture.store,
            &folder_request(Some(&project), "native_project_folders", json!({})),
        )
        .await
        .unwrap();
        assert_eq!(listed[0]["name"], "Week 1");
        assert_eq!(listed.as_array().unwrap().len(), 1);
    }
}
