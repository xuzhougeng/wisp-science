//! The host's exploration commands, a port of the Tauri commands in the
//! WebView's `exploration_commands.rs` and `exploration_promotion.rs`
//! without what this host does not have (terminals, a turn queue, window
//! state). Each runs under the project's activity lock as there.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use wisp_store::{ContextArchiveRecord, ExplorationStatus, StateScope};

use super::promotion::{
    self, load_round_context_archives, load_round_snapshots, validated_exploration_root,
    ExplorationPromotionService,
};
use super::service::{self, coded_error, counts_as_user_turn, ExplorationService};
use crate::desktop::TurnRef;
use crate::exploration_workspace::{
    ExplorationWorkspaceBackend, FileDeltaKind, MaterializedWorkspace,
    PersistentExplorationWorkspace,
};
use crate::Server;

/// A new exploration: its id and its conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedExploration {
    pub id: String,
    pub session: String,
}

/// One file an exploration (or the mainline since the checkpoint) changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// `added`, `modified` or `deleted`.
    pub kind: &'static str,
}

/// What keeping an exploration would bring into the mainline, and whether
/// it can be kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionPreview {
    pub eligible: bool,
    /// Why not, when it cannot be kept.
    pub blockers: Vec<String>,
    /// Handed back to keep it, so a preview that went stale is refused.
    pub guard: String,
    /// Files can still be copied over by hand, then the round ended.
    pub manual_resolution: bool,
    pub files: Vec<FileChange>,
    /// What changed in the mainline since the checkpoint, of those paths.
    pub mainline_files: Vec<FileChange>,
    pub artifacts: usize,
    pub runs: usize,
    pub decisions: usize,
    /// Effects outside this computer, which are not undone.
    pub external_effects: usize,
}

fn kind(kind: &FileDeltaKind) -> &'static str {
    match kind {
        FileDeltaKind::Added => "added",
        FileDeltaKind::Modified => "modified",
        FileDeltaKind::Deleted => "deleted",
    }
}

impl Server {
    pub(crate) fn exploration_data(&self) -> PathBuf {
        self.0.config.data.clone()
    }

    fn running(&self, frame: &str) -> bool {
        self.0
            .sessions
            .lock()
            .unwrap()
            .get(frame)
            .is_some_and(|session| session.live.lock().unwrap().running)
    }

    /// Stops and forgets a conversation's live state: its approvals are
    /// refused and its interpreters go with it.
    fn drop_live(&self, frame: &str) {
        if let Some(session) = self.0.sessions.lock().unwrap().remove(frame) {
            session.stop();
        }
    }

    /// The explorations of the project's current round, if any.
    pub async fn explorations(&self, project: &str) -> Result<Vec<wisp_dto::ExplorationSummary>> {
        self.known_project(project).await?;
        let rows = self.0.store.list_project_explorations(project).await?;
        Ok(serde_json::from_value(serde_json::to_value(rows)?)?)
    }

    /// The exploration state a conversation's view shows.
    pub(crate) async fn exploration_state(
        &self,
        project: &str,
        session: &str,
    ) -> Option<wisp_dto::native_conversations::ExplorationState> {
        let rows = self.explorations(project).await.ok()?;
        let state = wisp_dto::native_conversations::ExplorationState::for_session(&rows, session);
        (state.summary.is_some() || state.candidate_count > 0 || state.source_frozen)
            .then_some(state)
    }

