//! Tauri/service layer for checkpointing and opening persistent exploration
//! branches. The service is UI-independent so failure and restart behavior can
//! be exercised with a temporary Store and filesystem.

use crate::exploration_workspace::{
    ExplorationWorkspaceBackend, PersistentExplorationWorkspace, WorkspaceSnapshot,
};
use crate::workspace_surface::WorkspaceSurface;
use crate::{load_skill_index, ActiveProject, AppState, MemoryManager};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use wisp_store::{
    ArtifactHead, ContextArchiveRecord, Exploration, ExplorationBaselineArtifactHead,
    ExplorationBaselineEntity, ExplorationCheckpoint, ExplorationFamily, ExplorationStatus,
    ExplorationSummary, StateScope, Store, WorkspaceSnapshotRecord, AGENT_WORKFLOW_COMPLETION_TOOL,
    MAINLINE_SCOPE_KEY,
};

const ERR_SOURCE_BUSY: &str = "exploration_source_busy";
const ERR_ACP_UNSUPPORTED: &str = "exploration_acp_unsupported";
const ERR_SOURCE_INCOMPLETE: &str = "exploration_source_incomplete";
const ERR_ACTIVE_RUN: &str = "exploration_active_run";
const ERR_HISTORY_UNAVAILABLE: &str = "exploration_history_unavailable";
const ERR_NOT_WRITABLE: &str = "exploration_not_writable";
const ERR_ROUND_ACTIVE: &str = "exploration_round_active";
const ERR_MAINLINE_FROZEN: &str = "exploration_mainline_frozen";
const ERR_BRANCH_UNSUPPORTED: &str = "exploration_branch_unsupported";

fn coded_error(code: &str, message: impl AsRef<str>) -> String {
    format!("{code}: {}", message.as_ref())
}

/// Native Wisp renders a successful `attempt_completion` result as the final
/// assistant bubble. Persisted history keeps that result as a Tool message and
/// may append synthetic results for skipped sibling calls after it. Validate
/// the latest user turn using the same visible completion semantics instead of
/// requiring the physical tail row to be Assistant.
fn counts_as_user_turn(message: &wisp_llm::Message) -> bool {
    message.role == wisp_llm::Role::User
        && message.tool_name.as_deref() != Some(AGENT_WORKFLOW_COMPLETION_TOOL)
        && !message.content.as_text().trim().is_empty()
        && !wisp_store::is_compaction_checkpoint(&message.content.as_text())
}

fn latest_native_turn_is_complete(messages: &[wisp_llm::Message]) -> bool {
    let Some(turn_start) = messages.iter().rposition(|message| {
        message.role == wisp_llm::Role::User
            && message.tool_name.as_deref() != Some(AGENT_WORKFLOW_COMPLETION_TOOL)
            && !message.content.as_text().trim().is_empty()
    }) else {
        return false;
    };
    messages[turn_start + 1..].iter().any(|message| {
        (message.role == wisp_llm::Role::Assistant && message.tool_calls.is_empty())
            || (message.role == wisp_llm::Role::Tool
                && message.tool_name.as_deref() == Some("attempt_completion")
                && !message.content.as_text().trim().is_empty())
    })
}

#[derive(Clone)]
pub(crate) struct ExplorationService {
    store: Store,
    app_data: PathBuf,
}

struct CheckpointSource {
    message_head: i64,
    ui_event_head: i64,
    state_generation: i64,
    snapshot: WorkspaceSnapshot,
    context_archive_id: String,
    artifact_heads: Vec<ArtifactHead>,
    entities: Vec<ExplorationBaselineEntity>,
    messages: Vec<wisp_llm::Message>,
}

impl ExplorationService {
    pub(crate) fn new(store: Store, app_data: PathBuf) -> Self {
        Self { store, app_data }
    }

    fn workspace_backend(&self) -> PersistentExplorationWorkspace {
        PersistentExplorationWorkspace::new(self.app_data.clone())
    }

    #[cfg(test)]
    pub(crate) async fn create_checkpoint(
        &self,
        project_id: &str,
        source_frame_id: &str,
    ) -> Result<ExplorationCheckpoint, String> {
        self.create_checkpoint_at(project_id, source_frame_id, None)
            .await
    }

