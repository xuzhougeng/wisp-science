//! SQLite persistence for Wisp: projects, frames, messages, settings.
//!
//! Replaces the mangopi JSON session file with a structured store. API keys
//! live in the OS keyring (see [`secrets`]); everything else lives here.

pub mod secrets;

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use wisp_llm::Message;

pub const MIGRATION_SQL: &str = include_str!("../migrations/0000_init.sql");

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (or create) the SQLite database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(opts).await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        // Strip `--` line comments before splitting on `;` so semicolons inside
        // comments don't produce bogus statements.
        let stripped: String = MIGRATION_SQL
            .lines()
            .map(|l| match l.split_once("--") { Some((code, _)) => code.to_string(), None => l.to_string() })
            .collect::<Vec<_>>()
            .join("\n");
        for stmt in stripped.split(';') {
            let s = stmt.trim();
            if s.is_empty() { continue; }
            sqlx::query(s).execute(pool).await?;
        }
        Ok(())
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(key).bind(value)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key=?")
            .bind(key).fetch_optional(&self.pool).await?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn create_project(&self, id: &str, name: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO projects(id,name,description,created_at,updated_at) VALUES(?,?,'',?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name, updated_at=excluded.updated_at")
            .bind(id).bind(name).bind(now).bind(now)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn create_frame(&self, id: &str, project_id: &str, agent_name: &str, model: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let sql = "INSERT INTO frames(id,parent_frame_id,root_frame_id,agent_name,status,project_id,model,input_tokens,output_tokens,created_at,updated_at,completed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,NULL)";
        sqlx::query(sql)
            .bind(id).bind(id).bind(id).bind(agent_name).bind("running").bind(project_id).bind(model)
            .bind(0i64).bind(0i64).bind(now).bind(now)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn append_message(&self, frame_id: &str, seq: i64, msg: &Message) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let role = format!("{:?}", msg.role).to_ascii_lowercase();
        let content = serde_json::to_string(&msg.content)?;
        let tool_calls = if msg.tool_calls.is_empty() { None } else { Some(serde_json::to_string(&msg.tool_calls)?) };
        sqlx::query("INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts) VALUES(?,?,?,?,?,?,?,?,?,?)")
            .bind(id).bind(frame_id).bind(seq).bind(role).bind(content)
            .bind(tool_calls)
            .bind(msg.tool_call_id.as_deref())
            .bind(msg.tool_name.as_deref())
            .bind(msg.reasoning.as_deref())
            .bind(msg.ts)
            .execute(&self.pool).await?;
        Ok(())
    }

    /// Drop persisted turns after `keep` (seq is 1-based; keep=3 retains seq 1..=3).
    pub async fn truncate_messages(&self, frame_id: &str, keep: i64) -> Result<()> {
        sqlx::query("DELETE FROM messages WHERE frame_id = ? AND seq > ?")
            .bind(frame_id)
            .bind(keep)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Load all messages for a frame, ordered by sequence.
    pub async fn load_messages(&self, frame_id: &str) -> Result<Vec<Message>> {
        let rows = sqlx::query("SELECT role,content,tool_calls,tool_call_id,tool_name,reasoning,ts FROM messages WHERE frame_id=? ORDER BY seq ASC")
            .bind(frame_id)
            .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let role: String = row.try_get("role")?;
            let content_json: String = row.try_get("content")?;
            let content: wisp_llm::Content = serde_json::from_str(&content_json).unwrap_or(wisp_llm::Content::text(""));
            let tool_calls_json: Option<String> = row.try_get("tool_calls")?;
            let tool_calls: Vec<wisp_llm::ToolCall> = tool_calls_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let tool_call_id: Option<String> = row.try_get("tool_call_id")?;
            let tool_name: Option<String> = row.try_get("tool_name")?;
            let reasoning: Option<String> = row.try_get("reasoning")?;
            let ts: i64 = row.try_get("ts")?;
            let role = parse_role(&role);
            out.push(Message { role, content, tool_calls, tool_call_id, tool_name, reasoning, ts });
        }
        Ok(out)
    }

    /// Root frames that have at least one user turn, newest first, each with a
    /// title derived from its first user message. Used to populate the UI's
    /// session-history sidebar. Returns `(frame_id, title, created_at)`.
    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, f.created_at AS created_at, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS first_user \
             FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role = 'user') \
             ORDER BY f.created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for row in rows {
            let id: String = row.try_get("id")?;
            let created: i64 = row.try_get("created_at")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let title = first_user
                .and_then(|c| serde_json::from_str::<wisp_llm::Content>(&c).ok())
                .map(|c| c.as_text().chars().take(80).collect::<String>())
                .unwrap_or_default();
            out.push((id, title, created));
        }
        Ok(out)
    }

    pub async fn list_root_frames(&self, project_id: &str) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query("SELECT id, agent_name, created_at FROM frames WHERE project_id=? AND parent_frame_id=id ORDER BY created_at DESC")
            .bind(project_id)
            .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            out.push((row.try_get::<String, _>("id")?, row.try_get::<String, _>("agent_name")?, row.try_get::<i64, _>("created_at")?));
        }
        Ok(out)
    }

    /// Persist a workspace artifact record (file already on disk at `storage_path`).
    pub async fn save_artifact(
        &self,
        id: &str,
        project_id: &str,
        root_frame_id: &str,
        filename: &str,
        content_type: &str,
        storage_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO artifacts(id,project_id,root_frame_id,filename,content_type,storage_path,created_at) \
             VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET filename=excluded.filename, content_type=excluded.content_type, storage_path=excluded.storage_path",
        )
        .bind(id)
        .bind(project_id)
        .bind(root_frame_id)
        .bind(filename)
        .bind(content_type)
        .bind(storage_path)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Artifacts for one conversation frame, newest first.
    pub async fn list_artifacts(&self, root_frame_id: &str) -> Result<Vec<(String, String, String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT id, filename, content_type, storage_path, created_at FROM artifacts \
             WHERE root_frame_id=? ORDER BY created_at DESC",
        )
        .bind(root_frame_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for row in rows {
            out.push((
                row.try_get("id")?,
                row.try_get("filename")?,
                row.try_get("content_type")?,
                row.try_get("storage_path")?,
                row.try_get("created_at")?,
            ));
        }
        Ok(out)
    }

    pub async fn get_artifact(&self, id: &str) -> Result<Option<(String, String, String, String)>> {
        let row = sqlx::query("SELECT filename, content_type, storage_path, root_frame_id FROM artifacts WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            (
                r.try_get("filename").unwrap_or_default(),
                r.try_get("content_type").unwrap_or_default(),
                r.try_get("storage_path").unwrap_or_default(),
                r.try_get("root_frame_id").unwrap_or_default(),
            )
        }))
    }
}

