use crate::{workspace_surface::WorkspaceSurface, AppState};
use std::{io::Write, path::Path};
use tauri::{Manager, State};
use wisp_dto::{DocxReviewAction, DocxReviewSelection, DocxReviewSnapshot};
use wisp_tools::docx::review::{inspect_revisions, read_document, resolve_revisions};

fn read_at(root: &Path, path: &str, writable: bool) -> Result<DocxReviewSnapshot, String> {
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    let mut snapshot = inspect_revisions(&read_document(&real)?)?;
    snapshot.writable = writable;
    Ok(snapshot)
}

fn resolve_at(
    root: &Path,
    path: &str,
    source_sha256: &str,
    selection: &DocxReviewSelection,
    action: DocxReviewAction,
) -> Result<DocxReviewSnapshot, String> {
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    let source = read_document(&real)?;
    let output = resolve_revisions(&source, source_sha256, selection, action)?;
    let mut snapshot = inspect_revisions(&output)?;
    // Retain a content-addressed preimage before atomically replacing the review
    // copy. Fixed hidden directories may not be redirected by symlinks.
    for name in [".wisp", ".wisp/docx-review"] {
        let dir = root.join(name);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        let metadata = std::fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("DOCX backup directory must be a local directory".into());
        }
    }
    // Hash was validated by resolve_revisions; it cannot contain path syntax.
    let backup = root
        .join(".wisp/docx-review")
        .join(format!("{source_sha256}.docx"));
    let mut backup_temp =
        tempfile::NamedTempFile::new_in(backup.parent().unwrap()).map_err(|e| e.to_string())?;
    backup_temp
        .write_all(&source)
        .and_then(|()| backup_temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    if let Err(e) = backup_temp.persist_noclobber(&backup) {
        if std::fs::symlink_metadata(&backup)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
            || std::fs::read(&backup).map_err(|e| e.to_string())? != source
        {
            return Err(e.to_string());
        }
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(real.parent().ok_or("Missing parent directory")?)
            .map_err(|e| e.to_string())?;
    temp.as_file()
        .set_permissions(
            std::fs::metadata(&real)
                .map_err(|e| e.to_string())?
                .permissions(),
        )
        .map_err(|e| e.to_string())?;
    temp.write_all(&output)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    if read_document(&real)? != source {
        return Err("DOCX changed; refresh before reviewing".into());
    }
    temp.persist(&real).map_err(|e| e.to_string())?;
    snapshot.writable = true;
    Ok(snapshot)
}

#[tauri::command]
pub(super) async fn read_docx_review(
    state: State<'_, AppState>,
    window: WorkspaceSurface,
    path: String,
) -> Result<DocxReviewSnapshot, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    let _activity = state.begin_project_activity(&project.id)?;
    let writable = crate::exploration_commands::require_writable_scope(&state.store, &scope)
        .await
        .is_ok();
    tokio::task::spawn_blocking(move || read_at(&project.root, &path, writable))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn resolve_docx_review(
    state: State<'_, AppState>,
    window: WorkspaceSurface,
    path: String,
    source_sha256: String,
    selection: DocxReviewSelection,
    action: DocxReviewAction,
) -> Result<DocxReviewSnapshot, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    resolve_scoped(
        &state,
        project.id,
        project.root,
        scope,
        path,
        source_sha256,
        selection,
        action,
    )
    .await
}

