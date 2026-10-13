use crate::{context_epochs::LIVE_LOG_ROWS, session_display_title, Store};
use anyhow::Result;
use sqlx::Row;
use wisp_dto::{
    ProjectChatSearchCursor, ProjectChatSearchHit, ProjectChatSearchPage, ProjectChatSearchTarget,
};

impl Store {
    /// Visual event-backed outlines intentionally omit message cursors (legacy
    /// compaction could renumber them). Align the saved questions with the
    /// visual history before resolving a hit; never guess among repeated runs
    /// of identical questions. Normal epoch histories have a complete match.
    pub async fn locate_project_chat_match(
        &self,
        project_id: &str,
        session_id: &str,
        message_seq: i64,
    ) -> Result<Option<ProjectChatSearchTarget>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.locate_project_chat_match(project_id, session_id, message_seq))
                .await;
        }
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN frames f ON f.id=m.frame_id \
             WHERE f.project_id=? AND f.parent_frame_id=f.id AND f.exploration_id IS NULL \
             AND m.frame_id=? AND m.seq=? AND m.role IN ('user','assistant','tool') AND {LIVE_LOG_ROWS})"
        )).bind(project_id).bind(session_id).bind(message_seq).fetch_one(&self.pool).await?;
        anyhow::ensure!(
            exists,
            "The matching message is no longer available in this project"
        );
        let questions = self.load_session_user_messages(session_id).await?;
        let Some(index) = questions.iter().rposition(|(seq, ..)| *seq <= message_seq) else {
            return Ok(None);
        };
        let outline = self.load_session_outline(session_id).await?;
        let base = if questions.len() == outline.len() {
            // A complete visual history and message log share user ordinals.
            // Their text can differ because model messages include attachments
            // and composer references that the visual User event omits.
            0
        } else {
            let mut candidates =
                outline
                    .windows(questions.len())
                    .enumerate()
                    .filter(|(_, window)| {
                        window
                            .iter()
                            .zip(&questions)
                            .all(|(entry, (_, text, ..))| &entry.text == text)
                    });
            let Some((base, _)) = candidates.next() else {
                return Ok(None);
            };
            if candidates.next().is_some() {
                return Ok(None);
            }
            base
        };
        Ok(Some(ProjectChatSearchTarget {
            user_index: outline[base + index].user_index,
            before_seq: questions.get(index + 1).map(|(seq, ..)| *seq),
        }))
    }

    /// Search saved message text, including older epochs and shelved sessions.
    /// Work stays in the owning project database, with bounded IPC results.
    /// SQLite lower() folds ASCII case; other scripts match literally. instr()
    /// deliberately treats %, _, quotes and backslashes as ordinary characters.
    pub async fn search_project_chats(
        &self,
        project_id: &str,
        query: &str,
        cursor: Option<&ProjectChatSearchCursor>,
        limit: usize,
    ) -> Result<ProjectChatSearchPage> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(ProjectChatSearchPage::default());
        }
        anyhow::ensure!(
            query.chars().count() <= 256,
            "Search keyword is too long (maximum 256 characters)"
        );
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.search_project_chats(project_id, query, cursor, limit)).await;
        }
        let limit = limit.clamp(1, 100);
        // Decode JSON content before matching, so escaped newlines/quotes work
        // and attachment URLs/base64/JSON keys are never searched. Match only
        // the live transcript: compaction checkpoints and copied tails must not
        // manufacture duplicate results. Shelving is not a history deletion.
        let rows = sqlx::query(&format!(
            "WITH texts AS (\
                SELECT m.frame_id, m.seq, m.role, m.ts, \
                    CASE json_type(m.content) \
                        WHEN 'text' THEN json_extract(m.content, '$') \
                        WHEN 'array' THEN COALESCE((\
                            SELECT group_concat(json_extract(part.value, '$.text'), char(10)) \
                            FROM json_each(m.content) part \
                            WHERE json_extract(part.value, '$.type')='text'\
                        ), '') ELSE '' END AS body \
                FROM messages m JOIN frames f ON f.id=m.frame_id \
                WHERE f.project_id=?1 AND f.parent_frame_id=f.id \
                    AND f.exploration_id IS NULL \
                    AND m.role IN ('user','assistant','tool') AND {LIVE_LOG_ROWS} \
                    AND (?3 IS NULL OR (m.ts, m.frame_id, m.seq) < (?3, ?4, ?5))\
             ), matches AS (\
                SELECT *, instr(lower(body), lower(?2)) AS pos FROM texts\
             ), page AS (\
                SELECT * FROM matches WHERE pos>0 \
                    AND body NOT LIKE '[context summary checkpoint]%' \
                    AND body NOT LIKE '[compacted;%' \
                ORDER BY ts DESC, frame_id DESC, seq DESC LIMIT ?6\
             ) \
             SELECT frame_id, seq, role, ts, \
                CASE WHEN pos>81 THEN '…' ELSE '' END || \
                    substr(body, max(1, pos-80), min(80, pos-1)) AS snippet_before, \
                substr(body, pos, length(?2)) AS matched, \
                substr(body, pos+length(?2), 160) || \
                    CASE WHEN length(body)>pos+length(?2)+159 THEN '…' ELSE '' END AS snippet_after, \
                (SELECT title FROM frames WHERE id=page.frame_id) AS custom_title, \
                (SELECT content FROM messages WHERE frame_id=page.frame_id AND role='user' ORDER BY seq LIMIT 1) AS first_user \
             FROM page ORDER BY ts DESC, frame_id DESC, seq DESC"
        ))
        .bind(project_id)
        .bind(query)
        .bind(cursor.map(|c| c.ts))
        .bind(cursor.map(|c| c.session_id.as_str()))
        .bind(cursor.map(|c| c.message_seq))
        .bind((limit + 1) as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut items = rows
            .into_iter()
            .map(|row| {
                Ok(ProjectChatSearchHit {
                    session_id: row.try_get("frame_id")?,
                    session_title: session_display_title(
                        row.try_get("custom_title")?,
                        row.try_get("first_user")?,
                    ),
                    message_seq: row.try_get("seq")?,
                    role: row.try_get("role")?,
                    ts: row.try_get("ts")?,
                    before: row.try_get("snippet_before")?,
                    matched: row.try_get("matched")?,
                    after: row.try_get("snippet_after")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let has_more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = has_more.then(|| {
            let last = items.last().expect("a full search page has a last hit");
            ProjectChatSearchCursor {
                ts: last.ts,
                session_id: last.session_id.clone(),
                message_seq: last.message_seq,
            }
        });
        Ok(ProjectChatSearchPage { items, next_cursor })
    }
}