fn parse_role(s: &str) -> wisp_llm::Role {
    match s {
        "system" => wisp_llm::Role::System,
        "user" => wisp_llm::Role::User,
        "assistant" => wisp_llm::Role::Assistant,
        "tool" => wisp_llm::Role::Tool,
        _ => wisp_llm::Role::User,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn roundtrip() {
        let tmp = std::env::temp_dir().join(format!("wisp_store_test_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p1", "proj").await.unwrap();
        store.create_frame("f1", "p1", "OPERON", "test-model").await.unwrap();
        store.append_message("f1", 0, &Message::system("hi")).await.unwrap();
        store.append_message("f1", 1, &Message::user("hello")).await.unwrap();
        let msgs = store.load_messages("f1").await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].content.as_text(), "hello");
        let frames = store.list_root_frames("p1").await.unwrap();
        assert_eq!(frames.len(), 1);

        // list_sessions derives a title from the first user message and skips
        // frames with no user turn.
        store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
        store.append_message("f2", 0, &Message::system("only system")).await.unwrap();
        let sessions = store.list_sessions("p1").await.unwrap();
        assert_eq!(sessions.len(), 1, "f2 has no user turn, must be excluded");
        assert_eq!(sessions[0].0, "f1");
        assert_eq!(sessions[0].1, "hello");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn multi_turn_append() {
        // Mirrors the Tauri wiring: a frame is created once, then messages are
        // appended across turns with incrementing seq; load_messages returns
        // them all in order.
        let tmp = std::env::temp_dir().join(format!("wisp_store_multiturn_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();

        // Turn 1: system + user.
        store.append_message("f", 0, &Message::system("sys")).await.unwrap();
        store.append_message("f", 1, &Message::user("hi")).await.unwrap();
        let m1 = store.load_messages("f").await.unwrap();
        assert_eq!(m1.len(), 2);

        // Turn 2: assistant + tool result appended with seq 2,3.
        store.append_message("f", 2, &Message::assistant("hello")).await.unwrap();
        store.append_message("f", 3, &Message::tool("c1", "read", "ok")).await.unwrap();
        let m2 = store.load_messages("f").await.unwrap();
        assert_eq!(m2.len(), 4);
        assert_eq!(m2[0].content.as_text(), "sys");
        assert_eq!(m2[3].tool_name.as_deref(), Some("read"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn truncate_messages() {
        let tmp = std::env::temp_dir().join(format!("wisp_store_trunc_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store.append_message("f", 1, &Message::user("a")).await.unwrap();
        store.append_message("f", 2, &Message::assistant("b")).await.unwrap();
        store.append_message("f", 3, &Message::user("c")).await.unwrap();
        store.truncate_messages("f", 1).await.unwrap();
        let msgs = store.load_messages("f").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.as_text(), "a");
        let _ = std::fs::remove_file(&tmp);
    }
}
