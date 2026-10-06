//! Import a wisp session-export archive (produced by `session_export::export_session`)
//! as a session in the current project. Mirrors the Codex/Claude importers:
//! re-importing the same source session fast-forwards the existing frame
//! instead of creating a duplicate (`session_imports` table).
//!
//! Deliberately not restored: `provenance/*.json` (execution_log rows reference
//! cell indexes that are meaningless across databases) and `tool-calls.json` /
//! `transcript.md` (derived views of `messages.json`).

use super::{models, AppState};
use crate::session_export::{to_workspace_rel, zip_component};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use wisp_llm::Message;
use wisp_store::Store;

#[derive(Clone, serde::Deserialize)]
struct ImportManifestArtifact {
    workspace_path: String,
    zip_path: String,
    #[serde(default)]
    mime: String,
}

#[derive(serde::Deserialize)]
struct ImportManifest {
    session_id: String,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    artifacts: Vec<ImportManifestArtifact>,
}

struct ParsedImport {
    session_id: String,
    exported_at: String,
    messages: Vec<Message>,
    artifacts: Vec<ImportManifestArtifact>,
}

const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

fn read_archive_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open archive: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_ARCHIVE_BYTES {
        return Err("Session archive exceeds the 256 MiB input limit".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ARCHIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err("Session archive grew beyond the input limit".into());
    }
    Ok(bytes)
}

fn read_zip_string<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, String> {
    let entry = zip
        .by_name(name)
        .map_err(|_| format!("not a wisp session export: {name} missing"))?;
    if entry.size() > MAX_TEXT_BYTES {
        return Err(format!("{name} exceeds the 64 MiB text limit"));
    }
    let mut body = String::new();
    entry
        .take(MAX_TEXT_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|e| format!("read {name}: {e}"))?;
    if body.len() as u64 > MAX_TEXT_BYTES {
        return Err(format!("{name} exceeds the 64 MiB text limit"));
    }
    Ok(body)
}

#[cfg(test)]
fn parse_import_archive(path: &Path) -> Result<ParsedImport, String> {
    parse_import_bytes(&read_archive_bytes(path)?)
}

fn parse_import_bytes(bytes: &[u8]) -> Result<ParsedImport, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("open archive: {e}"))?;
    if zip.len() > 4096 {
        return Err("Session archive contains too many entries".into());
    }
    let mut total = 0u64;
    for index in 0..zip.len() {
        total = total
            .checked_add(zip.by_index(index).map_err(|e| e.to_string())?.size())
            .ok_or("Archive size overflow")?;
        if total > MAX_EXPANDED_BYTES {
            return Err("Session archive exceeds the 512 MiB expanded limit".into());
        }
    }

    let manifest: ImportManifest =
        serde_json::from_str(&read_zip_string(&mut zip, "manifest.json")?)
            .map_err(|e| format!("parse manifest.json: {e}"))?;
    if manifest.session_id.trim().is_empty() {
        return Err("not a wisp session export: manifest session_id is empty".into());
    }
    let messages: Vec<Message> = serde_json::from_str(&read_zip_string(&mut zip, "messages.json")?)
        .map_err(|e| format!("parse messages.json: {e}"))?;
    if messages.is_empty() {
        return Err("not a wisp session export: no messages".into());
    }
    for artifact in &manifest.artifacts {
        if artifact.zip_path.contains("..") || !artifact.zip_path.starts_with("artifacts/") {
            return Err(format!("invalid artifact zip path: {}", artifact.zip_path));
        }
    }
    Ok(ParsedImport {
        session_id: manifest.session_id,
        exported_at: manifest.exported_at,
        messages,
        artifacts: manifest.artifacts,
    })
}

