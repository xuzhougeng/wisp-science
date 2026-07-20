use super::{
    parse_role, session_display_title, MessageResourceLink, RecentSessionDetail,
    SessionSearchResult, Store,
};
use anyhow::Result;
use sqlx::{Row, Sqlite, Transaction};
use wisp_llm::Message;

/// One bounded, turn-aligned slice of a saved conversation.
pub struct SessionTranscriptPage {
    pub messages: Vec<(i64, Message)>,
    pub reviews: Vec<(i64, String)>,
    pub ui_events: Vec<String>,
    pub resources: Vec<MessageResourceLink>,
    pub next_before_seq: Option<i64>,
    pub user_offset: usize,
    pub latest_seq: i64,
}

/// Delete every database row owned by a conversation. Legacy databases do not
/// consistently enable SQLite foreign keys, so the cascade must be explicit.
/// Runs are project-level records and survive, but their stale frame reference
/// is cleared. Artifact files are also left untouched in the workspace.
async fn delete_session_rows(tx: &mut Transaction<'_, Sqlite>, frame_id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE runs SET frame_id=NULL \
         WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "DELETE FROM message_resource_links \
         WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "DELETE FROM research_edges WHERE source_id IN (\
            SELECT id FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
                SELECT id FROM artifacts WHERE root_frame_id=?\
            )\
         ) OR target_id IN (\
            SELECT id FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
                SELECT id FROM artifacts WHERE root_frame_id=?\
            )\
         )",
    )
    .bind(frame_id)
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
            SELECT id FROM artifacts WHERE root_frame_id=?\
         )",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM run_artifacts WHERE artifact_id IN (\
            SELECT id FROM artifacts WHERE root_frame_id=?\
         )",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifact_dependencies WHERE artifact_version_id IN (\
            SELECT av.id FROM artifact_versions av \
            JOIN artifacts a ON a.id=av.artifact_id WHERE a.root_frame_id=?\
         ) OR depends_on_version_id IN (\
            SELECT av.id FROM artifact_versions av \
            JOIN artifacts a ON a.id=av.artifact_id WHERE a.root_frame_id=?\
         )",
    )
    .bind(frame_id)
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifact_versions WHERE artifact_id IN (\
            SELECT id FROM artifacts WHERE root_frame_id=?\
         )",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM artifacts WHERE root_frame_id=?")
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;

    for statement in [
        "DELETE FROM session_execution_contexts WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM session_reviews WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM session_ui_events WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM proposed_plans WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM codex_turn_configs WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM acp_sessions WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM execution_log WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    ] {
        sqlx::query(statement)
            .bind(frame_id)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("DELETE FROM frames WHERE root_frame_id=?")
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Store {
    pub async fn list_project_frame_ids(&self, project_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM frames WHERE project_id=? ORDER BY id")
                .bind(project_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn frame_project_id(&self, frame_id: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT project_id FROM frames WHERE id=?")
            .bind(frame_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|value| value.0))
    }

    /// The root conversation that most recently accepted a user message,
    /// across every project. Assistant/tool messages do not move this pointer:
    /// callers use it as a deterministic cold-start fallback for cross-surface
    /// conversation routing.
    pub async fn last_user_message_session(&self) -> Result<Option<(String, String)>> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT m.frame_id, f.project_id \
             FROM messages m JOIN frames f ON f.id=m.frame_id \
             WHERE m.role='user' AND f.parent_frame_id=f.id \
             ORDER BY m.ts DESC, m.rowid DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
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
        let role = if msg.role == wisp_llm::Role::User
            && msg.tool_name.as_deref() == Some(super::AGENT_WORKFLOW_COMPLETION_TOOL)
        {
            "internal".into()
        } else {
            format!("{:?}", msg.role).to_ascii_lowercase()
        };
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

    pub async fn message_count(&self, frame_id: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Drop persisted turns after `keep` (seq is 1-based; keep=3 retains seq 1..=3).
    pub async fn truncate_messages(&self, frame_id: &str, keep: i64) -> Result<()> {
        sqlx::query("DELETE FROM message_resource_links WHERE frame_id=? AND message_seq>?")
            .bind(frame_id)
            .bind(keep)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM session_ui_events WHERE frame_id=? AND seq > COALESCE((\
             SELECT MAX(seq) FROM session_ui_events WHERE frame_id=? \
             AND json_extract(event_json,'$.kind')='MessageBoundary' \
             AND CAST(json_extract(event_json,'$.seq') AS INTEGER)<=?), 0)",
        )
        .bind(frame_id)
        .bind(frame_id)
        .bind(keep)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM session_reviews WHERE frame_id = ? AND message_seq > ?")
            .bind(frame_id)
            .bind(keep)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM messages WHERE frame_id = ? AND seq > ?")
            .bind(frame_id)
            .bind(keep)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Load all messages for a frame, ordered by sequence.
    pub async fn load_messages(&self, frame_id: &str) -> Result<Vec<Message>> {
        Ok(self
            .load_messages_with_seq(frame_id)
            .await?
            .into_iter()
            .map(|(_, message)| message)
            .collect())
    }

    /// Load all messages with their durable sequence numbers. Readers use the
    /// sequence as a stable evidence locator even when one large transcript is
    /// split across several model calls.
    pub async fn load_messages_with_seq(&self, frame_id: &str) -> Result<Vec<(i64, Message)>> {
        let rows = sqlx::query("SELECT seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name FROM messages WHERE frame_id=? ORDER BY seq ASC")
            .bind(frame_id)
            .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let seq: i64 = row.try_get("seq")?;
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
            out.push((
                seq,
                Message {
                    role,
                    content,
                    tool_calls,
                    tool_call_id,
                    tool_name,
                    reasoning,
                    ts,
                    model_name,
                },
            ));
        }
        Ok(out)
    }

    /// Load at most `turn_limit` complete user turns before `before_seq`.
    ///
    /// The slice starts at a user message (or the first saved message on the
    /// oldest page), so a tool call and its result are never split across pages.
    pub async fn load_session_transcript_page(
        &self,
        frame_id: &str,
        before_seq: Option<i64>,
        turn_limit: usize,
    ) -> Result<SessionTranscriptPage> {
        let limit = turn_limit.max(1);
        let user_rows = sqlx::query(
            "SELECT seq FROM messages WHERE frame_id=? AND role='user' \
             AND (? IS NULL OR seq < ?) ORDER BY seq DESC LIMIT ?",
        )
        .bind(frame_id)
        .bind(before_seq)
        .bind(before_seq)
        .bind((limit + 1) as i64)
        .fetch_all(&self.pool)
        .await?;
        let user_seqs = user_rows
            .into_iter()
            .map(|row| row.try_get::<i64, _>("seq"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = user_seqs.len() > limit;
        let selected = &user_seqs[..user_seqs.len().min(limit)];
        let oldest_available: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(seq) FROM messages WHERE frame_id=? AND (? IS NULL OR seq < ?)",
        )
        .bind(frame_id)
        .bind(before_seq)
        .bind(before_seq)
        .fetch_one(&self.pool)
        .await?;
        let start_seq = if has_more {
            *selected
                .last()
                .expect("a page with older turns is non-empty")
        } else {
            oldest_available.unwrap_or(0)
        };
        let next_before_seq = has_more.then_some(start_seq);

        let rows = sqlx::query(
            "SELECT seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name \
             FROM messages WHERE frame_id=? AND seq>=? AND (? IS NULL OR seq < ?) ORDER BY seq",
        )
        .bind(frame_id)
        .bind(start_seq)
        .bind(before_seq)
        .bind(before_seq)
        .fetch_all(&self.pool)
        .await?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i64 = row.try_get("seq")?;
            let role: String = row.try_get("role")?;
            let content_json: String = row.try_get("content")?;
            let content: wisp_llm::Content =
                serde_json::from_str(&content_json).unwrap_or(wisp_llm::Content::text(""));
            let tool_calls_json: Option<String> = row.try_get("tool_calls")?;
            messages.push((
                seq,
                Message {
                    role: parse_role(&role),
                    content,
                    tool_calls: tool_calls_json
                        .and_then(|value| serde_json::from_str(&value).ok())
                        .unwrap_or_default(),
                    tool_call_id: row.try_get("tool_call_id")?,
                    tool_name: row.try_get("tool_name")?,
                    reasoning: row.try_get("reasoning")?,
                    ts: row.try_get("ts")?,
                    model_name: row.try_get("model_name")?,
                },
            ));
        }

        let user_offset: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE frame_id=? AND role='user' AND seq < ?",
        )
        .bind(frame_id)
        .bind(start_seq)
        .fetch_one(&self.pool)
        .await?;
        let latest_seq: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?;

        let start_event_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=? \
             AND json_extract(event_json,'$.kind')='MessageBoundary' \
             AND CAST(json_extract(event_json,'$.seq') AS INTEGER) < ?",
        )
        .bind(frame_id)
        .bind(start_seq)
        .fetch_one(&self.pool)
        .await?;
        let end_event_seq = if let Some(before) = before_seq {
            sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=? \
                 AND json_extract(event_json,'$.kind')='MessageBoundary' \
                 AND CAST(json_extract(event_json,'$.seq') AS INTEGER) < ?",
            )
            .bind(frame_id)
            .bind(before)
            .fetch_one(&self.pool)
            .await?
        } else {
            i64::MAX
        };
        let event_rows = sqlx::query(
            "SELECT event_json FROM session_ui_events WHERE frame_id=? AND seq>? AND seq<=? \
             ORDER BY seq",
        )
        .bind(frame_id)
        .bind(start_event_seq)
        .bind(end_event_seq)
        .fetch_all(&self.pool)
        .await?;
        let ui_events = event_rows
            .into_iter()
            .map(|row| row.try_get("event_json").map_err(Into::into))
            .collect::<Result<Vec<_>>>()?;

        let review_rows = sqlx::query(
            "SELECT message_seq,report_json FROM session_reviews WHERE frame_id=? \
             AND message_seq>=? AND (? IS NULL OR message_seq < ?) \
             ORDER BY message_seq,created_at",
        )
        .bind(frame_id)
        .bind(start_seq)
        .bind(before_seq)
        .bind(before_seq)
        .fetch_all(&self.pool)
        .await?;
        let reviews = review_rows
            .into_iter()
            .map(|row| Ok((row.try_get("message_seq")?, row.try_get("report_json")?)))
            .collect::<Result<Vec<_>>>()?;
        let resources = self
            .list_message_resource_links(frame_id, start_seq, before_seq)
            .await?;

        Ok(SessionTranscriptPage {
            messages,
            reviews,
            ui_events,
            resources,
            next_before_seq,
            user_offset: user_offset as usize,
            latest_seq,
        })
    }

    pub async fn upsert_session_review(
        &self,
        frame_id: &str,
        id: &str,
        message_seq: i64,
        report_json: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO session_reviews(id,frame_id,message_seq,report_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET \
             report_json=excluded.report_json,updated_at=excluded.updated_at",
        )
        .bind(id)
        .bind(frame_id)
        .bind(message_seq)
        .bind(report_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_session_reviews(&self, frame_id: &str) -> Result<Vec<(i64, String)>> {
        let rows = sqlx::query(
            "SELECT message_seq,report_json FROM session_reviews \
             WHERE frame_id=? ORDER BY message_seq,created_at",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("message_seq")?, row.try_get("report_json")?)))
            .collect()
    }

    pub async fn append_session_ui_event(
        &self,
        frame_id: &str,
        seq: i64,
        event_json: &str,
    ) -> Result<()> {
        sqlx::query("INSERT INTO session_ui_events(frame_id,seq,event_json) VALUES(?,?,?)")
            .bind(frame_id)
            .bind(seq)
            .bind(event_json)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn load_session_ui_events(&self, frame_id: &str) -> Result<Vec<String>> {
        let rows =
            sqlx::query("SELECT event_json FROM session_ui_events WHERE frame_id=? ORDER BY seq")
                .bind(frame_id)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|row| row.try_get("event_json").map_err(Into::into))
            .collect()
    }

    pub async fn next_session_ui_event_seq(&self, frame_id: &str) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COALESCE(MAX(seq),0)+1 FROM session_ui_events WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    /// Root frames that have at least one user turn, newest first, each with a
    /// title derived from its first user message. Used to populate the UI's
    /// session-history sidebar. Returns `(frame_id, title, created_at, folder_id)`.
    pub async fn list_sessions(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, String, i64, Option<String>)>> {
        self.list_sessions_page(project_id, None, usize::MAX).await
    }

    /// One stable, newest-first page for the session-history sidebar. The
    /// cursor is the final `(created_at, frame_id)` pair from the previous page.
    pub async fn list_sessions_page(
        &self,
        project_id: &str,
        cursor: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<(String, String, i64, Option<String>)>> {
        let cursor_ts = cursor.map(|value| value.0);
        let cursor_id = cursor.map(|value| value.1);
        let rows = sqlx::query(
            "SELECT f.id AS id, f.created_at AS created_at, f.title AS custom_title, f.folder_id AS folder_id, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS first_user \
             FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role = 'user') \
               AND (? IS NULL OR f.created_at < ? OR (f.created_at = ? AND f.id < ?)) \
             ORDER BY f.created_at DESC, f.id DESC LIMIT ?",
        )
        .bind(project_id)
        .bind(cursor_ts)
        .bind(cursor_ts)
        .bind(cursor_ts)
        .bind(cursor_id)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
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
        delete_session_rows(&mut tx, frame_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Copy the user-visible transcript into another project. Workspace files,
    /// artifacts, runs, external-agent bindings, and provider turn IDs stay in
    /// the source project. The copy resumes as a fresh local conversation.
    pub async fn copy_session_to_project(
        &self,
        frame_id: &str,
        source_project_id: &str,
        target_project_id: &str,
        new_frame_id: &str,
    ) -> Result<()> {
        self.transfer_session_to_project(
            frame_id,
            source_project_id,
            target_project_id,
            new_frame_id,
            false,
        )
        .await
    }

    /// Move a transcript to another project atomically. Project workspace files
    /// remain on disk in the source workspace; only conversation-owned database
    /// rows are removed after the target transcript has been created.
    pub async fn move_session_to_project(
        &self,
        frame_id: &str,
        source_project_id: &str,
        target_project_id: &str,
        new_frame_id: &str,
    ) -> Result<()> {
        self.transfer_session_to_project(
            frame_id,
            source_project_id,
            target_project_id,
            new_frame_id,
            true,
        )
        .await
    }

    async fn transfer_session_to_project(
        &self,
        frame_id: &str,
        source_project_id: &str,
        target_project_id: &str,
        new_frame_id: &str,
        remove_source: bool,
    ) -> Result<()> {
        if source_project_id == target_project_id {
            anyhow::bail!("Source and target projects must be different");
        }
        if new_frame_id.trim().is_empty() {
            anyhow::bail!("New session id cannot be empty");
        }

        let mut tx = self.pool.begin().await?;
        let target_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
            .bind(target_project_id)
            .fetch_optional(&mut *tx)
            .await?;
        if target_exists.is_none() {
            anyhow::bail!("Target project not found");
        }

        let source = sqlx::query(
            "SELECT agent_name,status,model,input_tokens,output_tokens,completed_at,title \
             FROM frames WHERE id=? AND project_id=? AND parent_frame_id=id",
        )
        .bind(frame_id)
        .bind(source_project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO frames(\
                id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,\
                input_tokens,output_tokens,created_at,updated_at,completed_at,title\
             ) VALUES(?,?,?,?,?,?,NULL,?,?,?,?,?,?,?)",
        )
        .bind(new_frame_id)
        .bind(new_frame_id)
        .bind(new_frame_id)
        .bind(source.try_get::<String, _>("agent_name")?)
        .bind(source.try_get::<String, _>("status")?)
        .bind(target_project_id)
        .bind(source.try_get::<Option<String>, _>("model")?)
        .bind(source.try_get::<Option<i64>, _>("input_tokens")?)
        .bind(source.try_get::<Option<i64>, _>("output_tokens")?)
        .bind(now)
        .bind(now)
        .bind(source.try_get::<Option<i64>, _>("completed_at")?)
        .bind(source.try_get::<Option<String>, _>("title")?)
        .execute(&mut *tx)
        .await?;

        let messages = sqlx::query(
            "SELECT seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name \
             FROM messages WHERE frame_id=? ORDER BY seq",
        )
        .bind(frame_id)
        .fetch_all(&mut *tx)
        .await?;
        for message in messages {
            sqlx::query(
                "INSERT INTO messages(\
                    id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(new_frame_id)
            .bind(message.try_get::<i64, _>("seq")?)
            .bind(message.try_get::<String, _>("role")?)
            .bind(message.try_get::<Option<String>, _>("content")?)
            .bind(message.try_get::<Option<String>, _>("tool_calls")?)
            .bind(message.try_get::<Option<String>, _>("tool_call_id")?)
            .bind(message.try_get::<Option<String>, _>("tool_name")?)
            .bind(message.try_get::<Option<String>, _>("reasoning")?)
            .bind(message.try_get::<i64, _>("ts")?)
            .bind(message.try_get::<Option<String>, _>("model_name")?)
            .execute(&mut *tx)
            .await?;
        }

        let reviews = sqlx::query(
            "SELECT message_seq,report_json,created_at,updated_at \
             FROM session_reviews WHERE frame_id=? ORDER BY message_seq,created_at",
        )
        .bind(frame_id)
        .fetch_all(&mut *tx)
        .await?;
        for review in reviews {
            sqlx::query(
                "INSERT INTO session_reviews(\
                    id,frame_id,message_seq,report_json,created_at,updated_at\
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(new_frame_id)
            .bind(review.try_get::<i64, _>("message_seq")?)
            .bind(review.try_get::<String, _>("report_json")?)
            .bind(review.try_get::<i64, _>("created_at")?)
            .bind(review.try_get::<i64, _>("updated_at")?)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO session_ui_events(frame_id,seq,event_json) \
             SELECT ?,seq,json_set(event_json,'$.frame_id',?) \
             FROM session_ui_events WHERE frame_id=? ORDER BY seq",
        )
        .bind(new_frame_id)
        .bind(new_frame_id)
        .bind(frame_id)
        .execute(&mut *tx)
        .await?;

        if remove_source {
            delete_session_rows(&mut tx, frame_id).await?;
        }
        sqlx::query("UPDATE projects SET updated_at=? WHERE id IN (?,?)")
            .bind(now)
            .bind(source_project_id)
            .bind(target_project_id)
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
