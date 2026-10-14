use super::Store;
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::HashSet;
use wisp_dto::research_checklist::{Item, Outcome, SyncRequest, SyncResponse, SCHEMA};

const PAGE_SIZE: usize = 200;
const COLUMNS: &str = "id,title,day,status,project_id,session_id,sync_revision";

fn item(row: sqlx::sqlite::SqliteRow) -> Result<Item> {
    Ok(Item {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        day: row.try_get("day")?,
        status: row.try_get("status")?,
        project_id: row.try_get("project_id")?,
        session_id: row.try_get("session_id")?,
        revision: row.try_get("sync_revision")?,
    })
}

fn visible(item: &Item, projects: &HashSet<String>) -> bool {
    item.project_id
        .as_ref()
        .is_none_or(|id| projects.contains(id))
}

fn validate(request: &SyncRequest) -> Result<()> {
    ensure!(
        request.cursor >= 0 && request.changes.len() <= 50,
        "Invalid checklist sync batch"
    );
    let mut ids = HashSet::new();
    let mut mutations = HashSet::new();
    for change in &request.changes {
        ensure!(
            ids.insert(&change.id) && mutations.insert(&change.mutation_id),
            "Duplicate checklist mutation"
        );
        ensure!(
            uuid::Uuid::parse_str(&change.mutation_id).is_ok(),
            "Invalid mutation id"
        );
        ensure!(
            !change.id.is_empty() && change.id.len() <= 128 && change.base_revision >= 0,
            "Invalid checklist identity"
        );
        if change.base_revision == 0 {
            ensure!(
                uuid::Uuid::parse_str(&change.id).is_ok(),
                "New checklist items need a UUID"
            );
        }
        ensure!(
            !change.title.trim().is_empty() && change.title.chars().count() <= 1000,
            "Use a title of 1–1000 characters"
        );
        let date = chrono::NaiveDate::parse_from_str(&change.day, "%Y-%m-%d")?;
        ensure!(date.to_string() == change.day, "Use YYYY-MM-DD");
        ensure!(
            super::ASSISTANT_TASK_STATUSES.contains(&change.status.as_str()),
            "Unknown checklist status"
        );
    }
    Ok(())
}

