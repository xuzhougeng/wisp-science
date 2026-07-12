//! SQLite persistence for Wisp: projects, frames, messages, settings.
//!
//! Replaces the mangopi JSON session file with a structured store. API keys
//! live in the OS keyring (see [`secrets`]); everything else lives here.

mod acp_sessions;
mod artifacts;
mod execution_contexts;
mod lab;
mod models;
mod projects;
mod provenance;
mod research;
mod runs;
pub mod secrets;
mod sessions;

pub use acp_sessions::AcpSessionBinding;
pub use models::*;

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
const LAB_REGISTRY_MIGRATION: &str = "0008_lab_registry_v0";
const LAB_TRANSACTION_MIGRATION: &str = "0009_lab_transactions";
const LAB_RESOURCE_DEFINITION_MIGRATION: &str = "0010_lab_resource_definitions";
const LAB_INVENTORY_MIGRATION: &str = "0011_lab_inventory";
const LAB_LOCATIONS_MIGRATION: &str = "0012_lab_locations";
const LAB_DOCUMENTS_MIGRATION: &str = "0013_lab_documents";
const LAB_WET_RUNS_MIGRATION: &str = "0014_lab_wet_runs";
const LAB_PROTOCOL_REVISIONS_MIGRATION: &str = "0015_lab_protocol_revisions";
const LAB_RUN_PARTICIPANTS_MIGRATION: &str = "0016_lab_run_participants";
const LAB_RESERVATIONS_MIGRATION: &str = "0017_lab_reservations";
const LAB_QC_MIGRATION: &str = "0018_lab_qc";
const LAB_CONVERSATION_RUN_MIGRATION: &str = "0019_lab_conversation_run";
const LAB_RUN_DEVIATIONS_MIGRATION: &str = "0020_lab_run_deviations";
const LAB_DATA_EVIDENCE_MIGRATION: &str = "0021_lab_data_evidence";
const LAB_SUBJECTS_MIGRATION: &str = "0022_lab_subjects";
const LAB_DERIVATIONS_MIGRATION: &str = "0023_lab_derivations";
const LAB_AMENDMENTS_MIGRATION: &str = "0024_lab_amendments";
const LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION: &str = "0025_lab_scoped_aux_display_ids";

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
            .create_if_missing(true);
        Self::preflight_foreign_keys(opts.clone()).await?;
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts.foreign_keys(true))
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
        if !Self::migration_applied(pool, LAB_REGISTRY_MIGRATION).await? {
            Self::apply_lab_registry_v0(pool).await?;
            Self::record_migration(pool, LAB_REGISTRY_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_TRANSACTION_MIGRATION).await? {
            Self::apply_lab_transactions(pool).await?;
            Self::record_migration(pool, LAB_TRANSACTION_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_RESOURCE_DEFINITION_MIGRATION).await? {
            Self::apply_lab_resource_definitions(pool).await?;
            Self::record_migration(pool, LAB_RESOURCE_DEFINITION_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_INVENTORY_MIGRATION).await? {
            Self::apply_lab_inventory(pool).await?;
            Self::record_migration(pool, LAB_INVENTORY_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_LOCATIONS_MIGRATION).await? {
            Self::apply_lab_locations(pool).await?;
            Self::record_migration(pool, LAB_LOCATIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_DOCUMENTS_MIGRATION).await? {
            Self::apply_lab_documents(pool).await?;
            Self::record_migration(pool, LAB_DOCUMENTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_WET_RUNS_MIGRATION).await? {
            Self::apply_lab_wet_runs(pool).await?;
            Self::record_migration(pool, LAB_WET_RUNS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_PROTOCOL_REVISIONS_MIGRATION).await? {
            Self::apply_lab_protocol_revisions(pool).await?;
            Self::record_migration(pool, LAB_PROTOCOL_REVISIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_RUN_PARTICIPANTS_MIGRATION).await? {
            Self::apply_lab_run_participants(pool).await?;
            Self::record_migration(pool, LAB_RUN_PARTICIPANTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_RESERVATIONS_MIGRATION).await? {
            Self::apply_lab_reservations(pool).await?;
            Self::record_migration(pool, LAB_RESERVATIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_QC_MIGRATION).await? {
            Self::apply_lab_qc(pool).await?;
            Self::record_migration(pool, LAB_QC_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_CONVERSATION_RUN_MIGRATION).await? {
            Self::apply_lab_conversation_run(pool).await?;
            Self::record_migration(pool, LAB_CONVERSATION_RUN_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_RUN_DEVIATIONS_MIGRATION).await? {
            Self::apply_lab_run_deviations(pool).await?;
            Self::record_migration(pool, LAB_RUN_DEVIATIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_DATA_EVIDENCE_MIGRATION).await? {
            Self::apply_lab_data_evidence(pool).await?;
            Self::record_migration(pool, LAB_DATA_EVIDENCE_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_SUBJECTS_MIGRATION).await? {
            Self::apply_lab_subjects(pool).await?;
            Self::record_migration(pool, LAB_SUBJECTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_DERIVATIONS_MIGRATION).await? {
            Self::apply_lab_derivations(pool).await?;
            Self::record_migration(pool, LAB_DERIVATIONS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_AMENDMENTS_MIGRATION).await? {
            Self::apply_lab_amendments(pool).await?;
            Self::record_migration(pool, LAB_AMENDMENTS_MIGRATION).await?;
        }
        if !Self::migration_applied(pool, LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION).await? {
            Self::apply_lab_scoped_aux_display_ids(pool).await?;
            Self::record_migration(pool, LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION).await?;
        }
        Ok(())
    }

    async fn preflight_foreign_keys(opts: SqliteConnectOptions) -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        let rows = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await?;
        let violation = rows.first().map(|row| {
            let table: String = row.try_get("table").unwrap_or_else(|_| "unknown".into());
            let rowid: Option<i64> = row.try_get("rowid").ok();
            let parent: String = row.try_get("parent").unwrap_or_else(|_| "unknown".into());
            format!("table={table}, rowid={rowid:?}, parent={parent}")
        });
        pool.close().await;
        if let Some(violation) = violation {
            anyhow::bail!(
                "SQLite foreign-key preflight failed ({violation}). Repair the database before upgrading."
            );
        }
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
        sqlx::query("INSERT INTO wisp_schema_migrations(version,applied_at) VALUES(?,?)")
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
             env_snapshot_json TEXT NOT NULL DEFAULT '{}')",
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

    async fn apply_lab_registry_v0(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_registries (\
             id TEXT PRIMARY KEY, name TEXT NOT NULL, root_path TEXT, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS project_lab_registries (\
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             created_at INTEGER NOT NULL, PRIMARY KEY(project_id,registry_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_project_lab_registries_registry \
             ON project_lab_registries(registry_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_entities (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             display_id TEXT NOT NULL, kind TEXT NOT NULL, subtype TEXT, title TEXT NOT NULL, \
             revision INTEGER NOT NULL CHECK(revision>0), metadata_json TEXT NOT NULL DEFAULT '{}', \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
             UNIQUE(registry_id,display_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_entities_registry_kind \
             ON lab_entities(registry_id,kind,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_entity_projects (\
             entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
             relation TEXT NOT NULL, created_at INTEGER NOT NULL, \
             PRIMARY KEY(entity_id,project_id,relation))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_entity_projects_project \
             ON lab_entity_projects(project_id,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_id_counters (\
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             prefix TEXT NOT NULL, next_value INTEGER NOT NULL CHECK(next_value>0), \
             PRIMARY KEY(registry_id,prefix))",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_transactions(pool: &SqlitePool) -> Result<()> {
        if !Self::has_column(pool, "lab_entities", "last_transaction_id").await? {
            sqlx::query("ALTER TABLE lab_entities ADD COLUMN last_transaction_id TEXT")
                .execute(pool)
                .await?;
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_transactions (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             project_id TEXT REFERENCES projects(id) ON DELETE SET NULL, \
             command_id TEXT NOT NULL, schema_version INTEGER NOT NULL CHECK(schema_version>0), \
             actor_kind TEXT NOT NULL, actor_ref TEXT, confirmation_json TEXT NOT NULL, \
             request_json TEXT NOT NULL, receipt_json TEXT NOT NULL, status TEXT NOT NULL, \
             created_at INTEGER NOT NULL, committed_at INTEGER NOT NULL, \
             UNIQUE(registry_id,command_id), UNIQUE(registry_id,display_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_transactions_project \
             ON lab_transactions(project_id,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_events (\
             id TEXT PRIMARY KEY, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             project_id TEXT REFERENCES projects(id) ON DELETE SET NULL, \
             transaction_id TEXT NOT NULL REFERENCES lab_transactions(id) ON DELETE RESTRICT, \
             sequence INTEGER NOT NULL CHECK(sequence>0), \
             entity_id TEXT REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             prior_event_id TEXT REFERENCES lab_events(id) ON DELETE RESTRICT, \
             kind TEXT NOT NULL, schema_version INTEGER NOT NULL CHECK(schema_version>0), \
             payload_json TEXT NOT NULL, occurred_at INTEGER NOT NULL, recorded_at INTEGER NOT NULL, \
             expected_revision INTEGER, resulting_revision INTEGER, reason TEXT, \
             UNIQUE(transaction_id,sequence))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_events_entity ON lab_events(entity_id,recorded_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_events_registry ON lab_events(registry_id,recorded_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_resource_definitions(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_resource_definitions (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             category TEXT NOT NULL, supplier TEXT, catalog_number TEXT, \
             attributes_json TEXT NOT NULL DEFAULT '{}')",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_aliases (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             alias_type TEXT NOT NULL, namespace TEXT, value TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_aliases_lookup ON lab_aliases(registry_id,value)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_lab_alias_barcode \
             ON lab_aliases(registry_id,value) WHERE alias_type='barcode'",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_lab_alias_namespaced_identity \
             ON lab_aliases(registry_id,alias_type,namespace,value) \
             WHERE alias_type IN ('legacy_id','internal_id')",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_inventory(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_lots (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             resource_definition_id TEXT NOT NULL REFERENCES lab_resource_definitions(entity_id) ON DELETE RESTRICT, \
             supplier TEXT, catalog_number TEXT, lot_number TEXT NOT NULL, \
             received_at INTEGER, expiry_at INTEGER, origin_kind TEXT NOT NULL, \
             CHECK(origin_kind IN ('receipt','prepared','legacy_import')), \
             CHECK(expiry_at IS NULL OR received_at IS NULL OR expiry_at >= received_at))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_lab_lots_vendor_identity \
             ON lab_lots(registry_id,supplier,catalog_number,lot_number) \
             WHERE supplier IS NOT NULL AND catalog_number IS NOT NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_material_units (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             lot_id TEXT REFERENCES lab_lots(entity_id) ON DELETE RESTRICT, \
             usage_class TEXT NOT NULL CHECK(usage_class IN ('inventory','sample')), \
             quantity_state TEXT NOT NULL CHECK(quantity_state IN ('measured','unknown','not_measured')), \
             quantity_value TEXT, quantity_unit TEXT, vessel_description TEXT, \
             availability TEXT NOT NULL CHECK(availability IN ('available','quarantined','depleted','disposed')), \
             origin_kind TEXT NOT NULL CHECK(origin_kind IN ('receipt','prepared','legacy_import')), \
             CHECK((quantity_state='measured' AND quantity_value IS NOT NULL AND quantity_unit IS NOT NULL) \
                OR (quantity_state!='measured' AND quantity_value IS NULL AND quantity_unit IS NULL)))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_material_units_lot ON lab_material_units(lot_id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_locations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_locations (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             parent_location_id TEXT REFERENCES lab_locations(entity_id) ON DELETE RESTRICT, \
             location_class TEXT NOT NULL, single_occupancy INTEGER NOT NULL CHECK(single_occupancy IN (0,1)))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_locations_parent ON lab_locations(registry_id,parent_location_id)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_material_locations (\
             material_unit_id TEXT PRIMARY KEY REFERENCES lab_material_units(entity_id) ON DELETE RESTRICT, \
             location_id TEXT NOT NULL REFERENCES lab_locations(entity_id) ON DELETE RESTRICT, \
             established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_material_locations_location ON lab_material_locations(location_id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_documents(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_documents (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             entity_id TEXT NOT NULL UNIQUE REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             relative_path TEXT NOT NULL, schema_version INTEGER NOT NULL CHECK(schema_version>0), \
             narrative_markdown TEXT NOT NULL, extension_json TEXT NOT NULL DEFAULT '{}', \
             last_projected_content TEXT, revision INTEGER NOT NULL CHECK(revision>0), \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_lab_documents_path ON lab_documents(registry_id,relative_path)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_projection_outbox (\
             id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES lab_documents(id) ON DELETE RESTRICT, \
             target_path TEXT NOT NULL, content TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, \
             last_error TEXT, created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_projection_outbox_pending ON lab_projection_outbox(created_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_wet_runs(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_wet_runs (\
             run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE RESTRICT, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             display_id TEXT NOT NULL, command_id TEXT NOT NULL, operator TEXT, protocol_revision_id TEXT, \
             deviations_json TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL, \
             UNIQUE(registry_id,display_id), UNIQUE(registry_id,command_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_wet_runs_registry ON lab_wet_runs(registry_id,created_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_protocol_revisions(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_protocol_revisions (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             protocol_entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             revision_number INTEGER NOT NULL CHECK(revision_number>0), checksum_sha256 TEXT NOT NULL, \
             content TEXT NOT NULL, created_at INTEGER NOT NULL, \
             UNIQUE(protocol_entity_id,revision_number), UNIQUE(protocol_entity_id,checksum_sha256))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_protocol_revisions_registry ON lab_protocol_revisions(registry_id,protocol_entity_id,revision_number DESC)",
        )
        .execute(pool)
        .await?;
        if !Self::has_column(pool, "lab_wet_runs", "protocol_revision_id").await? {
            sqlx::query("ALTER TABLE lab_wet_runs ADD COLUMN protocol_revision_id TEXT")
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    async fn apply_lab_run_participants(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_run_participants (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, \
             material_unit_id TEXT NOT NULL REFERENCES lab_material_units(entity_id) ON DELETE RESTRICT, \
             direction TEXT NOT NULL CHECK(direction IN ('input','output')), role TEXT NOT NULL, effect TEXT NOT NULL, \
             quantity_state TEXT, quantity_value TEXT, quantity_unit TEXT, transformation_group TEXT, \
             established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL, \
             CHECK((quantity_state IS NULL AND quantity_value IS NULL AND quantity_unit IS NULL) \
               OR (quantity_state='measured' AND quantity_value IS NOT NULL AND quantity_unit IS NOT NULL) \
               OR (quantity_state IN ('unknown','not_measured') AND quantity_value IS NULL AND quantity_unit IS NULL)))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_run_participants_run ON lab_run_participants(run_id,direction,created_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_run_participants_material ON lab_run_participants(material_unit_id,created_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_reservations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_reservations (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, \
             material_unit_id TEXT NOT NULL REFERENCES lab_material_units(entity_id) ON DELETE RESTRICT, \
             quantity_value TEXT NOT NULL, quantity_unit TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('active','released','expired')), \
             expires_at INTEGER, created_at INTEGER NOT NULL, released_at INTEGER)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_reservations_material_active ON lab_reservations(material_unit_id,status,expires_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_reservations_run ON lab_reservations(run_id,status)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_qc(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_qc_observations (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             run_id TEXT REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, method_revision_id TEXT REFERENCES lab_protocol_revisions(id) ON DELETE RESTRICT, \
             measurement_json TEXT NOT NULL, evidence_json TEXT NOT NULL, observed_at INTEGER NOT NULL, recorded_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_qc_observations_entity ON lab_qc_observations(entity_id,observed_at)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_qc_assessments (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, observation_ids_json TEXT NOT NULL, \
             criteria_json TEXT NOT NULL, verdict TEXT NOT NULL CHECK(verdict IN ('pass','fail','inconclusive')), \
             rationale TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_qc_assessments_entity ON lab_qc_assessments(entity_id,created_at)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// A conversation is the durable capture surface for one themed wet-lab
    /// experiment. `runs.frame_id` remains nullable for imported/legacy Runs,
    /// but a saved conversation can never silently acquire a second wet-lab
    /// Run.
    async fn apply_lab_conversation_run(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_wet_lab_runs_conversation \
             ON runs(frame_id) WHERE kind='wet_lab' AND frame_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_run_deviations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_run_deviations (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, \
             step_ref TEXT, description TEXT NOT NULL, impact TEXT NOT NULL \
             CHECK(impact IN ('none','minor','major','unknown')), disposition TEXT, \
             occurred_at INTEGER NOT NULL, recorded_at INTEGER NOT NULL, \
             established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_run_deviations_run \
             ON lab_run_deviations(run_id,occurred_at,id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_data_evidence(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_data_evidence (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             owner_project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT, \
             owner_registry_id TEXT REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             producing_run_id TEXT REFERENCES runs(id) ON DELETE RESTRICT, \
             role TEXT NOT NULL, uri TEXT NOT NULL, format TEXT, size_bytes INTEGER, \
             checksum_sha256 TEXT, origin TEXT NOT NULL, manifest_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL, established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             CHECK ((owner_project_id IS NOT NULL) != (owner_registry_id IS NOT NULL)), \
             CHECK (size_bytes IS NULL OR size_bytes >= 0), \
             UNIQUE(registry_id,display_id))",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_lab_data_evidence_run ON lab_data_evidence(producing_run_id,created_at,id)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn apply_lab_subjects(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_subjects (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             species TEXT NOT NULL, strain TEXT, sex TEXT CHECK(sex IS NULL OR sex IN ('female','male','unknown')), \
             date_of_birth TEXT, origin_kind TEXT NOT NULL CHECK(origin_kind IN ('birth','receipt','legacy_import')), \
             established_event_id TEXT NOT NULL UNIQUE REFERENCES lab_events(id) ON DELETE RESTRICT)",
        ).execute(pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_subject_participants (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, \
             subject_id TEXT NOT NULL REFERENCES lab_subjects(entity_id) ON DELETE RESTRICT, role TEXT NOT NULL, \
             effect TEXT NOT NULL CHECK(effect IN ('observed','handled','sample_collected')), \
             established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL, \
             UNIQUE(run_id,subject_id,role,effect))",
        ).execute(pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_lab_subject_participants_run ON lab_subject_participants(run_id,created_at,id)")
            .execute(pool).await?;
        Ok(())
    }

    async fn apply_lab_derivations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_material_derivations (\
             id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, \
             operation TEXT NOT NULL CHECK(operation IN ('split','aliquot','merge','pool','passage','transform')), \
             group_id TEXT NOT NULL, parent_material_unit_id TEXT NOT NULL REFERENCES lab_material_units(entity_id) ON DELETE RESTRICT, \
             child_material_unit_id TEXT NOT NULL REFERENCES lab_material_units(entity_id) ON DELETE RESTRICT, \
             established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL, \
             CHECK(parent_material_unit_id <> child_material_unit_id), UNIQUE(group_id,parent_material_unit_id,child_material_unit_id))",
        ).execute(pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_lab_derivations_parent ON lab_material_derivations(parent_material_unit_id,created_at,id)")
            .execute(pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_lab_derivations_child ON lab_material_derivations(child_material_unit_id,created_at,id)")
            .execute(pool).await?;
        Ok(())
    }

    async fn apply_lab_amendments(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS lab_amendments (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, original_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             reason TEXT NOT NULL, correction_json TEXT NOT NULL, affected_ids_json TEXT NOT NULL, \
             established_event_id TEXT NOT NULL UNIQUE REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL, \
             UNIQUE(registry_id,display_id))",
        ).execute(pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS ix_lab_amendments_run ON lab_amendments(run_id,created_at,id)")
            .execute(pool).await?;
        Ok(())
    }

    async fn apply_lab_scoped_aux_display_ids(pool: &SqlitePool) -> Result<()> {
        let evidence_has_registry =
            Self::has_column(pool, "lab_data_evidence", "registry_id").await?;
        let evidence_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='lab_data_evidence'",
        )
        .fetch_one(pool)
        .await?;
        let amendments_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='lab_amendments'",
        )
        .fetch_one(pool)
        .await?;
        let evidence_needs_rebuild =
            !evidence_has_registry || evidence_sql.contains("display_id TEXT NOT NULL UNIQUE");
        let amendments_need_rebuild = amendments_sql.contains("display_id TEXT NOT NULL UNIQUE");
        if !evidence_needs_rebuild && !amendments_need_rebuild {
            return Ok(());
        }
        if !evidence_has_registry {
            sqlx::query(
                "ALTER TABLE lab_data_evidence ADD COLUMN registry_id TEXT REFERENCES lab_registries(id) ON DELETE RESTRICT",
            )
            .execute(pool)
            .await?;
        }
        let mut tx = pool.begin().await?;
        sqlx::query(
            "UPDATE lab_data_evidence SET registry_id=owner_registry_id \
             WHERE registry_id IS NULL AND owner_registry_id IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE lab_data_evidence SET registry_id=(\
                SELECT registry_id FROM lab_wet_runs WHERE run_id=lab_data_evidence.producing_run_id) \
             WHERE registry_id IS NULL AND producing_run_id IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE lab_data_evidence SET registry_id=(\
                SELECT MIN(registry_id) FROM project_lab_registries \
                WHERE project_id=lab_data_evidence.owner_project_id) \
             WHERE registry_id IS NULL AND owner_project_id IS NOT NULL AND (\
                SELECT COUNT(*) FROM project_lab_registries \
                WHERE project_id=lab_data_evidence.owner_project_id)=1",
        )
        .execute(&mut *tx)
        .await?;
        let unresolved: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM lab_data_evidence WHERE registry_id IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        if unresolved != 0 {
            anyhow::bail!(
                "Cannot migrate {unresolved} lab data evidence records with ambiguous Registry ownership"
            );
        }
        sqlx::query(
            "CREATE TABLE lab_data_evidence_scoped (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             owner_project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT, \
             owner_registry_id TEXT REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             producing_run_id TEXT REFERENCES runs(id) ON DELETE RESTRICT, \
             role TEXT NOT NULL, uri TEXT NOT NULL, format TEXT, size_bytes INTEGER, \
             checksum_sha256 TEXT, origin TEXT NOT NULL, manifest_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL, established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             CHECK ((owner_project_id IS NOT NULL) != (owner_registry_id IS NOT NULL)), \
             CHECK (size_bytes IS NULL OR size_bytes >= 0), \
             UNIQUE(registry_id,display_id))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO lab_data_evidence_scoped(\
                id,display_id,registry_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id) \
             SELECT id,display_id,registry_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id \
             FROM lab_data_evidence",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE lab_data_evidence")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE lab_data_evidence_scoped RENAME TO lab_data_evidence")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE INDEX ix_lab_data_evidence_run ON lab_data_evidence(producing_run_id,created_at,id)",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE TABLE lab_amendments_scoped (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, original_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             reason TEXT NOT NULL, correction_json TEXT NOT NULL, affected_ids_json TEXT NOT NULL, \
             established_event_id TEXT NOT NULL UNIQUE REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL, \
             UNIQUE(registry_id,display_id))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO lab_amendments_scoped(id,display_id,registry_id,run_id,original_event_id,reason,correction_json,affected_ids_json,established_event_id,created_at) \
             SELECT id,display_id,registry_id,run_id,original_event_id,reason,correction_json,affected_ids_json,established_event_id,created_at FROM lab_amendments",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE lab_amendments")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE lab_amendments_scoped RENAME TO lab_amendments")
            .execute(&mut *tx)
            .await?;
        sqlx::query("CREATE INDEX ix_lab_amendments_run ON lab_amendments(run_id,created_at,id)")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
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