/// Reject absolute paths and any `..`/`.` components; return the path as a
/// safe relative form. `root` itself is the trusted workspace root.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Where an exported artifact lands in this workspace. The recorded
/// `workspace_path` wins when it is relative, safe, and free; anything else
/// (absolute foreign paths, traversal, collisions) falls back to
/// `imports/<session>/`. `None` means skip the artifact.
fn artifact_target(root: &Path, workspace_path: &str, session_id: &str) -> Option<PathBuf> {
    let file_name = Path::new(workspace_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(zip_component)
        .unwrap_or_else(|| "artifact".into());
    if let Some(rel) = safe_relative(workspace_path) {
        let target = root.join(rel);
        if !target.exists() {
            return Some(target);
        }
    }
    let target = root
        .join("imports")
        .join(zip_component(session_id))
        .join(file_name);
    if target.exists() {
        return None;
    }
    Some(target)
}

/// Extract archived artifacts into the workspace. IO-bound: call from
/// spawn_blocking. Failures skip the artifact instead of aborting the import.
fn extract_artifacts_from<R: Read + Seek>(
    reader: R,
    artifacts: &[ImportManifestArtifact],
    root: &Path,
    session_id: &str,
) -> (Vec<(String, PathBuf, String)>, Vec<String>) {
    let mut extracted = vec![];
    let mut missing = vec![];
    let mut zip = match zip::ZipArchive::new(reader) {
        Ok(zip) => zip,
        Err(e) => {
            missing.extend(
                artifacts
                    .iter()
                    .map(|a| format!("{}: {e}", a.workspace_path)),
            );
            return (extracted, missing);
        }
    };
    let mut remaining = MAX_EXPANDED_BYTES;
    for artifact in artifacts {
        let result = (|| -> Result<PathBuf, String> {
            let target = artifact_target(root, &artifact.workspace_path, session_id)
                .ok_or_else(|| "target already exists".to_string())?;
            let entry = zip
                .by_name(&artifact.zip_path)
                .map_err(|e| format!("{e}"))?;
            if entry.size() > remaining {
                return Err("Artifact extraction exceeds the expanded archive limit".into());
            }
            if let Some(parent) = target.parent() {
                // Check the nearest existing parent before creating directories;
                // a workspace symlink must not redirect extraction outside it.
                let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
                let mut ancestor = parent;
                while !ancestor.exists() {
                    ancestor = ancestor.parent().ok_or("Invalid artifact parent")?;
                }
                if !ancestor
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&canonical_root)
                {
                    return Err("Artifact parent leaves the destination workspace".into());
                }
                std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
                if !parent
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&canonical_root)
                {
                    return Err("Artifact parent leaves the destination workspace".into());
                }
            }
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| format!("{e}"))?;
            let copied = std::io::copy(&mut entry.take(remaining + 1), &mut out);
            drop(out);
            match copied {
                Ok(bytes) if bytes <= remaining => remaining -= bytes,
                outcome => {
                    // Only this call's create_new file is removed on failed extraction.
                    let _ = std::fs::remove_file(&target);
                    remaining = 0;
                    return Err(match outcome {
                        Err(error) => error.to_string(),
                        _ => "Artifact extraction exceeds the expanded archive limit".into(),
                    });
                }
            }
            Ok(target)
        })();
        match result {
            Ok(target) => extracted.push((
                artifact.workspace_path.clone(),
                target,
                artifact.mime.clone(),
            )),
            Err(e) => missing.push(format!("{}: {e}", artifact.workspace_path)),
        }
    }
    (extracted, missing)
}

