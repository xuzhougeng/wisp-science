//! Bounded retention for `run_code_snapshots`: the command, git commit and
//! uncommitted patch (up to 1 MB) recorded when a Run is created. Nothing
//! ever removed a row, and one project grew the table to 584 MB (#1689,
//! #1690 W6.4). Background maintenance keeps the snapshots of the newest
//! [`RUN_CODE_SNAPSHOT_RUNS_KEPT`] Runs of each project and of every Run that
//! is still needed, and drops the rest. The Run itself, its outputs,
//! environment snapshot and logs stay; only the code snapshot rows go.
use anyhow::Result;
use sqlx::SqlitePool;

/// Runs per project whose code snapshots are kept whatever their age.
pub const RUN_CODE_SNAPSHOT_RUNS_KEPT: i64 = 200;

/// Runs handled by one statement, so the first pass over a large table does
/// not hold one transaction, and one WAL, for all of it.
const BATCH_RUNS: i64 = 100;

/// Snapshot rows of the oldest Runs, up to `?2` of them, that are neither
/// among the newest `?1` of their project nor still needed. A Run is still
/// needed while it has not ended (draft, submitted, running, paused,
/// cancelling), while it belongs to an exploration round that was neither
/// promoted nor abandoned or whose promotion is unfinished, and while a
/// publication cites it as evidence or reproduces it. Runs that already lost
/// their snapshots are not candidates, so an empty result means nothing is
/// left to do and a second pass over the same data changes nothing.
const PRUNE: &str = "\
    WITH ranked AS (\
      SELECT id, ROW_NUMBER() OVER (\
        PARTITION BY project_id ORDER BY created_at DESC, id DESC\
      ) AS position FROM runs\
    ) \
    DELETE FROM run_code_snapshots WHERE run_id IN (\
      SELECT run.id FROM runs run JOIN ranked ON ranked.id=run.id \
      WHERE ranked.position > ?1 \
        AND run.status IN ('succeeded','failed','cancelled','timed_out','lost') \
        AND NOT EXISTS (SELECT 1 FROM explorations e WHERE e.id=run.exploration_id) \
        AND NOT EXISTS (\
          SELECT 1 FROM exploration_promotions p WHERE p.exploration_id=run.exploration_id \
          AND p.status IN ('prepared','files_applied','metadata_committed','committed')\
        ) \
        AND NOT EXISTS (SELECT 1 FROM evidence_bindings b WHERE b.run_id=run.id) \
        AND NOT EXISTS (SELECT 1 FROM reproduction_runs r WHERE r.source_run_id=run.id) \
        AND EXISTS (SELECT 1 FROM run_code_snapshots s WHERE s.run_id=run.id) \
      ORDER BY run.created_at, run.id LIMIT ?2\
    )";

/// Delete the code snapshots this database no longer needs, `batch` Runs at
/// a time. Returns the number of rows deleted; zero when nothing was due.
async fn prune(pool: &SqlitePool, kept: i64, batch: i64) -> Result<u64> {
    let mut deleted = 0;
    loop {
        let affected = sqlx::query(PRUNE)
            .bind(kept)
            .bind(batch)
            .execute(pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Ok(deleted);
        }
        deleted += affected;
        tokio::task::yield_now().await;
    }
}

impl crate::Store {
    /// Drop the code snapshots this store's own database no longer needs
    /// (see the module docs). Idempotent; returns the rows deleted.
    pub async fn prune_run_code_snapshots(&self) -> Result<u64> {
        prune(&self.pool, RUN_CODE_SNAPSHOT_RUNS_KEPT, BATCH_RUNS).await
    }

    /// Prune the application database and every project database already
    /// opened by this process, like `maintain_daily_backups`: handles are
    /// cloned under the registry mutex and released before any deletion.
    pub async fn maintain_run_code_snapshots(&self) {
        let mut pools = vec![(None, self.pool.clone())];
        if let Some(registry) = &self.registry {
            pools.extend(
                registry
                    .pools
                    .lock()
                    .await
                    .iter()
                    .map(|(id, pool)| (Some(id.clone()), pool.clone())),
            );
        }
        for (project, pool) in pools {
            match prune(&pool, RUN_CODE_SNAPSHOT_RUNS_KEPT, BATCH_RUNS).await {
                Ok(0) => {}
                Ok(deleted) => {
                    tracing::info!(project = ?project, deleted, "Pruned run code snapshots")
                }
                Err(error) => tracing::warn!(%error, "Run code snapshot pruning failed"),
            }
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextArchiveRecord, EvidenceBindingDraft, EvidenceSelectionState, EvidenceSourceKind,
        EvidenceVisibility, Exploration, ExplorationCheckpoint, ExplorationFamily,
        ExplorationPromotion, ExplorationPromotionStatus, ExplorationStatus, ReproductionRunStart,
        RunRecord, Store, WorkspaceSnapshotRecord,
    };
    use wisp_llm::Message;

    async fn store() -> (Store, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("wisp.sqlite")).await.unwrap();
        store
            .create_project("p", "Project", "/tmp/project")
            .await
            .unwrap();
        store
            .create_frame("main", "p", "OPERON", "model")
            .await
            .unwrap();
        (store, root)
    }

