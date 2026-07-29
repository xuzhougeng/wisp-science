use super::Store;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};
use wisp_llm::Message;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpConversationParticipant {
    pub parent_frame_id: String,
    pub agent_profile_id: String,
    pub agent_label: String,
    pub child_frame_id: String,
    pub synced_parent_seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpConversationTurn {
    pub id: String,
    pub parent_frame_id: String,
    pub child_frame_id: String,
    pub agent_profile_id: String,
    pub agent_label: String,
    pub profile_fingerprint: String,
    pub agent_session_id: String,
    pub user_message_seq: i64,
    pub response_start_seq: i64,
    pub response_end_seq: i64,
    pub child_response_start: i64,
    pub child_response_end: i64,
    pub created_at: i64,
}

fn participant_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AcpConversationParticipant> {
    Ok(AcpConversationParticipant {
        parent_frame_id: row.try_get("parent_frame_id")?,
        agent_profile_id: row.try_get("agent_profile_id")?,
        agent_label: row.try_get("agent_label")?,
        child_frame_id: row.try_get("child_frame_id")?,
        synced_parent_seq: row.try_get("synced_parent_seq")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn turn_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AcpConversationTurn> {
    Ok(AcpConversationTurn {
        id: row.try_get("id")?,
        parent_frame_id: row.try_get("parent_frame_id")?,
        child_frame_id: row.try_get("child_frame_id")?,
        agent_profile_id: row.try_get("agent_profile_id")?,
        agent_label: row.try_get("agent_label")?,
        profile_fingerprint: row.try_get("profile_fingerprint")?,
        agent_session_id: row.try_get("agent_session_id")?,
        user_message_seq: row.try_get("user_message_seq")?,
        response_start_seq: row.try_get("response_start_seq")?,
        response_end_seq: row.try_get("response_end_seq")?,
        child_response_start: row.try_get("child_response_start")?,
        child_response_end: row.try_get("child_response_end")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    pub async fn get_acp_conversation_participant(
        &self,
        parent_frame_id: &str,
        agent_profile_id: &str,
    ) -> Result<Option<AcpConversationParticipant>> {
        let row = sqlx::query(
            "SELECT parent_frame_id,agent_profile_id,agent_label,child_frame_id,\
             synced_parent_seq,created_at,updated_at \
             FROM acp_conversation_participants \
             WHERE parent_frame_id=? AND agent_profile_id=?",
        )
        .bind(parent_frame_id)
        .bind(agent_profile_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(participant_from_row).transpose()
    }

    /// Create the participant's hidden ACP frame and durable parent binding in
    /// one transaction. The parent/project predicate prevents cross-project
    /// lineage even when a stale UI races a project switch.
    pub async fn create_acp_conversation_participant(
        &self,
        parent_frame_id: &str,
        project_id: &str,
        agent_profile_id: &str,
        agent_label: &str,
        child_frame_id: &str,
    ) -> Result<AcpConversationParticipant> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let inserted = sqlx::query(
            "INSERT INTO frames(\
                id,parent_frame_id,root_frame_id,agent_name,status,project_id,model,\
                input_tokens,output_tokens,created_at,updated_at,completed_at\
             ) SELECT ?,id,COALESCE(root_frame_id,id),?,'running',project_id,'acp',0,0,?,?,NULL \
             FROM frames WHERE id=? AND project_id=?",
        )
        .bind(child_frame_id)
        .bind(agent_label)
        .bind(now)
        .bind(now)
        .bind(parent_frame_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            bail!("Parent conversation not found");
        }
        sqlx::query(
            "INSERT INTO acp_conversation_participants(\
             parent_frame_id,agent_profile_id,agent_label,child_frame_id,\
             synced_parent_seq,created_at,updated_at) VALUES(?,?,?,?,0,?,?)",
        )
        .bind(parent_frame_id)
        .bind(agent_profile_id)
        .bind(agent_label)
        .bind(child_frame_id)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AcpConversationParticipant {
            parent_frame_id: parent_frame_id.to_string(),
            agent_profile_id: agent_profile_id.to_string(),
            agent_label: agent_label.to_string(),
            child_frame_id: child_frame_id.to_string(),
            synced_parent_seq: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn acp_conversation_parent_for_child(
        &self,
        child_frame_id: &str,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT parent_frame_id FROM acp_conversation_participants \
             WHERE child_frame_id=?",
        )
        .bind(child_frame_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn acp_conversation_child_frames(
        &self,
        parent_frame_id: &str,
    ) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT child_frame_id FROM acp_conversation_participants \
             WHERE parent_frame_id=? ORDER BY created_at,child_frame_id",
        )
        .bind(parent_frame_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn acp_conversation_turns(
        &self,
        parent_frame_id: &str,
    ) -> Result<Vec<AcpConversationTurn>> {
        let rows = sqlx::query(
            "SELECT id,parent_frame_id,child_frame_id,agent_profile_id,agent_label,\
             profile_fingerprint,agent_session_id,user_message_seq,response_start_seq,\
             response_end_seq,child_response_start,child_response_end,created_at \
             FROM acp_conversation_turns WHERE parent_frame_id=? \
             ORDER BY user_message_seq",
        )
        .bind(parent_frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(turn_from_row).collect()
    }

    pub async fn acp_conversation_response_ranges(
        &self,
        parent_frame_id: &str,
        agent_profile_id: &str,
    ) -> Result<Vec<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT response_start_seq,response_end_seq \
             FROM acp_conversation_turns \
             WHERE parent_frame_id=? AND agent_profile_id=? \
             ORDER BY response_start_seq",
        )
        .bind(parent_frame_id)
        .bind(agent_profile_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Atomically mirror a completed child response into the parent transcript,
    /// freeze its provenance, and advance only this participant's context cursor.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_acp_conversation_turn(
        &self,
        parent_frame_id: &str,
        child_frame_id: &str,
        agent_profile_id: &str,
        agent_label: &str,
        profile_fingerprint: &str,
        agent_session_id: &str,
        user_message_seq: i64,
        child_response_start: i64,
        child_response_end: i64,
        responses: &[Message],
    ) -> Result<Vec<(i64, Message)>> {
        if responses.is_empty() {
            bail!("ACP participant turn produced no response messages");
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let mut next_seq: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE frame_id=?")
                .bind(parent_frame_id)
                .fetch_one(&mut *tx)
                .await?;
        let response_start_seq = next_seq;
        let mut mirrored = Vec::with_capacity(responses.len());
        for response in responses {
            let mut response = response.clone();
            if response.role == wisp_llm::Role::Assistant {
                response.model_name = Some(agent_label.to_string());
            }
            insert_message(&mut tx, parent_frame_id, next_seq, &response).await?;
            mirrored.push((next_seq, response));
            next_seq += 1;
        }
        let response_end_seq = next_seq - 1;
        sqlx::query(
            "INSERT INTO acp_conversation_turns(\
             id,parent_frame_id,child_frame_id,agent_profile_id,agent_label,\
             profile_fingerprint,agent_session_id,user_message_seq,response_start_seq,\
             response_end_seq,child_response_start,child_response_end,created_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(parent_frame_id)
        .bind(child_frame_id)
        .bind(agent_profile_id)
        .bind(agent_label)
        .bind(profile_fingerprint)
        .bind(agent_session_id)
        .bind(user_message_seq)
        .bind(response_start_seq)
        .bind(response_end_seq)
        .bind(child_response_start)
        .bind(child_response_end)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let updated = sqlx::query(
            "UPDATE acp_conversation_participants \
             SET agent_label=?,synced_parent_seq=?,updated_at=? \
             WHERE parent_frame_id=? AND agent_profile_id=? AND child_frame_id=?",
        )
        .bind(agent_label)
        .bind(user_message_seq)
        .bind(now)
        .bind(parent_frame_id)
        .bind(agent_profile_id)
        .bind(child_frame_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            bail!("ACP conversation participant binding changed during the turn");
        }
        tx.commit().await?;
        Ok(mirrored)
    }
}

async fn insert_message(
    tx: &mut Transaction<'_, Sqlite>,
    frame_id: &str,
    seq: i64,
    message: &Message,
) -> Result<()> {
    let role = if message.role == wisp_llm::Role::User
        && message.tool_name.as_deref() == Some(super::AGENT_WORKFLOW_COMPLETION_TOOL)
    {
        "internal".to_string()
    } else {
        format!("{:?}", message.role).to_ascii_lowercase()
    };
    let content = serde_json::to_string(&message.content)?;
    let tool_calls = (!message.tool_calls.is_empty())
        .then(|| serde_json::to_string(&message.tool_calls))
        .transpose()?;
    sqlx::query(
        "INSERT INTO messages(\
         id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name\
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(frame_id)
    .bind(seq)
    .bind(role)
    .bind(content)
    .bind(tool_calls)
    .bind(message.tool_call_id.as_deref())
    .bind(message.tool_name.as_deref())
    .bind(message.reasoning.as_deref())
    .bind(message.ts)
    .bind(message.model_name.as_deref())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ACP_CONVERSATION_PARTICIPANTS_MIGRATION;

    async fn store_with_parent() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_acp_conversation_participants_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        store
            .create_frame("parent", "p", "Wisp", "native")
            .await
            .unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn participant_child_and_turn_provenance_round_trip() {
        let (store, path) = store_with_parent().await;
        let participant = store
            .create_acp_conversation_participant(
                "parent",
                "p",
                "profile-codex",
                "Codex",
                "child-codex",
            )
            .await
            .unwrap();
        assert_eq!(participant.synced_parent_seq, 0);
        assert_eq!(
            store
                .acp_conversation_parent_for_child("child-codex")
                .await
                .unwrap()
                .as_deref(),
            Some("parent")
        );

        store
            .append_message("parent", 1, &Message::user("Design the homepage"))
            .await
            .unwrap();
        let responses = vec![
            Message::tool("call-1", "acp:read", "{}"),
            Message::assistant("Start with the information hierarchy."),
        ];
        let mirrored = store
            .record_acp_conversation_turn(
                "parent",
                "child-codex",
                "profile-codex",
                "Codex",
                "fnv1a64:test",
                "agent-session-1",
                1,
                2,
                3,
                &responses,
            )
            .await
            .unwrap();
        assert_eq!(
            mirrored.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            store
                .get_acp_conversation_participant("parent", "profile-codex")
                .await
                .unwrap()
                .unwrap()
                .synced_parent_seq,
            1
        );
        let turns = store.acp_conversation_turns("parent").await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].agent_profile_id, "profile-codex");
        assert_eq!(turns[0].agent_session_id, "agent-session-1");
        assert_eq!(turns[0].response_start_seq, 2);
        assert_eq!(turns[0].response_end_seq, 3);
        assert_eq!(
            store
                .acp_conversation_response_ranges("parent", "profile-codex")
                .await
                .unwrap(),
            vec![(2, 3)]
        );
        let messages = store.load_messages("parent").await.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].model_name.as_deref(), Some("Codex"));
        store.truncate_messages("parent", 1).await.unwrap();
        assert!(store
            .acp_conversation_turns("parent")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .get_acp_conversation_participant("parent", "profile-codex")
                .await
                .unwrap()
                .unwrap()
                .synced_parent_seq,
            0
        );
        store.delete_session("parent", "p").await.unwrap();
        assert!(store
            .get_acp_conversation_participant("parent", "profile-codex")
            .await
            .unwrap()
            .is_none());
        assert!(store
            .frame_project_id("child-codex")
            .await
            .unwrap()
            .is_none());

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn participant_migration_is_idempotent() {
        let (store, path) = store_with_parent().await;
        store
            .create_acp_conversation_participant(
                "parent",
                "p",
                "profile-codex",
                "Codex",
                "child-codex",
            )
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(ACP_CONVERSATION_PARTICIPANTS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).await.unwrap();
        assert!(reopened
            .get_acp_conversation_participant("parent", "profile-codex")
            .await
            .unwrap()
            .is_some());
        assert!(reopened
            .schema_migrations()
            .await
            .unwrap()
            .contains(&ACP_CONVERSATION_PARTICIPANTS_MIGRATION.to_string()));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
