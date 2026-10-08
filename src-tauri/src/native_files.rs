//! File locations, listings, previews and path copies resolve an explicit frame.
use crate::native_settings::Broker;
use serde_json::Value;
use std::path::Path;
use tauri::Manager;
use wisp_dto::{native_files as dto, native_settings::Request};

pub(crate) async fn dispatch(
    broker: &Broker,
    request: &Request,
    project_id: &str,
    session: &str,
) -> Result<Value, String> {
    if matches!(
        request.command.as_str(),
        "native_conversation_panel_file_upload" | "native_conversation_panel_file_download"
    ) {
        return dispatch_transfer(broker, request, project_id, session).await;
    }
    let args: dto::Request =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    if args.session_id != session {
        return Err("Session scope mismatch".into());
    }
    let state = broker.app.state::<crate::AppState>();
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, session).await?;
    if project.id != project_id {
        return Err("Project scope mismatch".into());
    }
    let context = args.context_id.as_deref().unwrap_or("local");
    match request.command.as_str() {
        "native_conversation_panel_file_locations" => {
            if context != "local" || args.path.is_some() || !args.paths.is_empty() {
                return Err("Locations take no path or remote context".into());
            }
            let contexts = state
                .store
                .list_execution_contexts()
                .await
                .map_err(|e| e.to_string())?;
            let read_only =
                crate::exploration_commands::require_writable_scope(&state.store, &scope)
                    .await
                    .is_err()
                    || state
                        .store
                        .require_unarchived_session(session)
                        .await
                        .is_err();
            let root = wisp_tools::safety::resolve_under_root(&project.root, ".")?;
            value(dto::Locations {
                schema: dto::SCHEMA.into(),
                project_id: project_id.into(),
                session_id: session.into(),
                local_root: root.to_string_lossy().into_owned(),
                read_only,
                locations: locations(&contexts),
            })
        }
        "native_conversation_panel_file_directory" => {
            if !args.paths.is_empty() {
                return Err("Directory takes one path".into());
            }
            let listing = if context == "local" {
                let root = project.root;
                let path = args.path.unwrap_or_else(|| ".".into());
                tokio::task::spawn_blocking(move || local_directory(&root, &path))
                    .await
                    .map_err(|e| e.to_string())??
            } else {
                require_ssh(&state.store, context).await?;
                crate::file_browser::list_remote_dir(state.clone(), context.into(), args.path)
                    .await?
            };
            value(dto::Directory {
                schema: dto::SCHEMA.into(),
                project_id: project_id.into(),
                session_id: session.into(),
                context_id: context.into(),
                path: listing.path,
                entries: listing.entries,
            })
        }
        "native_conversation_panel_file_paths" => {
            if context != "local" || args.path.is_some() {
                return Err("Path copies require the local project location".into());
            }
            let root = project.root;
            let paths = tokio::task::spawn_blocking(move || local_paths(&root, &args.paths))
                .await
                .map_err(|e| e.to_string())??;
            value(dto::Paths {
                schema: dto::SCHEMA.into(),
                project_id: project_id.into(),
                session_id: session.into(),
                paths,
            })
        }
        "native_conversation_panel_file_read" => {
            if !args.paths.is_empty() {
                return Err("Preview takes one path".into());
            }
            let path = args.path.ok_or("File path is required")?;
            let content = if context == "local" {
                let root = project.root;
                let requested = path.clone();
                tokio::task::spawn_blocking(move || {
                    crate::file_browser::read_native_preview_at(
                        &root,
                        requested,
                        args.render_pdf,
                        args.render_office,
                    )
                })
                .await
                .map_err(|e| e.to_string())??
            } else {
                require_ssh(&state.store, context).await?;
                let reference = remote_reference(context, &path)?;
                let mut content = crate::file_browser::read_native_remote_preview(
                    state.clone(),
                    context.into(),
                    path.clone(),
                    args.render_pdf,
                    args.render_office,
                )
                .await?;
                content.path = reference;
                content
            };
            value(dto::Preview {
                schema: dto::SCHEMA.into(),
                project_id: project_id.into(),
                session_id: session.into(),
                context_id: context.into(),
                requested_path: path,
                content,
            })
        }
        _ => Err("Unknown native Files command".into()),
    }
}