    /// Starts an exploration from the answer to `turn` (the latest when
    /// none) of a mainline conversation: the first of a round freezes one
    /// checkpoint of the project's files and records, and every candidate
    /// gets its own copy of the files and the conversation up to there.
    pub async fn start_exploration(
        &self,
        project: &str,
        source: &str,
        turn: Option<&TurnRef>,
        name: &str,
    ) -> Result<StartedExploration> {
        self.own_conversation(project, source).await?;
        let store = &self.0.store;
        let owner = store.frame_state_scope(source).await?.context(coded_error(
            service::ERR_HISTORY_UNAVAILABLE,
            "source conversation not found",
        ))?;
        anyhow::ensure!(
            owner.project_id() == project && matches!(owner, StateScope::Mainline { .. }),
            "Source conversation does not belong to the active mainline"
        );
        anyhow::ensure!(
            !wisp_store::is_assistant_project_id(project),
            "The research assistant keeps a single conversation."
        );
        anyhow::ensure!(
            store.session_branch_state(source).await?.is_none(),
            coded_error(
                service::ERR_BRANCH_UNSUPPORTED,
                "Conversation branches cannot start an exploration."
            )
        );
        anyhow::ensure!(
            crate::acp_turn::agent_of(store, source).await?.is_none(),
            coded_error(
                service::ERR_ACP_UNSUPPORTED,
                "a conversation run by an ACP agent cannot start an exploration"
            )
        );
        // The turn as the transcript names it, counted from the start.
        let turn_index = match turn {
            None => None,
            Some(turn) => {
                self.locate_turn(source, turn).await?;
                let messages = store.load_messages(source).await?;
                let fallback = messages.iter().filter(|m| counts_as_user_turn(m)).count() as i64;
                let visual = store.visual_user_count(source).await? as i64;
                Some(visual.max(fallback) - 1 - turn.from_end as i64)
            }
        };
        let _creation = self
            .0
            .project_locks
            .exploration_creation(project)
            .lock_owned()
            .await;
        let round_already_active = store.mainline_frame_is_frozen(source).await?;
        let lock = self.0.project_locks.project(project);
        let (_shared, _exclusive) = if round_already_active {
            (
                Some(lock.try_read_owned().map_err(|_| {
                    anyhow!("This project is busy. Try again when the current project operation finishes.")
                })?),
                None,
            )
        } else {
            (
                None,
                Some(lock.try_write_owned().map_err(|_| {
                    anyhow!(coded_error(
                        service::ERR_SOURCE_BUSY,
                        "wait for every turn in this project to finish"
                    ))
                })?),
            )
        };
        anyhow::ensure!(
            !store
                .project_has_current_exploration_for_other_source(project, source)
                .await?,
            coded_error(
                service::ERR_ROUND_ACTIVE,
                "finish the current exploration round before starting from another conversation"
            )
        );
        anyhow::ensure!(
            !self.running(source),
            coded_error(
                service::ERR_SOURCE_BUSY,
                "wait for the source turn to finish"
            )
        );
        let explorer = ExplorationService::new(store.clone(), self.exploration_data());
        let checkpoint = explorer
            .create_checkpoint_at(project, source, turn_index)
            .await
            .map_err(|error| anyhow!(error))?;
        let exploration = explorer
            .create_exploration(&checkpoint.id, name)
            .await
            .map_err(|error| anyhow!(error))?;
        // The conversation's own choices come along: its model, full access.
        for key in [
            crate::desktop::model_choice_key,
            crate::desktop::full_access_key,
        ] {
            if let Some(value) = store.get_setting(&key(source)).await? {
                store
                    .set_setting(&key(&exploration.frame_id), &value)
                    .await?;
            }
        }
        let allowed = self
            .session(source)
            .live
            .lock()
            .unwrap()
            .allowed_tools
            .clone();
        self.session(&exploration.frame_id)
            .live
            .lock()
            .unwrap()
            .allowed_tools = allowed;
        Ok(StartedExploration {
            id: exploration.id,
            session: exploration.frame_id,
        })
    }

    /// Ends the round of `source` without keeping any exploration: every
    /// candidate goes, and the mainline is as it was.
    pub async fn abandon_exploration_round(&self, project: &str, source: &str) -> Result<()> {
        self.own_conversation(project, source).await?;
        self.abandon_round(source)
            .await
            .map(drop)
            .map_err(|error| anyhow!(error))
    }

