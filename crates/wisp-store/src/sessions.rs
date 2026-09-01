use super::{
    parse_role, session_display_title, MessageResourceLink, RecentSessionDetail,
    SessionSearchResult, Store,
};
use anyhow::Result;
use chrono::{Datelike, Duration, Local};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::{HashMap, HashSet};
use wisp_llm::Message;

/// Sidebar, project-card count, and search (#888): a root frame is visible
/// once it has a user turn **or** an explicit title. Untitled empty drafts stay
/// hidden. Keep this in lockstep with every list/count/search query that
/// should match `list_sessions_page`.
pub(crate) const SESSION_IS_LISTABLE_SQL: &str = "(\
EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role = 'user') \
OR TRIM(COALESCE(f.title, '')) <> '')";

/// Recent, last-role, and resume: a conversation that has actually been used.
/// A named unused draft is listable but has nothing to rank or reopen.
pub(crate) const SESSION_HAS_USER_TURN_SQL: &str =
    "EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role = 'user')";

/// Token totals for one root session, folded from the persisted per-round
/// `Usage` transcript events (the `frames` token columns are never updated).
#[derive(serde::Serialize)]
pub struct SessionTokenUsage {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub input: i64,
    pub output: i64,
    pub reasoning: i64,
    pub cached: i64,
}

/// Token totals for one project workspace. Only sessions with persisted usage
/// events contribute to the totals and count.
#[derive(serde::Serialize)]
pub struct ProjectTokenUsage {
    pub project_id: String,
    pub name: String,
    pub workspace_dir: String,
    pub updated_at: i64,
    pub session_count: i64,
    pub input: i64,
    pub output: i64,
    pub reasoning: i64,
    pub cached: i64,
}

#[derive(serde::Serialize)]
pub struct SessionTokenUsagePage {
    pub items: Vec<SessionTokenUsage>,
    pub total: i64,
}

#[derive(serde::Serialize)]
pub struct TokenUsageDay {
    pub date: String,
    pub tokens: i64,
    pub future: bool,
}

#[derive(serde::Serialize)]
pub struct ModelTokenUsage {
    pub model: String,
    pub tokens: i64,
}

/// One ranked SKILL or MCP tool from persisted `ToolCall` transcript events.
#[derive(serde::Serialize)]
pub struct ToolCallUsage {
    /// `"skill"` for `use_skill`, `"mcp"` for `mcp:*` tools.
    pub kind: String,
    pub name: String,
    pub calls: i64,
}