async fn dispatch_transfer(
    broker: &Broker,
    request: &Request,
    project_id: &str,
    session: &str,
) -> Result<Value, String> {
    let args: dto::TransferRequest =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    let upload = request.command.ends_with("file_upload");
    validate_transfer(&args, session, upload)?;
    let state = broker.app.state::<crate::AppState>();
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, session).await?;
    if project.id != project_id {
        return Err("Project scope mismatch".into());
    }
    let activity = state.begin_project_activity(project_id)?;
    let items = if upload {
        crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
        state
            .store
            .require_unarchived_session(session)
            .await
            .map_err(|error| error.to_string())?;
        if args.context_id == "local" {
            let root = project.root;
            let path = args.path.clone();
            let sources = args.source_paths.clone();
            let (results, _activity) = tokio::task::spawn_blocking(move || {
                (
                    crate::file_browser::upload_local_files_at(&root, &path, sources),
                    activity,
                )
            })
            .await
            .map_err(|error| error.to_string())?;
            let results = results?;
            if results.iter().any(|item| item.path.is_some()) {
                state
                    .store
                    .bump_state_generation(&scope)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            results
                .into_iter()
                .map(|item| dto::TransferItem {
                    source_path: item.source,
                    status: if item.path.is_some() {
                        "succeeded"
                    } else {
                        "failed"
                    }
                    .into(),
                    destination_path: item.path,
                    error: item.error,
                    run_id: None,
                })
                .collect()
        } else {
            let _activity = activity;
            remote_upload(&state.store, &state.run_manager, project_id, session, &args).await?
        }
    } else {
        let _activity = activity;
        let context = require_ssh(&state.store, &args.context_id).await?;
        let destination = args
            .destination_path
            .as_ref()
            .ok_or("Download destination is required")?;
        let run_id = state
            .run_manager
            .submit_ssh_file_download(
                &state.store,
                project_id,
                Some(session),
                &context,
                &args.path,
                Path::new(destination),
            )
            .await?;
        vec![dto::TransferItem {
            source_path: args.path.clone(),
            destination_path: Some(destination.clone()),
            run_id: Some(run_id),
            status: "running".into(),
            error: None,
        }]
    };
    value(dto::Transfer {
        schema: dto::SCHEMA.into(),
        project_id: project_id.into(),
        session_id: session.into(),
        context_id: args.context_id,
        path: args.path,
        items,
    })
}

fn validate_transfer(
    args: &dto::TransferRequest,
    session: &str,
    upload: bool,
) -> Result<(), String> {
    if args.session_id != session {
        return Err("Session scope mismatch".into());
    }
    if upload {
        if args.destination_path.is_some()
            || args.source_paths.is_empty()
            || args
                .source_paths
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != args.source_paths.len()
            || args
                .source_paths
                .iter()
                .any(|path| !Path::new(path).is_absolute() || path.contains('\0'))
        {
            return Err(
                "Upload requires unique absolute local sources and one destination directory"
                    .into(),
            );
        }
        if args.context_id != "local" {
            crate::run_context::join_remote_upload_destination(&args.path, "item")?;
        }
    } else {
        if !args.source_paths.is_empty() || args.context_id == "local" {
            return Err("Download requires one SSH file".into());
        }
        remote_reference(&args.context_id, &args.path)?;
        let destination = args
            .destination_path
            .as_ref()
            .ok_or("Download destination is required")?;
        if !Path::new(destination).is_absolute() || destination.contains('\0') {
            return Err("Download destination must be absolute".into());
        }
    }
    Ok(())
}