    async fn abandon_round(&self, source: &str) -> Result<String, String> {
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let StateScope::Mainline { project_id } = store
            .frame_state_scope(source)
            .await
            .map_err(e)?
            .ok_or("Mainline conversation not found")?
        else {
            return Err("Only a mainline conversation can abandon an exploration round".into());
        };
        let selected = store
            .list_project_explorations(&project_id)
            .await
            .map_err(e)?
            .into_iter()
            .find(|summary| summary.source_frame_id == source)
            .map(|summary| summary.exploration)
            .ok_or("No current exploration round belongs to this mainline")?;
        let candidates = store
            .list_exploration_round_candidates(&selected.id)
            .await
            .map_err(e)?;
        let backend = PersistentExplorationWorkspace::new(self.exploration_data());
        let mut snapshots = Vec::new();
        let mut archives: Vec<ContextArchiveRecord> = Vec::new();
        for candidate in &candidates {
            if let Some(checkpoint) = store
                .get_exploration_checkpoint(&candidate.checkpoint_id)
                .await
                .map_err(e)?
            {
                if let Ok(snapshot) = backend.load_snapshot(&checkpoint.workspace_snapshot_id) {
                    snapshots.push(snapshot);
                }
                if let Some(archive) = store
                    .get_context_archive(&checkpoint.context_archive_id)
                    .await
                    .map_err(e)?
                {
                    if !archives.iter().any(|existing| existing.id == archive.id) {
                        archives.push(archive);
                    }
                }
            }
        }
        let _exclusive = self
            .0
            .project_locks
            .project(&project_id)
            .try_write_owned()
            .map_err(|_| "ProjectBusy: another project operation is still active".to_string())?;
        if self.running(source) || candidates.iter().any(|c| self.running(&c.frame_id)) {
            return Err(coded_error(
                service::ERR_SOURCE_BUSY,
                "wait for mainline and exploration turns to finish before abandoning the round",
            ));
        }
        let mut active_runs = store
            .project_has_active_runs(&project_id)
            .await
            .map_err(e)?;
        for candidate in &candidates {
            active_runs |= store
                .exploration_has_active_runs(&candidate.id)
                .await
                .map_err(e)?;
        }
        if active_runs {
            return Err(coded_error(
                service::ERR_ACTIVE_RUN,
                "finish or cancel mainline and exploration Runs before abandoning the round",
            ));
        }
        for candidate in &candidates {
            self.drop_live(&candidate.frame_id);
        }
        store
            .abandon_exploration_round(&selected.id)
            .await
            .map_err(e)?;
        ExplorationPromotionService::new(store.clone(), self.exploration_data())
            .dispose_resolved_round_workspaces(&candidates, &snapshots, &archives)
            .await;
        Ok(project_id)
    }

    /// What keeping `exploration` would bring in, and whether it can be
    /// kept now.
    pub async fn preview_exploration(
        &self,
        project: &str,
        exploration: &str,
    ) -> Result<PromotionPreview> {
        let preview = self.owned_preview(project, exploration).await?;
        let changes = |files: &[crate::exploration_workspace::FileDelta]| {
            files
                .iter()
                .map(|delta| FileChange {
                    path: delta.path.clone(),
                    kind: kind(&delta.kind),
                })
                .collect::<Vec<_>>()
        };
        Ok(PromotionPreview {
            eligible: preview.eligibility.eligible,
            blockers: preview
                .eligibility
                .reasons
                .iter()
                .map(|reason| reason.message.clone())
                .collect(),
            guard: preview.eligibility.expected_guard_hash.clone(),
            manual_resolution: preview.eligibility.manual_resolution_available,
            files: changes(&preview.diff.files),
            mainline_files: changes(&preview.mainline_changes.files),
            artifacts: preview.diff.artifacts.len(),
            runs: preview.diff.runs.len(),
            decisions: preview.diff.decisions.len(),
            external_effects: preview.diff.external_effects.len(),
        })
    }

    async fn owned_preview(
        &self,
        project: &str,
        exploration: &str,
    ) -> Result<promotion::ExplorationPromotionPreview> {
        let row = self
            .0
            .store
            .get_exploration(exploration)
            .await?
            .context("Exploration not found")?;
        self.own_conversation(project, &row.frame_id).await?;
        ExplorationPromotionService::new(self.0.store.clone(), self.exploration_data())
            .preview(exploration)
            .await
            .map_err(|error| anyhow!(error))
    }

