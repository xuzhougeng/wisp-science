//! Metadata cache for fast external CLI session discovery.

use super::Store;
use anyhow::{bail, Result};
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSessionCacheRecord {
    pub source_id: String,
    pub provider: String,
    pub source_path: String,
    pub file_size: i64,
    pub modified_at_ms: i64,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub message_count: i64,
    pub created_at_ms: i64,
    pub last_active_at_ms: i64,
    pub changed_since_import: bool,
}

impl Store {
    pub async fn list_external_session_cache(
        &self,
        source_id: &str,
        provider: &str,
    ) -> Result<Vec<ExternalSessionCacheRecord>> {
        let rows = sqlx::query(
            "SELECT source_id,provider,source_path,file_size,modified_at_ms,session_id,title,cwd,\
             message_count,created_at_ms,last_active_at_ms,changed_since_import \
             FROM external_session_cache WHERE source_id=? AND provider=? \
             ORDER BY last_active_at_ms DESC, source_path",
        )
        .bind(source_id)
        .bind(provider)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExternalSessionCacheRecord {
                    source_id: row.try_get("source_id")?,
                    provider: row.try_get("provider")?,
                    source_path: row.try_get("source_path")?,
                    file_size: row.try_get("file_size")?,
                    modified_at_ms: row.try_get("modified_at_ms")?,
                    session_id: row.try_get("session_id")?,
                    title: row.try_get("title")?,
                    cwd: row.try_get("cwd")?,
                    message_count: row.try_get("message_count")?,
                    created_at_ms: row.try_get("created_at_ms")?,
                    last_active_at_ms: row.try_get("last_active_at_ms")?,
                    changed_since_import: row.try_get::<i64, _>("changed_since_import")? != 0,
                })
            })
            .collect()
    }

    pub async fn replace_external_session_cache(
        &self,
        source_id: &str,
        provider: &str,
        records: &[ExternalSessionCacheRecord],
    ) -> Result<()> {
        if records
            .iter()
            .any(|record| record.source_id != source_id || record.provider != provider)
        {
            bail!("External session cache record has the wrong source or provider");
        }
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM external_session_cache WHERE source_id=? AND provider=?")
            .bind(source_id)
            .bind(provider)
            .execute(&mut *tx)
            .await?;
        for record in records {
            sqlx::query(
                "INSERT INTO external_session_cache(\
                 source_id,provider,source_path,file_size,modified_at_ms,session_id,title,cwd,\
                 message_count,created_at_ms,last_active_at_ms,changed_since_import) \
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&record.source_id)
            .bind(&record.provider)
            .bind(&record.source_path)
            .bind(record.file_size)
            .bind(record.modified_at_ms)
            .bind(&record.session_id)
            .bind(&record.title)
            .bind(&record.cwd)
            .bind(record.message_count)
            .bind(record.created_at_ms)
            .bind(record.last_active_at_ms)
            .bind(record.changed_since_import as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_external_session_cache_synced(
        &self,
        source_id: &str,
        provider: &str,
        source_path: &str,
        message_count: i64,
        last_active_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE external_session_cache SET message_count=?,\
             last_active_at_ms=CASE WHEN ?>0 THEN ? ELSE last_active_at_ms END,\
             changed_since_import=0 WHERE source_id=? AND provider=? AND source_path=?",
        )
        .bind(message_count)
        .bind(last_active_at_ms)
        .bind(last_active_at_ms)
        .bind(source_id)
        .bind(provider)
        .bind(source_path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EXTERNAL_SESSION_CACHE_MIGRATION;

    fn record(path: &str, size: i64) -> ExternalSessionCacheRecord {
        ExternalSessionCacheRecord {
            source_id: "ssh:cpu".into(),
            provider: "claude".into(),
            source_path: path.into(),
            file_size: size,
            modified_at_ms: 10,
            session_id: path.into(),
            title: "Title".into(),
            cwd: "/work".into(),
            message_count: 2,
            created_at_ms: 1,
            last_active_at_ms: 10,
            changed_since_import: true,
        }
    }

    #[tokio::test]
    async fn cache_replaces_prunes_and_marks_synced() {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_external_cache_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .replace_external_session_cache(
                "ssh:cpu",
                "claude",
                &[record("/a.jsonl", 1), record("/b.jsonl", 2)],
            )
            .await
            .unwrap();
        store
            .replace_external_session_cache("ssh:cpu", "claude", &[record("/b.jsonl", 3)])
            .await
            .unwrap();
        store
            .mark_external_session_cache_synced("ssh:cpu", "claude", "/b.jsonl", 9, 20)
            .await
            .unwrap();
        let rows = store
            .list_external_session_cache("ssh:cpu", "claude")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file_size, 3);
        assert_eq!(rows[0].message_count, 9);
        assert!(!rows[0].changed_since_import);

        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(EXTERNAL_SESSION_CACHE_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened
                .list_external_session_cache("ssh:cpu", "claude")
                .await
                .unwrap()
                .len(),
            1
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