    pub(crate) async fn create_checkpoint_at(
        &self,
        project_id: &str,
        source_frame_id: &str,
        turn_index: Option<i64>,
    ) -> Result<ExplorationCheckpoint, String> {
        let scope = self
            .store
            .frame_state_scope(source_frame_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded_error(ERR_HISTORY_UNAVAILABLE, "source conversation not found"))?;
        if scope.project_id() != project_id || !matches!(scope, StateScope::Mainline { .. }) {
            return Err(coded_error(
                ERR_HISTORY_UNAVAILABLE,
                "checkpoints must be created from the current mainline conversation",
            ));
        }
        if self
            .store
            .session_branch_state(source_frame_id)
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err(coded_error(
                ERR_BRANCH_UNSUPPORTED,
                "Conversation branches cannot start an exploration.",
            ));
        }
        if self
            .store
            .get_acp_session(source_frame_id)
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err(coded_error(
                ERR_ACP_UNSUPPORTED,
                "ACP conversations cannot be checkpointed in the MVP",
            ));
        }
        if self
            .store
            .project_has_active_runs(project_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(coded_error(
                ERR_ACTIVE_RUN,
                "finish or cancel active mainline Runs before checkpointing",
            ));
        }
        let (message_seqs, current_messages): (Vec<_>, Vec<_>) = self
            .store
            .load_messages_with_seq(source_frame_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .unzip();
        let current_message_head = self
            .store
            .frame_message_head(source_frame_id)
            .await
            .map_err(|error| error.to_string())?;
        let current_ui_event_head = self
            .store
            .frame_ui_event_head(source_frame_id)
            .await
            .map_err(|error| error.to_string())?;
        let visual_turn_count = self
            .store
            .visual_user_count(source_frame_id)
            .await
            .map_err(|error| error.to_string())? as i64;
        let fallback_turn_count = current_messages
            .iter()
            .filter(|message| counts_as_user_turn(message))
            .count() as i64;
        let current_turn_index = visual_turn_count
            .max(fallback_turn_count)
            .checked_sub(1)
            .ok_or_else(|| {
                coded_error(
                    ERR_HISTORY_UNAVAILABLE,
                    "the source has no stable completed turn boundary",
                )
            })?;
        let selected_turn_index = turn_index.unwrap_or(current_turn_index);
        if selected_turn_index < 0 || selected_turn_index > current_turn_index {
            return Err(coded_error(
                ERR_HISTORY_UNAVAILABLE,
                "the selected turn is outside the available conversation history",
            ));
        }
        let historical = selected_turn_index != current_turn_index;
        // Older conversations can have a model-message prefix from before UI
        // events were persisted. Those turns have no visual anchor.
        let legacy_turn_count = (fallback_turn_count - visual_turn_count).max(0);
        let (selected_message_head, selected_messages, selected_ui_event_head) = if !historical {
            let head = message_seqs.last().copied().unwrap_or(current_message_head);
            (head, current_messages, current_ui_event_head)
        } else if selected_turn_index < legacy_turn_count {
            let rows = self
                .store
                .load_messages_in_epoch(source_frame_id, 0)
                .await
                .map_err(|error| error.to_string())?;
            let user_positions = rows
                .iter()
                .enumerate()
                .filter(|(_, (_, message))| counts_as_user_turn(message))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let prefix_len = user_positions
                .get(selected_turn_index as usize + 1)
                .copied()
                .ok_or_else(|| {
                    coded_error(
                        ERR_HISTORY_UNAVAILABLE,
                        "the selected turn boundary is unavailable",
                    )
                })?;
            let selected_message_head =
                rows.get(prefix_len - 1)
                    .map(|(seq, _)| *seq)
                    .ok_or_else(|| {
                        coded_error(
                            ERR_HISTORY_UNAVAILABLE,
                            "the selected turn boundary is unavailable",
                        )
                    })?;
            let selected_messages = rows
                .into_iter()
                .take(prefix_len)
                .map(|(_, message)| message)
                .collect();
            (selected_message_head, selected_messages, 0)
        } else {
            let visual_index = usize::try_from(selected_turn_index - legacy_turn_count)
                .map_err(|error| error.to_string())?;
            let (keep_seq, ui_seq) = self
                .store
                .visual_turn_end(source_frame_id, visual_index)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    coded_error(
                        ERR_HISTORY_UNAVAILABLE,
                        "the selected turn has no completed visual boundary",
                    )
                })?;
            let epoch = self
                .store
                .resolve_message_epoch(source_frame_id, keep_seq)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    coded_error(
                        ERR_HISTORY_UNAVAILABLE,
                        "the selected turn boundary is unavailable",
                    )
                })?;
            let selected_messages = self
                .store
                .load_messages_in_epoch(source_frame_id, epoch)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|(seq, _)| *seq <= keep_seq)
                .map(|(_, message)| message)
                .collect::<Vec<_>>();
            (keep_seq, selected_messages, ui_seq)
        };
        if !latest_native_turn_is_complete(&selected_messages) {
            return Err(coded_error(
                ERR_SOURCE_INCOMPLETE,
                "the selected turn has no completed assistant response",
            ));
        }
        let (_, workspace_dir) = self
            .store
            .get_project(project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Project not found".to_string())?;
        let project_root = dunce::canonicalize(&workspace_dir)
            .map_err(|error| format!("cannot resolve project workspace: {error}"))?;

        let now = chrono::Utc::now().timestamp();
        let family = match self
            .store
            .exploration_family_for_mainline(project_id, source_frame_id)
            .await
            .map_err(|error| error.to_string())?
        {
            Some(family) => family,
            None => {
                let family = ExplorationFamily {
                    id: uuid::Uuid::new_v4().to_string(),
                    project_id: project_id.to_string(),
                    root_frame_id: source_frame_id.to_string(),
                    mainline_frame_id: source_frame_id.to_string(),
                    generation: 0,
                    created_at: now,
                    updated_at: now,
                };
                match self.store.create_exploration_family(&family).await {
                    Ok(()) => family,
                    Err(_) => self
                        .store
                        .exploration_family_for_mainline(project_id, source_frame_id)
                        .await
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| "failed to create exploration family".to_string())?,
                }
            }
        };
        if let Some(existing) = self
            .store
            .current_exploration_checkpoint_for_source(
                project_id,
                source_frame_id,
                &family.id,
                family.generation,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            // The first candidate freezes the entire round. Later candidates
            // must clone that exact checkpoint even if an external process
            // changed the live workspace, because Wisp cannot redefine an
            // already-open round's baseline.
            if existing.source_message_seq != selected_message_head
                || existing.source_ui_event_seq != selected_ui_event_head
            {
                return Err(coded_error(
                    ERR_ROUND_ACTIVE,
                    "finish the current exploration round before starting from another response",
                ));
            }
            return Ok(existing);
        }
        // Exploration V2 snapshots the live current head only when the user
        // explicitly starts an exploration. Ordinary completed turns no
        // longer pay the workspace snapshot cost or create hidden history.
        let state_generation = self
            .store
            .project_state_generation(project_id)
            .await
            .map_err(|error| error.to_string())?;
        let snapshot = self.workspace_backend().checkpoint(&project_root).await?;
        self.store
            .create_workspace_snapshot(&WorkspaceSnapshotRecord {
                id: snapshot.id.clone(),
                project_id: project_id.to_string(),
                manifest_json: serde_json::to_string(&snapshot)
                    .map_err(|error| error.to_string())?,
                manifest_sha256: snapshot.manifest_sha256.clone(),
                created_at: snapshot.created_at,
            })
            .await
            .map_err(|error| error.to_string())?;
        let archive_id = uuid::Uuid::new_v4().to_string();
        let archive_relative =
            PathBuf::from("exploration-contexts").join(format!("{archive_id}.json"));
        let archive_path = self.app_data.join(&archive_relative);
        write_context_archive(
            &self.app_data,
            &archive_path,
            source_frame_id,
            selected_message_head,
            &selected_messages,
        )?;
        let archive_bytes = std::fs::read(&archive_path).map_err(|error| error.to_string())?;
        self.store
            .create_context_archive(&ContextArchiveRecord {
                id: archive_id.clone(),
                project_id: project_id.to_string(),
                frame_id: source_frame_id.to_string(),
                storage_path: archive_relative.to_string_lossy().replace('\\', "/"),
                checksum: hex::encode(Sha256::digest(&archive_bytes)),
                created_at: now,
            })
            .await
            .map_err(|error| error.to_string())?;
        let source = CheckpointSource {
            message_head: selected_message_head,
            ui_event_head: selected_ui_event_head,
            state_generation,
            snapshot,
            context_archive_id: archive_id,
            artifact_heads: self
                .store
                .list_artifact_heads(project_id, MAINLINE_SCOPE_KEY)
                .await
                .map_err(|error| error.to_string())?,
            entities: self
                .store
                .snapshot_mainline_entities(project_id)
                .await
                .map_err(|error| error.to_string())?,
            messages: selected_messages,
        };
        if !latest_native_turn_is_complete(&source.messages) {
            return Err(coded_error(
                ERR_SOURCE_INCOMPLETE,
                "the selected revision does not end at a completed assistant turn",
            ));
        }
        let entity_hash = hash_json(&serde_json::json!({
            "artifact_heads": &source.artifact_heads,
            "entities": &source.entities,
        }))?;
        let guard_hash = hash_json(&serde_json::json!({
            "family_id": family.id,
            "family_generation": family.generation,
            "mainline_frame_id": family.mainline_frame_id,
            "source_frame_id": source_frame_id,
            "source_message_head": source.message_head,
            "source_frame_head": current_message_head,
            "source_ui_event_head": current_ui_event_head,
            "state_generation": source.state_generation,
            "workspace_manifest": source.snapshot.manifest_sha256,
            "artifact_heads": &source.artifact_heads,
            "entities": &source.entities,
        }))?;
        let checkpoint = ExplorationCheckpoint {
            id: uuid::Uuid::new_v4().to_string(),
            family_id: family.id,
            project_id: project_id.to_string(),
            source_frame_id: source_frame_id.to_string(),
            source_message_seq: source.message_head,
            source_frame_head_seq: current_message_head,
            source_ui_event_seq: source.ui_event_head,
            source_ui_event_head_seq: current_ui_event_head,
            source_family_generation: family.generation,
            source_state_generation: source.state_generation,
            workspace_snapshot_id: source.snapshot.id.clone(),
            context_archive_id: source.context_archive_id,
            guard_hash,
            entity_hash,
            isolation_summary_json: serde_json::json!({
                "warnings": source.snapshot.warnings,
                "entry_count": source.snapshot.entries.len(),
                "fully_isolated": source.snapshot.entries.iter().all(|entry| entry.recoverable),
            })
            .to_string(),
            created_at: now,
        };
        if let Some(existing) = self
            .store
            .get_exploration_checkpoint_by_guard(
                &checkpoint.family_id,
                &checkpoint.source_frame_id,
                checkpoint.source_message_seq,
                &checkpoint.guard_hash,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(existing);
        }
        if let Err(error) = self.store.create_exploration_checkpoint(&checkpoint).await {
            if let Some(existing) = self
                .store
                .get_exploration_checkpoint_by_guard(
                    &checkpoint.family_id,
                    &checkpoint.source_frame_id,
                    checkpoint.source_message_seq,
                    &checkpoint.guard_hash,
                )
                .await
                .map_err(|lookup_error| lookup_error.to_string())?
            {
                return Ok(existing);
            }
            return Err(error.to_string());
        }
        for head in source.artifact_heads {
            self.store
                .record_exploration_baseline_artifact_head(&ExplorationBaselineArtifactHead {
                    checkpoint_id: checkpoint.id.clone(),
                    logical_key: head.logical_key,
                    artifact_id: head.artifact_id.clone(),
                    artifact_version_id: head.artifact_version_id.clone(),
                    fingerprint: hash_json(&serde_json::json!({
                        "artifact_id": head.artifact_id,
                        "artifact_version_id": head.artifact_version_id,
                    }))?,
                })
                .await
                .map_err(|error| error.to_string())?;
        }
        for mut entity in source.entities {
            entity.checkpoint_id = checkpoint.id.clone();
            self.store
                .record_exploration_baseline_entity(&entity)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(checkpoint)
    }

    pub(crate) async fn create_exploration(
        &self,
        checkpoint_id: &str,
        name: &str,
    ) -> Result<Exploration, String> {
        let checkpoint = self
            .store
            .get_exploration_checkpoint(checkpoint_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded_error(ERR_HISTORY_UNAVAILABLE, "checkpoint not found"))?;
        let (_, source_workspace) = self
            .store
            .get_project(&checkpoint.project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Project not found".to_string())?;
        let source_workspace =
            dunce::canonicalize(source_workspace).map_err(|error| error.to_string())?;
        if self
            .store
            .get_acp_session(&checkpoint.source_frame_id)
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err(coded_error(
                ERR_ACP_UNSUPPORTED,
                "ACP conversations cannot create explorations in the MVP",
            ));
        }
        let snapshot_record = self
            .store
            .get_workspace_snapshot_record(&checkpoint.workspace_snapshot_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded_error(ERR_HISTORY_UNAVAILABLE, "workspace snapshot not found"))?;
        let snapshot: WorkspaceSnapshot = serde_json::from_str(&snapshot_record.manifest_json)
            .map_err(|error| format!("invalid workspace snapshot manifest: {error}"))?;
        let persisted_snapshot = self
            .workspace_backend()
            .load_snapshot(&checkpoint.workspace_snapshot_id)?;
        if persisted_snapshot != snapshot
            || persisted_snapshot.manifest_sha256 != snapshot_record.manifest_sha256
        {
            return Err(coded_error(
                ERR_HISTORY_UNAVAILABLE,
                "workspace snapshot record does not match persistent storage",
            ));
        }

        let exploration_id = uuid::Uuid::new_v4().to_string();
        let frame_id = uuid::Uuid::new_v4().to_string();
        let backend = self.workspace_backend();
        let workspace = backend
            .materialize(&persisted_snapshot, &exploration_id)
            .await?;
        if let Err(error) = materialize_checkpoint_context_archive(
            &self.store,
            &self.app_data,
            &checkpoint,
            &source_workspace,
            &workspace.root,
        )
        .await
        {
            let _ = backend.dispose(&workspace).await;
            return Err(coded_error(ERR_HISTORY_UNAVAILABLE, error));
        }
        if let Err(error) = self
            .store
            .clone_exploration_frame(
                &checkpoint.source_frame_id,
                &frame_id,
                checkpoint.source_message_seq,
                checkpoint.source_ui_event_seq,
            )
            .await
        {
            let _ = backend.dispose(&workspace).await;
            return Err(coded_error(ERR_HISTORY_UNAVAILABLE, error.to_string()));
        }
        if let Err(error) = self
            .store
            .rewrite_cloned_context_archive_references(&frame_id, &source_workspace)
            .await
        {
            let _ = self
                .store
                .delete_session(&frame_id, &checkpoint.project_id)
                .await;
            let _ = backend.dispose(&workspace).await;
            return Err(coded_error(ERR_HISTORY_UNAVAILABLE, error.to_string()));
        }

        let now = chrono::Utc::now().timestamp();
        let exploration = Exploration {
            id: exploration_id,
            checkpoint_id: checkpoint.id,
            frame_id: frame_id.clone(),
            name: normalize_name(name),
            status: ExplorationStatus::Creating,
            workspace_dir: workspace.root.to_string_lossy().into_owned(),
            workspace_backend: "persistent_snapshot_v1".into(),
            scope_generation: 0,
            warnings_json: serde_json::to_string(&persisted_snapshot.warnings)
                .map_err(|error| error.to_string())?,
            created_at: now,
            updated_at: now,
        };
        if let Err(error) = self.store.create_exploration(&exploration).await {
            let _ = self
                .store
                .delete_session(&frame_id, &checkpoint.project_id)
                .await;
            let _ = backend.dispose(&workspace).await;
            return Err(error.to_string());
        }
        if !self
            .store
            .transition_exploration(
                &exploration.id,
                ExplorationStatus::Creating,
                ExplorationStatus::Active,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("exploration activation lost a concurrent status update".into());
        }
        self.store
            .get_exploration(&exploration.id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "exploration disappeared after creation".to_string())
    }
}

