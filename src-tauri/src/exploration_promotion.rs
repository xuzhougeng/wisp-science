//! Fast-forward-only exploration promotion with a durable filesystem journal.

use crate::exploration_workspace::{
    ExplorationWorkspaceBackend, FileDelta, FileDeltaKind, MaterializedWorkspace,
    PersistentExplorationWorkspace, SnapshotMaterialization, WorkspaceSnapshot,
};
use crate::{project_commands, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tauri::State;
use wisp_store::{
    ArtifactHead, Exploration, ExplorationEffect, ExplorationPromotion, ExplorationPromotionStatus,
    ExplorationStatus, ExternalResource, ResearchEdge, ResearchNode, ResearchNodeKind, RunRecord,
    Store, MAINLINE_SCOPE_KEY,
};

const ERR_MAINLINE_ADVANCED: &str = "MainlineAdvanced";
const ERR_EXPLORATION_BUSY: &str = "ExplorationBusy";
const ERR_NOT_PROMOTABLE: &str = "ExplorationNotPromotable";
const ERR_WORKSPACE_CHANGED: &str = "WorkspaceChangedDuringPromotion";
const ERR_EXTERNAL_REFERENCE_CHANGED: &str = "ExternalReferenceChanged";
const ERR_ROLLBACK_FAILED: &str = "PromotionRollbackFailed";
const JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactDelta {
    pub logical_key: String,
    pub before_artifact_id: Option<String>,
    pub before_version_id: Option<String>,
    pub after_artifact_id: String,
    pub after_version_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainlineChanges {
    pub files: Vec<FileDelta>,
    pub artifact_keys: Vec<String>,
    pub entity_keys: Vec<String>,
    pub source_message_head: i64,
    pub source_ui_event_head: i64,
    pub state_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationDiff {
    pub exploration_id: String,
    pub files: Vec<FileDelta>,
    pub artifacts: Vec<ArtifactDelta>,
    pub runs: Vec<RunRecord>,
    pub decisions: Vec<ResearchNode>,
    pub research_edges: Vec<ResearchEdge>,
    pub external_resources: Vec<ExternalResource>,
    pub external_effects: Vec<ExplorationEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromotionBlocker {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromotionEligibility {
    pub eligible: bool,
    pub code: Option<String>,
    pub reasons: Vec<PromotionBlocker>,
    pub expected_guard_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationPromotionPreview {
    pub exploration: Exploration,
    pub diff: ExplorationDiff,
    pub mainline_changes: MainlineChanges,
    pub eligibility: PromotionEligibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationPromotionResult {
    pub exploration: Exploration,
    pub promotion_id: String,
    pub adopted_frame_id: String,
}

#[derive(Clone)]
struct ExplorationPromotionService {
    store: Store,
    app_data: PathBuf,
}

impl ExplorationPromotionService {
    fn new(store: Store, app_data: PathBuf) -> Self {
        Self { store, app_data }
    }

    fn workspace_backend(&self) -> PersistentExplorationWorkspace {
        PersistentExplorationWorkspace::new(self.app_data.clone())
    }

    async fn preview(&self, exploration_id: &str) -> Result<ExplorationPromotionPreview, String> {
        let exploration = self
            .store
            .get_exploration(exploration_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "exploration not found"))?;
        let checkpoint = self
            .store
            .get_exploration_checkpoint(&exploration.checkpoint_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "checkpoint not found"))?;
        let family = self
            .store
            .get_exploration_family(&checkpoint.family_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "exploration family not found"))?;
        let (_, workspace_dir) = self
            .store
            .get_project(&checkpoint.project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "project not found"))?;
        let project_root = dunce::canonicalize(workspace_dir)
            .map_err(|error| format!("cannot resolve project root: {error}"))?;
        let backend = self.workspace_backend();
        let base = backend.load_snapshot(&checkpoint.workspace_snapshot_id)?;
        let branch_root = validated_exploration_root(&exploration, &base, &self.app_data)?;
        let files = backend.diff(&base, &branch_root).await?;
        let mainline_files = backend.diff(&base, &project_root).await?;

        let baseline_heads = self
            .store
            .list_exploration_baseline_artifact_heads(&checkpoint.id)
            .await
            .map_err(|error| error.to_string())?;
        let branch_heads = self
            .store
            .list_artifact_heads(&checkpoint.project_id, &exploration.id)
            .await
            .map_err(|error| error.to_string())?;
        let current_heads = self
            .store
            .list_artifact_heads(&checkpoint.project_id, MAINLINE_SCOPE_KEY)
            .await
            .map_err(|error| error.to_string())?;
        let baseline_by_key = baseline_heads
            .iter()
            .map(|head| (head.logical_key.as_str(), head))
            .collect::<BTreeMap<_, _>>();
        let artifacts = branch_heads
            .iter()
            .filter_map(|head| {
                let before = baseline_by_key.get(head.logical_key.as_str()).copied();
                if before.is_some_and(|before| {
                    before.artifact_id == head.artifact_id
                        && before.artifact_version_id == head.artifact_version_id
                }) {
                    return None;
                }
                Some(ArtifactDelta {
                    logical_key: head.logical_key.clone(),
                    before_artifact_id: before.map(|before| before.artifact_id.clone()),
                    before_version_id: before.map(|before| before.artifact_version_id.clone()),
                    after_artifact_id: head.artifact_id.clone(),
                    after_version_id: head.artifact_version_id.clone(),
                })
            })
            .collect::<Vec<_>>();
        let artifact_keys = changed_artifact_keys(&baseline_heads, &current_heads);

        let baseline_entities = self
            .store
            .list_exploration_baseline_entities(&checkpoint.id)
            .await
            .map_err(|error| error.to_string())?;
        let current_entities = self
            .store
            .snapshot_mainline_entities(&checkpoint.project_id)
            .await
            .map_err(|error| error.to_string())?;
        let entity_keys = changed_entity_keys(&baseline_entities, &current_entities);
        let runs = self
            .store
            .list_runs_owned_by_exploration(&exploration.id)
            .await
            .map_err(|error| error.to_string())?;
        let graph = self
            .store
            .research_graph_owned_by_exploration(&exploration.id)
            .await
            .map_err(|error| error.to_string())?;
        let decisions = graph
            .nodes
            .into_iter()
            .filter(|node| node.kind == ResearchNodeKind::Decision)
            .collect::<Vec<_>>();
        let external_resources = self
            .store
            .list_external_resources_owned_by_exploration(&exploration.id)
            .await
            .map_err(|error| error.to_string())?;
        let external_effects = self
            .store
            .list_exploration_effects(&exploration.id)
            .await
            .map_err(|error| error.to_string())?;
        let source_message_head = self
            .store
            .frame_message_head(&checkpoint.source_frame_id)
            .await
            .map_err(|error| error.to_string())?;
        let source_ui_event_head = self
            .store
            .frame_ui_event_head(&checkpoint.source_frame_id)
            .await
            .map_err(|error| error.to_string())?;
        let state_generation = self
            .store
            .project_state_generation(&checkpoint.project_id)
            .await
            .map_err(|error| error.to_string())?;
        let diff = ExplorationDiff {
            exploration_id: exploration.id.clone(),
            files,
            artifacts,
            runs,
            decisions,
            research_edges: graph.edges,
            external_resources,
            external_effects,
        };
        let mainline_changes = MainlineChanges {
            files: mainline_files,
            artifact_keys,
            entity_keys,
            source_message_head,
            source_ui_event_head,
            state_generation,
        };

        let mut reasons = Vec::new();
        if exploration.status != ExplorationStatus::Active {
            reasons.push(PromotionBlocker {
                code: ERR_NOT_PROMOTABLE.into(),
                message: "Only an active exploration can be promoted.".into(),
            });
        }
        let mainline_advanced = family.mainline_frame_id != checkpoint.source_frame_id
            || family.generation != checkpoint.source_family_generation
            || source_message_head != checkpoint.source_frame_head_seq
            || source_ui_event_head != checkpoint.source_ui_event_seq
            || state_generation != checkpoint.source_state_generation
            || !mainline_changes.files.is_empty()
            || !mainline_changes.artifact_keys.is_empty()
            || !mainline_changes.entity_keys.is_empty();
        if mainline_advanced {
            reasons.push(PromotionBlocker {
                code: ERR_MAINLINE_ADVANCED.into(),
                message: "The mainline no longer matches this exploration checkpoint.".into(),
            });
        }
        if has_changed_external_reference(&diff.files)
            || has_changed_external_reference(&mainline_changes.files)
        {
            reasons.push(PromotionBlocker {
                code: ERR_EXTERNAL_REFERENCE_CHANGED.into(),
                message: "A referenced or unsupported file changed and cannot be promoted safely."
                    .into(),
            });
        }
        if diff.runs.iter().any(|run| !run.status.is_terminal()) {
            reasons.push(PromotionBlocker {
                code: ERR_EXPLORATION_BUSY.into(),
                message: "Finish or cancel exploration Runs before promotion.".into(),
            });
        }
        let expected_guard_hash = hash_json(&serde_json::json!({
            "checkpoint_id": checkpoint.id,
            "checkpoint_guard": checkpoint.guard_hash,
            "family_mainline": family.mainline_frame_id,
            "family_generation": family.generation,
            "mainline": &mainline_changes,
            "current_artifact_heads": stable_heads(&current_heads),
            "current_entities": &current_entities,
            "exploration_status": exploration.status,
            "exploration_generation": exploration.scope_generation,
            "diff": &diff,
        }))?;
        Ok(ExplorationPromotionPreview {
            exploration,
            diff,
            mainline_changes,
            eligibility: PromotionEligibility {
                eligible: reasons.is_empty(),
                code: reasons.first().map(|reason| reason.code.clone()),
                reasons,
                expected_guard_hash,
            },
        })
    }

    async fn promote_locked(
        &self,
        exploration_id: &str,
        expected_guard_hash: &str,
    ) -> Result<ExplorationPromotionResult, String> {
        let preview = self.preview(exploration_id).await?;
        if !preview.eligibility.eligible {
            return Err(coded(
                preview
                    .eligibility
                    .code
                    .as_deref()
                    .unwrap_or(ERR_NOT_PROMOTABLE),
                "exploration is not eligible for fast-forward promotion",
            ));
        }
        if preview.eligibility.expected_guard_hash != expected_guard_hash {
            return Err(coded(
                ERR_WORKSPACE_CHANGED,
                "the promotion preview is stale; preview the exploration again",
            ));
        }
        let checkpoint = self
            .store
            .get_exploration_checkpoint(&preview.exploration.checkpoint_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "checkpoint not found"))?;
        let (_, workspace_dir) = self
            .store
            .get_project(&checkpoint.project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "project not found"))?;
        let project_root = dunce::canonicalize(workspace_dir)
            .map_err(|error| format!("cannot resolve project root: {error}"))?;
        let promotion_id = uuid::Uuid::new_v4().to_string();
        let mut journal = match PromotionJournal::prepare(
            &self.app_data,
            &promotion_id,
            &checkpoint.project_id,
            &preview.exploration,
            &project_root,
            &preview.diff.files,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                if let Ok(root) = promotion_storage_root(&self.app_data) {
                    let _ = std::fs::remove_dir_all(root.join(&promotion_id));
                }
                return Err(error);
            }
        };
        let journal_relative = journal_relative_path(&promotion_id);
        self.store
            .create_exploration_promotion(&ExplorationPromotion {
                id: promotion_id.clone(),
                exploration_id: preview.exploration.id.clone(),
                expected_guard_hash: expected_guard_hash.to_string(),
                status: ExplorationPromotionStatus::Prepared,
                diff_json: serde_json::to_string(&preview.diff)
                    .map_err(|error| error.to_string())?,
                journal_path: Some(path_to_slash(&journal_relative)),
                error: None,
                started_at: chrono::Utc::now().timestamp(),
                committed_at: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        if !self
            .store
            .transition_exploration(
                &preview.exploration.id,
                ExplorationStatus::Active,
                ExplorationStatus::Promoting,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            journal.cleanup_unapplied()?;
            let _ = self
                .store
                .transition_exploration_promotion(
                    &promotion_id,
                    ExplorationPromotionStatus::Prepared,
                    ExplorationPromotionStatus::Failed,
                    Some("exploration status changed before promotion"),
                )
                .await;
            return Err(coded(
                ERR_NOT_PROMOTABLE,
                "exploration status changed before promotion",
            ));
        }

        if let Err(error) = journal.apply() {
            return Err(self
                .rollback_failed_promotion(
                    &promotion_id,
                    &preview.exploration.id,
                    ExplorationPromotionStatus::Prepared,
                    &mut journal,
                    &error,
                )
                .await);
        }
        if !self
            .store
            .transition_exploration_promotion(
                &promotion_id,
                ExplorationPromotionStatus::Prepared,
                ExplorationPromotionStatus::FilesApplied,
                None,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(self
                .rollback_failed_promotion(
                    &promotion_id,
                    &preview.exploration.id,
                    ExplorationPromotionStatus::Prepared,
                    &mut journal,
                    "promotion status changed after file application",
                )
                .await);
        }
        if let Err(error) = self
            .store
            .commit_exploration_promotion_metadata(&promotion_id)
            .await
        {
            return Err(self
                .rollback_failed_promotion(
                    &promotion_id,
                    &preview.exploration.id,
                    ExplorationPromotionStatus::FilesApplied,
                    &mut journal,
                    &error.to_string(),
                )
                .await);
        }
        if !self
            .store
            .transition_exploration_promotion(
                &promotion_id,
                ExplorationPromotionStatus::MetadataCommitted,
                ExplorationPromotionStatus::Committed,
                None,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("promotion metadata committed but final status update was lost".into());
        }
        if let Err(error) = journal.finish_commit() {
            tracing::warn!(promotion_id = %promotion_id, %error, "promotion committed but journal cleanup is incomplete");
        }
        let exploration = self
            .store
            .get_exploration(&preview.exploration.id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "promoted exploration disappeared".to_string())?;
        Ok(ExplorationPromotionResult {
            adopted_frame_id: exploration.frame_id.clone(),
            exploration,
            promotion_id,
        })
    }

    async fn rollback_failed_promotion(
        &self,
        promotion_id: &str,
        exploration_id: &str,
        status: ExplorationPromotionStatus,
        journal: &mut PromotionJournal,
        cause: &str,
    ) -> String {
        match journal.rollback() {
            Ok(()) => {
                let _ = self
                    .store
                    .transition_exploration_promotion(
                        promotion_id,
                        status,
                        ExplorationPromotionStatus::RolledBack,
                        Some(cause),
                    )
                    .await;
                let _ = self
                    .store
                    .transition_exploration(
                        exploration_id,
                        ExplorationStatus::Promoting,
                        ExplorationStatus::Active,
                    )
                    .await;
                coded(ERR_WORKSPACE_CHANGED, cause)
            }
            Err(rollback_error) => {
                let combined = format!("{cause}; rollback failed: {rollback_error}");
                let _ = self
                    .store
                    .transition_exploration_promotion(
                        promotion_id,
                        status,
                        ExplorationPromotionStatus::Failed,
                        Some(&combined),
                    )
                    .await;
                let _ = self
                    .store
                    .transition_exploration(
                        exploration_id,
                        ExplorationStatus::Promoting,
                        ExplorationStatus::Failed,
                    )
                    .await;
                coded(ERR_ROLLBACK_FAILED, &combined)
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn preview_exploration_promotion(
    state: State<'_, AppState>,
    exploration_id: String,
) -> Result<ExplorationPromotionPreview, String> {
    ExplorationPromotionService::new(state.store.clone(), state.app_data.clone())
        .preview(&exploration_id)
        .await
}

#[tauri::command]
pub(crate) async fn promote_exploration(
    state: State<'_, AppState>,
    terminals: State<'_, crate::terminal_sessions::TerminalManager>,
    window: tauri::WebviewWindow,
    exploration_id: String,
    expected_guard_hash: String,
) -> Result<ExplorationPromotionResult, String> {
    let exploration = state
        .store
        .get_exploration(&exploration_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "exploration not found"))?;
    let checkpoint = state
        .store
        .get_exploration_checkpoint(&exploration.checkpoint_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "checkpoint not found"))?;
    let _exclusive = state.begin_project_exclusive_activity(&checkpoint.project_id)?;
    if terminals.has_running(&checkpoint.project_id, MAINLINE_SCOPE_KEY)
        || terminals.has_running(&checkpoint.project_id, &exploration_id)
    {
        return Err(coded(
            ERR_EXPLORATION_BUSY,
            "close mainline and exploration terminals before promotion",
        ));
    }
    ensure_no_queued_turns(
        state.inner(),
        [&checkpoint.source_frame_id, &exploration.frame_id],
    )
    .await?;
    if state
        .store
        .project_has_active_runs(&checkpoint.project_id)
        .await
        .map_err(|error| error.to_string())?
        || state
            .store
            .exploration_has_active_runs(&exploration_id)
            .await
            .map_err(|error| error.to_string())?
    {
        return Err(coded(
            ERR_EXPLORATION_BUSY,
            "active mainline or exploration Runs block promotion",
        ));
    }
    let service = ExplorationPromotionService::new(state.store.clone(), state.app_data.clone());
    let result = service
        .promote_locked(&exploration_id, &expected_guard_hash)
        .await?;

    state
        .runtime_manager
        .stop_scope(&checkpoint.project_id, MAINLINE_SCOPE_KEY)
        .await;
    state
        .runtime_manager
        .stop_scope(&checkpoint.project_id, &exploration_id)
        .await;
    {
        let mut sessions = state.sessions.lock().await;
        sessions.remove(&checkpoint.source_frame_id);
        sessions.remove(&result.adopted_frame_id);
    }
    let (project, _, _) =
        project_commands::load_active_project(&state, &checkpoint.project_id).await?;
    state.set_active(window.label(), project);
    state.set_active_frame(window.label(), Some(result.adopted_frame_id.clone()));
    Ok(result)
}

#[tauri::command]
pub(crate) async fn discard_exploration(
    state: State<'_, AppState>,
    terminals: State<'_, crate::terminal_sessions::TerminalManager>,
    window: tauri::WebviewWindow,
    exploration_id: String,
) -> Result<Exploration, String> {
    let exploration = state
        .store
        .get_exploration(&exploration_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "exploration not found"))?;
    let checkpoint = state
        .store
        .get_exploration_checkpoint(&exploration.checkpoint_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| coded(ERR_NOT_PROMOTABLE, "checkpoint not found"))?;
    let _exclusive = state.begin_project_exclusive_activity(&checkpoint.project_id)?;
    if terminals.has_running(&checkpoint.project_id, &exploration_id) {
        return Err(coded(
            ERR_EXPLORATION_BUSY,
            "close the exploration terminal before discarding",
        ));
    }
    ensure_no_queued_turns(state.inner(), [&exploration.frame_id]).await?;
    if state
        .store
        .exploration_has_active_runs(&exploration_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(coded(
            ERR_EXPLORATION_BUSY,
            "finish or cancel exploration Runs before discarding",
        ));
    }
    if !matches!(
        exploration.status,
        ExplorationStatus::Active | ExplorationStatus::Archived
    ) {
        return Err(coded(
            ERR_NOT_PROMOTABLE,
            "only an active or archived exploration can be discarded",
        ));
    }
    let snapshot = PersistentExplorationWorkspace::new(state.app_data.clone())
        .load_snapshot(&checkpoint.workspace_snapshot_id)?;
    let validated_root = validated_exploration_root(&exploration, &snapshot, &state.app_data)?;
    let workspace = MaterializedWorkspace {
        exploration_id: exploration.id.clone(),
        project_key: snapshot.project_key,
        snapshot_id: snapshot.id,
        root: validated_root,
    };
    let expected = exploration.status;
    if !state
        .store
        .transition_exploration(&exploration_id, expected, ExplorationStatus::Discarded)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(coded(
            ERR_NOT_PROMOTABLE,
            "exploration status changed before discard",
        ));
    }
    state
        .runtime_manager
        .stop_scope(&checkpoint.project_id, &exploration_id)
        .await;
    state.sessions.lock().await.remove(&exploration.frame_id);
    PersistentExplorationWorkspace::new(state.app_data.clone())
        .dispose(&workspace)
        .await?;
    if state.active_frame(window.label()).as_deref() == Some(exploration.frame_id.as_str()) {
        let (project, _, _) =
            project_commands::load_active_project(&state, &checkpoint.project_id).await?;
        state.set_active(window.label(), project);
        state.set_active_frame(window.label(), Some(checkpoint.source_frame_id));
    }
    state
        .store
        .get_exploration(&exploration_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "discarded exploration disappeared".to_string())
}

pub(crate) async fn recover_incomplete_promotions(store: &Store, app_data: &Path) {
    let promotions = match store.list_incomplete_exploration_promotions().await {
        Ok(promotions) => promotions,
        Err(error) => {
            tracing::error!(%error, "cannot list incomplete exploration promotions");
            return;
        }
    };
    for promotion in promotions {
        let Some(relative) = promotion.journal_path.as_deref() else {
            tracing::error!(promotion_id = %promotion.id, "promotion journal path is missing");
            continue;
        };
        let journal_path = match safe_app_data_path(app_data, relative) {
            Ok(path) => path,
            Err(error) => {
                tracing::error!(promotion_id = %promotion.id, %error, "unsafe promotion journal path");
                continue;
            }
        };
        if promotion.status == ExplorationPromotionStatus::MetadataCommitted
            && !journal_path.is_file()
            && !journal_path.with_file_name("journal.prev").is_file()
        {
            let _ = store
                .transition_exploration_promotion(
                    &promotion.id,
                    ExplorationPromotionStatus::MetadataCommitted,
                    ExplorationPromotionStatus::Committed,
                    Some("journal cleanup completed before restart"),
                )
                .await;
            continue;
        }
        let mut journal = match PromotionJournal::load(&journal_path, store).await {
            Ok(journal) => journal,
            Err(error) => {
                tracing::error!(promotion_id = %promotion.id, %error, "cannot load promotion journal");
                continue;
            }
        };
        if journal.promotion_id != promotion.id
            || journal.exploration_id != promotion.exploration_id
        {
            tracing::error!(promotion_id = %promotion.id, "promotion journal ownership mismatch");
            continue;
        }
        match promotion.status {
            ExplorationPromotionStatus::Prepared | ExplorationPromotionStatus::FilesApplied => {
                match journal.rollback() {
                    Ok(()) => {
                        let _ = store
                            .transition_exploration_promotion(
                                &promotion.id,
                                promotion.status,
                                ExplorationPromotionStatus::RolledBack,
                                Some("recovered after interrupted promotion"),
                            )
                            .await;
                        let _ = store
                            .transition_exploration(
                                &promotion.exploration_id,
                                ExplorationStatus::Promoting,
                                ExplorationStatus::Active,
                            )
                            .await;
                    }
                    Err(error) => tracing::error!(
                        promotion_id = %promotion.id,
                        %error,
                        "promotion rollback requires manual recovery"
                    ),
                }
            }
            ExplorationPromotionStatus::MetadataCommitted => {
                let _ = store
                    .transition_exploration_promotion(
                        &promotion.id,
                        ExplorationPromotionStatus::MetadataCommitted,
                        ExplorationPromotionStatus::Committed,
                        None,
                    )
                    .await;
                if let Err(error) = journal.finish_commit() {
                    tracing::warn!(promotion_id = %promotion.id, %error, "promotion recovered but journal cleanup is incomplete");
                }
            }
            _ => {}
        }
    }
}

pub(crate) async fn ensure_no_queued_turns<'a>(
    state: &AppState,
    frame_ids: impl IntoIterator<Item = &'a String>,
) -> Result<(), String> {
    let sessions = state.sessions.lock().await;
    for frame_id in frame_ids {
        if let Some(runtime) = sessions.get(frame_id) {
            if runtime.control.follow_ups().driver_active()
                || !runtime.control.follow_ups().is_empty()
            {
                return Err(coded(
                    ERR_EXPLORATION_BUSY,
                    "queued conversation turns block this exploration operation",
                ));
            }
        }
    }
    Ok(())
}

fn changed_artifact_keys(
    baseline: &[wisp_store::ExplorationBaselineArtifactHead],
    current: &[ArtifactHead],
) -> Vec<String> {
    let before = baseline
        .iter()
        .map(|head| {
            (
                head.logical_key.clone(),
                (head.artifact_id.clone(), head.artifact_version_id.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let after = current
        .iter()
        .map(|head| {
            (
                head.logical_key.clone(),
                (head.artifact_id.clone(), head.artifact_version_id.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

fn changed_entity_keys(
    baseline: &[wisp_store::ExplorationBaselineEntity],
    current: &[wisp_store::ExplorationBaselineEntity],
) -> Vec<String> {
    let before = baseline
        .iter()
        .map(|entity| {
            (
                format!("{}:{}", entity.entity_kind, entity.entity_id),
                entity.fingerprint.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let after = current
        .iter()
        .map(|entity| {
            (
                format!("{}:{}", entity.entity_kind, entity.entity_id),
                entity.fingerprint.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

fn stable_heads(heads: &[ArtifactHead]) -> Vec<(&str, &str, &str)> {
    heads
        .iter()
        .map(|head| {
            (
                head.logical_key.as_str(),
                head.artifact_id.as_str(),
                head.artifact_version_id.as_str(),
            )
        })
        .collect()
}

fn has_changed_external_reference(files: &[FileDelta]) -> bool {
    files.iter().any(|delta| {
        delta
            .before
            .iter()
            .chain(delta.after.iter())
            .any(|entry| entry.materialization != SnapshotMaterialization::Blob)
    })
}

fn validated_exploration_root(
    exploration: &Exploration,
    snapshot: &WorkspaceSnapshot,
    app_data: &Path,
) -> Result<PathBuf, String> {
    let expected = app_data
        .join("explorations")
        .join(&snapshot.project_key)
        .join(&exploration.id)
        .join("workspace");
    let stored = PathBuf::from(&exploration.workspace_dir);
    if stored != expected {
        return Err("exploration workspace is outside its canonical storage root".into());
    }
    let canonical = dunce::canonicalize(&stored)
        .map_err(|error| format!("cannot resolve exploration workspace: {error}"))?;
    let canonical_expected = dunce::canonicalize(&expected)
        .map_err(|error| format!("cannot resolve expected exploration workspace: {error}"))?;
    if canonical != canonical_expected {
        return Err("exploration workspace canonical path mismatch".into());
    }
    Ok(canonical)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromotionJournal {
    schema_version: u32,
    promotion_id: String,
    project_id: String,
    exploration_id: String,
    project_root: PathBuf,
    branch_root: PathBuf,
    journal_dir: PathBuf,
    entries: Vec<JournalEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalEntry {
    path: String,
    kind: FileDeltaKind,
    before_checksum: Option<String>,
    before_size: Option<u64>,
    after_checksum: Option<String>,
    after_size: Option<u64>,
    after_executable: bool,
    applied: bool,
}

impl PromotionJournal {
    fn prepare(
        app_data: &Path,
        promotion_id: &str,
        project_id: &str,
        exploration: &Exploration,
        project_root: &Path,
        deltas: &[FileDelta],
    ) -> Result<Self, String> {
        validate_component(promotion_id)?;
        let journal_dir = promotion_storage_root(app_data)?.join(promotion_id);
        if journal_dir.exists() {
            return Err("promotion journal already exists".into());
        }
        std::fs::create_dir(&journal_dir).map_err(|error| error.to_string())?;
        std::fs::create_dir(journal_dir.join("staging")).map_err(|error| error.to_string())?;
        let branch_root =
            dunce::canonicalize(&exploration.workspace_dir).map_err(|error| error.to_string())?;
        let mut entries = Vec::new();
        for (index, delta) in deltas.iter().enumerate() {
            validate_relative_path(&delta.path)?;
            if delta
                .before
                .iter()
                .chain(delta.after.iter())
                .any(|entry| entry.materialization != SnapshotMaterialization::Blob)
            {
                return Err(coded(
                    ERR_EXTERNAL_REFERENCE_CHANGED,
                    "promotion includes an unmaterialized file reference",
                ));
            }
            let after_executable = delta.after.as_ref().is_some_and(|entry| entry.executable);
            let entry = JournalEntry {
                path: delta.path.clone(),
                kind: delta.kind.clone(),
                before_checksum: delta
                    .before
                    .as_ref()
                    .and_then(|entry| entry.checksum.clone()),
                before_size: delta.before.as_ref().map(|entry| entry.size_bytes),
                after_checksum: delta
                    .after
                    .as_ref()
                    .and_then(|entry| entry.checksum.clone()),
                after_size: delta.after.as_ref().map(|entry| entry.size_bytes),
                after_executable,
                applied: false,
            };
            if entry.after_checksum.is_some() {
                let source = safe_join(&branch_root, &entry.path)?;
                verify_regular_file(
                    &source,
                    entry.after_size.unwrap_or_default(),
                    entry.after_checksum.as_deref().unwrap_or_default(),
                )?;
                copy_new_file(
                    &source,
                    &journal_dir.join("staging").join(index.to_string()),
                )?;
            }
            entries.push(entry);
        }
        let journal = Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            promotion_id: promotion_id.to_string(),
            project_id: project_id.to_string(),
            exploration_id: exploration.id.clone(),
            project_root: project_root.to_path_buf(),
            branch_root,
            journal_dir,
            entries,
        };
        journal.persist()?;
        Ok(journal)
    }

    async fn load(path: &Path, store: &Store) -> Result<Self, String> {
        let previous = path.with_file_name("journal.prev");
        let source = if path.is_file() {
            path
        } else if previous.is_file() {
            previous.as_path()
        } else {
            return Err("promotion journal and fallback are missing".into());
        };
        let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
        let journal: Self = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if journal.schema_version != JOURNAL_SCHEMA_VERSION
            || journal.journal_path() != path
            || journal.journal_dir.parent().and_then(Path::file_name)
                != Some(std::ffi::OsStr::new("exploration-promotions"))
        {
            return Err("promotion journal identity is invalid".into());
        }
        let (_, workspace) = store
            .get_project(&journal.project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "promotion journal project no longer exists".to_string())?;
        let canonical = dunce::canonicalize(workspace).map_err(|error| error.to_string())?;
        if canonical != journal.project_root {
            return Err("promotion journal project root changed".into());
        }
        Ok(journal)
    }

    fn journal_path(&self) -> PathBuf {
        self.journal_dir.join("journal.json")
    }

    fn persist(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.journal_dir).map_err(|error| error.to_string())?;
        let temporary = self
            .journal_dir
            .join(format!(".journal-{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        output
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())?;
        let destination = self.journal_path();
        if !destination.exists() {
            return std::fs::rename(&temporary, destination).map_err(|error| error.to_string());
        }
        let previous = self.journal_dir.join("journal.prev");
        if previous.exists() {
            std::fs::remove_file(&previous).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&destination, &previous).map_err(|error| error.to_string())?;
        if let Err(error) = std::fs::rename(&temporary, &destination) {
            let _ = std::fs::rename(&previous, &destination);
            return Err(error.to_string());
        }
        std::fs::remove_file(previous).map_err(|error| error.to_string())
    }

    fn apply(&mut self) -> Result<(), String> {
        for index in 0..self.entries.len() {
            self.apply_entry(index)?;
            self.entries[index].applied = true;
            self.persist()?;
        }
        Ok(())
    }

    fn apply_entry(&self, index: usize) -> Result<(), String> {
        let entry = &self.entries[index];
        let target = safe_join(&self.project_root, &entry.path)?;
        let parent = target
            .parent()
            .ok_or_else(|| "promotion target has no parent".to_string())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.adjacent_path(index, "staging")?;
        let backup = self.adjacent_path(index, "backup")?;
        if temporary.exists() || backup.exists() {
            return Err("promotion found a stale adjacent journal file".into());
        }
        match entry.kind {
            FileDeltaKind::Added => {
                if target.exists() {
                    return Err(format!("promotion target appeared: {}", entry.path));
                }
            }
            FileDeltaKind::Modified | FileDeltaKind::Deleted => verify_regular_file(
                &target,
                entry.before_size.unwrap_or_default(),
                entry.before_checksum.as_deref().unwrap_or_default(),
            )?,
        }
        if entry.after_checksum.is_some() {
            verify_regular_file(
                &safe_join(&self.branch_root, &entry.path)?,
                entry.after_size.unwrap_or_default(),
                entry.after_checksum.as_deref().unwrap_or_default(),
            )?;
            copy_new_file(
                &self.journal_dir.join("staging").join(index.to_string()),
                &temporary,
            )?;
            verify_regular_file(
                &temporary,
                entry.after_size.unwrap_or_default(),
                entry.after_checksum.as_deref().unwrap_or_default(),
            )?;
        }
        if matches!(entry.kind, FileDeltaKind::Modified | FileDeltaKind::Deleted) {
            std::fs::rename(&target, &backup).map_err(|error| {
                format!("cannot move {} into promotion backup: {error}", entry.path)
            })?;
        }
        if entry.after_checksum.is_some() {
            if let Err(error) = std::fs::rename(&temporary, &target) {
                if backup.exists() && !target.exists() {
                    let _ = std::fs::rename(&backup, &target);
                }
                return Err(format!("cannot replace {}: {error}", entry.path));
            }
            set_executable(&target, entry.after_executable)?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), String> {
        for index in (0..self.entries.len()).rev() {
            let entry = &self.entries[index];
            let target = safe_join(&self.project_root, &entry.path)?;
            let temporary = self.adjacent_path(index, "staging")?;
            let backup = self.adjacent_path(index, "backup")?;
            if backup.exists() {
                if target.exists() {
                    ensure_promoted_target(&target, entry)?;
                    std::fs::remove_file(&target).map_err(|error| error.to_string())?;
                }
                std::fs::rename(&backup, &target).map_err(|error| error.to_string())?;
            } else if matches!(entry.kind, FileDeltaKind::Added) && target.exists() {
                ensure_promoted_target(&target, entry)?;
                std::fs::remove_file(&target).map_err(|error| error.to_string())?;
            }
            if temporary.exists() {
                std::fs::remove_file(&temporary).map_err(|error| error.to_string())?;
            }
            self.entries[index].applied = false;
        }
        self.persist()?;
        self.cleanup_unapplied()
    }

    fn cleanup_unapplied(&mut self) -> Result<(), String> {
        if self.journal_dir.exists() {
            std::fs::remove_dir_all(&self.journal_dir).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn finish_commit(&mut self) -> Result<(), String> {
        for index in 0..self.entries.len() {
            for suffix in ["staging", "backup"] {
                let path = self.adjacent_path(index, suffix)?;
                if path.exists() {
                    std::fs::remove_file(path).map_err(|error| error.to_string())?;
                }
            }
        }
        self.cleanup_unapplied()
    }

    fn adjacent_path(&self, index: usize, suffix: &str) -> Result<PathBuf, String> {
        let target = safe_join(&self.project_root, &self.entries[index].path)?;
        let parent = target
            .parent()
            .ok_or_else(|| "promotion target has no parent".to_string())?;
        Ok(parent.join(format!(
            ".wisp-promotion-{}-{index}.{suffix}",
            self.promotion_id
        )))
    }
}

fn ensure_promoted_target(path: &Path, entry: &JournalEntry) -> Result<(), String> {
    let Some(checksum) = entry.after_checksum.as_deref() else {
        return Err(format!(
            "unexpected target appeared while rolling back {}",
            entry.path
        ));
    };
    verify_regular_file(path, entry.after_size.unwrap_or_default(), checksum)
}

fn verify_regular_file(path: &Path, expected_size: u64, expected_hash: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_size {
        return Err(format!("file identity changed: {}", path.display()));
    }
    let actual = hash_file(path)?;
    if actual != expected_hash {
        return Err(format!("file checksum changed: {}", path.display()));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut input = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<(), String> {
    let mut input = File::open(source).map_err(|error| error.to_string())?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| error.to_string())?;
    std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o111;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    Ok(root.join(relative))
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return Err(format!("unsafe promotion path: {path}"));
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe promotion path: {path}"));
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("invalid promotion storage component".into());
    }
    Ok(())
}

fn journal_relative_path(promotion_id: &str) -> PathBuf {
    PathBuf::from("exploration-promotions")
        .join(promotion_id)
        .join("journal.json")
}

fn safe_app_data_path(app_data: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("promotion journal escaped app data".into());
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.len() != 3
        || components[0] != "exploration-promotions"
        || components[2] != "journal.json"
    {
        return Err("promotion journal path has an invalid layout".into());
    }
    validate_component(components[1])?;
    let container = promotion_storage_root(app_data)?.join(components[1]);
    if container.exists() {
        let metadata = std::fs::symlink_metadata(&container).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("promotion journal container is not a real directory".into());
        }
    }
    Ok(container.join("journal.json"))
}

fn promotion_storage_root(app_data: &Path) -> Result<PathBuf, String> {
    let root = app_data.join("exploration-promotions");
    if !root.exists() {
        std::fs::create_dir(&root).map_err(|error| error.to_string())?;
    }
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("promotion journal root is not a real directory".into());
    }
    Ok(root)
}

fn path_to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn hash_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .map_err(|error| error.to_string())
}

fn coded(code: &str, message: &str) -> String {
    format!("{code}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exploration_commands::ExplorationService;
    use wisp_store::{
        scoped_logical_artifact_id, ArtifactCaptureTiming, ArtifactMaterialization,
        ArtifactVersionDraft, ExternalResource, ResearchNode, RunRecord, RunStatus, StateScope,
    };

    async fn fixture(label: &str) -> (ExplorationService, Store, PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "wisp_exploration_promotion_{label}_{}",
            uuid::Uuid::new_v4()
        ));
        let project = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(project.join("baseline.txt"), b"baseline").unwrap();
        std::fs::write(project.join("remove.txt"), b"remove me").unwrap();
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
        let service = ExplorationService::new(store.clone(), app_data.clone());
        (service, store, base, project, app_data)
    }

    async fn baseline_artifact(store: &Store) -> (String, String) {
        let logical_key = "path:baseline.txt";
        let artifact_id = wisp_store::logical_artifact_id("p", logical_key);
        let version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("baseline-version".into()),
                artifact_id: artifact_id.clone(),
                project_id: "p".into(),
                root_frame_id: "main".into(),
                filename: "baseline.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "baseline.txt".into(),
                logical_key: Some(logical_key.into()),
                size_bytes: Some(8),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        (artifact_id, version_id)
    }

    async fn branch_artifact(store: &Store, exploration: &Exploration, version_id: &str) -> String {
        let logical_key = "path:baseline.txt";
        let artifact_id = scoped_logical_artifact_id("p", &exploration.id, logical_key);
        store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some(version_id.into()),
                artifact_id: artifact_id.clone(),
                project_id: "p".into(),
                root_frame_id: exploration.frame_id.clone(),
                filename: "baseline.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "baseline.txt".into(),
                logical_key: Some(logical_key.into()),
                size_bytes: Some(14),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        artifact_id
    }

    #[tokio::test]
    async fn promotion_adopts_one_complete_scope_and_archives_sibling() {
        let (creator, store, base, project, app_data) = fixture("adopt").await;
        baseline_artifact(&store).await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let selected = creator
            .create_exploration(&checkpoint.id, "Selected")
            .await
            .unwrap();
        let sibling = creator
            .create_exploration(&checkpoint.id, "Sibling")
            .await
            .unwrap();
        let selected_root = PathBuf::from(&selected.workspace_dir);
        std::fs::write(selected_root.join("baseline.txt"), b"selected value").unwrap();
        std::fs::write(selected_root.join("added.txt"), b"new result").unwrap();
        std::fs::remove_file(selected_root.join("remove.txt")).unwrap();
        let selected_artifact_id = branch_artifact(&store, &selected, "selected-version").await;

        let mut run = RunRecord::new("selected-run", "p", "local", "Selected run", "command");
        run.frame_id = Some(selected.frame_id.clone());
        run.status = RunStatus::Succeeded;
        store.create_run(&run).await.unwrap();
        let mut decision = ResearchNode::new(
            "selected-decision",
            "p",
            ResearchNodeKind::Decision,
            "Use selected normalization",
        )
        .unwrap();
        decision.metadata_json = r#"{"reason":"better residuals"}"#.into();
        store
            .save_research_node_in_scope(
                &decision,
                &StateScope::exploration("p", selected.id.clone()),
            )
            .await
            .unwrap();
        store
            .save_external_resource_in_scope(
                &ExternalResource {
                    id: "selected-resource".into(),
                    project_id: "p".into(),
                    kind: "dataset".into(),
                    uri: "doi:10.0000/selected".into(),
                    version: Some("v1".into()),
                    checksum: Some("a".repeat(64)),
                    size_bytes: Some(12),
                    license: None,
                    visibility: "restricted".into(),
                    access_instructions: None,
                    accessed_at: Some(1),
                    created_at: 1,
                    updated_at: 1,
                },
                &StateScope::exploration("p", selected.id.clone()),
            )
            .await
            .unwrap();

        let sibling_root = PathBuf::from(&sibling.workspace_dir);
        std::fs::write(sibling_root.join("sibling-only.txt"), b"private").unwrap();
        let sibling_decision = ResearchNode::new(
            "sibling-decision",
            "p",
            ResearchNodeKind::Decision,
            "Keep sibling private",
        )
        .unwrap();
        store
            .save_research_node_in_scope(
                &sibling_decision,
                &StateScope::exploration("p", sibling.id.clone()),
            )
            .await
            .unwrap();

        let promotion = ExplorationPromotionService::new(store.clone(), app_data.clone());
        let preview = promotion.preview(&selected.id).await.unwrap();
        assert!(preview.eligibility.eligible, "{:?}", preview.eligibility);
        assert_eq!(preview.diff.files.len(), 3);
        assert_eq!(preview.diff.artifacts.len(), 1);
        assert_eq!(preview.diff.runs.len(), 1);
        assert_eq!(preview.diff.decisions, vec![decision.clone()]);
        assert_eq!(preview.diff.external_resources.len(), 1);
        let result = promotion
            .promote_locked(&selected.id, &preview.eligibility.expected_guard_hash)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(project.join("baseline.txt")).unwrap(),
            b"selected value"
        );
        assert_eq!(
            std::fs::read(project.join("added.txt")).unwrap(),
            b"new result"
        );
        assert!(!project.join("remove.txt").exists());
        assert!(!project.join("sibling-only.txt").exists());
        let mainline_head = store
            .get_artifact_head("p", MAINLINE_SCOPE_KEY, "path:baseline.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mainline_head.artifact_id, selected_artifact_id);
        assert_eq!(mainline_head.artifact_version_id, "selected-version");
        assert_eq!(
            store
                .list_runs_by_project("p")
                .await
                .unwrap()
                .into_iter()
                .map(|run| run.id)
                .collect::<Vec<_>>(),
            vec!["selected-run".to_string()]
        );
        let graph = store.research_graph("p").await.unwrap();
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.id == "selected-decision"));
        assert!(!graph.nodes.iter().any(|node| node.id == "sibling-decision"));
        assert!(store
            .get_external_resource("selected-resource")
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            store.frame_state_scope(&selected.frame_id).await.unwrap(),
            Some(StateScope::mainline("p"))
        );
        assert_eq!(
            store.frame_state_scope(&sibling.frame_id).await.unwrap(),
            Some(StateScope::exploration("p", sibling.id.clone()))
        );
        assert_eq!(
            store
                .get_exploration(&sibling.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExplorationStatus::Archived
        );
        assert_eq!(
            store
                .get_exploration_family(&checkpoint.family_id)
                .await
                .unwrap()
                .unwrap()
                .mainline_frame_id,
            selected.frame_id
        );
        assert_eq!(
            store
                .get_exploration_promotion(&result.promotion_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExplorationPromotionStatus::Committed
        );
        assert!(!app_data
            .join("exploration-promotions")
            .join(&result.promotion_id)
            .exists());
        store
            .append_message(
                &result.adopted_frame_id,
                3,
                &wisp_llm::Message::user("continue"),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .frame_message_head(&result.adopted_frame_id)
                .await
                .unwrap(),
            3
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn mainline_and_branch_changes_invalidate_the_preview_guard() {
        let (creator, store, base, project, app_data) = fixture("guards").await;
        baseline_artifact(&store).await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Guarded")
            .await
            .unwrap();
        let promotion = ExplorationPromotionService::new(store.clone(), app_data.clone());
        let initial = promotion.preview(&exploration.id).await.unwrap();
        assert!(initial.eligibility.eligible);

        std::fs::write(
            PathBuf::from(&exploration.workspace_dir).join("branch.txt"),
            b"new branch state",
        )
        .unwrap();
        let error = promotion
            .promote_locked(&exploration.id, &initial.eligibility.expected_guard_hash)
            .await
            .unwrap_err();
        assert!(error.starts_with(ERR_WORKSPACE_CHANGED));

        let refreshed = promotion.preview(&exploration.id).await.unwrap();
        assert!(refreshed.eligibility.eligible);
        std::fs::write(project.join("baseline.txt"), b"advanced").unwrap();
        let advanced = promotion.preview(&exploration.id).await.unwrap();
        assert!(!advanced.eligibility.eligible);
        assert_eq!(
            advanced.eligibility.code.as_deref(),
            Some(ERR_MAINLINE_ADVANCED)
        );
        assert!(!advanced.mainline_changes.files.is_empty());
        let error = promotion
            .promote_locked(&exploration.id, &refreshed.eligibility.expected_guard_hash)
            .await
            .unwrap_err();
        assert!(error.starts_with(ERR_MAINLINE_ADVANCED));

        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn source_messages_artifacts_runs_and_decisions_each_advance_mainline() {
        let (creator, store, base, _project, app_data) = fixture("message_guard").await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Message guard")
            .await
            .unwrap();
        store
            .append_message("main", 3, &wisp_llm::Message::user("continued mainline"))
            .await
            .unwrap();
        store
            .append_message("main", 4, &wisp_llm::Message::assistant("done"))
            .await
            .unwrap();
        let preview = ExplorationPromotionService::new(store.clone(), app_data)
            .preview(&exploration.id)
            .await
            .unwrap();
        assert_eq!(
            preview.eligibility.code.as_deref(),
            Some(ERR_MAINLINE_ADVANCED)
        );
        assert_eq!(preview.mainline_changes.source_message_head, 4);
        let _ = std::fs::remove_dir_all(base);

        let (creator, store, base, _project, app_data) = fixture("artifact_guard").await;
        let (artifact_id, _) = baseline_artifact(&store).await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Artifact guard")
            .await
            .unwrap();
        store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("advanced-version".into()),
                artifact_id,
                project_id: "p".into(),
                root_frame_id: "main".into(),
                filename: "baseline.txt".into(),
                content_type: "text/plain".into(),
                storage_path: "baseline.txt".into(),
                logical_key: Some("path:baseline.txt".into()),
                size_bytes: Some(8),
                checksum: None,
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let preview = ExplorationPromotionService::new(store.clone(), app_data)
            .preview(&exploration.id)
            .await
            .unwrap();
        assert_eq!(
            preview.eligibility.code.as_deref(),
            Some(ERR_MAINLINE_ADVANCED)
        );
        assert_eq!(
            preview.mainline_changes.artifact_keys,
            vec!["path:baseline.txt"]
        );
        let _ = std::fs::remove_dir_all(base);

        let (creator, store, base, _project, app_data) = fixture("entity_guard").await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Entity guard")
            .await
            .unwrap();
        let mut run = RunRecord::new("mainline-run", "p", "local", "Mainline run", "command");
        run.frame_id = Some("main".into());
        run.status = RunStatus::Succeeded;
        store.create_run(&run).await.unwrap();
        let decision = ResearchNode::new(
            "mainline-decision",
            "p",
            ResearchNodeKind::Decision,
            "Advance the mainline",
        )
        .unwrap();
        store.save_research_node(&decision).await.unwrap();
        let preview = ExplorationPromotionService::new(store.clone(), app_data)
            .preview(&exploration.id)
            .await
            .unwrap();
        assert_eq!(
            preview.eligibility.code.as_deref(),
            Some(ERR_MAINLINE_ADVANCED)
        );
        assert!(preview
            .mainline_changes
            .entity_keys
            .iter()
            .any(|key| key == "run:mainline-run"));
        assert!(preview
            .mainline_changes
            .entity_keys
            .iter()
            .any(|key| key == "research_node:mainline-decision"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn interrupted_file_application_rolls_back_on_recovery() {
        let (creator, store, base, project, app_data) = fixture("recovery").await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Recover")
            .await
            .unwrap();
        std::fs::write(
            PathBuf::from(&exploration.workspace_dir).join("baseline.txt"),
            b"promoted",
        )
        .unwrap();
        let service = ExplorationPromotionService::new(store.clone(), app_data.clone());
        let preview = service.preview(&exploration.id).await.unwrap();
        let promotion_id = uuid::Uuid::new_v4().to_string();
        let mut journal = PromotionJournal::prepare(
            &app_data,
            &promotion_id,
            "p",
            &exploration,
            &dunce::canonicalize(&project).unwrap(),
            &preview.diff.files,
        )
        .unwrap();
        store
            .create_exploration_promotion(&ExplorationPromotion {
                id: promotion_id.clone(),
                exploration_id: exploration.id.clone(),
                expected_guard_hash: preview.eligibility.expected_guard_hash,
                status: ExplorationPromotionStatus::Prepared,
                diff_json: serde_json::to_string(&preview.diff).unwrap(),
                journal_path: Some(path_to_slash(&journal_relative_path(&promotion_id))),
                error: None,
                started_at: 1,
                committed_at: None,
            })
            .await
            .unwrap();
        store
            .transition_exploration(
                &exploration.id,
                ExplorationStatus::Active,
                ExplorationStatus::Promoting,
            )
            .await
            .unwrap();
        journal.apply().unwrap();
        assert_eq!(
            std::fs::read(project.join("baseline.txt")).unwrap(),
            b"promoted"
        );

        recover_incomplete_promotions(&store, &app_data).await;
        assert_eq!(
            std::fs::read(project.join("baseline.txt")).unwrap(),
            b"baseline"
        );
        assert_eq!(
            store
                .get_exploration_promotion(&promotion_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExplorationPromotionStatus::RolledBack
        );
        assert_eq!(
            store
                .get_exploration(&exploration.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ExplorationStatus::Active
        );
        assert!(!app_data
            .join("exploration-promotions")
            .join(&promotion_id)
            .exists());

        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn rollback_never_overwrites_an_external_edit() {
        let (creator, store, base, project, app_data) = fixture("rollback_conflict").await;
        let checkpoint = creator.create_checkpoint("p", "main").await.unwrap();
        let exploration = creator
            .create_exploration(&checkpoint.id, "Rollback conflict")
            .await
            .unwrap();
        std::fs::write(
            PathBuf::from(&exploration.workspace_dir).join("baseline.txt"),
            b"promoted",
        )
        .unwrap();
        let preview = ExplorationPromotionService::new(store, app_data.clone())
            .preview(&exploration.id)
            .await
            .unwrap();
        let mut journal = PromotionJournal::prepare(
            &app_data,
            &uuid::Uuid::new_v4().to_string(),
            "p",
            &exploration,
            &dunce::canonicalize(&project).unwrap(),
            &preview.diff.files,
        )
        .unwrap();
        journal.apply().unwrap();
        std::fs::write(project.join("baseline.txt"), b"external edit").unwrap();
        let error = journal.rollback().unwrap_err();
        assert!(error.contains("checksum changed") || error.contains("identity changed"));
        assert_eq!(
            std::fs::read(project.join("baseline.txt")).unwrap(),
            b"external edit"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn promotion_journal_rejects_a_symlinked_storage_root() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "wisp_exploration_promotion_symlink_{}",
            uuid::Uuid::new_v4()
        ));
        let app_data = base.join("app-data");
        let outside = base.join("outside");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, app_data.join("exploration-promotions")).unwrap();

        assert!(promotion_storage_root(&app_data)
            .unwrap_err()
            .contains("not a real directory"));
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(base);
    }
}
