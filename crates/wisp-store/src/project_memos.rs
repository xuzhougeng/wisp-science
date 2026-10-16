//! Researcher-private project memos.
//!
//! A memo is the researcher's own note inside a project: a prompt draft, a
//! reminder, a habit for one server. It is deliberately outside the agent's
//! view. Nothing here is read by prompt assembly, by a tool, by the research
//! journey projection or by a conversation export; the UI pastes a memo into
//! the composer and only the sent message reaches the model. Memos are shared
//! by the mainline and every exploration branch and never touch the project's
//! state generation, because they are not research state.
use super::Store;
use anyhow::{bail, Result};
use sqlx::Row;
use wisp_dto::{ProjectMemo, ProjectMemoInput, ProjectMemoPatch};

const TITLE_LIMIT: usize = 200;
const BODY_LIMIT: usize = 20_000;

fn validate_title(title: &str) -> Result<&str> {
    let title = title.trim();
    if title.chars().count() > TITLE_LIMIT {
        bail!("A memo title holds at most {TITLE_LIMIT} characters");
    }
    Ok(title)
}

fn validate_body(body: &str) -> Result<&str> {
    let body = body.trim();
    if body.is_empty() || body.chars().count() > BODY_LIMIT {
        bail!("A memo body of 1–{BODY_LIMIT} characters is required");
    }
    Ok(body)
}

