//! A timer owns one complete turn, delimited while holding the session workflow
//! lock. Replacing it preserves intervening human turns and never undoes files.
use super::Store;
use anyhow::Result;
use sqlx::Row;

impl Store {
    /// Seal an interrupted timer before any subsequent normal turn can append.
    /// Also used after a timer finishes, while its workflow lock is still held.
    pub async fn finish_session_timer_turn(&self, frame_id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.finish_session_timer_turn(frame_id)).await;
        }
        sqlx::query("UPDATE session_timer_turns SET end_seq=(SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?1), end_ui_seq=(SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=?1) WHERE frame_id=?1 AND end_seq IS NULL")
            .bind(frame_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn begin_session_timer_turn(&self, frame_id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.begin_session_timer_turn(frame_id)).await;
        }
        sqlx::query("INSERT INTO session_timer_turns(frame_id,base_epoch,start_seq,start_ui_seq) SELECT id,head_epoch,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?1),(SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=?1) FROM frames WHERE id=?1")
            .bind(frame_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Roll out exactly the previous timer. Later compaction snapshots may
    /// contain it, so restore the pre-timer epoch and replay subsequent *live*
    /// rows with their original seqs. Human messages and tool pairs survive;
    /// stale summaries cannot leak the removed timer back into model context.
    pub async fn clear_session_timer_turn(
        &self,
        frame_id: &str,
    ) -> Result<Option<wisp_dto::TimerTurnRemoval>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.clear_session_timer_turn(frame_id)).await;
        }
        self.finish_session_timer_turn(frame_id).await?;
        let mut tx = self.begin_write().await?;
        let row = sqlx::query("SELECT base_epoch,start_seq,end_seq,start_ui_seq,end_ui_seq FROM session_timer_turns WHERE frame_id=?")
            .bind(frame_id).fetch_optional(&mut *tx).await?;
        let Some(row) = row else { return Ok(None) };
        let base: i64 = row.try_get("base_epoch")?;
        let start: i64 = row.try_get("start_seq")?;
        let end: i64 = row.try_get("end_seq")?;
        let ui_start: i64 = row.try_get("start_ui_seq")?;
        let ui_end: i64 = row.try_get("end_ui_seq")?;
        let (before, count): (i64, i64) = sqlx::query_as(&format!("SELECT COALESCE(SUM(seq<=?2),0),COALESCE(SUM(seq>?2 AND seq<=?3),0) FROM messages m WHERE frame_id=?1 AND role='user' AND tool_name IS NULL AND {}", super::context_epochs::LIVE_LOG_ROWS))
            .bind(frame_id).bind(start).bind(end).fetch_one(&mut *tx).await?;
        // Preserve the latest system prompt if rules/settings were refreshed
        // in a later epoch. No user/tool row is renumbered.
        sqlx::query("UPDATE messages SET content=(SELECT content FROM messages WHERE frame_id=?1 AND role='system' ORDER BY seq DESC LIMIT 1) WHERE frame_id=?1 AND epoch=?2 AND role='system' AND seq<=?3")
            .bind(frame_id).bind(base).bind(start).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM messages WHERE frame_id=?1 AND ((seq>?2 AND seq<=?3) OR EXISTS(SELECT 1 FROM context_epochs ce WHERE ce.frame_id=?1 AND ce.epoch>?4 AND messages.seq BETWEEN ce.first_seq AND ce.initial_head_seq))")
            .bind(frame_id).bind(start).bind(end).bind(base).execute(&mut *tx).await?;
        sqlx::query("UPDATE messages SET epoch=?2 WHERE frame_id=?1 AND epoch>?2")
            .bind(frame_id)
            .bind(base)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE frames SET head_epoch=? WHERE id=?")
            .bind(base)
            .bind(frame_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM session_ui_events WHERE frame_id=?1 AND ((seq>?2 AND seq<=?3) OR (json_extract(event_json,'$.kind') IN ('Compaction','CompactionUndone') AND CAST(json_extract(event_json,'$.epoch') AS INTEGER)>?4))")
            .bind(frame_id).bind(ui_start).bind(ui_end).bind(base).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM context_epochs WHERE frame_id=? AND epoch>?")
            .bind(frame_id)
            .bind(base)
            .execute(&mut *tx)
            .await?;
        for statement in [
            "DELETE FROM message_resource_links WHERE frame_id=?1 AND message_seq>?2 AND message_seq<=?3",
            "DELETE FROM turn_file_undo WHERE frame_id=?1 AND user_message_seq>?2 AND user_message_seq<=?3",
        ] {
            sqlx::query(statement).bind(frame_id).bind(start).bind(end).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM session_timer_turns WHERE frame_id=?")
            .bind(frame_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(wisp_dto::TimerTurnRemoval {
            frame_id: frame_id.into(),
            first_user_index: before as usize,
            user_count: count as usize,
            base_epoch: base,
        }))
    }

    pub async fn update_session_timer(
        &self,
        id: &str,
        prompt: &str,
        interval: i64,
        now: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("schedules", "id", id).await? {
            return Box::pin(store.update_session_timer(id, prompt, interval, now)).await;
        }
        let result = sqlx::query("UPDATE schedules SET prompt=?,name=?,interval_secs=?,next_run_at=?,updated_at=? WHERE id=? AND replace_previous_turn=1")
            .bind(prompt).bind(prompt.chars().take(80).collect::<String>()).bind(interval).bind(now + interval).bind(now).bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::Message;

    async fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("timer.sqlite")).await.unwrap();
        store.create_project("p", "project", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .append_message("f", 1, &Message::system("rules"))
            .await
            .unwrap();
        store
            .append_message("f", 2, &Message::user("original research task"))
            .await
            .unwrap();
        (dir, store)
    }

    async fn append(store: &Store, message: Message) -> i64 {
        let seq = store.max_message_seq("f").await.unwrap() + 1;
        store.append_message("f", seq, &message).await.unwrap();
        seq
    }

    async fn event(store: &Store, value: serde_json::Value) {
        let seq = store.next_session_ui_event_seq("f").await.unwrap();
        store
            .append_session_ui_event("f", seq, &value.to_string())
            .await
            .unwrap();
    }

    async fn compact(store: &Store, text: &str) -> i64 {
        let messages = vec![Message::system("rules"), Message::user(text)];
        store
            .open_context_epoch(
                "f",
                crate::OpenContextEpoch {
                    messages: &messages,
                    strategy: "auto",
                    kind: "semantic",
                    before_tokens: 100,
                    after_tokens: 20,
                    checkpoint_index: Some(1),
                    first_kept_seq: None,
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn timer_replacement_removes_tools_and_compaction_but_keeps_intervening_human_turns() {
        let (_dir, store) = fixture().await;
        event(
            &store,
            serde_json::json!({"kind":"User","text":"original research task"}),
        )
        .await;
        store.begin_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("timer check")).await;
        let mut tool_call = Message::assistant("");
        tool_call.tool_calls.push(serde_json::from_value(serde_json::json!({"id":"call-1","type":"function","function":{"name":"shell","arguments":"{}"}})).unwrap());
        append(&store, tool_call).await;
        append(&store, Message::tool("call-1", "shell", "old log")).await;
        event(
            &store,
            serde_json::json!({"kind":"User","text":"timer check"}),
        )
        .await;
        event(
            &store,
            serde_json::json!({"kind":"ToolResult","output":"old log"}),
        )
        .await;
        compact(&store, "summary with old timer result").await;
        store.finish_session_timer_turn("f").await.unwrap();
        let human_seq = append(&store, Message::user("keep my manual follow-up")).await;
        event(
            &store,
            serde_json::json!({"kind":"User","text":"keep my manual follow-up"}),
        )
        .await;
        let epoch = compact(&store, "summary with timer AND manual follow-up").await;
        event(
            &store,
            serde_json::json!({"kind":"Compaction","epoch":epoch}),
        )
        .await;
        let removal = store.clear_session_timer_turn("f").await.unwrap().unwrap();
        assert_eq!((removal.first_user_index, removal.user_count), (1, 1));
        let rows = store.load_messages_with_seq("f").await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|(_, m)| m.content.as_text())
                .collect::<Vec<_>>(),
            vec![
                "rules",
                "original research task",
                "keep my manual follow-up"
            ]
        );
        assert_eq!(rows.last().unwrap().0, human_seq, "human seq stays stable");
        assert!(store.context_epochs("f").await.unwrap().is_empty());
        let events = store.load_session_ui_events("f").await.unwrap().join("\n");
        assert!(
            !events.contains("timer check")
                && !events.contains("old log")
                && !events.contains("Compaction")
        );
        assert!(events.contains("keep my manual follow-up"));
        assert!(store.clear_session_timer_turn("f").await.unwrap().is_none());
        store.pool.close().await;
    }

    #[tokio::test]
    async fn timer_crash_recovery_seals_before_new_human_turn_and_survives_reopen() {
        let (dir, store) = fixture().await;
        store.begin_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("interrupted timer")).await;
        store.pool.close().await;
        let store = Store::open(&dir.path().join("timer.sqlite")).await.unwrap();
        store.finish_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("human after restart")).await;
        store.clear_session_timer_turn("f").await.unwrap();
        let rows = store.load_messages("f").await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].content.as_text(), "human after restart");
        store.pool.close().await;
    }

    #[tokio::test]
    async fn timer_rewind_cannot_reuse_ownership_to_delete_new_messages() {
        let (_dir, store) = fixture().await;
        store.begin_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("timer")).await;
        store.finish_session_timer_turn("f").await.unwrap();
        store.rewind_to_seq("f", 0, 2).await.unwrap();
        append(&store, Message::user("new human message at reused seq")).await;
        assert!(store.clear_session_timer_turn("f").await.unwrap().is_none());
        assert_eq!(store.load_messages("f").await.unwrap().len(), 3);
        store.pool.close().await;
    }

    #[tokio::test]
    async fn undoing_timer_compaction_cannot_delete_human_messages_at_reused_seqs() {
        let (_dir, store) = fixture().await;
        store.begin_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("timer")).await;
        compact(&store, "timer summary").await;
        store.finish_session_timer_turn("f").await.unwrap();
        store.undo_context_epoch("f").await.unwrap();
        append(&store, Message::user("human at a former snapshot seq")).await;
        store.clear_session_timer_turn("f").await.unwrap();
        let rows = store.load_messages("f").await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].content.as_text(), "human at a former snapshot seq");
        store.pool.close().await;
    }

    #[tokio::test]
    async fn timer_ui_indexes_exclude_internal_user_messages() {
        let (_dir, store) = fixture().await;
        let mut internal = Message::user("internal workflow completion");
        internal.tool_name = Some(crate::AGENT_WORKFLOW_COMPLETION_TOOL.into());
        append(&store, internal).await;
        store.begin_session_timer_turn("f").await.unwrap();
        append(&store, Message::user("timer")).await;
        store.finish_session_timer_turn("f").await.unwrap();
        let removal = store.clear_session_timer_turn("f").await.unwrap().unwrap();
        assert_eq!((removal.first_user_index, removal.user_count), (1, 1));
        assert_eq!(store.load_messages("f").await.unwrap().len(), 3);
        store.pool.close().await;
    }

    #[tokio::test]
    async fn timer_repeated_checks_remain_one_turn_and_session_delete_removes_schedule() {
        let (_dir, store) = fixture().await;
        let schedule = crate::ScheduleRecord {
            id: "timer".into(),
            project_id: "p".into(),
            frame_id: Some("f".into()),
            replace_previous_turn: true,
            prompt: "status".into(),
            interval_secs: 3600,
            enabled: true,
            next_run_at: 100,
            ..Default::default()
        };
        store.create_schedule(&schedule).await.unwrap();
        let mut duplicate = schedule.clone();
        duplicate.id = "duplicate".into();
        assert!(store.create_schedule(&duplicate).await.is_err());
        for round in 0..4 {
            store.clear_session_timer_turn("f").await.unwrap();
            store.begin_session_timer_turn("f").await.unwrap();
            append(&store, Message::user("status")).await;
            append(&store, Message::assistant(format!("result {round}"))).await;
            store.finish_session_timer_turn("f").await.unwrap();
            assert_eq!(store.load_messages("f").await.unwrap().len(), 4);
        }
        store.set_schedule_enabled("timer", false, 2).await.unwrap();
        store
            .update_session_timer("timer", "logs", 1800, 10)
            .await
            .unwrap();
        let updated = store.get_schedule("timer").await.unwrap().unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.next_run_at, 1810);
        store.delete_session("f", "p").await.unwrap();
        assert!(store.get_schedule("timer").await.unwrap().is_none());
        store.pool.close().await;
    }
}
