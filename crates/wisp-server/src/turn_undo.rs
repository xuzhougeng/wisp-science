//! Undoing a turn's files: before a tool changes a text file the core
//! reports what it held, the host keeps that at the end of the turn, and
//! rewinding the latest turn can put the files back.
//!
//! The rows and the snapshot folder (`.wisp/undo/sha256/`) are the ones
//! the WebView desktop uses, following `src-tauri/src/turn_undo.rs`,
//! which cannot be shared from here because it lives in the Tauri shell.
//! A file that was changed again since the turn is left as it is and
//! named; a file the turn created goes to the project's trash, not away.

use std::path::Path;

use anyhow::{ensure, Result};
use wisp_core::provenance::ProvenanceRecord;
use wisp_store::Store;

use crate::{desktop::TurnRef, Server};

const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// What undoing a turn did to the project's files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnFiles {
    /// Put back as they were before the turn.
    pub restored: Vec<String>,
    /// Created by the turn; moved to the project's trash.
    pub removed: Vec<String>,
    /// Left alone, each with why.
    pub left: Vec<(String, String)>,
}

/// Keeps what the turn that started at `user_seq` changed, so it can be
/// undone. A file changed twice keeps its first state.
pub(crate) async fn keep(
    store: &Store,
    root: &Path,
    frame: &str,
    user_seq: i64,
    records: &[ProvenanceRecord],
) {
    let mut seen = std::collections::HashSet::new();
    for change in records.iter().flat_map(|record| &record.file_changes) {
        if !seen.insert(change.path.clone()) {
            // The row keeps the first preimage; only the result moves on.
            let _ = store
                .save_turn_file_undo(
                    frame,
                    user_seq,
                    &change.path,
                    change.before_exists,
                    None,
                    None,
                    change.after_checksum.as_deref(),
                    change.reversible,
                    change.reason.as_deref(),
                )
                .await;
            continue;
        }
        let snapshot = match (&change.before_text, &change.before_checksum) {
            (Some(text), Some(checksum)) if checksum.len() >= 2 => {
                let relative = format!(".wisp/undo/sha256/{}/{checksum}", &checksum[..2]);
                let path = root.join(&relative);
                let written = path.exists()
                    || path
                        .parent()
                        .map(std::fs::create_dir_all)
                        .transpose()
                        .and_then(|_| std::fs::write(&path, text.as_bytes()))
                        .is_ok();
                written.then_some(relative)
            }
            _ => None,
        };
        let reversible = change.reversible
            && (!change.before_exists || snapshot.is_some())
            && change.after_checksum.is_some();
        let reason = match reversible {
            true => None,
            false => change
                .reason
                .as_deref()
                .or(Some("undo snapshot could not be kept")),
        };
        if let Err(error) = store
            .save_turn_file_undo(
                frame,
                user_seq,
                &change.path,
                change.before_exists,
                snapshot.as_deref(),
                change.before_checksum.as_deref(),
                change.after_checksum.as_deref(),
                reversible,
                reason,
            )
            .await
        {
            tracing::warn!(%error, path = %change.path, "Could not keep a turn's undo row");
        }
    }
}

fn checksum(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err("file now exceeds the 10 MiB undo limit".into());
    }
    std::fs::read(path)
        .map(|bytes| wisp_sync::sha256_hex(&bytes))
        .map_err(|e| e.to_string())
}

/// Puts back what the turn that started at `user_seq` changed.
pub(crate) async fn restore(
    store: &Store,
    root: &Path,
    frame: &str,
    user_seq: i64,
) -> Result<TurnFiles> {
    let mut done = TurnFiles::default();
    let trash = root
        .join(".wisp/trash")
        .join(format!("undo-{}", uuid::Uuid::new_v4().simple()));
    for change in store.list_turn_file_undo(frame, user_seq).await? {
        let mut leave = |why: String| done.left.push((change.path.clone(), why));
        if !change.reversible {
            leave(
                change
                    .reason
                    .clone()
                    .unwrap_or_else(|| "cannot be undone".into()),
            );
            continue;
        }
        let real = match wisp_tools::safety::validate_file_path(root, &change.path) {
            Ok(real) => real,
            Err(problem) => {
                leave(problem);
                continue;
            }
        };
        let Some(after) = change.after_checksum.as_deref() else {
            leave("its state after the turn was not recorded".into());
            continue;
        };
        if !real.exists() {
            if change.before_exists {
                leave("was removed after this turn".into());
            }
            continue;
        }
        let current = match checksum(&real) {
            Ok(current) => current,
            Err(problem) => {
                leave(problem);
                continue;
            }
        };
        if change.before_checksum.as_deref() == Some(current.as_str()) {
            // Already as it was.
            continue;
        }
        if current != after {
            leave("was changed again after this turn".into());
            continue;
        }
        let applied = if change.before_exists {
            match &change.before_snapshot_path {
                Some(snapshot) => std::fs::read(root.join(snapshot))
                    .and_then(|bytes| std::fs::write(&real, bytes))
                    .map(|()| done.restored.push(change.path.clone())),
                None => Err(std::io::Error::other("its earlier content was not kept")),
            }
        } else {
            let target = trash.join(&change.path);
            target
                .parent()
                .map(std::fs::create_dir_all)
                .transpose()
                .and_then(|_| std::fs::rename(&real, &target))
                .map(|()| done.removed.push(change.path.clone()))
        };
        if let Err(error) = applied {
            done.left.push((change.path.clone(), error.to_string()));
        }
    }
    Ok(done)
}

impl Server {
    /// Rewinds the latest turn and puts back the files it changed: a file
    /// it edited gets its earlier content, a file it created goes to the
    /// project's trash, and a file changed again since is left and named.
    /// Returns the turn's message, to edit and send again, and what was
    /// done to the files.
    pub async fn undo_turn(
        &self,
        project: &str,
        session: &str,
        turn: &TurnRef,
    ) -> Result<(String, TurnFiles)> {
        ensure!(
            turn.from_end == 0,
            "Only the latest turn's files can be undone"
        );
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn can start meanwhile.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        let (_, root) = self
            .0
            .store
            .get_project(project)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Project not found"))?;
        // Everything that can refuse the rewind is checked before a file
        // is touched.
        let seq = self.rewind_target(session, turn).await?;
        let files = restore(&self.0.store, Path::new(&root), session, seq).await?;
        let text = self.rewind_locked(&live, session, turn).await?;
        Ok((text, files))
    }
}