/// One bounded, turn-aligned slice of a saved conversation.
pub struct SessionTranscriptPage {
    pub messages: Vec<(i64, Message)>,
    pub branch_merges: Vec<SessionBranchMergeCard>,
    pub reviews: Vec<(i64, String)>,
    pub ui_events: Vec<String>,
    pub resources: Vec<MessageResourceLink>,
    pub next_before_seq: Option<i64>,
    pub user_offset: usize,
    pub latest_seq: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionBranchMergeCard {
    pub summary_message_seq: i64,
    pub branch_session_id: String,
    pub branch_title: String,
    pub checkpoint_user_index: usize,
    pub checkpoint_kind: String,
    pub summary: String,
}

/// One immutable read boundary over the append-only visual transcript.
///
/// `through_event_seq` is the newest completed message boundary visible when
/// the snapshot was taken. Readers must not load events beyond it: the main
/// conversation may keep streaming while a secondary read-only answer runs.
pub struct SessionUiEventSnapshot {
    pub through_event_seq: i64,
    pub events: Vec<(i64, String)>,
}

/// One persisted visual-transcript event with its wall-clock stamp.
/// `created_at` is unix epoch milliseconds; `None` for rows written before
/// the column existed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionUiEventRecord {
    pub seq: i64,
    pub created_at: Option<i64>,
    pub event_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionBranchDeltaMessage {
    pub seq: i64,
    pub role: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionBranchLink {
    pub id: String,
    pub title: String,
    pub source_session_id: String,
    pub checkpoint_user_index: usize,
    pub checkpoint_kind: String,
    pub merged: bool,
    pub merge_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionBranchMergePreview {
    pub main_session_id: String,
    pub branch_session_id: String,
    pub branch_title: String,
    pub checkpoint_user_index: usize,
    pub checkpoint_kind: String,
    pub guard_hash: String,
    pub new_message_count: usize,
    pub messages: Vec<SessionBranchDeltaMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionBranchMerge {
    pub main_session_id: String,
    pub branch_session_id: String,
    pub summary_message_seq: i64,
}

#[derive(Clone, serde::Serialize)]
struct BranchMessageRow {
    seq: i64,
    role: String,
    content: Option<String>,
    tool_calls: Option<String>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    reasoning: Option<String>,
    ts: i64,
    model_name: Option<String>,
}

const BRANCH_DELTA_MESSAGE_CHARS: usize = 4_000;
const BRANCH_DELTA_MESSAGES: usize = 40;

async fn branch_message_rows(
    tx: &mut Transaction<'_, Sqlite>,
    frame_id: &str,
) -> Result<Vec<BranchMessageRow>> {
    let rows = sqlx::query(
        "SELECT seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name \
         FROM messages WHERE frame_id=? ORDER BY seq",
    )
    .bind(frame_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(BranchMessageRow {
                seq: row.try_get("seq")?,
                role: row.try_get("role")?,
                content: row.try_get("content")?,
                tool_calls: row.try_get("tool_calls")?,
                tool_call_id: row.try_get("tool_call_id")?,
                tool_name: row.try_get("tool_name")?,
                reasoning: row.try_get("reasoning")?,
                ts: row.try_get("ts")?,
                model_name: row.try_get("model_name")?,
            })
        })
        .collect()
}

fn clipped_branch_text(text: &str) -> String {
    let mut chars = text.chars();
    let clipped = chars
        .by_ref()
        .take(BRANCH_DELTA_MESSAGE_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}\n[… truncated …]")
    } else {
        clipped
    }
}

fn branch_delta_message(row: &BranchMessageRow) -> SessionBranchDeltaMessage {
    let mut text = row
        .content
        .as_deref()
        .and_then(|content| serde_json::from_str::<wisp_llm::Content>(content).ok())
        .map(|content| content.as_text())
        .unwrap_or_default();
    if text.trim().is_empty() {
        text = row
            .tool_name
            .as_deref()
            .map(|name| format!("[tool: {name}]"))
            .or_else(|| row.tool_calls.as_ref().map(|_| "[tool call]".into()))
            .unwrap_or_else(|| "[empty message]".into());
    }
    SessionBranchDeltaMessage {
        seq: row.seq,
        role: if row.role == "internal" {
            "system".into()
        } else {
            row.role.clone()
        },
        text: clipped_branch_text(text.trim()),
    }
}

fn branch_delta_messages(rows: &[BranchMessageRow]) -> Vec<SessionBranchDeltaMessage> {
    if rows.len() <= BRANCH_DELTA_MESSAGES {
        return rows.iter().map(branch_delta_message).collect();
    }
    let half = BRANCH_DELTA_MESSAGES / 2;
    let omitted = rows.len() - BRANCH_DELTA_MESSAGES;
    let mut messages = rows[..half]
        .iter()
        .map(branch_delta_message)
        .collect::<Vec<_>>();
    messages.push(SessionBranchDeltaMessage {
        seq: 0,
        role: "system".into(),
        text: format!("[… {omitted} messages omitted from summary preview …]"),
    });
    messages.extend(rows[rows.len() - half..].iter().map(branch_delta_message));
    messages
}

async fn session_branch_merge_snapshot(
    tx: &mut Transaction<'_, Sqlite>,
    branch_session_id: &str,
    project_id: &str,
) -> Result<(SessionBranchMergePreview, Vec<BranchMessageRow>)> {
    let row = sqlx::query(
        "SELECT f.branched_from,f.branch_point_user_index,f.branch_point_kind,f.title, \
         (SELECT content FROM messages m WHERE m.frame_id=f.id AND m.role='user' \
          ORDER BY m.seq LIMIT 1) AS first_user \
         FROM frames f WHERE f.id=? AND f.project_id=? AND f.parent_frame_id=f.id \
           AND f.exploration_id IS NULL AND f.branched_from IS NOT NULL \
           AND f.branch_point_user_index IS NOT NULL \
           AND f.branch_point_kind IN ('before_user','after_response') \
           AND NOT EXISTS(SELECT 1 FROM session_branch_merges merge WHERE merge.branch_frame_id=f.id)",
    )
    .bind(branch_session_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Conversation is not a mergeable branch"))?;
    let main_session_id: String = row.try_get("branched_from")?;
    let checkpoint_user_index = usize::try_from(row.try_get::<i64, _>("branch_point_user_index")?)?;
    let checkpoint_kind: String = row.try_get("branch_point_kind")?;
    let title = session_display_title(row.try_get("title")?, row.try_get("first_user")?);
    let messages = branch_message_rows(tx, branch_session_id).await?;
    let user_positions = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "user" && message.tool_name.is_none())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let delta_start = match checkpoint_kind.as_str() {
        "before_user" => user_positions
            .get(checkpoint_user_index)
            .copied()
            .unwrap_or(messages.len()),
        "after_response" => user_positions
            .get(checkpoint_user_index.saturating_add(1))
            .copied()
            .unwrap_or(messages.len()),
        _ => unreachable!(),
    };
    let delta = messages[delta_start..].to_vec();
    let guard_hash = hex::encode(Sha256::digest(serde_json::to_vec(&(
        branch_session_id,
        checkpoint_user_index,
        &checkpoint_kind,
        &delta,
    ))?));
    Ok((
        SessionBranchMergePreview {
            main_session_id,
            branch_session_id: branch_session_id.to_string(),
            branch_title: title.strip_prefix("Branch: ").unwrap_or(&title).to_string(),
            checkpoint_user_index,
            checkpoint_kind,
            guard_hash,
            new_message_count: delta.len(),
            messages: branch_delta_messages(&delta),
        },
        delta,
    ))
}

/// Maximum stdout characters returned for one tool activity group when a
/// transcript page is replayed. The Tauri/UI layer applies its byte ceiling as
/// a second guard; this database-side cap prevents legacy event logs from
/// being materialized in full before that guard can run.
pub(crate) const SESSION_UI_STDOUT_REPLAY_MAX_CHARS: usize = 64 * 1024;
const RECENT_TURN_PREVIEW_MAX_CHARS: usize = 20_000;
pub(crate) const RECENT_TURN_TOOL_PREVIEW_MAX_CHARS: usize = 4_000;

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
        "DELETE FROM turn_file_undo \
         WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "DELETE FROM research_edges WHERE source_id IN (\
            SELECT id FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
                SELECT artifact.id FROM artifacts artifact WHERE artifact.root_frame_id=? \
                AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                    ON input.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                    ON output.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                    ON binding.artifact_version_id=version.id WHERE version.artifact_id=artifact.id)\
            )\
         ) OR target_id IN (\
            SELECT id FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
                SELECT artifact.id FROM artifacts artifact WHERE artifact.root_frame_id=? \
                AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                    ON input.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                    ON output.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
                AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                    ON binding.artifact_version_id=version.id WHERE version.artifact_id=artifact.id)\
            )\
         )",
    )
    .bind(frame_id)
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM research_nodes WHERE kind='artifact' AND ref_id IN (\
            SELECT artifact.id FROM artifacts artifact WHERE artifact.root_frame_id=? \
            AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                ON input.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                ON output.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                ON binding.artifact_version_id=version.id WHERE version.artifact_id=artifact.id)\
         )",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifact_dependencies WHERE artifact_version_id IN (\
            SELECT av.id FROM artifact_versions av \
            JOIN artifacts a ON a.id=av.artifact_id WHERE a.root_frame_id=? \
            AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                ON input.artifact_version_id=version.id WHERE version.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                ON output.artifact_version_id=version.id WHERE version.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                ON binding.artifact_version_id=version.id WHERE version.artifact_id=a.id)\
         ) OR depends_on_version_id IN (\
            SELECT av.id FROM artifact_versions av \
            JOIN artifacts a ON a.id=av.artifact_id WHERE a.root_frame_id=? \
            AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                ON input.artifact_version_id=version.id WHERE version.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                ON output.artifact_version_id=version.id WHERE version.artifact_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                ON binding.artifact_version_id=version.id WHERE version.artifact_id=a.id)\
         )",
    )
    .bind(frame_id)
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifact_versions WHERE artifact_id IN (\
            SELECT artifact.id FROM artifacts artifact WHERE artifact.root_frame_id=? \
            AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
                ON input.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
                ON output.artifact_version_id=version.id WHERE version.artifact_id=artifact.id) \
            AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
                ON binding.artifact_version_id=version.id WHERE version.artifact_id=artifact.id)\
         )",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifacts WHERE root_frame_id=? \
         AND NOT EXISTS (SELECT 1 FROM run_artifacts link WHERE link.artifact_id=artifacts.id) \
         AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_inputs input \
             ON input.artifact_version_id=version.id WHERE version.artifact_id=artifacts.id) \
         AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN run_outputs output \
             ON output.artifact_version_id=version.id WHERE version.artifact_id=artifacts.id) \
         AND NOT EXISTS (SELECT 1 FROM artifact_versions version JOIN evidence_bindings binding \
             ON binding.artifact_version_id=version.id WHERE version.artifact_id=artifacts.id)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE global_memories SET source_frame_id=NULL \
         WHERE source_frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE schedules SET frame_id=NULL \
         WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE schedule_runs SET frame_id=NULL \
         WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
    )
    .bind(frame_id)
    .execute(&mut **tx)
    .await?;

    for statement in [
        "DELETE FROM agent_workflow_deliveries WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM session_branch_merges WHERE EXISTS (SELECT 1 FROM frames frame WHERE frame.root_frame_id=? AND (frame.id=session_branch_merges.source_frame_id OR frame.id=session_branch_merges.branch_frame_id))",
        "DELETE FROM ask_user_requests WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM session_imports WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
        "DELETE FROM codex_imports WHERE frame_id IN (SELECT id FROM frames WHERE root_frame_id=?)",
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
    // An Artifact promoted into Run lineage is project evidence, no longer
    // disposable Session state. Keep its root frame as an invisible ownership
    // tombstone because the legacy Artifact schema requires a frame FK. Session
    // pickers only show roots with a user message, all of which were removed.
    sqlx::query("DELETE FROM frames WHERE root_frame_id=? AND id<>?")
        .bind(frame_id)
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    let retained: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE root_frame_id=?)")
            .bind(frame_id)
            .fetch_one(&mut **tx)
            .await?;
    if retained {
        sqlx::query(
            "UPDATE frames SET status='deleted',folder_id=NULL,branched_from=NULL,pinned=0,\
             title=NULL WHERE id=?",
        )
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query("DELETE FROM frames WHERE id=?")
            .bind(frame_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

impl Store {
    pub async fn session_has_conversation_branches(
        &self,
        frame_id: &str,
        project_id: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM frames WHERE project_id=? AND parent_frame_id=id \
             AND exploration_id IS NULL AND branched_from=?)",
        )
        .bind(project_id)
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?)
    }

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
             WHERE m.role='user' AND f.parent_frame_id=f.id AND f.exploration_id IS NULL \
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
    ///
    /// Deliberately requires a user turn, unlike `list_sessions_page` (#888):
    /// a named-but-unused draft has no activity to rank as "recent".
    pub async fn list_recent_sessions_detail(
        &self,
        limit: i64,
    ) -> Result<Vec<RecentSessionDetail>> {
        let sql = format!(
            "SELECT f.id AS id, f.project_id AS pid, f.created_at AS created_at, f.title AS custom_title, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                (SELECT role FROM messages m WHERE m.frame_id = f.id ORDER BY m.seq DESC LIMIT 1) AS last_role, \
                (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id = f.id) AS activity_at, \
                (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id = f.id) > f.seen_at AS unseen \
             FROM frames f \
             WHERE f.parent_frame_id = f.id \
               AND f.exploration_id IS NULL \
               AND f.project_id NOT LIKE 'scratch:%' \
               AND {used} ORDER BY activity_at DESC, f.rowid DESC LIMIT ?",
            used = SESSION_HAS_USER_TURN_SQL,
        );
        let rows = sqlx::query(&sql).bind(limit).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let id: String = row.try_get("id")?;
            let pid: String = row.try_get("pid")?;
            let created: i64 = row.try_get("created_at")?;
            let activity_at: i64 = row.try_get("activity_at")?;
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let last_role: Option<String> = row.try_get("last_role")?;
            let unseen: bool = row.try_get("unseen")?;
            let title = session_display_title(custom_title, first_user);
            out.push(RecentSessionDetail {
                id,
                project_id: pid,
                title,
                created_at: created,
                activity_at,
                last_role,
                unseen,
            });
        }
        Ok(out)
    }

    /// Last message role and unseen flag per saved session in a project (for
    /// dashboard counts).
    ///
    /// Deliberately requires a user turn, unlike `list_sessions_page` (#888):
    /// a message-less draft has no last role, and `last_role_needs_you(None)`
    /// is false anyway, so including it could only add noise.
    pub async fn list_session_last_roles(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, Option<String>, bool)>> {
        let sql = format!(
            "SELECT f.id AS id, \
                (SELECT role FROM messages m WHERE m.frame_id = f.id ORDER BY m.seq DESC LIMIT 1) AS last_role, \
                (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id = f.id) > f.seen_at AS unseen \
             FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id \
               AND {used}",
            used = SESSION_HAS_USER_TURN_SQL,
        );
        let rows = sqlx::query(&sql)
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.try_get("id")?,
                    r.try_get("last_role")?,
                    r.try_get("unseen")?,
                ))
            })
            .collect()
    }

    /// Record that the user has viewed this session's current transcript:
    /// snapshot `seen_at` to the latest activity so status checks can treat
    /// anything newer as unseen. Unit-agnostic — no wall clock involved.
    pub async fn mark_frame_seen(&self, id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE frames SET seen_at = \
                (SELECT COALESCE(MAX(ts), frames.updated_at) FROM messages m WHERE m.frame_id = frames.id) \
             WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    pub async fn create_child_frame(
        &self,
        id: &str,
        parent_frame_id: &str,
        project_id: &str,
        agent_name: &str,
        model: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let inserted = sqlx::query(
            "INSERT INTO frames(\
                id,parent_frame_id,root_frame_id,agent_name,status,project_id,model,reasoning_effort,service_tier,exploration_id,\
                input_tokens,output_tokens,created_at,updated_at,completed_at\
             ) SELECT ?,id,COALESCE(root_frame_id,id),?,'running',project_id,?,reasoning_effort,service_tier,exploration_id,0,0,?,?,NULL \
             FROM frames WHERE id=? AND project_id=?",
        )
        .bind(id)
        .bind(agent_name)
        .bind(model)
        .bind(now)
        .bind(now)
        .bind(parent_frame_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() != 1 {
            anyhow::bail!("Parent conversation not found");
        }
        Ok(())
    }

    pub async fn root_frame_id(&self, frame_id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT COALESCE(root_frame_id, id) FROM frames WHERE id=?",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    pub async fn frame_model(&self, frame_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar::<_, Option<String>>("SELECT model FROM frames WHERE id=?")
                .bind(frame_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// Per-conversation reasoning-effort override. `None` inherits the bound
    /// model profile; an empty string explicitly requests the provider default.
    pub async fn frame_reasoning_effort(&self, frame_id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT reasoning_effort FROM frames WHERE id=?",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// Per-conversation service-tier override. `None` inherits the bound
    /// model profile; `Some("")` explicitly selects provider default.
    pub async fn frame_service_tier(&self, frame_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar::<_, Option<String>>("SELECT service_tier FROM frames WHERE id=?")
                .bind(frame_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// Overwrite a frame's created/updated timestamps. Used by importers so
    /// external conversations keep their original chronology in the sidebar.
    pub async fn set_frame_timestamps(
        &self,
        frame_id: &str,
        created_at: i64,
        updated_at: i64,
    ) -> Result<()> {
        sqlx::query("UPDATE frames SET created_at=?,updated_at=? WHERE id=?")
            .bind(created_at)
            .bind(updated_at)
            .bind(frame_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_frame_model(
        &self,
        frame_id: &str,
        project_id: &str,
        model: &str,
    ) -> Result<()> {
        let updated =
            sqlx::query("UPDATE frames SET model=?,updated_at=? WHERE id=? AND project_id=?")
                .bind(model)
                .bind(chrono::Utc::now().timestamp())
                .bind(frame_id)
                .bind(project_id)
                .execute(&self.pool)
                .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Session not found");
        }
        Ok(())
    }

    pub async fn set_frame_reasoning_effort(
        &self,
        frame_id: &str,
        project_id: &str,
        reasoning_effort: Option<&str>,
    ) -> Result<()> {
        let updated = sqlx::query(
            "UPDATE frames SET reasoning_effort=?,updated_at=? WHERE id=? AND project_id=?",
        )
        .bind(reasoning_effort)
        .bind(chrono::Utc::now().timestamp())
        .bind(frame_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Session not found");
        }
        Ok(())
    }

    pub async fn set_frame_service_tier(
        &self,
        frame_id: &str,
        project_id: &str,
        service_tier: Option<&str>,
    ) -> Result<()> {
        let updated = sqlx::query(
            "UPDATE frames SET service_tier=?,updated_at=? WHERE id=? AND project_id=?",
        )
        .bind(service_tier)
        .bind(chrono::Utc::now().timestamp())
        .bind(frame_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Session not found");
        }
        Ok(())
    }

    pub async fn append_message(&self, frame_id: &str, seq: i64, msg: &Message) -> Result<()> {
        insert_message_row(&self.pool, frame_id, seq, msg).await
    }

    /// Replace a frame's model context wholesale (user-triggered /compact and
    /// automatic compaction). Only the `messages` rows are rewritten — the
    /// session_ui_events visual transcript keeps the full history on purpose.
    /// Resource links and turn undo anchor to message seqs, which a rewrite
    /// invalidates, so they are dropped too. Remapping `turn_file_undo` onto
    /// the rewritten seqs would need a stable turn/message id (known #973
    /// limitation); do not expand that here. Deletes and inserts share one
    /// write transaction: an interruption mid-replace leaves the previous
    /// transcript fully intact instead of an emptied or half-written frame.
    pub async fn replace_messages(&self, frame_id: &str, msgs: &[Message]) -> Result<()> {
        let mut tx = self.begin_write().await?;
        replace_message_rows(&mut tx, frame_id, msgs).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Rewrite a frame's persisted system prompt (the first `system` message)
    /// in place, e.g. to reload AGENTS.md / WISP.md into a long-lived session.
    /// Unlike `replace_messages`, other messages and resource links are
    /// untouched. Returns false when the frame has no system message.
    pub async fn replace_system_message(&self, frame_id: &str, msg: &Message) -> Result<bool> {
        let content = serde_json::to_string(&msg.content)?;
        let updated = sqlx::query(
            "UPDATE messages SET content=? WHERE frame_id=? AND role='system' \
             AND seq=(SELECT MIN(seq) FROM messages WHERE frame_id=? AND role='system')",
        )
        .bind(content)
        .bind(frame_id)
        .bind(frame_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Batch-load each frame's persisted system prompt content (JSON), keyed
    /// by frame id. Frames without a system message are absent from the map.
    pub async fn load_system_messages(
        &self,
        frame_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        if frame_ids.is_empty() {
            return Ok(map);
        }
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "SELECT frame_id, content FROM messages m WHERE role='system' \
             AND seq=(SELECT MIN(seq) FROM messages WHERE frame_id=m.frame_id AND role='system') \
             AND frame_id IN (",
        );
        let mut separated = qb.separated(", ");
        for id in frame_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let rows: Vec<(String, String)> = qb
            .build_query_as::<(String, String)>()
            .fetch_all(&self.pool)
            .await?;
        for (frame_id, content) in rows {
            map.insert(frame_id, content);
        }
        Ok(map)
    }

    pub async fn message_count(&self, frame_id: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Durable seq cursor for a frame: `COALESCE(MAX(seq), 0)`.
    ///
    /// This is the source of truth for `last_seq` recovery. Do not use
    /// `messages.len()` or `COUNT(*)` — gaps make those diverge from `MAX(seq)`.
    pub async fn max_message_seq(&self, frame_id: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Drop persisted turns after `keep` (seq is 1-based; keep=3 retains seq 1..=3).
    pub async fn truncate_messages(&self, frame_id: &str, keep: i64) -> Result<()> {
        let mut tx = self.begin_write().await?;
        reconcile_session_branches_after_truncate(&mut tx, frame_id, keep).await?;
        truncate_message_rows(&mut tx, frame_id, keep).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn truncate_message_rows(
        tx: &mut Transaction<'_, Sqlite>,
        frame_id: &str,
        keep: i64,
    ) -> Result<()> {
        truncate_message_rows(tx, frame_id, keep).await
    }

    #[cfg(test)]
    pub(crate) async fn replace_message_rows_for_test(
        tx: &mut Transaction<'_, Sqlite>,
        frame_id: &str,
        msgs: &[Message],
    ) -> Result<()> {
        replace_message_rows(tx, frame_id, msgs).await
    }
}

pub(crate) async fn insert_message_row<'e, E>(
    executor: E,
    frame_id: &str,
    seq: i64,
    msg: &Message,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
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
        .execute(executor).await?;
    Ok(())
}

async fn replace_message_rows(
    tx: &mut Transaction<'_, Sqlite>,
    frame_id: &str,
    msgs: &[Message],
) -> Result<()> {
    sqlx::query("DELETE FROM message_resource_links WHERE frame_id=?")
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    // #973/#978: undo rows key off pre-compaction seqs. Mapping them onto the
    // rewritten 1..n seqs needs a stable turn/message id; wipe instead.
    sqlx::query("DELETE FROM turn_file_undo WHERE frame_id=?")
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM messages WHERE frame_id=?")
        .bind(frame_id)
        .execute(&mut **tx)
        .await?;
    for (i, msg) in msgs.iter().enumerate() {
        insert_message_row(&mut **tx, frame_id, (i + 1) as i64, msg).await?;
    }
    Ok(())
}

/// Reconcile branch provenance before removing a mainline suffix.
///
/// A merge summary is persisted at the real mainline tail even though its UI
/// card is projected back to the branch checkpoint. Removing that tail revokes
/// the merge. A checkpoint itself remains valid only while its concrete anchor
/// is retained: `before_user` needs that user message, while `after_response`
/// also needs a retained assistant reply for the turn.
pub(crate) async fn reconcile_session_branches_after_truncate(
    tx: &mut Transaction<'_, Sqlite>,
    frame_id: &str,
    keep: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM session_branch_merges WHERE source_frame_id=? AND summary_message_seq>?",
    )
    .bind(frame_id)
    .bind(keep)
    .execute(&mut **tx)
    .await?;

    let retained = sqlx::query(
        "SELECT role,content,tool_name FROM messages WHERE frame_id=? AND seq<=? ORDER BY seq",
    )
    .bind(frame_id)
    .bind(keep)
    .fetch_all(&mut **tx)
    .await?;
    let mut retained_turns = Vec::<bool>::new();
    for row in retained {
        let role: String = row.try_get("role")?;
        let tool_name: Option<String> = row.try_get("tool_name")?;
        if role == "user"
            && tool_name.as_deref() != Some(crate::AGENT_WORKFLOW_COMPLETION_TOOL)
            && row
                .try_get::<Option<String>, _>("content")?
                .and_then(|content| serde_json::from_str::<wisp_llm::Content>(&content).ok())
                .is_some_and(|content| !content.as_text().trim().is_empty())
        {
            retained_turns.push(false);
        } else if role == "assistant" {
            if let Some(has_reply) = retained_turns.last_mut() {
                *has_reply = true;
            }
        }
    }

    let branches = sqlx::query(
        "SELECT id,branch_point_user_index,branch_point_kind FROM frames WHERE branched_from=? \
         AND branch_point_user_index IS NOT NULL AND branch_point_kind IN ('before_user','after_response')",
    )
    .bind(frame_id)
    .fetch_all(&mut **tx)
    .await?;
    for branch in branches {
        let id: String = branch.try_get("id")?;
        let user_index = usize::try_from(branch.try_get::<i64, _>("branch_point_user_index")?)?;
        let kind: String = branch.try_get("branch_point_kind")?;
        let checkpoint_retained = match kind.as_str() {
            "before_user" => user_index < retained_turns.len(),
            "after_response" => retained_turns.get(user_index).copied() == Some(true),
            _ => false,
        };
        if !checkpoint_retained {
            sqlx::query("UPDATE frames SET branch_point_kind='orphaned' WHERE id=?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

async fn truncate_message_rows(
    tx: &mut Transaction<'_, Sqlite>,
    frame_id: &str,
    keep: i64,
) -> Result<()> {
    sqlx::query("DELETE FROM message_resource_links WHERE frame_id=? AND message_seq>?")
        .bind(frame_id)
        .bind(keep)
        .execute(&mut **tx)
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
    .execute(&mut **tx)
    .await?;
    let retained_turns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM session_ui_events WHERE frame_id=? \
         AND json_extract(event_json,'$.kind')='User'",
    )
    .bind(frame_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM project_state_revisions WHERE frame_id=? AND turn_index>=?")
        .bind(frame_id)
        .bind(retained_turns)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM session_reviews WHERE frame_id = ? AND message_seq > ?")
        .bind(frame_id)
        .bind(keep)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM messages WHERE frame_id = ? AND seq > ?")
        .bind(frame_id)
        .bind(keep)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM turn_file_undo WHERE frame_id = ? AND user_message_seq > ?")
        .bind(frame_id)
        .bind(keep)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Store {
    /// Load all messages for a frame, ordered by sequence.
    pub async fn load_messages(&self, frame_id: &str) -> Result<Vec<Message>> {
        Ok(self
            .load_messages_with_seq(frame_id)
            .await?
            .into_iter()
            .map(|(_, message)| message)
            .collect())
    }

    /// Load only the newest complete user turns as bounded text previews,
    /// without UI events, reviews, resources, reasoning, image data, or full
    /// tool-call arguments. This is for secondary model calls (for example
    /// follow-up suggestions), not transcript rendering or agent recovery.
    pub async fn load_recent_turn_preview_messages(
        &self,
        frame_id: &str,
        turn_limit: usize,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            "WITH recent_user_turns AS (\
                 SELECT seq FROM messages \
                 WHERE frame_id=? AND role='user' AND tool_name IS NULL \
                 ORDER BY seq DESC LIMIT ?\
             ), start_seq AS (SELECT MIN(seq) AS seq FROM recent_user_turns) \
             SELECT seq,role,json_quote(substr(\
                 CASE json_type(content) \
                     WHEN 'text' THEN json_extract(content,'$') \
                     WHEN 'array' THEN COALESCE((\
                         SELECT group_concat(json_extract(part.value,'$.text'),'\n') \
                         FROM json_each(content) AS part \
                         WHERE json_extract(part.value,'$.type')='text'\
                     ),'') ELSE '' END,1,\
                 CASE WHEN role='tool' AND COALESCE(tool_name,'') NOT IN \
                     ('attempt_completion','propose_plan','ask_user') THEN ? ELSE ? END\
             )) AS content,NULL AS tool_calls,tool_call_id,tool_name,NULL AS reasoning,ts,model_name \
             FROM messages WHERE frame_id=? \
             AND seq>=COALESCE((SELECT seq FROM start_seq), 0) ORDER BY seq ASC",
        )
        .bind(frame_id)
        .bind(turn_limit.max(1) as i64)
        .bind(RECENT_TURN_TOOL_PREVIEW_MAX_CHARS as i64)
        .bind(RECENT_TURN_PREVIEW_MAX_CHARS as i64)
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let role: String = row.try_get("role")?;
            let content_json: String = row.try_get("content")?;
            let content: wisp_llm::Content =
                serde_json::from_str(&content_json).unwrap_or(wisp_llm::Content::text(""));
            let tool_calls_json: Option<String> = row.try_get("tool_calls")?;
            messages.push(Message {
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
            });
        }
        Ok(messages)
    }

    /// Load the durable user-authored turns used by the conversation outline,
    /// including when each question was sent and its final assistant reply.
    pub async fn load_session_user_messages(
        &self,
        frame_id: &str,
    ) -> Result<Vec<(i64, String, i64, Option<i64>)>> {
        let rows = sqlx::query(
            "SELECT seq,role,content,ts FROM messages \
             WHERE frame_id=? AND role IN ('user','assistant') ORDER BY seq",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i64 = row.try_get("seq")?;
            let role: String = row.try_get("role")?;
            let ts: i64 = row.try_get("ts")?;
            if role == "assistant" {
                if ts > 0 {
                    if let Some((_, _, _, response_at)) = messages.last_mut() {
                        *response_at = Some(ts);
                    }
                }
                continue;
            }
            let content_json: String = row.try_get("content")?;
            let content: wisp_llm::Content =
                serde_json::from_str(&content_json).unwrap_or(wisp_llm::Content::text(""));
            messages.push((seq, content.as_text(), ts, None));
        }
        Ok(messages)
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
        let latest_seq = self.max_message_seq(frame_id).await?;

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
            "WITH page_events AS (\
                 SELECT seq,event_json,json_extract(event_json,'$.kind') AS kind \
                 FROM session_ui_events WHERE frame_id=? AND seq>? AND seq<=? \
                 AND json_extract(event_json,'$.kind')<>'ToolPresentation'\
             ), grouped_events AS (\
                 SELECT *,SUM(CASE WHEN kind IN ('ToolCall','User') THEN 1 ELSE 0 END) \
                     OVER (ORDER BY seq) AS stdout_group \
                 FROM page_events\
             ), budgeted_events AS (\
                 SELECT *,COALESCE(SUM(CASE WHEN kind='Stdout' \
                         THEN length(json_extract(event_json,'$.chunk')) ELSE 0 END) \
                     OVER (PARTITION BY stdout_group ORDER BY seq \
                           ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) AS stdout_before \
                 FROM grouped_events\
             ) \
             SELECT CASE WHEN kind='Stdout' THEN json_set(\
                         event_json,'$.chunk',substr(json_extract(event_json,'$.chunk'),1,? - stdout_before)\
                     ) ELSE event_json END AS event_json \
             FROM budgeted_events \
             WHERE kind<>'Stdout' OR stdout_before<? ORDER BY seq",
        )
        .bind(frame_id)
        .bind(start_event_seq)
        .bind(end_event_seq)
        .bind(SESSION_UI_STDOUT_REPLAY_MAX_CHARS as i64)
        .bind(SESSION_UI_STDOUT_REPLAY_MAX_CHARS as i64)
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
        let merge_rows = sqlx::query(
            "SELECT merge.summary_message_seq,merge.branch_frame_id,merge.checkpoint_user_index, \
             merge.checkpoint_kind,message.content AS summary_content,branch.title, \
             (SELECT content FROM messages m WHERE m.frame_id=branch.id AND m.role='user' \
              ORDER BY m.seq LIMIT 1) AS first_user \
             FROM session_branch_merges merge JOIN frames branch ON branch.id=merge.branch_frame_id \
             JOIN messages message ON message.frame_id=merge.source_frame_id \
               AND message.seq=merge.summary_message_seq \
             WHERE merge.source_frame_id=? AND merge.summary_message_seq>=? \
               AND (? IS NULL OR merge.summary_message_seq < ?) \
             ORDER BY merge.summary_message_seq",
        )
        .bind(frame_id)
        .bind(start_seq)
        .bind(before_seq)
        .bind(before_seq)
        .fetch_all(&self.pool)
        .await?;
        let branch_merges = merge_rows
            .into_iter()
            .map(|row| {
                let title =
                    session_display_title(row.try_get("title")?, row.try_get("first_user")?);
                Ok(SessionBranchMergeCard {
                    summary_message_seq: row.try_get("summary_message_seq")?,
                    branch_session_id: row.try_get("branch_frame_id")?,
                    branch_title: title.strip_prefix("Branch: ").unwrap_or(&title).to_string(),
                    checkpoint_user_index: usize::try_from(
                        row.try_get::<i64, _>("checkpoint_user_index")?,
                    )?,
                    checkpoint_kind: row.try_get("checkpoint_kind")?,
                    summary: row
                        .try_get::<String, _>("summary_content")
                        .ok()
                        .and_then(|content| {
                            serde_json::from_str::<wisp_llm::Content>(&content).ok()
                        })
                        .map(|content| content.as_text())
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(SessionTranscriptPage {
            messages,
            branch_merges,
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

    pub async fn append_session_ui_event(
        &self,
        frame_id: &str,
        seq: i64,
        event_json: &str,
    ) -> Result<()> {
        // Unix epoch milliseconds; ui events join against second-granularity
        // message timestamps, so the trajectory view needs finer resolution.
        let created_at = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO session_ui_events(frame_id,seq,event_json,created_at) VALUES(?,?,?,?)",
        )
        .bind(frame_id)
        .bind(seq)
        .bind(event_json)
        .bind(created_at)
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

    /// Load the persisted visual transcript with per-event wall-clock stamps
    /// (unix epoch milliseconds; `None` for rows written before the column
    /// existed). Used by the trajectory view, which reconstructs timing.
    pub async fn load_session_ui_events_timed(
        &self,
        frame_id: &str,
    ) -> Result<Vec<SessionUiEventRecord>> {
        let rows = sqlx::query(
            "SELECT seq,created_at,event_json FROM session_ui_events \
             WHERE frame_id=? ORDER BY seq",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(SessionUiEventRecord {
                    seq: row.try_get("seq")?,
                    created_at: row.try_get("created_at")?,
                    event_json: row.try_get("event_json")?,
                })
            })
            .collect()
    }

    /// Freeze and load the complete visual transcript through its newest
    /// completed model-message boundary. Unlike `messages`, this event log is
    /// not rewritten by context compaction, so it remains suitable for
    /// evidence retrieval over the conversation the user can still see.
    pub async fn load_session_ui_event_snapshot(
        &self,
        frame_id: &str,
    ) -> Result<SessionUiEventSnapshot> {
        let through_event_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=? \
             AND json_extract(event_json,'$.kind')='MessageBoundary'",
        )
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT seq,event_json FROM session_ui_events \
             WHERE frame_id=? AND seq<=? ORDER BY seq",
        )
        .bind(frame_id)
        .bind(through_event_seq)
        .fetch_all(&self.pool)
        .await?;
        let events = rows
            .into_iter()
            .map(|row| Ok((row.try_get("seq")?, row.try_get("event_json")?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(SessionUiEventSnapshot {
            through_event_seq,
            events,
        })
    }

    pub async fn load_latest_session_ui_event(
        &self,
        frame_id: &str,
        kind: &str,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT event_json FROM session_ui_events WHERE frame_id=? \
             AND json_extract(event_json,'$.kind')=? ORDER BY seq DESC LIMIT 1",
        )
        .bind(frame_id)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn next_session_ui_event_seq(&self, frame_id: &str) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COALESCE(MAX(seq),0)+1 FROM session_ui_events WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    /// Root frames the sidebar should show, most recently active first, each
    /// with a title from the custom name or the first user message. Untitled
    /// empty drafts stay hidden; a named unused draft is included (#888).
    /// Returns `(frame_id, title, activity_at, folder_id, branched_from)`.
    pub async fn list_sessions(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, String, i64, Option<String>, Option<String>)>> {
        self.list_sessions_page(project_id, None, usize::MAX).await
    }

    /// One stable, most-recently-active-first page for the session-history
    /// sidebar. The cursor is the final `(activity_at, frame_id)` pair from the
    /// previous page.
    pub async fn list_sessions_page(
        &self,
        project_id: &str,
        cursor: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<(String, String, i64, Option<String>, Option<String>)>> {
        let cursor_ts = cursor.map(|value| value.0);
        let cursor_id = cursor.map(|value| value.1);
        let sql = format!(
            "SELECT * FROM ( \
                SELECT f.id AS id, \
                    COALESCE((SELECT MAX(NULLIF(m.ts, 0)) FROM messages m WHERE m.frame_id = f.id), f.updated_at) AS activity_at, \
                    f.title AS custom_title, f.folder_id AS folder_id, f.branched_from AS branched_from, \
                    (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS first_user \
                FROM frames f \
                WHERE f.project_id = ? AND f.parent_frame_id = f.id \
                  AND f.exploration_id IS NULL \
                  AND {listable} \
             ) sessions \
             WHERE (? IS NULL OR activity_at < ? OR (activity_at = ? AND id < ?)) \
             ORDER BY activity_at DESC, id DESC LIMIT ?",
            listable = SESSION_IS_LISTABLE_SQL,
        );
        let rows = sqlx::query(&sql)
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
            let activity_at: i64 = row.try_get("activity_at")?;
            let folder_id: Option<String> = row.try_get("folder_id")?;
            let branched_from: Option<String> = row.try_get("branched_from")?;
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let title = session_display_title(custom_title, first_user);
            out.push((id, title, activity_at, folder_id, branched_from));
        }
        Ok(out)
    }

    /// Most recently active used conversation in a project. Named unused drafts
    /// stay out: they are listable (#888) but `resume_last_session` should not
    /// reopen a blank chat just because it was renamed last.
    pub async fn latest_used_session_id(&self, project_id: &str) -> Result<Option<String>> {
        let sql = format!(
            "SELECT f.id FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id \
               AND f.exploration_id IS NULL \
               AND {used} ORDER BY COALESCE(\
                (SELECT MAX(NULLIF(m.ts, 0)) FROM messages m WHERE m.frame_id = f.id), \
                f.updated_at) DESC, f.id DESC LIMIT 1",
            used = SESSION_HAS_USER_TURN_SQL,
        );
        Ok(sqlx::query_scalar(&sql)
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Pinned root frames for a project, newest first. Returned as
    /// `(frame_id, title, activity_at, folder_id, branched_from)` like `list_sessions_page`,
    /// but unpaginated so the sidebar's "Pinned" section is complete regardless
    /// of how far the keyset history has been scrolled.
    pub async fn list_pinned_sessions(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, String, i64, Option<String>, Option<String>)>> {
        let sql = format!(
            "SELECT f.id AS id, \
                COALESCE((SELECT MAX(NULLIF(m.ts, 0)) FROM messages m WHERE m.frame_id = f.id), f.updated_at) AS activity_at, \
                f.title AS custom_title, f.folder_id AS folder_id, f.branched_from AS branched_from, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1) AS first_user \
             FROM frames f \
             WHERE f.project_id = ? AND f.parent_frame_id = f.id AND COALESCE(f.pinned, 0) = 1 \
               AND f.exploration_id IS NULL \
               AND {listable} ORDER BY activity_at DESC, f.id DESC",
            listable = SESSION_IS_LISTABLE_SQL,
        );
        let rows = sqlx::query(&sql)
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        let mut out = vec![];
        for row in rows {
            let id: String = row.try_get("id")?;
            let activity_at: i64 = row.try_get("activity_at")?;
            let folder_id: Option<String> = row.try_get("folder_id")?;
            let branched_from: Option<String> = row.try_get("branched_from")?;
            let custom_title: Option<String> = row.try_get("custom_title")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            out.push((
                id,
                session_display_title(custom_title, first_user),
                activity_at,
                folder_id,
                branched_from,
            ));
        }
        Ok(out)
    }

    /// Pin or unpin a saved conversation so it floats to the top of the sidebar.
    pub async fn set_session_pinned(
        &self,
        frame_id: &str,
        project_id: &str,
        pinned: bool,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let n = sqlx::query(
            "UPDATE frames SET pinned=?, updated_at=? WHERE id=? AND project_id=? AND parent_frame_id=id",
        )
        .bind(pinned as i64)
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
        if self
            .session_has_conversation_branches(frame_id, project_id)
            .await?
        {
            anyhow::bail!("session_has_branches: delete its branches before deleting main");
        }
        let owns_current_exploration: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.project_id=? AND checkpoint.source_frame_id=? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation)",
        )
        .bind(project_id)
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?;
        if owns_current_exploration {
            anyhow::bail!(
                "exploration_mainline_frozen: abandon or select an exploration before deleting main"
            );
        }
        let mut tx = self.begin_write().await?;
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

        if remove_source {
            if self
                .session_has_conversation_branches(frame_id, source_project_id)
                .await?
            {
                anyhow::bail!("session_has_branches: delete its branches before moving main");
            }
            let owns_current_exploration: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM explorations exploration \
                 JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
                 JOIN exploration_families family ON family.id=checkpoint.family_id \
                 WHERE checkpoint.project_id=? AND checkpoint.source_frame_id=? \
                   AND checkpoint.source_frame_id=family.mainline_frame_id \
                   AND checkpoint.source_family_generation=family.generation)",
            )
            .bind(source_project_id)
            .bind(frame_id)
            .fetch_one(&self.pool)
            .await?;
            if owns_current_exploration {
                anyhow::bail!(
                    "exploration_mainline_frozen: abandon or select an exploration before moving main"
                );
            }
        }

        let mut tx = self.begin_write().await?;
        let target_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
            .bind(target_project_id)
            .fetch_optional(&mut *tx)
            .await?;
        if target_exists.is_none() {
            anyhow::bail!("Target project not found");
        }

        let source = sqlx::query(
            "SELECT agent_name,status,model,reasoning_effort,service_tier,input_tokens,output_tokens,completed_at,title \
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
                id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,reasoning_effort,service_tier,\
                input_tokens,output_tokens,created_at,updated_at,completed_at,title\
             ) VALUES(?,?,?,?,?,?,NULL,?,?,?,?,?,?,?,?,?)",
        )
        .bind(new_frame_id)
        .bind(new_frame_id)
        .bind(new_frame_id)
        .bind(source.try_get::<String, _>("agent_name")?)
        .bind(source.try_get::<String, _>("status")?)
        .bind(target_project_id)
        .bind(source.try_get::<Option<String>, _>("model")?)
        .bind(source.try_get::<Option<String>, _>("reasoning_effort")?)
        .bind(source.try_get::<Option<String>, _>("service_tier")?)
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
        let mut tx = self.begin_write().await?;
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

    /// Record which session a branch was forked from. Purely a display link for
    /// the sidebar's nesting until an explicit branch action is requested.
    pub async fn set_session_branched_from(&self, frame_id: &str, source_id: &str) -> Result<()> {
        sqlx::query("UPDATE frames SET branched_from=? WHERE id=?")
            .bind(source_id)
            .bind(frame_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark a branch created by the current checkpoint-aware flow. Legacy rows
    /// with only `branched_from` deliberately do not participate.
    pub async fn set_session_branch_point(
        &self,
        frame_id: &str,
        source_id: &str,
        checkpoint_user_index: usize,
        checkpoint_kind: &str,
    ) -> Result<()> {
        if !matches!(checkpoint_kind, "before_user" | "after_response") {
            anyhow::bail!("Invalid conversation branch checkpoint kind");
        }
        sqlx::query(
            "UPDATE frames SET branched_from=?,branch_point_user_index=?,branch_point_kind=? \
             WHERE id=?",
        )
        .bind(source_id)
        .bind(i64::try_from(checkpoint_user_index)?)
        .bind(checkpoint_kind)
        .bind(frame_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_session_branches(
        &self,
        source_session_id: &str,
        project_id: &str,
    ) -> Result<Vec<SessionBranchLink>> {
        let rows = sqlx::query(
            "SELECT f.id,f.title,f.branch_point_user_index,f.branch_point_kind, \
             merge.summary_message_seq,summary.content AS merge_summary, \
             (SELECT content FROM messages m WHERE m.frame_id=f.id AND m.role='user' \
              ORDER BY m.seq LIMIT 1) AS first_user \
             FROM frames f LEFT JOIN session_branch_merges merge ON merge.id=( \
               SELECT latest.id FROM session_branch_merges latest WHERE latest.branch_frame_id=f.id \
               ORDER BY latest.created_at DESC,latest.summary_message_seq DESC,latest.id DESC LIMIT 1) \
             LEFT JOIN messages summary ON summary.frame_id=merge.source_frame_id \
               AND summary.seq=merge.summary_message_seq \
             WHERE f.branched_from=? AND f.project_id=? \
               AND f.parent_frame_id=f.id AND f.exploration_id IS NULL \
               AND f.branch_point_user_index IS NOT NULL \
               AND f.branch_point_kind IN ('before_user','after_response') \
             ORDER BY f.created_at,f.id",
        )
        .bind(source_session_id)
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let title =
                    session_display_title(row.try_get("title")?, row.try_get("first_user")?);
                Ok(SessionBranchLink {
                    id: row.try_get("id")?,
                    title: title.strip_prefix("Branch: ").unwrap_or(&title).to_string(),
                    source_session_id: source_session_id.to_string(),
                    checkpoint_user_index: usize::try_from(
                        row.try_get::<i64, _>("branch_point_user_index")?,
                    )?,
                    checkpoint_kind: row.try_get("branch_point_kind")?,
                    merged: row
                        .try_get::<Option<i64>, _>("summary_message_seq")?
                        .is_some(),
                    merge_summary: row
                        .try_get::<Option<String>, _>("merge_summary")?
                        .and_then(|content| {
                            serde_json::from_str::<wisp_llm::Content>(&content).ok()
                        })
                        .map(|content| content.as_text()),
                })
            })
            .collect()
    }

    pub async fn list_mergeable_branch_ids(&self, project_id: &str) -> Result<HashSet<String>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM frames WHERE project_id=? AND parent_frame_id=id \
             AND exploration_id IS NULL AND branched_from IS NOT NULL \
             AND branch_point_user_index IS NOT NULL \
             AND branch_point_kind IN ('before_user','after_response')",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn list_session_branch_states(
        &self,
        project_id: &str,
    ) -> Result<HashMap<String, String>> {
        let rows = sqlx::query(
            "SELECT frame.id,frame.branch_point_kind, \
             EXISTS(SELECT 1 FROM session_branch_merges merge WHERE merge.branch_frame_id=frame.id) AS merged \
             FROM frames frame WHERE frame.project_id=? AND frame.branched_from IS NOT NULL \
             AND frame.branch_point_user_index IS NOT NULL",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let kind: String = row.try_get("branch_point_kind")?;
                let merged = row.try_get::<i64, _>("merged")? != 0;
                Ok((
                    id,
                    if merged {
                        "merged"
                    } else if kind == "orphaned" {
                        "orphaned"
                    } else {
                        "active"
                    }
                    .to_string(),
                ))
            })
            .collect()
    }

    pub async fn session_branch_state(&self, frame_id: &str) -> Result<Option<&'static str>> {
        let row = sqlx::query(
            "SELECT branch_point_kind,EXISTS(SELECT 1 FROM session_branch_merges merge \
             WHERE merge.branch_frame_id=frames.id) AS merged FROM frames WHERE id=? \
             AND branched_from IS NOT NULL AND branch_point_user_index IS NOT NULL",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let kind: String = row.try_get("branch_point_kind")?;
            let merged = row.try_get::<i64, _>("merged")? != 0;
            Ok(if merged {
                "merged"
            } else if kind == "orphaned" {
                "orphaned"
            } else {
                "active"
            })
        })
        .transpose()
    }

    pub async fn preview_session_branch_merge(
        &self,
        branch_session_id: &str,
        project_id: &str,
    ) -> Result<SessionBranchMergePreview> {
        let mut tx = self.pool.begin().await?;
        let preview = session_branch_merge_snapshot(&mut tx, branch_session_id, project_id)
            .await?
            .0;
        tx.rollback().await?;
        Ok(preview)
    }

    /// Append the user-approved branch summary to the main conversation's
    /// current tail. Mainline messages created after the checkpoint are never
    /// read, rewritten, or included in the branch guard.
    pub async fn merge_session_branch_summary(
        &self,
        branch_session_id: &str,
        project_id: &str,
        expected_guard_hash: &str,
        summary: &str,
    ) -> Result<SessionBranchMerge> {
        let summary = summary.trim();
        if summary.is_empty() {
            anyhow::bail!("Branch merge summary cannot be empty");
        }
        if summary.chars().count() > 64_000 {
            anyhow::bail!("Branch merge summary is too long");
        }
        let mut tx = self.begin_write().await?;
        let already_merged: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM session_branch_merges WHERE branch_frame_id=?)",
        )
        .bind(branch_session_id)
        .fetch_one(&mut *tx)
        .await?;
        if already_merged {
            anyhow::bail!("Conversation branch has already been merged");
        }
        let (preview, _) =
            session_branch_merge_snapshot(&mut tx, branch_session_id, project_id).await?;
        if preview.guard_hash != expected_guard_hash {
            anyhow::bail!(
                "The branch changed while its summary was being prepared. Summarize it again."
            );
        }
        let summary_message_seq: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE frame_id=?")
                .bind(&preview.main_session_id)
                .fetch_one(&mut *tx)
                .await?;
        let message = Message::assistant(summary);
        sqlx::query(
            "INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&preview.main_session_id)
        .bind(summary_message_seq)
        .bind("assistant")
        .bind(serde_json::to_string(&message.content)?)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(message.ts)
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;
        // Do not emit a normal Text event for the summary. Event replay
        // coalesces adjacent assistant text, so a tail-only merge could be
        // folded into the previous answer. The persisted message remains in
        // model context while branch metadata drives its checkpoint card.
        sqlx::query(
            "INSERT INTO session_branch_merges(id,source_frame_id,branch_frame_id,checkpoint_user_index,checkpoint_kind,summary_message_seq,guard_hash,created_at) \
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&preview.main_session_id)
        .bind(branch_session_id)
        .bind(i64::try_from(preview.checkpoint_user_index)?)
        .bind(&preview.checkpoint_kind)
        .bind(summary_message_seq)
        .bind(expected_guard_hash)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE frames SET updated_at=? WHERE id=?")
            .bind(chrono::Utc::now().timestamp())
            .bind(&preview.main_session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(SessionBranchMerge {
            main_session_id: preview.main_session_id,
            branch_session_id: branch_session_id.to_string(),
            summary_message_seq,
        })
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

    /// Persist an artifact and mint an immutable version for its current location.

    pub async fn search_sessions(
        &self,
        project_id: Option<&str>,
        query: &str,
        limit: i64,
        session_id: Option<&str>,
        preferred_project_id: Option<&str>,
    ) -> Result<Vec<SessionSearchResult>> {
        let q = query.trim().to_lowercase();
        let pattern = format!("%{q}%");
        let sql = format!(
            "WITH searchable_sessions AS ( \
                SELECT f.rowid AS frame_rowid, f.id AS id, f.project_id AS project_id, \
                    COALESCE(p.name,'') AS project_name, f.created_at AS created_at, \
                    COALESCE(f.title,'') AS custom_title, \
                    (SELECT content FROM messages m WHERE m.frame_id=f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                    (SELECT role FROM messages m WHERE m.frame_id=f.id ORDER BY m.seq DESC LIMIT 1) AS last_role, \
                    (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id=f.id) AS activity_at, \
                    (SELECT COALESCE(MAX(ts), f.updated_at) FROM messages m WHERE m.frame_id=f.id) > f.seen_at AS unseen \
                FROM frames f JOIN projects p ON p.id=f.project_id \
                WHERE f.parent_frame_id=f.id \
                  AND f.exploration_id IS NULL \
                  AND f.project_id NOT LIKE 'scratch:%' \
                  AND {listable} \
                  AND (? IS NULL OR f.project_id=?) \
                  AND (? IS NULL OR f.id=?) \
             ) \
             SELECT * FROM searchable_sessions s \
             WHERE (?='' OR lower(COALESCE(NULLIF(s.custom_title,''), s.first_user, '')) LIKE ? \
                OR EXISTS (SELECT 1 FROM messages m WHERE m.frame_id=s.id \
                    AND lower(COALESCE(m.content,'')) LIKE ?)) \
             ORDER BY CASE WHEN ? IS NOT NULL AND s.project_id=? THEN 0 ELSE 1 END, \
                CASE WHEN ?='' OR lower(COALESCE(NULLIF(s.custom_title,''), s.first_user, '')) LIKE ? THEN 0 ELSE 1 END, \
                s.activity_at DESC, s.frame_rowid DESC LIMIT ?",
            listable = SESSION_IS_LISTABLE_SQL,
        );
        let rows = sqlx::query(&sql)
            .bind(project_id)
            .bind(project_id)
            .bind(session_id)
            .bind(session_id)
            .bind(&q)
            .bind(&pattern)
            .bind(&pattern)
            .bind(preferred_project_id)
            .bind(preferred_project_id)
            .bind(&q)
            .bind(&pattern)
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
                    unseen: row.try_get("unseen")?,
                })
            })
            .collect()
    }

    pub async fn get_session_reference(&self, id: &str) -> Result<Option<SessionSearchResult>> {
        Ok(self
            .search_sessions(None, "", 1, Some(id), None)
            .await?
            .into_iter()
            .next())
    }

    /// Per-project totals for the Usage settings page. A project is the durable
    /// workspace boundary in Wisp; scratch projects are intentionally omitted.
    pub async fn token_usage_by_project(&self) -> Result<Vec<ProjectTokenUsage>> {
        let rows = sqlx::query(
            "WITH session_usage AS (\
                SELECT r.id AS id, r.project_id AS project_id, r.updated_at AS updated_at, \
                    SUM(COALESCE(json_extract(e.event_json,'$.input'),0)) AS input, \
                    SUM(COALESCE(json_extract(e.event_json,'$.output'),0)) AS output, \
                    SUM(COALESCE(json_extract(e.event_json,'$.reasoning'),0)) AS reasoning, \
                    SUM(COALESCE(json_extract(e.event_json,'$.cached'),0)) AS cached \
                FROM session_ui_events e \
                JOIN frames f ON f.id = e.frame_id \
                JOIN frames r ON r.id = COALESCE(f.root_frame_id, f.id) \
                WHERE e.event_json LIKE '{\"kind\":\"Usage\"%' \
                GROUP BY r.id\
             ) \
             SELECT p.id AS project_id, COALESCE(p.name,'') AS name, \
                    COALESCE(p.workspace_dir,'') AS workspace_dir, \
                    MAX(s.updated_at) AS updated_at, COUNT(*) AS session_count, \
                    SUM(s.input) AS input, SUM(s.output) AS output, \
                    SUM(s.reasoning) AS reasoning, SUM(s.cached) AS cached \
             FROM session_usage s JOIN projects p ON p.id = s.project_id \
             WHERE p.id NOT LIKE 'scratch:%' \
             GROUP BY p.id ORDER BY updated_at DESC, p.id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProjectTokenUsage {
                    project_id: row.try_get("project_id")?,
                    name: row.try_get("name")?,
                    workspace_dir: row.try_get("workspace_dir")?,
                    updated_at: row.try_get("updated_at")?,
                    session_count: row.try_get("session_count")?,
                    input: row.try_get("input")?,
                    output: row.try_get("output")?,
                    reasoning: row.try_get("reasoning")?,
                    cached: row.try_get("cached")?,
                })
            })
            .collect()
    }

    /// Daily input + output token totals for 53 calendar weeks. New Usage
    /// events carry their own timestamp; legacy events fall back to the root
    /// session's last activity because no round timestamp was persisted then.
    pub async fn token_usage_activity(&self) -> Result<Vec<TokenUsageDay>> {
        let rows = sqlx::query(
            "SELECT date(COALESCE(\
                        NULLIF(CAST(json_extract(e.event_json,'$.created_at') AS INTEGER),0),\
                        r.updated_at\
                    ),'unixepoch','localtime') AS day, \
                    SUM(COALESCE(json_extract(e.event_json,'$.input'),0) + \
                        COALESCE(json_extract(e.event_json,'$.output'),0)) AS tokens \
             FROM session_ui_events e \
             JOIN frames f ON f.id=e.frame_id \
             JOIN frames r ON r.id=COALESCE(f.root_frame_id,f.id) \
             JOIN projects p ON p.id=r.project_id \
             WHERE p.id NOT LIKE 'scratch:%' \
               AND e.event_json LIKE '{\"kind\":\"Usage\"%' \
             GROUP BY day",
        )
        .fetch_all(&self.pool)
        .await?;
        let totals = rows
            .into_iter()
            .map(|row| Ok((row.try_get("day")?, row.try_get("tokens")?)))
            .collect::<Result<HashMap<String, i64>>>()?;
        let today = Local::now().date_naive();
        let start =
            today - Duration::days(i64::from(today.weekday().num_days_from_monday()) + 52 * 7);
        Ok((0..53 * 7)
            .map(|offset| {
                let date = start + Duration::days(offset);
                let key = date.format("%Y-%m-%d").to_string();
                TokenUsageDay {
                    tokens: totals.get(&key).copied().unwrap_or(0),
                    future: date > today,
                    date: key,
                }
            })
            .collect())
    }

    /// Input + output token share by the model selected for each round.
    /// Legacy events use their frame's current model binding as a fallback.
    pub async fn token_usage_by_model(&self) -> Result<Vec<ModelTokenUsage>> {
        let rows = sqlx::query(
            "SELECT COALESCE(\
                        NULLIF(json_extract(e.event_json,'$.model'),''),\
                        NULLIF(f.model,''),\
                        'unknown'\
                    ) AS model_key, \
                    SUM(COALESCE(json_extract(e.event_json,'$.input'),0) + \
                        COALESCE(json_extract(e.event_json,'$.output'),0)) AS tokens \
             FROM session_ui_events e \
             JOIN frames f ON f.id=e.frame_id \
             JOIN frames r ON r.id=COALESCE(f.root_frame_id,f.id) \
             JOIN projects p ON p.id=r.project_id \
             WHERE p.id NOT LIKE 'scratch:%' \
               AND e.event_json LIKE '{\"kind\":\"Usage\"%' \
             GROUP BY model_key ORDER BY tokens DESC, model_key",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModelTokenUsage {
                    model: row.try_get("model_key")?,
                    tokens: row.try_get("tokens")?,
                })
            })
            .collect()
    }

    /// Ranked SKILL (`use_skill`) and MCP (`mcp:*`) tool-call counts from
    /// persisted transcript events. Skill identity comes from the call preview
    /// (the skill name); skipped-batch placeholders are ignored.
    pub async fn tool_call_usage_ranking(&self) -> Result<Vec<ToolCallUsage>> {
        let rows = sqlx::query(
            "SELECT kind, name, COUNT(*) AS calls FROM (\
                SELECT CASE \
                        WHEN json_extract(e.event_json,'$.name') = 'use_skill' THEN 'skill' \
                        ELSE 'mcp' \
                    END AS kind, \
                    CASE \
                        WHEN json_extract(e.event_json,'$.name') = 'use_skill' THEN \
                            COALESCE(\
                                NULLIF(TRIM(json_extract(e.event_json,'$.preview')),''),\
                                'unknown'\
                            ) \
                        ELSE COALESCE(\
                            NULLIF(SUBSTR(json_extract(e.event_json,'$.name'),5),''),\
                            'unknown'\
                        ) \
                    END AS name \
                FROM session_ui_events e \
                JOIN frames f ON f.id=e.frame_id \
                JOIN frames r ON r.id=COALESCE(f.root_frame_id,f.id) \
                JOIN projects p ON p.id=r.project_id \
                WHERE p.id NOT LIKE 'scratch:%' \
                  AND e.event_json LIKE '{\"kind\":\"ToolCall\"%' \
                  AND (\
                        json_extract(e.event_json,'$.name') LIKE 'mcp:%' \
                        OR (\
                            json_extract(e.event_json,'$.name') = 'use_skill' \
                            AND COALESCE(json_extract(e.event_json,'$.preview'),'') \
                                NOT LIKE 'Skipped%' \
                        )\
                  )\
             ) ranked \
             GROUP BY kind, name \
             ORDER BY calls DESC, kind, name",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ToolCallUsage {
                    kind: row.try_get("kind")?,
                    name: row.try_get("name")?,
                    calls: row.try_get("calls")?,
                })
            })
            .collect()
    }

    /// One project workspace's session usage, newest first. Sub-agent frames
    /// fold into their root session. Serde tags internally-tagged enums first,
    /// so the LIKE prefix is a cheap exact filter for `Usage` events.
    pub async fn token_usage_by_session(
        &self,
        project_id: &str,
        offset: i64,
        limit: i64,
    ) -> Result<SessionTokenUsagePage> {
        let total = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT r.id) \
             FROM session_ui_events e \
             JOIN frames f ON f.id = e.frame_id \
             JOIN frames r ON r.id = COALESCE(f.root_frame_id, f.id) \
             WHERE r.project_id=? AND e.event_json LIKE '{\"kind\":\"Usage\"%'",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT r.id AS id, r.title AS custom_title, r.updated_at AS updated_at, \
                    (SELECT content FROM messages m WHERE m.frame_id = r.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                    SUM(COALESCE(json_extract(e.event_json,'$.input'),0)) AS input, \
                    SUM(COALESCE(json_extract(e.event_json,'$.output'),0)) AS output, \
                    SUM(COALESCE(json_extract(e.event_json,'$.reasoning'),0)) AS reasoning, \
                    SUM(COALESCE(json_extract(e.event_json,'$.cached'),0)) AS cached \
             FROM session_ui_events e \
             JOIN frames f ON f.id = e.frame_id \
             JOIN frames r ON r.id = COALESCE(f.root_frame_id, f.id) \
             WHERE r.project_id=? AND e.event_json LIKE '{\"kind\":\"Usage\"%' \
             GROUP BY r.id ORDER BY r.updated_at DESC, r.id DESC LIMIT ? OFFSET ?",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 100))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;
        let items = rows
            .into_iter()
            .map(|row| {
                Ok(SessionTokenUsage {
                    id: row.try_get("id")?,
                    title: session_display_title(
                        row.try_get::<Option<String>, _>("custom_title")?,
                        row.try_get::<Option<String>, _>("first_user")?,
                    ),
                    updated_at: row.try_get("updated_at")?,
                    input: row.try_get("input")?,
                    output: row.try_get("output")?,
                    reasoning: row.try_get("reasoning")?,
                    cached: row.try_get("cached")?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(SessionTokenUsagePage { items, total })
    }
}
