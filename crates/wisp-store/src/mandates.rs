//! Research mandates: long-running responsibilities carried round by round.
//!
//! A mandate owns its own `next_run_at` instead of a `schedules` row: each
//! round decides when the next one happens, so there is no fixed cadence to
//! anchor. Rounds fire only in-process, like schedules, and a round is
//! claimed atomically so a slow turn never runs twice.

use super::Store;
use anyhow::{ensure, Result};
use sqlx::Row;

pub use wisp_dto::{
    MandateRecord, MandateReport, MandateRequest, MandateRound, MANDATE_REQUEST_KINDS,
    MANDATE_STATUSES,
};

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
        for table in ["mandate_rounds", "mandate_requests", "mandate_reports"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE mandate_id=?"))
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
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

const REQUEST_COLUMNS: &str =
    "id,mandate_id,kind,what,why,then_what,status,reply,created_at,answered_at";

fn request_from_row(row: sqlx::sqlite::SqliteRow) -> Result<MandateRequest> {
    Ok(MandateRequest {
        id: row.try_get("id")?,
        mandate_id: row.try_get("mandate_id")?,
        kind: row.try_get("kind")?,
        what: row.try_get("what")?,
        why: row.try_get("why")?,
        then_what: row.try_get("then_what")?,
        status: row.try_get("status")?,
        reply: row.try_get("reply")?,
        created_at: row.try_get("created_at")?,
        answered_at: row.try_get("answered_at")?,
    })
}

impl Store {
    /// Open a request for the researcher. An earlier request still open is
    /// withdrawn: the mandate waits on one thing at a time.
    pub async fn open_mandate_request(&self, request: &MandateRequest) -> Result<()> {
        if let Some(store) = self
            .route_entity("mandates", "id", &request.mandate_id)
            .await?
        {
            return Box::pin(store.open_mandate_request(request)).await;
        }
        ensure!(
            MANDATE_REQUEST_KINDS.contains(&request.kind.as_str()),
            "Unknown request kind"
        );
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE mandate_requests SET status='withdrawn' WHERE mandate_id=? AND status='open'",
        )
        .bind(&request.mandate_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO mandate_requests({REQUEST_COLUMNS}) VALUES(?,?,?,?,?,?,'open',NULL,?,NULL)"
        ))
        .bind(&request.id)
        .bind(&request.mandate_id)
        .bind(&request.kind)
        .bind(request.what.trim())
        .bind(request.why.trim())
        .bind(request.then_what.trim())
        .bind(request.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Withdraw the mandate's unanswered request, if it has one: a closed
    /// mandate asks for nothing.
    pub async fn withdraw_mandate_requests(&self, mandate_id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.withdraw_mandate_requests(mandate_id)).await;
        }
        sqlx::query(
            "UPDATE mandate_requests SET status='withdrawn' WHERE mandate_id=? AND status='open'",
        )
        .bind(mandate_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The mandate's newest request, whatever its status.
    pub async fn latest_mandate_request(&self, mandate_id: &str) -> Result<Option<MandateRequest>> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.latest_mandate_request(mandate_id)).await;
        }
        sqlx::query(&format!(
            "SELECT {REQUEST_COLUMNS} FROM mandate_requests WHERE mandate_id=? \
             ORDER BY created_at DESC, rowid DESC LIMIT 1"
        ))
        .bind(mandate_id)
        .fetch_optional(&self.pool)
        .await?
        .map(request_from_row)
        .transpose()
    }

    /// Answer the mandate's open request. Returns it as answered, or `None`
    /// when nothing was open — a second reply never overwrites the first.
    pub async fn answer_mandate_request(
        &self,
        mandate_id: &str,
        reply: &str,
        now: i64,
    ) -> Result<Option<MandateRequest>> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.answer_mandate_request(mandate_id, reply, now)).await;
        }
        sqlx::query(&format!(
            "UPDATE mandate_requests SET status='answered', reply=?, answered_at=? \
             WHERE mandate_id=? AND status='open' RETURNING {REQUEST_COLUMNS}"
        ))
        .bind(reply.trim())
        .bind(now)
        .bind(mandate_id)
        .fetch_optional(&self.pool)
        .await?
        .map(request_from_row)
        .transpose()
    }

    pub async fn save_mandate_report(&self, report: &MandateReport) -> Result<()> {
        if let Some(store) = self
            .route_entity("mandates", "id", &report.mandate_id)
            .await?
        {
            return Box::pin(store.save_mandate_report(report)).await;
        }
        sqlx::query(
            "INSERT INTO mandate_reports(id,mandate_id,period_from,period_until,report_json,created_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&report.id)
        .bind(&report.mandate_id)
        .bind(report.period_from)
        .bind(report.period_until)
        .bind(serde_json::to_string(report)?)
        .bind(report.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The mandate's latest reports, newest first.
    pub async fn mandate_reports(
        &self,
        mandate_id: &str,
        limit: usize,
    ) -> Result<Vec<MandateReport>> {
        if let Some(store) = self.route_entity("mandates", "id", mandate_id).await? {
            return Box::pin(store.mandate_reports(mandate_id, limit)).await;
        }
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT report_json FROM mandate_reports WHERE mandate_id=? \
             ORDER BY period_until DESC, created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(mandate_id)
        .bind(limit.clamp(1, 200) as i64)
        .fetch_all(&self.pool)
        .await?;
        // A report from a future version is skipped, not fatal.
        Ok(rows
            .iter()
            .filter_map(|raw| serde_json::from_str(raw).ok())
            .collect())
    }

    /// Mandates still being carried (active or waiting) whose report is due.
    /// A paused or closed mandate reports nothing.
    pub async fn mandates_due_for_report(&self, now: i64) -> Result<Vec<MandateRecord>> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                result.extend(Box::pin(store.mandates_due_for_report(now)).await?);
            }
            return Ok(result);
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates \
             WHERE status IN ('active','waiting') AND next_report_at IS NOT NULL \
             AND next_report_at<=? \
             AND EXISTS (SELECT 1 FROM projects WHERE id=mandates.project_id) \
             ORDER BY next_report_at,id"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(mandate_from_row)
        .collect()
    }

    /// Atomically move the report slot on. False when another tick took it.
    pub async fn claim_mandate_report(
        &self,
        id: &str,
        expected_next_report_at: i64,
        new_next_report_at: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("mandates", "id", id).await? {
            return Box::pin(store.claim_mandate_report(
                id,
                expected_next_report_at,
                new_next_report_at,
            ))
            .await;
        }
        let result =
            sqlx::query("UPDATE mandates SET next_report_at=? WHERE id=? AND next_report_at=?")
                .bind(new_next_report_at)
                .bind(id)
                .bind(expected_next_report_at)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Active mandates whose next round is waiting on a Run to finish.
    pub async fn mandates_waiting_on_runs(&self) -> Result<Vec<MandateRecord>> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                result.extend(Box::pin(store.mandates_waiting_on_runs()).await?);
            }
            return Ok(result);
        }
        sqlx::query(&format!(
            "SELECT {MANDATE_COLUMNS} FROM mandates \
             WHERE status='active' AND wait_run_id IS NOT NULL ORDER BY id"
        ))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(mandate_from_row)
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

    fn test_request(id: &str, mandate_id: &str, created_at: i64) -> MandateRequest {
        MandateRequest {
            id: id.into(),
            mandate_id: mandate_id.into(),
            kind: "login".into(),
            what: " Sign in to the journal's submission system ".into(),
            why: "The status page needs an account".into(),
            then_what: "I will read the decision letter".into(),
            created_at,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn one_request_is_open_at_a_time_and_is_answered_once() {
        let (store, root) = test_store().await;
        store
            .create_mandate(&test_mandate("m1", "p1", 100))
            .await
            .unwrap();
        assert_eq!(store.latest_mandate_request("m1").await.unwrap(), None);
        assert_eq!(
            store.answer_mandate_request("m1", "done", 5).await.unwrap(),
            None,
            "nothing is open yet"
        );
        store
            .open_mandate_request(&test_request("r1", "m1", 10))
            .await
            .unwrap();
        let mut bogus = test_request("r0", "m1", 11);
        bogus.kind = "coffee".into();
        assert!(store.open_mandate_request(&bogus).await.is_err());
        store
            .open_mandate_request(&test_request("r2", "m1", 20))
            .await
            .unwrap();
        let open = store.latest_mandate_request("m1").await.unwrap().unwrap();
        assert_eq!((open.id.as_str(), open.status.as_str()), ("r2", "open"));
        assert_eq!(open.what, "Sign in to the journal's submission system");

        let answered = store
            .answer_mandate_request("m1", "  Signed in.  ", 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(answered.id, "r2", "the withdrawn r1 is not answered");
        assert_eq!(answered.reply.as_deref(), Some("Signed in."));
        assert_eq!(answered.answered_at, Some(30));
        assert_eq!(
            store
                .answer_mandate_request("m1", "again", 40)
                .await
                .unwrap(),
            None,
            "a second reply never overwrites the first"
        );
        assert_eq!(
            store.latest_mandate_request("m1").await.unwrap(),
            Some(answered)
        );

        // Closing a mandate withdraws what it still asked for, and only that.
        store
            .open_mandate_request(&test_request("r3", "m1", 50))
            .await
            .unwrap();
        store.withdraw_mandate_requests("m1").await.unwrap();
        let withdrawn = store.latest_mandate_request("m1").await.unwrap().unwrap();
        assert_eq!(
            (withdrawn.id.as_str(), withdrawn.status.as_str()),
            ("r3", "withdrawn")
        );
        assert_eq!(
            store
                .answer_mandate_request("m1", "too late", 60)
                .await
                .unwrap(),
            None
        );

        store.delete_mandate("m1").await.unwrap();
        assert_eq!(store.latest_mandate_request("m1").await.unwrap(), None);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reports_are_kept_newest_first_and_claimed_once_per_slot() {
        let (store, root) = test_store().await;
        for (id, status) in [("m1", "active"), ("held", "waiting"), ("off", "paused")] {
            let mut mandate = test_mandate(id, "p1", 9_000);
            mandate.status = status.into();
            mandate.next_report_at = Some(500);
            store.create_mandate(&mandate).await.unwrap();
        }
        let due = store.mandates_due_for_report(600).await.unwrap();
        assert_eq!(
            due.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["held", "m1"],
            "a waiting mandate still reports; a paused one does not"
        );
        assert!(store.mandates_due_for_report(499).await.unwrap().is_empty());
        assert!(store.claim_mandate_report("m1", 500, 1_100).await.unwrap());
        assert!(!store.claim_mandate_report("m1", 500, 1_100).await.unwrap());
        assert_eq!(
            store
                .get_mandate("m1")
                .await
                .unwrap()
                .unwrap()
                .next_report_at,
            Some(1_100)
        );

        let report = |id: &str, until: i64| MandateReport {
            id: id.into(),
            mandate_id: "m1".into(),
            period_from: until - 100,
            period_until: until,
            rounds: 2,
            kpis: test_mandate("m1", "p1", 0).kpis,
            body: wisp_dto::ResearchRecap {
                headline: format!("Report {id}"),
                done: vec![wisp_dto::ResearchRecapItem {
                    text: "Filed 3 papers".into(),
                    refs: vec![0],
                }],
                sources: vec![wisp_dto::ResearchRecapSource {
                    kind: "round".into(),
                    id: "r1".into(),
                    title: "Round 1".into(),
                }],
                ..Default::default()
            },
            created_at: until,
        };
        store
            .save_mandate_report(&report("a", 1_000))
            .await
            .unwrap();
        store
            .save_mandate_report(&report("b", 2_000))
            .await
            .unwrap();
        let reports = store.mandate_reports("m1", 10).await.unwrap();
        assert_eq!(reports, vec![report("b", 2_000), report("a", 1_000)]);
        assert_eq!(store.mandate_reports("m1", 1).await.unwrap().len(), 1);
        store.delete_mandate("m1").await.unwrap();
        assert!(store.mandate_reports("m1", 10).await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn only_active_mandates_with_a_run_are_waiting_on_runs() {
        let (store, root) = test_store().await;
        for (id, status, run) in [
            ("plain", "active", None),
            ("run", "active", Some("run-1")),
            ("paused", "paused", Some("run-2")),
        ] {
            let mut mandate = test_mandate(id, "p1", 9_000);
            mandate.status = status.into();
            mandate.wait_run_id = run.map(str::to_string);
            store.create_mandate(&mandate).await.unwrap();
        }
        let waiting = store.mandates_waiting_on_runs().await.unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].id, "run");
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
