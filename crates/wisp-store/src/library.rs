use anyhow::{bail, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

/// App-global, immutable snapshots. This deliberately uses a separate SQLite
/// pool from [`crate::Store`], so project/session cascades cannot delete stars.
#[derive(Clone)]
pub struct LibraryStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
    pub code: String,
    pub content_type: Option<String>,
    pub source_project_id: String,
    pub source_project_name: String,
    pub source_session_id: String,
    pub source_session_title: String,
    pub source_path: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct LibraryItemDetail {
    pub item: LibraryItem,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct NewLibraryItem {
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
    pub code: String,
    pub content_type: Option<String>,
    pub content: Option<Vec<u8>>,
    pub source_project_id: String,
    pub source_project_name: String,
    pub source_session_id: String,
    pub source_session_title: String,
    pub source_path: Option<String>,
}

const LIBRARY_ITEMS_DDL: &str = "CREATE TABLE IF NOT EXISTS library_items (\
     id TEXT PRIMARY KEY, \
     kind TEXT NOT NULL CHECK(kind IN ('code','figure','text')), \
     title TEXT NOT NULL, language TEXT, code TEXT NOT NULL DEFAULT '', \
     content_type TEXT, content_blob BLOB, content_sha256 TEXT NOT NULL, \
     source_project_id TEXT NOT NULL, source_project_name TEXT NOT NULL, \
     source_session_id TEXT NOT NULL, source_session_title TEXT NOT NULL, \
     source_path TEXT, source_key TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, \
     CHECK((kind IN ('code','text') AND content_blob IS NULL) OR \
           (kind='figure' AND content_blob IS NOT NULL)))";

impl LibraryStore {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        // Older databases baked `kind IN ('code','figure')` into a CHECK
        // constraint; SQLite cannot alter it in place, so rebuild once.
        let existing_ddl: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='library_items'",
        )
        .fetch_optional(&pool)
        .await?;
        if existing_ddl.is_some_and(|ddl| !ddl.contains("'text'")) {
            let mut tx = pool.begin().await?;
            sqlx::query("ALTER TABLE library_items RENAME TO library_items_legacy")
                .execute(&mut *tx)
                .await?;
            sqlx::query(LIBRARY_ITEMS_DDL).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO library_items SELECT * FROM library_items_legacy")
                .execute(&mut *tx)
                .await?;
            sqlx::query("DROP TABLE library_items_legacy")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        sqlx::query(LIBRARY_ITEMS_DDL).execute(&pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_library_items_created \
             ON library_items(created_at DESC)",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    /// Insert once for a logical source. Re-starring the same code cell,
    /// text excerpt, or figure path returns its original immutable snapshot.
    pub async fn insert(&self, item: NewLibraryItem) -> Result<LibraryItem> {
        if !matches!(item.kind.as_str(), "code" | "figure" | "text") {
            bail!("unsupported library item kind: {}", item.kind);
        }
        if item.title.trim().is_empty() {
            bail!("library item title is required");
        }
        if item.kind != "figure" && item.content.is_some() {
            bail!("only figure library items can contain a binary snapshot");
        }
        if item.kind == "figure" && item.content.is_none() {
            bail!("figure library items require a binary snapshot");
        }

        let content_hash = if let Some(content) = item.content.as_deref() {
            hex::encode(Sha256::digest(content))
        } else {
            let mut hasher = Sha256::new();
            hasher.update(item.language.as_deref().unwrap_or_default().as_bytes());
            hasher.update([0]);
            hasher.update(item.code.as_bytes());
            hex::encode(hasher.finalize())
        };
        let source_key = if item.kind == "figure" {
            format!(
                "figure\0{}\0{}\0{}",
                item.source_project_id,
                item.source_session_id,
                item.source_path.as_deref().unwrap_or_default()
            )
        } else {
            format!(
                "{}\0{}\0{}\0{}",
                item.kind, item.source_project_id, item.source_session_id, content_hash
            )
        };
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO library_items(\
             id,kind,title,language,code,content_type,content_blob,content_sha256,\
             source_project_id,source_project_name,source_session_id,source_session_title,\
             source_path,source_key,created_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(source_key) DO NOTHING",
        )
        .bind(id)
        .bind(&item.kind)
        .bind(item.title.trim())
        .bind(&item.language)
        .bind(&item.code)
        .bind(&item.content_type)
        .bind(&item.content)
        .bind(content_hash)
        .bind(&item.source_project_id)
        .bind(&item.source_project_name)
        .bind(&item.source_session_id)
        .bind(&item.source_session_title)
        .bind(&item.source_path)
        .bind(&source_key)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        self.get_by_source_key(&source_key)
            .await?
            .map(|detail| detail.item)
            .ok_or_else(|| anyhow::anyhow!("failed to read inserted library item"))
    }

    pub async fn list(&self) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(
            "SELECT id,kind,title,language,code,content_type,source_project_id,\
             source_project_name,source_session_id,source_session_title,source_path,created_at \
             FROM library_items ORDER BY created_at DESC,rowid DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_item).collect()
    }

    pub async fn get(&self, id: &str) -> Result<Option<LibraryItemDetail>> {
        let row = sqlx::query(
            "SELECT id,kind,title,language,code,content_type,source_project_id,\
             source_project_name,source_session_id,source_session_title,source_path,created_at,\
             content_blob FROM library_items WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_detail).transpose()
    }

    async fn get_by_source_key(&self, key: &str) -> Result<Option<LibraryItemDetail>> {
        let row = sqlx::query(
            "SELECT id,kind,title,language,code,content_type,source_project_id,\
             source_project_name,source_session_id,source_session_title,source_path,created_at,\
             content_blob FROM library_items WHERE source_key=?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_detail).transpose()
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM library_items WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }
}

fn row_to_item(row: SqliteRow) -> Result<LibraryItem> {
    Ok(LibraryItem {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        title: row.try_get("title")?,
        language: row.try_get("language")?,
        code: row.try_get("code")?,
        content_type: row.try_get("content_type")?,
        source_project_id: row.try_get("source_project_id")?,
        source_project_name: row.try_get("source_project_name")?,
        source_session_id: row.try_get("source_session_id")?,
        source_session_title: row.try_get("source_session_title")?,
        source_path: row.try_get("source_path")?,
        created_at: row.try_get("created_at")?,
    })
}

fn row_to_detail(row: SqliteRow) -> Result<LibraryItemDetail> {
    let content = row.try_get("content_blob")?;
    Ok(LibraryItemDetail {
        item: row_to_item(row)?,
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use wisp_llm::Message;

    fn new_item(kind: &str) -> NewLibraryItem {
        NewLibraryItem {
            kind: kind.into(),
            title: if kind == "code" {
                "print(1)".into()
            } else {
                "plot.png".into()
            },
            language: Some("python".into()),
            code: "print(1)".into(),
            content_type: (kind == "figure").then(|| "image/png".into()),
            content: (kind == "figure").then(|| vec![1, 2, 3, 4]),
            source_project_id: "project-1".into(),
            source_project_name: "Project one".into(),
            source_session_id: "session-1".into(),
            source_session_title: "Analysis".into(),
            source_path: (kind == "figure").then(|| "figures/plot.png".into()),
        }
    }

    async fn store() -> LibraryStore {
        let path = std::env::temp_dir()
            .join(format!("wisp-library-test-{}", uuid::Uuid::new_v4()))
            .join("library.sqlite");
        LibraryStore::open(&path).await.unwrap()
    }

    #[tokio::test]
    async fn snapshots_are_deduplicated_and_keep_binary_content() {
        let store = store().await;
        let first = store.insert(new_item("figure")).await.unwrap();
        let second = store.insert(new_item("figure")).await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(
            store.get(&first.id).await.unwrap().unwrap().content,
            Some(vec![1, 2, 3, 4])
        );
    }

    #[tokio::test]
    async fn text_excerpts_are_stored_and_deduplicated_per_session() {
        let store = store().await;
        let excerpt = NewLibraryItem {
            kind: "text".into(),
            title: "The p-value is significant".into(),
            language: None,
            code: "The p-value is significant across all replicates.".into(),
            content_type: None,
            content: None,
            source_path: None,
            ..new_item("code")
        };
        let first = store.insert(excerpt.clone()).await.unwrap();
        let second = store.insert(excerpt.clone()).await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, "text");
        // The same excerpt starred as code is a distinct snapshot.
        let as_code = store
            .insert(NewLibraryItem {
                kind: "code".into(),
                ..excerpt
            })
            .await
            .unwrap();
        assert_ne!(first.id, as_code.id);
    }

    #[tokio::test]
    async fn legacy_databases_are_rebuilt_to_accept_text_items() {
        let path = std::env::temp_dir()
            .join(format!(
                "wisp-library-migrate-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("library.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let opts =
            sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
                .unwrap()
                .create_if_missing(true);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::query(
            "CREATE TABLE library_items (\
             id TEXT PRIMARY KEY, \
             kind TEXT NOT NULL CHECK(kind IN ('code','figure')), \
             title TEXT NOT NULL, language TEXT, code TEXT NOT NULL DEFAULT '', \
             content_type TEXT, content_blob BLOB, content_sha256 TEXT NOT NULL, \
             source_project_id TEXT NOT NULL, source_project_name TEXT NOT NULL, \
             source_session_id TEXT NOT NULL, source_session_title TEXT NOT NULL, \
             source_path TEXT, source_key TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, \
             CHECK((kind='code' AND content_blob IS NULL) OR \
                   (kind='figure' AND content_blob IS NOT NULL)))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO library_items(id,kind,title,code,content_sha256,\
             source_project_id,source_project_name,source_session_id,source_session_title,\
             source_key,created_at) VALUES('legacy-1','code','print(1)','print(1)','hash',\
             'project-1','Project one','session-1','Analysis',?,0)",
        )
        .bind("code\0k")
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let store = LibraryStore::open(&path).await.unwrap();
        let excerpt = NewLibraryItem {
            kind: "text".into(),
            title: "excerpt".into(),
            language: None,
            content_type: None,
            content: None,
            source_path: None,
            ..new_item("code")
        };
        store.insert(excerpt).await.unwrap();
        let items = store.list().await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.id == "legacy-1"));
        assert!(items.iter().any(|item| item.kind == "text"));
    }

    #[tokio::test]
    async fn deleting_a_star_does_not_touch_other_snapshots() {
        let store = store().await;
        let figure = store.insert(new_item("figure")).await.unwrap();
        let code = store.insert(new_item("code")).await.unwrap();
        assert!(store.delete(&figure.id).await.unwrap());
        assert!(store.get(&figure.id).await.unwrap().is_none());
        assert_eq!(store.get(&code.id).await.unwrap().unwrap().item, code);
    }

    #[tokio::test]
    async fn project_deletion_cannot_cascade_into_the_global_library() {
        let dir = std::env::temp_dir().join(format!(
            "wisp-library-separate-db-test-{}",
            uuid::Uuid::new_v4()
        ));
        let project_store = Store::open(&dir.join("wisp.sqlite")).await.unwrap();
        let library = LibraryStore::open(&dir.join("library.sqlite"))
            .await
            .unwrap();
        let project_root = dir.join("project-one").to_string_lossy().into_owned();
        project_store
            .create_project("project-1", "Project one", &project_root)
            .await
            .unwrap();
        project_store
            .create_frame("session-1", "project-1", "OPERON", "model")
            .await
            .unwrap();
        project_store
            .append_message("session-1", 1, &Message::user("make a plot"))
            .await
            .unwrap();
        let starred = library.insert(new_item("figure")).await.unwrap();

        project_store.delete_project("project-1").await.unwrap();

        assert!(project_store
            .get_project("project-1")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            library.get(&starred.id).await.unwrap().unwrap().content,
            Some(vec![1, 2, 3, 4])
        );
    }
}