async fn resolve_scoped(
    state: &AppState,
    project_id: String,
    root: std::path::PathBuf,
    scope: wisp_store::StateScope,
    path: String,
    source_sha256: String,
    selection: DocxReviewSelection,
    action: DocxReviewAction,
) -> Result<DocxReviewSnapshot, String> {
    let activity = state.begin_project_activity(&project_id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let request =
        crate::resource_leases::request_for_call(&root, "edit", &serde_json::json!({"path":path}))
            .ok_or("DOCX path is outside this project")?;
    let lease = match state.resource_leases.try_acquire(
        &project_id,
        &format!("docx-review-{}", uuid::Uuid::new_v4()),
        "resolve_docx_review",
        &path,
        request,
    ) {
        crate::resource_leases::AcquireResult::Acquired(lease) => lease,
        crate::resource_leases::AcquireResult::Conflict(_) => {
            return Err("DOCX is being edited; wait and refresh the preview".into())
        }
    };
    let snapshot = tokio::task::spawn_blocking(move || {
        let (_activity, _lease) = (activity, lease);
        resolve_at(&root, &path, &source_sha256, &selection, action)
    })
    .await
    .map_err(|e| e.to_string())??;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|e| e.to_string())?;
    Ok(snapshot)
}

fn validate_native_request(
    args: &wisp_dto::NativeDocxReviewRequest,
    session: Option<&str>,
    resolve: bool,
) -> Result<(), String> {
    if args.session_id.as_deref() != session {
        return Err("Session scope mismatch".into());
    }
    let complete =
        args.source_sha256.is_some() && args.selection.is_some() && args.action.is_some();
    let empty = args.source_sha256.is_none() && args.selection.is_none() && args.action.is_none();
    if (resolve && !complete) || (!resolve && !empty) {
        return Err("DOCX review action arguments do not match command".into());
    }
    if args.path.starts_with("artifact:")
        || args.path.starts_with("artifact-version:")
        || args.path.starts_with("remote:")
        || args.path.contains("://")
    {
        return Err("Open the current local DOCX file to review changes".into());
    }
    Ok(())
}

fn native_read_at(
    root: &Path,
    path: &str,
    writable: bool,
) -> Result<(DocxReviewSnapshot, wisp_dto::FileContent), String> {
    use base64::Engine;
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    let bytes = read_document(&real)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("DOCX exceeds native preview limit".into());
    }
    let mut review = inspect_revisions(&bytes)?;
    review.writable = writable;
    // Use the same bytes for the snapshot and render, including the existing
    // TIFF display conversion. Conversion never changes the file under review.
    let bytes = crate::file_browser::transcode_ooxml_tiff_media(bytes);
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("DOCX exceeds native preview limit".into());
    }
    let content = wisp_dto::FileContent {
        path: real.to_string_lossy().into_owned(),
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        text: None,
        base64: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
        truncated: false,
        total_bytes: Some(bytes.len() as u64),
    };
    Ok((review, content))
}

pub(super) async fn dispatch_native(
    broker: &crate::native_settings::Broker,
    request: &wisp_dto::native_settings::Request,
    project_id: &str,
    session: Option<&str>,
    resolve: bool,
) -> Result<serde_json::Value, String> {
    let args: wisp_dto::NativeDocxReviewRequest =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    validate_native_request(&args, session, resolve)?;
    let state = broker.app.state::<AppState>();
    let (root, scope) = crate::native_files::file_scope(&state, project_id, session).await?;
    let writable = crate::exploration_commands::require_writable_scope(&state.store, &scope)
        .await
        .is_ok()
        && if let Some(session) = session {
            state
                .store
                .require_unarchived_session(session)
                .await
                .is_ok()
        } else {
            true
        };
    if resolve {
        if !writable {
            return Err("DOCX review is read-only in this workspace".into());
        }
        resolve_scoped(
            &state,
            project_id.into(),
            root.clone(),
            scope,
            args.path.clone(),
            args.source_sha256.unwrap(),
            args.selection.unwrap(),
            args.action.unwrap(),
        )
        .await?;
    }
    let activity = state.begin_project_activity(project_id)?;
    let path = args.path.clone();
    let (review, content) = tokio::task::spawn_blocking(move || {
        let _activity = activity;
        native_read_at(&root, &path, writable)
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::to_value(wisp_dto::NativeDocxReview {
        schema: wisp_dto::native_files::SCHEMA.into(),
        project_id: project_id.into(),
        session_id: session.map(str::to_owned),
        path: args.path,
        review,
        content,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