    /// A Run that ended, with the code snapshot `create_run` records.
    async fn ended_run(store: &Store, id: &str, project: &str, created_at: i64) {
        run_in_frame(store, id, project, None, created_at).await;
        end(store, id).await;
    }

    async fn run_in_frame(
        store: &Store,
        id: &str,
        project: &str,
        frame: Option<&str>,
        created_at: i64,
    ) {
        let mut run = RunRecord::new(id, project, "local", id, "command");
        run.frame_id = frame.map(str::to_string);
        run.command = Some(format!("python {id}.py"));
        run.created_at = created_at;
        store.create_run(&run).await.unwrap();
    }

    async fn end(store: &Store, id: &str) {
        sqlx::query("UPDATE runs SET status='succeeded', ended_at=created_at WHERE id=?")
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    async fn snapshots(store: &Store, id: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM run_code_snapshots WHERE run_id=?")
            .bind(id)
            .fetch_one(&store.pool)
            .await
            .unwrap()
    }

    /// `branch` becomes the conversation of exploration `explore` in `p`.
    async fn exploration(store: &Store) {
        store
            .append_message("main", 1, &Message::user("compare approaches"))
            .await
            .unwrap();
        store
            .append_message("main", 2, &Message::assistant("stable checkpoint"))
            .await
            .unwrap();
        store
            .create_frame("branch", "p", "OPERON", "model")
            .await
            .unwrap();
        store
            .create_workspace_snapshot(&WorkspaceSnapshotRecord {
                id: "snapshot".into(),
                project_id: "p".into(),
                manifest_json: r#"{"version":1,"files":[]}"#.into(),
                manifest_sha256: "a".repeat(64),
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .create_context_archive(&ContextArchiveRecord {
                id: "archive".into(),
                project_id: "p".into(),
                frame_id: "main".into(),
                storage_path: ".wisp/history/archive.json".into(),
                checksum: "b".repeat(64),
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .create_exploration_family(&ExplorationFamily {
                id: "family".into(),
                project_id: "p".into(),
                root_frame_id: "main".into(),
                mainline_frame_id: "main".into(),
                generation: 0,
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
        store
            .create_exploration_checkpoint(&ExplorationCheckpoint {
                id: "checkpoint".into(),
                family_id: "family".into(),
                project_id: "p".into(),
                source_frame_id: "main".into(),
                source_message_seq: 2,
                source_frame_head_seq: 2,
                source_ui_event_seq: 0,
                source_ui_event_head_seq: 0,
                source_family_generation: 0,
                source_state_generation: store.project_state_generation("p").await.unwrap(),
                workspace_snapshot_id: "snapshot".into(),
                context_archive_id: "archive".into(),
                guard_hash: "c".repeat(64),
                entity_hash: "d".repeat(64),
                isolation_summary_json: r#"{"level":"full"}"#.into(),
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .create_exploration(&Exploration {
                id: "explore".into(),
                checkpoint_id: "checkpoint".into(),
                frame_id: "branch".into(),
                name: "Alternative".into(),
                status: ExplorationStatus::Creating,
                workspace_dir: "/tmp/explore".into(),
                workspace_backend: "copy".into(),
                scope_generation: 0,
                warnings_json: "[]".into(),
                created_at: 2,
                updated_at: 2,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_newest_runs_of_each_project_keep_their_code_snapshots() {
        let (store, _root) = store().await;
        store
            .create_project("q", "Other", "/tmp/other")
            .await
            .unwrap();
        for created_at in 1..=5 {
            ended_run(&store, &format!("p-{created_at}"), "p", created_at).await;
        }
        for created_at in 1..=2 {
            ended_run(&store, &format!("q-{created_at}"), "q", created_at).await;
        }
        assert_eq!(prune(&store.pool, 3, 1).await.unwrap(), 2);
        for (id, kept) in [
            ("p-1", 0),
            ("p-2", 0),
            ("p-3", 1),
            ("p-4", 1),
            ("p-5", 1),
            ("q-1", 1),
            ("q-2", 1),
        ] {
            assert_eq!(snapshots(&store, id).await, kept, "{id}");
        }
        // The Runs themselves stay.
        assert!(store.get_run("p-1").await.unwrap().is_some());
        // A second pass has nothing left to do.
        assert_eq!(prune(&store.pool, 3, 1).await.unwrap(), 0);
        assert_eq!(snapshots(&store, "p-3").await, 1);
        // The production limit keeps everything here; the maintenance entry
        // point is harmless on a store without a project registry.
        assert_eq!(store.prune_run_code_snapshots().await.unwrap(), 0);
        store.maintain_run_code_snapshots().await;
        assert_eq!(snapshots(&store, "p-3").await, 1);
        Store::close_pool(&store.pool).await;
    }

    #[tokio::test]
    async fn runs_still_needed_keep_their_code_snapshots() {
        let (store, _root) = store().await;
        exploration(&store).await;
        run_in_frame(&store, "running", "p", None, 1).await;
        sqlx::query("UPDATE runs SET status='running' WHERE id='running'")
            .execute(&store.pool)
            .await
            .unwrap();
        run_in_frame(&store, "explored", "p", Some("branch"), 2).await;
        end(&store, "explored").await;
        ended_run(&store, "cited", "p", 3).await;
        ended_run(&store, "reproduced", "p", 4).await;
        ended_run(&store, "plain", "p", 5).await;
        ended_run(&store, "newest", "p", 6).await;
        store
            .create_publication("publication", "p", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "publication", None, "v1")
            .await
            .unwrap();
        store
            .save_evidence_binding(&EvidenceBindingDraft {
                id: "binding".into(),
                revision_id: "revision".into(),
                item_id: None,
                source_kind: EvidenceSourceKind::Run,
                source_id: "cited".into(),
                purpose: "Methods".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Private,
            })
            .await
            .unwrap();
        // Evidence binds to a draft revision; reproduction needs a frozen one.
        sqlx::query(
            "UPDATE publication_revisions SET state='frozen', manifest_json='{}', \
             manifest_sha256=?, frozen_at=1 WHERE id='revision'",
        )
        .bind("e".repeat(64))
        .execute(&store.pool)
        .await
        .unwrap();
        let (environment_json, environment_hash) =
            crate::canonical_json_sha256(&serde_json::json!({}));
        store
            .start_reproduction_run(&ReproductionRunStart {
                id: "reproduction".into(),
                revision_id: "revision".into(),
                source_run_id: "reproduced".into(),
                command_sha256: "0".repeat(64),
                expected_environment_hash: None,
                actual_environment_json: environment_json,
                actual_environment_hash: environment_hash,
                environment_matched: false,
                workspace_manifest_json: "{}".into(),
            })
            .await
            .unwrap();

        // Only `plain` is old, ended and unreferenced.
        assert_eq!(prune(&store.pool, 1, 10).await.unwrap(), 1);
        for (id, kept) in [
            ("running", 1),
            ("explored", 1),
            ("cited", 1),
            ("reproduced", 1),
            ("plain", 0),
            ("newest", 1),
        ] {
            assert_eq!(snapshots(&store, id).await, kept, "{id}");
        }

        // Protection ends with the Run: once it has ended, its snapshot goes.
        end(&store, "running").await;
        assert_eq!(prune(&store.pool, 1, 10).await.unwrap(), 1);
        assert_eq!(snapshots(&store, "running").await, 0);

        // An unfinished promotion still protects the round's Runs after the
        // exploration row itself is gone; a finished one does not.
        store
            .create_exploration_promotion(&ExplorationPromotion {
                id: "promotion".into(),
                exploration_id: "explore".into(),
                expected_guard_hash: "c".repeat(64),
                status: ExplorationPromotionStatus::Prepared,
                diff_json: "{}".into(),
                journal_path: None,
                error: None,
                started_at: 3,
                committed_at: None,
            })
            .await
            .unwrap();
        sqlx::query("DELETE FROM explorations WHERE id='explore'")
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(prune(&store.pool, 1, 10).await.unwrap(), 0);
        assert_eq!(snapshots(&store, "explored").await, 1);
        sqlx::query("UPDATE exploration_promotions SET status='rolled_back' WHERE id='promotion'")
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(prune(&store.pool, 1, 10).await.unwrap(), 1);
        assert_eq!(snapshots(&store, "explored").await, 0);
        Store::close_pool(&store.pool).await;
    }

    #[tokio::test]
    async fn a_large_backlog_is_pruned_in_batches_until_nothing_is_left() {
        let (store, _root) = store().await;
        for created_at in 1..=7 {
            ended_run(&store, &format!("run-{created_at}"), "p", created_at).await;
        }
        // Five candidates, two Runs per statement: three statements plus the
        // empty one that ends the loop.
        assert_eq!(prune(&store.pool, 2, 2).await.unwrap(), 5);
        for created_at in 1..=7 {
            let id = format!("run-{created_at}");
            assert_eq!(
                snapshots(&store, &id).await,
                i64::from(created_at > 5),
                "{id}"
            );
        }
        assert_eq!(prune(&store.pool, 2, 2).await.unwrap(), 0);
        Store::close_pool(&store.pool).await;
    }
}
