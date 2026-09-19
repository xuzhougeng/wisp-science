use wisp_llm::Message;
use wisp_store::{OpenContextEpoch, Store};

async fn setup() -> Store {
    let path =
        std::env::temp_dir().join(format!("epoch-composition-{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&path).await.unwrap();
    store.create_project("p", "test", "").await.unwrap();
    store
        .create_frame("f", "p", "OPERON", "fake")
        .await
        .unwrap();
    for (i, message) in [
        Message::system("sys"),
        Message::user("q1"),
        Message::assistant("a1"),
        Message::user("q2"),
        Message::assistant("a2"),
    ]
    .iter()
    .enumerate()
    {
        store
            .append_message("f", i as i64 + 1, message)
            .await
            .unwrap();
    }
    for (i, event) in [
        serde_json::json!({"kind":"User","frame_id":"f","text":"q1"}),
        serde_json::json!({"kind":"MessageBoundary","frame_id":"f","seq":3}),
        serde_json::json!({"kind":"User","frame_id":"f","text":"q2"}),
        serde_json::json!({"kind":"MessageBoundary","frame_id":"f","seq":5}),
    ]
    .iter()
    .enumerate()
    {
        store
            .append_session_ui_event("f", i as i64 + 1, &event.to_string())
            .await
            .unwrap();
    }
    store
}

async fn compact(store: &Store, kept: i64) -> i64 {
    let messages = vec![
        Message::system("sys"),
        Message::user("[context summary checkpoint]\nsummary"),
        Message::user("q2"),
        Message::assistant("a2"),
    ];
    store
        .open_context_epoch(
            "f",
            OpenContextEpoch {
                messages: &messages,
                strategy: "manual",
                kind: "semantic",
                before_tokens: 1000,
                after_tokens: 200,
                checkpoint_index: Some(1),
                first_kept_seq: Some(kept),
                archive_ref: None,
                ui_event_seq: None,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn undo_then_recompact_must_not_inherit_the_old_undone_identity() {
    let store = setup().await;
    let first = compact(&store, 4).await;
    store.undo_context_epoch("f").await.unwrap();
    store
        .append_session_ui_event(
            "f",
            5,
            &serde_json::json!({"kind":"CompactionUndone","frame_id":"f","epoch":first})
                .to_string(),
        )
        .await
        .unwrap();
    let second = compact(&store, 4).await;
    let undone = store.undone_context_epochs("f").await.unwrap();
    assert!(
        !undone.contains(&second),
        "new active epoch {second} collides with retained CompactionUndone events {undone:?}"
    );
}

#[tokio::test]
async fn second_compaction_retained_tail_must_still_resolve_to_the_original_visual_turn() {
    let store = setup().await;
    compact(&store, 4).await;
    let copied_q2_seq = store.load_messages_with_seq("f").await.unwrap()[2].0;
    compact(&store, copied_q2_seq).await;
    assert_eq!(
        store
            .visual_user_index_for_kept_seq("f", copied_q2_seq)
            .await
            .unwrap(),
        Some(1),
        "retained tail copied through epoch 1 must still identify q2"
    );
}

#[tokio::test]
async fn epoch_identity_survives_undo_without_an_event_and_rewind() {
    let store = setup().await;
    assert_eq!(compact(&store, 4).await, 1);
    store.undo_context_epoch("f").await.unwrap();
    assert_eq!(compact(&store, 4).await, 2);
    store.rewind_to_seq("f", 0, 5).await.unwrap();
    assert_eq!(compact(&store, 4).await, 3);
    store
        .replace_messages("f", &[Message::system("sys"), Message::user("new")])
        .await
        .unwrap();
    assert_eq!(compact(&store, 2).await, 4);
}

#[tokio::test]
async fn legacy_undone_event_reserves_its_identity_on_import() {
    let store = setup().await;
    store
        .append_session_ui_event(
            "f",
            5,
            &serde_json::json!({"kind":"CompactionUndone","frame_id":"f","epoch":42}).to_string(),
        )
        .await
        .unwrap();
    assert_eq!(compact(&store, 4).await, 43);
}

#[tokio::test]
async fn automatic_compaction_event_gets_its_durable_epoch_after_flush() {
    let store = setup().await;
    store.append_session_ui_event("f", 5,
        r#"{"kind":"Compaction","frame_id":"f","before":1000,"after":200,"strategy":"auto","epoch":null}"#).await.unwrap();
    let epoch = compact(&store, 4).await;
    store
        .set_context_epoch_ui_event("f", epoch, 5)
        .await
        .unwrap();
    let events = store.load_session_ui_events("f").await.unwrap();
    let event: serde_json::Value = serde_json::from_str(events.last().unwrap()).unwrap();
    assert_eq!(event["epoch"], epoch);
    assert_eq!(event["strategy"], "auto");
}

#[tokio::test]
async fn copied_tail_origin_does_not_jump_to_a_later_live_turn() {
    let store = setup().await;
    compact(&store, 4).await;
    let copied_q2_seq = store.load_messages_with_seq("f").await.unwrap()[2].0;
    store
        .append_message("f", 10, &Message::user("q3"))
        .await
        .unwrap();
    store
        .append_message("f", 11, &Message::assistant("a3"))
        .await
        .unwrap();
    store
        .append_session_ui_event("f", 5, r#"{"kind":"User","frame_id":"f","text":"q3"}"#)
        .await
        .unwrap();
    store
        .append_session_ui_event(
            "f",
            6,
            r#"{"kind":"MessageBoundary","frame_id":"f","seq":11}"#,
        )
        .await
        .unwrap();
    compact(&store, copied_q2_seq).await;
    assert_eq!(
        store
            .visual_user_index_for_kept_seq("f", copied_q2_seq)
            .await
            .unwrap(),
        Some(1)
    );
}

#[tokio::test]
async fn later_retained_turn_resolves_through_semantic_and_prune_epochs() {
    let store = setup().await;
    let original = store.load_messages("f").await.unwrap();
    let mut messages = vec![
        original[0].clone(),
        Message::user("[context summary checkpoint]\nsummary"),
    ];
    // Real compaction copies the retained messages, including their timestamps.
    messages.extend_from_slice(&original[1..]);
    store
        .open_context_epoch(
            "f",
            OpenContextEpoch {
                messages: &messages,
                strategy: "manual",
                kind: "semantic",
                before_tokens: 1000,
                after_tokens: 200,
                checkpoint_index: Some(1),
                first_kept_seq: Some(2),
                archive_ref: None,
                ui_event_seq: None,
            },
        )
        .await
        .unwrap();
    store
        .open_context_epoch(
            "f",
            OpenContextEpoch {
                messages: &messages,
                strategy: "auto",
                kind: "prune_only",
                before_tokens: 200,
                after_tokens: 150,
                checkpoint_index: Some(1),
                first_kept_seq: None,
                archive_ref: None,
                ui_event_seq: None,
            },
        )
        .await
        .unwrap();
    let copied_q2_seq = store.load_messages_with_seq("f").await.unwrap()[4].0;
    assert_eq!(
        store
            .visual_user_index_for_kept_seq("f", copied_q2_seq)
            .await
            .unwrap(),
        Some(1)
    );
}

#[tokio::test]
async fn semantic_tail_mapping_handles_skipped_turns_and_rejects_duplicate_anchors() {
    for duplicate in [false, true] {
        let store = setup().await;
        let original = store.load_messages("f").await.unwrap();
        let last = if duplicate {
            original[3].clone()
        } else {
            Message::user("q3")
        };
        store.append_message("f", 6, &last).await.unwrap();
        store
            .append_session_ui_event(
                "f",
                5,
                &serde_json::json!({"kind":"User","frame_id":"f","text":last.content.as_text()})
                    .to_string(),
            )
            .await
            .unwrap();
        store
            .append_session_ui_event(
                "f",
                6,
                r#"{"kind":"MessageBoundary","frame_id":"f","seq":6}"#,
            )
            .await
            .unwrap();
        // The semantic fold keeps q1 and the last request but skips q2.
        let messages = vec![
            original[0].clone(),
            Message::user("[context summary checkpoint]\nsummary"),
            original[1].clone(),
            last,
        ];
        store
            .open_context_epoch(
                "f",
                OpenContextEpoch {
                    messages: &messages,
                    strategy: "manual",
                    kind: "semantic",
                    before_tokens: 1000,
                    after_tokens: 200,
                    checkpoint_index: Some(1),
                    first_kept_seq: Some(2),
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap();
        let last_seq = store.load_messages_with_seq("f").await.unwrap()[3].0;
        assert_eq!(
            store
                .visual_user_index_for_kept_seq("f", last_seq)
                .await
                .unwrap(),
            if duplicate { None } else { Some(2) }
        );
    }
}
