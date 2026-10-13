use wisp_llm::{Content, Message};
use wisp_store::{OpenContextEpoch, Store};

async fn fixture(routed: bool) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("search.sqlite");
    let store = if routed {
        let store = Store::open_application(&db).await.unwrap();
        store
            .set_setting("decentralized_project_storage", "true")
            .await
            .unwrap();
        store
    } else {
        Store::open(&db).await.unwrap()
    };
    for project in ["project", "other"] {
        store
            .create_project(project, project, dir.path().join(project).to_str().unwrap())
            .await
            .unwrap();
    }
    (dir, store)
}

async fn saved(store: &Store, frame: &str, seq: i64, mut message: Message) {
    // Deliberately tie timestamps to exercise the complete paging key.
    message.ts = 100;
    store.append_message(frame, seq, &message).await.unwrap();
}

async fn verify_project_history(routed: bool) {
    let (_dir, store) = fixture(routed).await;
    for (id, project) in [("a", "project"), ("z", "project"), ("foreign", "other")] {
        store
            .create_frame(id, project, "OPERON", "fake")
            .await
            .unwrap();
        saved(&store, id, 1, Message::user("Opening question")).await;
        saved(
            &store,
            id,
            2,
            Message::assistant("A NeEdLe in an old answer"),
        )
        .await;
    }
    store
        .rename_session("a", "project", "Named conversation")
        .await
        .unwrap();
    store
        .set_session_shelved("z", "project", true)
        .await
        .unwrap();
    // History well beyond the initial transcript window is still searched.
    for seq in 3..125 {
        saved(
            &store,
            "a",
            seq,
            Message::user(format!("later question {seq}")),
        )
        .await;
    }
    saved(
        &store,
        "z",
        3,
        Message::tool("call", "read", "needle in tool output"),
    )
    .await;
    saved(
        &store,
        "z",
        4,
        Message::system("needle hidden system prompt"),
    )
    .await;
    store
        .create_child_frame("internal", "a", "project", "OPERON", "fake")
        .await
        .unwrap();
    saved(
        &store,
        "internal",
        1,
        Message::user("needle internal execution"),
    )
    .await;

    let first = store
        .search_project_chats("project", "NEEDLE", None, 2)
        .await
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|hit| (hit.session_id.as_str(), hit.message_seq))
            .collect::<Vec<_>>(),
        [("z", 3), ("z", 2)]
    );
    assert_eq!(first.items[0].role, "tool");
    assert_eq!(first.items[1].matched, "NeEdLe");
    let second = store
        .search_project_chats("project", "needle", first.next_cursor.as_ref(), 2)
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].session_id, "a");
    assert_eq!(second.items[0].session_title, "Named conversation");
    assert_eq!(second.items[0].message_seq, 2);
    assert!(second.next_cursor.is_none());
    let target = store
        .locate_project_chat_match("project", "a", 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.user_index, 0);
    assert_eq!(target.before_seq, Some(3));
    assert!(store
        .locate_project_chat_match("project", "foreign", 2)
        .await
        .is_err());
    assert!(store
        .locate_project_chat_match("project", "a", 999)
        .await
        .is_err());
    assert!(store
        .search_project_chats("project", "   ", None, 50)
        .await
        .unwrap()
        .items
        .is_empty());
    assert!(store
        .search_project_chats("project", "absent", None, 50)
        .await
        .unwrap()
        .items
        .is_empty());
    assert!(store
        .search_project_chats("project", &"字".repeat(257), None, 50)
        .await
        .is_err());
}

#[tokio::test]
async fn project_history_is_scoped_paged_and_includes_shelved_messages() {
    verify_project_history(false).await;
}

#[tokio::test]
async fn project_history_routes_to_the_owning_database() {
    verify_project_history(true).await;
}

