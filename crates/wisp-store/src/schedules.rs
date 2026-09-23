//! Scheduled automations: interval-based triggers that fire a prompt (with an
//! optional skill) into a chat session while the app is running.
//!
//! Schedules only fire in-process; there is no system-level daemon. Missed
//! slots collapse into one catch-up fire on the next launch, and slot math is
//! anchored at `next_run_at` so a daily schedule keeps a stable wall-clock
//! time instead of drifting by the poll interval.

use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRecord {
    pub id: String,
    pub project_id: String,
    /// Target session. `None` creates a fresh session for every fire.
    pub frame_id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub skill: Option<String>,
    pub interval_secs: i64,
    pub enabled: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRunRecord {
    pub id: String,
    pub schedule_id: String,
    pub frame_id: Option<String>,
    /// `fired` or `failed`.
    pub status: String,
    pub error: Option<String>,
    pub fired_at: i64,
}

/// Smallest `anchor + k*interval` strictly greater than `now`. Advancing from
/// the previous slot (not `now`) keeps the schedule on its original cadence.
pub fn next_slot_after(anchor: i64, interval_secs: i64, now: i64) -> i64 {
    let interval = interval_secs.max(1);
    if anchor > now {
        return anchor;
    }
    anchor + ((now - anchor) / interval + 1) * interval
}

fn schedule_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ScheduleRecord> {
    Ok(ScheduleRecord {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        frame_id: row.try_get("frame_id")?,
        name: row.try_get("name")?,
        prompt: row.try_get("prompt")?,
        skill: row.try_get("skill")?,
        interval_secs: row.try_get("interval_secs")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        next_run_at: row.try_get("next_run_at")?,
        last_run_at: row.try_get("last_run_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const SCHEDULE_COLUMNS: &str =
    "id,project_id,frame_id,name,prompt,skill,interval_secs,enabled,next_run_at,last_run_at,created_at,updated_at";

impl Store {
    pub async fn create_schedule(&self, schedule: &ScheduleRecord) -> Result<()> {
        if let Some(store) = self.route_project(&schedule.project_id).await? {
            return Box::pin(store.create_schedule(schedule)).await;
        }
        sqlx::query(
            "INSERT INTO schedules(\
             id,project_id,frame_id,name,prompt,skill,interval_secs,enabled,next_run_at,last_run_at,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&schedule.id)
        .bind(&schedule.project_id)
        .bind(schedule.frame_id.as_deref())
        .bind(&schedule.name)
        .bind(&schedule.prompt)
        .bind(schedule.skill.as_deref())
        .bind(schedule.interval_secs)
        .bind(schedule.enabled as i64)
        .bind(schedule.next_run_at)
        .bind(schedule.last_run_at)
        .bind(schedule.created_at)
        .bind(schedule.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_schedule(&self, id: &str) -> Result<Option<ScheduleRecord>> {
        if let Some(store) = self.route_entity("schedules", "id", id).await? {
            return Box::pin(store.get_schedule(id)).await;
        }
        let row = sqlx::query(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE id=?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(schedule_from_row).transpose()
    }

    pub async fn list_schedules(&self, project_id: &str) -> Result<Vec<ScheduleRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_schedules(project_id)).await;
        }
        let rows = sqlx::query(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE project_id=? ORDER BY created_at,id"
        ))
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(schedule_from_row).collect()
    }

    pub async fn set_schedule_enabled(&self, id: &str, enabled: bool, now: i64) -> Result<bool> {
        if let Some(store) = self.route_entity("schedules", "id", id).await? {
            return Box::pin(store.set_schedule_enabled(id, enabled, now)).await;
        }
        let result = sqlx::query("UPDATE schedules SET enabled=?, updated_at=? WHERE id=?")
            .bind(enabled as i64)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("schedules", "id", id).await? {
            return Box::pin(store.delete_schedule(id)).await;
        }
        sqlx::query("DELETE FROM schedule_runs WHERE schedule_id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM schedules WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Enabled schedules whose slot has passed. The poller claims each row
    /// before firing so a slow turn can never double-fire a schedule.
    pub async fn due_schedules(&self, now: i64) -> Result<Vec<ScheduleRecord>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value = Box::pin(store.due_schedules(now)).await?;
                result.extend(value);
            }
            result.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at).then(a.id.cmp(&b.id)));
            return Ok(result);
        }
        let rows = sqlx::query(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules \
             WHERE enabled=1 AND next_run_at<=? \
             AND EXISTS (SELECT 1 FROM projects WHERE id=schedules.project_id) \
             ORDER BY next_run_at,id"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(schedule_from_row).collect()
    }

    /// Atomically advance `next_run_at` past `now` and stamp `last_run_at`.
    /// Returns false when another tick already claimed this slot or the
    /// schedule was disabled in between, so exactly one fire happens per slot.
    pub async fn claim_schedule_fire(
        &self,
        id: &str,
        expected_next_run_at: i64,
        new_next_run_at: i64,
        fired_at: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("schedules", "id", id).await? {
            return Box::pin(store.claim_schedule_fire(
                id,
                expected_next_run_at,
                new_next_run_at,
                fired_at,
            ))
            .await;
        }
        let result = sqlx::query(
            "UPDATE schedules SET next_run_at=?, last_run_at=?, updated_at=? \
             WHERE id=? AND next_run_at=? AND enabled=1",
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

    pub async fn record_schedule_run(&self, run: &ScheduleRunRecord) -> Result<()> {
        if let Some(store) = self
            .route_entity("schedules", "id", &run.schedule_id)
            .await?
        {
            return Box::pin(store.record_schedule_run(run)).await;
        }
        sqlx::query(
            "INSERT INTO schedule_runs(id,schedule_id,frame_id,status,error,fired_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&run.id)
        .bind(&run.schedule_id)
        .bind(run.frame_id.as_deref())
        .bind(&run.status)
        .bind(run.error.as_deref())
        .bind(run.fired_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_schedule_runs(
        &self,
        schedule_id: &str,
        limit: usize,
    ) -> Result<Vec<ScheduleRunRecord>> {
        if let Some(store) = self.route_entity("schedules", "id", schedule_id).await? {
            return Box::pin(store.list_schedule_runs(schedule_id, limit)).await;
        }
        let rows = sqlx::query(
            "SELECT id,schedule_id,frame_id,status,error,fired_at FROM schedule_runs \
             WHERE schedule_id=? ORDER BY fired_at DESC,id DESC LIMIT ?",
        )
        .bind(schedule_id)
        .bind(limit.clamp(1, 200) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ScheduleRunRecord {
                    id: row.try_get("id")?,
                    schedule_id: row.try_get("schedule_id")?,
                    frame_id: row.try_get("frame_id")?,
                    status: row.try_get("status")?,
                    error: row.try_get("error")?,
                    fired_at: row.try_get("fired_at")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn next_slot_stays_anchored_and_moves_past_now() {
        // Daily slot anchored at t=100; polled at t=150 lands on the next day.
        assert_eq!(next_slot_after(100, 86_400, 150), 86_500);
        // Three missed days still land on the single next slot, not now+delta.
        assert_eq!(
            next_slot_after(100, 86_400, 100 + 86_400 * 3 + 1),
            100 + 86_400 * 4
        );
        // A future anchor is its own first slot.
        assert_eq!(next_slot_after(1_000, 60, 150), 1_000);
        // Exact boundary counts as due, so the next slot is one interval out.
        assert_eq!(next_slot_after(100, 60, 100), 160);
        // Degenerate intervals never divide by zero or stall.
        assert_eq!(next_slot_after(100, 0, 250), 251);
    }

    fn test_schedule(id: &str, project_id: &str, next_run_at: i64) -> ScheduleRecord {
        ScheduleRecord {
            id: id.into(),
            project_id: project_id.into(),
            frame_id: None,
            name: "Daily summary".into(),
            prompt: "Summarize today's progress.".into(),
            skill: Some("literature-review".into()),
            interval_secs: 86_400,
            enabled: true,
            next_run_at,
            last_run_at: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    async fn test_store() -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("wisp-schedules-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store.create_project("p1", "proj", "").await.unwrap();
        (store, root)
    }

    #[tokio::test]
    async fn schedule_crud_and_enable_roundtrip() {
        let (store, root) = test_store().await;
        let schedule = test_schedule("s1", "p1", 500);
        store.create_schedule(&schedule).await.unwrap();
        assert_eq!(
            store.get_schedule("s1").await.unwrap(),
            Some(schedule.clone())
        );
        assert_eq!(
            store.list_schedules("p1").await.unwrap(),
            vec![schedule.clone()]
        );
        assert!(store.list_schedules("p2").await.unwrap().is_empty());

        assert!(store.set_schedule_enabled("s1", false, 10).await.unwrap());
        let disabled = store.get_schedule("s1").await.unwrap().unwrap();
        assert!(!disabled.enabled);
        assert!(store.due_schedules(1_000).await.unwrap().is_empty());
        assert!(!store
            .set_schedule_enabled("missing", true, 10)
            .await
            .unwrap());

        // Close and reopen: the disabled state and slot must be durable,
        // not an artifact of the still-open connection pool.
        store.pool.close().await;
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        let reloaded = store.get_schedule("s1").await.unwrap().unwrap();
        assert!(!reloaded.enabled, "enabled=false must survive reopen");
        assert_eq!(reloaded.next_run_at, 500, "next_run_at must survive reopen");
        assert_eq!(reloaded, disabled);

        store.delete_schedule("s1").await.unwrap();
        assert_eq!(store.get_schedule("s1").await.unwrap(), None);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn due_claim_is_atomic_and_advances_the_slot() {
        let (store, root) = test_store().await;
        let mut future = test_schedule("future", "p1", 10_000);
        store.create_schedule(&future).await.unwrap();
        let due = test_schedule("due", "p1", 100);
        store.create_schedule(&due).await.unwrap();

        let due_rows = store.due_schedules(500).await.unwrap();
        assert_eq!(due_rows, vec![due.clone()]);

        let new_next = next_slot_after(due.next_run_at, due.interval_secs, 500);
        assert!(store
            .claim_schedule_fire("due", due.next_run_at, new_next, 500)
            .await
            .unwrap());
        // A second claim for the same slot loses the race.
        assert!(!store
            .claim_schedule_fire("due", due.next_run_at, new_next, 500)
            .await
            .unwrap());
        let claimed = store.get_schedule("due").await.unwrap().unwrap();
        assert_eq!(claimed.next_run_at, new_next);
        assert_eq!(claimed.last_run_at, Some(500));
        // The claimed slot is gone; nothing else is due before `future`.
        assert!(store.due_schedules(9_999).await.unwrap().is_empty());
        assert_eq!(
            store.due_schedules(new_next).await.unwrap(),
            vec![
                // `future` (10_000) comes first: ordering is by next_run_at.
                store.get_schedule("future").await.unwrap().unwrap(),
                claimed.clone()
            ]
        );

        future.enabled = false;
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn schedule_runs_record_and_delete_with_schedule() {
        let (store, root) = test_store().await;
        store
            .create_schedule(&test_schedule("s1", "p1", 100))
            .await
            .unwrap();
        for (id, status, fired_at) in [("r1", "fired", 100), ("r2", "failed", 200)] {
            store
                .record_schedule_run(&ScheduleRunRecord {
                    id: id.into(),
                    schedule_id: "s1".into(),
                    frame_id: Some("f1".into()),
                    status: status.into(),
                    error: (status == "failed").then(|| "boom".into()),
                    fired_at,
                })
                .await
                .unwrap();
        }
        let runs = store.list_schedule_runs("s1", 10).await.unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "r2", "newest first");
        assert_eq!(runs[0].error.as_deref(), Some("boom"));
        assert_eq!(runs[1].id, "r1");
        assert_eq!(store.list_schedule_runs("s1", 1).await.unwrap().len(), 1);

        store.delete_schedule("s1").await.unwrap();
        assert!(store.list_schedule_runs("s1", 10).await.unwrap().is_empty());

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn due_schedules_skips_rows_whose_project_is_gone() {
        let (store, root) = test_store().await;
        store
            .create_schedule(&test_schedule("orphan", "p1", 100))
            .await
            .unwrap();
        // sqlx enables foreign_keys per connection, so a leftover row has to
        // be planted the way a PRAGMA-off or pre-sqlx opener would leave it.
        let mut conn = store.pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("DELETE FROM projects WHERE id='p1'")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
        assert!(
            store.get_schedule("orphan").await.unwrap().is_some(),
            "the leftover schedule row is still present"
        );
        assert!(
            store.due_schedules(1_000).await.unwrap().is_empty(),
            "orphan schedules must not be claimed"
        );

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