impl Store {
    /// Atomic compare-and-set plus a bounded change feed. Replaying a committed
    /// edit converges on the same values, while concurrent edits are returned as
    /// conflicts. The existing plan APIs also advance revisions via SQL triggers.
    pub async fn sync_research_checklist(
        &self,
        request: &SyncRequest,
        visible_projects: &HashSet<String>,
    ) -> Result<SyncResponse> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.sync_research_checklist(request, visible_projects)).await;
        }
        validate(request)?;
        let mut projects: Vec<_> = visible_projects.iter().cloned().collect();
        projects.sort();
        let visibility = hex::encode(Sha256::digest(serde_json::to_vec(&projects)?));
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let clock =
            sqlx::query("SELECT epoch,revision FROM research_checklist_clock WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        let epoch: String = clock.try_get("epoch")?;
        let revision: i64 = clock.try_get("revision")?;
        let reset = request.epoch.as_deref() != Some(epoch.as_str()) || request.cursor > revision;
        let mut outcomes = Vec::new();
        if !reset {
            for change in &request.changes {
                let current =
                    sqlx::query(&format!("SELECT {COLUMNS} FROM assistant_tasks WHERE id=?"))
                        .bind(&change.id)
                        .fetch_optional(&mut *tx)
                        .await?
                        .map(item)
                        .transpose()?;
                let mut status = "applied";
                match current.as_ref() {
                    Some(row) if !visible(row, visible_projects) => status = "unavailable",
                    Some(row)
                        if row.title == change.title.trim()
                            && row.day == change.day
                            && row.status == change.status => {}
                    Some(row) if row.revision == change.base_revision => {
                        sqlx::query("UPDATE assistant_tasks SET title=?,day=?,status=?,updated_at=? WHERE id=?")
                            .bind(change.title.trim()).bind(&change.day).bind(&change.status)
                            .bind(chrono::Utc::now().timestamp()).bind(&change.id).execute(&mut *tx).await?;
                    }
                    None if change.base_revision == 0 => {
                        let now = chrono::Utc::now().timestamp();
                        sqlx::query("INSERT INTO assistant_tasks(id,day,title,status,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                            .bind(&change.id).bind(&change.day).bind(change.title.trim()).bind(&change.status)
                            .bind(now).bind(now).execute(&mut *tx).await?;
                    }
                    _ => status = "conflict",
                }
                let item = if status == "unavailable" {
                    None
                } else {
                    sqlx::query(&format!("SELECT {COLUMNS} FROM assistant_tasks WHERE id=?"))
                        .bind(&change.id)
                        .fetch_optional(&mut *tx)
                        .await?
                        .map(item)
                        .transpose()?
                };
                outcomes.push(Outcome {
                    mutation_id: change.mutation_id.clone(),
                    id: change.id.clone(),
                    status: status.into(),
                    item,
                });
            }
        }
        let from = if reset || request.visibility.as_deref() != Some(visibility.as_str()) {
            0
        } else {
            request.cursor
        };
        // Scan hidden rows too so a page made only of hidden items still advances.
        let mut rows = sqlx::query(&format!("SELECT {COLUMNS} FROM assistant_tasks WHERE sync_revision>? ORDER BY sync_revision LIMIT ?"))
            .bind(from).bind((PAGE_SIZE + 1) as i64).fetch_all(&mut *tx).await?
            .into_iter().map(item).collect::<Result<Vec<_>>>()?;
        let has_more = rows.len() > PAGE_SIZE;
        rows.truncate(PAGE_SIZE);
        let cursor = if has_more {
            rows.last().map_or(from, |row| row.revision)
        } else {
            sqlx::query_scalar("SELECT revision FROM research_checklist_clock WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?
        };
        rows.retain(|row| visible(row, visible_projects));
        tx.commit().await?;
        Ok(SyncResponse {
            schema: SCHEMA.into(),
            epoch,
            cursor,
            visibility,
            visible_project_ids: projects,
            reset,
            has_more,
            items: rows,
            outcomes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssistantTask;
    use wisp_dto::research_checklist::Change;

    fn follow(reply: &SyncResponse) -> SyncRequest {
        SyncRequest {
            epoch: Some(reply.epoch.clone()),
            cursor: reply.cursor,
            visibility: Some(reply.visibility.clone()),
            changes: vec![],
        }
    }
    fn change(id: &str, revision: i64, title: &str) -> Change {
        Change {
            id: id.into(),
            mutation_id: uuid::Uuid::new_v4().to_string(),
            base_revision: revision,
            title: title.into(),
            day: "2026-10-14".into(),
            status: "open".into(),
        }
    }
    async fn seed(store: &Store, id: &str, project: Option<&str>) {
        store
            .add_assistant_task(&AssistantTask {
                id: id.into(),
                day: "2026-10-14".into(),
                title: "desktop".into(),
                project_id: project.map(str::to_owned),
                session_id: None,
                status: "open".into(),
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn checklist_cas_retry_conflicts_and_desktop_tombstones() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        let visible = HashSet::new();
        let first = store
            .sync_research_checklist(&SyncRequest::default(), &visible)
            .await
            .unwrap();
        assert!(first.reset);
        let id = uuid::Uuid::new_v4().to_string();
        let mut request = follow(&first);
        request.changes.push(change(&id, 0, "phone"));
        let created = store
            .sync_research_checklist(&request, &visible)
            .await
            .unwrap();
        assert_eq!(created.outcomes[0].status, "applied");
        assert_eq!(created.items[0].project_id, None);
        assert_eq!(created.items[0].session_id, None);
        let retry = store
            .sync_research_checklist(&request, &visible)
            .await
            .unwrap();
        assert_eq!(
            retry.cursor, created.cursor,
            "lost response must not create another revision"
        );
        let mut stale = follow(&created);
        stale
            .changes
            .push(change(&id, created.items[0].revision, "offline edit"));
        store
            .set_assistant_task_status(&id, "done", 0)
            .await
            .unwrap(); // clock moved backwards
        let conflict = store
            .sync_research_checklist(&stale, &visible)
            .await
            .unwrap();
        assert_eq!(conflict.outcomes[0].status, "conflict");
        assert_eq!(conflict.outcomes[0].item.as_ref().unwrap().status, "done");
        assert!(conflict.cursor > created.cursor);
        stale.changes[0].base_revision = conflict.outcomes[0].item.as_ref().unwrap().revision;
        let resolved = store
            .sync_research_checklist(&stale, &visible)
            .await
            .unwrap();
        assert_eq!(resolved.outcomes[0].status, "applied");
        store
            .set_assistant_task_status(&id, "dropped", 0)
            .await
            .unwrap();
        let dropped = store
            .sync_research_checklist(&follow(&resolved), &visible)
            .await
            .unwrap();
        assert_eq!(dropped.items[0].status, "dropped");
        assert!(dropped.items[0].revision > resolved.cursor);
    }

    #[tokio::test]
    async fn checklist_privacy_pagination_and_unhide_without_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        for i in 0..201 {
            seed(&store, &format!("hidden-{i}"), Some("private")).await;
        }
        seed(&store, "public", None).await;
        let empty = HashSet::new();
        let page = store
            .sync_research_checklist(&SyncRequest::default(), &empty)
            .await
            .unwrap();
        assert!(page.has_more);
        assert!(page.items.is_empty());
        assert!(page.cursor > 0);
        let end = store
            .sync_research_checklist(&follow(&page), &empty)
            .await
            .unwrap();
        assert!(!end.has_more);
        assert_eq!(end.items.len(), 1);
        assert_eq!(end.items[0].id, "public");
        let visible = HashSet::from(["private".to_string()]);
        let unhidden = store
            .sync_research_checklist(&follow(&end), &visible)
            .await
            .unwrap();
        assert_eq!(unhidden.items.len(), 200);
        assert!(unhidden.has_more);
        let hidden = &unhidden.items[0];
        let mut write = follow(&unhidden);
        write
            .changes
            .push(change(&hidden.id, hidden.revision, "do not leak"));
        let denied = store.sync_research_checklist(&write, &empty).await.unwrap();
        assert_eq!(denied.outcomes[0].status, "unavailable");
        assert!(denied.outcomes[0].item.is_none());
        assert!(denied.items.iter().all(|item| item.project_id.is_none()));
    }

    #[tokio::test]
    async fn checklist_migration_reopen_reset_and_invalid_batch_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let store = Store::open(&path).await.unwrap();
        seed(&store, "legacy", None).await;
        // Simulate an existing plan table before this additive migration.
        sqlx::raw_sql("DROP TRIGGER research_checklist_insert; DROP TRIGGER research_checklist_update; UPDATE assistant_tasks SET sync_revision=0;").execute(&store.pool).await.unwrap();
        store.pool.close().await;
        let store = Store::open(&path).await.unwrap();
        let first = store
            .sync_research_checklist(&SyncRequest::default(), &HashSet::new())
            .await
            .unwrap();
        assert!(first.items[0].revision > 0);
        store.pool.close().await;
        let store = Store::open(&path).await.unwrap();
        let unchanged = store
            .sync_research_checklist(&follow(&first), &HashSet::new())
            .await
            .unwrap();
        assert_eq!(first.epoch, unchanged.epoch);
        assert_eq!(first.cursor, unchanged.cursor);
        assert!(unchanged.items.is_empty());
        let mut invalid = follow(&first);
        invalid.changes = vec![
            change(&uuid::Uuid::new_v4().to_string(), 0, "valid"),
            change(&uuid::Uuid::new_v4().to_string(), 0, "invalid"),
        ];
        invalid.changes[1].day = "2026-02-30".into();
        assert!(store
            .sync_research_checklist(&invalid, &HashSet::new())
            .await
            .is_err());
        assert!(store
            .sync_research_checklist(&follow(&first), &HashSet::new())
            .await
            .unwrap()
            .items
            .is_empty());
        invalid.changes.pop();
        invalid.epoch = Some("replaced-host".into());
        let reset = store
            .sync_research_checklist(&invalid, &HashSet::new())
            .await
            .unwrap();
        assert!(reset.reset);
        assert!(reset.outcomes.is_empty());
        assert_eq!(reset.items.len(), 1);
        invalid = follow(&first);
        invalid.cursor += 100;
        assert!(
            store
                .sync_research_checklist(&invalid, &HashSet::new())
                .await
                .unwrap()
                .reset
        );
    }
}
