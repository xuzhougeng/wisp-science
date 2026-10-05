use wisp_llm::Message;
use wisp_store::Store;

async fn verify(routed: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.sqlite");
    let store = if routed {
        let store = Store::open_application(&path).await.unwrap();
        store
            .set_setting("decentralized_project_storage", "true")
            .await
            .unwrap();
        store
    } else {
        Store::open(&path).await.unwrap()
    };
    for project in ["current", "a-body", "z-title", "hidden"] {
        store
            .create_project(project, project, dir.path().join(project).to_str().unwrap())
            .await
            .unwrap();
    }
    // A title hit beyond the display-title truncation must still outrank a
    // newer body-only hit, regardless of alphabetical project ordering.
    for (id, project, title) in [
        (
            "title",
            "z-title",
            format!("{} needle", "long title ".repeat(30)),
        ),
        ("body", "a-body", "ordinary title".into()),
        ("preferred", "current", "preferred ordinary title".into()),
    ] {
        store
            .create_frame(id, project, "OPERON", "fake")
            .await
            .unwrap();
        store
            .append_message(id, 1, &Message::user(title))
            .await
            .unwrap();
        store
            .append_message(id, 2, &Message::assistant("needle in body"))
            .await
            .unwrap();
    }
    // More hidden hits than the search cap may never exhaust visible slots.
    for index in 0..15 {
        let id = format!("hidden-{index}");
        store
            .create_frame(&id, "hidden", "OPERON", "fake")
            .await
            .unwrap();
        store
            .append_message(&id, 1, &Message::user("needle hidden title"))
            .await
            .unwrap();
    }
    let excluded = ["hidden".to_string()];
    for preferred in [None, Some("current")] {
        let all = store
            .search_sessions(None, "needle", 100, None, preferred)
            .await
            .unwrap();
        let expected: Vec<_> = all
            .into_iter()
            .filter(|s| s.project_id != "hidden")
            .take(3)
            .collect();
        let actual = store
            .search_sessions_excluding_projects(None, "needle", 3, None, preferred, &excluded)
            .await
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 3);
        assert_eq!(
            actual[0].id,
            if preferred.is_some() {
                "preferred"
            } else {
                "title"
            }
        );
        if preferred.is_some() {
            assert_eq!(actual[1].id, "title");
        }
    }
    assert!(store
        .search_sessions_excluding_projects(Some("hidden"), "needle", 3, None, None, &excluded)
        .await
        .unwrap()
        .is_empty());
    store
        .set_session_shelved("title", "z-title", true)
        .await
        .unwrap();
    let visible = store
        .search_sessions_excluding_projects(None, "needle", 3, None, None, &excluded)
        .await
        .unwrap();
    assert_eq!(visible.len(), 2);
    assert!(visible.iter().all(|s| s.id != "title"));
}

#[tokio::test]
async fn visible_session_search_preserves_local_ranking_before_limit() {
    verify(false).await;
}

#[tokio::test]
async fn visible_session_search_preserves_routed_ranking_before_limit() {
    verify(true).await;
}
