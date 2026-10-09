//! Research mandates: long-running responsibilities carried round by round.
//!
//! A mandate owns its own `next_run_at` instead of a `schedules` row: each
//! round decides when the next one happens, so there is no fixed cadence to
//! anchor. Rounds fire only in-process, like schedules, and a round is
//! claimed atomically so a slow turn never runs twice.

use super::Store;
use anyhow::{ensure, Result};
use sqlx::Row;

pub use wisp_dto::{MandateRecord, MandateRound, MANDATE_STATUSES};

const MANDATE_COLUMNS: &str = "id,project_id,frame_id,name,goal,kpis,constraints,ends_at,\
    interval_secs,report_interval_secs,next_report_at,status,next_run_at,last_run_at,\
    wait_run_id,created_at,updated_at";

fn mandate_from_row(row: sqlx::sqlite::SqliteRow) -> Result<MandateRecord> {
    Ok(MandateRecord {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        frame_id: row.try_get("frame_id")?,
        name: row.try_get("name")?,
        goal: row.try_get("goal")?,
        // A hand-edited or future-version row must not hide the mandate.
        kpis: serde_json::from_str(&row.try_get::<String, _>("kpis")?).unwrap_or_default(),
        constraints: serde_json::from_str(&row.try_get::<String, _>("constraints")?)
            .unwrap_or_default(),
        ends_at: row.try_get("ends_at")?,
        interval_secs: row.try_get("interval_secs")?,
        report_interval_secs: row.try_get("report_interval_secs")?,
        next_report_at: row.try_get("next_report_at")?,
        status: row.try_get("status")?,
        next_run_at: row.try_get("next_run_at")?,
        last_run_at: row.try_get("last_run_at")?,
        wait_run_id: row.try_get("wait_run_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    pub async fn create_mandate(&self, mandate: &MandateRecord) -> Result<()> {
        if let Some(store) = self.route_project(&mandate.project_id).await? {
            return Box::pin(store.create_mandate(mandate)).await;
        }
        ensure!(
            MANDATE_STATUSES.contains(&mandate.status.as_str()),
            "Unknown mandate status"
        );
        sqlx::query(&format!(
            "INSERT INTO mandates({MANDATE_COLUMNS}) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
        ))
        .bind(&mandate.id)
        .bind(&mandate.project_id)
        .bind(mandate.frame_id.as_deref())
        .bind(mandate.name.trim())
        .bind(mandate.goal.trim())
        .bind(serde_json::to_string(&mandate.kpis)?)
        .bind(serde_json::to_string(&mandate.constraints)?)
        .bind(mandate.ends_at)
        .bind(mandate.interval_secs)
        .bind(mandate.report_interval_secs)
        .bind(mandate.next_report_at)
        .bind(&mandate.status)
        .bind(mandate.next_run_at)
        .bind(mandate.last_run_at)
        .bind(mandate.wait_run_id.as_deref())
        .bind(mandate.created_at)
        .bind(mandate.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_mandate(&self, id: &str) -> Result<Option<MandateRecord>> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.get_mandate(id)).await;
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates WHERE id=?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(mandate_from_row)
        .transpose()
    }

    /// The mandate whose conversation this is. The caller names the frame's
    /// project so an ordinary turn costs one indexed lookup, not a scan of
    /// every project database.
    pub async fn mandate_for_frame(
        &self,
        project_id: &str,
        frame_id: &str,
    ) -> Result<Option<MandateRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.mandate_for_frame(project_id, frame_id)).await;
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates WHERE frame_id=? AND project_id=? \
             ORDER BY created_at DESC,id LIMIT 1"
        ))
        .bind(frame_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .map(mandate_from_row)
        .transpose()
    }

    pub async fn list_mandates(&self, project_id: &str) -> Result<Vec<MandateRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_mandates(project_id)).await;
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates WHERE project_id=? ORDER BY created_at,id"
        ))
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(mandate_from_row)
        .collect()
    }

    /// Save what the researcher (or a round's KPI report) defines: name,
    /// goal, KPIs, constraints, period and cadences. Status, the conversation
    /// and round timing have their own writers below, so an edit can never
    /// undo a round claimed or a status changed while the form was open.
    /// Returns false when the mandate is gone.
    pub async fn update_mandate(&self, mandate: &MandateRecord) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", &mandate.id).await? {
            return Box::pin(store.update_mandate(mandate)).await;
        }
        let result = sqlx::query(
            "UPDATE mandates SET name=?,goal=?,kpis=?,constraints=?,ends_at=?,\
             interval_secs=?,report_interval_secs=?,updated_at=? WHERE id=?",
        )
        .bind(mandate.name.trim())
        .bind(mandate.goal.trim())
        .bind(serde_json::to_string(&mandate.kpis)?)
        .bind(serde_json::to_string(&mandate.constraints)?)
        .bind(mandate.ends_at)
        .bind(mandate.interval_secs)
        .bind(mandate.report_interval_secs)
        .bind(mandate.updated_at)
        .bind(&mandate.id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn set_mandate_status(&self, id: &str, status: &str, now: i64) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.set_mandate_status(id, status, now)).await;
        }
        ensure!(MANDATE_STATUSES.contains(&status), "Unknown mandate status");
        let result = sqlx::query("UPDATE mandates SET status=?, updated_at=? WHERE id=?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Bind the mandate to its (new) conversation.
    pub async fn set_mandate_frame(&self, id: &str, frame_id: Option<&str>) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.set_mandate_frame(id, frame_id)).await;
        }
        let result = sqlx::query("UPDATE mandates SET frame_id=? WHERE id=?")
            .bind(frame_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Set when the next round happens, and the Run (if any) whose completion
    /// starts it early.
    pub async fn schedule_mandate_round(
        &self,
        id: &str,
        next_run_at: i64,
        wait_run_id: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.schedule_mandate_round(id, next_run_at, wait_run_id, now)).await;
        }
        let result = sqlx::query(
            "UPDATE mandates SET next_run_at=?, wait_run_id=?, updated_at=? WHERE id=?",
        )
        .bind(next_run_at)
        .bind(wait_run_id)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_mandate(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.delete_mandate(id)).await;
        }
        sqlx::query("DELETE FROM mandate_rounds WHERE mandate_id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM mandates WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Active mandates whose round is due, oldest slot first.
    pub async fn due_mandates(&self, now: i64) -> Result<Vec<MandateRecord>> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                result.extend(Box::pin(store.due_mandates(now)).await?);
            }
            result.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at).then(a.id.cmp(&b.id)));
            return Ok(result);
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates \
             WHERE status='active' AND next_run_at<=? \
             AND EXISTS (SELECT 1 FROM projects WHERE id=mandates.project_id) \
             ORDER BY next_run_at,id"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(mandate_from_row)
        .collect()
    }

    /// Atomically move `next_run_at` on and stamp `last_run_at`. False when
    /// another tick claimed this round, or the mandate was paused, edited or
    /// closed in between — so exactly one round runs per slot.
    pub async fn claim_mandate_round(
        &self,
        id: &str,
        expected_next_run_at: i64,
        new_next_run_at: i64,
        fired_at: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.claim_mandate_round(
                id,
                expected_next_run_at,
                new_next_run_at,
                fired_at,
            ))
            .await;
        }
        let result = sqlx::query(
            "UPDATE mandates SET next_run_at=?, last_run_at=?, updated_at=? \
             WHERE id=? AND next_run_at=? AND status='active'",
        )
        .bind(new_next_run_at)
        .bind(fired_at)
        .bind(fired_at)
        .bind(id)
        .bind(expected_next_run_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}