/// Folder imported sessions are grouped under in the sidebar.
async fn ensure_import_folder(store: &Store, project_id: &str) -> Result<String, String> {
    if let Some((id, _, _)) = store
        .list_folders(project_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(_, name, _)| name.eq_ignore_ascii_case("imported"))
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_folder(&id, project_id, "imported")
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

fn import_timestamps(parsed: &ParsedImport) -> (i64, i64) {
    let mut created = parsed
        .messages
        .iter()
        .map(|m| m.ts)
        .filter(|ts| *ts > 0)
        .min();
    let mut updated = parsed
        .messages
        .iter()
        .map(|m| m.ts)
        .filter(|ts| *ts > 0)
        .max();
    if created.is_none() || updated.is_none() {
        let exported = chrono::DateTime::parse_from_rfc3339(&parsed.exported_at)
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        created = created.or(Some(exported));
        updated = updated.or(Some(exported));
    }
    (created.unwrap(), updated.unwrap())
}

pub(crate) fn project_import_key(project_id: &str, source_session_id: &str) -> String {
    format!(
        "project:{}:{project_id}:{source_session_id}",
        project_id.len()
    )
}

async fn existing_import(
    store: &Store,
    project_id: &str,
    source_session_id: &str,
) -> Result<Option<String>, String> {
    for key in [
        project_import_key(project_id, source_session_id),
        source_session_id.to_owned(),
    ] {
        if let Some(frame) = store
            .find_session_import(&key)
            .await
            .map_err(|e| e.to_string())?
        {
            if store
                .frame_project_id(&frame)
                .await
                .map_err(|e| e.to_string())?
                .as_deref()
                == Some(project_id)
            {
                return Ok(Some(frame));
            }
        }
    }
    Ok(None)
}

/// Create or fast-forward the frame for a parsed archive. Returns the frame id
/// and the outcome ("imported" / "updated" / "skipped").
async fn import_parsed(
    store: &Store,
    project_id: &str,
    model_id: &str,
    source_path: &str,
    parsed: &ParsedImport,
) -> Result<(String, &'static str), String> {
    // Imports from different windows must not race the initial lookup/insertion.
    static IMPORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _import = IMPORTS.lock().await;
    let (created_at, updated_at) = import_timestamps(parsed);

    if let Some(frame_id) = existing_import(store, project_id, &parsed.session_id).await? {
        let stored = store
            .message_count(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        // Only fast-forward: if the imported session was continued inside Wisp
        // it can hold more turns than the archive; merging diverged histories
        // is out of scope, so leave it untouched.
        if (parsed.messages.len() as i64) <= stored {
            return Ok((frame_id, "skipped"));
        }
        store
            .replace_messages(&frame_id, &parsed.messages)
            .await
            .map_err(|e| e.to_string())?;
        store
            .set_frame_timestamps(&frame_id, created_at, updated_at)
            .await
            .map_err(|e| e.to_string())?;
        store
            .record_session_import(
                &project_import_key(project_id, &parsed.session_id),
                &frame_id,
                source_path,
            )
            .await
            .map_err(|e| e.to_string())?;
        return Ok((frame_id, "updated"));
    }

    let frame_id = uuid::Uuid::new_v4().to_string();
    let folder_id = ensure_import_folder(store, project_id).await?;
    store
        .create_frame(&frame_id, project_id, "OPERON", model_id)
        .await
        .map_err(|e| e.to_string())?;
    store
        .move_session_to_folder(&frame_id, project_id, Some(&folder_id))
        .await
        .map_err(|e| e.to_string())?;
    for (i, msg) in parsed.messages.iter().enumerate() {
        store
            .append_message(&frame_id, (i + 1) as i64, msg)
            .await
            .map_err(|e| e.to_string())?;
    }
    store
        .set_frame_timestamps(&frame_id, created_at, updated_at)
        .await
        .map_err(|e| e.to_string())?;
    store
        .record_session_import(
            &project_import_key(project_id, &parsed.session_id),
            &frame_id,
            source_path,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok((frame_id, "imported"))
}

struct PreparedArchive {
    bytes: Vec<u8>,
    parsed: ParsedImport,
    sha256: String,
}

async fn prepare_archive(path: &str) -> Result<PreparedArchive, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("Choose an absolute session archive path".into());
    }
    tokio::task::spawn_blocking(move || {
        let bytes = read_archive_bytes(&path)?;
        let sha256 = wisp_sync::sha256_hex(&bytes);
        let parsed = parse_import_bytes(&bytes)?;
        Ok(PreparedArchive {
            bytes,
            parsed,
            sha256,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn preview_archive(
    store: &Store,
    project: &str,
    path: &str,
    prepared: &PreparedArchive,
) -> Result<wisp_dto::native_session_import::ArchivePreview, String> {
    use wisp_dto::native_session_import::{ArchivePreview, SCHEMA};
    let parsed = &prepared.parsed;
    let existing = existing_import(store, project, &parsed.session_id).await?;
    let state = if let Some(id) = &existing {
        if store.message_count(id).await.map_err(|e| e.to_string())? < parsed.messages.len() as i64
        {
            "updatable"
        } else {
            "imported"
        }
    } else {
        "new"
    };
    let messages = parsed
        .messages
        .iter()
        .filter_map(|m| {
            let role = match m.role {
                wisp_llm::Role::User => "user",
                wisp_llm::Role::Assistant => "assistant",
                _ => return None,
            };
            Some(wisp_dto::ExternalSessionPreviewLine {
                role: role.into(),
                text: m.content.as_text().chars().take(600).collect(),
            })
        })
        .take(4)
        .collect();
    let title = parsed
        .messages
        .iter()
        .find(|m| m.role == wisp_llm::Role::User)
        .map(|m| m.content.as_text().chars().take(120).collect())
        .unwrap_or_else(|| parsed.session_id.clone());
    Ok(ArchivePreview {
        schema: SCHEMA.into(),
        project_id: project.into(),
        archive_path: path.into(),
        sha256: prepared.sha256.clone(),
        source_session_id: parsed.session_id.clone(),
        title,
        message_count: parsed.messages.len(),
        artifacts: parsed
            .artifacts
            .iter()
            .map(|a| a.workspace_path.clone())
            .collect(),
        messages,
        existing_session_id: existing,
        state: state.into(),
    })
}

fn validate_review(
    prepared: &PreparedArchive,
    request: &wisp_dto::native_session_import::ImportRequest,
) -> Result<(), String> {
    if prepared.sha256 != request.sha256 || prepared.parsed.session_id != request.source_session_id
    {
        return Err("The archive changed after preview. Preview it again before importing.".into());
    }
    Ok(())
}

async fn apply_prepared_archive(
    store: &Store,
    project: &str,
    root: &Path,
    model: &str,
    source: &str,
    prepared: PreparedArchive,
) -> Result<wisp_dto::native_session_import::ImportResult, String> {
    use wisp_dto::native_session_import::{ImportResult, SCHEMA};
    let parsed = prepared.parsed;
    let (frame_id, status) = import_parsed(store, project, model, source, &parsed).await?;
    let mut artifact_count = 0;
    let mut missing_artifacts = vec![];
    // Artifacts are restored on first import only; a fast-forward update keeps
    // the files and artifact rows registered by the initial import.
    if status == "imported" && !parsed.artifacts.is_empty() {
        let destination = root.to_owned();
        let source_id = parsed.session_id.clone();
        let artifacts = parsed.artifacts.clone();
        let (extracted, missing) = tokio::task::spawn_blocking(move || {
            extract_artifacts_from(
                std::io::Cursor::new(prepared.bytes),
                &artifacts,
                &destination,
                &source_id,
            )
        })
        .await
        .map_err(|e| e.to_string())?;
        missing_artifacts = missing;
        for (original, target, mime) in extracted {
            let filename = target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("artifact");
            match store
                .save_artifact(
                    &uuid::Uuid::new_v4().to_string(),
                    project,
                    &frame_id,
                    filename,
                    &mime,
                    &to_workspace_rel(root, &target.to_string_lossy()),
                )
                .await
            {
                Ok(_) => artifact_count += 1,
                Err(error) => {
                    missing_artifacts.push(format!("{original}: registration failed: {error}"))
                }
            }
        }
    }
    Ok(ImportResult {
        schema: SCHEMA.into(),
        project_id: project.into(),
        source_session_id: parsed.session_id,
        frame_id,
        status: status.into(),
        message_count: parsed.messages.len(),
        artifact_count,
        missing_artifacts,
    })
}

/// Every import that may update an existing conversation holds its workflow
/// lock, so it never rewrites a running or queued conversation.
pub(crate) async fn lock_import_target(
    state: &AppState,
    project: &str,
    existing: Option<&str>,
) -> Result<
    Option<(
        std::sync::Arc<crate::SessionRuntime>,
        tokio::sync::OwnedMutexGuard<()>,
    )>,
    String,
> {
    let Some(id) = existing else {
        return Ok(None);
    };
    state
        .store
        .require_unarchived_session(id)
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(
        state.store.frame_state_scope(id).await.map_err(|e| e.to_string())?,
        Some(wisp_store::StateScope::Mainline { project_id }) if project_id == project
    ) {
        return Err(
            "Session imports can only update mainline conversations in the selected project".into(),
        );
    }
    if state
        .store
        .get_acp_session(id)
        .await
        .map_err(|e| e.to_string())?
        .is_some()
        || state
            .store
            .session_branch_state(id)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err(
            "Bound ACP sessions and conversation branches cannot be updated by session import"
                .into(),
        );
    }
    if state.running_turns.lock().await.contains(id) || state.reviewing.lock().unwrap().contains(id)
    {
        return Err("Wait for the imported conversation to finish before updating it".into());
    }
    let rt = state.session_runtime(id).await;
    let guard = rt
        .workflow
        .clone()
        .try_lock_owned()
        .map_err(|_| "The imported conversation is busy")?;
    if rt.has_queued_turns() {
        return Err("Remove queued messages before updating this conversation".into());
    }
    Ok(Some((rt, guard)))
}

/// Apply a prepared archive into `project`; WebView and native imports share it.
/// An updated conversation is locked meanwhile and reloads its agent afterwards.
/// The caller holds a project activity guard, so the destination cannot move
/// while its root is resolved here and the archive lands in it.
async fn import_prepared_archive(
    state: &AppState,
    project: &str,
    source: &str,
    prepared: PreparedArchive,
) -> Result<wisp_dto::native_session_import::ImportResult, String> {
    let (_, root) = state
        .store
        .get_project(project)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Target project not found")?;
    let root = PathBuf::from(root);
    if !root.is_absolute() || !root.is_dir() {
        return Err("The target workspace directory is unavailable".into());
    }
    crate::exploration_commands::require_writable_scope(
        &state.store,
        &wisp_store::StateScope::mainline(project),
    )
    .await?;
    let existing = existing_import(&state.store, project, &prepared.parsed.session_id).await?;
    let locked = lock_import_target(state, project, existing.as_deref()).await?;
    let model = models::active_profile_id(&state.store).await;
    let result =
        apply_prepared_archive(&state.store, project, &root, &model, source, prepared).await?;
    if let Some((rt, _workflow)) = &locked {
        *rt.agent.lock().await = None;
        rt.sync_last_seq_from_store(&state.store, &result.frame_id)
            .await?;
    }
    Ok(result)
}

/// The explicit, regular destination project of a native import request.
pub(crate) async fn native_import_project<'a>(
    state: &AppState,
    request: &'a wisp_dto::native_settings::Request,
) -> Result<&'a str, String> {
    let project = request
        .project_id
        .as_deref()
        .filter(|id| !id.is_empty() && id.trim() == *id)
        .ok_or("An explicit destination project is required")?;
    if wisp_store::is_assistant_project_id(project) {
        return Err("Choose a regular destination project".into());
    }
    state
        .store
        .get_project(project)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Destination project not found")?;
    Ok(project)
}

/// Native archive import uses explicit project identity, without selecting a WebView.
pub(crate) async fn execute_native(
    state: &AppState,
    request: &wisp_dto::native_settings::Request,
) -> Result<serde_json::Value, String> {
    use wisp_dto::native_session_import::{ImportRequest, PreviewRequest};
    let project = native_import_project(state, request).await?;
    match request.command.as_str() {
        "native_session_archive_preview" => {
            let input: PreviewRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let prepared = prepare_archive(&input.archive_path).await?;
            serde_json::to_value(
                preview_archive(&state.store, project, &input.archive_path, &prepared).await?,
            )
            .map_err(|e| e.to_string())
        }
        "native_session_archive_import" => {
            let input: ImportRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let prepared = prepare_archive(&input.archive_path).await?;
            validate_review(&prepared, &input)?;
            let _project_guard = state.begin_project_exclusive_activity(project)?;
            let result =
                import_prepared_archive(state, project, &input.archive_path, prepared).await?;
            serde_json::to_value(result).map_err(|e| e.to_string())
        }
        _ => Err("Unsupported native session import command".into()),
    }
}

/// Pick a session-export zip and import it into the active project. Returns
/// the import summary, or `None` if the user cancelled the dialog.
#[tauri::command]
pub(super) async fn import_session_archive(
    app: AppHandle,
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<Option<wisp_dto::native_session_import::ImportResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    // The project this window showed when the import began, even if the
    // window switches projects while the picker is open.
    let project = state.require_active(window.label())?.id;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp session export", &["zip"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let source = picked.to_string();
    let prepared = prepare_archive(&source).await?;
    let _project_activity = state.begin_project_activity(&project)?;
    import_prepared_archive(&state, &project, &source, prepared)
        .await
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_archive_preview_and_apply_use_reviewed_bytes_in_the_selected_project() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        store
            .create_project("a", "A", a.to_str().unwrap())
            .await
            .unwrap();
        store
            .create_project("b", "B", b.to_str().unwrap())
            .await
            .unwrap();
        let messages = vec![
            Message::user("Compare synthetic samples"),
            Message::assistant("Counts ready"),
        ];
        let archive = build_archive(
            dir.path(),
            "source",
            &messages,
            &[(
                "artifacts/counts.csv",
                "results/counts.csv",
                "sample,count\nA,12",
            )],
        );
        let path = archive.to_str().unwrap();
        let first = prepare_archive(path).await.unwrap();
        let preview = preview_archive(&store, "a", path, &first).await.unwrap();
        assert_eq!(preview.message_count, 2);
        assert_eq!(preview.messages[0].text, "Compare synthetic samples");
        assert!(store.list_project_frame_ids("a").await.unwrap().is_empty());
        let review = wisp_dto::native_session_import::ImportRequest {
            archive_path: path.into(),
            sha256: preview.sha256,
            source_session_id: preview.source_session_id,
        };
        validate_review(&first, &review).unwrap();
        // The source is replaced after reading. Extraction must use the reviewed
        // bytes, never reopen the changed filename after creating the session.
        build_archive(
            dir.path(),
            "source",
            &messages,
            &[("artifacts/counts.csv", "results/counts.csv", "CHANGED")],
        );
        let changed = prepare_archive(path).await.unwrap();
        assert!(validate_review(&changed, &review).is_err());
        let imported_a = apply_prepared_archive(&store, "a", &a, "fake", path, first)
            .await
            .unwrap();
        assert_eq!(imported_a.status, "imported");
        assert_eq!(imported_a.artifact_count, 1);
        assert_eq!(
            std::fs::read_to_string(a.join("results/counts.csv")).unwrap(),
            "sample,count\nA,12"
        );
        let imported_b = apply_prepared_archive(&store, "b", &b, "fake", path, changed)
            .await
            .unwrap();
        assert_eq!(imported_b.status, "imported");
        assert_ne!(imported_a.frame_id, imported_b.frame_id);
        assert_eq!(
            store
                .frame_project_id(&imported_b.frame_id)
                .await
                .unwrap()
                .as_deref(),
            Some("b")
        );
        assert_eq!(
            std::fs::read_to_string(b.join("results/counts.csv")).unwrap(),
            "CHANGED"
        );
        let again = apply_prepared_archive(
            &store,
            "a",
            &a,
            "fake",
            path,
            prepare_archive(path).await.unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(again.frame_id, imported_a.frame_id);
        assert_eq!(again.status, "skipped");
        assert_eq!(again.artifact_count, 0);
        assert_eq!(store.list_project_frame_ids("a").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn native_archive_legacy_mapping_never_updates_another_project() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        for project in ["a", "b"] {
            store.create_project(project, project, "").await.unwrap();
        }
        store
            .create_frame("legacy", "a", "OPERON", "fake")
            .await
            .unwrap();
        store
            .append_message("legacy", 1, &Message::user("Original legacy question"))
            .await
            .unwrap();
        store
            .record_session_import("source", "legacy", "old.zip")
            .await
            .unwrap();
        let archive = build_archive(
            dir.path(),
            "source",
            &[
                Message::user("Imported question"),
                Message::assistant("Imported answer"),
            ],
            &[],
        );
        let parsed = parse_import_archive(&archive).unwrap();
        let (other, status) = import_parsed(&store, "b", "fake", "new.zip", &parsed)
            .await
            .unwrap();
        assert_eq!(status, "imported");
        assert_ne!(other, "legacy");
        assert_eq!(store.message_count("legacy").await.unwrap(), 1);
        assert_eq!(
            existing_import(&store, "a", "source")
                .await
                .unwrap()
                .as_deref(),
            Some("legacy")
        );
        let (same, status) = import_parsed(&store, "a", "fake", "new.zip", &parsed)
            .await
            .unwrap();
        assert_eq!(same, "legacy");
        assert_eq!(status, "updated");
        assert_eq!(store.message_count(&other).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn native_archive_reports_artifact_failures_without_overwriting_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("blocked"), "keep").unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        store
            .create_project("p", "P", root.to_str().unwrap())
            .await
            .unwrap();
        let archive = build_archive(
            dir.path(),
            "source",
            &[Message::user("Question")],
            &[("artifacts/file.txt", "blocked/file.txt", "content")],
        );
        let result = apply_prepared_archive(
            &store,
            "p",
            &root,
            "fake",
            archive.to_str().unwrap(),
            prepare_archive(archive.to_str().unwrap()).await.unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(result.status, "imported");
        assert_eq!(result.artifact_count, 0);
        assert_eq!(result.missing_artifacts.len(), 1);
        assert_eq!(
            std::fs::read_to_string(root.join("blocked")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn native_archive_corrupt_artifact_does_not_leave_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let archive = build_archive(
            dir.path(),
            "corrupt",
            &[Message::user("Question")],
            &[(
                "artifacts/data.txt",
                "results/data.txt",
                "synthetic content for CRC validation",
            )],
        );
        let mut bytes = std::fs::read(&archive).unwrap();
        let offset = {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let entry = zip.by_name("artifacts/data.txt").unwrap();
            entry.data_start() + entry.compressed_size() / 2
        };
        bytes[offset as usize] ^= 0xff;
        let parsed = parse_import_bytes(&bytes).unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let (files, missing) = extract_artifacts_from(
            std::io::Cursor::new(bytes),
            &parsed.artifacts,
            &root,
            "corrupt",
        );
        assert!(files.is_empty());
        assert_eq!(missing.len(), 1);
        assert!(!root.join("results/data.txt").exists());
    }

    fn build_archive(
        dir: &Path,
        session_id: &str,
        messages: &[Message],
        artifacts: &[(&str, &str, &str)], // (zip_path, workspace_path, contents)
    ) -> PathBuf {
        let manifest = serde_json::json!({
            "session_id": session_id,
            "exported_at": "2026-08-01T00:00:00Z",
            "message_count": messages.len(),
            "tool_call_count": 0,
            "artifacts": artifacts.iter().map(|(zip_path, workspace_path, contents)| {
                serde_json::json!({
                    "source_path": workspace_path,
                    "workspace_path": workspace_path,
                    "zip_path": zip_path,
                    "mime": "text/plain",
                    "bytes": contents.len(),
                    "provenance_path": null,
                })
            }).collect::<Vec<_>>(),
            "missing_artifacts": [],
        });
        let path = dir.join(format!("wisp-session-{}.zip", zip_component(session_id)));
        let out = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(out);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", opts).unwrap();
        std::io::Write::write_all(&mut zip, manifest.to_string().as_bytes()).unwrap();
        zip.start_file("messages.json", opts).unwrap();
        std::io::Write::write_all(
            &mut zip,
            serde_json::to_string_pretty(messages).unwrap().as_bytes(),
        )
        .unwrap();
        for (zip_path, _, contents) in artifacts {
            zip.start_file(zip_path, opts).unwrap();
            std::io::Write::write_all(&mut zip, contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wisp_import_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_round_trips_messages_and_artifacts() {
        let dir = test_dir("parse");
        let messages = vec![Message::user("hello"), Message::assistant("hi")];
        let archive = build_archive(
            &dir,
            "s1",
            &messages,
            &[("artifacts/001-data.txt", "results/data.txt", "stream me")],
        );

        let parsed = parse_import_archive(&archive).unwrap();
        assert_eq!(parsed.session_id, "s1");
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].content.as_text(), "hello");
        assert_eq!(parsed.artifacts.len(), 1);
        assert_eq!(parsed.artifacts[0].workspace_path, "results/data.txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_rejects_non_export_zip() {
        let dir = test_dir("reject");
        let path = dir.join("random.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"nope").unwrap();
        zip.finish().unwrap();

        let err = match parse_import_archive(&path) {
            Ok(_) => panic!("non-export zip must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("not a wisp session export"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_rejects_traversing_artifact_paths() {
        let dir = test_dir("traversal");
        let messages = vec![Message::user("hi")];
        let path = dir.join("evil.zip");
        let manifest = serde_json::json!({
            "session_id": "evil",
            "artifacts": [{"workspace_path": "x", "zip_path": "artifacts/../escape", "mime": "", "bytes": 0}],
        });
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", opts).unwrap();
        std::io::Write::write_all(&mut zip, manifest.to_string().as_bytes()).unwrap();
        zip.start_file("messages.json", opts).unwrap();
        std::io::Write::write_all(
            &mut zip,
            serde_json::to_string(&messages).unwrap().as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();

        assert!(parse_import_archive(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_target_falls_back_on_collision_and_rejects_absolute() {
        let root = test_dir("target");
        std::fs::create_dir_all(root.join("results")).unwrap();
        std::fs::write(root.join("results/data.txt"), b"existing").unwrap();

        // Free relative path → used as-is.
        let free = artifact_target(&root, "output/new.txt", "s1").unwrap();
        assert_eq!(free, root.join("output/new.txt"));
        // Collision → imports/<session>/ fallback.
        let fallback = artifact_target(&root, "results/data.txt", "s1").unwrap();
        assert_eq!(fallback, root.join("imports/s1/data.txt"));
        // Foreign absolute path → never the absolute location.
        let abs = artifact_target(&root, "/etc/hostname", "s1").unwrap();
        assert!(abs.starts_with(&root));
        // Traversal → validation fails, fallback still stays under root.
        let trav = artifact_target(&root, "../escape.txt", "s1").unwrap();
        assert!(trav.starts_with(&root));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn import_parsed_imports_skips_and_fast_forwards() {
        let db = std::env::temp_dir().join(format!("wisp_import_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&db).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();

        let dir = test_dir("store");
        let two = vec![Message::user("one"), Message::assistant("two")];
        let archive = build_archive(&dir, "s1", &two, &[]);
        let parsed = parse_import_archive(&archive).unwrap();

        let (frame_id, status) = import_parsed(&store, "p", "m", "archive.zip", &parsed)
            .await
            .unwrap();
        assert_eq!(status, "imported");
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 2);

        // Same archive again → skipped, same frame.
        let (again, status) = import_parsed(&store, "p", "m", "archive.zip", &parsed)
            .await
            .unwrap();
        assert_eq!(status, "skipped");
        assert_eq!(again, frame_id);

        // Longer archive for the same source session → fast-forward update.
        let mut three = two.clone();
        three.push(Message::user("three"));
        let archive3 = build_archive(&dir, "s1", &three, &[]);
        let parsed3 = parse_import_archive(&archive3).unwrap();
        let (updated, status) = import_parsed(&store, "p", "m", "archive.zip", &parsed3)
            .await
            .unwrap();
        assert_eq!(status, "updated");
        assert_eq!(updated, frame_id);
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 3);

        drop(store);
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn extract_artifacts_writes_into_workspace() {
        let root = test_dir("extract");
        let dir = test_dir("extract_zip");
        let messages = vec![Message::user("hi")];
        let archive = build_archive(
            &dir,
            "s1",
            &messages,
            &[("artifacts/001-data.txt", "results/data.txt", "stream me")],
        );
        let parsed = parse_import_archive(&archive).unwrap();

        let (extracted, missing) = extract_artifacts_from(
            std::fs::File::open(&archive).unwrap(),
            &parsed.artifacts,
            &root,
            "s1",
        );
        assert!(missing.is_empty());
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].1, root.join("results/data.txt"));
        assert_eq!(
            std::fs::read_to_string(root.join("results/data.txt")).unwrap(),
            "stream me"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
