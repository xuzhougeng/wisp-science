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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        // WAL journaling so a crash mid-turn can't corrupt the DB and committed
        // messages survive (pairs with incremental message persistence).
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        // Strip `--` line comments before splitting on `;` so semicolons inside
        // comments don't produce bogus statements.
        let stripped: String = MIGRATION_SQL
            .lines()
            .map(|l| match l.split_once("--") {
                Some((code, _)) => code.to_string(),
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        for stmt in stripped.split(';') {
            let s = stmt.trim();
            if s.is_empty() {
                continue;
            }
            sqlx::query(s).execute(pool).await?;
        }

        // Add projects.workspace_dir on DBs that predate it (fresh DBs already
        // have it via 0000_init.sql). pragma_table_info makes this idempotent
        // without a migration-version table.
        let has: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name='workspace_dir'",
        )
        .fetch_one(pool)
        .await?;
        if has.0 == 0 {
            sqlx::query("ALTER TABLE projects ADD COLUMN workspace_dir TEXT NOT NULL DEFAULT ''")
                .execute(pool)
                .await?;
        }
        let has_model_name: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='model_name'",
        )
        .fetch_one(pool)
        .await?;
        if has_model_name.0 == 0 {
            sqlx::query("ALTER TABLE messages ADD COLUMN model_name TEXT")
                .execute(pool)
                .await?;
        }
        let has_frame_title: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('frames') WHERE name='title'")
                .fetch_one(pool)
                .await?;
        if has_frame_title.0 == 0 {
            sqlx::query("ALTER TABLE frames ADD COLUMN title TEXT")
                .execute(pool)
                .await?;
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
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn create_project(&self, id: &str, name: &str, workspace_dir: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,'',?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, workspace_dir=excluded.workspace_dir, updated_at=excluded.updated_at",
        )
        .bind(id).bind(name).bind(workspace_dir).bind(now).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<(String, String)>> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Full editable metadata for the Project Settings modal: (name, description, workspace_dir).
    pub async fn get_project_meta(&self, id: &str) -> Result<Option<(String, String, String)>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(description,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    /// Update a project's user-visible name and description (touches updated_at).
    pub async fn update_project(&self, id: &str, name: &str, description: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE projects SET name=?, description=?, updated_at=? WHERE id=?")
            .bind(name)
            .bind(description)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// All projects, newest-updated first, each with its session count
    /// (root frames that have at least one user turn — matches `list_sessions`).
    pub async fn list_projects(
        &self,
    ) -> Result<Vec<(String, String, String, i64, i64, i64, String)>> {
        let rows = sqlx::query(
            "SELECT p.id AS id, COALESCE(p.name,'') AS name, COALESCE(p.workspace_dir,'') AS ws, \
                    p.created_at AS created_at, p.updated_at AS updated_at, \
                    COALESCE(p.description,'') AS description, \
                    (SELECT COUNT(*) FROM frames f WHERE f.project_id = p.id AND f.parent_frame_id = f.id \
                       AND EXISTS (SELECT 1 FROM messages m WHERE m.frame_id = f.id AND m.role='user')) AS sessions \
             FROM projects p ORDER BY p.updated_at DESC, p.rowid DESC",
        )
        .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for r in rows {
            out.push((
                r.try_get("id")?,
                r.try_get("name")?,
                r.try_get("ws")?,
                r.try_get("created_at")?,
                r.try_get("updated_at")?,
                r.try_get("sessions")?,
                r.try_get("description")?,
            ));
        }
        Ok(out)
    }

    /// Delete a project and everything under it. Explicit child deletes (SQLite
    /// FKs are OFF by default, so declared CASCADE would not fire). Filesystem
    /// is untouched — only DB rows.
    /// ponytail: explicit cascade of the 3 known child tables; switch to
    /// `PRAGMA foreign_keys=ON` if more child tables appear.
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM artifacts WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM frames WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projects WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Newest sessions across ALL projects, for the landing "Recent sessions" list.
    pub async fn list_recent_sessions(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i64)>> {
        Ok(self
            .list_recent_sessions_detail(limit)
            .await?
            .into_iter()
            .map(|r| (r.id, r.project_id, r.title, r.created_at))
            .collect())
    }

    /// Recent sessions with last-turn metadata for the projects dashboard.
    pub async fn list_recent_sessions_detail(
        &self,
        limit: i64,
    ) -> Result<Vec<RecentSessionDetail>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, f.project_id AS pid, f.created_at AS created_at, f.title AS custom_title, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                (SELECT role FROM messages m WHERE m.frame_id = f.id ORDER BY m.seq DESC LIMIT 1) AS last_role, \
                (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id = f.id) AS activity_at \
             FROM frames f \
             WHERE f.parent_frame_id = f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role='user') \
             ORDER BY activity_at DESC, f.rowid DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for row in rows {
            let id: String = row.try_get("id")?;
            let pid: String = row.try_get("pid")?;
            let created: i64 = row.try_get("created_at")?;
            let activity_at: i64 = row.try_get("activity_at")?;
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let last_role: Option<String> = row.try_get("last_role")?;
            let title = session_display_title(custom_title, first_user);
            out.push(RecentSessionDetail {
                id,
                project_id: pid,
                title,
                created_at: created,
                activity_at,
                last_role,
            });
        }
        Ok(out)
    }

    /// Last message role per saved session in a project (for dashboard counts).
    pub async fn list_session_last_roles(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, \
                (SELECT role FROM messages m WHERE m.frame_id = f.id ORDER BY m.seq DESC LIMIT 1) AS last_role \
             FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role='user')",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get("id")?, r.try_get("last_role")?)))
            .collect()
    }

    pub async fn create_frame(
        &self,
        id: &str,
        project_id: &str,
        agent_name: &str,
        model: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let sql = "INSERT INTO frames(id,parent_frame_id,root_frame_id,agent_name,status,project_id,model,input_tokens,output_tokens,created_at,updated_at,completed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,NULL)";
        sqlx::query(sql)
            .bind(id)
            .bind(id)
            .bind(id)
            .bind(agent_name)
            .bind("running")
            .bind(project_id)
            .bind(model)
            .bind(0i64)
            .bind(0i64)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn append_message(&self, frame_id: &str, seq: i64, msg: &Message) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let role = format!("{:?}", msg.role).to_ascii_lowercase();
        let content = serde_json::to_string(&msg.content)?;
        let tool_calls = if msg.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&msg.tool_calls)?)
        };
        sqlx::query("INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(id).bind(frame_id).bind(seq).bind(role).bind(content)
            .bind(tool_calls)
            .bind(msg.tool_call_id.as_deref())
            .bind(msg.tool_name.as_deref())
            .bind(msg.reasoning.as_deref())
            .bind(msg.ts)
            .bind(msg.model_name.as_deref())
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
        let rows = sqlx::query("SELECT role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name FROM messages WHERE frame_id=? ORDER BY seq ASC")
            .bind(frame_id)
            .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let role: String = row.try_get("role")?;
            let content_json: String = row.try_get("content")?;
            let content: wisp_llm::Content =
                serde_json::from_str(&content_json).unwrap_or(wisp_llm::Content::text(""));
            let tool_calls_json: Option<String> = row.try_get("tool_calls")?;
            let tool_calls: Vec<wisp_llm::ToolCall> = tool_calls_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let tool_call_id: Option<String> = row.try_get("tool_call_id")?;
            let tool_name: Option<String> = row.try_get("tool_name")?;
            let reasoning: Option<String> = row.try_get("reasoning")?;
            let ts: i64 = row.try_get("ts")?;
            let model_name: Option<String> = row.try_get("model_name")?;
            let role = parse_role(&role);
            out.push(Message {
                role,
                content,
                tool_calls,
                tool_call_id,
                tool_name,
                reasoning,
                ts,
                model_name,
            });
        }
        Ok(out)
    }

    /// Root frames that have at least one user turn, newest first, each with a
    /// title derived from its first user message. Used to populate the UI's
    /// session-history sidebar. Returns `(frame_id, title, created_at)`.
    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, f.created_at AS created_at, f.title AS custom_title, \
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
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let title = session_display_title(custom_title, first_user);
            out.push((id, title, created));
        }
        Ok(out)
    }

    /// Delete a saved conversation (root frame) and all of its messages/artifacts.
    pub async fn delete_session(&self, frame_id: &str, project_id: &str) -> Result<()> {
        let exists: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM frames WHERE id=? AND project_id=? AND parent_frame_id=id",
        )
        .bind(frame_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        if exists.is_none() {
            anyhow::bail!("Session not found");
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        )
        .bind(frame_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM artifacts WHERE root_frame_id=?")
            .bind(frame_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM frames WHERE root_frame_id=?")
            .bind(frame_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Set a custom sidebar title for a saved conversation.
    pub async fn rename_session(
        &self,
        frame_id: &str,
        project_id: &str,
        title: &str,
    ) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("Title cannot be empty");
        }
        let now = chrono::Utc::now().timestamp();
        let n = sqlx::query(
            "UPDATE frames SET title=?, updated_at=? WHERE id=? AND project_id=? AND parent_frame_id=id",
        )
        .bind(title)
        .bind(now)
        .bind(frame_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if n.rows_affected() == 0 {
            anyhow::bail!("Session not found");
        }
        Ok(())
    }

    pub async fn list_root_frames(&self, project_id: &str) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query("SELECT id, agent_name, created_at FROM frames WHERE project_id=? AND parent_frame_id=id ORDER BY created_at DESC")
            .bind(project_id)
            .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            out.push((
                row.try_get::<String, _>("id")?,
                row.try_get::<String, _>("agent_name")?,
                row.try_get::<i64, _>("created_at")?,
            ));
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
    pub async fn list_artifacts(
        &self,
        root_frame_id: &str,
    ) -> Result<Vec<(String, String, String, String, i64)>> {
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
        let row = sqlx::query(
            "SELECT filename, content_type, storage_path, root_frame_id FROM artifacts WHERE id=?",
        )
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

#[derive(Debug, Clone)]
pub struct RecentSessionDetail {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
}

fn session_display_title(custom_title: Option<String>, first_user: Option<String>) -> String {
    if let Some(t) = custom_title {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    first_user
        .and_then(|c| serde_json::from_str::<wisp_llm::Content>(&c).ok())
        .map(|c| c.as_text().chars().take(80).collect::<String>())
        .unwrap_or_default()
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
        let tmp =
            std::env::temp_dir().join(format!("wisp_store_test_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p1", "proj", "").await.unwrap();
        store
            .create_frame("f1", "p1", "OPERON", "test-model")
            .await
            .unwrap();
        store
            .append_message("f1", 0, &Message::system("hi"))
            .await
            .unwrap();
        store
            .append_message("f1", 1, &Message::user("hello"))
            .await
            .unwrap();
        let msgs = store.load_messages("f1").await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].content.as_text(), "hello");
        let frames = store.list_root_frames("p1").await.unwrap();
        assert_eq!(frames.len(), 1);

        // list_sessions derives a title from the first user message and skips
        // frames with no user turn.
        store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
        store
            .append_message("f2", 0, &Message::system("only system"))
            .await
            .unwrap();
        let sessions = store.list_sessions("p1").await.unwrap();
        assert_eq!(sessions.len(), 1, "f2 has no user turn, must be excluded");
        assert_eq!(sessions[0].0, "f1");
        assert_eq!(sessions[0].1, "hello");
        store
            .rename_session("f1", "p1", "Renamed chat")
            .await
            .unwrap();
        let sessions = store.list_sessions("p1").await.unwrap();
        assert_eq!(sessions[0].1, "Renamed chat");
        store.delete_session("f1", "p1").await.unwrap();
        assert!(store.list_sessions("p1").await.unwrap().is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn multi_turn_append() {
        // Mirrors the Tauri wiring: a frame is created once, then messages are
        // appended across turns with incrementing seq; load_messages returns
        // them all in order.
        let tmp = std::env::temp_dir().join(format!(
            "wisp_store_multiturn_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();

        // Turn 1: system + user.
        store
            .append_message("f", 0, &Message::system("sys"))
            .await
            .unwrap();
        store
            .append_message("f", 1, &Message::user("hi"))
            .await
            .unwrap();
        let m1 = store.load_messages("f").await.unwrap();
        assert_eq!(m1.len(), 2);

        // Turn 2: assistant + tool result appended with seq 2,3.
        store
            .append_message("f", 2, &Message::assistant("hello"))
            .await
            .unwrap();
        store
            .append_message("f", 3, &Message::tool("c1", "read", "ok"))
            .await
            .unwrap();
        let m2 = store.load_messages("f").await.unwrap();
        assert_eq!(m2.len(), 4);
        assert_eq!(m2[0].content.as_text(), "sys");
        assert_eq!(m2[3].tool_name.as_deref(), Some("read"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn truncate_messages() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_store_trunc_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .append_message("f", 1, &Message::user("a"))
            .await
            .unwrap();
        store
            .append_message("f", 2, &Message::assistant("b"))
            .await
            .unwrap();
        store
            .append_message("f", 3, &Message::user("c"))
            .await
            .unwrap();
        store.truncate_messages("f", 1).await.unwrap();
        let msgs = store.load_messages("f").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.as_text(), "a");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn project_crud_and_listing() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_store_proj_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();

        // create + get roundtrips workspace_dir
        store
            .create_project("a", "Alpha", "/tmp/alpha")
            .await
            .unwrap();
        store
            .create_project("b", "Beta", "/tmp/beta")
            .await
            .unwrap();
        assert_eq!(
            store.get_project("a").await.unwrap(),
            Some(("Alpha".into(), "/tmp/alpha".into()))
        );

        // one session under "a" (root frame with a user turn), none under "b"
        store.create_frame("f1", "a", "OPERON", "m").await.unwrap();
        store
            .append_message("f1", 1, &Message::user("hi"))
            .await
            .unwrap();

        let projs = store.list_projects().await.unwrap();
        assert_eq!(projs.len(), 2);
        // ordered by updated_at desc; "b" created last so it sorts first
        assert_eq!(projs[0].0, "b");
        let a = projs.iter().find(|p| p.0 == "a").unwrap();
        assert_eq!(a.5, 1, "project a has one session");
        let b = projs.iter().find(|p| p.0 == "b").unwrap();
        assert_eq!(b.5, 0, "project b has no sessions");

        // recent sessions span projects
        store.create_frame("f2", "b", "OPERON", "m").await.unwrap();
        store
            .append_message("f2", 1, &Message::user("yo"))
            .await
            .unwrap();
        let recent = store.list_recent_sessions(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent
            .iter()
            .any(|(_, pid, title, _)| pid == "a" && title == "hi"));

        // delete removes rows for "a" only, leaves "b"
        store.delete_project("a").await.unwrap();
        assert!(store.get_project("a").await.unwrap().is_none());
        assert!(store.load_messages("f1").await.unwrap().is_empty());
        assert!(store.get_project("b").await.unwrap().is_some());
        assert_eq!(store.load_messages("f2").await.unwrap().len(), 1);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn recent_sessions_detail_last_role() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_store_recent_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();

        store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
        store
            .append_message("f1", 1, &Message::user("q"))
            .await
            .unwrap();
        store
            .append_message("f1", 2, &Message::assistant("done"))
            .await
            .unwrap();

        store.create_frame("f2", "p", "OPERON", "m").await.unwrap();
        store
            .append_message("f2", 1, &Message::user("only user"))
            .await
            .unwrap();

        let details = store.list_recent_sessions_detail(10).await.unwrap();
        let f1 = details.iter().find(|d| d.id == "f1").unwrap();
        assert_eq!(f1.last_role.as_deref(), Some("assistant"));
        let f2 = details.iter().find(|d| d.id == "f2").unwrap();
        assert_eq!(f2.last_role.as_deref(), Some("user"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn recent_sessions_detail_respects_limit() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_store_recent_lim_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        for i in 0..7 {
            let fid = format!("f{i}");
            store.create_frame(&fid, "p", "OPERON", "m").await.unwrap();
            store
                .append_message(&fid, 1, &Message::user(&format!("msg {i}")))
                .await
                .unwrap();
        }
        let recent = store.list_recent_sessions_detail(5).await.unwrap();
        assert_eq!(recent.len(), 5);
        let _ = std::fs::remove_file(&tmp);
    }
}
