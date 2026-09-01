//! SQLite persistence for Wisp: projects, frames, messages, settings.
//!
//! Replaces the mangopi JSON session file with a structured store. API keys
//! live in the OS keyring (see [`secrets`]); everything else lives here.

mod acp_sessions;
mod agent_workflow_attempts;
mod agent_workflow_deliveries;
mod agent_workflow_run_activities;
mod agent_workflows;
mod artifacts;
mod ask_user_requests;
mod codex_imports;
mod execution_contexts;
mod explorations;
mod external_session_cache;
mod global_memories;
mod library;
mod lineage;
pub mod mcp_secrets;
mod method_search;
mod models;
mod persist_seq;
mod plugins;
mod project_state_revisions;
mod project_sync;
mod project_transfer;
mod projects;
mod provenance;
mod publications;
mod remote_staging;
mod research;
mod resources;
mod runs;
mod schedules;
pub mod secrets;
mod session_imports;
mod sessions;
mod storage_prefs;
mod turn_undo;

pub use acp_sessions::AcpSessionBinding;
pub use agent_workflow_attempts::{
    AgentWorkflowAttempt, AgentWorkflowAttemptStart, AgentWorkflowAttemptStatus,
};
pub use agent_workflow_deliveries::{AgentWorkflowDelivery, AGENT_WORKFLOW_COMPLETION_TOOL};
pub use agent_workflow_run_activities::AgentWorkflowRunActivity;
pub use agent_workflows::{
    AgentDelegationRootLimits, AgentWorkflow, AgentWorkflowStatus, AgentWorkflowStep,
    MAX_ROOT_AGENT_DEPTH, MAX_ROOT_AGENT_TASKS,
};
pub use artifacts::{logical_artifact_id, scoped_logical_artifact_id};
pub use ask_user_requests::AskUserPoll;
pub use explorations::{
    ArtifactHead, ContextArchiveRecord, Exploration, ExplorationBaselineArtifactHead,
    ExplorationBaselineEntity, ExplorationCheckpoint, ExplorationEffect, ExplorationFamily,
    ExplorationPromotion, ExplorationPromotionStatus, ExplorationStatus, ExplorationSummary,
    StateScope, WorkspaceSnapshotRecord, MAINLINE_SCOPE_KEY,
};
pub use external_session_cache::ExternalSessionCacheRecord;
pub use global_memories::GlobalMemory;
pub use library::{
    LibraryItem, LibraryItemDetail, LibraryItemSummary, LibraryItemVersion, LibraryStore,
    NewLibraryItem,
};
pub use method_search::{
    MethodCandidate, MethodCandidateBlob, MethodCandidateStatus, MethodSearchRunState,
    MethodStrategyStat,
};
pub use models::*;
pub use persist_seq::{join_or_abort_persist, persist_seq_loop, PersistJoinError};
pub use project_state_revisions::{ProjectStateRevision, ProjectStateRevisionSummary};
pub use project_sync::ProjectSyncState;
pub use project_transfer::ProjectTransferStats;
pub use projects::{is_scratch_project_id, SCRATCH_PROJECT_PREFIX};
pub use provenance::{canonical_json, canonical_json_sha256};
pub use remote_staging::RemoteStagingEntry;
pub use schedules::{next_slot_after, ScheduleRecord, ScheduleRunRecord};
pub use session_imports::RecoveredWorkspaceSession;
pub use sessions::{
    ModelTokenUsage, ProjectTokenUsage, SessionBranchDeltaMessage, SessionBranchLink,
    SessionBranchMerge, SessionBranchMergeCard, SessionBranchMergePreview, SessionTokenUsage,
    SessionTokenUsagePage, SessionTranscriptPage, SessionUiEventRecord, SessionUiEventSnapshot,
    TokenUsageDay, ToolCallUsage,
};
pub use storage_prefs::{
    validate_local_results_dir, validate_remote_data_root, validate_remote_workdir_root,
    ContextStoragePrefs,
};

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
#[cfg(test)]
use wisp_llm::Message;

pub const MIGRATION_SQL: &str = include_str!("../migrations/0000_init.sql");
const INITIAL_SCHEMA_MIGRATION: &str = "0000_initial_schema";
const CONTROL_PLANE_MIGRATION: &str = "0001_control_plane_backfill";
const ARTIFACT_LINEAGE_MIGRATION: &str = "0002_artifact_lineage";
const SSH_RUN_CONTROL_MIGRATION: &str = "0003_ssh_run_control";
const RUN_LIFECYCLE_LEASE_MIGRATION: &str = "0004_run_lifecycle_lease";
const PROPOSED_PLANS_MIGRATION: &str = "0005_proposed_plans";
const CODEX_TURN_CONFIGS_MIGRATION: &str = "0006_codex_turn_configs";
const ACP_SESSIONS_MIGRATION: &str = "0007_acp_sessions";
const SESSION_REVIEWS_MIGRATION: &str = "0008_session_reviews";
const SESSION_UI_EVENTS_MIGRATION: &str = "0009_session_ui_events";
const PROJECT_SYNC_STATE_MIGRATION: &str = "0010_project_sync_state";
const SESSION_HISTORY_INDEX_MIGRATION: &str = "0011_session_history_index";
const MESSAGE_RESOURCE_LINKS_MIGRATION: &str = "0012_message_resource_links";
const SESSION_EXECUTION_CONTEXTS_MIGRATION: &str = "0013_session_execution_contexts";
const AGENT_WORKFLOWS_MIGRATION: &str = "0014_agent_workflows";
const AGENT_WORKFLOWS_MIGRATION_SQL: &str = include_str!("../migrations/0014_agent_workflows.sql");
const AGENT_WORKFLOW_CONTRACTS_MIGRATION: &str = "0015_agent_workflow_contracts";
const AGENT_WORKFLOW_PLANS_MIGRATION: &str = "0016_agent_workflow_plans";
const AGENT_WORKFLOW_ATTEMPTS_MIGRATION: &str = "0017_agent_workflow_attempts";
const AGENT_WORKFLOW_ATTEMPTS_MIGRATION_SQL: &str =
    include_str!("../migrations/0017_agent_workflow_attempts.sql");
const RUN_PROGRESS_MIGRATION: &str = "0018_run_progress";
const AGENT_WORKFLOW_DELIVERIES_MIGRATION: &str = "0019_agent_workflow_deliveries";
const AGENT_WORKFLOW_DELIVERIES_MIGRATION_SQL: &str =
    include_str!("../migrations/0019_agent_workflow_deliveries.sql");
const AGENT_WORKFLOW_LINEAGE_MIGRATION: &str = "0020_agent_workflow_lineage";
const PLUGIN_INSTALLATIONS_MIGRATION: &str = "0021_plugin_installations";
const PLUGIN_INSTALLATIONS_MIGRATION_SQL: &str =
    include_str!("../migrations/0021_plugin_installations.sql");
const FRAME_SEEN_MIGRATION: &str = "0022_frame_seen";
const SESSION_PINNED_MIGRATION: &str = "0023_session_pinned";
const CODEX_IMPORTS_MIGRATION: &str = "0024_codex_imports";
const EXTERNAL_SESSION_CACHE_MIGRATION: &str = "0025_external_session_cache";
const TURN_FILE_UNDO_MIGRATION: &str = "0026_turn_file_undo";
const SESSION_BRANCH_LINEAGE_MIGRATION: &str = "0027_session_branch_lineage";
const ASK_USER_REQUESTS_MIGRATION: &str = "0028_ask_user_requests";
const RUN_ARTIFACT_LINEAGE_MIGRATION: &str = "0029_run_artifact_lineage";
const PUBLICATION_DOMAIN_MIGRATION: &str = "0030_publication_domain";
const PUBLICATION_DOMAIN_MIGRATION_SQL: &str =
    include_str!("../migrations/0030_publication_domain.sql");
const PUBLICATION_FREEZE_MIGRATION: &str = "0031_publication_freeze";
const PUBLICATION_FREEZE_MIGRATION_SQL: &str =
    include_str!("../migrations/0031_publication_freeze.sql");
const PUBLICATION_VERIFICATION_MIGRATION: &str = "0032_publication_verification";
const PUBLICATION_VERIFICATION_MIGRATION_SQL: &str =
    include_str!("../migrations/0032_publication_verification.sql");
const AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION: &str = "0033_agent_workflow_run_activities";
const AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION_SQL: &str =
    include_str!("../migrations/0033_agent_workflow_run_activities.sql");
const METHOD_SEARCH_MIGRATION: &str = "0034_method_search";
const METHOD_SEARCH_MIGRATION_SQL: &str = include_str!("../migrations/0034_method_search.sql");
const METHOD_SEARCH_CONTROL_MIGRATION: &str = "0035_method_search_control";
const SESSION_IMPORTS_MIGRATION: &str = "0036_session_imports";
const EXPLORATION_BRANCHES_MIGRATION: &str = "0037_exploration_branches";
const EXPLORATION_BRANCHES_MIGRATION_SQL: &str =
    include_str!("../migrations/0037_exploration_branches.sql");