fn normalize_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "Exploration".into()
    } else {
        name.chars().take(120).collect()
    }
}

fn hash_json(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(crate) fn write_context_archive(
    app_data: &Path,
    destination: &Path,
    frame_id: &str,
    message_head: i64,
    messages: &[wisp_llm::Message],
) -> Result<(), String> {
    let root = app_data.join("exploration-contexts");
    if !root.exists() {
        std::fs::create_dir(&root).map_err(|error| error.to_string())?;
    }
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("exploration context archive root is not a real directory".into());
    }
    if destination.parent() != Some(root.as_path()) {
        return Err("exploration context archive escaped its storage root".into());
    }
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": 1,
        "frame_id": frame_id,
        "message_head": message_head,
        "messages": messages,
    }))
    .map_err(|error| error.to_string())?;
    let temporary = root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, destination).map_err(|error| error.to_string())
}

async fn materialize_checkpoint_context_archive(
    store: &Store,
    app_data: &Path,
    checkpoint: &ExplorationCheckpoint,
    source_workspace: &Path,
    workspace_root: &Path,
) -> Result<(), String> {
    let archive = store
        .get_context_archive(&checkpoint.context_archive_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "checkpoint context archive is missing".to_string())?;
    let relative = Path::new(&archive.storage_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("checkpoint context archive has an unsafe storage path".into());
    }
    let source = app_data.join(relative);
    let metadata = std::fs::symlink_metadata(&source).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("checkpoint context archive is not a regular file".into());
    }
    let bytes = std::fs::read(&source).map_err(|error| error.to_string())?;
    if hex::encode(Sha256::digest(&bytes)) != archive.checksum {
        return Err("checkpoint context archive failed integrity verification".into());
    }
    let history = workspace_root.join(".wisp").join("history");
    std::fs::create_dir_all(&history).map_err(|error| error.to_string())?;
    let legacy_history = source_workspace.join(".wisp").join("history");
    if checkpoint.source_message_seq == checkpoint.source_frame_head_seq && legacy_history.exists()
    {
        let metadata =
            std::fs::symlink_metadata(&legacy_history).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("legacy context history is not a real directory".into());
        }
        let mut copied = 0usize;
        for entry in std::fs::read_dir(&legacy_history).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let metadata =
                std::fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > 50 * 1024 * 1024
            {
                continue;
            }
            let name = entry.file_name();
            if Path::new(&name)
                .extension()
                .and_then(|value| value.to_str())
                != Some("json")
            {
                continue;
            }
            copied += 1;
            if copied > 1_000 {
                return Err("legacy context history has too many archive files".into());
            }
            std::fs::copy(entry.path(), history.join(name)).map_err(|error| error.to_string())?;
        }
    }
    let destination = history.join(format!("{}.json", archive.id));
    std::fs::write(&destination, &bytes).map_err(|error| error.to_string())?;

    let references_path = workspace_root
        .join(".wisp")
        .join("exploration-references.json");
    let encoded = std::fs::read(&references_path).map_err(|error| error.to_string())?;
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| "exploration reference manifest is invalid".to_string())?;
    object.insert(
        "context_archives".into(),
        serde_json::json!([{
            "uri": format!("wisp-history:{}", archive.id),
            "path": format!(".wisp/history/{}.json", archive.id),
            "checksum": archive.checksum,
        }]),
    );
    let temporary = references_path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(temporary, references_path).map_err(|error| error.to_string())
}

