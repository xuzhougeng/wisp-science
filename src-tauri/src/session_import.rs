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
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use wisp_llm::Message;
use wisp_store::Store;

#[derive(serde::Deserialize)]
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

#[derive(serde::Serialize)]
pub(super) struct ImportSessionSummary {
    frame_id: String,
    status: String,
    message_count: usize,
    artifact_count: usize,
    missing_artifacts: Vec<String>,
}

fn read_zip_string<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, String> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| format!("not a wisp session export: {name} missing"))?;
    let mut body = String::new();
    entry
        .read_to_string(&mut body)
        .map_err(|e| format!("read {name}: {e}"))?;
    Ok(body)
}

/// Read and validate an export archive. CPU/IO-bound: call from spawn_blocking.
fn parse_import_archive(path: &Path) -> Result<ParsedImport, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open archive: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("open archive: {e}"))?;

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
fn extract_artifacts(
    archive_path: &Path,
    artifacts: &[ImportManifestArtifact],
    root: &Path,
    session_id: &str,
) -> (Vec<(String, PathBuf, String)>, Vec<String>) {
    let mut extracted = vec![];
    let mut missing = vec![];
    let file = match std::fs::File::open(archive_path) {
        Ok(file) => file,
        Err(e) => {
            missing.extend(
                artifacts
                    .iter()
                    .map(|a| format!("{}: {e}", a.workspace_path)),
            );
            return (extracted, missing);
        }
    };
    let mut zip = match zip::ZipArchive::new(file) {
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
    for artifact in artifacts {
        let result = (|| -> Result<PathBuf, String> {
            let target = artifact_target(root, &artifact.workspace_path, session_id)
                .ok_or_else(|| "target already exists".to_string())?;
            let mut entry = zip
                .by_name(&artifact.zip_path)
                .map_err(|e| format!("{e}"))?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
            }
            let mut out = std::fs::File::create(&target).map_err(|e| format!("{e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("{e}"))?;
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

/// Create or fast-forward the frame for a parsed archive. Returns the frame id
/// and the outcome ("imported" / "updated" / "skipped").
async fn import_parsed(
    store: &Store,
    project_id: &str,
    model_id: &str,
    source_path: &str,
    parsed: &ParsedImport,
) -> Result<(String, &'static str), String> {
    let (created_at, updated_at) = import_timestamps(parsed);

    if let Some(frame_id) = store
        .find_session_import(&parsed.session_id)
        .await
        .map_err(|e| e.to_string())?
    {
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
            .record_session_import(&parsed.session_id, &frame_id, source_path)
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
        .record_session_import(&parsed.session_id, &frame_id, source_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok((frame_id, "imported"))
}

/// Pick a session-export zip and import it into the active project. Returns
/// the import summary, or `None` if the user cancelled the dialog.
#[tauri::command]
pub(super) async fn import_session_archive(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Option<ImportSessionSummary>, String> {
    use tauri_plugin_dialog::DialogExt;

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
    let archive_path = PathBuf::from(picked.to_string());

    let parsed = {
        let path = archive_path.clone();
        tokio::task::spawn_blocking(move || parse_import_archive(&path))
            .await
            .map_err(|e| format!("{e}"))??
    };

    let ap = state.require_active(window.label())?;
    let model_id = models::active_profile_id(&state.store).await;
    let source_path = archive_path.to_string_lossy().into_owned();
    let (frame_id, status) =
        import_parsed(&state.store, &ap.id, &model_id, &source_path, &parsed).await?;

    // Artifacts are restored on first import only; a fast-forward update keeps
    // the files and artifact rows registered by the initial import.
    let (artifact_count, missing_artifacts) =
        if status == "imported" && !parsed.artifacts.is_empty() {
            let (extracted, missing) = {
                let path = archive_path.clone();
                let root = ap.root.clone();
                let session_id = parsed.session_id.clone();
                let artifacts = parsed
                    .artifacts
                    .iter()
                    .map(|a| ImportManifestArtifact {
                        workspace_path: a.workspace_path.clone(),
                        zip_path: a.zip_path.clone(),
                        mime: a.mime.clone(),
                    })
                    .collect::<Vec<_>>();
                tokio::task::spawn_blocking(move || {
                    extract_artifacts(&path, &artifacts, &root, &session_id)
                })
                .await
                .map_err(|e| format!("{e}"))?
            };
            let mut count = 0usize;
            for (_, target, mime) in extracted {
                let filename = target
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                let storage_path = to_workspace_rel(&ap.root, &target.to_string_lossy());
                if state
                    .store
                    .save_artifact(
                        &uuid::Uuid::new_v4().to_string(),
                        &ap.id,
                        &frame_id,
                        &filename,
                        &mime,
                        &storage_path,
                    )
                    .await
                    .is_ok()
                {
                    count += 1;
                }
            }
            (count, missing)
        } else {
            (0, vec![])
        };

    Ok(Some(ImportSessionSummary {
        frame_id,
        status: status.into(),
        message_count: parsed.messages.len(),
        artifact_count,
        missing_artifacts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let (extracted, missing) = extract_artifacts(&archive, &parsed.artifacts, &root, "s1");
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
