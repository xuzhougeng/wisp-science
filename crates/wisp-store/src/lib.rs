//! SQLite persistence for Wisp: projects, frames, messages, settings.
//!
//! Replaces the mangopi JSON session file with a structured store. API keys
//! live in the OS keyring (see [`secrets`]); everything else lives here.

mod acp_sessions;
mod agent_workflow_attempts;
mod agent_workflow_deliveries;
mod agent_workflows;
mod artifacts;
mod execution_contexts;
mod library;
mod models;
mod project_sync;
mod project_transfer;
mod projects;
mod provenance;
mod research;
mod resources;
mod runs;
pub mod secrets;
mod sessions;

pub use acp_sessions::AcpSessionBinding;
pub use agent_workflow_attempts::{
    AgentWorkflowAttempt, AgentWorkflowAttemptStart, AgentWorkflowAttemptStatus,
};
pub use agent_workflow_deliveries::{AgentWorkflowDelivery, AGENT_WORKFLOW_COMPLETION_TOOL};
pub use agent_workflows::{
    AgentDelegationRootLimits, AgentWorkflow, AgentWorkflowStatus, AgentWorkflowStep,
    MAX_ROOT_AGENT_DEPTH, MAX_ROOT_AGENT_TASKS,
};
pub use library::{LibraryItem, LibraryItemDetail, LibraryStore, NewLibraryItem};
pub use models::*;
pub use project_sync::ProjectSyncState;
pub use project_transfer::ProjectTransferStats;
pub use sessions::SessionTranscriptPage;

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

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (or create) the SQLite database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Self> {
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
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        Self::migrate(&pool).await?;
        let store = Self { pool };
        store
            .upsert_execution_context(&ExecutionContext::new("local", "Local")?)
            .await?;
        Ok(store)
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
             status TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL, \
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