async fn remote_upload(
    store: &wisp_store::Store,
    manager: &crate::run_context::RunManager,
    project: &str,
    session: &str,
    args: &dto::TransferRequest,
) -> Result<Vec<dto::TransferItem>, String> {
    let items = crate::run_context::submit_local_uploads_to_context(
        store,
        manager,
        project,
        Some(session),
        &args.context_id,
        &args.path,
        &args.source_paths,
    )
    .await?;
    Ok(items
        .into_iter()
        .zip(&args.source_paths)
        .map(|(item, source)| dto::TransferItem {
            source_path: source.clone(),
            destination_path: Some(item.destination_path),
            run_id: Some(item.run_id),
            status: item.status,
            error: None,
        })
        .collect())
}

fn value<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn locations(contexts: &[wisp_store::ExecutionContext]) -> Vec<dto::Location> {
    let mut result = vec![dto::Location {
        context_id: "local".into(),
        label: String::new(),
        kind: "local".into(),
    }];
    result.extend(
        contexts
            .iter()
            .filter(|context| context.kind == wisp_store::ExecutionContextKind::Ssh)
            .map(|context| dto::Location {
                context_id: context.id.clone(),
                label: if context.label.trim().is_empty() {
                    context.id.clone()
                } else {
                    context.label.clone()
                },
                kind: "ssh".into(),
            }),
    );
    result
}

