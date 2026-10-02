use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalMemory {
    pub id: String,
    pub content: String,
    pub source_frame_id: Option<String>,
    pub source_turn_index: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Store {
    pub async fn insert_global_memory(&self, memory: &GlobalMemory) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.insert_global_memory(memory)).await;
        }
        sqlx::query(
            "INSERT INTO global_memories(\
             id,content,source_frame_id,source_turn_index,created_at,updated_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&memory.id)
        .bind(memory.content.trim())
        .bind(memory.source_frame_id.as_deref())
        .bind(memory.source_turn_index)
        .bind(memory.created_at)
        .bind(memory.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_global_memories(&self, limit: usize) -> Result<Vec<GlobalMemory>> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.list_global_memories(limit)).await;
        }
        let rows = sqlx::query(
            "SELECT id,content,source_frame_id,source_turn_index,created_at,updated_at \
             FROM global_memories ORDER BY updated_at DESC,id DESC LIMIT ?",
        )
        .bind(limit.clamp(1, 100) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GlobalMemory {
                    id: row.try_get("id")?,
                    content: row.try_get("content")?,
                    source_frame_id: row.try_get("source_frame_id")?,
                    source_turn_index: row.try_get("source_turn_index")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn update_global_memory(
        &self,
        id: &str,
        content: &str,
        updated_at: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.update_global_memory(id, content, updated_at)).await;
        }
        let result = sqlx::query(
            "UPDATE global_memories SET content=?,updated_at=MAX(?,updated_at+1) WHERE id=?",
        )
        .bind(content.trim())
        .bind(updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_global_memory(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.delete_global_memory(id)).await;
        }
        sqlx::query("DELETE FROM global_memories WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn global_memory_roundtrips_and_deletes() {
        let root = std::env::temp_dir().join(format!("wisp-global-memory-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        let memory = GlobalMemory {
            id: "m1".into(),
            content: "Prefer Chinese answers.".into(),
            source_frame_id: None,
            source_turn_index: Some(2),
            created_at: 10,
            updated_at: 10,
        };

        store.insert_global_memory(&memory).await.unwrap();
        assert!(store
            .update_global_memory("m1", "Prefer concise Chinese answers.", 20)
            .await
            .unwrap());
        assert!(!store
            .update_global_memory("missing", "unused", 20)
            .await
            .unwrap());
        let mut updated = memory.clone();
        updated.content = "Prefer concise Chinese answers.".into();
        updated.updated_at = 20;
        assert_eq!(store.list_global_memories(10).await.unwrap(), vec![updated]);
        store.delete_global_memory("m1").await.unwrap();
        assert!(store.list_global_memories(10).await.unwrap().is_empty());

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