    /// Keeps `exploration`: its files and records become the mainline's,
    /// its conversation continues the source conversation, and the other
    /// candidates go. `guard` is the preview's; a stale one is refused.
    /// Returns the mainline conversation.
    pub async fn promote_exploration(
        &self,
        project: &str,
        exploration: &str,
        guard: &str,
    ) -> Result<String> {
        self.owned_preview(project, exploration).await?;
        self.promote(exploration, guard)
            .await
            .map_err(|error| anyhow!(error))
    }

    async fn promote(&self, exploration_id: &str, guard: &str) -> Result<String, String> {
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let exploration = store
            .get_exploration(exploration_id)
            .await
            .map_err(e)?
            .ok_or_else(|| {
                promotion::coded(promotion::ERR_NOT_PROMOTABLE, "exploration not found")
            })?;
        let checkpoint = store
            .get_exploration_checkpoint(&exploration.checkpoint_id)
            .await
            .map_err(e)?
            .ok_or_else(|| {
                promotion::coded(promotion::ERR_NOT_PROMOTABLE, "checkpoint not found")
            })?;
        let _exclusive = self
            .0
            .project_locks
            .project(&checkpoint.project_id)
            .try_write_owned()
            .map_err(|_| "ProjectBusy: another project operation is still active".to_string())?;
        let candidates = store
            .list_exploration_round_candidates(exploration_id)
            .await
            .map_err(e)?;
        let explorer = ExplorationPromotionService::new(store.clone(), self.exploration_data());
        let snapshots = load_round_snapshots(&explorer, &candidates).await;
        let archives = load_round_context_archives(&explorer, &candidates).await;
        let preflight = explorer.preview(exploration_id).await?;
        if !preflight.eligibility.eligible {
            return Err(promotion::coded(
                preflight
                    .eligibility
                    .code
                    .as_deref()
                    .unwrap_or(promotion::ERR_NOT_PROMOTABLE),
                "exploration is not eligible for fast-forward promotion",
            ));
        }
        if preflight.eligibility.expected_guard_hash != guard {
            return Err(promotion::coded(
                promotion::ERR_WORKSPACE_CHANGED,
                "the promotion preview is stale; preview the exploration again",
            ));
        }
        if self.running(&checkpoint.source_frame_id)
            || candidates
                .iter()
                .any(|candidate| self.running(&candidate.frame_id))
        {
            return Err(promotion::coded(
                promotion::ERR_EXPLORATION_BUSY,
                "wait for mainline and exploration turns to finish before promotion",
            ));
        }
        let mut active_runs = store
            .project_has_active_runs(&checkpoint.project_id)
            .await
            .map_err(e)?;
        for candidate in &candidates {
            active_runs |= store
                .exploration_has_active_runs(&candidate.id)
                .await
                .map_err(e)?;
        }
        if active_runs {
            return Err(promotion::coded(
                promotion::ERR_EXPLORATION_BUSY,
                "active mainline or exploration Runs block promotion",
            ));
        }
        let result = explorer.promote_locked(exploration_id, guard).await?;
        self.drop_live(&checkpoint.source_frame_id);
        for candidate in &candidates {
            self.drop_live(&candidate.frame_id);
        }
        explorer
            .dispose_resolved_round_workspaces(&candidates, &snapshots, &archives)
            .await;
        if let Err(error) = explorer.workspace_backend().sweep_disposal_quarantine() {
            tracing::warn!(%error, "cannot sweep quarantined exploration workspaces after promotion");
        }
        Ok(result.mainline_frame_id)
    }

    /// Gives up one exploration; the round goes on with the others.
    pub async fn discard_exploration(&self, project: &str, exploration: &str) -> Result<()> {
        let row = self
            .0
            .store
            .get_exploration(exploration)
            .await?
            .context("Exploration not found")?;
        self.own_conversation(project, &row.frame_id).await?;
        self.discard(exploration)
            .await
            .map_err(|error| anyhow!(error))
    }

