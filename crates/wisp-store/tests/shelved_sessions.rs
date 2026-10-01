use wisp_dto::ResearchArchive;
use wisp_llm::Message;
use wisp_store::Store;

async fn conversation(store: &Store, id: &str, title: &str) {
    store.create_frame(id, "p", "OPERON", "fake").await.unwrap();
    store.rename_session(id, "p", title).await.unwrap();
    store
        .append_message(id, 1, &Message::user("preserved needle transcript"))
        .await
        .unwrap();
}

#[tokio::test]
async fn shelving_filters_discovery_preserves_references_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let store = Store::open(&path).await.unwrap();
    store.create_project("p", "Research", "").await.unwrap();
    store.create_project("other", "Other", "").await.unwrap();
    conversation(&store, "f", "Temporary analysis").await;
    store.set_session_pinned("f", "p", true).await.unwrap();
    let before = store.list_sessions("p").await.unwrap();
    assert!(store.set_session_shelved("f", "other", true).await.is_err());
    assert!(store
        .set_session_shelved("missing", "p", true)
        .await
        .is_err());
    assert!(!store.session_is_shelved("f").await.unwrap());
    store.set_session_shelved("f", "p", true).await.unwrap();
    store.set_session_shelved("f", "p", true).await.unwrap();
    assert!(store
        .list_sessions_page("p", None, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(store.list_pinned_sessions("p").await.unwrap().is_empty());
    assert!(store
        .list_recent_sessions_detail(100)
        .await
        .unwrap()
        .is_empty());
    assert!(store.latest_used_session_id("p").await.unwrap().is_none());
    for project in [None, Some("p")] {
        for query in ["", "Temporary", "needle"] {
            assert!(store
                .search_sessions(project, query, 100, None, None)
                .await
                .unwrap()
                .is_empty());
        }
    }
    // Existing # links and ownership/export enumeration remain valid.
    assert!(store.last_user_message_session().await.unwrap().is_none());
    assert!(store.get_session_reference("f").await.unwrap().is_some());
    assert_eq!(store.list_sessions("p").await.unwrap(), before);
    assert_eq!(store.load_messages("f").await.unwrap().len(), 1);
    assert_eq!(
        store
            .list_sessions_page_with_visibility("p", None, 100, Some(true), "needle")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .list_sessions_page_with_visibility("p", None, 100, Some(true), "missing")
        .await
        .unwrap()
        .is_empty());

    let reopened = Store::open(&path).await.unwrap();
    assert!(reopened.session_is_shelved("f").await.unwrap());
    assert_eq!(
        reopened
            .schema_migrations()
            .await
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == "0061_session_shelved")
            .count(),
        1
    );
    reopened.set_session_shelved("f", "p", false).await.unwrap();
    assert_eq!(
        reopened.list_sessions_page("p", None, 100).await.unwrap(),
        before
    );
    assert_eq!(reopened.list_pinned_sessions("p").await.unwrap().len(), 1);
    assert_eq!(
        reopened
            .search_sessions(None, "needle", 100, None, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn visibility_is_filtered_before_pagination_and_independent_of_research_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
    store.create_project("p", "Research", "").await.unwrap();
    for id in ["a", "b", "c", "d"] {
        conversation(&store, id, "Page example").await;
    }
    store.set_session_shelved("b", "p", true).await.unwrap();
    store.set_session_shelved("d", "p", true).await.unwrap();
    for shelved in [true, false] {
        let all = store
            .list_sessions_page_with_visibility("p", None, 100, Some(shelved), "Page")
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        let first = store
            .list_sessions_page_with_visibility("p", None, 1, Some(shelved), "Page")
            .await
            .unwrap();
        let last = &first[0];
        let next = store
            .list_sessions_page_with_visibility(
                "p",
                Some((last.2, &last.0)),
                1,
                Some(shelved),
                "Page",
            )
            .await
            .unwrap();
        assert_eq!([first, next].concat(), all);
    }
    let mut archive = ResearchArchive {
        id: "archive".into(),
        project_id: "p".into(),
        frame_id: "a".into(),
        title: "Research milestone".into(),
        source_hash: store.research_archive_source("a").await.unwrap().1,
        ..Default::default()
    };
    store.save_research_archive_draft(&archive).await.unwrap();
    archive.frozen_at = Some(1);
    store.freeze_research_archive(&archive).await.unwrap();
    store.set_session_shelved("a", "p", true).await.unwrap();
    assert_eq!(
        store.research_archive("a").await.unwrap(),
        Some(archive.clone())
    );
    // Complete project export/import retains the display preference and lock.
    let bundle = dir.path().join("bundle.sqlite");
    store.export_project_database("p", &bundle).await.unwrap();
    let imported = Store::open(&dir.path().join("imported.sqlite"))
        .await
        .unwrap();
    imported
        .import_project_database(&bundle, "p", &dir.path().join("workspace"))
        .await
        .unwrap();
    assert!(imported.session_is_shelved("a").await.unwrap());
    imported.set_session_shelved("a", "p", false).await.unwrap();
    assert!(imported.require_unarchived_session("a").await.is_err());
    assert_eq!(imported.research_archive("a").await.unwrap(), Some(archive));
}

#[tokio::test]
async fn shelving_routes_to_portable_projects_and_legacy_read_only_databases_remain_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let store = Store::open_application(&path).await.unwrap();
    store
        .set_setting("decentralized_project_storage", "true")
        .await
        .unwrap();
    let workspace = dir.path().join("project");
    store
        .create_project("p", "Portable", workspace.to_str().unwrap())
        .await
        .unwrap();
    conversation(&store, "portable", "Portable conversation").await;
    store
        .set_session_shelved("portable", "p", true)
        .await
        .unwrap();
    assert!(store
        .list_sessions_page("p", None, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .search_sessions(None, "Portable", 100, None, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .list_sessions_page_with_visibility("p", None, 100, Some(true), "")
            .await
            .unwrap()
            .len(),
        1
    );
    let reopened = Store::open_application(&path).await.unwrap();
    assert!(reopened.session_is_shelved("portable").await.unwrap());
    reopened
        .set_session_shelved("portable", "p", false)
        .await
        .unwrap();
    assert_eq!(
        reopened
            .search_sessions(None, "Portable", 100, None, None)
            .await
            .unwrap()
            .len(),
        1
    );

    // Simulate a pre-feature database without running its migration on read.
    let old_path = dir.path().join("legacy.sqlite");
    let old = Store::open(&old_path).await.unwrap();
    old.create_project("p", "Legacy", "").await.unwrap();
    conversation(&old, "legacy", "Legacy conversation").await;
    let connection = sqlx::SqlitePool::connect(&format!("sqlite:{}", old_path.display()))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE frames DROP COLUMN shelved")
        .execute(&connection)
        .await
        .unwrap();
    let read_only = Store::open_read_only(&old_path).await.unwrap();
    assert!(!read_only.session_is_shelved("legacy").await.unwrap());
    assert_eq!(
        read_only
            .list_sessions_page("p", None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(read_only
        .list_sessions_page_with_visibility("p", None, 10, Some(true), "")
        .await
        .unwrap()
        .is_empty());
    // Reopening writable repairs the column even with an existing marker.
    let repaired = Store::open(&old_path).await.unwrap();
    repaired
        .set_session_shelved("legacy", "p", true)
        .await
        .unwrap();
    assert!(repaired.session_is_shelved("legacy").await.unwrap());
}

#[tokio::test]
async fn an_explicitly_shelved_empty_draft_can_be_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
    store.create_project("p", "Research", "").await.unwrap();
    store
        .create_frame("draft", "p", "OPERON", "fake")
        .await
        .unwrap();
    store.set_session_shelved("draft", "p", true).await.unwrap();
    let hidden = store
        .list_sessions_page_with_visibility("p", None, 10, Some(true), "")
        .await
        .unwrap();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].0, "draft");
    assert_eq!(store.list_sessions("p").await.unwrap().len(), 1);
    store
        .set_session_shelved("draft", "p", false)
        .await
        .unwrap();
    assert!(!store.session_is_shelved("draft").await.unwrap());
    assert!(store.load_messages("draft").await.unwrap().is_empty());
}