fn memo_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ProjectMemo> {
    Ok(ProjectMemo {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        pinned: row.try_get::<i64, _>("pinned")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const MEMO_COLUMNS: &str = "id,project_id,title,body,pinned,created_at,updated_at";

impl Store {
    /// Pinned memos first, then most recently updated.
    pub async fn list_project_memos(&self, project_id: &str) -> Result<Vec<ProjectMemo>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_project_memos(project_id)).await;
        }
        let rows = sqlx::query(&format!(
            "SELECT {MEMO_COLUMNS} FROM project_memos WHERE project_id=? \
             ORDER BY pinned DESC, updated_at DESC, id DESC"
        ))
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(memo_from_row).collect()
    }

    pub async fn add_project_memo(
        &self,
        project_id: &str,
        input: &ProjectMemoInput,
    ) -> Result<ProjectMemo> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.add_project_memo(project_id, input)).await;
        }
        let title = validate_title(&input.title)?;
        let body = validate_body(&input.body)?;
        if self.get_project(project_id).await?.is_none() {
            bail!("project not found");
        }
        let mut tx = self.begin_write().await?;
        // Strictly later than every memo already in the project, so a memo
        // created in the same second still lists first.
        let latest: Option<i64> =
            sqlx::query_scalar("SELECT MAX(updated_at) FROM project_memos WHERE project_id=?")
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
        let now = chrono::Utc::now()
            .timestamp()
            .max(latest.map_or(0, |latest| latest + 1));
        // Fresh ids: a deleted memo's id is never reused.
        let memo = ProjectMemo {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            pinned: false,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(&format!(
            "INSERT INTO project_memos({MEMO_COLUMNS}) VALUES(?,?,?,?,?,?,?)"
        ))
        .bind(&memo.id)
        .bind(&memo.project_id)
        .bind(&memo.title)
        .bind(&memo.body)
        .bind(0i64)
        .bind(memo.created_at)
        .bind(memo.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(memo)
    }

    /// Apply the patch to one of this project's memos. A memo id from another
    /// project is "not found", never modified.
    pub async fn update_project_memo(
        &self,
        project_id: &str,
        patch: &ProjectMemoPatch,
    ) -> Result<ProjectMemo> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.update_project_memo(project_id, patch)).await;
        }
        let title = patch.title.as_deref().map(validate_title).transpose()?;
        let body = patch.body.as_deref().map(validate_body).transpose()?;
        let mut tx = self.begin_write().await?;
        let current = sqlx::query(&format!(
            "SELECT {MEMO_COLUMNS} FROM project_memos WHERE id=? AND project_id=?"
        ))
        .bind(&patch.id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(memo_from_row)
        .transpose()?;
        let Some(current) = current else {
            bail!("Memo not found");
        };
        let next = ProjectMemo {
            title: title.map(str::to_string).unwrap_or(current.title),
            body: body.map(str::to_string).unwrap_or(current.body),
            pinned: patch.pinned.unwrap_or(current.pinned),
            // Strictly increasing so the list order never ties with a stale row.
            updated_at: chrono::Utc::now().timestamp().max(current.updated_at + 1),
            ..current
        };
        sqlx::query(
            "UPDATE project_memos SET title=?,body=?,pinned=?,updated_at=? WHERE id=? AND project_id=?",
        )
        .bind(&next.title)
        .bind(&next.body)
        .bind(next.pinned as i64)
        .bind(next.updated_at)
        .bind(&next.id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(next)
    }

    /// Hard delete; memos have no audit value.
    pub async fn delete_project_memo(&self, project_id: &str, memo_id: &str) -> Result<()> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.delete_project_memo(project_id, memo_id)).await;
        }
        let mut tx = self.begin_write().await?;
        let deleted = sqlx::query("DELETE FROM project_memos WHERE id=? AND project_id=?")
            .bind(memo_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        if deleted == 0 {
            bail!("Memo not found");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StateScope;

    fn memo(title: &str, body: &str) -> ProjectMemoInput {
        ProjectMemoInput {
            title: title.into(),
            body: body.into(),
        }
    }

    #[tokio::test]
    async fn memos_stay_inside_their_project_and_order_pinned_first() {
        let path = std::env::temp_dir().join(format!("memos-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_project("other", "Other", "").await.unwrap();

        let first = store
            .add_project_memo("p", &memo("  Server habits ", " ssh gpu02 first\n"))
            .await
            .unwrap();
        assert_eq!(first.title, "Server habits");
        assert_eq!(first.body, "ssh gpu02 first");
        assert!(!first.pinned);
        let second = store
            .add_project_memo("p", &memo("", "Opening prompt"))
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        store
            .add_project_memo("other", &memo("Elsewhere", "not mine"))
            .await
            .unwrap();

        // Validation: empty body, oversized title, unknown project.
        assert!(store
            .add_project_memo("p", &memo("t", "   "))
            .await
            .is_err());
        assert!(store
            .add_project_memo("p", &memo(&"x".repeat(201), "body"))
            .await
            .is_err());
        assert!(store
            .add_project_memo("missing", &memo("t", "body"))
            .await
            .is_err());

        let listed = store.list_project_memos("p").await.unwrap();
        assert_eq!(
            listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec![second.id.as_str(), first.id.as_str()],
            "newest first before any pin"
        );

        let pinned = store
            .update_project_memo(
                "p",
                &ProjectMemoPatch {
                    id: first.id.clone(),
                    pinned: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(pinned.pinned);
        assert!(pinned.updated_at > first.updated_at);
        assert_eq!(pinned.body, first.body, "untouched fields survive a patch");
        let listed = store.list_project_memos("p").await.unwrap();
        assert_eq!(listed[0].id, first.id, "pinned memos lead the list");

        let edited = store
            .update_project_memo(
                "p",
                &ProjectMemoPatch {
                    id: second.id.clone(),
                    title: Some(" Prompt ".into()),
                    body: Some("You are a careful analyst.".into()),
                    pinned: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.title, "Prompt");
        assert_eq!(edited.body, "You are a careful analyst.");
        assert!(store
            .update_project_memo(
                "p",
                &ProjectMemoPatch {
                    id: second.id.clone(),
                    body: Some(String::new()),
                    ..Default::default()
                },
            )
            .await
            .is_err());

        // Another project's id is not found, and nothing changes.
        assert!(store
            .update_project_memo(
                "other",
                &ProjectMemoPatch {
                    id: first.id.clone(),
                    pinned: Some(false),
                    ..Default::default()
                },
            )
            .await
            .is_err());
        assert!(store.delete_project_memo("other", &first.id).await.is_err());
        assert!(store.list_project_memos("p").await.unwrap()[0].pinned);
        assert_eq!(store.list_project_memos("other").await.unwrap().len(), 1);

        store.delete_project_memo("p", &second.id).await.unwrap();
        assert!(store.delete_project_memo("p", &second.id).await.is_err());
        assert_eq!(store.list_project_memos("p").await.unwrap().len(), 1);

        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn memos_never_enter_the_research_journey_or_state_revisions() {
        let path = std::env::temp_dir().join(format!("memos-journey-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        let scope = StateScope::mainline("p");
        let before = store.state_generation(&scope).await.unwrap();
        store
            .add_project_memo("p", &memo("Secret", "Only for me"))
            .await
            .unwrap();
        assert_eq!(
            store.state_generation(&scope).await.unwrap(),
            before,
            "memos are not research state"
        );
        let now = chrono::Utc::now().timestamp();
        let journey = store
            .research_journey(&scope, now - 86_400, now + 86_400)
            .await
            .unwrap();
        assert!(
            journey.entries.iter().all(|entry| {
                !entry.title.contains("Secret") && !entry.summary.contains("Only for me")
            }),
            "the journey projection must not surface private memos"
        );
        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn memos_travel_with_project_bundles_and_old_bundles_still_import() {
        let token = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("memos-transfer-{token}.db"));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        let empty_export = std::env::temp_dir().join(format!("memos-empty-{token}.db"));
        store
            .export_project_database("p", &empty_export)
            .await
            .unwrap();
        let empty_hash = Store::portable_project_database_hash(&empty_export)
            .await
            .unwrap();
        let snapshot = Store::open_snapshot(&empty_export).await.unwrap();
        sqlx::query("DROP TABLE project_memos")
            .execute(&snapshot.pool)
            .await
            .unwrap();
        snapshot.pool.close().await;
        assert_eq!(
            Store::portable_project_database_hash(&empty_export)
                .await
                .unwrap(),
            empty_hash,
            "an empty memo table must not change published fingerprints"
        );

        let kept = store
            .add_project_memo("p", &memo("Keep", "travels with the bundle"))
            .await
            .unwrap();
        let export = std::env::temp_dir().join(format!("memos-export-{token}.db"));
        store.export_project_database("p", &export).await.unwrap();
        assert_ne!(
            Store::portable_project_database_hash(&export)
                .await
                .unwrap(),
            empty_hash
        );
        let copy = Store::open_snapshot(&export).await.unwrap();
        assert_eq!(
            copy.list_project_memos("p").await.unwrap(),
            vec![kept.clone()]
        );
        copy.pool.close().await;

        // Import into a fresh store restores the memo; a bundle without the
        // table (older release) imports with none.
        let target_path = std::env::temp_dir().join(format!("memos-target-{token}.db"));
        let target = Store::open(&target_path).await.unwrap();
        let workspace = std::env::temp_dir().join(format!("memos-ws-{token}"));
        std::fs::create_dir_all(&workspace).unwrap();
        target
            .import_project_database(&export, "p", &workspace)
            .await
            .unwrap();
        assert_eq!(target.list_project_memos("p").await.unwrap(), vec![kept]);
        let old_target_path = std::env::temp_dir().join(format!("memos-old-target-{token}.db"));
        let old_target = Store::open(&old_target_path).await.unwrap();
        let old_workspace = std::env::temp_dir().join(format!("memos-old-ws-{token}"));
        std::fs::create_dir_all(&old_workspace).unwrap();
        old_target
            .import_project_database(&empty_export, "p", &old_workspace)
            .await
            .unwrap();
        assert!(old_target.list_project_memos("p").await.unwrap().is_empty());

        store.delete_project("p").await.unwrap();
        assert!(store.list_project_memos("p").await.unwrap().is_empty());

        for pool in [store.pool, target.pool, old_target.pool] {
            pool.close().await;
        }
        for file in [path, empty_export, export, target_path, old_target_path] {
            let _ = std::fs::remove_file(file);
        }
        for dir in [workspace, old_workspace] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