async fn require_ssh(
    store: &wisp_store::Store,
    id: &str,
) -> Result<wisp_store::ExecutionContext, String> {
    let context = store
        .get_execution_context(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Execution context not found")?;
    if context.kind != wisp_store::ExecutionContextKind::Ssh {
        return Err("Files remote location must be an SSH context".into());
    }
    Ok(context)
}

fn local_directory(root: &Path, path: &str) -> Result<wisp_dto::DirectoryListing, String> {
    let root = wisp_tools::safety::resolve_under_root(root, ".")?;
    let directory = wisp_tools::safety::resolve_under_root(&root, path)?;
    let relative = relative_name(directory.strip_prefix(&root).map_err(|e| e.to_string())?);
    Ok(wisp_dto::DirectoryListing {
        path: if relative.is_empty() {
            ".".into()
        } else {
            relative
        },
        entries: crate::file_browser::list_dir_entries(&directory)?,
    })
}

fn local_paths(root: &Path, paths: &[String]) -> Result<Vec<dto::PathPair>, String> {
    if paths.is_empty() {
        return Err("Select at least one path".into());
    }
    if paths.iter().collect::<std::collections::HashSet<_>>().len() != paths.len() {
        return Err("Duplicate selected paths".into());
    }
    let root = wisp_tools::safety::resolve_under_root(root, ".")?;
    paths
        .iter()
        .map(|requested| {
            let absolute = wisp_tools::safety::resolve_under_root(&root, requested)?;
            if !absolute.exists() {
                return Err("Selected path no longer exists".into());
            }
            let relative = relative_name(absolute.strip_prefix(&root).map_err(|e| e.to_string())?);
            if relative.is_empty() {
                return Err("Select a project file or directory".into());
            }
            Ok(dto::PathPair {
                requested_path: requested.clone(),
                relative_path: relative,
                absolute_path: absolute.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

fn relative_name(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    // A backslash is a valid filename character on macOS/Linux, while it is a
    // separator on Windows. Normalize only the latter's directory separators.
    #[cfg(windows)]
    {
        value.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        value
    }
}

fn remote_reference(context: &str, path: &str) -> Result<String, String> {
    let alias = context
        .strip_prefix("ssh:")
        .filter(|alias| !alias.is_empty())
        .ok_or("Invalid SSH context ID")?;
    if alias.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Invalid SSH context ID".into());
    }
    if !(path.starts_with('/') || path.starts_with("~/"))
        || path.contains('\0')
        || path == "/"
        || path == "~/"
    {
        return Err("Remote file path must be absolute or home-relative".into());
    }
    Ok(format!("ssh://{alias}/{}", path.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_requests_require_exact_session_unique_absolute_sources_and_matching_operation_fields(
    ) {
        let source = std::env::temp_dir()
            .join("upload.csv")
            .to_string_lossy()
            .into_owned();
        let mut args = dto::TransferRequest {
            session_id: "s".into(),
            context_id: "local".into(),
            path: ".".into(),
            source_paths: vec![source.clone()],
            destination_path: None,
        };
        assert!(validate_transfer(&args, "s", true).is_ok());
        assert!(validate_transfer(&args, "foreign", true).is_err());
        args.source_paths.push(source.clone());
        assert!(validate_transfer(&args, "s", true).is_err());
        args.source_paths = vec!["relative.csv".into()];
        assert!(validate_transfer(&args, "s", true).is_err());
        args.source_paths = vec![source.clone()];
        args.context_id = "ssh:lab".into();
        for path in ["../results", "/results/*", "/results\nother"] {
            args.path = path.into();
            assert!(validate_transfer(&args, "s", true).is_err());
        }
        args.path = "/results".into();
        assert!(validate_transfer(&args, "s", true).is_ok());
        args.destination_path = Some(source);
        assert!(validate_transfer(&args, "s", true).is_err());
        assert!(validate_transfer(&args, "s", false).is_err());
        args.source_paths.clear();
        args.path = "/results/qc.csv".into();
        assert!(validate_transfer(&args, "s", false).is_ok());
        args.context_id = "local".into();
        assert!(validate_transfer(&args, "s", false).is_err());
    }

    #[test]
    fn local_uploads_preserve_branch_scope_collisions_partial_results_and_native_names() {
        let root = tempfile::tempdir().unwrap();
        let branch = root.path().join("branch");
        std::fs::create_dir_all(branch.join("results")).unwrap();
        let source = root.path().join("QC.csv");
        std::fs::write(&source, b"new QC bytes").unwrap();
        std::fs::write(branch.join("results/QC.csv"), b"existing QC").unwrap();
        let results = crate::file_browser::upload_local_files_at(
            &branch,
            "results",
            vec![
                source.to_string_lossy().into_owned(),
                root.path().join("missing").to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        assert_eq!(results[0].path.as_deref(), Some("results/QC_1.csv"));
        assert!(results[1].path.is_none() && results[1].error.is_some());
        assert_eq!(
            std::fs::read(branch.join("results/QC.csv")).unwrap(),
            b"existing QC"
        );
        assert_eq!(
            std::fs::read(branch.join("results/QC_1.csv")).unwrap(),
            b"new QC bytes"
        );
        assert!(!root.path().join("results").exists());
        assert!(crate::file_browser::upload_local_files_at(
            &branch,
            "..",
            vec![source.to_string_lossy().into_owned()]
        )
        .is_err());
        #[cfg(unix)]
        {
            let source = root.path().join("QC\\copy.csv");
            std::fs::write(&source, b"literal backslash").unwrap();
            let results = crate::file_browser::upload_local_files_at(
                &branch,
                "results",
                vec![source.to_string_lossy().into_owned()],
            )
            .unwrap();
            assert_eq!(results[0].path.as_deref(), Some("results/QC\\copy.csv"));
        }
    }

    #[test]
    fn directory_and_copies_use_the_actual_physical_working_root() {
        let root = tempfile::tempdir().unwrap();
        let working = root.path().join("branch");
        std::fs::create_dir_all(working.join("results")).unwrap();
        std::fs::write(working.join("results/qc.csv"), "n,value\n1,2\n").unwrap();
        let listing = local_directory(&working, "results").unwrap();
        assert_eq!(listing.path, "results");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "qc.csv");
        let paths = local_paths(&working, &["results/qc.csv".into(), "results".into()]).unwrap();
        assert_eq!(paths[0].requested_path, "results/qc.csv");
        assert_eq!(paths[1].relative_path, "results");
        assert_eq!(
            Path::new(&paths[0].absolute_path),
            working.canonicalize().unwrap().join("results/qc.csv")
        );
        assert!(local_paths(&working, &[]).is_err());
        assert!(local_paths(&working, &["results".into(), "results".into()]).is_err());
        for path in [".", "missing.csv", "../branch-other/qc.csv"] {
            assert!(local_paths(&working, &[path.into()]).is_err(), "{path}");
        }
        assert!(local_directory(&working, "results/qc.csv").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn root_alias_is_accepted_and_external_links_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let working = root.path().join("working");
        let outside = root.path().join("outside");
        std::fs::create_dir(&working).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(working.join("a.txt"), "a").unwrap();
        std::fs::write(outside.join("b.txt"), "b").unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&working, &alias).unwrap();
        std::os::unix::fs::symlink(&outside, working.join("escape")).unwrap();
        assert_eq!(local_directory(&alias, ".").unwrap().path, ".");
        assert_eq!(
            local_paths(&alias, &["a.txt".into()]).unwrap()[0].absolute_path,
            working
                .canonicalize()
                .unwrap()
                .join("a.txt")
                .to_string_lossy()
        );
        assert!(local_directory(&alias, "escape").is_err());
        assert!(local_paths(&alias, &["escape/b.txt".into()]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_directory_and_path_copies_preserve_literal_backslashes() {
        let root = tempfile::tempdir().unwrap();
        let directory = "results\\v2";
        std::fs::create_dir(root.path().join(directory)).unwrap();
        let path = format!("{directory}/QC.csv");
        std::fs::write(root.path().join(&path), "sample,QC\nA,pass\n").unwrap();
        let listing = local_directory(root.path(), directory).unwrap();
        assert_eq!(listing.path, directory);
        assert_eq!(listing.entries[0].name, "QC.csv");
        let paths = local_paths(root.path(), &[path.clone()]).unwrap();
        assert_eq!(paths[0].relative_path, path);
        assert_eq!(
            Path::new(&paths[0].absolute_path),
            root.path().canonicalize().unwrap().join(path)
        );
    }

    #[tokio::test]
    async fn location_catalog_uses_all_configured_ssh_without_probe_or_secrets() {
        let root = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&root.path().join("store.db"))
            .await
            .unwrap();
        let mut ssh = wisp_store::ExecutionContext::new("ssh:lab", "Lab server").unwrap();
        ssh.config_json = r#"{"private_key":"never-export-this"}"#.into();
        store.upsert_execution_context(&ssh).await.unwrap();
        let wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        store.upsert_execution_context(&wsl).await.unwrap();
        let values = locations(&store.list_execution_contexts().await.unwrap());
        assert_eq!(
            values
                .iter()
                .map(|v| v.context_id.as_str())
                .collect::<Vec<_>>(),
            ["local", "ssh:lab"]
        );
        assert_eq!(values[1].label, "Lab server");
        assert!(!serde_json::to_string(&values)
            .unwrap()
            .contains("never-export-this"));
        require_ssh(&store, "ssh:lab").await.unwrap();
        assert!(require_ssh(&store, "local").await.is_err());
        assert!(require_ssh(&store, "wsl:Ubuntu").await.is_err());
        assert!(require_ssh(&store, "ssh:missing").await.is_err());
    }

    #[test]
    fn remote_quotes_identify_the_exact_context_and_original_path() {
        assert_eq!(
            remote_reference("ssh:lab", "/work/project/qc.csv").unwrap(),
            "ssh://lab/work/project/qc.csv"
        );
        assert_eq!(
            remote_reference("ssh:lab", "~/results/qc.csv").unwrap(),
            "ssh://lab/~/results/qc.csv"
        );
        assert_ne!(
            remote_reference("ssh:lab", "/qc.csv").unwrap(),
            remote_reference("ssh:other", "/qc.csv").unwrap()
        );
        for (context, path) in [
            ("local", "/a"),
            ("ssh:", "/a"),
            ("ssh:bad\n", "/a"),
            ("ssh:lab", "relative"),
            ("ssh:lab", "/"),
            ("ssh:lab", "/a\0"),
        ] {
            assert!(remote_reference(context, path).is_err());
        }
    }
}
