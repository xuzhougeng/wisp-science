use super::{parse_role, session_display_title, RecentSessionDetail, SessionSearchResult, Store};
use anyhow::Result;
use sqlx::Row;
use wisp_llm::Message;

impl Store {
    pub async fn frame_project_id(&self, frame_id: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT project_id FROM frames WHERE id=?")
            .bind(frame_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|value| value.0))
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
    /// session-history sidebar. Returns `(frame_id, title, created_at, folder_id)`.
    pub async fn list_sessions(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, String, i64, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, f.created_at AS created_at, f.title AS custom_title, f.folder_id AS folder_id, \
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
            let folder_id: Option<String> = row.try_get("folder_id")?;
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let title = session_display_title(custom_title, first_user);
            out.push((id, title, created, folder_id));
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

    pub async fn list_folders(&self, project_id: &str) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT id, name, created_at FROM folders WHERE project_id=? ORDER BY created_at ASC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get("id")?,
                    r.try_get("name")?,
                    r.try_get("created_at")?,
                ))
            })
            .collect()
    }

    pub async fn create_folder(&self, id: &str, project_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Folder name cannot be empty");
        }
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO folders(id, project_id, name, created_at, updated_at) VALUES(?,?,?,?,?)",
        )
        .bind(id)
        .bind(project_id)
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_folder(&self, id: &str, project_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Folder name cannot be empty");
        }
        let now = chrono::Utc::now().timestamp();
        let n = sqlx::query("UPDATE folders SET name=?, updated_at=? WHERE id=? AND project_id=?")
            .bind(name)
            .bind(now)
            .bind(id)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        if n.rows_affected() == 0 {
            anyhow::bail!("Folder not found");
        }
        Ok(())
    }

    /// Delete a folder; sessions inside are kept (folder_id cleared).
    pub async fn delete_folder(&self, id: &str, project_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE frames SET folder_id=NULL WHERE folder_id=? AND project_id=?")
            .bind(id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        let n = sqlx::query("DELETE FROM folders WHERE id=? AND project_id=?")
            .bind(id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        if n.rows_affected() == 0 {
            anyhow::bail!("Folder not found");
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn move_session_to_folder(
        &self,
        frame_id: &str,
        project_id: &str,
        folder_id: Option<&str>,
    ) -> Result<()> {
        if let Some(fid) = folder_id {
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT id FROM folders WHERE id=? AND project_id=?")
                    .bind(fid)
                    .bind(project_id)
                    .fetch_optional(&self.pool)
                    .await?;
            if exists.is_none() {
                anyhow::bail!("Folder not found");
            }
        }
        let now = chrono::Utc::now().timestamp();
        let n = sqlx::query(
            "UPDATE frames SET folder_id=?, updated_at=? WHERE id=? AND project_id=? AND parent_frame_id=id",
        )
        .bind(folder_id)
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

    /// Persist an artifact and mint an immutable version for its current location.

    pub async fn search_sessions(
        &self,
        project_id: Option<&str>,
        query: &str,
        limit: i64,
        session_id: Option<&str>,
    ) -> Result<Vec<SessionSearchResult>> {
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT f.id AS id, f.project_id AS project_id, COALESCE(p.name,'') AS project_name, \
                    f.created_at AS created_at, COALESCE(f.title,'') AS custom_title, \
                    (SELECT content FROM messages m WHERE m.frame_id=f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                    (SELECT role FROM messages m WHERE m.frame_id=f.id ORDER BY m.seq DESC LIMIT 1) AS last_role, \
                    (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id=f.id) AS activity_at \
             FROM frames f JOIN projects p ON p.id=f.project_id \
             WHERE f.parent_frame_id=f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id=f.id AND mm.role='user') \
               AND (? IS NULL OR f.project_id=?) \
               AND (? IS NULL OR f.id=?) \
               AND (?='' OR lower(COALESCE(NULLIF(f.title,''), \
                    (SELECT content FROM messages m WHERE m.frame_id=f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1), '')) LIKE ?) \
             ORDER BY activity_at DESC, f.rowid DESC LIMIT ?",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(session_id)
        .bind(session_id)
        .bind(&q)
        .bind(format!("%{q}%"))
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(SessionSearchResult {
                    id: row.try_get("id")?,
                    project_id: row.try_get("project_id")?,
                    project_name: row.try_get("project_name")?,
                    title: session_display_title(
                        row.try_get::<Option<String>, _>("custom_title")?,
                        row.try_get::<Option<String>, _>("first_user")?,
                    ),
                    created_at: row.try_get("created_at")?,
                    activity_at: row.try_get("activity_at")?,
                    last_role: row.try_get("last_role")?,
                })
            })
            .collect()
    }

    pub async fn get_session_reference(&self, id: &str) -> Result<Option<SessionSearchResult>> {
        Ok(self
            .search_sessions(None, "", 1, Some(id))
            .await?
            .into_iter()
            .next())
    }
}