fn round_from_row(row: sqlx::sqlite::SqliteRow) -> Result<MandateRound> {
    Ok(MandateRound {
        id: row.try_get("id")?,
        mandate_id: row.try_get("mandate_id")?,
        seq: row.try_get("seq")?,
        done: row.try_get("done")?,
        kpis: serde_json::from_str(&row.try_get::<String, _>("kpis")?).unwrap_or_default(),
        blockers: row.try_get("blockers")?,
        next_step: row.try_get("next_step")?,
        next_run_at: row.try_get("next_run_at")?,
        source: row.try_get("source")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// Append a round to the mandate's ledger and return it with its `seq`,
    /// the next number in the mandate's gapless sequence. `round.seq` is ignored.
    pub async fn add_mandate_round(&self, round: &MandateRound) -> Result<MandateRound> {
        if let Some(store) = self
            .route_entity("mandates", "id", &round.mandate_id)
            .await?
        {
            return Box::pin(store.add_mandate_round(round)).await;
        }
        // The write lock is taken at BEGIN, so two rounds cannot read the
        // same MAX(seq).
        let mut tx = self.begin_write().await?;
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0)+1 FROM mandate_rounds WHERE mandate_id=?",
        )
        .bind(&round.mandate_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO mandate_rounds(\
             id,mandate_id,seq,done,kpis,blockers,next_step,next_run_at,source,created_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&round.id)
        .bind(&round.mandate_id)
        .bind(seq)
        .bind(round.done.trim())
        .bind(serde_json::to_string(&round.kpis)?)
        .bind(round.blockers.trim())
        .bind(round.next_step.trim())
        .bind(round.next_run_at)
        .bind(&round.source)
        .bind(round.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MandateRound {
            seq,
            done: round.done.trim().into(),
            blockers: round.blockers.trim().into(),
            next_step: round.next_step.trim().into(),
            ..round.clone()
        })
    }

    /// The mandate's latest rounds, newest first.
    pub async fn mandate_rounds(
        &self,
        mandate_id: &str,
        limit: usize,
    ) -> Result<Vec<MandateRound>> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.mandate_rounds(mandate_id, limit)).await;
        }
        sqlx::query(
            "SELECT id,mandate_id,seq,done,kpis,blockers,next_step,next_run_at,source,created_at \
             FROM mandate_rounds WHERE mandate_id=? ORDER BY seq DESC LIMIT ?",
        )
        .bind(mandate_id)
        .bind(limit.clamp(1, 500) as i64)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(round_from_row)
        .collect()
    }

    /// Rounds recorded in `[from, until)`, oldest first.
    pub async fn mandate_rounds_between(
        &self,
        mandate_id: &str,
        from: i64,
        until: i64,
    ) -> Result<Vec<MandateRound>> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.mandate_rounds_between(mandate_id, from, until)).await;
        }
        sqlx::query(
            "SELECT id,mandate_id,seq,done,kpis,blockers,next_step,next_run_at,source,created_at \
             FROM mandate_rounds WHERE mandate_id=? AND created_at>=? AND created_at<? \
             ORDER BY seq LIMIT 500",
        )
        .bind(mandate_id)
        .bind(from)
        .bind(until)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(round_from_row)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use wisp_dto::{MandateConstraints, MandateKpi, MandateKpiValue};

    pub(crate) fn test_mandate(id: &str, project_id: &str, next_run_at: i64) -> MandateRecord {
        MandateRecord {
            id: id.into(),
            project_id: project_id.into(),
            frame_id: Some(format!("frame-{id}")),
            name: "Literature watch".into(),
            goal: "Track new single-cell papers for a year.".into(),
            kpis: vec![MandateKpi {
                name: "Papers filed".into(),
                definition: "Screened and added to the library".into(),
                target: "5".into(),
                period: "per week".into(),
                current: None,
            }],
            constraints: MandateConstraints {
                autonomous: "Search and summarize".into(),
                ..Default::default()
            },
            ends_at: Some(9_999_999),
            interval_secs: 86_400,
            report_interval_secs: 7 * 86_400,
            next_report_at: None,
            status: "active".into(),
            next_run_at,
            last_run_at: None,
            wait_run_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn test_round(mandate_id: &str, created_at: i64) -> MandateRound {
        MandateRound {
            id: Uuid::new_v4().to_string(),
            mandate_id: mandate_id.into(),
            seq: 99,
            done: "  Screened 12 abstracts  ".into(),
            kpis: vec![MandateKpiValue {
                name: "Papers filed".into(),
                value: "3".into(),
            }],
            blockers: String::new(),
            next_step: "Read the two flagged reviews".into(),
            next_run_at: Some(created_at + 3_600),
            source: "agent".into(),
            created_at,
        }
    }

    #[tokio::test]
    async fn the_ledger_numbers_rounds_without_gaps_per_mandate() {
        let (store, root) = test_store().await;
        store
            .create_mandate(&test_mandate("m1", "p1", 100))
            .await
            .unwrap();
        store
            .create_mandate(&test_mandate("m2", "p1", 100))
            .await
            .unwrap();
        let first = store
            .add_mandate_round(&test_round("m1", 1_000))
            .await
            .unwrap();
        assert_eq!(first.seq, 1, "the caller's seq is ignored");
        assert_eq!(first.done, "Screened 12 abstracts");
        let mut host = test_round("m1", 2_000);
        host.source = "host".into();
        assert_eq!(store.add_mandate_round(&host).await.unwrap().seq, 2);
        assert_eq!(
            store
                .add_mandate_round(&test_round("m2", 1_500))
                .await
                .unwrap()
                .seq,
            1,
            "each mandate has its own sequence"
        );
        let mut bogus = test_round("m1", 3_000);
        bogus.source = "model".into();
        assert!(store.add_mandate_round(&bogus).await.is_err());

        let rounds = store.mandate_rounds("m1", 10).await.unwrap();
        assert_eq!(rounds.iter().map(|r| r.seq).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(rounds[1], first);
        assert_eq!(rounds[0].source, "host");
        assert_eq!(store.mandate_rounds("m1", 1).await.unwrap().len(), 1);
        let window = store
            .mandate_rounds_between("m1", 1_000, 2_000)
            .await
            .unwrap();
        assert_eq!(window, vec![first], "the window is half-open");

        store.delete_mandate("m1").await.unwrap();
        assert!(store.mandate_rounds("m1", 10).await.unwrap().is_empty());
        assert_eq!(store.mandate_rounds("m2", 10).await.unwrap().len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    async fn test_store() -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("wisp-mandates-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store.create_project("p1", "proj", "").await.unwrap();
        (store, root)
    }

    #[tokio::test]
    async fn mandate_roundtrips_and_survives_reopen() {
        let (store, root) = test_store().await;
        let mut mandate = test_mandate("m1", "p1", 500);
        store.create_mandate(&mandate).await.unwrap();
        assert_eq!(
            store.get_mandate("m1").await.unwrap(),
            Some(mandate.clone())
        );
        assert_eq!(
            store.list_mandates("p1").await.unwrap(),
            vec![mandate.clone()]
        );
        assert!(store.list_mandates("p2").await.unwrap().is_empty());
        assert_eq!(
            store.mandate_for_frame("p1", "frame-m1").await.unwrap(),
            Some(mandate.clone())
        );
        assert_eq!(store.mandate_for_frame("p1", "other").await.unwrap(), None);

        mandate.kpis[0].current = Some("3".into());
        mandate
            .constraints
            .notes
            .push("Lead with methods papers".into());
        mandate.updated_at = 20;
        assert!(store.update_mandate(&mandate).await.unwrap());
        // Status, conversation and timing never ride along with an edit.
        let mut stale = mandate.clone();
        stale.status = "done".into();
        stale.frame_id = None;
        stale.next_run_at = 1;
        assert!(store.update_mandate(&stale).await.unwrap());
        assert_eq!(
            store.get_mandate("m1").await.unwrap(),
            Some(mandate.clone())
        );

        assert!(store.set_mandate_status("m1", "waiting", 30).await.unwrap());
        assert!(store.set_mandate_status("m1", "bogus", 30).await.is_err());
        assert!(store.set_mandate_frame("m1", None).await.unwrap());
        assert!(store
            .schedule_mandate_round("m1", 900, Some("run-1"), 30)
            .await
            .unwrap());
        mandate.status = "waiting".into();
        mandate.frame_id = None;
        mandate.next_run_at = 900;
        mandate.wait_run_id = Some("run-1".into());
        mandate.updated_at = 30;

        store.pool.close().await;
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        assert_eq!(
            store.get_mandate("m1").await.unwrap(),
            Some(mandate.clone())
        );

        store.delete_mandate("m1").await.unwrap();
        assert_eq!(store.get_mandate("m1").await.unwrap(), None);
        assert!(!store.update_mandate(&mandate).await.unwrap());
        assert!(!store.set_mandate_status("m1", "active", 40).await.unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn only_active_due_mandates_are_claimed_once() {
        let (store, root) = test_store().await;
        store
            .create_mandate(&test_mandate("due", "p1", 100))
            .await
            .unwrap();
        store
            .create_mandate(&test_mandate("later", "p1", 10_000))
            .await
            .unwrap();
        let mut paused = test_mandate("paused", "p1", 100);
        paused.status = "paused".into();
        store.create_mandate(&paused).await.unwrap();
        let mut waiting = test_mandate("waiting", "p1", 100);
        waiting.status = "waiting".into();
        store.create_mandate(&waiting).await.unwrap();

        let due = store.due_mandates(500).await.unwrap();
        assert_eq!(
            due.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["due"]
        );
        assert!(store
            .claim_mandate_round("due", 100, 86_500, 500)
            .await
            .unwrap());
        assert!(
            !store
                .claim_mandate_round("due", 100, 86_500, 500)
                .await
                .unwrap(),
            "a second tick loses the race"
        );
        assert!(
            !store
                .claim_mandate_round("paused", 100, 86_500, 500)
                .await
                .unwrap(),
            "a paused mandate never fires"
        );
        let claimed = store.get_mandate("due").await.unwrap().unwrap();
        assert_eq!(
            (claimed.next_run_at, claimed.last_run_at),
            (86_500, Some(500))
        );
        assert!(store.due_mandates(9_999).await.unwrap().is_empty());

        // Deleting the project takes its mandates and their ledgers with it.
        store
            .add_mandate_round(&test_round("due", 600))
            .await
            .unwrap();
        store.delete_project("p1").await.unwrap();
        assert_eq!(store.get_mandate("due").await.unwrap(), None);
        assert!(store.mandate_rounds("due", 10).await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