    async fn discard(&self, exploration_id: &str) -> Result<(), String> {
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let exploration = store
            .get_exploration(exploration_id)
            .await
            .map_err(e)?
            .ok_or_else(|| {
                promotion::coded(promotion::ERR_NOT_PROMOTABLE, "exploration not found")
            })?;
        let checkpoint = store
            .get_exploration_checkpoint(&exploration.checkpoint_id)
            .await
            .map_err(e)?
            .ok_or_else(|| {
                promotion::coded(promotion::ERR_NOT_PROMOTABLE, "checkpoint not found")
            })?;
        let archive = store
            .get_context_archive(&checkpoint.context_archive_id)
            .await
            .map_err(e)?;
        let _exclusive = self
            .0
            .project_locks
            .project(&checkpoint.project_id)
            .try_write_owned()
            .map_err(|_| "ProjectBusy: another project operation is still active".to_string())?;
        if self.running(&exploration.frame_id) {
            return Err(promotion::coded(
                promotion::ERR_EXPLORATION_BUSY,
                "wait for exploration turns to finish before discarding",
            ));
        }
        if store
            .exploration_has_active_runs(exploration_id)
            .await
            .map_err(e)?
        {
            return Err(promotion::coded(
                promotion::ERR_EXPLORATION_BUSY,
                "finish or cancel exploration Runs before discarding",
            ));
        }
        if !matches!(
            exploration.status,
            ExplorationStatus::Active | ExplorationStatus::Failed
        ) {
            return Err(promotion::coded(
                promotion::ERR_NOT_PROMOTABLE,
                "only an active or failed exploration can be discarded",
            ));
        }
        let data = self.exploration_data();
        let snapshot = PersistentExplorationWorkspace::new(data.clone())
            .load_snapshot(&checkpoint.workspace_snapshot_id)?;
        let root = validated_exploration_root(&exploration, &snapshot, &data)?;
        let workspace = MaterializedWorkspace {
            exploration_id: exploration.id.clone(),
            project_key: snapshot.project_key.clone(),
            snapshot_id: snapshot.id.clone(),
            root,
        };
        self.drop_live(&exploration.frame_id);
        store
            .discard_exploration_scope(exploration_id)
            .await
            .map_err(e)?;
        if let Err(error) = PersistentExplorationWorkspace::new(data.clone())
            .dispose(&workspace)
            .await
        {
            // The records are gone already; startup sweeps what is left.
            tracing::warn!(exploration_id = %exploration_id, %error, "discarded exploration workspace cleanup is incomplete");
        }
        let cleanup = ExplorationPromotionService::new(store.clone(), data);
        cleanup
            .dispose_resolved_round_metadata(std::slice::from_ref(&snapshot))
            .await;
        if let Some(archive) = archive.as_ref() {
            cleanup.dispose_context_archive(archive).await;
        }
        Ok(())
    }

    /// The mainline's folder and the exploration's, for copying files over
    /// by hand when it cannot be kept as it is.
    pub async fn exploration_folders(
        &self,
        project: &str,
        exploration: &str,
    ) -> Result<(PathBuf, PathBuf)> {
        let preview = self.owned_preview(project, exploration).await?;
        anyhow::ensure!(
            preview.eligibility.manual_resolution_available,
            promotion::coded(
                promotion::ERR_NOT_PROMOTABLE,
                "this exploration does not have a file-level manual resolution path",
            )
        );
        ExplorationPromotionService::new(self.0.store.clone(), self.exploration_data())
            .manual_resolution_roots(exploration)
            .await
            .map_err(|error| anyhow!(error))
    }

    /// After files were copied over by hand: the round ends, keeping none
    /// of the explorations' conversations or records.
    pub async fn finish_manual_resolution(&self, project: &str, exploration: &str) -> Result<()> {
        let preview = self.owned_preview(project, exploration).await?;
        anyhow::ensure!(
            preview.eligibility.manual_resolution_available,
            promotion::coded(
                promotion::ERR_NOT_PROMOTABLE,
                "this exploration does not have a file-level manual resolution path",
            )
        );
        let checkpoint = self
            .0
            .store
            .get_exploration_checkpoint(&preview.exploration.checkpoint_id)
            .await?
            .context("checkpoint not found")?;
        self.abandon_round(&checkpoint.source_frame_id)
            .await
            .map(drop)
            .map_err(|error| anyhow!(error))
    }
}