pub(crate) fn exploration_runtime_injection(
    root: &Path,
    scope: &StateScope,
) -> Result<Option<String>, String> {
    let StateScope::Exploration { exploration_id, .. } = scope else {
        return Ok(None);
    };
    let references_path = root.join(".wisp").join("exploration-references.json");
    let references = std::fs::read_to_string(&references_path)
        .map_err(|error| format!("cannot read exploration reference manifest: {error}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&references).map_err(|error| error.to_string())?;
    let context_uris = parsed
        .get("context_archives")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("uri").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    Ok(Some(format!(
        "You are working in isolated exploration {exploration_id}. Treat {} as the only writable local project root. Mainline and sibling exploration state is private. Discarding this exploration rolls back only its local workspace and scoped records; external execution contexts are not rolled back. Referenced or unsupported snapshot entries are listed in .wisp/exploration-references.json. Checkpoint history archives can be read in narrow ranges through: {}.",
        root.display(),
        if context_uris.is_empty() {
            "none".into()
        } else {
            context_uris.join(", ")
        }
    )))
}

pub(crate) async fn working_project_for_frame(
    state: &AppState,
    frame_id: &str,
) -> Result<(ActiveProject, StateScope), String> {
    let scope = state
        .store
        .frame_state_scope(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Conversation not found".to_string())?;
    match &scope {
        StateScope::Mainline { project_id } => {
            let project = crate::project_commands::load_active_project(state, project_id)
                .await?
                .0;
            Ok((project, scope))
        }
        StateScope::Exploration {
            project_id,
            exploration_id,
        } => {
            let exploration = state
                .store
                .get_exploration(exploration_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Exploration not found".to_string())?;
            if exploration.status == ExplorationStatus::Failed {
                return Err(coded_error(
                    ERR_NOT_WRITABLE,
                    "this exploration is no longer available",
                ));
            }
            let root = dunce::canonicalize(&exploration.workspace_dir).map_err(|error| {
                coded_error(
                    ERR_HISTORY_UNAVAILABLE,
                    format!("exploration workspace is unavailable: {error}"),
                )
            })?;
            let expected_root = dunce::canonicalize(state.app_data.join("explorations"))
                .map_err(|error| error.to_string())?;
            if !root.starts_with(&expected_root) {
                return Err("exploration workspace is outside app data".into());
            }
            let skills = Arc::new(load_skill_index(&root));
            let memory = Arc::new(MemoryManager::new(&root));
            Ok((
                ActiveProject {
                    id: project_id.clone(),
                    root,
                    skills,
                    memory,
                },
                scope,
            ))
        }
    }
}

pub(crate) async fn working_project_for_active_frame(
    state: &AppState,
    window_label: &str,
) -> Result<(ActiveProject, StateScope), String> {
    match state.active_frame(window_label) {
        Some(frame_id) => working_project_for_frame(state, &frame_id).await,
        None => {
            let project = state.require_active(window_label)?;
            Ok((project.clone(), StateScope::mainline(project.id.clone())))
        }
    }
}

pub(crate) async fn require_writable_scope(
    store: &Store,
    scope: &StateScope,
) -> Result<(), String> {
    match scope {
        StateScope::Mainline { project_id } => {
            if store
                .project_mainline_is_frozen(project_id)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err(coded_error(
                    ERR_MAINLINE_FROZEN,
                    "the mainline is frozen until an exploration is selected or the complete round is abandoned",
                ));
            }
        }
        StateScope::Exploration { exploration_id, .. } => {
            let status = store
                .get_exploration(exploration_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Exploration not found".to_string())?
                .status;
            if status != ExplorationStatus::Active {
                return Err(coded_error(
                    ERR_NOT_WRITABLE,
                    "only an active exploration accepts writes",
                ));
            }
        }
    }
    Ok(())
}

/// A live exploration round owns the project mainline. Its exact source
/// transcript is immutable. Other ordinary conversations may continue to
/// chat, but return `true` so their project-mutating tools are withheld.
pub(crate) async fn conversation_project_write_locked(
    store: &Store,
    scope: &StateScope,
    frame_id: Option<&str>,
) -> Result<bool, String> {
    match scope {
        StateScope::Mainline { project_id } => {
            if let Some(frame_id) = frame_id {
                if store
                    .mainline_frame_is_frozen(frame_id)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    return Err(coded_error(
                        ERR_MAINLINE_FROZEN,
                        "the exploration source mainline is frozen until the round is resolved",
                    ));
                }
            }
            store
                .project_mainline_is_frozen(project_id)
                .await
                .map_err(|error| error.to_string())
        }
        StateScope::Exploration { .. } => {
            require_writable_scope(store, scope).await?;
            Ok(false)
        }
    }
}

pub(crate) async fn reject_private_exploration_project_mutation(
    store: &Store,
    project_id: &str,
    action: &str,
) -> Result<(), String> {
    if store
        .project_has_private_explorations(project_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "exploration_project_mutation_blocked: {action} is unavailable while this project has an unresolved exploration round."
        ));
    }
    Ok(())
}

/// Starts the first candidate under the project's exclusive activity lock so
/// its snapshot cannot race project writes. Later candidates use a shared lock,
/// allowing sibling exploration turns to keep running. The short creation gate
/// makes every candidate in the round reuse one immutable baseline.
#[tauri::command]
pub(crate) async fn start_exploration(
    state: State<'_, AppState>,
    terminals: State<'_, crate::terminal_sessions::TerminalManager>,
    window: WorkspaceSurface,
    source_frame_id: String,
    turn_index: Option<i64>,
    name: String,
) -> Result<Exploration, String> {
    let owner = state
        .store
        .frame_state_scope(&source_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| coded_error(ERR_HISTORY_UNAVAILABLE, "source conversation not found"))?;
    let active = state.require_active(window.label())?;
    if owner.project_id() != active.id || !matches!(owner, StateScope::Mainline { .. }) {
        return Err("Source conversation does not belong to the active mainline".into());
    }
    if wisp_store::is_assistant_project_id(&active.id) {
        return Err("The research assistant keeps a single conversation.".into());
    }
    if state
        .store
        .session_branch_state(&source_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(coded_error(
            ERR_BRANCH_UNSUPPORTED,
            "Conversation branches cannot start an exploration.",
        ));
    }
    let _creation = state.begin_exploration_creation(&active.id).await;
    let round_already_active = state
        .store
        .mainline_frame_is_frozen(&source_frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let (_shared_activity, _exclusive_activity) = if round_already_active {
        (Some(state.begin_project_activity(&active.id)?), None)
    } else {
        (
            None,
            Some(state.begin_project_exclusive_activity(&active.id)?),
        )
    };
    if state
        .store
        .project_has_current_exploration_for_other_source(&active.id, &source_frame_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(coded_error(
            ERR_ROUND_ACTIVE,
            "finish the current exploration round before starting from another conversation",
        ));
    }
    if state.running_turns.lock().await.contains(&source_frame_id) {
        return Err(coded_error(
            ERR_SOURCE_BUSY,
            "wait for the source turn to finish",
        ));
    }
    if terminals.has_running(&active.id, MAINLINE_SCOPE_KEY) {
        return Err(coded_error(
            ERR_SOURCE_BUSY,
            "close the mainline terminal before starting an exploration",
        ));
    }
    crate::exploration_promotion::ensure_no_queued_turns(&state, std::iter::once(&source_frame_id))
        .await?;
    let service = ExplorationService::new(state.store.clone(), state.app_data.clone());
    let checkpoint = service
        .create_checkpoint_at(&active.id, &source_frame_id, turn_index)
        .await?;
    let exploration = service.create_exploration(&checkpoint.id, &name).await?;
    let (project, _) = working_project_for_frame(&state, &exploration.frame_id).await?;
    state.set_active(window.label(), project);
    state.set_active_frame(window.label(), Some(exploration.frame_id.clone()));
    Ok(exploration)
}

#[tauri::command]
pub(crate) async fn list_project_explorations(
    state: State<'_, AppState>,
    window: WorkspaceSurface,
) -> Result<Vec<ExplorationSummary>, String> {
    let project = state.require_active(window.label())?;
    state
        .store
        .list_project_explorations(&project.id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn open_exploration(
    state: State<'_, AppState>,
    window: WorkspaceSurface,
    exploration_id: String,
) -> Result<Exploration, String> {
    let exploration = state
        .store
        .get_exploration(&exploration_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Exploration not found".to_string())?;
    let (project, _) = working_project_for_frame(&state, &exploration.frame_id).await?;
    state.set_active(window.label(), project);
    state.set_active_frame(window.label(), Some(exploration.frame_id.clone()));
    Ok(exploration)
}

/// Explicitly resolve the current round without selecting a winner. The
/// original mainline stays untouched; every candidate scope is purged before
/// the family generation advances and releases the freeze.
#[tauri::command]
pub(crate) async fn abandon_exploration_round(
    state: State<'_, AppState>,
    terminals: State<'_, crate::terminal_sessions::TerminalManager>,
    window: WorkspaceSurface,
    source_frame_id: String,
) -> Result<(), String> {
    let owner = state
        .store
        .frame_state_scope(&source_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Mainline conversation not found".to_string())?;
    let StateScope::Mainline { project_id } = owner else {
        return Err("Only a mainline conversation can abandon an exploration round".into());
    };
    let summaries = state
        .store
        .list_project_explorations(&project_id)
        .await
        .map_err(|error| error.to_string())?;
    let selected = summaries
        .iter()
        .find(|summary| summary.source_frame_id == source_frame_id)
        .map(|summary| summary.exploration.clone())
        .ok_or_else(|| "No current exploration round belongs to this mainline".to_string())?;
    let candidates = state
        .store
        .list_exploration_round_candidates(&selected.id)
        .await
        .map_err(|error| error.to_string())?;
    let workspace_backend = PersistentExplorationWorkspace::new(state.app_data.clone());
    let mut snapshots = Vec::new();
    let mut context_archives = Vec::new();
    for candidate in &candidates {
        if let Some(checkpoint) = state
            .store
            .get_exploration_checkpoint(&candidate.checkpoint_id)
            .await
            .map_err(|error| error.to_string())?
        {
            if let Ok(snapshot) = workspace_backend.load_snapshot(&checkpoint.workspace_snapshot_id)
            {
                snapshots.push(snapshot);
            }
            if let Some(archive) = state
                .store
                .get_context_archive(&checkpoint.context_archive_id)
                .await
                .map_err(|error| error.to_string())?
            {
                if !context_archives
                    .iter()
                    .any(|existing: &ContextArchiveRecord| existing.id == archive.id)
                {
                    context_archives.push(archive);
                }
            }
        }
    }
    let _exclusive = state.begin_project_exclusive_activity(&project_id)?;

    let running_frames = state.running_turns.lock().await.clone();
    if running_frames.contains(&source_frame_id)
        || candidates
            .iter()
            .any(|candidate| running_frames.contains(&candidate.frame_id))
    {
        return Err(coded_error(
            ERR_SOURCE_BUSY,
            "wait for mainline and exploration turns to finish before abandoning the round",
        ));
    }
    crate::exploration_promotion::ensure_no_queued_turns(
        state.inner(),
        std::iter::once(&source_frame_id)
            .chain(candidates.iter().map(|candidate| &candidate.frame_id)),
    )
    .await?;
    if terminals.has_running(&project_id, MAINLINE_SCOPE_KEY)
        || candidates
            .iter()
            .any(|candidate| terminals.has_running(&project_id, &candidate.id))
    {
        return Err(coded_error(
            ERR_SOURCE_BUSY,
            "close mainline and exploration terminals before abandoning the round",
        ));
    }
    if state
        .store
        .project_has_active_runs(&project_id)
        .await
        .map_err(|error| error.to_string())?
        || futures_util::future::try_join_all(candidates.iter().map(|candidate| async {
            state
                .store
                .exploration_has_active_runs(&candidate.id)
                .await
                .map_err(|error| error.to_string())
        }))
        .await?
        .into_iter()
        .any(|active| active)
    {
        return Err(coded_error(
            ERR_ACTIVE_RUN,
            "finish or cancel mainline and exploration Runs before abandoning the round",
        ));
    }

    for candidate in &candidates {
        state
            .runtime_manager
            .stop_scope(&project_id, &candidate.id)
            .await;
        terminals.stop_scope(&project_id, &candidate.id);
    }
    state
        .store
        .abandon_exploration_round(&selected.id)
        .await
        .map_err(|error| error.to_string())?;
    {
        let mut sessions = state.sessions.lock().await;
        for candidate in &candidates {
            sessions.remove(&candidate.frame_id);
        }
    }
    if let Ok(mut allowed) = state.full_permission_sessions.write() {
        for candidate in &candidates {
            allowed.remove(&candidate.frame_id);
        }
    }
    for candidate in &candidates {
        crate::approval_commands::cancel_pending_confirmation(&state, &candidate.frame_id);
        state.remove_notification_window(&candidate.frame_id);
    }
    crate::exploration_promotion::ExplorationPromotionService::new(
        state.store.clone(),
        state.app_data.clone(),
    )
    .dispose_resolved_round_workspaces(&candidates, &snapshots, &context_archives)
    .await;
    let (project, _, _) = crate::project_commands::load_active_project(&state, &project_id).await?;
    state.set_active(window.label(), project);
    state.set_active_frame(window.label(), Some(source_frame_id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DenyExternalRunEnv {
        root: PathBuf,
        prompts: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl wisp_tools::ToolEnv for DenyExternalRunEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, message: &str) -> bool {
            self.prompts.lock().unwrap().push(message.to_string());
            false
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    async fn fixture(label: &str) -> (ExplorationService, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "wisp_exploration_commands_{label}_{}",
            uuid::Uuid::new_v4()
        ));
        let project = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(project.join("baseline.txt"), b"baseline").unwrap();
        let store = Store::open(&base.join("store.sqlite")).await.unwrap();
        store
            .create_project("p", "Project", &project.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("main", "p", "OPERON", "model")
            .await
            .unwrap();
        store
            .append_message("main", 1, &wisp_llm::Message::user("question"))
            .await
            .unwrap();
        store
            .append_message("main", 2, &wisp_llm::Message::assistant("answer"))
            .await
            .unwrap();
        store
            .append_session_ui_event(
                "main",
                1,
                r#"{"kind":"User","frame_id":"main","text":"question"}"#,
            )
            .await
            .unwrap();
        (ExplorationService::new(store, app_data), base, project)
    }

    #[tokio::test]
    async fn historical_exploration_keeps_only_selected_context_and_reuses_it_after_restart() {
        let (service, base, project) = fixture("historical").await;
        service
            .store
            .append_message("main", 8, &wisp_llm::Message::user("later question"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 9, &wisp_llm::Message::assistant("later answer"))
            .await
            .unwrap();
        for (seq, event) in [
            (2, r#"{"kind":"Text","frame_id":"main","delta":"answer"}"#),
            (3, r#"{"kind":"MessageBoundary","frame_id":"main","seq":2}"#),
            (
                4,
                r#"{"kind":"User","frame_id":"main","text":"later question"}"#,
            ),
            (
                5,
                r#"{"kind":"Text","frame_id":"main","delta":"later answer"}"#,
            ),
            (6, r#"{"kind":"MessageBoundary","frame_id":"main","seq":9}"#),
        ] {
            service
                .store
                .append_session_ui_event("main", seq, event)
                .await
                .unwrap();
        }
        std::fs::write(project.join("baseline.txt"), b"current files").unwrap();
        let checkpoint = service
            .create_checkpoint_at("p", "main", Some(0))
            .await
            .unwrap();
        assert_eq!(checkpoint.source_message_seq, 2);
        assert_eq!(checkpoint.source_frame_head_seq, 9);
        assert_eq!(checkpoint.source_ui_event_seq, 3);
        assert_eq!(checkpoint.source_ui_event_head_seq, 6);
        let exploration = service
            .create_exploration(&checkpoint.id, "Historical")
            .await
            .unwrap();
        let messages = service
            .store
            .load_messages(&exploration.frame_id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content.as_text(), "answer");
        assert_eq!(
            service
                .store
                .load_session_ui_events(&exploration.frame_id)
                .await
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            std::fs::read(Path::new(&exploration.workspace_dir).join("baseline.txt")).unwrap(),
            b"current files"
        );
        let archive = std::fs::read_to_string(
            Path::new(&exploration.workspace_dir)
                .join(".wisp/history")
                .join(format!("{}.json", checkpoint.context_archive_id)),
        )
        .unwrap();
        assert!(!archive.contains("later answer"));
        assert!(!archive.contains("later question"));
        assert_eq!(service.store.load_messages("main").await.unwrap().len(), 4);
        let restarted = ExplorationService::new(
            Store::open(&base.join("store.sqlite")).await.unwrap(),
            base.join("app-data"),
        );
        std::fs::write(
            project.join("baseline.txt"),
            b"external edit after creation",
        )
        .unwrap();
        let reused = restarted
            .create_checkpoint_at("p", "main", Some(0))
            .await
            .unwrap();
        assert_eq!(reused.id, checkpoint.id);
        assert!(restarted
            .create_checkpoint_at("p", "main", Some(1))
            .await
            .unwrap_err()
            .contains(ERR_ROUND_ACTIVE));
        let sibling = restarted
            .create_exploration(&reused.id, "Sibling")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(Path::new(&sibling.workspace_dir).join("baseline.txt")).unwrap(),
            b"current files"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn historical_exploration_does_not_copy_later_legacy_ui_events() {
        let (service, base, _) = fixture("historical_legacy").await;
        // Replace the original context with an additional prefix that predates
        // UI persistence. The only UI user event belongs to the later turn.
        service
            .store
            .replace_messages(
                "main",
                &[
                    wisp_llm::Message::user("legacy question"),
                    wisp_llm::Message::assistant("legacy answer"),
                    wisp_llm::Message::user("question"),
                    wisp_llm::Message::assistant("answer"),
                ],
            )
            .await
            .unwrap();
        let checkpoint = service
            .create_checkpoint_at("p", "main", Some(0))
            .await
            .unwrap();
        assert_eq!(checkpoint.source_ui_event_seq, 0);
        assert_eq!(checkpoint.source_ui_event_head_seq, 1);
        let exploration = service
            .create_exploration(&checkpoint.id, "Legacy")
            .await
            .unwrap();
        assert!(service
            .store
            .load_session_ui_events(&exploration.frame_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            service
                .store
                .load_messages(&exploration.frame_id)
                .await
                .unwrap()[1]
                .content
                .as_text(),
            "legacy answer"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn historical_exploration_rejects_missing_or_incomplete_context() {
        let (service, base, _) = fixture("historical_invalid").await;
        for index in [-1, 1] {
            assert!(service
                .create_checkpoint_at("p", "main", Some(index))
                .await
                .unwrap_err()
                .contains(ERR_HISTORY_UNAVAILABLE));
        }
        service
            .store
            .append_message("main", 3, &wisp_llm::Message::user("unfinished"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 4, &wisp_llm::Message::user("final question"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 5, &wisp_llm::Message::assistant("final answer"))
            .await
            .unwrap();
        assert!(service
            .create_checkpoint_at("p", "main", Some(1))
            .await
            .unwrap_err()
            .contains(ERR_SOURCE_INCOMPLETE));
        for seq in 2..=4 {
            service
                .store
                .append_session_ui_event(
                    "main",
                    seq,
                    r#"{"kind":"User","frame_id":"main","text":"archived question"}"#,
                )
                .await
                .unwrap();
        }
        assert!(service
            .create_checkpoint_at("p", "main", Some(0))
            .await
            .unwrap_err()
            .contains(ERR_HISTORY_UNAVAILABLE));
        let _ = std::fs::remove_dir_all(base);
    }

    async fn seed_compacted_turns(service: &ExplorationService) {
        service
            .store
            .append_message("main", 3, &wisp_llm::Message::user("q1"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 4, &wisp_llm::Message::assistant("a1"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 5, &wisp_llm::Message::user("q2"))
            .await
            .unwrap();
        service
            .store
            .append_message("main", 6, &wisp_llm::Message::assistant("a2"))
            .await
            .unwrap();
        for (seq, event) in [
            (2, r#"{"kind":"Text","frame_id":"main","delta":"answer"}"#),
            (3, r#"{"kind":"MessageBoundary","frame_id":"main","seq":2}"#),
            (4, r#"{"kind":"User","frame_id":"main","text":"q1"}"#),
            (5, r#"{"kind":"Text","frame_id":"main","delta":"a1"}"#),
            (6, r#"{"kind":"MessageBoundary","frame_id":"main","seq":4}"#),
            (7, r#"{"kind":"User","frame_id":"main","text":"q2"}"#),
            (8, r#"{"kind":"Text","frame_id":"main","delta":"a2"}"#),
            (9, r#"{"kind":"MessageBoundary","frame_id":"main","seq":6}"#),
        ] {
            service
                .store
                .append_session_ui_event("main", seq, event)
                .await
                .unwrap();
        }
        let first = vec![
            wisp_llm::Message::system("sys"),
            wisp_llm::Message::user("[context summary checkpoint]\n\nsummary"),
            wisp_llm::Message::user("q2"),
            wisp_llm::Message::assistant("a2"),
        ];
        service
            .store
            .open_context_epoch(
                "main",
                wisp_store::OpenContextEpoch {
                    messages: &first,
                    strategy: "manual",
                    kind: "semantic",
                    before_tokens: 1000,
                    after_tokens: 200,
                    checkpoint_index: Some(1),
                    first_kept_seq: Some(5),
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap();
        let second = vec![
            wisp_llm::Message::system("sys"),
            wisp_llm::Message::user("[context summary checkpoint]\n\nsummary 2"),
            wisp_llm::Message::user("q2"),
            wisp_llm::Message::assistant("a2"),
        ];
        service
            .store
            .open_context_epoch(
                "main",
                wisp_store::OpenContextEpoch {
                    messages: &second,
                    strategy: "manual",
                    kind: "semantic",
                    before_tokens: 1000,
                    after_tokens: 200,
                    checkpoint_index: Some(1),
                    first_kept_seq: Some(5),
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap();
        service
            .store
            .append_session_ui_event(
                "main",
                10,
                r#"{"kind":"User","frame_id":"main","text":"[context summary checkpoint]\n\nsummary 2"}"#,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn historical_exploration_after_compaction_clones_frozen_epoch_prefix() {
        let (service, base, _) = fixture("historical_compact").await;
        seed_compacted_turns(&service).await;
        let historical = service
            .create_checkpoint_at("p", "main", Some(0))
            .await
            .unwrap();
        assert_eq!(historical.source_message_seq, 2);
        let exploration = service
            .create_exploration(&historical.id, "Pre-compact")
            .await
            .unwrap();
        let cloned = service
            .store
            .load_messages(&exploration.frame_id)
            .await
            .unwrap();
        assert_eq!(
            cloned
                .iter()
                .map(|message| message.content.as_text().to_string())
                .collect::<Vec<_>>(),
            ["question", "answer"]
        );
        assert!(latest_native_turn_is_complete(&cloned));
        assert!(cloned
            .iter()
            .all(|message| !wisp_store::is_compaction_checkpoint(&message.content.as_text())));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn latest_exploration_after_compaction_clones_head_epoch() {
        let (service, base, _) = fixture("latest_compact").await;
        seed_compacted_turns(&service).await;
        let latest = service.create_checkpoint("p", "main").await.unwrap();
        let exploration = service
            .create_exploration(&latest.id, "Head")
            .await
            .unwrap();
        let cloned = service
            .store
            .load_messages(&exploration.frame_id)
            .await
            .unwrap();
        assert!(latest_native_turn_is_complete(&cloned));
        assert!(cloned
            .iter()
            .any(|message| wisp_store::is_compaction_checkpoint(&message.content.as_text())));
        assert!(cloned
            .iter()
            .any(|message| message.content.as_text() == "q2"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn concurrent_first_candidates_share_one_checkpoint() {
        let (service, base, _) = fixture("concurrent_create").await;
        let locks = Arc::new(crate::ProjectActivityLocks::default());

        let create = |name: &'static str| {
            let service = service.clone();
            let locks = locks.clone();
            tokio::spawn(async move {
                let _creation = locks.exploration_creation("p").lock_owned().await;
                let round_already_active = service
                    .store
                    .mainline_frame_is_frozen("main")
                    .await
                    .unwrap();
                let (_shared_activity, _exclusive_activity) = if round_already_active {
                    (Some(locks.project("p").read_owned().await), None)
                } else {
                    (None, Some(locks.project("p").write_owned().await))
                };
                let checkpoint = service.create_checkpoint("p", "main").await.unwrap();
                service
                    .create_exploration(&checkpoint.id, name)
                    .await
                    .unwrap()
            })
        };
        let (first, second) = tokio::join!(create("First"), create("Second"));
        let first = first.unwrap();
        let second = second.unwrap();

        assert_ne!(first.id, second.id);
        assert_ne!(first.frame_id, second.frame_id);
        assert_ne!(first.workspace_dir, second.workspace_dir);
        assert_eq!(first.checkpoint_id, second.checkpoint_id);
        assert_eq!(
            service
                .store
                .list_exploration_round_candidates(&first.id)
                .await
                .unwrap()
                .len(),
            2
        );

        drop(service);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn repeated_start_reuses_one_checkpoint_for_independent_explorations() {
        let (service, base, project) = fixture("create").await;
        let logical_key = "path:result.txt";
        std::fs::write(project.join("result.txt"), b"baseline result").unwrap();
        let baseline_artifact_id = wisp_store::logical_artifact_id("p", logical_key);
        let baseline_version_id = service
            .store
            .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                version_id: Some("baseline-version".into()),
                artifact_id: baseline_artifact_id.clone(),
                project_id: "p".into(),
                root_frame_id: "main".into(),
                filename: "result.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "result.txt".into(),
                logical_key: Some(logical_key.into()),
                size_bytes: Some(15),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: wisp_store::ArtifactMaterialization::Snapshot,
                capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let baseline_resource = wisp_store::ExternalResource {
            id: "baseline-resource".into(),
            project_id: "p".into(),
            kind: "dataset".into(),
            uri: "doi:10.0000/baseline".into(),
            version: Some("v1".into()),
            checksum: Some("a".repeat(64)),
            size_bytes: Some(42),
            license: None,
            visibility: "public".into(),
            access_instructions: None,
            accessed_at: Some(1),
            created_at: 1,
            updated_at: 1,
        };
        service
            .store
            .save_external_resource(&baseline_resource)
            .await
            .unwrap();
        service
            .store
            .replace_message_resource_links(
                "main",
                1,
                &[wisp_store::MessageResourceLink {
                    id: "baseline-link".into(),
                    frame_id: "main".into(),
                    message_seq: 1,
                    ordinal: 0,
                    original_reference: "result.txt".into(),
                    artifact_id: Some(baseline_artifact_id.clone()),
                    artifact_version_id: Some(baseline_version_id.clone()),
                    display_name: "result.txt".into(),
                    resource_kind: "file".into(),
                    mime_type: "text/plain".into(),
                    status: "ready".into(),
                    error: None,
                    created_artifact: false,
                    created_version: false,
                    created_at: 1,
                }],
            )
            .await
            .unwrap();
        service
            .store
            .append_session_ui_event(
                "main",
                2,
                r#"{"kind":"Text","frame_id":"main","delta":"answer"}"#,
            )
            .await
            .unwrap();
        let checkpoint = service.create_checkpoint("p", "main").await.unwrap();
        let first = service
            .create_exploration(&checkpoint.id, "First")
            .await
            .unwrap();
        let nested_error = service
            .create_checkpoint("p", &first.frame_id)
            .await
            .unwrap_err();
        assert!(nested_error
            .contains("checkpoints must be created from the current mainline conversation"));
        assert!(
            require_writable_scope(&service.store, &StateScope::mainline("p"))
                .await
                .unwrap_err()
                .starts_with(ERR_MAINLINE_FROZEN)
        );
        assert!(conversation_project_write_locked(
            &service.store,
            &StateScope::mainline("p"),
            Some("main"),
        )
        .await
        .unwrap_err()
        .starts_with(ERR_MAINLINE_FROZEN));
        assert!(conversation_project_write_locked(
            &service.store,
            &StateScope::mainline("p"),
            Some("other-conversation"),
        )
        .await
        .unwrap());
        require_writable_scope(
            &service.store,
            &StateScope::exploration("p", first.id.clone()),
        )
        .await
        .unwrap();
        assert!(!conversation_project_write_locked(
            &service.store,
            &StateScope::exploration("p", first.id.clone()),
            Some(&first.frame_id),
        )
        .await
        .unwrap());
        let repeated_checkpoint = service.create_checkpoint("p", "main").await.unwrap();
        assert_eq!(repeated_checkpoint.id, checkpoint.id);
        std::fs::write(
            project.join("external-after-freeze.txt"),
            b"external change",
        )
        .unwrap();
        let frozen_checkpoint = service.create_checkpoint("p", "main").await.unwrap();
        assert_eq!(frozen_checkpoint.id, checkpoint.id);
        let frozen_snapshot = service
            .workspace_backend()
            .load_snapshot(&frozen_checkpoint.workspace_snapshot_id)
            .unwrap();
        assert!(!frozen_snapshot
            .entries
            .iter()
            .any(|entry| entry.path == "external-after-freeze.txt"));
        let second = service
            .create_exploration(&repeated_checkpoint.id, "Second")
            .await
            .unwrap();
        assert_ne!(first.frame_id, second.frame_id);
        assert_ne!(first.workspace_dir, second.workspace_dir);
        assert_eq!(
            service
                .store
                .list_sessions("p")
                .await
                .unwrap()
                .into_iter()
                .map(|session| session.0)
                .collect::<Vec<_>>(),
            vec!["main".to_string()]
        );
        service
            .store
            .create_child_frame("first-child", &first.frame_id, "p", "worker", "model")
            .await
            .unwrap();
        assert_eq!(
            service
                .store
                .frame_state_scope("first-child")
                .await
                .unwrap(),
            Some(StateScope::exploration("p", first.id.clone()))
        );
        assert!(Path::new(&first.workspace_dir)
            .join(".wisp/history")
            .join(format!("{}.json", checkpoint.context_archive_id))
            .is_file());
        let injection = exploration_runtime_injection(
            Path::new(&first.workspace_dir),
            &StateScope::exploration("p", first.id.clone()),
        )
        .unwrap()
        .unwrap();
        assert!(injection.contains(&format!("wisp-history:{}", checkpoint.context_archive_id)));
        std::fs::write(
            Path::new(&first.workspace_dir).join("baseline.txt"),
            b"first",
        )
        .unwrap();
        assert_eq!(
            std::fs::read(Path::new(&second.workspace_dir).join("baseline.txt")).unwrap(),
            b"baseline"
        );
        assert_eq!(
            service
                .store
                .load_messages(&first.frame_id)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(service
            .store
            .load_session_ui_events(&first.frame_id)
            .await
            .unwrap()[0]
            .contains(&first.frame_id));
        let cloned_links = service
            .store
            .list_message_resource_links(&first.frame_id, 0, None)
            .await
            .unwrap();
        assert_eq!(cloned_links.len(), 1);
        assert_eq!(
            cloned_links[0].artifact_version_id.as_deref(),
            Some(baseline_version_id.as_str())
        );

        let first_scope = StateScope::exploration("p", first.id.clone());
        let second_scope = StateScope::exploration("p", second.id.clone());
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let external_tool = crate::run_context::RunInContextTool::new(
            service.store.clone(),
            crate::run_context::RunManager::new(),
            "p".into(),
            Some(first.frame_id.clone()),
        );
        let denied = wisp_tools::Tool::run(
            &external_tool,
            &serde_json::json!({
                "context_id": "ssh:gpu",
                "command": "echo branch"
            }),
            &DenyExternalRunEnv {
                root: PathBuf::from(&first.workspace_dir),
                prompts: prompts.clone(),
            },
        )
        .await;
        assert!(!denied.success);
        assert_eq!(denied.control, wisp_tools::ToolControl::StopBatch);
        assert!(prompts.lock().unwrap()[0].contains("cannot be rolled back"));
        let denied_mainline_run = wisp_tools::Tool::run(
            &external_tool,
            &serde_json::json!({
                "context_id": "local",
                "command": format!("cat '{}'", project.join("baseline.txt").display())
            }),
            &DenyExternalRunEnv {
                root: PathBuf::from(&first.workspace_dir),
                prompts: prompts.clone(),
            },
        )
        .await;
        assert!(!denied_mainline_run.success);
        assert_eq!(
            denied_mainline_run.control,
            wisp_tools::ToolControl::StopBatch
        );
        assert!(denied_mainline_run
            .content
            .contains("exploration_scope_violation"));
        std::fs::write(project.join("later.txt"), b"later mainline result").unwrap();
        let later_mainline_version = service
            .store
            .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                version_id: Some("later-mainline-version".into()),
                artifact_id: baseline_artifact_id.clone(),
                project_id: "p".into(),
                root_frame_id: "main".into(),
                filename: "result.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "later.txt".into(),
                logical_key: Some(logical_key.into()),
                size_bytes: Some(21),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: wisp_store::ArtifactMaterialization::Snapshot,
                capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let first_artifact_id = wisp_store::scoped_logical_artifact_id("p", &first.id, logical_key);
        std::fs::write(
            Path::new(&first.workspace_dir).join("result.txt"),
            b"first result",
        )
        .unwrap();
        let first_version_id = service
            .store
            .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                version_id: Some("first-version".into()),
                artifact_id: first_artifact_id.clone(),
                project_id: "p".into(),
                root_frame_id: first.frame_id.clone(),
                filename: "result.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "result.txt".into(),
                logical_key: Some(logical_key.into()),
                size_bytes: Some(12),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: wisp_store::ArtifactMaterialization::Snapshot,
                capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        assert_eq!(
            service
                .store
                .get_artifact_version(&first_version_id)
                .await
                .unwrap()
                .unwrap()
                .parent_version_id
                .as_deref(),
            Some(baseline_version_id.as_str())
        );
        assert_eq!(
            service
                .store
                .get_artifact_head("p", MAINLINE_SCOPE_KEY, logical_key)
                .await
                .unwrap()
                .unwrap()
                .artifact_version_id,
            later_mainline_version
        );
        assert!(service
            .store
            .get_artifact_head("p", &first.id, logical_key)
            .await
            .unwrap()
            .is_some());
        assert!(service
            .store
            .get_artifact_head("p", &second.id, logical_key)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            service
                .store
                .search_exploration_artifacts("p", &first.id, "", 20)
                .await
                .unwrap()[0]
                .id,
            first_artifact_id
        );
        let second_artifacts = service
            .store
            .search_exploration_artifacts("p", &second.id, "", 20)
            .await
            .unwrap();
        assert_eq!(second_artifacts[0].id, baseline_artifact_id);
        assert_eq!(second_artifacts[0].path, "result.txt");
        assert_eq!(
            service
                .store
                .artifact_path_in_scope(&second_artifacts[0].id, &second_scope)
                .await
                .unwrap()
                .as_deref(),
            Some("result.txt")
        );
        assert!(service
            .store
            .get_external_resource_in_scope(&baseline_resource.id, &first_scope)
            .await
            .unwrap()
            .is_some());
        let private_resource = wisp_store::ExternalResource {
            id: "first-private-resource".into(),
            project_id: "p".into(),
            kind: "dataset".into(),
            uri: "file:private.csv".into(),
            version: None,
            checksum: None,
            size_bytes: Some(12),
            license: None,
            visibility: "private".into(),
            access_instructions: None,
            accessed_at: Some(2),
            created_at: 2,
            updated_at: 2,
        };
        service
            .store
            .save_external_resource_in_scope(&private_resource, &first_scope)
            .await
            .unwrap();
        assert!(service
            .store
            .get_external_resource(&private_resource.id)
            .await
            .unwrap()
            .is_none());
        assert!(service
            .store
            .get_external_resource_in_scope(&private_resource.id, &first_scope)
            .await
            .unwrap()
            .is_some());
        assert!(service
            .store
            .get_external_resource_in_scope(&private_resource.id, &second_scope)
            .await
            .unwrap()
            .is_none());

        let mut run = wisp_store::RunRecord::new("branch-run", "p", "local", "Run", "command");
        run.frame_id = Some(first.frame_id.clone());
        service.store.create_run(&run).await.unwrap();
        service
            .store
            .save_run_input(&wisp_store::RunInput {
                id: "branch-input".into(),
                run_id: run.id.clone(),
                artifact_version_id: Some(first_version_id.clone()),
                external_resource_id: None,
                source_ref: "result.txt".into(),
                role: "input".into(),
                required: true,
                basis: wisp_store::LineageBasis::Declared,
                confidence: wisp_store::LineageConfidence::Exact,
                created_at: 2,
            })
            .await
            .unwrap();
        assert!(service
            .store
            .save_run_input(&wisp_store::RunInput {
                id: "private-resource-input".into(),
                run_id: run.id.clone(),
                artifact_version_id: None,
                external_resource_id: Some(private_resource.id.clone()),
                source_ref: private_resource.uri.clone(),
                role: "input".into(),
                required: true,
                basis: wisp_store::LineageBasis::Declared,
                confidence: wisp_store::LineageConfidence::Exact,
                created_at: 2,
            })
            .await
            .is_ok());
        assert!(service
            .store
            .list_runs_by_project("p")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            service
                .store
                .list_runs_in_scope(&first_scope)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(service
            .store
            .list_runs_in_scope(&second_scope)
            .await
            .unwrap()
            .is_empty());
        let mut second_run =
            wisp_store::RunRecord::new("second-run", "p", "local", "Second Run", "command");
        second_run.frame_id = Some(second.frame_id.clone());
        service.store.create_run(&second_run).await.unwrap();
        assert!(service
            .store
            .save_run_input(&wisp_store::RunInput {
                id: "cross-branch-input".into(),
                run_id: second_run.id.clone(),
                artifact_version_id: None,
                external_resource_id: Some(private_resource.id.clone()),
                source_ref: private_resource.uri.clone(),
                role: "input".into(),
                required: true,
                basis: wisp_store::LineageBasis::Declared,
                confidence: wisp_store::LineageConfidence::Exact,
                created_at: 2,
            })
            .await
            .is_err());

        let decision = wisp_store::ResearchNode::new(
            "branch-decision",
            "p",
            wisp_store::ResearchNodeKind::Decision,
            "Use the first result",
        )
        .unwrap();
        service
            .store
            .save_research_node_in_scope(&decision, &first_scope)
            .await
            .unwrap();
        assert!(!service
            .store
            .research_graph("p")
            .await
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == decision.id));
        assert!(service
            .store
            .research_graph_in_scope(&first_scope)
            .await
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == decision.id));
        assert!(!service
            .store
            .research_graph_in_scope(&second_scope)
            .await
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == decision.id));
        let effects = service
            .store
            .list_exploration_effects(&first.id)
            .await
            .unwrap();
        assert!(effects.iter().any(|effect| {
            effect.effect_kind == "run" && effect.recoverability == "local_reversible"
        }));
        let reopened = Store::open(&base.join("store.sqlite")).await.unwrap();
        assert_eq!(
            reopened
                .get_exploration(&first.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExplorationStatus::Active
        );
        assert!(Path::new(&first.workspace_dir).is_dir());
        assert!(project.join("baseline.txt").is_file());
        drop(reopened);
        drop(service);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn checkpoint_accepts_completion_tool_head_and_rejects_active_run() {
        let (service, base, _) = fixture("guards").await;
        service
            .store
            .append_message("main", 3, &wisp_llm::Message::user("unfinished"))
            .await
            .unwrap();
        assert!(service
            .create_checkpoint("p", "main")
            .await
            .unwrap_err()
            .starts_with(ERR_SOURCE_INCOMPLETE));
        service
            .store
            .append_message(
                "main",
                4,
                &wisp_llm::Message::tool("complete-1", "attempt_completion", "completed analysis"),
            )
            .await
            .unwrap();
        service
            .store
            .append_message(
                "main",
                5,
                &wisp_llm::Message::tool(
                    "skipped-1",
                    "shell",
                    "Skipped because attempt_completion ended the turn.",
                ),
            )
            .await
            .unwrap();
        service.create_checkpoint("p", "main").await.unwrap();
        let mut run = wisp_store::RunRecord::new("run", "p", "local", "Run", "command");
        run.frame_id = Some("main".into());
        run.status = wisp_store::RunStatus::Running;
        service.store.create_run(&run).await.unwrap();
        assert!(service
            .create_checkpoint("p", "main")
            .await
            .unwrap_err()
            .starts_with(ERR_ACTIVE_RUN));
        drop(service);
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn conversation_branches_cannot_create_exploration_checkpoints() {
        let (service, base, _) = fixture("branch_no_explore").await;
        service
            .store
            .create_frame("branch", "p", "OPERON", "model")
            .await
            .unwrap();
        service
            .store
            .append_message("branch", 1, &wisp_llm::Message::user("question"))
            .await
            .unwrap();
        service
            .store
            .append_message("branch", 2, &wisp_llm::Message::assistant("answer"))
            .await
            .unwrap();
        service
            .store
            .set_session_branch_point("branch", "main", 0, "after_response")
            .await
            .unwrap();
        let error = service.create_checkpoint("p", "branch").await.unwrap_err();
        assert!(error.starts_with(ERR_BRANCH_UNSUPPORTED), "{error}");
        assert!(error.contains("Conversation branches cannot start an exploration."));
        drop(service);
        let _ = std::fs::remove_dir_all(base);
    }
}