#[tokio::test]
async fn keywords_are_literal_decoded_text_and_snippets_keep_unicode_boundaries() {
    let (_dir, store) = fixture(false).await;
    store
        .create_frame("a", "project", "OPERON", "fake")
        .await
        .unwrap();
    saved(&store, "a", 1, Message::user("ordinary question")).await;
    let text = format!(
        "{}中文 100%_done C:\\data \"quoted\"\nnext{}",
        "前".repeat(100),
        "后".repeat(200)
    );
    let mut message = Message::user("");
    message.content = serde_json::from_value::<Content>(serde_json::json!([
        {"type":"text", "text": text},
        {"type":"image_url", "image_url":{"url":"data:image/png;base64,hidden-image-needle"}}
    ]))
    .unwrap();
    message.reasoning = Some("hidden-reasoning-needle".into());
    saved(&store, "a", 2, message).await;
    for query in ["%_", "C:\\data", "\"quoted\"\nnext", "中文"] {
        let page = store
            .search_project_chats("project", query, None, 50)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1, "{query}");
        let hit = &page.items[0];
        assert_eq!(hit.matched, query);
        assert!(hit.before.starts_with('…'));
        assert!(hit.after.ends_with('…'));
        assert!(hit.before.chars().count() <= 81);
        assert!(hit.after.chars().count() <= 161);
    }
    for query in [
        "hidden-image-needle",
        "hidden-reasoning-needle",
        "image_url",
        "dataX",
        "100%missing",
    ] {
        assert!(
            store
                .search_project_chats("project", query, None, 50)
                .await
                .unwrap()
                .items
                .is_empty(),
            "{query}"
        );
    }
}

#[tokio::test]
async fn compaction_keeps_old_matches_without_counting_copied_tails_or_checkpoints() {
    let (_dir, store) = fixture(false).await;
    store
        .create_frame("a", "project", "OPERON", "fake")
        .await
        .unwrap();
    for (seq, message) in [
        (1, Message::system("system")),
        (2, Message::user("needle old question")),
        (3, Message::assistant("needle old answer")),
        (4, Message::user("needle retained question")),
        (5, Message::assistant("needle retained answer")),
    ] {
        saved(&store, "a", seq, message).await;
    }
    for (event_seq, event) in [
        (
            1,
            serde_json::json!({"kind":"User", "frame_id":"a", "text":"needle old question"}),
        ),
        (
            2,
            serde_json::json!({"kind":"MessageBoundary", "frame_id":"a", "seq":2}),
        ),
        (
            3,
            serde_json::json!({"kind":"User", "frame_id":"a", "text":"needle retained question"}),
        ),
        (
            4,
            serde_json::json!({"kind":"MessageBoundary", "frame_id":"a", "seq":4}),
        ),
    ] {
        store
            .append_session_ui_event("a", event_seq, &event.to_string())
            .await
            .unwrap();
    }
    let compacted = [
        Message::system("system"),
        Message::user("[context summary checkpoint]\nneedle summary"),
        Message::user("needle retained question"),
        Message::assistant("needle retained answer"),
    ];
    store
        .open_context_epoch(
            "a",
            OpenContextEpoch {
                messages: &compacted,
                strategy: "manual",
                kind: "semantic",
                before_tokens: 1000,
                after_tokens: 200,
                checkpoint_index: Some(1),
                first_kept_seq: Some(4),
                archive_ref: None,
                ui_event_seq: None,
            },
        )
        .await
        .unwrap();
    let page = store
        .search_project_chats("project", "needle", None, 50)
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|hit| hit.message_seq)
            .collect::<Vec<_>>(),
        [5, 4, 3, 2]
    );
    assert!(store
        .load_session_outline("a")
        .await
        .unwrap()
        .iter()
        .all(|entry| entry.seq.is_none()));
    let target = store
        .locate_project_chat_match("project", "a", 3)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.user_index, 0);
    assert_eq!(target.before_seq, Some(4));
}

#[tokio::test]
async fn locator_handles_reference_enriched_turns_and_rejects_ambiguous_legacy_positions() {
    let (_dir, store) = fixture(false).await;
    store
        .create_frame("a", "project", "OPERON", "fake")
        .await
        .unwrap();
    saved(
        &store,
        "a",
        1,
        Message::user("Question\n[reference context]"),
    )
    .await;
    saved(&store, "a", 2, Message::assistant("answer")).await;
    store
        .append_session_ui_event(
            "a",
            1,
            &serde_json::json!({"kind":"User","text":"Question"}).to_string(),
        )
        .await
        .unwrap();
    let target = store
        .locate_project_chat_match("project", "a", 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.user_index, 0);
    assert_eq!(target.before_seq, None);

    store
        .create_frame("legacy", "project", "OPERON", "fake")
        .await
        .unwrap();
    saved(&store, "legacy", 1, Message::user("again")).await;
    saved(&store, "legacy", 2, Message::assistant("answer")).await;
    for (index, text) in ["again", "middle", "again"].into_iter().enumerate() {
        store
            .append_session_ui_event(
                "legacy",
                index as i64 + 1,
                &serde_json::json!({"kind":"User", "text":text}).to_string(),
            )
            .await
            .unwrap();
    }
    assert!(store
        .locate_project_chat_match("project", "legacy", 2)
        .await
        .unwrap()
        .is_none());
}
