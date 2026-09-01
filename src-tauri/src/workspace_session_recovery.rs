//! Best-effort recovery of conversations from an orphaned workspace.
//!
//! `.wisp/history/*.json` files are compaction snapshots or Exploration context
//! checkpoints, not a complete project database. Recovery therefore imports
//! only their message timelines into newly generated frames and keeps the source
//! files untouched.

use super::{models, project_commands, AppState};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_dto::{WorkspaceSessionRecoveryPreview, WorkspaceSessionRecoveryResult};
use wisp_llm::{Message, Role};
use wisp_store::{RecoveredWorkspaceSession, Store};

const HISTORY_RELATIVE: &str = ".wisp/history";
const MAX_ARCHIVE_FILES: usize = 1_000;
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TOTAL_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_RECOVERED_MESSAGES: usize = 100_000;

#[derive(Deserialize)]
struct ContextArchive {
    schema_version: u32,
    frame_id: String,
    message_head: i64,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HistoryArchive {
    Versioned(ContextArchive),
    MessageArray(Vec<Message>),
}

struct RecoveryCandidate {
    source_frame_id: String,
    source_path: String,
    message_head: i64,
    messages: Vec<Message>,
    created_at: i64,
    updated_at: i64,
}

struct RecoveryScan {
    preview: WorkspaceSessionRecoveryPreview,
    manifest_project_id: Option<String>,
    sessions: Vec<RecoveryCandidate>,
}

fn coded(code: &str, message: impl AsRef<str>) -> String {
    format!("{code}: {}", message.as_ref())
}

fn manifest_string(root: &Path, key: &str) -> Option<String> {
    let bytes = std::fs::read(root.join(".wisp/project.toml")).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    text.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        serde_json::from_str::<String>(value.trim())
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

fn workspace_name(root: &Path) -> String {
    manifest_string(root, "name")
        .or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Recovered project".into())
}

fn message_times(messages: &[Message], fallback: i64) -> (i64, i64) {
    let earliest = messages
        .iter()
        .map(|message| message.ts)
        .filter(|timestamp| *timestamp > 0)
        .min()
        .unwrap_or(fallback);
    let latest = messages
        .iter()
        .map(|message| message.ts)
        .filter(|timestamp| *timestamp > 0)
        .max()
        .unwrap_or(earliest);
    (earliest, latest)
}

fn modified_at(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp())
}

fn scan_workspace(root: &Path) -> Result<RecoveryScan, String> {
    let root = dunce::canonicalize(root).map_err(|error| {
        coded(
            "workspace_recovery_invalid_workspace",
            format!("Cannot open the selected workspace: {error}"),
        )
    })?;
    if !root.is_dir() {
        return Err(coded(
            "workspace_recovery_invalid_workspace",
            "The selected workspace is not a directory.",
        ));
    }
    let history = root.join(HISTORY_RELATIVE);
    let history_metadata = std::fs::symlink_metadata(&history).map_err(|error| {
        coded(
            "workspace_recovery_no_history",
            format!("No readable {HISTORY_RELATIVE} directory was found: {error}"),
        )
    })?;
    if history_metadata.file_type().is_symlink() || !history_metadata.is_dir() {
        return Err(coded(
            "workspace_recovery_no_history",
            format!("{HISTORY_RELATIVE} must be a real directory."),
        ));
    }

    let mut paths = std::fs::read_dir(&history)
        .map_err(|error| {
            coded(
                "workspace_recovery_no_history",
                format!("Cannot read {HISTORY_RELATIVE}: {error}"),
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_ARCHIVE_FILES {
        return Err(coded(
            "workspace_recovery_too_large",
            format!("The workspace has more than {MAX_ARCHIVE_FILES} history archives."),
        ));
    }

    let archive_count = paths.len();
    let mut valid_archive_count = 0usize;
    let mut invalid_archive_count = 0usize;
    let mut duplicate_archive_count = 0usize;
    let mut total_bytes = 0u64;
    let mut candidates = BTreeMap::<String, RecoveryCandidate>::new();

    for path in paths {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() > 0
                    && metadata.len() <= MAX_ARCHIVE_BYTES =>
            {
                metadata
            }
            _ => {
                invalid_archive_count += 1;
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_TOTAL_ARCHIVE_BYTES {
            return Err(coded(
                "workspace_recovery_too_large",
                "The history archives exceed the 200 MiB recovery limit.",
            ));
        }
        let archive = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HistoryArchive>(&bytes).ok());
        let Some(archive) = archive else {
            invalid_archive_count += 1;
            continue;
        };
        let (dedupe_key, source_frame_id, message_head, messages) = match archive {
            HistoryArchive::Versioned(archive)
                if archive.schema_version == 1
                    && archive.message_head > 0
                    && !archive.frame_id.trim().is_empty()
                    && archive.frame_id.len() <= 256 =>
            {
                (
                    format!("frame:{}", archive.frame_id),
                    archive.frame_id,
                    archive.message_head,
                    archive.messages,
                )
            }
            HistoryArchive::Versioned(_) => {
                invalid_archive_count += 1;
                continue;
            }
            HistoryArchive::MessageArray(messages) => {
                let encoded = match serde_json::to_vec(&messages) {
                    Ok(encoded) => encoded,
                    Err(_) => {
                        invalid_archive_count += 1;
                        continue;
                    }
                };
                let digest = hex::encode(Sha256::digest(encoded));
                (
                    format!("messages:{digest}"),
                    format!("message-sha256:{digest}"),
                    i64::try_from(messages.len()).unwrap_or(i64::MAX),
                    messages,
                )
            }
        };
        if messages.is_empty() || !messages.iter().any(|message| message.role == Role::User) {
            invalid_archive_count += 1;
            continue;
        }
        valid_archive_count += 1;
        let fallback = modified_at(&metadata);
        let (created_at, updated_at) = message_times(&messages, fallback);
        let source_path = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let candidate = RecoveryCandidate {
            source_frame_id,
            source_path,
            message_head,
            messages,
            created_at,
            updated_at,
        };
        match candidates.entry(dedupe_key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(candidate);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                duplicate_archive_count += 1;
                let current = entry.get();
                if (candidate.message_head, candidate.messages.len())
                    > (current.message_head, current.messages.len())
                {
                    entry.insert(candidate);
                }
            }
        }
    }

    let sessions = candidates.into_values().collect::<Vec<_>>();
    let message_count = sessions
        .iter()
        .map(|session| session.messages.len())
        .sum::<usize>();
    if message_count > MAX_RECOVERED_MESSAGES {
        return Err(coded(
            "workspace_recovery_too_large",
            format!("The recovery contains more than {MAX_RECOVERED_MESSAGES} messages."),
        ));
    }
    let earliest_message_at = sessions.iter().map(|session| session.created_at).min();
    let latest_message_at = sessions.iter().map(|session| session.updated_at).max();
    let preview = WorkspaceSessionRecoveryPreview {
        workspace_dir: root.to_string_lossy().into_owned(),
        suggested_name: workspace_name(&root),
        archive_count,
        valid_archive_count,
        recoverable_session_count: sessions.len(),
        message_count,
        invalid_archive_count,
        duplicate_archive_count,
        earliest_message_at,
        latest_message_at,
    };
    Ok(RecoveryScan {
        preview,
        manifest_project_id: manifest_string(&root, "project_id"),
        sessions,
    })
}

async fn ensure_workspace_is_unregistered(store: &Store, workspace: &Path) -> Result<(), String> {
    let registered = store
        .list_projects()
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .any(|project| {
            project_commands::same_workspace_path(workspace, Path::new(project.2.as_str()))
        });
    if registered {
        return Err(coded(
            "workspace_recovery_registered",
            "This folder is already registered as a project.",
        ));
    }
    Ok(())
}

fn probe_workspace_writable(root: &Path) -> Result<(), String> {
    let marker = root.join(format!(
        ".wisp-recovery-write-test-{}",
        uuid::Uuid::new_v4()
    ));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .map_err(|error| {
            coded(
                "workspace_recovery_not_writable",
                format!("The selected workspace is not writable: {error}"),
            )
        })?;
    drop(file);
    std::fs::remove_file(&marker).map_err(|error| {
        coded(
            "workspace_recovery_not_writable",
            format!("Could not remove the workspace write probe: {error}"),
        )
    })
}

async fn persist_scan(
    store: &Store,
    scan: RecoveryScan,
    project_name: &str,
    model_id: &str,
) -> Result<WorkspaceSessionRecoveryResult, String> {
    let project_name = project_name.trim();
    if project_name.is_empty() {
        return Err(coded(
            "workspace_recovery_name_required",
            "Project name is required.",
        ));
    }
    if scan.sessions.is_empty() {
        return Err(coded(
            "workspace_recovery_no_sessions",
            "No valid conversation archives were found.",
        ));
    }
    let project_id = match scan
        .manifest_project_id
        .filter(|project_id| !project_id.trim().is_empty())
    {
        Some(project_id)
            if store
                .get_project(&project_id)
                .await
                .map_err(|e| e.to_string())?
                .is_none() =>
        {
            project_id
        }
        _ => uuid::Uuid::new_v4().to_string(),
    };
    let recovered = scan
        .sessions
        .into_iter()
        .map(|session| RecoveredWorkspaceSession {
            source_session_id: format!(
                "workspace-history:{project_id}:{}",
                session.source_frame_id
            ),
            source_path: session.source_path,
            messages: session.messages,
            created_at: session.created_at,
            updated_at: session.updated_at,
        })
        .collect::<Vec<_>>();
    store
        .create_recovered_workspace_project(
            &project_id,
            project_name,
            &scan.preview.workspace_dir,
            model_id,
            &recovered,
        )
        .await
        .map_err(|error| {
            coded(
                "workspace_recovery_import_failed",
                format!("Could not import the recovered conversations: {error}"),
            )
        })?;
    Ok(WorkspaceSessionRecoveryResult {
        project_id,
        project_name: project_name.into(),
        recovered_session_count: recovered.len(),
        message_count: scan.preview.message_count,
        invalid_archive_count: scan.preview.invalid_archive_count,
        duplicate_archive_count: scan.preview.duplicate_archive_count,
    })
}

#[tauri::command]
pub(super) async fn preview_workspace_session_recovery(
    state: State<'_, AppState>,
    workspace_dir: String,
) -> Result<WorkspaceSessionRecoveryPreview, String> {
    let path = PathBuf::from(workspace_dir.trim());
    let scan = tokio::task::spawn_blocking(move || scan_workspace(&path))
        .await
        .map_err(|error| error.to_string())??;
    ensure_workspace_is_unregistered(&state.store, Path::new(&scan.preview.workspace_dir)).await?;
    Ok(scan.preview)
}

#[tauri::command]
pub(super) async fn recover_workspace_sessions(
    state: State<'_, AppState>,
    workspace_dir: String,
    name: String,
) -> Result<WorkspaceSessionRecoveryResult, String> {
    let path = PathBuf::from(workspace_dir.trim());
    let scan = tokio::task::spawn_blocking(move || scan_workspace(&path))
        .await
        .map_err(|error| error.to_string())??;
    let root = PathBuf::from(&scan.preview.workspace_dir);
    ensure_workspace_is_unregistered(&state.store, &root).await?;
    probe_workspace_writable(&root)?;
    let model_id = models::active_profile_id(&state.store).await;
    persist_scan(&state.store, scan, &name, &model_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wisp_workspace_recovery_{label}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(HISTORY_RELATIVE)).unwrap();
        root
    }

    fn message(role: Role, text: &str, ts: i64) -> Message {
        let mut message = match role {
            Role::User => Message::user(text),
            Role::Assistant => Message::assistant(text),
            Role::System => Message::system(text),
            Role::Tool => Message::tool("call", "read", text),
        };
        message.ts = ts;
        message
    }

    fn write_archive(
        root: &Path,
        name: &str,
        frame_id: &str,
        message_head: i64,
        messages: Vec<Message>,
    ) {
        let body = serde_json::json!({
            "schema_version": 1,
            "frame_id": frame_id,
            "message_head": message_head,
            "messages": messages,
        });
        std::fs::write(
            root.join(HISTORY_RELATIVE).join(name),
            serde_json::to_vec_pretty(&body).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn scan_deduplicates_frames_and_reports_damaged_archives() {
        let root = test_workspace("scan");
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(
            root.join(".wisp/project.toml"),
            "layout_version = 1\nproject_id = \"original\"\nname = \"Recovered Study\"\n",
        )
        .unwrap();
        write_archive(
            &root,
            "old.json",
            "frame-1",
            2,
            vec![message(Role::User, "old", 10)],
        );
        write_archive(
            &root,
            "new.json",
            "frame-1",
            4,
            vec![
                message(Role::User, "question", 20),
                message(Role::Assistant, "answer", 30),
            ],
        );
        write_archive(
            &root,
            "other.json",
            "frame-2",
            1,
            vec![message(Role::User, "other", 40)],
        );
        std::fs::write(root.join(HISTORY_RELATIVE).join("broken.json"), b"{").unwrap();

        let scan = scan_workspace(&root).unwrap();
        assert_eq!(scan.preview.suggested_name, "Recovered Study");
        assert_eq!(scan.preview.archive_count, 4);
        assert_eq!(scan.preview.valid_archive_count, 3);
        assert_eq!(scan.preview.recoverable_session_count, 2);
        assert_eq!(scan.preview.message_count, 3);
        assert_eq!(scan.preview.invalid_archive_count, 1);
        assert_eq!(scan.preview.duplicate_archive_count, 1);
        assert_eq!(scan.preview.earliest_message_at, Some(20));
        assert_eq!(scan.preview.latest_message_at, Some(40));
        assert_eq!(scan.manifest_project_id.as_deref(), Some("original"));
        assert_eq!(scan.sessions[0].message_head, 4);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_recovers_plain_message_arrays_and_deduplicates_exact_snapshots() {
        let root = test_workspace("message_arrays");
        let first = vec![
            message(Role::System, "rules", 0),
            message(Role::User, "question", 10),
            message(Role::Assistant, "answer", 20),
        ];
        let second = vec![message(Role::User, "another question", 30)];
        for (name, messages) in [
            ("session-100.json", first.clone()),
            ("session-101.json", first),
            ("session-200.json", second),
        ] {
            std::fs::write(
                root.join(HISTORY_RELATIVE).join(name),
                serde_json::to_vec_pretty(&messages).unwrap(),
            )
            .unwrap();
        }

        let scan = scan_workspace(&root).unwrap();
        assert_eq!(scan.preview.archive_count, 3);
        assert_eq!(scan.preview.valid_archive_count, 3);
        assert_eq!(scan.preview.recoverable_session_count, 2);
        assert_eq!(scan.preview.message_count, 4);
        assert_eq!(scan.preview.duplicate_archive_count, 1);
        assert_eq!(scan.preview.earliest_message_at, Some(10));
        assert_eq!(scan.preview.latest_message_at, Some(30));
        assert!(scan
            .sessions
            .iter()
            .all(|session| session.source_frame_id.starts_with("message-sha256:")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn writable_probe_closes_and_removes_its_windows_marker() {
        let root = test_workspace("writable");
        probe_workspace_writable(&root).unwrap();
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".wisp-recovery-write-test-")
        }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn empty_history_is_previewed_without_creating_a_recovery_candidate() {
        let root = test_workspace("empty");
        let scan = scan_workspace(&root).unwrap();
        assert_eq!(scan.preview.archive_count, 0);
        assert_eq!(scan.preview.recoverable_session_count, 0);
        assert_eq!(scan.preview.message_count, 0);
        assert_eq!(scan.preview.earliest_message_at, None);
        assert_eq!(scan.preview.latest_message_at, None);
        assert!(scan.sessions.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn persist_scan_registers_project_and_recovered_sessions() {
        let root = test_workspace("persist");
        write_archive(
            &root,
            "one.json",
            "source-frame",
            2,
            vec![
                message(Role::User, "question", 10),
                message(Role::Assistant, "answer", 20),
            ],
        );
        let scan = scan_workspace(&root).unwrap();
        let database = std::env::temp_dir().join(format!(
            "wisp_workspace_recovery_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&database).await.unwrap();

        let result = persist_scan(&store, scan, "Recovered", "model")
            .await
            .unwrap();
        assert_eq!(result.recovered_session_count, 1);
        assert_eq!(result.message_count, 2);
        assert_eq!(
            store.list_sessions(&result.project_id).await.unwrap().len(),
            1
        );

        drop(store);
        let _ = std::fs::remove_file(database);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn persist_scan_generates_a_new_id_when_the_manifest_id_is_registered() {
        let root = test_workspace("project_id_conflict");
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(
            root.join(".wisp/project.toml"),
            "layout_version = 1\nproject_id = \"original\"\nname = \"Recovered Study\"\n",
        )
        .unwrap();
        write_archive(
            &root,
            "one.json",
            "source-frame",
            1,
            vec![message(Role::User, "question", 10)],
        );
        let scan = scan_workspace(&root).unwrap();
        let database = std::env::temp_dir().join(format!(
            "wisp_workspace_recovery_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&database).await.unwrap();
        store
            .create_project("original", "Existing", "/different/workspace")
            .await
            .unwrap();

        let result = persist_scan(&store, scan, "Recovered", "model")
            .await
            .unwrap();
        assert_ne!(result.project_id, "original");
        assert_eq!(
            store.list_sessions(&result.project_id).await.unwrap().len(),
            1
        );
        assert_eq!(
            store.get_project("original").await.unwrap().unwrap().0,
            "Existing"
        );

        drop(store);
        let _ = std::fs::remove_file(database);
        let _ = std::fs::remove_dir_all(root);
    }
}