const PROJECT_STATE_REVISIONS_MIGRATION: &str = "0038_project_state_revisions";
const PROJECT_STATE_REVISIONS_MIGRATION_SQL: &str =
    include_str!("../migrations/0038_project_state_revisions.sql");
const GLOBAL_MEMORIES_MIGRATION: &str = "0039_global_memories";
const GLOBAL_MEMORIES_MIGRATION_SQL: &str = include_str!("../migrations/0039_global_memories.sql");
const SESSION_REASONING_EFFORT_MIGRATION: &str = "0040_session_reasoning_effort";
const SESSION_BRANCH_MERGE_MIGRATION: &str = "0041_session_branch_merge";
const EXPLORATION_PROMOTION_RECOVERY_MIGRATION: &str = "0042_exploration_promotion_recovery";
const RUN_HARVEST_STATE_MIGRATION: &str = "0043_run_harvest_state";
const CONTEXT_STORAGE_PREFS_MIGRATION: &str = "0044_context_storage_prefs";
const RUN_CLEANUP_STATE_MIGRATION: &str = "0045_run_cleanup_state";
const REMOTE_STAGING_MIGRATION: &str = "0046_remote_staging";
const RUN_RETENTION_MIGRATION: &str = "0047_run_retention";
const SCHEDULES_MIGRATION: &str = "0048_schedules";
const ARTIFACT_SOURCE_DISCARDED_MIGRATION: &str = "0049_artifact_source_discarded";
const RUN_LOG_PULL_MIGRATION: &str = "0050_run_log_pull";
const ORPHAN_FILE_RETENTION_MIGRATION: &str = "0051_orphan_file_retention";
const RUN_REVIEW_DISMISSED_MIGRATION: &str = "0052_run_review_dismissed";
const SESSION_SERVICE_TIER_MIGRATION: &str = "0053_session_service_tier";

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (or create) the SQLite database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        Self::open_with_journal(path, true).await
    }

    /// Open a throwaway snapshot/transfer database in the default rollback
    /// journal mode. Switching a database out of WAL needs an exclusive lock
    /// that ignores `busy_timeout` and fails with SQLITE_BUSY immediately, so
    /// portable single-file snapshots must never enter WAL to begin with.
    pub(crate) async fn open_snapshot(path: &Path) -> Result<Self> {
        Self::open_with_journal(path, false).await
    }

    async fn open_with_journal(path: &Path, wal: bool) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            // WAL allows only one writer at a time; with a pool of concurrent
            // connections a second writer would otherwise get SQLITE_BUSY
            // immediately (default timeout is 0) and fail. Wait for the lock
            // instead — concurrent tasks writing the same store (e.g. message +
            // provenance persistence) must serialize, not error out.
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        // WAL journaling so a crash mid-turn can't corrupt the DB and committed
        // messages survive (pairs with incremental message persistence).
        if wal {
            sqlx::query("PRAGMA journal_mode=WAL")
                .execute(&pool)
                .await?;
        }
        Self::migrate(&pool).await?;
        let store = Self { pool };
        store
            .upsert_execution_context(&ExecutionContext::new("local", "Local")?)
            .await?;
        Ok(store)
    }

    /// Every multi-statement transaction in this store writes. Take the write
    /// lock at BEGIN so a concurrent writer queues on `busy_timeout` instead
    /// of failing immediately with SQLITE_BUSY_SNAPSHOT when a deferred
    /// transaction that read first upgrades to a write mid-flight.
    pub(crate) async fn begin_write(
        &self,
    ) -> sqlx::Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        self.pool.begin_with("BEGIN IMMEDIATE").await
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS wisp_schema_migrations (\
             version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;

        if !Self::migration_applied(pool, INITIAL_SCHEMA_MIGRATION).await? {
            Self::execute_sql_script(pool, MIGRATION_SQL).await?;
            Self::record_migration(pool, INITIAL_SCHEMA_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, CONTROL_PLANE_MIGRATION).await? {
            Self::apply_control_plane_backfill(pool).await?;
            Self::record_migration(pool, CONTROL_PLANE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, ARTIFACT_LINEAGE_MIGRATION).await? {
            Self::apply_artifact_lineage(pool).await?;
            Self::record_migration(pool, ARTIFACT_LINEAGE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SSH_RUN_CONTROL_MIGRATION).await? {
            Self::apply_ssh_run_control(pool).await?;
            Self::record_migration(pool, SSH_RUN_CONTROL_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_LIFECYCLE_LEASE_MIGRATION).await? {
            Self::apply_run_lifecycle_lease(pool).await?;
            Self::record_migration(pool, RUN_LIFECYCLE_LEASE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PROPOSED_PLANS_MIGRATION).await? {
            Self::apply_proposed_plans(pool).await?;
            Self::record_migration(pool, PROPOSED_PLANS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, CODEX_TURN_CONFIGS_MIGRATION).await? {
            Self::apply_codex_turn_configs(pool).await?;
            Self::record_migration(pool, CODEX_TURN_CONFIGS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, ACP_SESSIONS_MIGRATION).await? {
            Self::apply_acp_sessions(pool).await?;
            Self::record_migration(pool, ACP_SESSIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_REVIEWS_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS session_reviews (\
                 id TEXT PRIMARY KEY, frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 message_seq INTEGER NOT NULL, report_json TEXT NOT NULL, \
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_session_reviews_frame \
                 ON session_reviews(frame_id, message_seq)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, SESSION_REVIEWS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_UI_EVENTS_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS session_ui_events (\
                 frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 seq INTEGER NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY(frame_id,seq))",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, SESSION_UI_EVENTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PROJECT_SYNC_STATE_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS project_sync_state (\
                 project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE, \
                 transport_kind TEXT NOT NULL, transport_location TEXT NOT NULL, \
                 relay_project_id TEXT NOT NULL, \
                 base_revision TEXT, base_state_hash TEXT, \
                 base_manifest_json TEXT NOT NULL DEFAULT '{\"version\":1,\"files\":[],\"skipped_paths\":[]}', \
                 last_synced_at INTEGER, last_direction TEXT)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, PROJECT_SYNC_STATE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_HISTORY_INDEX_MIGRATION).await? {
            let frames_exist: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='frames'",
            )
            .fetch_one(pool)
            .await?;
            if frames_exist.0 > 0 {
                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS ix_frames_project_created \
                     ON frames(project_id, created_at DESC, id DESC)",
                )
                .execute(pool)
                .await?;
            }
            Self::record_migration(pool, SESSION_HISTORY_INDEX_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, MESSAGE_RESOURCE_LINKS_MIGRATION).await? {
            Self::apply_message_resource_links(pool).await?;
            Self::record_migration(pool, MESSAGE_RESOURCE_LINKS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_EXECUTION_CONTEXTS_MIGRATION).await? {
            Self::apply_session_execution_contexts(pool).await?;
            Self::record_migration(pool, SESSION_EXECUTION_CONTEXTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOWS_MIGRATION).await? {
            Self::execute_sql_script(pool, AGENT_WORKFLOWS_MIGRATION_SQL).await?;
            Self::record_migration(pool, AGENT_WORKFLOWS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_CONTRACTS_MIGRATION).await? {
            Self::apply_agent_workflow_contracts(pool).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_CONTRACTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_PLANS_MIGRATION).await? {
            Self::apply_agent_workflow_plans(pool).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_PLANS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_ATTEMPTS_MIGRATION).await? {
            Self::execute_sql_script(pool, AGENT_WORKFLOW_ATTEMPTS_MIGRATION_SQL).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_ATTEMPTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_PROGRESS_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "runs",
                &[("progress_json", "TEXT NOT NULL DEFAULT '{}'")],
            )
            .await?;
            Self::record_migration(pool, RUN_PROGRESS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_DELIVERIES_MIGRATION).await? {
            Self::execute_sql_script(pool, AGENT_WORKFLOW_DELIVERIES_MIGRATION_SQL).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_DELIVERIES_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_LINEAGE_MIGRATION).await? {
            Self::apply_agent_workflow_lineage(pool).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_LINEAGE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PLUGIN_INSTALLATIONS_MIGRATION).await? {
            Self::execute_sql_script(pool, PLUGIN_INSTALLATIONS_MIGRATION_SQL).await?;
            Self::record_migration(pool, PLUGIN_INSTALLATIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, FRAME_SEEN_MIGRATION).await? {
            // Last activity the user has viewed, compared against message ts.
            // 0 = never viewed, so a session flags "needs you" until opened.
            Self::add_columns_if_missing(
                pool,
                "frames",
                &[("seen_at", "INTEGER NOT NULL DEFAULT 0")],
            )
            .await?;
            // Treat pre-existing history as read — without this, upgrading
            // flags every old conversation at once and each one would have to
            // be opened to clear it.
            let _ = sqlx::query(
                "UPDATE frames SET seen_at = \
                    (SELECT COALESCE(MAX(ts), frames.updated_at) FROM messages m \
                     WHERE m.frame_id = frames.id)",
            )
            .execute(pool)
            .await;
            Self::record_migration(pool, FRAME_SEEN_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_PINNED_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "frames",
                &[("pinned", "INTEGER NOT NULL DEFAULT 0")],
            )
            .await?;
            Self::record_migration(pool, SESSION_PINNED_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, CODEX_IMPORTS_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS codex_imports (\
                 codex_session_id TEXT PRIMARY KEY, \
                 frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 source_path TEXT NOT NULL, \
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_codex_imports_frame ON codex_imports(frame_id)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, CODEX_IMPORTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, EXTERNAL_SESSION_CACHE_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS external_session_cache (\
                 source_id TEXT NOT NULL, provider TEXT NOT NULL, source_path TEXT NOT NULL, \
                 file_size INTEGER NOT NULL, modified_at_ms INTEGER NOT NULL, \
                 session_id TEXT NOT NULL, title TEXT NOT NULL, cwd TEXT NOT NULL, \
                 message_count INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, \
                 last_active_at_ms INTEGER NOT NULL, changed_since_import INTEGER NOT NULL DEFAULT 0, \
                 PRIMARY KEY(source_id,provider,source_path))",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_external_session_cache_source \
                 ON external_session_cache(source_id,provider,last_active_at_ms DESC)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, EXTERNAL_SESSION_CACHE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, TURN_FILE_UNDO_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "message_resource_links",
                &[
                    ("created_artifact", "INTEGER NOT NULL DEFAULT 0"),
                    ("created_version", "INTEGER NOT NULL DEFAULT 0"),
                ],
            )
            .await?;
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS turn_file_undo (\
                 frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 user_message_seq INTEGER NOT NULL, path TEXT NOT NULL, \
                 before_exists INTEGER NOT NULL, before_snapshot_path TEXT, \
                 before_checksum TEXT, after_checksum TEXT, reversible INTEGER NOT NULL, \
                 reason TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
                 PRIMARY KEY(frame_id,user_message_seq,path))",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_turn_file_undo_turn \
                 ON turn_file_undo(frame_id,user_message_seq)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, TURN_FILE_UNDO_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_BRANCH_LINEAGE_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "frames", &[("branched_from", "TEXT")]).await?;
            Self::record_migration(pool, SESSION_BRANCH_LINEAGE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, ASK_USER_REQUESTS_MIGRATION).await? {
            // The ACP bridge's ask_user handshake: the bridge process INSERTs
            // and polls, the host answers or expires. SQLite is the only
            // channel the two processes share.
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS ask_user_requests (\
                 request_id TEXT PRIMARY KEY, \
                 frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 payload_json TEXT NOT NULL, \
                 answer TEXT, \
                 status TEXT NOT NULL DEFAULT 'pending', \
                 created_at INTEGER NOT NULL, answered_at INTEGER)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_ask_user_requests_frame \
                 ON ask_user_requests(frame_id,status)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, ASK_USER_REQUESTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_ARTIFACT_LINEAGE_MIGRATION).await? {
            Self::apply_run_artifact_lineage(pool).await?;
            Self::record_migration(pool, RUN_ARTIFACT_LINEAGE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PUBLICATION_DOMAIN_MIGRATION).await? {
            Self::execute_sql_script(pool, PUBLICATION_DOMAIN_MIGRATION_SQL).await?;
            Self::install_publication_triggers(pool).await?;
            Self::record_migration(pool, PUBLICATION_DOMAIN_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PUBLICATION_FREEZE_MIGRATION).await? {
            Self::apply_publication_freeze(pool).await?;
            Self::record_migration(pool, PUBLICATION_FREEZE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PUBLICATION_VERIFICATION_MIGRATION).await? {
            Self::execute_sql_script(pool, PUBLICATION_VERIFICATION_MIGRATION_SQL).await?;
            Self::record_migration(pool, PUBLICATION_VERIFICATION_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "agent_workflow_steps",
                &[
                    ("task_kind", "TEXT NOT NULL DEFAULT 'agent'"),
                    ("activity_json", "TEXT NOT NULL DEFAULT '{}'"),
                ],
            )
            .await?;
            Self::execute_sql_script(pool, AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION_SQL).await?;
            Self::record_migration(pool, AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, METHOD_SEARCH_MIGRATION).await? {
            Self::execute_sql_script(pool, METHOD_SEARCH_MIGRATION_SQL).await?;
            Self::record_migration(pool, METHOD_SEARCH_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, METHOD_SEARCH_CONTROL_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "method_search_runs",
                &[("control_state", "TEXT NOT NULL DEFAULT 'run'")],
            )
            .await?;
            Self::record_migration(pool, METHOD_SEARCH_CONTROL_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_IMPORTS_MIGRATION).await? {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS session_imports (\
                 source_session_id TEXT PRIMARY KEY, \
                 frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 source_path TEXT NOT NULL, \
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_session_imports_frame ON session_imports(frame_id)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, SESSION_IMPORTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, EXPLORATION_BRANCHES_MIGRATION).await? {
            Self::apply_exploration_branches(pool).await?;
            Self::record_migration(pool, EXPLORATION_BRANCHES_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, PROJECT_STATE_REVISIONS_MIGRATION).await? {
            Self::execute_sql_script(pool, PROJECT_STATE_REVISIONS_MIGRATION_SQL).await?;
            Self::record_migration(pool, PROJECT_STATE_REVISIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, GLOBAL_MEMORIES_MIGRATION).await? {
            Self::execute_sql_script(pool, GLOBAL_MEMORIES_MIGRATION_SQL).await?;
            Self::record_migration(pool, GLOBAL_MEMORIES_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_REASONING_EFFORT_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "frames", &[("reasoning_effort", "TEXT")]).await?;
            Self::record_migration(pool, SESSION_REASONING_EFFORT_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_BRANCH_MERGE_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "frames",
                &[
                    ("branch_point_user_index", "INTEGER"),
                    ("branch_point_kind", "TEXT"),
                ],
            )
            .await?;
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS session_branch_merges (\
                 id TEXT PRIMARY KEY, \
                 source_frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 branch_frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
                 checkpoint_user_index INTEGER NOT NULL, checkpoint_kind TEXT NOT NULL, \
                 summary_message_seq INTEGER NOT NULL, guard_hash TEXT NOT NULL, \
                 created_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_session_branch_merges_source \
                 ON session_branch_merges(source_frame_id, created_at DESC)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, SESSION_BRANCH_MERGE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, EXPLORATION_PROMOTION_RECOVERY_MIGRATION).await? {
            Self::apply_exploration_promotion_recovery(pool).await?;
            Self::record_migration(pool, EXPLORATION_PROMOTION_RECOVERY_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_HARVEST_STATE_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "runs", &[("harvested_at", "INTEGER")]).await?;
            Self::record_migration(pool, RUN_HARVEST_STATE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, CONTEXT_STORAGE_PREFS_MIGRATION).await? {
            Self::apply_context_storage_prefs(pool).await?;
            Self::record_migration(pool, CONTEXT_STORAGE_PREFS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_CLEANUP_STATE_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "runs",
                &[("cleaned_at", "INTEGER"), ("cleanup_error", "TEXT")],
            )
            .await?;
            Self::record_migration(pool, RUN_CLEANUP_STATE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, REMOTE_STAGING_MIGRATION).await? {
            Self::apply_remote_staging(pool).await?;
            Self::record_migration(pool, REMOTE_STAGING_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_RETENTION_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "projects",
                &[
                    ("run_retention_days", "INTEGER"),
                    ("failed_run_retention_days", "INTEGER"),
                ],
            )
            .await?;
            Self::record_migration(pool, RUN_RETENTION_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SCHEDULES_MIGRATION).await? {
            // frame_id stays a plain column on purpose: a schedule must
            // survive its target session being deleted so it can keep firing
            // into fresh sessions or be re-pointed by the user.
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS schedules(\
                 id TEXT PRIMARY KEY, \
                 project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
                 frame_id TEXT, \
                 name TEXT NOT NULL, prompt TEXT NOT NULL, skill TEXT, \
                 interval_secs INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, \
                 next_run_at INTEGER NOT NULL, last_run_at INTEGER, \
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_schedules_due \
                 ON schedules(enabled, next_run_at)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS schedule_runs(\
                 id TEXT PRIMARY KEY, \
                 schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE, \
                 frame_id TEXT, status TEXT NOT NULL, error TEXT, \
                 fired_at INTEGER NOT NULL)",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_schedule_runs_schedule \
                 ON schedule_runs(schedule_id, fired_at DESC)",
            )
            .execute(pool)
            .await?;
            Self::record_migration(pool, SCHEDULES_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, ARTIFACT_SOURCE_DISCARDED_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "artifact_versions",
                &[("source_discarded_at", "INTEGER")],
            )
            .await?;
            Self::record_migration(pool, ARTIFACT_SOURCE_DISCARDED_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_LOG_PULL_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "runs", &[("logs_path", "TEXT")]).await?;
            Self::record_migration(pool, RUN_LOG_PULL_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, ORPHAN_FILE_RETENTION_MIGRATION).await? {
            Self::add_columns_if_missing(
                pool,
                "projects",
                &[("orphan_file_retention_days", "INTEGER")],
            )
            .await?;
            Self::record_migration(pool, ORPHAN_FILE_RETENTION_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, RUN_REVIEW_DISMISSED_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "runs", &[("review_dismissed_at", "INTEGER")])
                .await?;
            Self::record_migration(pool, RUN_REVIEW_DISMISSED_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, SESSION_SERVICE_TIER_MIGRATION).await? {
            Self::add_columns_if_missing(pool, "frames", &[("service_tier", "TEXT")]).await?;
            Self::record_migration(pool, SESSION_SERVICE_TIER_MIGRATION).await?;
        }
        // Re-apply additive DDL even when a migration marker is already
        // recorded. Jumping many releases can leave a table/column that was
        // later folded into 0000_init.sql (or into an already-shipped apply_*
        // body) missing, and the next query then fails with "no such column".
        Self::ensure_schema_compat(pool).await?;
        Ok(())
    }

    /// Idempotent repair for schema objects that numbered migrations can miss
    /// after a large version skip. Only CREATE IF NOT EXISTS / ADD COLUMN.
    async fn ensure_schema_compat(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS folders (\
             id TEXT PRIMARY KEY, \
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             name TEXT NOT NULL, \
             created_at INTEGER NOT NULL, \
             updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_folders_project ON folders(project_id)")
            .execute(pool)
            .await?;

        Self::add_columns_if_missing(
            pool,
            "projects",
            &[
                ("workspace_dir", "TEXT NOT NULL DEFAULT ''"),
                ("run_retention_days", "INTEGER"),
                ("failed_run_retention_days", "INTEGER"),
                ("orphan_file_retention_days", "INTEGER"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(pool, "messages", &[("model_name", "TEXT")]).await?;
        Self::add_columns_if_missing(
            pool,
            "frames",
            &[
                ("title", "TEXT"),
                ("folder_id", "TEXT"),
                ("seen_at", "INTEGER NOT NULL DEFAULT 0"),
                ("pinned", "INTEGER NOT NULL DEFAULT 0"),
                ("branched_from", "TEXT"),
                ("reasoning_effort", "TEXT"),
                ("service_tier", "TEXT"),
                ("branch_point_user_index", "INTEGER"),
                ("branch_point_kind", "TEXT"),
                ("exploration_id", "TEXT"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "artifacts",
            &[
                ("latest_version_id", "TEXT"),
                ("logical_key", "TEXT"),
                ("exploration_id", "TEXT"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "artifact_versions",
            &[
                ("materialization", "TEXT NOT NULL DEFAULT 'reference'"),
                ("capture_timing", "TEXT NOT NULL DEFAULT 'unknown'"),
                ("source_discarded_at", "INTEGER"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "artifact_dependencies",
            &[
                ("basis", "TEXT NOT NULL DEFAULT 'inferred'"),
                ("confidence", "TEXT NOT NULL DEFAULT 'uncertain'"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "env_snapshots",
            &[
                ("snapshot_json", "TEXT NOT NULL DEFAULT '{}'"),
                ("hash_algorithm", "TEXT NOT NULL DEFAULT 'legacy'"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "runs",
            &[
                ("remote_handle_json", "TEXT"),
                ("timeout_secs", "INTEGER"),
                ("last_polled_at", "INTEGER"),
                ("last_poll_error", "TEXT"),
                ("lifecycle_owner", "TEXT"),
                ("lifecycle_lease_until", "INTEGER"),
                ("progress_json", "TEXT NOT NULL DEFAULT '{}'"),
                ("harvested_at", "INTEGER"),
                ("cleaned_at", "INTEGER"),
                ("cleanup_error", "TEXT"),
                ("logs_path", "TEXT"),
                ("exploration_id", "TEXT"),
                ("review_dismissed_at", "INTEGER"),
            ],
        )
        .await?;
        for table in ["research_nodes", "research_edges", "external_resources"] {
            Self::add_columns_if_missing(pool, table, &[("exploration_id", "TEXT")]).await?;
        }
        Self::add_columns_if_missing(
            pool,
            "message_resource_links",
            &[
                ("created_artifact", "INTEGER NOT NULL DEFAULT 0"),
                ("created_version", "INTEGER NOT NULL DEFAULT 0"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "method_search_runs",
            &[("control_state", "TEXT NOT NULL DEFAULT 'run'")],
        )
        .await?;
        Self::add_columns_if_missing(pool, "session_ui_events", &[("created_at", "INTEGER")])
            .await?;
        Ok(())
    }

    /// Promotion recovery must outlive the exploration row that is hard
    /// deleted by a successful metadata commit. Early exploration builds used
    /// `ON DELETE CASCADE` here, which removed the recovery row just before the
    /// transaction advanced it to `metadata_committed`.
    async fn apply_exploration_promotion_recovery(pool: &SqlitePool) -> Result<()> {
        let table_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type='table' AND name='exploration_promotions')",
        )
        .fetch_one(pool)
        .await?;
        if !table_exists {
            return Ok(());
        }
        let foreign_key_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('exploration_promotions')",
        )
        .fetch_one(pool)
        .await?;
        if foreign_key_count == 0 {
            return Ok(());
        }

        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DROP INDEX IF EXISTS ix_exploration_promotions_exploration")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE TABLE exploration_promotions_recovery (\
               id TEXT PRIMARY KEY, exploration_id TEXT NOT NULL,\
               expected_guard_hash TEXT NOT NULL, status TEXT NOT NULL,\
               diff_json TEXT NOT NULL, journal_path TEXT, error TEXT,\
               started_at INTEGER NOT NULL, committed_at INTEGER\
             )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO exploration_promotions_recovery(\
               id,exploration_id,expected_guard_hash,status,diff_json,journal_path,error,\
               started_at,committed_at)\
             SELECT id,exploration_id,expected_guard_hash,status,diff_json,journal_path,error,\
                    started_at,committed_at FROM exploration_promotions",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE exploration_promotions")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE exploration_promotions_recovery RENAME TO exploration_promotions")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_exploration_promotions_exploration \
             ON exploration_promotions(exploration_id,started_at DESC)",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn apply_exploration_branches(pool: &SqlitePool) -> Result<()> {
        for table in [
            "frames",
            "artifacts",
            "runs",
            "research_nodes",
            "research_edges",
            "external_resources",
        ] {
            Self::add_columns_if_missing(pool, table, &[("exploration_id", "TEXT")]).await?;
        }
        Self::execute_sql_script(pool, EXPLORATION_BRANCHES_MIGRATION_SQL).await?;

        let artifacts_exist: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='artifacts')",
        )
        .fetch_one(pool)
        .await?;
        if artifacts_exist {
            sqlx::query("DROP INDEX IF EXISTS ux_artifacts_project_logical_key")
                .execute(pool)
                .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS ix_artifacts_project_logical_key \
                 ON artifacts(project_id,logical_key) WHERE logical_key IS NOT NULL",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "INSERT INTO artifact_heads(\
                   project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at\
                 ) SELECT project_id,?1,logical_key,id,latest_version_id,created_at \
                   FROM artifacts WHERE logical_key IS NOT NULL AND latest_version_id IS NOT NULL \
                 ON CONFLICT(project_id,scope_key,logical_key) DO UPDATE SET \
                   artifact_id=excluded.artifact_id, \
                   artifact_version_id=excluded.artifact_version_id, \
                   updated_at=excluded.updated_at",
            )
            .bind(MAINLINE_SCOPE_KEY)
            .execute(pool)
            .await?;
        }
        let projects_exist: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='projects')",
        )
        .fetch_one(pool)
        .await?;
        if projects_exist {
            sqlx::query(
                "INSERT OR IGNORE INTO project_state_counters(project_id,mainline_generation,updated_at) \
                 SELECT id,0,? FROM projects",
            )
            .bind(chrono::Utc::now().timestamp())
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    async fn apply_publication_freeze(pool: &SqlitePool) -> Result<()> {
        Self::add_columns_if_missing(
            pool,
            "publication_readiness_reports",
            &[
                (
                    "target_visibility",
                    "TEXT NOT NULL DEFAULT 'public' CHECK(target_visibility IN ('public','restricted','private'))",
                ),
                ("policy_json", "TEXT NOT NULL DEFAULT '{}'"),
            ],
        )
        .await?;
        Self::execute_sql_script(pool, PUBLICATION_FREEZE_MIGRATION_SQL).await?;
        Self::install_publication_freeze_triggers(pool).await
    }

    async fn apply_run_artifact_lineage(pool: &SqlitePool) -> Result<()> {
        Self::add_columns_if_missing(pool, "artifacts", &[("logical_key", "TEXT")]).await?;
        Self::add_columns_if_missing(
            pool,
            "artifact_versions",
            &[
                ("materialization", "TEXT NOT NULL DEFAULT 'reference'"),
                ("capture_timing", "TEXT NOT NULL DEFAULT 'unknown'"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "artifact_dependencies",
            &[
                ("basis", "TEXT NOT NULL DEFAULT 'inferred'"),
                ("confidence", "TEXT NOT NULL DEFAULT 'uncertain'"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "env_snapshots",
            &[
                ("snapshot_json", "TEXT NOT NULL DEFAULT '{}'"),
                ("hash_algorithm", "TEXT NOT NULL DEFAULT 'legacy'"),
            ],
        )
        .await?;
        let artifacts_exist: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='artifacts')",
        )
        .fetch_one(pool)
        .await?;
        let artifact_heads_exist: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='artifact_heads')",
        )
        .fetch_one(pool)
        .await?;
        if artifacts_exist && !artifact_heads_exist {
            sqlx::query(
                "CREATE UNIQUE INDEX IF NOT EXISTS ux_artifacts_project_logical_key \
                 ON artifacts(project_id,logical_key) WHERE logical_key IS NOT NULL",
            )
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS external_resources (\
             id TEXT PRIMARY KEY, \
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             kind TEXT NOT NULL, uri TEXT NOT NULL, version TEXT, checksum TEXT, \
             size_bytes INTEGER, license TEXT, visibility TEXT NOT NULL DEFAULT 'restricted', \
             access_instructions TEXT, accessed_at INTEGER, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             UNIQUE(project_id,uri,version))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS run_inputs (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, \
             artifact_version_id TEXT REFERENCES artifact_versions(id) ON DELETE RESTRICT, \
             external_resource_id TEXT REFERENCES external_resources(id) ON DELETE RESTRICT, \
             source_ref TEXT NOT NULL, role TEXT NOT NULL, required INTEGER NOT NULL DEFAULT 1, \
             basis TEXT NOT NULL, confidence TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_run_inputs_run ON run_inputs(run_id)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_run_inputs_artifact_version \
             ON run_inputs(artifact_version_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS run_outputs (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, \
             artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE RESTRICT, \
             role TEXT NOT NULL, logical_output_key TEXT NOT NULL, source_path TEXT NOT NULL, \
             created_at INTEGER NOT NULL, UNIQUE(run_id,artifact_version_id,role))",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_run_outputs_run ON run_outputs(run_id)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_run_outputs_artifact_version \
             ON run_outputs(artifact_version_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS run_code_snapshots (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, \
             source_kind TEXT NOT NULL, source_path TEXT, source_text TEXT NOT NULL, \
             checksum TEXT NOT NULL, storage_path TEXT, git_commit TEXT, dirty_patch TEXT, \
             created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_run_code_snapshots_run \
             ON run_code_snapshots(run_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS run_environment_snapshots (\
             run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE, \
             env_snapshot_hash TEXT NOT NULL REFERENCES env_snapshots(hash) ON DELETE RESTRICT)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn install_publication_triggers(pool: &SqlitePool) -> Result<()> {
        for statement in [
            "CREATE TRIGGER IF NOT EXISTS trg_publication_revision_parent_insert \
             BEFORE INSERT ON publication_revisions \
             WHEN NEW.parent_revision_id=NEW.id \
               OR (NEW.parent_revision_id IS NOT NULL AND NOT EXISTS(\
               SELECT 1 FROM publication_revisions parent \
               WHERE parent.id=NEW.parent_revision_id \
                 AND parent.publication_id=NEW.publication_id)) \
             BEGIN SELECT RAISE(ABORT,'Publication revision parent must belong to the same Publication'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_revision_parent_update \
             BEFORE UPDATE OF parent_revision_id,publication_id ON publication_revisions \
             WHEN NEW.parent_revision_id=NEW.id \
               OR (NEW.parent_revision_id IS NOT NULL AND NOT EXISTS(\
               SELECT 1 FROM publication_revisions parent \
               WHERE parent.id=NEW.parent_revision_id \
                 AND parent.publication_id=NEW.publication_id)) \
             BEGIN SELECT RAISE(ABORT,'Publication revision parent must belong to the same Publication'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_revision_immutable_update \
             BEFORE UPDATE ON publication_revisions \
             WHEN OLD.state IN ('frozen','published') AND (\
               NEW.publication_id IS NOT OLD.publication_id \
               OR NEW.parent_revision_id IS NOT OLD.parent_revision_id \
               OR NEW.revision_number IS NOT OLD.revision_number \
               OR NEW.label IS NOT OLD.label \
               OR NEW.capability_level IS NOT OLD.capability_level \
               OR NEW.manifest_json IS NOT OLD.manifest_json \
               OR NEW.manifest_sha256 IS NOT OLD.manifest_sha256 \
               OR NEW.frozen_at IS NOT OLD.frozen_at \
               OR NEW.created_at IS NOT OLD.created_at \
               OR (NOT(OLD.state='frozen' AND NEW.state='published') \
                   AND NEW.published_at IS NOT OLD.published_at) \
               OR (OLD.state='frozen' AND NEW.state='published' \
                   AND NEW.published_at IS NULL) \
               OR (OLD.state='frozen' AND NEW.state NOT IN ('published','deleting')) \
               OR (OLD.state='published' AND NEW.state<>'deleting')\
             ) \
             BEGIN SELECT RAISE(ABORT,'Publication revision is immutable'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_revision_immutable_delete \
             BEFORE DELETE ON publication_revisions \
             WHEN OLD.state NOT IN ('draft','deleting') \
             BEGIN SELECT RAISE(ABORT,'Publication revision is immutable'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_item_parent_insert \
             BEFORE INSERT ON publication_items \
             WHEN NEW.parent_item_id IS NOT NULL AND NOT EXISTS(\
               SELECT 1 FROM publication_items parent \
               WHERE parent.id=NEW.parent_item_id AND parent.revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Publication item parent must belong to the same revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_item_parent_update \
             BEFORE UPDATE OF parent_item_id,revision_id ON publication_items \
             WHEN NEW.parent_item_id IS NOT NULL AND NOT EXISTS(\
               SELECT 1 FROM publication_items parent \
               WHERE parent.id=NEW.parent_item_id AND parent.revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Publication item parent must belong to the same revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_item_link_scope_insert \
             BEFORE INSERT ON publication_item_links \
             WHEN NOT EXISTS(SELECT 1 FROM publication_items \
                             WHERE id=NEW.source_item_id AND revision_id=NEW.revision_id) \
               OR NOT EXISTS(SELECT 1 FROM publication_items \
                             WHERE id=NEW.target_item_id AND revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Publication item link must stay inside one revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_publication_item_link_scope_update \
             BEFORE UPDATE ON publication_item_links \
             WHEN NOT EXISTS(SELECT 1 FROM publication_items \
                             WHERE id=NEW.source_item_id AND revision_id=NEW.revision_id) \
               OR NOT EXISTS(SELECT 1 FROM publication_items \
                             WHERE id=NEW.target_item_id AND revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Publication item link must stay inside one revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_binding_scope_insert \
             BEFORE INSERT ON evidence_bindings \
             WHEN (NEW.item_id IS NOT NULL AND NOT EXISTS(\
                     SELECT 1 FROM publication_items \
                     WHERE id=NEW.item_id AND revision_id=NEW.revision_id)) \
               OR (NEW.supported_claim_item_id IS NOT NULL AND NOT EXISTS(\
                     SELECT 1 FROM publication_items \
                     WHERE id=NEW.supported_claim_item_id \
                       AND revision_id=NEW.revision_id AND kind='claim')) \
             BEGIN SELECT RAISE(ABORT,'Evidence binding items must belong to the revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_binding_scope_update \
             BEFORE UPDATE ON evidence_bindings \
             WHEN (NEW.item_id IS NOT NULL AND NOT EXISTS(\
                     SELECT 1 FROM publication_items \
                     WHERE id=NEW.item_id AND revision_id=NEW.revision_id)) \
               OR (NEW.supported_claim_item_id IS NOT NULL AND NOT EXISTS(\
                     SELECT 1 FROM publication_items \
                     WHERE id=NEW.supported_claim_item_id \
                       AND revision_id=NEW.revision_id AND kind='claim')) \
             BEGIN SELECT RAISE(ABORT,'Evidence binding items must belong to the revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_binding_source_project_insert \
             BEFORE INSERT ON evidence_bindings \
             WHEN (NEW.source_kind='artifact_version' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN artifact_versions version ON version.id=NEW.artifact_version_id \
                     JOIN artifacts artifact ON artifact.id=version.artifact_id \
                     WHERE revision.id=NEW.revision_id \
                       AND artifact.project_id=publication.project_id)) \
               OR (NEW.source_kind='run' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN runs run ON run.id=NEW.run_id \
                     WHERE revision.id=NEW.revision_id \
                       AND run.project_id=publication.project_id)) \
               OR (NEW.source_kind='external_resource' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN external_resources resource ON resource.id=NEW.external_resource_id \
                     WHERE revision.id=NEW.revision_id \
                       AND resource.project_id=publication.project_id)) \
             BEGIN SELECT RAISE(ABORT,'Evidence source must belong to the Publication project'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_binding_source_project_update \
             BEFORE UPDATE ON evidence_bindings \
             WHEN (NEW.source_kind='artifact_version' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN artifact_versions version ON version.id=NEW.artifact_version_id \
                     JOIN artifacts artifact ON artifact.id=version.artifact_id \
                     WHERE revision.id=NEW.revision_id \
                       AND artifact.project_id=publication.project_id)) \
               OR (NEW.source_kind='run' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN runs run ON run.id=NEW.run_id \
                     WHERE revision.id=NEW.revision_id \
                       AND run.project_id=publication.project_id)) \
               OR (NEW.source_kind='external_resource' AND NOT EXISTS(\
                     SELECT 1 FROM publication_revisions revision \
                     JOIN publications publication ON publication.id=revision.publication_id \
                     JOIN external_resources resource ON resource.id=NEW.external_resource_id \
                     WHERE revision.id=NEW.revision_id \
                       AND resource.project_id=publication.project_id)) \
             BEGIN SELECT RAISE(ABORT,'Evidence source must belong to the Publication project'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_supersession_scope_insert \
             BEFORE INSERT ON evidence_supersessions \
             WHEN NOT EXISTS(SELECT 1 FROM evidence_bindings \
                             WHERE id=NEW.old_binding_id AND revision_id=NEW.revision_id) \
               OR NOT EXISTS(SELECT 1 FROM evidence_bindings \
                             WHERE id=NEW.new_binding_id AND revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Evidence supersession must stay inside one revision'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_evidence_supersession_scope_update \
             BEFORE UPDATE ON evidence_supersessions \
             WHEN NOT EXISTS(SELECT 1 FROM evidence_bindings \
                             WHERE id=NEW.old_binding_id AND revision_id=NEW.revision_id) \
               OR NOT EXISTS(SELECT 1 FROM evidence_bindings \
                             WHERE id=NEW.new_binding_id AND revision_id=NEW.revision_id) \
             BEGIN SELECT RAISE(ABORT,'Evidence supersession must stay inside one revision'); END",
        ] {
            sqlx::query(statement).execute(pool).await?;
        }

        for table in [
            "publication_items",
            "publication_item_links",
            "evidence_bindings",
            "evidence_supersessions",
            "publication_readiness_reports",
            "publication_waivers",
        ] {
            for operation in ["insert", "update", "delete"] {
                let state_check = match operation {
                    "insert" => {
                        "COALESCE((SELECT state FROM publication_revisions \
                         WHERE id=NEW.revision_id),'missing') \
                         NOT IN ('draft','freezing','deleting')"
                    }
                    "update" => {
                        "COALESCE((SELECT state FROM publication_revisions \
                         WHERE id=OLD.revision_id),'missing') \
                         NOT IN ('draft','freezing','deleting') \
                         OR COALESCE((SELECT state FROM publication_revisions \
                         WHERE id=NEW.revision_id),'missing') \
                         NOT IN ('draft','freezing','deleting')"
                    }
                    "delete" => {
                        "COALESCE((SELECT state FROM publication_revisions \
                         WHERE id=OLD.revision_id),'missing') \
                         NOT IN ('draft','freezing','deleting')"
                    }
                    _ => unreachable!(),
                };
                let trigger = format!(
                    "CREATE TRIGGER IF NOT EXISTS trg_{table}_{operation}_draft \
                     BEFORE {} ON {table} \
                     WHEN {state_check} \
                     BEGIN SELECT RAISE(ABORT,'Publication revision content is immutable'); END",
                    operation.to_ascii_uppercase()
                );
                sqlx::query(&trigger).execute(pool).await?;
            }
        }

        for operation in ["insert", "update", "delete"] {
            let state_check = match operation {
                "insert" => {
                    "COALESCE((\
                       SELECT revision.state FROM evidence_bindings binding \
                       JOIN publication_revisions revision ON revision.id=binding.revision_id \
                       WHERE binding.id=NEW.binding_id\
                     ),'missing') NOT IN ('draft','freezing','deleting')"
                }
                "update" => {
                    "COALESCE((\
                       SELECT revision.state FROM evidence_bindings binding \
                       JOIN publication_revisions revision ON revision.id=binding.revision_id \
                       WHERE binding.id=OLD.binding_id\
                     ),'missing') NOT IN ('draft','freezing','deleting') \
                     OR COALESCE((\
                       SELECT revision.state FROM evidence_bindings binding \
                       JOIN publication_revisions revision ON revision.id=binding.revision_id \
                       WHERE binding.id=NEW.binding_id\
                     ),'missing') NOT IN ('draft','freezing','deleting')"
                }
                "delete" => {
                    "COALESCE((\
                       SELECT revision.state FROM evidence_bindings binding \
                       JOIN publication_revisions revision ON revision.id=binding.revision_id \
                       WHERE binding.id=OLD.binding_id\
                     ),'missing') NOT IN ('draft','freezing','deleting')"
                }
                _ => unreachable!(),
            };
            let trigger = format!(
                "CREATE TRIGGER IF NOT EXISTS trg_evidence_reviews_{operation}_draft \
                 BEFORE {} ON evidence_reviews \
                 WHEN {state_check} \
                 BEGIN SELECT RAISE(ABORT,'Publication revision content is immutable'); END",
                operation.to_ascii_uppercase()
            );
            sqlx::query(&trigger).execute(pool).await?;
        }
        Ok(())
    }

    async fn install_publication_freeze_triggers(pool: &SqlitePool) -> Result<()> {
        for (base_table, statement) in [
            (
                None,
                "CREATE TRIGGER IF NOT EXISTS trg_publication_freeze_attempt_revision_insert \
             BEFORE INSERT ON publication_freeze_attempts \
             WHEN NOT EXISTS(\
               SELECT 1 FROM publication_revisions revision \
               WHERE revision.id=NEW.revision_id AND revision.state='freezing'\
             ) \
             BEGIN SELECT RAISE(ABORT,'Publication freeze attempt requires a Freezing revision'); END",
            ),
            (
                None,
                "CREATE TRIGGER IF NOT EXISTS trg_publication_freezing_exit \
             BEFORE UPDATE OF state ON publication_revisions \
             WHEN OLD.state='freezing' AND NEW.state<>'freezing' \
               AND EXISTS(\
                 SELECT 1 FROM publication_freeze_attempts attempt \
                 WHERE attempt.revision_id=OLD.id\
               ) \
             BEGIN SELECT RAISE(ABORT,'Publication freeze attempt must finish atomically'); END",
            ),
            (
                Some("artifact_versions"),
                "CREATE TRIGGER IF NOT EXISTS trg_frozen_evidence_artifact_version_delete \
             BEFORE DELETE ON artifact_versions \
             WHEN EXISTS(\
               SELECT 1 FROM evidence_bindings binding \
               JOIN publication_revisions revision ON revision.id=binding.revision_id \
               WHERE binding.artifact_version_id=OLD.id \
                 AND revision.state IN ('frozen','published')\
               ) \
             BEGIN SELECT RAISE(ABORT,'ArtifactVersion is retained by frozen Publication evidence'); END",
            ),
            (
                Some("runs"),
                "CREATE TRIGGER IF NOT EXISTS trg_frozen_evidence_run_delete \
             BEFORE DELETE ON runs \
             WHEN EXISTS(\
               SELECT 1 FROM evidence_bindings binding \
               JOIN publication_revisions revision ON revision.id=binding.revision_id \
               WHERE binding.run_id=OLD.id \
                 AND revision.state IN ('frozen','published')\
               ) \
             BEGIN SELECT RAISE(ABORT,'Run is retained by frozen Publication evidence'); END",
            ),
            (
                Some("external_resources"),
                "CREATE TRIGGER IF NOT EXISTS trg_frozen_evidence_external_resource_delete \
             BEFORE DELETE ON external_resources \
             WHEN EXISTS(\
               SELECT 1 FROM evidence_bindings binding \
               JOIN publication_revisions revision ON revision.id=binding.revision_id \
               WHERE binding.external_resource_id=OLD.id \
                 AND revision.state IN ('frozen','published')\
               ) \
             BEGIN SELECT RAISE(ABORT,'ExternalResource is retained by frozen Publication evidence'); END",
            ),
        ] {
            if let Some(table) = base_table {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
                )
                .bind(table)
                .fetch_one(pool)
                .await?;
                if !exists {
                    continue;
                }
            }
            sqlx::query(statement).execute(pool).await?;
        }
        Ok(())
    }

    async fn apply_agent_workflow_lineage(pool: &SqlitePool) -> Result<()> {
        let limits = serde_json::to_string(&AgentDelegationRootLimits::default())?;
        Self::add_columns_if_missing(
            pool,
            "agent_workflows",
            &[
                ("root_workflow_id", "TEXT NOT NULL DEFAULT ''"),
                ("parent_attempt_id", "TEXT"),
                ("depth", "INTEGER NOT NULL DEFAULT 0"),
                (
                    "root_limits_json",
                    "TEXT NOT NULL DEFAULT '{\"max_depth\":1,\"max_tasks\":8,\"max_parallel\":2,\"max_tokens\":256000,\"max_tool_calls\":512,\"max_cost_microunits\":8000000,\"wall_time_secs\":1800}'",
                ),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "agent_workflow_attempts",
            &[
                ("root_workflow_id", "TEXT NOT NULL DEFAULT ''"),
                ("parent_attempt_id", "TEXT"),
                ("depth", "INTEGER NOT NULL DEFAULT 1"),
                ("allow_delegation", "INTEGER NOT NULL DEFAULT 0"),
                ("delegation_slot_yielded", "INTEGER NOT NULL DEFAULT 0"),
            ],
        )
        .await?;
        sqlx::query(
            "UPDATE agent_workflows SET root_workflow_id=id,root_limits_json=? \
             WHERE root_workflow_id='' OR root_workflow_id IS NULL",
        )
        .bind(&limits)
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE agent_workflow_attempts SET root_workflow_id=workflow_id,depth=1 \
             WHERE root_workflow_id='' OR root_workflow_id IS NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_agent_workflows_root_depth \
             ON agent_workflows(root_workflow_id,depth,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_agent_workflow_attempts_root_status \
             ON agent_workflow_attempts(root_workflow_id,status,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_agent_workflow_attempts_parent \
             ON agent_workflow_attempts(parent_attempt_id,created_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_session_execution_contexts(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_execution_contexts (\
             frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
             context_id TEXT NOT NULL REFERENCES execution_contexts(id) ON DELETE CASCADE, \
             created_at INTEGER NOT NULL, PRIMARY KEY(frame_id,context_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_session_execution_contexts_context \
             ON session_execution_contexts(context_id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_agent_workflow_contracts(pool: &SqlitePool) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(agent_workflow_steps)")
            .fetch_all(pool)
            .await?;
        let columns = rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<std::result::Result<std::collections::HashSet<_>, _>>()?;
        for (column, definition) in [
            ("input_contract_json", "TEXT NOT NULL DEFAULT '{}'"),
            ("output_contract_json", "TEXT NOT NULL DEFAULT '{}'"),
            ("budget_json", "TEXT NOT NULL DEFAULT '{}'"),
        ] {
            if columns.contains(column) {
                continue;
            }
            let query =
                format!("ALTER TABLE agent_workflow_steps ADD COLUMN {column} {definition}");
            match sqlx::query(&query).execute(pool).await {
                Ok(_) => {}
                Err(error) if error.to_string().contains("duplicate column name") => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    async fn apply_agent_workflow_plans(pool: &SqlitePool) -> Result<()> {
        Self::add_columns_if_missing(
            pool,
            "agent_workflows",
            &[
                ("frame_id", "TEXT"),
                ("goal", "TEXT NOT NULL DEFAULT ''"),
                ("mode", "TEXT NOT NULL DEFAULT 'manual'"),
                ("status", "TEXT NOT NULL DEFAULT 'draft'"),
                ("max_parallel", "INTEGER NOT NULL DEFAULT 2"),
                ("requires_confirmation", "INTEGER NOT NULL DEFAULT 1"),
                ("plan_json", "TEXT NOT NULL DEFAULT '{}'"),
                ("approved_at", "INTEGER"),
            ],
        )
        .await?;
        Self::add_columns_if_missing(
            pool,
            "agent_workflow_steps",
            &[
                ("template_id", "TEXT NOT NULL DEFAULT ''"),
                ("spec_json", "TEXT NOT NULL DEFAULT '{}'"),
            ],
        )
        .await?;
        sqlx::query("UPDATE agent_workflows SET goal=name WHERE goal='' OR goal IS NULL")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_agent_workflows_frame_status \
             ON agent_workflows(frame_id,status,updated_at DESC)",
        )
        .execute(pool)
        .await?;
        for statement in [
            "CREATE TRIGGER IF NOT EXISTS trg_agent_workflow_steps_insert_draft \
             BEFORE INSERT ON agent_workflow_steps \
             WHEN COALESCE((SELECT status FROM agent_workflows WHERE id=NEW.workflow_id),'missing')<>'draft' \
             BEGIN SELECT RAISE(ABORT,'agent workflow plan is immutable'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_agent_workflow_steps_update_draft \
             BEFORE UPDATE ON agent_workflow_steps \
             WHEN COALESCE((SELECT status FROM agent_workflows WHERE id=OLD.workflow_id),'missing')<>'draft' \
               OR COALESCE((SELECT status FROM agent_workflows WHERE id=NEW.workflow_id),'missing')<>'draft' \
             BEGIN SELECT RAISE(ABORT,'agent workflow plan is immutable'); END",
            "CREATE TRIGGER IF NOT EXISTS trg_agent_workflow_steps_delete_draft \
             BEFORE DELETE ON agent_workflow_steps \
             WHEN COALESCE((SELECT status FROM agent_workflows WHERE id=OLD.workflow_id),'missing')<>'draft' \
             BEGIN SELECT RAISE(ABORT,'agent workflow plan is immutable'); END",
        ] {
            sqlx::query(statement).execute(pool).await?;
        }
        Ok(())
    }

    async fn add_columns_if_missing(
        pool: &SqlitePool,
        table: &str,
        definitions: &[(&str, &str)],
    ) -> Result<()> {
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await?;
        if rows.is_empty() {
            // Table absent (legacy fixtures skip tables their test never
            // touches) — nothing to add columns to.
            return Ok(());
        }
        let columns = rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<std::result::Result<std::collections::HashSet<_>, _>>()?;
        for (column, definition) in definitions {
            if columns.contains(*column) {
                continue;
            }
            let query = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
            match sqlx::query(&query).execute(pool).await {
                Ok(_) => {}
                Err(error) if error.to_string().contains("duplicate column name") => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    async fn apply_message_resource_links(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS message_resource_links (\
             id TEXT PRIMARY KEY, \
             frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
             message_seq INTEGER NOT NULL, ordinal INTEGER NOT NULL, \
             original_reference TEXT NOT NULL, \
             artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL, \
             artifact_version_id TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL, \
             display_name TEXT NOT NULL, resource_kind TEXT NOT NULL, mime_type TEXT NOT NULL, \
             status TEXT NOT NULL, error TEXT, created_artifact INTEGER NOT NULL DEFAULT 0, \
             created_version INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, \
             UNIQUE(frame_id,message_seq,ordinal))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_message_resource_links_message \
             ON message_resource_links(frame_id,message_seq,ordinal)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn migration_applied(pool: &SqlitePool, version: &str) -> Result<bool> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT version FROM wisp_schema_migrations WHERE version=?")
                .bind(version)
                .fetch_optional(pool)
                .await?;
        Ok(row.is_some())
    }

    async fn record_migration(pool: &SqlitePool, version: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO wisp_schema_migrations(version,applied_at) VALUES(?,?)")
            .bind(version)
            .bind(chrono::Utc::now().timestamp())
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn execute_sql_script(pool: &SqlitePool, sql: &str) -> Result<()> {
        // Strip `--` line comments before splitting on `;` so semicolons inside
        // comments don't produce bogus statements.
        let stripped: String = sql
            .lines()
            .map(|l| match l.split_once("--") {
                Some((code, _)) => code.to_string(),
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        for stmt in stripped.split(';') {
            let s = stmt.trim();
            if s.is_empty() {
                continue;
            }
            sqlx::query(s).execute(pool).await?;
        }
        Ok(())
    }

    async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
        let sql = format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?");
        let has: (i64,) = sqlx::query_as(&sql).bind(column).fetch_one(pool).await?;
        Ok(has.0 > 0)
    }

    async fn apply_control_plane_backfill(pool: &SqlitePool) -> Result<()> {
        if !Self::has_column(pool, "projects", "workspace_dir").await? {
            sqlx::query("ALTER TABLE projects ADD COLUMN workspace_dir TEXT NOT NULL DEFAULT ''")
                .execute(pool)
                .await?;
        }
        if !Self::has_column(pool, "messages", "model_name").await? {
            sqlx::query("ALTER TABLE messages ADD COLUMN model_name TEXT")
                .execute(pool)
                .await?;
        }
        if !Self::has_column(pool, "frames", "title").await? {
            sqlx::query("ALTER TABLE frames ADD COLUMN title TEXT")
                .execute(pool)
                .await?;
        }
        if !Self::has_column(pool, "frames", "folder_id").await? {
            sqlx::query("ALTER TABLE frames ADD COLUMN folder_id TEXT")
                .execute(pool)
                .await?;
        }
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_frames_folder ON frames(folder_id)")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS execution_log (\
             id TEXT PRIMARY KEY, frame_id TEXT NOT NULL, cell_index INTEGER NOT NULL, \
             tool TEXT NOT NULL, language TEXT NOT NULL, source TEXT NOT NULL, \
             stdout TEXT, stderr TEXT, exit_status TEXT NOT NULL, wall_s REAL, \
             files_written TEXT NOT NULL, files_read TEXT NOT NULL, env_hash TEXT, \
             created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_execution_log_frame ON execution_log(frame_id, cell_index)",
        ).execute(pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS env_snapshots (\
             hash TEXT PRIMARY KEY, env_name TEXT, packages_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS execution_contexts (\
             id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL, \
             config_json TEXT NOT NULL DEFAULT '{}', capabilities_json TEXT NOT NULL DEFAULT '{}', \
             last_probe_at INTEGER, last_probe_status TEXT, last_probe_error TEXT, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_execution_contexts_kind ON execution_contexts(kind)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS runs (\
             id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             frame_id TEXT, context_id TEXT NOT NULL, title TEXT NOT NULL, kind TEXT NOT NULL, \
             status TEXT NOT NULL, command TEXT, script_path TEXT, \
             input_refs_json TEXT NOT NULL DEFAULT '[]', output_specs_json TEXT NOT NULL DEFAULT '[]', \
             created_at INTEGER NOT NULL, started_at INTEGER, ended_at INTEGER, exit_code INTEGER, \
             stdout_tail TEXT, stderr_tail TEXT, remote_workdir TEXT, \
             remote_handle_json TEXT, timeout_secs INTEGER, last_polled_at INTEGER, last_poll_error TEXT, \
             lifecycle_owner TEXT, lifecycle_lease_until INTEGER, \
             progress_json TEXT NOT NULL DEFAULT '{}', env_snapshot_json TEXT NOT NULL DEFAULT '{}')",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_runs_project ON runs(project_id, created_at)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_runs_context ON runs(context_id)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS run_artifacts (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE, \
             artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE, \
             role TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_run_artifacts_run ON run_artifacts(run_id)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS research_nodes (\
             id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             kind TEXT NOT NULL, title TEXT NOT NULL, ref_id TEXT, \
             metadata_json TEXT NOT NULL DEFAULT '{}', \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_research_nodes_project ON research_nodes(project_id, kind)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS research_edges (\
             id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             source_id TEXT NOT NULL REFERENCES research_nodes(id) ON DELETE CASCADE, \
             target_id TEXT NOT NULL REFERENCES research_nodes(id) ON DELETE CASCADE, \
             relation TEXT NOT NULL, metadata_json TEXT NOT NULL DEFAULT '{}', \
             created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_research_edges_project ON research_edges(project_id, source_id, target_id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_remote_staging(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS remote_staging (\
             id TEXT PRIMARY KEY, \
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             context_id TEXT NOT NULL, run_id TEXT, \
             remote_path TEXT NOT NULL, source TEXT NOT NULL, \
             checksum TEXT, size_bytes INTEGER, \
             created_at INTEGER NOT NULL, removed_at INTEGER)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_remote_staging_ctx \
             ON remote_staging(context_id, removed_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_context_storage_prefs(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS context_storage_prefs (\
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             context_id TEXT NOT NULL, \
             remote_data_root TEXT NOT NULL, \
             remote_workdir_root TEXT NOT NULL, \
             local_results_dir TEXT NOT NULL, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             PRIMARY KEY (project_id, context_id))",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_ssh_run_control(pool: &SqlitePool) -> Result<()> {
        for (column, definition) in [
            ("remote_handle_json", "TEXT"),
            ("timeout_secs", "INTEGER"),
            ("last_polled_at", "INTEGER"),
            ("last_poll_error", "TEXT"),
        ] {
            if !Self::has_column(pool, "runs", column).await? {
                sqlx::query(&format!(
                    "ALTER TABLE runs ADD COLUMN {column} {definition}"
                ))
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn apply_run_lifecycle_lease(pool: &SqlitePool) -> Result<()> {
        for (column, definition) in [
            ("lifecycle_owner", "TEXT"),
            ("lifecycle_lease_until", "INTEGER"),
        ] {
            if !Self::has_column(pool, "runs", column).await? {
                sqlx::query(&format!(
                    "ALTER TABLE runs ADD COLUMN {column} {definition}"
                ))
                .execute(pool)
                .await?;
            }
        }
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_runs_active_lease \
             ON runs(status, lifecycle_lease_until)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_proposed_plans(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS proposed_plans (\
             id TEXT PRIMARY KEY, \
             frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
             codex_thread_id TEXT, codex_turn_id TEXT, revision INTEGER NOT NULL, \
             markdown TEXT NOT NULL, status TEXT NOT NULL, \
             mode TEXT NOT NULL DEFAULT 'native', \
             progress_json TEXT NOT NULL DEFAULT '[]', \
             runtime_config_json TEXT NOT NULL DEFAULT '{}', \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             UNIQUE(frame_id, revision))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_proposed_plans_frame \
             ON proposed_plans(frame_id, revision DESC)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_codex_turn_configs(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS codex_turn_configs (\
             id TEXT PRIMARY KEY, \
             frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE, \
             codex_thread_id TEXT, codex_turn_id TEXT, mode TEXT NOT NULL, \
             config_version INTEGER NOT NULL DEFAULT 0, config_version_text TEXT NOT NULL DEFAULT '', requested_json TEXT NOT NULL, \
             effective_json TEXT NOT NULL, actual_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             UNIQUE(frame_id, codex_turn_id))",
        )
        .execute(pool)
        .await?;
        if !Self::has_column(pool, "codex_turn_configs", "config_version_text").await? {
            sqlx::query(
                "ALTER TABLE codex_turn_configs ADD COLUMN config_version_text TEXT NOT NULL DEFAULT ''",
            )
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_codex_turn_configs_frame \
             ON codex_turn_configs(frame_id, created_at DESC)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_acp_sessions(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS acp_sessions (\
             frame_id TEXT PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE, \
             agent_profile_id TEXT NOT NULL, profile_fingerprint TEXT NOT NULL, \
             agent_session_id TEXT NOT NULL, cwd TEXT NOT NULL, \
             protocol_version INTEGER NOT NULL, \
             agent_info_json TEXT NOT NULL DEFAULT '{}', \
             capabilities_json TEXT NOT NULL DEFAULT '{}', \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             UNIQUE(agent_profile_id, agent_session_id))",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_artifact_lineage(pool: &SqlitePool) -> Result<()> {
        if !Self::has_column(pool, "artifacts", "latest_version_id").await? {
            sqlx::query("ALTER TABLE artifacts ADD COLUMN latest_version_id TEXT")
                .execute(pool)
                .await?;
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS artifact_versions (\
             id TEXT PRIMARY KEY, artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE, \
             version_number INTEGER NOT NULL, content_type TEXT NOT NULL, storage_path TEXT NOT NULL, \
             size_bytes INTEGER, checksum TEXT, parent_version_id TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL, \
             producing_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL, \
             env_snapshot_hash TEXT REFERENCES env_snapshots(hash) ON DELETE SET NULL, \
             created_at INTEGER NOT NULL, UNIQUE(artifact_id, version_number))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_artifact_versions_artifact \
             ON artifact_versions(artifact_id, version_number DESC)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_artifact_versions_run \
             ON artifact_versions(producing_run_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS artifact_dependencies (\
             id TEXT PRIMARY KEY, artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE, \
             depends_on_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE, \
             reference_name TEXT, created_at INTEGER NOT NULL, \
             UNIQUE(artifact_version_id, depends_on_version_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_artifact_dependencies_version \
             ON artifact_dependencies(artifact_version_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO artifact_versions(\
                id,artifact_id,version_number,content_type,storage_path,created_at\
             ) SELECT 'legacy-' || id,id,1,content_type,storage_path,created_at FROM artifacts",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE artifacts SET latest_version_id=(\
                SELECT id FROM artifact_versions v WHERE v.artifact_id=artifacts.id \
                ORDER BY version_number DESC LIMIT 1\
             ) WHERE latest_version_id IS NULL",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn schema_migrations(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT version FROM wisp_schema_migrations ORDER BY applied_at, version",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(version,)| version).collect())
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(key).bind(value)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }
}

#[cfg(test)]
mod store_tests;
