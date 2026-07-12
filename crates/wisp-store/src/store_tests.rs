use super::*;

#[tokio::test]
async fn roundtrip() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_test_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "proj", "").await.unwrap();
    store
        .create_frame("f1", "p1", "OPERON", "test-model")
        .await
        .unwrap();
    store
        .append_message("f1", 0, &Message::system("hi"))
        .await
        .unwrap();
    store
        .append_message("f1", 1, &Message::user("hello"))
        .await
        .unwrap();
    let msgs = store.load_messages("f1").await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1].content.as_text(), "hello");
    let frames = store.list_root_frames("p1").await.unwrap();
    assert_eq!(frames.len(), 1);

    // list_sessions derives a title from the first user message and skips
    // frames with no user turn.
    store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
    store
        .append_message("f2", 0, &Message::system("only system"))
        .await
        .unwrap();
    let sessions = store.list_sessions("p1").await.unwrap();
    assert_eq!(sessions.len(), 1, "f2 has no user turn, must be excluded");
    assert_eq!(sessions[0].0, "f1");
    assert_eq!(sessions[0].1, "hello");
    store
        .rename_session("f1", "p1", "Renamed chat")
        .await
        .unwrap();
    let sessions = store.list_sessions("p1").await.unwrap();
    assert_eq!(sessions[0].1, "Renamed chat");
    store.delete_session("f1", "p1").await.unwrap();
    assert!(store.list_sessions("p1").await.unwrap().is_empty());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn multi_turn_append() {
    // Mirrors the Tauri wiring: a frame is created once, then messages are
    // appended across turns with incrementing seq; load_messages returns
    // them all in order.
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_multiturn_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    // Turn 1: system + user.
    store
        .append_message("f", 0, &Message::system("sys"))
        .await
        .unwrap();
    store
        .append_message("f", 1, &Message::user("hi"))
        .await
        .unwrap();
    let m1 = store.load_messages("f").await.unwrap();
    assert_eq!(m1.len(), 2);

    // Turn 2: assistant + tool result appended with seq 2,3.
    store
        .append_message("f", 2, &Message::assistant("hello"))
        .await
        .unwrap();
    store
        .append_message("f", 3, &Message::tool("c1", "read", "ok"))
        .await
        .unwrap();
    let m2 = store.load_messages("f").await.unwrap();
    assert_eq!(m2.len(), 4);
    assert_eq!(m2[0].content.as_text(), "sys");
    assert_eq!(m2[3].tool_name.as_deref(), Some("read"));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn global_composer_search_carries_project_and_session_metadata() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_composer_search_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("p1", "Alpha", "/tmp/alpha")
        .await
        .unwrap();
    store
        .create_project("p2", "Beta", "/tmp/beta")
        .await
        .unwrap();
    for (frame, project, title) in [("f1", "p1", "alpha result"), ("f2", "p2", "beta result")] {
        store
            .create_frame(frame, project, "OPERON", "m")
            .await
            .unwrap();
        store
            .append_message(frame, 1, &Message::user(title))
            .await
            .unwrap();
    }
    store
        .save_artifact(
            "a1",
            "p1",
            "f1",
            "alpha.csv",
            "text/csv",
            "/tmp/alpha/uploads/alpha.csv",
        )
        .await
        .unwrap();
    store
        .save_artifact(
            "a2",
            "p2",
            "f2",
            "beta.csv",
            "text/csv",
            "/tmp/beta/results/beta.csv",
        )
        .await
        .unwrap();

    let all = store.search_artifacts(None, "", 20, None).await.unwrap();
    assert_eq!(all.len(), 2);
    let alpha = all.iter().find(|a| a.id == "a1").unwrap();
    assert_eq!(alpha.project_name, "Alpha");
    assert_eq!(alpha.session_title, "alpha result");
    assert_eq!(alpha.origin, "upload");
    assert_eq!(
        store
            .search_artifacts(Some("p1"), "beta", 20, None)
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .search_artifacts(None, "beta", 20, None)
            .await
            .unwrap()[0]
            .id,
        "a2"
    );

    let sessions = store
        .search_sessions(None, "result", 20, None)
        .await
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        store
            .get_session_reference("f2")
            .await
            .unwrap()
            .unwrap()
            .project_name,
        "Beta"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn truncate_messages() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_store_trunc_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("a"))
        .await
        .unwrap();
    store
        .append_message("f", 2, &Message::assistant("b"))
        .await
        .unwrap();
    store
        .append_message("f", 3, &Message::user("c"))
        .await
        .unwrap();
    store.truncate_messages("f", 1).await.unwrap();
    let msgs = store.load_messages("f").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content.as_text(), "a");
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn session_reviews_are_upserted_and_truncated_with_the_transcript() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_review_test_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    store
        .upsert_session_review("f", "review-1", 2, r#"{"summary":"first"}"#)
        .await
        .unwrap();
    store
        .upsert_session_review("f", "review-1", 3, r#"{"summary":"verified"}"#)
        .await
        .unwrap();

    assert_eq!(
        store.load_session_reviews("f").await.unwrap(),
        vec![(2, r#"{"summary":"verified"}"#.into())]
    );

    store.truncate_messages("f", 1).await.unwrap();
    assert!(store.load_session_reviews("f").await.unwrap().is_empty());
}

#[tokio::test]
async fn session_ui_events_keep_insertion_order() {
    let tmp = std::env::temp_dir().join(format!("wisp_ui_events_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    assert_eq!(store.next_session_ui_event_seq("f").await.unwrap(), 1);
    let first = r#"{"kind":"MessageBoundary","frame_id":"f","seq":1}"#;
    let second = r#"{"kind":"MessageBoundary","frame_id":"f","seq":2}"#;
    store.append_session_ui_event("f", 1, first).await.unwrap();
    store.append_session_ui_event("f", 2, second).await.unwrap();
    assert_eq!(
        store.load_session_ui_events("f").await.unwrap(),
        vec![first, second]
    );
    assert_eq!(store.next_session_ui_event_seq("f").await.unwrap(), 3);
    store.truncate_messages("f", 1).await.unwrap();
    assert_eq!(
        store.load_session_ui_events("f").await.unwrap(),
        vec![first]
    );
}

#[tokio::test]
async fn project_crud_and_listing() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_proj_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();

    // create + get roundtrips workspace_dir
    store
        .create_project("a", "Alpha", "/tmp/alpha")
        .await
        .unwrap();
    store
        .create_project("b", "Beta", "/tmp/beta")
        .await
        .unwrap();
    assert_eq!(
        store.get_project("a").await.unwrap(),
        Some(("Alpha".into(), "/tmp/alpha".into()))
    );

    // one session under "a" (root frame with a user turn), none under "b"
    store.create_frame("f1", "a", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::user("hi"))
        .await
        .unwrap();

    let projs = store.list_projects().await.unwrap();
    assert_eq!(projs.len(), 2);
    // ordered by updated_at desc; "b" created last so it sorts first
    assert_eq!(projs[0].0, "b");
    let a = projs.iter().find(|p| p.0 == "a").unwrap();
    assert_eq!(a.5, 1, "project a has one session");
    let b = projs.iter().find(|p| p.0 == "b").unwrap();
    assert_eq!(b.5, 0, "project b has no sessions");

    // recent sessions span projects
    store.create_frame("f2", "b", "OPERON", "m").await.unwrap();
    store
        .append_message("f2", 1, &Message::user("yo"))
        .await
        .unwrap();
    let recent = store.list_recent_sessions(10).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert!(recent
        .iter()
        .any(|(_, pid, title, _)| pid == "a" && title == "hi"));

    // delete removes rows for "a" only, leaves "b"
    store.delete_project("a").await.unwrap();
    assert!(store.get_project("a").await.unwrap().is_none());
    assert!(store.load_messages("f1").await.unwrap().is_empty());
    assert!(store.get_project("b").await.unwrap().is_some());
    assert_eq!(store.load_messages("f2").await.unwrap().len(), 1);

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn recent_sessions_detail_last_role() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_store_recent_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();

    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::user("q"))
        .await
        .unwrap();
    store
        .append_message("f1", 2, &Message::assistant("done"))
        .await
        .unwrap();

    store.create_frame("f2", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f2", 1, &Message::user("only user"))
        .await
        .unwrap();

    let details = store.list_recent_sessions_detail(10).await.unwrap();
    let f1 = details.iter().find(|d| d.id == "f1").unwrap();
    assert_eq!(f1.last_role.as_deref(), Some("assistant"));
    let f2 = details.iter().find(|d| d.id == "f2").unwrap();
    assert_eq!(f2.last_role.as_deref(), Some("user"));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn recent_sessions_detail_respects_limit() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_recent_lim_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for i in 0..7 {
        let fid = format!("f{i}");
        store.create_frame(&fid, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(&fid, 1, &Message::user(&format!("msg {i}")))
            .await
            .unwrap();
    }
    let recent = store.list_recent_sessions_detail(5).await.unwrap();
    assert_eq!(recent.len(), 5);
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_adds_folder_id_on_legacy_db() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_store_legacy_{}.sqlite", uuid::Uuid::new_v4()));
    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        // Pre-folder schema: frames without folder_id, no folders table.
        sqlx::query(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, description TEXT, \
             workspace_dir TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE frames (id TEXT PRIMARY KEY, parent_frame_id TEXT, root_frame_id TEXT, \
             agent_name TEXT NOT NULL, status TEXT NOT NULL, project_id TEXT, model TEXT, \
             input_tokens INTEGER, output_tokens INTEGER, created_at INTEGER NOT NULL, \
             updated_at INTEGER NOT NULL, completed_at INTEGER, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id TEXT PRIMARY KEY, frame_id TEXT NOT NULL, seq INTEGER NOT NULL, \
             role TEXT NOT NULL, content TEXT, tool_calls TEXT, tool_call_id TEXT, tool_name TEXT, \
             reasoning TEXT, ts INTEGER NOT NULL, model_name TEXT, UNIQUE(frame_id, seq))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::user("legacy"))
        .await
        .unwrap();
    let sessions = store.list_sessions("p").await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].3.is_none());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn folder_crud_and_move() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_store_folder_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::user("in folder"))
        .await
        .unwrap();
    store.create_frame("f2", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f2", 1, &Message::user("ungrouped"))
        .await
        .unwrap();

    store.create_folder("d1", "p", "Research").await.unwrap();
    let folders = store.list_folders("p").await.unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].1, "Research");

    store
        .move_session_to_folder("f1", "p", Some("d1"))
        .await
        .unwrap();
    let sessions = store.list_sessions("p").await.unwrap();
    let f1 = sessions.iter().find(|s| s.0 == "f1").unwrap();
    assert_eq!(f1.3.as_deref(), Some("d1"));
    let f2 = sessions.iter().find(|s| s.0 == "f2").unwrap();
    assert!(f2.3.is_none());

    store.rename_folder("d1", "p", "Analysis").await.unwrap();
    let folders = store.list_folders("p").await.unwrap();
    assert_eq!(folders[0].1, "Analysis");

    store.delete_folder("d1", "p").await.unwrap();
    assert!(store.list_folders("p").await.unwrap().is_empty());
    let sessions = store.list_sessions("p").await.unwrap();
    let f1 = sessions.iter().find(|s| s.0 == "f1").unwrap();
    assert!(f1.3.is_none(), "session kept after folder delete");

    store.move_session_to_folder("f1", "p", None).await.unwrap();
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn execution_context_id_parsing_and_serialization() {
    assert_eq!(
        ExecutionContextKind::from_id("local").unwrap(),
        ExecutionContextKind::Local
    );
    assert_eq!(
        ExecutionContextKind::from_id("ssh:gpu-server").unwrap(),
        ExecutionContextKind::Ssh
    );
    assert_eq!(
        ExecutionContextKind::from_id("wsl:Ubuntu-22.04").unwrap(),
        ExecutionContextKind::Wsl
    );

    for bad in ["", " local", "ssh:", "wsl:", "ssh:gpu host", "docker:lab"] {
        assert!(
            ExecutionContextKind::from_id(bad).is_err(),
            "{bad:?} should be rejected"
        );
    }

    let ctx = ExecutionContext::new("ssh:gpu-server", "GPU server").unwrap();
    let json = serde_json::to_value(&ctx).unwrap();
    assert_eq!(json["id"], "ssh:gpu-server");
    assert_eq!(json["kind"], "ssh");
    assert_eq!(json["config_json"], "{}");
    assert_eq!(json["capabilities_json"], "{}");
}

#[tokio::test]
async fn execution_context_store_roundtrip() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_context_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();

    let mut ctx = ExecutionContext::new("ssh:gpu-server", "GPU server").unwrap();
    ctx.config_json = r#"{"alias":"gpu-server"}"#.into();
    ctx.capabilities_json = r#"{"gpu_summary":"A100"}"#.into();
    ctx.last_probe_at = Some(123);
    ctx.last_probe_status = Some("ok".into());
    store.upsert_execution_context(&ctx).await.unwrap();

    let got = store
        .get_execution_context("ssh:gpu-server")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.id, "ssh:gpu-server");
    assert_eq!(got.kind, ExecutionContextKind::Ssh);
    assert_eq!(got.label, "GPU server");
    assert_eq!(got.config_json, r#"{"alias":"gpu-server"}"#);
    assert_eq!(got.capabilities_json, r#"{"gpu_summary":"A100"}"#);
    assert_eq!(got.last_probe_at, Some(123));
    assert_eq!(got.last_probe_status.as_deref(), Some("ok"));
    assert!(got.last_probe_error.is_none());

    let mut updated = got.clone();
    updated.label = "Updated GPU".into();
    updated.last_probe_status = Some("error".into());
    updated.last_probe_error = Some("ssh failed".into());
    store.upsert_execution_context(&updated).await.unwrap();

    let list = store.list_execution_contexts().await.unwrap();
    assert_eq!(list.len(), 2);
    let ssh = list.iter().find(|ctx| ctx.id == "ssh:gpu-server").unwrap();
    assert_eq!(ssh.label, "Updated GPU");
    assert_eq!(ssh.last_probe_error.as_deref(), Some("ssh failed"));

    store
        .delete_execution_context("ssh:gpu-server")
        .await
        .unwrap();
    assert!(store
        .get_execution_context("ssh:gpu-server")
        .await
        .unwrap()
        .is_none());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn store_open_records_migrations_and_seeds_local_context() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_migrations_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();

    assert!(store
        .get_execution_context("local")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store.schema_migrations().await.unwrap(),
        vec![
            INITIAL_SCHEMA_MIGRATION.to_string(),
            CONTROL_PLANE_MIGRATION.to_string(),
            ARTIFACT_LINEAGE_MIGRATION.to_string(),
            SSH_RUN_CONTROL_MIGRATION.to_string(),
            RUN_LIFECYCLE_LEASE_MIGRATION.to_string(),
            PROPOSED_PLANS_MIGRATION.to_string(),
            CODEX_TURN_CONFIGS_MIGRATION.to_string(),
            ACP_SESSIONS_MIGRATION.to_string(),
            LAB_REGISTRY_MIGRATION.to_string(),
            SESSION_REVIEWS_MIGRATION.to_string(),
            LAB_TRANSACTION_MIGRATION.to_string(),
            SESSION_UI_EVENTS_MIGRATION.to_string(),
            LAB_RESOURCE_DEFINITION_MIGRATION.to_string(),
            LAB_INVENTORY_MIGRATION.to_string(),
            LAB_LOCATIONS_MIGRATION.to_string(),
            LAB_DOCUMENTS_MIGRATION.to_string(),
            LAB_WET_RUNS_MIGRATION.to_string(),
            LAB_PROTOCOL_REVISIONS_MIGRATION.to_string(),
            LAB_RUN_PARTICIPANTS_MIGRATION.to_string(),
            LAB_RESERVATIONS_MIGRATION.to_string(),
            LAB_QC_MIGRATION.to_string(),
            LAB_CONVERSATION_RUN_MIGRATION.to_string(),
            LAB_RUN_DEVIATIONS_MIGRATION.to_string(),
            LAB_DATA_EVIDENCE_MIGRATION.to_string(),
            LAB_SUBJECTS_MIGRATION.to_string(),
            LAB_DERIVATIONS_MIGRATION.to_string(),
            LAB_AMENDMENTS_MIGRATION.to_string(),
            LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION.to_string(),
            LAB_QC_VERDICTS_MIGRATION.to_string(),
            LAB_PARTICIPANT_CONSTRAINTS_MIGRATION.to_string(),
            LAB_MATERIAL_STATE_FACETS_MIGRATION.to_string(),
            LAB_PROJECTION_RETRY_MIGRATION.to_string(),
            LAB_PROTOCOL_REVISION_COUNTER_MIGRATION.to_string(),
        ]
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_backfills_registry_for_existing_data_evidence() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_aux_id_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let evidence_id;
    {
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        store.link_project_lab_registry("p", "lab").await.unwrap();
        let run = store
            .create_wet_lab_run("p", "lab", "run", "Experiment", None, None)
            .await
            .unwrap();
        let mut request = LabTransactionRequest::new("lab", "evidence", LabActorKind::User);
        request.project_id = Some("p".into());
        request.data_evidence_creates.push(LabDataEvidenceCreate {
            owner_project_id: Some("p".into()),
            owner_registry_id: None,
            producing_run_id: Some(run.run_id.clone()),
            role: "raw_data".into(),
            uri: "file:///raw.dat".into(),
            format: None,
            size_bytes: None,
            checksum_sha256: None,
            origin: "instrument".into(),
            manifest_json: "{}".into(),
        });
        store.commit_lab_transaction(request).await.unwrap();
        evidence_id = store.list_lab_run_data_evidence(&run.run_id).await.unwrap()[0]
            .id
            .clone();
        store.pool.close().await;
    }

    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE lab_data_evidence RENAME TO lab_data_evidence_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE lab_data_evidence (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL UNIQUE, \
             owner_project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT, \
             owner_registry_id TEXT REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             producing_run_id TEXT REFERENCES runs(id) ON DELETE RESTRICT, \
             role TEXT NOT NULL, uri TEXT NOT NULL, format TEXT, size_bytes INTEGER, \
             checksum_sha256 TEXT, origin TEXT NOT NULL, manifest_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL, established_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             CHECK ((owner_project_id IS NOT NULL) != (owner_registry_id IS NOT NULL)), \
             CHECK (size_bytes IS NULL OR size_bytes >= 0))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO lab_data_evidence(id,display_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id) \
             SELECT id,display_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id \
             FROM lab_data_evidence_current",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DROP TABLE lab_data_evidence_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE lab_amendments RENAME TO lab_amendments_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE lab_amendments (\
             id TEXT PRIMARY KEY, display_id TEXT NOT NULL UNIQUE, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             run_id TEXT NOT NULL REFERENCES lab_wet_runs(run_id) ON DELETE RESTRICT, original_event_id TEXT NOT NULL REFERENCES lab_events(id) ON DELETE RESTRICT, \
             reason TEXT NOT NULL, correction_json TEXT NOT NULL, affected_ids_json TEXT NOT NULL, \
             established_event_id TEXT NOT NULL UNIQUE REFERENCES lab_events(id) ON DELETE RESTRICT, created_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO lab_amendments SELECT * FROM lab_amendments_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE lab_amendments_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    let evidence: (String, String) =
        sqlx::query_as("SELECT registry_id,display_id FROM lab_data_evidence WHERE id=?")
            .bind(evidence_id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(evidence, ("lab".into(), "DAT-000001".into()));
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&LAB_SCOPED_AUX_DISPLAY_IDS_MIGRATION.to_string()));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_maps_legacy_inconclusive_qc_verdict_to_pending() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_qc_verdict_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let assessment_id;
    {
        let store = Store::open(&tmp).await.unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        let entity = store
            .create_lab_entity(
                "lab",
                LabEntityKind::ResourceDefinition,
                "AB",
                "anti-CD3",
                None,
                None,
            )
            .await
            .unwrap();
        let mut observation =
            LabTransactionRequest::new("lab", "legacy-qc-observation", LabActorKind::User);
        observation
            .qc_observation_creates
            .push(LabQcObservationCreate {
                entity_id: entity.id.clone(),
                run_id: None,
                method_revision_id: None,
                measurement_json: "{}".into(),
                evidence_json: "{}".into(),
                observed_at: 10,
            });
        let receipt = store.commit_lab_transaction(observation).await.unwrap();
        let observation_id =
            serde_json::from_str::<serde_json::Value>(&receipt.events[0].payload_json).unwrap()
                ["observation_id"]
                .as_str()
                .unwrap()
                .to_string();
        let mut assessment =
            LabTransactionRequest::new("lab", "legacy-qc-assessment", LabActorKind::User);
        assessment
            .qc_assessment_creates
            .push(LabQcAssessmentCreate {
                entity_id: entity.id,
                observation_ids: vec![observation_id],
                criteria_json: "{}".into(),
                verdict: "pending".into(),
                rationale: "Awaiting review".into(),
            });
        let receipt = store.commit_lab_transaction(assessment).await.unwrap();
        assessment_id = serde_json::from_str::<serde_json::Value>(&receipt.events[0].payload_json)
            .unwrap()["assessment_id"]
            .as_str()
            .unwrap()
            .to_string();
        store.pool.close().await;
    }

    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE lab_qc_assessments RENAME TO lab_qc_assessments_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE lab_qc_assessments (\
             id TEXT PRIMARY KEY, registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             entity_id TEXT NOT NULL REFERENCES lab_entities(id) ON DELETE RESTRICT, observation_ids_json TEXT NOT NULL, \
             criteria_json TEXT NOT NULL, verdict TEXT NOT NULL CHECK(verdict IN ('pass','fail','inconclusive')), \
             rationale TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO lab_qc_assessments(\
                id,registry_id,entity_id,observation_ids_json,criteria_json,verdict,rationale,created_at) \
             SELECT id,registry_id,entity_id,observation_ids_json,criteria_json,'inconclusive',rationale,created_at \
             FROM lab_qc_assessments_current",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DROP TABLE lab_qc_assessments_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(LAB_QC_VERDICTS_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    let verdict: String = sqlx::query_scalar("SELECT verdict FROM lab_qc_assessments WHERE id=?")
        .bind(assessment_id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(verdict, "pending");
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&LAB_QC_VERDICTS_MIGRATION.to_string()));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_splits_legacy_material_availability_into_state_facets() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_material_state_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let material_id;
    {
        let store = Store::open(&tmp).await.unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        let mut request = LabTransactionRequest::new("lab", "legacy-material", LabActorKind::User);
        request.entity_creates.push(LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "MAT".into(),
            title: "Legacy vial".into(),
            subtype: None,
            metadata_json: "{}".into(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: "inventory".into(),
                quantity: LabQuantity::measured("0", "uL"),
                vessel_description: None,
                availability: "available".into(),
                origin_kind: "legacy_import".into(),
            }),
            location: None,
            subject: None,
        });
        material_id = store
            .commit_lab_transaction(request)
            .await
            .unwrap()
            .created_entities[0]
            .id
            .clone();
        store.pool.close().await;
    }

    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query("PRAGMA legacy_alter_table=ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE lab_material_units RENAME TO lab_material_units_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE lab_material_units (\
             entity_id TEXT PRIMARY KEY REFERENCES lab_entities(id) ON DELETE RESTRICT, \
             registry_id TEXT NOT NULL REFERENCES lab_registries(id) ON DELETE RESTRICT, \
             lot_id TEXT REFERENCES lab_lots(entity_id) ON DELETE RESTRICT, \
             usage_class TEXT NOT NULL CHECK(usage_class IN ('inventory','sample')), \
             quantity_state TEXT NOT NULL CHECK(quantity_state IN ('measured','unknown','not_measured')), \
             quantity_value TEXT, quantity_unit TEXT, vessel_description TEXT, \
             availability TEXT NOT NULL CHECK(availability IN ('available','quarantined','depleted','disposed')), \
             origin_kind TEXT NOT NULL CHECK(origin_kind IN ('receipt','prepared','legacy_import')), \
             CHECK((quantity_state='measured' AND quantity_value IS NOT NULL AND quantity_unit IS NOT NULL) \
                OR (quantity_state!='measured' AND quantity_value IS NULL AND quantity_unit IS NULL)))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO lab_material_units(\
                entity_id,registry_id,lot_id,usage_class,quantity_state,quantity_value,quantity_unit,vessel_description,availability,origin_kind) \
             SELECT entity_id,registry_id,lot_id,usage_class,quantity_state,quantity_value,quantity_unit,vessel_description,'depleted',origin_kind \
             FROM lab_material_units_current",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DROP TABLE lab_material_units_current")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(LAB_MATERIAL_STATE_FACETS_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    let material = store
        .get_lab_material_unit(&material_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(material.lifecycle, "depleted");
    assert_eq!(material.availability, "available");
    assert_eq!(material.identity_state, "verified");
    assert!(
        sqlx::query("UPDATE lab_material_units SET availability='disposed' WHERE entity_id=?",)
            .bind(&material_id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_backfills_protocol_revision_counters() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_protocol_counter_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let protocol_id;
    {
        let store = Store::open(&tmp).await.unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        let protocol = store
            .create_lab_entity(
                "lab",
                LabEntityKind::ProtocolSource,
                "PRT",
                "Protocol",
                None,
                None,
            )
            .await
            .unwrap();
        protocol_id = protocol.id;
        store
            .publish_lab_protocol_revision("lab", &protocol_id, "step one")
            .await
            .unwrap();
        store
            .publish_lab_protocol_revision("lab", &protocol_id, "step one\nstep two")
            .await
            .unwrap();
        store.pool.close().await;
    }

    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query("DROP TABLE lab_protocol_revision_counters")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(LAB_PROTOCOL_REVISION_COUNTER_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    let next_value: i64 = sqlx::query_scalar(
        "SELECT next_value FROM lab_protocol_revision_counters WHERE protocol_entity_id=?",
    )
    .bind(&protocol_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(next_value, 3);
    let third = store
        .publish_lab_protocol_revision("lab", &protocol_id, "step one\nstep two\nstep three")
        .await
        .unwrap();
    assert_eq!(third.revision_number, 3);
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn foreign_key_preflight_rejects_existing_orphans() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_fk_preflight_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE preflight_parent (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE preflight_child (\
             id INTEGER PRIMARY KEY, parent_id TEXT REFERENCES preflight_parent(id))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO preflight_child(id,parent_id) VALUES(1,'missing')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let error = Store::open(&tmp).await.err().unwrap().to_string();
    assert!(error.contains("foreign-key preflight failed"), "{error}");
    assert!(error.contains("preflight_child"), "{error}");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn lab_registry_entities_use_registry_scoped_ids_and_survive_project_deletion() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_lab_registry_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "Project one", "").await.unwrap();
    store.create_project("p2", "Project two", "").await.unwrap();

    let registry = LabRegistry::new("lab-a", "Shared lab", Some("C:/lab-registry".into())).unwrap();
    let other_registry = LabRegistry::new("lab-b", "Other lab", None).unwrap();
    store.create_lab_registry(&registry).await.unwrap();
    store.create_lab_registry(&other_registry).await.unwrap();
    store
        .link_project_lab_registry("p1", "lab-a")
        .await
        .unwrap();
    store
        .link_project_lab_registry("p2", "lab-a")
        .await
        .unwrap();

    let antibody = store
        .create_lab_entity(
            "lab-a",
            LabEntityKind::ResourceDefinition,
            "AB",
            "anti-CD3 antibody",
            Some("antibody"),
            Some(r#"{"target":"CD3"}"#),
        )
        .await
        .unwrap();
    let next_antibody = store
        .create_lab_entity(
            "lab-a",
            LabEntityKind::ResourceDefinition,
            "AB",
            "anti-CD4 antibody",
            Some("antibody"),
            None,
        )
        .await
        .unwrap();
    let other_lab_antibody = store
        .create_lab_entity(
            "lab-b",
            LabEntityKind::ResourceDefinition,
            "AB",
            "anti-CD8 antibody",
            Some("antibody"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(antibody.display_id, "AB-000001");
    assert_eq!(next_antibody.display_id, "AB-000002");
    assert_eq!(other_lab_antibody.display_id, "AB-000001");
    for reserved_prefix in ["TXN", "RUN", "DAT", "AMD"] {
        let error = store
            .create_lab_entity(
                "lab-a",
                LabEntityKind::ResourceDefinition,
                reserved_prefix,
                "Reserved prefix",
                None,
                None,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("reserved"), "{error}");
    }

    store
        .link_lab_entity_to_project(&antibody.id, "p1", "uses")
        .await
        .unwrap();
    let project_registries = store.list_project_lab_registries("p1").await.unwrap();
    assert_eq!(project_registries, vec![registry.clone()]);
    assert_eq!(
        store
            .list_lab_entities("lab-a", Some(LabEntityKind::ResourceDefinition))
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(store
        .link_lab_entity_to_project(&antibody.id, "p2", "uses")
        .await
        .is_ok());
    assert!(store
        .link_lab_entity_to_project(&other_lab_antibody.id, "p1", "uses")
        .await
        .is_err());

    store.delete_project("p1").await.unwrap();
    assert!(store.get_project("p1").await.unwrap().is_none());
    assert_eq!(
        store.get_lab_entity(&antibody.id).await.unwrap(),
        Some(antibody)
    );
    assert_eq!(
        store.list_project_lab_registries("p2").await.unwrap().len(),
        1
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn lab_transactions_are_idempotent_and_enforce_entity_revisions() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_lab_transaction_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();

    let mut create = LabTransactionRequest::new("lab", "cmd-create-antibody", LabActorKind::Agent);
    create.project_id = Some("p".into());
    create.confirmation_json = r#"{"approved":true}"#.into();
    create.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::ResourceDefinition,
        prefix: "AB".into(),
        title: "anti-CD3 antibody".into(),
        subtype: Some("antibody".into()),
        metadata_json: r#"{"target":"CD3"}"#.into(),
        project_relation: Some("uses".into()),
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: None,
        location: None,
        subject: None,
    });
    let first = store.commit_lab_transaction(create.clone()).await.unwrap();
    assert!(!first.idempotent);
    assert_eq!(first.transaction.display_id, "TXN-000001");
    assert_eq!(first.created_entities.len(), 1);
    assert_eq!(first.events.len(), 1);
    let entity = first.created_entities[0].clone();
    assert_eq!(entity.display_id, "AB-000001");
    assert_eq!(entity.revision, 1);

    let repeated = store.commit_lab_transaction(create.clone()).await.unwrap();
    assert!(repeated.idempotent);
    assert_eq!(repeated.transaction.id, first.transaction.id);
    assert_eq!(repeated.created_entities, vec![entity.clone()]);
    assert_eq!(repeated.events.len(), 1);
    assert_eq!(
        store
            .list_lab_events(&first.transaction.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let mut conflicting = create.clone();
    conflicting.entity_creates[0].title = "different request".into();
    assert!(store.commit_lab_transaction(conflicting).await.is_err());

    let mut update = LabTransactionRequest::new("lab", "cmd-update-antibody", LabActorKind::User);
    update.project_id = Some("p".into());
    update.entity_updates.push(LabEntityUpdate {
        entity_id: entity.id.clone(),
        expected_revision: 1,
        title: "anti-CD3 antibody (verified)".into(),
        subtype: Some("antibody".into()),
        metadata_json: r#"{"target":"CD3","verified":true}"#.into(),
        event_kind: "entity_corrected".into(),
        event_payload_json: r#"{"field":"title","value":"anti-CD3 antibody (verified)"}"#.into(),
        occurred_at: chrono::Utc::now().timestamp(),
        prior_event_id: Some(first.events[0].id.clone()),
        reason: Some("catalog verified".into()),
    });
    let updated = store.commit_lab_transaction(update).await.unwrap();
    assert!(!updated.idempotent);
    assert_eq!(updated.transaction.display_id, "TXN-000002");
    assert_eq!(updated.events[0].expected_revision, Some(1));
    assert_eq!(updated.events[0].resulting_revision, Some(2));
    let current = store.get_lab_entity(&entity.id).await.unwrap().unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.title, "anti-CD3 antibody (verified)");

    let mut stale = LabTransactionRequest::new("lab", "cmd-stale", LabActorKind::Agent);
    stale.entity_updates.push(LabEntityUpdate {
        entity_id: entity.id.clone(),
        expected_revision: 1,
        title: "stale update".into(),
        subtype: Some("antibody".into()),
        metadata_json: "{}".into(),
        event_kind: "entity_corrected".into(),
        event_payload_json: "{}".into(),
        occurred_at: chrono::Utc::now().timestamp(),
        prior_event_id: None,
        reason: None,
    });
    let error = store
        .commit_lab_transaction(stale)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("revision conflict"), "{error}");
    assert!(store
        .get_lab_transaction("lab", "cmd-stale")
        .await
        .unwrap()
        .is_none());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_adds_execution_context_table_on_legacy_db() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_context_legacy_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, description TEXT, \
             workspace_dir TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE frames (id TEXT PRIMARY KEY, parent_frame_id TEXT, root_frame_id TEXT, \
             agent_name TEXT NOT NULL, status TEXT NOT NULL, project_id TEXT, folder_id TEXT, model TEXT, \
             input_tokens INTEGER, output_tokens INTEGER, created_at INTEGER NOT NULL, \
             updated_at INTEGER NOT NULL, completed_at INTEGER, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id TEXT PRIMARY KEY, frame_id TEXT NOT NULL, seq INTEGER NOT NULL, \
             role TEXT NOT NULL, content TEXT, tool_calls TEXT, tool_call_id TEXT, tool_name TEXT, \
             reasoning TEXT, ts INTEGER NOT NULL, model_name TEXT, UNIQUE(frame_id, seq))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    store
        .upsert_execution_context(&ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_execution_context("local")
            .await
            .unwrap()
            .unwrap()
            .kind,
        ExecutionContextKind::Local
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn migrate_adds_ssh_run_control_columns_to_existing_runs() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_run_control_legacy_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", tmp.display()))
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE wisp_schema_migrations (\
             version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (applied_at, version) in [
            (1, INITIAL_SCHEMA_MIGRATION),
            (2, CONTROL_PLANE_MIGRATION),
            (3, ARTIFACT_LINEAGE_MIGRATION),
        ] {
            sqlx::query("INSERT INTO wisp_schema_migrations(version,applied_at) VALUES(?,?)")
                .bind(version)
                .bind(applied_at)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(
            "CREATE TABLE execution_contexts (\
             id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL, \
             config_json TEXT NOT NULL DEFAULT '{}', capabilities_json TEXT NOT NULL DEFAULT '{}', \
             last_probe_at INTEGER, last_probe_status TEXT, last_probe_error TEXT, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE runs (\
             id TEXT PRIMARY KEY, project_id TEXT NOT NULL, frame_id TEXT, context_id TEXT NOT NULL, \
             title TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL, command TEXT, script_path TEXT, \
             input_refs_json TEXT NOT NULL DEFAULT '[]', output_specs_json TEXT NOT NULL DEFAULT '[]', \
             created_at INTEGER NOT NULL, started_at INTEGER, ended_at INTEGER, exit_code INTEGER, \
             stdout_tail TEXT, stderr_tail TEXT, remote_workdir TEXT, \
             env_snapshot_json TEXT NOT NULL DEFAULT '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs(id,project_id,context_id,title,kind,status,created_at) \
             VALUES('legacy','p','local','Legacy','command','submitted',1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    let run = store.get_run("legacy").await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Submitted);
    assert!(run.remote_handle_json.is_none());
    assert!(run.timeout_secs.is_none());
    assert!(run.last_polled_at.is_none());
    assert!(run.last_poll_error.is_none());
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&SSH_RUN_CONTROL_MIGRATION.to_string()));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn run_manager_roundtrip_and_lifecycle() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_runs_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .upsert_execution_context(&ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();

    let mut run = RunRecord::new("r1", "p", "local", "QC", "command");
    run.frame_id = Some("f1".into());
    run.command = Some("python qc.py".into());
    run.input_refs_json = r#"["data/raw/counts.tsv"]"#.into();
    run.output_specs_json = r#"[{"glob":"results/*.tsv","kind":"table"}]"#.into();
    run.timeout_secs = Some(900);
    store.create_run(&run).await.unwrap();

    let got = store.get_run("r1").await.unwrap().unwrap();
    assert_eq!(got.status, RunStatus::Draft);
    assert_eq!(got.command.as_deref(), Some("python qc.py"));
    assert_eq!(got.input_refs_json, r#"["data/raw/counts.tsv"]"#);
    assert_eq!(got.timeout_secs, Some(900));

    store
        .update_run_status("r1", RunStatus::Submitted)
        .await
        .unwrap();
    store
        .set_run_remote_handle(
            "r1",
            r#"{"kind":"ssh_direct","pid":42,"start_time":7}"#,
            "/scratch/wisp/r1",
        )
        .await
        .unwrap();
    store
        .update_run_status("r1", RunStatus::Running)
        .await
        .unwrap();
    store
        .record_run_poll("r1", Some("ok stdout"), None, Some("temporary error"))
        .await
        .unwrap();
    store
        .record_run_poll("r1", None, Some("warn stderr"), None)
        .await
        .unwrap();
    store
        .finish_run("r1", RunStatus::Succeeded, Some(0))
        .await
        .unwrap();

    let finished = store.get_run("r1").await.unwrap().unwrap();
    assert_eq!(finished.status, RunStatus::Succeeded);
    assert_eq!(finished.exit_code, Some(0));
    assert_eq!(finished.stdout_tail.as_deref(), Some("ok stdout"));
    assert_eq!(finished.stderr_tail.as_deref(), Some("warn stderr"));
    assert_eq!(
        finished.remote_handle_json.as_deref(),
        Some(r#"{"kind":"ssh_direct","pid":42,"start_time":7}"#)
    );
    assert_eq!(finished.remote_workdir.as_deref(), Some("/scratch/wisp/r1"));
    assert_eq!(finished.timeout_secs, Some(900));
    assert!(finished.last_polled_at.is_some());
    assert!(finished.last_poll_error.is_none());
    assert!(finished.started_at.is_some());
    assert!(finished.ended_at.is_some());

    let runs = store.list_runs_by_project("p").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "r1");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn run_can_cancel_then_time_out() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_run_cancel_timeout_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_run(&RunRecord::new("r1", "p", "local", "Remote", "command"))
        .await
        .unwrap();

    store
        .update_run_status("r1", RunStatus::Submitted)
        .await
        .unwrap();
    store
        .update_run_status("r1", RunStatus::Cancelling)
        .await
        .unwrap();
    assert_eq!(
        store.get_run("r1").await.unwrap().unwrap().status,
        RunStatus::Cancelling
    );
    store
        .finish_run("r1", RunStatus::TimedOut, None)
        .await
        .unwrap();
    assert_eq!(
        store.get_run("r1").await.unwrap().unwrap().status,
        RunStatus::TimedOut
    );
    assert_eq!(
        serde_json::to_string(&RunStatus::TimedOut).unwrap(),
        r#""timed_out""#
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn conditional_terminal_update_does_not_overwrite_winner() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_run_terminal_race_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["submitted", "running", "cancelling", "draft"] {
        store
            .create_run(&RunRecord::new(id, "p", "local", id, "command"))
            .await
            .unwrap();
    }
    store
        .update_run_status("submitted", RunStatus::Submitted)
        .await
        .unwrap();
    store
        .update_run_status("running", RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status("cancelling", RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run_status("cancelling", RunStatus::Cancelling)
        .await
        .unwrap();

    let active = store.list_active_runs().await.unwrap();
    assert_eq!(active.len(), 3);
    assert!(active.iter().any(|run| run.status == RunStatus::Cancelling));
    assert!(store.mark_run_lost("running").await.unwrap());
    assert!(!store.mark_run_lost("running").await.unwrap());
    assert!(store
        .finish_active_run("cancelling", RunStatus::Cancelled, None)
        .await
        .unwrap());
    assert!(!store
        .finish_active_run("cancelling", RunStatus::TimedOut, None)
        .await
        .unwrap());
    assert!(!store
        .finish_active_run("draft", RunStatus::Failed, Some(1))
        .await
        .unwrap());
    assert!(store
        .finish_active_run("submitted", RunStatus::Succeeded, Some(0))
        .await
        .unwrap());
    assert_eq!(
        store.get_run("cancelling").await.unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    assert!(store
        .finish_active_run("draft", RunStatus::Running, None)
        .await
        .is_err());

    let mut restart_cancel = RunRecord::new("restart-cancel", "p", "local", "rc", "command");
    restart_cancel.status = RunStatus::Cancelling;
    store.create_run(&restart_cancel).await.unwrap();
    assert_eq!(store.mark_active_runs_lost().await.unwrap(), 1);
    assert_eq!(
        store
            .get_run("restart-cancel")
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Lost
    );

    let lease_run = RunRecord::new("lease", "p", "ssh:gpu", "lease", "ssh_direct");
    store.create_run(&lease_run).await.unwrap();
    assert!(store
        .activate_run_lifecycle("lease", RunStatus::Submitted, "owner-a", 30)
        .await
        .unwrap());
    assert!(!store
        .claim_run_lifecycle("lease", "owner-b", 30)
        .await
        .unwrap());
    assert!(!store
        .record_run_poll_owned("lease", "owner-b", None, None, Some("stale"))
        .await
        .unwrap());
    assert!(!store
        .finish_active_run_owned("lease", "owner-b", RunStatus::Cancelled, None)
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("lease", "owner-a", RunStatus::Cancelled, None)
        .await
        .unwrap());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn run_status_transitions_are_validated() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_run_status_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .upsert_execution_context(&ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    store
        .create_run(&RunRecord::new("r1", "p", "local", "Terminal", "command"))
        .await
        .unwrap();
    store
        .update_run_status("r1", RunStatus::Running)
        .await
        .unwrap();
    store
        .finish_run("r1", RunStatus::Failed, Some(1))
        .await
        .unwrap();

    let err = store
        .update_run_status("r1", RunStatus::Running)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Invalid run status transition"), "{err}");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn research_graph_links_research_objects() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_research_graph_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    store
        .create_run(&RunRecord::new(
            "run-1",
            "p",
            "local",
            "Differential expression",
            "command",
        ))
        .await
        .unwrap();
    store
        .save_artifact(
            "art-1",
            "p",
            "f1",
            "volcano.png",
            "image/png",
            "figures/volcano.png",
        )
        .await
        .unwrap();
    store
        .save_run_artifact_link("run-art-1", "run-1", "art-1", "figure")
        .await
        .unwrap();

    for node in [
        ResearchNode::new("data-1", "p", ResearchNodeKind::DataAsset, "Counts matrix"),
        ResearchNode::new(
            "paper-1",
            "p",
            ResearchNodeKind::Paper,
            "Kinase screen paper",
        ),
        ResearchNode::new(
            "decision-1",
            "p",
            ResearchNodeKind::Decision,
            "Use FDR 0.05",
        ),
    ] {
        let node = node.unwrap();
        store.save_research_node(&node).await.unwrap();
    }

    for edge in [
        ResearchEdge::new("edge-1", "p", "data-1", "run:run-1", "input_to"),
        ResearchEdge::new("edge-3", "p", "paper-1", "decision-1", "supports"),
        ResearchEdge::new("edge-4", "p", "decision-1", "run:run-1", "sets_parameter"),
    ] {
        store.save_research_edge(&edge.unwrap()).await.unwrap();
    }

    let graph = store.research_graph("p").await.unwrap();
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.edges.len(), 4);
    assert!(graph.edges.iter().any(|e| e.source_id == "run:run-1"
        && e.target_id == "artifact:art-1"
        && e.relation == "produced"));

    let papers = store
        .list_research_nodes("p", Some(ResearchNodeKind::Paper))
        .await
        .unwrap();
    assert_eq!(papers.len(), 1);
    assert_eq!(papers[0].title, "Kinase screen paper");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn artifacts_keep_version_lineage_and_dependencies() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_artifact_versions_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    let first = store
        .save_artifact("a", "p", "f", "report.md", "text/markdown", "reports/v1.md")
        .await
        .unwrap();
    let second = store
        .save_artifact("a", "p", "f", "report.md", "text/markdown", "reports/v2.md")
        .await
        .unwrap();
    store
        .save_artifact_dependency("dep", &second, &first, Some("prior-report"))
        .await
        .unwrap();

    let versions = store.list_artifact_versions("a").await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version_number, 2);
    assert_eq!(
        versions[0].parent_version_id.as_deref(),
        Some(first.as_str())
    );
    assert_eq!(versions[0].storage_path, "reports/v2.md");
    assert_eq!(versions[1].version_number, 1);

    let graph = store.research_graph("p").await.unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.id == "artifact:a" && node.ref_id.as_deref() == Some("a")));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn provenance_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("wisp_prov_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "proj", "").await.unwrap();
    store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
    store
        .record_env_snapshot(
            "h1",
            Some("kernel"),
            r#"[{"name":"numpy","version":"1.0"}]"#,
        )
        .await
        .unwrap();
    let e = ExecLog {
        id: "e1".into(),
        frame_id: "f1".into(),
        cell_index: 0,
        tool: "python".into(),
        language: "python".into(),
        source: "savefig('out/fig.png')".into(),
        stdout: "done".into(),
        stderr: String::new(),
        exit_status: "ok".into(),
        wall_s: Some(1.5),
        files_written: vec!["out/fig.png".into()],
        files_read: vec!["data.csv".into()],
        env_hash: Some("h1".into()),
    };
    store.insert_execution_log(&e).await.unwrap();
    let got = store
        .find_provenance_by_path("f1", "out/fig.png")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.source, "savefig('out/fig.png')");
    assert_eq!(got.files_read, vec!["data.csv".to_string()]);
    assert!(store
        .find_provenance_by_path("f1", "missing.png")
        .await
        .unwrap()
        .is_none());
    // LIKE-prefilter regressions: `_`/`%` must be escaped as literals, a
    // backslash path must match its JSON-encoded stored form, and a
    // suffix of a written path must not match (exact check, not substring).
    let e2 = ExecLog {
        id: "e2".into(),
        cell_index: 1,
        files_written: vec!["out/my_fig 100%.png".into(), r"C:\data\x.csv".into()],
        ..e.clone()
    };
    store.insert_execution_log(&e2).await.unwrap();
    for p in ["out/my_fig 100%.png", r"C:\data\x.csv"] {
        assert!(
            store
                .find_provenance_by_path("f1", p)
                .await
                .unwrap()
                .is_some(),
            "should find {p}"
        );
    }
    assert!(store
        .find_provenance_by_path("f1", "fig.png")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .get_env_snapshot("h1")
            .await
            .unwrap()
            .unwrap()
            .0
            .as_deref(),
        Some("kernel")
    );
    assert!(store
        .frame_written_paths("f1")
        .await
        .unwrap()
        .contains("out/fig.png"));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn resource_definitions_and_typed_aliases_are_transactional() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_lab_resource_definition_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Shared lab", None).unwrap())
        .await
        .unwrap();

    let mut request = LabTransactionRequest::new("lab", "register-antibody", LabActorKind::Agent);
    request.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::ResourceDefinition,
        prefix: "AB".into(),
        title: "anti-CD3 clone 17A2".into(),
        subtype: Some("antibody".into()),
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: Some(LabResourceDefinitionCreate {
            category: "antibody".into(),
            supplier: Some("BioLegend".into()),
            catalog_number: Some("100201".into()),
            attributes_json: r#"{"target":"CD3"}"#.into(),
        }),
        aliases: vec![
            LabAliasCreate {
                alias_type: "barcode".into(),
                namespace: None,
                value: "BC-0001".into(),
            },
            LabAliasCreate {
                alias_type: "legacy_id".into(),
                namespace: Some("old-notebook".into()),
                value: "A-7".into(),
            },
        ],
        lot: None,
        material_unit: None,
        location: None,
        subject: None,
    });
    let result = store.commit_lab_transaction(request).await.unwrap();
    let entity = &result.created_entities[0];
    let definition = store
        .get_lab_resource_definition(&entity.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(definition.catalog_number.as_deref(), Some("100201"));
    assert_eq!(store.list_lab_aliases(&entity.id).await.unwrap().len(), 2);
    assert_eq!(
        store
            .find_lab_aliases("lab", "BC-0001")
            .await
            .unwrap()
            .len(),
        1
    );

    let mut duplicate = LabTransactionRequest::new("lab", "duplicate-barcode", LabActorKind::Agent);
    duplicate.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::ResourceDefinition,
        prefix: "AB".into(),
        title: "another antibody".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![LabAliasCreate {
            alias_type: "barcode".into(),
            namespace: None,
            value: "BC-0001".into(),
        }],
        lot: None,
        material_unit: None,
        location: None,
        subject: None,
    });
    assert!(store.commit_lab_transaction(duplicate).await.is_err());
    assert_eq!(
        store
            .list_lab_entities("lab", Some(LabEntityKind::ResourceDefinition))
            .await
            .unwrap()
            .len(),
        1,
        "a failed alias insert must roll back its entity"
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn lots_and_material_units_validate_quantity_and_registry_ownership() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_lab_inventory_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for id in ["lab-a", "lab-b"] {
        store
            .create_lab_registry(&LabRegistry::new(id, id, None).unwrap())
            .await
            .unwrap();
    }

    let mut definition = LabTransactionRequest::new("lab-a", "definition", LabActorKind::User);
    definition.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::ResourceDefinition,
        prefix: "AB".into(),
        title: "anti-CD3".into(),
        subtype: Some("antibody".into()),
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: Some(LabResourceDefinitionCreate {
            category: "antibody".into(),
            supplier: None,
            catalog_number: None,
            attributes_json: "{}".into(),
        }),
        aliases: vec![],
        lot: None,
        material_unit: None,
        location: None,
        subject: None,
    });
    let definition_id = store
        .commit_lab_transaction(definition)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();

    let mut lot = LabTransactionRequest::new("lab-a", "lot", LabActorKind::User);
    lot.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::Lot,
        prefix: "LOT".into(),
        title: "BioLegend lot 123".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: Some(LabLotCreate {
            resource_definition_id: definition_id,
            supplier: Some("BioLegend".into()),
            catalog_number: Some("100201".into()),
            lot_number: "123".into(),
            received_at: Some(10),
            expiry_at: Some(20),
            origin_kind: "receipt".into(),
        }),
        material_unit: None,
        location: None,
        subject: None,
    });
    let lot_id = store
        .commit_lab_transaction(lot)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();

    let mut vial = LabTransactionRequest::new("lab-a", "vial", LabActorKind::User);
    vial.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::MaterialUnit,
        prefix: "VIAL".into(),
        title: "anti-CD3 vial".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: Some(LabMaterialUnitCreate {
            lot_id: Some(lot_id),
            usage_class: "inventory".into(),
            quantity: LabQuantity::measured("100", "uL"),
            vessel_description: Some("vial".into()),
            availability: "available".into(),
            origin_kind: "receipt".into(),
        }),
        location: None,
        subject: None,
    });
    let vial_id = store
        .commit_lab_transaction(vial)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();
    let material = store
        .get_lab_material_unit(&vial_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(material.quantity.value.as_deref(), Some("100"));
    assert_eq!(material.quantity.dimension(), Some("volume"));

    let mut adjustment = LabTransactionRequest::new("lab-a", "vial-adjustment", LabActorKind::User);
    adjustment.material_adjustments.push(LabMaterialAdjustment {
        material_unit_id: vial_id.clone(),
        expected_revision: 1,
        quantity: LabQuantity::measured("50", "uL"),
        lifecycle: None,
        availability: Some("quarantined".into()),
        identity_state: None,
        occurred_at: 30,
        reason: "remaining volume reconciled after receiving".into(),
        prior_event_id: None,
    });
    let adjustment_receipt = store.commit_lab_transaction(adjustment).await.unwrap();
    assert_eq!(
        adjustment_receipt.events[0].kind,
        "material_quantity_adjusted"
    );
    let adjusted = store
        .get_lab_material_unit(&vial_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(adjusted.quantity.value.as_deref(), Some("50"));
    assert_eq!(adjusted.lifecycle, "active");
    assert_eq!(adjusted.availability, "quarantined");
    assert_eq!(adjusted.identity_state, "verified");

    let mut state_adjustment =
        LabTransactionRequest::new("lab-a", "vial-state-adjustment", LabActorKind::User);
    state_adjustment
        .material_adjustments
        .push(LabMaterialAdjustment {
            material_unit_id: vial_id.clone(),
            expected_revision: 2,
            quantity: LabQuantity::measured("50", "uL"),
            lifecycle: Some("discarded".into()),
            availability: Some("available".into()),
            identity_state: Some("suspect".into()),
            occurred_at: 31,
            reason: "vial discarded after identity concern".into(),
            prior_event_id: None,
        });
    store
        .commit_lab_transaction(state_adjustment)
        .await
        .unwrap();
    let adjusted = store
        .get_lab_material_unit(&vial_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(adjusted.lifecycle, "discarded");
    assert_eq!(adjusted.availability, "available");
    assert_eq!(adjusted.identity_state, "suspect");

    let mut invalid = LabTransactionRequest::new("lab-a", "invalid-vial", LabActorKind::User);
    invalid.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::MaterialUnit,
        prefix: "VIAL".into(),
        title: "invalid vial".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: Some(LabMaterialUnitCreate {
            lot_id: None,
            usage_class: "inventory".into(),
            quantity: LabQuantity::measured("01.0", "uL"),
            vessel_description: None,
            availability: "available".into(),
            origin_kind: "receipt".into(),
        }),
        location: None,
        subject: None,
    });
    assert!(store.commit_lab_transaction(invalid).await.is_err());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn locations_are_acyclic_at_creation_and_single_slots_reject_double_occupancy() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_lab_locations_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();

    let mut location = LabTransactionRequest::new("lab", "slot", LabActorKind::User);
    location.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::Location,
        prefix: "LOC".into(),
        title: "Freezer A / box 1 / A1".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: None,
        location: Some(LabLocationCreate {
            parent_location_id: None,
            location_class: "slot".into(),
            single_occupancy: true,
        }),
        subject: None,
    });
    let slot_id = store
        .commit_lab_transaction(location)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();

    let mut materials = LabTransactionRequest::new("lab", "vials", LabActorKind::User);
    for title in ["vial 1", "vial 2"] {
        materials.entity_creates.push(LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "MAT".into(),
            title: title.into(),
            subtype: None,
            metadata_json: "{}".into(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: "inventory".into(),
                quantity: LabQuantity::measured("1", "each"),
                vessel_description: Some("vial".into()),
                availability: "available".into(),
                origin_kind: "receipt".into(),
            }),
            location: None,
            subject: None,
        });
    }
    let material_ids: Vec<String> = store
        .commit_lab_transaction(materials)
        .await
        .unwrap()
        .created_entities
        .into_iter()
        .map(|entity| entity.id)
        .collect();

    let mut first_move = LabTransactionRequest::new("lab", "move-1", LabActorKind::User);
    first_move.material_moves.push(LabMaterialMove {
        material_unit_id: material_ids[0].clone(),
        location_id: Some(slot_id.clone()),
        expected_revision: 1,
        occurred_at: 1,
        reason: Some("put away".into()),
        prior_event_id: None,
    });
    let receipt = store.commit_lab_transaction(first_move).await.unwrap();
    assert_eq!(receipt.events[0].kind, "material_moved");
    assert_eq!(
        store.get_material_location(&material_ids[0]).await.unwrap(),
        Some(slot_id.clone())
    );

    let mut conflicting_move = LabTransactionRequest::new("lab", "move-2", LabActorKind::User);
    conflicting_move.material_moves.push(LabMaterialMove {
        material_unit_id: material_ids[1].clone(),
        location_id: Some(slot_id),
        expected_revision: 1,
        occurred_at: 2,
        reason: None,
        prior_event_id: None,
    });
    assert!(store
        .commit_lab_transaction(conflicting_move)
        .await
        .is_err());
    assert!(store
        .get_material_location(&material_ids[1])
        .await
        .unwrap()
        .is_none());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn dossier_updates_are_transactional_and_project_through_an_outbox() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_lab_documents_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    let entity = store
        .create_lab_entity(
            "lab",
            LabEntityKind::ResourceDefinition,
            "AB",
            "anti-CD3",
            Some("antibody"),
            None,
        )
        .await
        .unwrap();
    let mut request = LabTransactionRequest::new("lab", "write-dossier", LabActorKind::User);
    request.document_upserts.push(LabDocumentUpsert {
        entity_id: entity.id.clone(),
        relative_path: "resources/AB-000001.md".into(),
        narrative_markdown: "# Notes\n\nValidated for flow cytometry.".into(),
        extension_json: r#"{"vendor_note":"keep cold"}"#.into(),
        expected_revision: None,
    });
    let receipt = store.commit_lab_transaction(request).await.unwrap();
    assert_eq!(receipt.events[0].kind, "document_updated");
    let document = store.get_lab_document(&entity.id).await.unwrap().unwrap();
    assert_eq!(document.revision, 1);
    let outbox = store.list_lab_projection_outbox().await.unwrap();
    assert_eq!(outbox.len(), 1);
    assert!(outbox[0].content.contains("document_revision: 1"));
    assert!(outbox[0].content.contains("Validated for flow cytometry."));
    store
        .acknowledge_lab_projection(&outbox[0].id)
        .await
        .unwrap();
    let document = store.get_lab_document(&entity.id).await.unwrap().unwrap();
    assert!(document.last_projected_content.is_some());
    assert!(store.list_lab_projection_outbox().await.unwrap().is_empty());

    let base_content = document.last_projected_content.clone().unwrap();
    let mut database_update =
        LabTransactionRequest::new("lab", "database-dossier-update", LabActorKind::User);
    database_update.document_upserts.push(LabDocumentUpsert {
        entity_id: entity.id.clone(),
        relative_path: document.relative_path.clone(),
        narrative_markdown: document.narrative_markdown.clone(),
        extension_json: r#"{"vendor_note":"refrigerate","database_tag":true}"#.into(),
        expected_revision: Some(1),
    });
    store.commit_lab_transaction(database_update).await.unwrap();
    let mut file_edit = parse_lab_document(&base_content).unwrap();
    file_edit.narrative_markdown = "# Notes\n\nEdited externally.".into();
    let file_markdown = serialize_lab_document(&file_edit).unwrap();
    let preview = store
        .preview_lab_document_import(&entity.id, &file_markdown)
        .await
        .unwrap();
    assert_eq!(preview.status, "ready_to_import");
    assert_eq!(preview.parsed.document_revision, 2);
    assert_eq!(
        preview.parsed.narrative_markdown,
        "# Notes\n\nEdited externally."
    );
    assert_eq!(
        preview.parsed.extensions["database_tag"], true,
        "a clean three-way merge keeps concurrent database-only fields"
    );

    let mut database_narrative =
        LabTransactionRequest::new("lab", "database-narrative-update", LabActorKind::User);
    database_narrative.document_upserts.push(LabDocumentUpsert {
        entity_id: entity.id.clone(),
        relative_path: document.relative_path.clone(),
        narrative_markdown: "# Notes\n\nEdited in database.".into(),
        extension_json: r#"{"vendor_note":"refrigerate","database_tag":true}"#.into(),
        expected_revision: Some(2),
    });
    store
        .commit_lab_transaction(database_narrative)
        .await
        .unwrap();
    let conflict = store
        .preview_lab_document_import(&entity.id, &file_markdown)
        .await
        .unwrap();
    assert_eq!(conflict.status, "conflict");
    assert!(conflict
        .conflicts
        .iter()
        .any(|value| value.contains("narrative")));
    let mut protected_edit = parse_lab_document(&base_content).unwrap();
    protected_edit.title = "silently renamed".into();
    let protected = store
        .preview_lab_document_import(
            &entity.id,
            &serialize_lab_document(&protected_edit).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(protected.status, "conflict");
    assert!(protected
        .conflicts
        .iter()
        .any(|value| value.contains("System-managed")));

    let mut stale = LabTransactionRequest::new("lab", "stale-dossier", LabActorKind::User);
    stale.document_upserts.push(LabDocumentUpsert {
        entity_id: entity.id,
        relative_path: "resources/AB-000001.md".into(),
        narrative_markdown: "new text".into(),
        extension_json: "{}".into(),
        expected_revision: Some(2),
    });
    assert!(store.commit_lab_transaction(stale).await.is_err());
    let pending = store.list_lab_projection_outbox().await.unwrap();
    assert_eq!(pending.len(), 1);
    for attempt in 1..=5 {
        store
            .fail_lab_projection(&pending[0].id, "editor lock")
            .await
            .unwrap();
        assert_eq!(
            store.list_lab_projection_outbox().await.unwrap().len(),
            usize::from(attempt < 5),
        );
    }
    let dead_letters = store.list_lab_projection_dead_letters().await.unwrap();
    assert_eq!(dead_letters.len(), 1);
    assert_eq!(dead_letters[0].attempts, 5);
    assert_eq!(dead_letters[0].last_error.as_deref(), Some("editor lock"));
    assert!(dead_letters[0].dead_lettered_at.is_some());
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn frontmatter_roundtrip_retains_unknown_fields_and_narrative() {
    let source = "---\nid: AB-000001\nentity_id: entity-1\nkind: resource_definition\ntitle: anti-CD3\nentity_revision: 2\ndocument_revision: 3\nschema_version: 1\nextensions:\n  vendor_note: keep cold\nlocal_tag: flow-panel\n---\n\n# Notes\n\nUseful.\n";
    let parsed = parse_lab_document(source).unwrap();
    assert_eq!(parsed.unknown_frontmatter["local_tag"], "flow-panel");
    assert_eq!(parsed.extensions["vendor_note"], "keep cold");
    let rendered = serialize_lab_document(&parsed).unwrap();
    let reparsed = parse_lab_document(&rendered).unwrap();
    assert_eq!(reparsed.unknown_frontmatter, parsed.unknown_frontmatter);
    assert_eq!(reparsed.narrative_markdown, parsed.narrative_markdown);
}

#[tokio::test]
async fn wet_lab_runs_have_no_compute_context_or_process_recovery_lease() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_wet_lab_run_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    let wet_run = store
        .create_wet_lab_run(
            "p",
            "lab",
            "wet-run-1",
            "Cell staining",
            None,
            Some("Alice"),
        )
        .await
        .unwrap();
    assert_eq!(wet_run.display_id, "RUN-000001");
    let run = store.get_run(&wet_run.run_id).await.unwrap().unwrap();
    assert_eq!(run.kind, "wet_lab");
    assert!(run.context_id.is_empty());
    assert!(store.list_active_runs().await.unwrap().is_empty());
    assert!(store
        .update_wet_lab_run_status(&wet_run.run_id, RunStatus::Running)
        .await
        .unwrap());
    assert!(store.list_active_runs().await.unwrap().is_empty());
    assert!(store
        .update_wet_lab_run_status(&wet_run.run_id, RunStatus::Succeeded)
        .await
        .unwrap());
    assert!(store
        .update_wet_lab_run_status(&wet_run.run_id, RunStatus::Lost)
        .await
        .is_err());
    let protocol = store
        .create_lab_entity(
            "lab",
            LabEntityKind::ProtocolSource,
            "PRT",
            "Cell staining protocol",
            None,
            None,
        )
        .await
        .unwrap();
    let first = store
        .publish_lab_protocol_revision("lab", &protocol.id, "1. Add antibody")
        .await
        .unwrap();
    let same = store
        .publish_lab_protocol_revision("lab", &protocol.id, "1. Add antibody")
        .await
        .unwrap();
    let second = store
        .publish_lab_protocol_revision("lab", &protocol.id, "1. Add antibody\n2. Wash")
        .await
        .unwrap();
    assert_eq!(first.id, same.id);
    assert_eq!(first.revision_number, 1);
    assert_eq!(second.revision_number, 2);
    assert_ne!(first.checksum_sha256, second.checksum_sha256);
    let next_revision: i64 = sqlx::query_scalar(
        "SELECT next_value FROM lab_protocol_revision_counters WHERE protocol_entity_id=?",
    )
    .bind(&protocol.id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(next_revision, 3);
    let planned = store
        .create_wet_lab_run("p", "lab", "wet-run-2", "Second staining", None, None)
        .await
        .unwrap();
    store
        .pin_wet_lab_run_protocol("p", &planned.run_id, &second.id)
        .await
        .unwrap();
    let mut materials = LabTransactionRequest::new("lab", "run-materials", LabActorKind::User);
    for title in ["input vial", "output sample"] {
        materials.entity_creates.push(LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "MAT".into(),
            title: title.into(),
            subtype: None,
            metadata_json: "{}".into(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: if title == "input vial" {
                    "inventory".into()
                } else {
                    "sample".into()
                },
                quantity: LabQuantity::measured("10", "uL"),
                vessel_description: None,
                availability: "available".into(),
                origin_kind: "prepared".into(),
            }),
            location: None,
            subject: None,
        });
    }
    let material_ids: Vec<String> = store
        .commit_lab_transaction(materials)
        .await
        .unwrap()
        .created_entities
        .into_iter()
        .map(|entity| entity.id)
        .collect();
    let mut participants =
        LabTransactionRequest::new("lab", "run-participants", LabActorKind::User);
    participants.project_id = Some("p".into());
    participants.run_participants.push(LabRunParticipantCreate {
        run_id: planned.run_id.clone(),
        material_unit_id: material_ids[0].clone(),
        direction: "input".into(),
        role: "reagent".into(),
        effect: "partially_consumed".into(),
        quantity: Some(LabQuantity::measured("0.002", "mL")),
        transformation_group: Some("well-a1".into()),
        expected_material_revision: Some(1),
    });
    let participant_receipt = store.commit_lab_transaction(participants).await.unwrap();
    assert_eq!(participant_receipt.events.len(), 1);
    assert_eq!(participant_receipt.events[0].expected_revision, Some(1));
    assert_eq!(participant_receipt.events[0].resulting_revision, Some(2));
    let remaining = store
        .get_lab_material_unit(&material_ids[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(remaining.quantity, LabQuantity::measured("8", "uL"));
    assert_eq!(remaining.availability, "available");
    assert_eq!(
        store
            .get_lab_entity(&material_ids[0])
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        store
            .list_wet_lab_run_participants(&planned.run_id)
            .await
            .unwrap()
            .len(),
        1
    );
    let participant_id = store
        .list_wet_lab_run_participants(&planned.run_id)
        .await
        .unwrap()[0]
        .id
        .clone();
    assert!(
        sqlx::query("UPDATE lab_run_participants SET role='equipment' WHERE id=?")
            .bind(&participant_id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE lab_run_participants SET effect='produced' WHERE id=?")
            .bind(&participant_id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    let mut consume_remaining =
        LabTransactionRequest::new("lab", "run-full-consumption", LabActorKind::User);
    consume_remaining.project_id = Some("p".into());
    consume_remaining
        .run_participants
        .push(LabRunParticipantCreate {
            run_id: planned.run_id.clone(),
            material_unit_id: material_ids[0].clone(),
            direction: "input".into(),
            role: "reagent".into(),
            effect: "fully_consumed".into(),
            quantity: Some(LabQuantity::measured("8", "uL")),
            transformation_group: Some("well-a1".into()),
            expected_material_revision: Some(2),
        });
    store
        .commit_lab_transaction(consume_remaining)
        .await
        .unwrap();
    let depleted = store
        .get_lab_material_unit(&material_ids[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(depleted.quantity, LabQuantity::measured("0", "uL"));
    assert_eq!(depleted.lifecycle, "depleted");
    assert_eq!(depleted.availability, "available");
    store
        .update_wet_lab_run_status(&planned.run_id, RunStatus::Running)
        .await
        .unwrap();
    assert!(store
        .pin_wet_lab_run_protocol("p", &planned.run_id, &first.id)
        .await
        .is_err());
    assert_eq!(
        store
            .get_wet_lab_run(&planned.run_id)
            .await
            .unwrap()
            .unwrap()
            .protocol_revision_id
            .as_deref(),
        Some(second.id.as_str())
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn conversation_is_bound_to_exactly_one_themed_wet_lab_run() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_conversation_wet_lab_run_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store.create_project("other", "Other", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    store
        .create_frame("conversation-1", "p", "OPERON", "wisp")
        .await
        .unwrap();
    store
        .create_frame("conversation-2", "p", "OPERON", "wisp")
        .await
        .unwrap();
    store
        .create_frame("other-conversation", "other", "OPERON", "wisp")
        .await
        .unwrap();

    let first = store
        .create_wet_lab_run(
            "p",
            "lab",
            "conversation-run-1",
            "RNA extraction",
            Some("conversation-1"),
            None,
        )
        .await
        .unwrap();
    let bound = store
        .get_conversation_wet_lab_run("p", "conversation-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bound.run.id, first.run_id);
    assert_eq!(bound.run.frame_id.as_deref(), Some("conversation-1"));
    assert_eq!(bound.wet_lab_run.display_id, "RUN-000001");

    let retry = store
        .create_wet_lab_run(
            "p",
            "lab",
            "conversation-run-1",
            "RNA extraction",
            Some("conversation-1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(retry.run_id, first.run_id);
    assert!(store
        .create_wet_lab_run(
            "p",
            "lab",
            "conversation-run-2",
            "Second experiment",
            Some("conversation-1"),
            None,
        )
        .await
        .is_err());
    assert!(store
        .create_wet_lab_run(
            "p",
            "lab",
            "conversation-run-3",
            "Wrong project",
            Some("other-conversation"),
            None,
        )
        .await
        .is_err());
    assert!(store
        .get_conversation_wet_lab_run("p", "conversation-2")
        .await
        .unwrap()
        .is_none());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn project_with_wet_lab_history_cannot_be_hard_deleted() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_project_lab_guard_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    let run = store
        .create_wet_lab_run("p", "lab", "guard-run", "Tracked experiment", None, None)
        .await
        .unwrap();
    let error = store.delete_project("p").await.unwrap_err().to_string();
    assert!(error.contains(&run.display_id), "{error}");
    assert!(store.get_project("p").await.unwrap().is_some());
    assert!(store.get_wet_lab_run(&run.run_id).await.unwrap().is_some());
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn realized_run_output_sample_and_participant_are_one_transaction() {
    let tmp = std::env::temp_dir().join(format!("wisp_run_output_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    let run = store
        .create_wet_lab_run("p", "lab", "output-run", "RNA extraction", None, None)
        .await
        .unwrap();

    let mut request = LabTransactionRequest::new("lab", "output-1", LabActorKind::User);
    request.project_id = Some("p".into());
    request.run_output_creates.push(LabRunOutputCreate {
        run_id: run.run_id.clone(),
        entity: LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "SMP".into(),
            title: "M23 RNA".into(),
            subtype: Some("rna".into()),
            metadata_json: "{}".into(),
            project_relation: Some("produced_by".into()),
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: "sample".into(),
                quantity: LabQuantity::measured("8.5", "uL"),
                vessel_description: Some("PCR tube".into()),
                availability: "available".into(),
                origin_kind: "prepared".into(),
            }),
            location: None,
            subject: None,
        },
        role: "product".into(),
        effect: "produced".into(),
        quantity: Some(LabQuantity::measured("8.5", "uL")),
        transformation_group: Some("M23".into()),
        initial_location_id: None,
    });
    let receipt = store.commit_lab_transaction(request).await.unwrap();
    assert_eq!(receipt.created_entities.len(), 1);
    assert_eq!(receipt.events.len(), 2);
    assert_eq!(receipt.events[0].kind, "entity_created");
    assert_eq!(receipt.events[1].kind, "run_participant_recorded");
    let output_id = &receipt.created_entities[0].id;
    assert_eq!(
        store
            .get_lab_material_unit(output_id)
            .await
            .unwrap()
            .unwrap()
            .origin_kind,
        "prepared"
    );
    let participants = store
        .list_wet_lab_run_participants(&run.run_id)
        .await
        .unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].material_unit_id, *output_id);
    assert_eq!(participants[0].direction, "output");
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn reservations_convert_units_reject_overallocation_and_release_on_close() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_reservation_lifecycle_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    let run_a = store
        .create_wet_lab_run("p", "lab", "reserve-run-a", "Run A", None, None)
        .await
        .unwrap();
    let run_b = store
        .create_wet_lab_run("p", "lab", "reserve-run-b", "Run B", None, None)
        .await
        .unwrap();
    let mut material_request =
        LabTransactionRequest::new("lab", "reserve-material", LabActorKind::User);
    material_request.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::MaterialUnit,
        prefix: "MAT".into(),
        title: "Shared stock".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: Some(LabMaterialUnitCreate {
            lot_id: None,
            usage_class: "inventory".into(),
            quantity: LabQuantity::measured("10", "uL"),
            vessel_description: None,
            availability: "available".into(),
            origin_kind: "prepared".into(),
        }),
        location: None,
        subject: None,
    });
    let material_id = store
        .commit_lab_transaction(material_request)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();

    let mut reserve_a = LabTransactionRequest::new("lab", "reserve-a", LabActorKind::User);
    reserve_a.project_id = Some("p".into());
    reserve_a.reservation_creates.push(LabReservationCreate {
        run_id: run_a.run_id.clone(),
        material_unit_id: material_id.clone(),
        quantity: LabQuantity::measured("0.004", "mL"),
        expires_at: None,
    });
    store.commit_lab_transaction(reserve_a).await.unwrap();
    assert_eq!(
        store
            .list_active_material_reservations(&material_id)
            .await
            .unwrap()
            .len(),
        1
    );

    let mut below_reserved =
        LabTransactionRequest::new("lab", "adjust-below-reserved", LabActorKind::User);
    below_reserved.project_id = Some("p".into());
    below_reserved
        .material_adjustments
        .push(LabMaterialAdjustment {
            material_unit_id: material_id.clone(),
            expected_revision: 1,
            quantity: LabQuantity::measured("3", "uL"),
            lifecycle: None,
            availability: None,
            identity_state: None,
            occurred_at: 10,
            reason: "invalid reconciliation".into(),
            prior_event_id: None,
        });
    let error = store
        .commit_lab_transaction(below_reserved)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("below active reservations"), "{error}");

    let mut incompatible =
        LabTransactionRequest::new("lab", "adjust-incompatible-unit", LabActorKind::User);
    incompatible.project_id = Some("p".into());
    incompatible
        .material_adjustments
        .push(LabMaterialAdjustment {
            material_unit_id: material_id.clone(),
            expected_revision: 1,
            quantity: LabQuantity::measured("10", "mg"),
            lifecycle: None,
            availability: None,
            identity_state: None,
            occurred_at: 11,
            reason: "invalid dimension change".into(),
            prior_event_id: None,
        });
    let error = store
        .commit_lab_transaction(incompatible)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("cannot change quantity dimension"),
        "{error}"
    );
    assert_eq!(
        store
            .get_lab_material_unit(&material_id)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        LabQuantity::measured("10", "uL")
    );

    let mut over = LabTransactionRequest::new("lab", "reserve-over", LabActorKind::User);
    over.project_id = Some("p".into());
    over.reservation_creates.push(LabReservationCreate {
        run_id: run_b.run_id,
        material_unit_id: material_id.clone(),
        quantity: LabQuantity::measured("7", "uL"),
        expires_at: None,
    });
    assert!(store.commit_lab_transaction(over).await.is_err());
    let mut start = LabTransactionRequest::new("lab", "start-reserved-run", LabActorKind::User);
    start.project_id = Some("p".into());
    start.run_status_updates.push(LabRunStatusUpdate {
        run_id: run_a.run_id.clone(),
        status: RunStatus::Running,
    });
    let start_receipt = store.commit_lab_transaction(start).await.unwrap();
    assert_eq!(start_receipt.events[0].kind, "run_status_changed");

    let mut cancel = LabTransactionRequest::new("lab", "cancel-reserved-run", LabActorKind::User);
    cancel.project_id = Some("p".into());
    cancel.confirmation_json = r#"{"closeout":{"confirmed":true}}"#.into();
    cancel.run_status_updates.push(LabRunStatusUpdate {
        run_id: run_a.run_id.clone(),
        status: RunStatus::Cancelled,
    });
    let replay = cancel.clone();
    let cancel_receipt = store.commit_lab_transaction(cancel).await.unwrap();
    assert_eq!(cancel_receipt.events[0].kind, "run_status_changed");
    assert_eq!(
        cancel_receipt.transaction.confirmation_json,
        r#"{"closeout":{"confirmed":true}}"#
    );
    let replay_receipt = store.commit_lab_transaction(replay).await.unwrap();
    assert!(replay_receipt.idempotent);
    assert_eq!(
        replay_receipt.transaction.id, cancel_receipt.transaction.id,
        "status retries must replay the original LabTransaction"
    );
    assert_eq!(replay_receipt.events, cancel_receipt.events);
    assert!(store
        .list_active_material_reservations(&material_id)
        .await
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn qc_events_feed_entity_provenance_without_changing_run_status() {
    let tmp = std::env::temp_dir().join(format!("wisp_lab_qc_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    let entity = store
        .create_lab_entity(
            "lab",
            LabEntityKind::ResourceDefinition,
            "AB",
            "anti-CD3",
            None,
            None,
        )
        .await
        .unwrap();
    let mut observation = LabTransactionRequest::new("lab", "qc-observation", LabActorKind::User);
    observation
        .qc_observation_creates
        .push(LabQcObservationCreate {
            entity_id: entity.id.clone(),
            run_id: None,
            method_revision_id: None,
            measurement_json: r#"{"mycoplasma":"negative"}"#.into(),
            evidence_json: r#"{"uri":"file:///qc/report.pdf"}"#.into(),
            observed_at: 10,
        });
    let receipt = store.commit_lab_transaction(observation).await.unwrap();
    let observation_id = serde_json::from_str::<serde_json::Value>(&receipt.events[0].payload_json)
        .unwrap()["observation_id"]
        .as_str()
        .unwrap()
        .to_string();
    for (index, verdict) in ["pending", "fail", "conditional", "pass", "not_applicable"]
        .into_iter()
        .enumerate()
    {
        let mut assessment =
            LabTransactionRequest::new("lab", format!("qc-assessment-{index}"), LabActorKind::User);
        assessment
            .qc_assessment_creates
            .push(LabQcAssessmentCreate {
                entity_id: entity.id.clone(),
                observation_ids: vec![observation_id.clone()],
                criteria_json: r#"{"mycoplasma":"negative"}"#.into(),
                verdict: verdict.into(),
                rationale: format!("Assessment {index}"),
            });
        store.commit_lab_transaction(assessment).await.unwrap();
    }
    let mut invalid =
        LabTransactionRequest::new("lab", "qc-assessment-invalid", LabActorKind::User);
    invalid.qc_assessment_creates.push(LabQcAssessmentCreate {
        entity_id: entity.id.clone(),
        observation_ids: vec![observation_id],
        criteria_json: "{}".into(),
        verdict: "inconclusive".into(),
        rationale: "Legacy value".into(),
    });
    assert!(store.commit_lab_transaction(invalid).await.is_err());
    let assessment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM lab_qc_assessments WHERE entity_id=?")
            .bind(&entity.id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(assessment_count, 5, "reassessment must append history");
    assert!(
        sqlx::query("UPDATE lab_qc_assessments SET verdict='inconclusive' WHERE entity_id=?",)
            .bind(&entity.id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    let provenance = store.lab_entity_provenance(&entity.id).await.unwrap();
    assert_eq!(provenance.qc_observation_count, 1);
    assert!(matches!(
        provenance.latest_qc_verdict.as_deref(),
        Some("pending" | "fail" | "conditional" | "pass" | "not_applicable")
    ));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn project_owned_wet_lab_runs_reject_cross_project_transactions() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_cross_project_wet_run_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for project_id in ["project-a", "project-b"] {
        store
            .create_project(project_id, project_id, "")
            .await
            .unwrap();
    }
    store
        .create_lab_registry(&LabRegistry::new("lab", "Shared lab", None).unwrap())
        .await
        .unwrap();
    for project_id in ["project-a", "project-b"] {
        store
            .link_project_lab_registry(project_id, "lab")
            .await
            .unwrap();
    }
    let run = store
        .create_wet_lab_run(
            "project-b",
            "lab",
            "project-b-run",
            "Project B experiment",
            None,
            None,
        )
        .await
        .unwrap();

    let mut material_request =
        LabTransactionRequest::new("lab", "shared-material", LabActorKind::User);
    material_request.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::MaterialUnit,
        prefix: "MAT".into(),
        title: "Shared stock".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: Some(LabMaterialUnitCreate {
            lot_id: None,
            usage_class: "inventory".into(),
            quantity: LabQuantity::measured("10", "uL"),
            vessel_description: None,
            availability: "available".into(),
            origin_kind: "prepared".into(),
        }),
        location: None,
        subject: None,
    });
    let material_id = store
        .commit_lab_transaction(material_request)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();

    let protocol = store
        .create_lab_entity(
            "lab",
            LabEntityKind::ProtocolSource,
            "PRT",
            "Shared protocol",
            None,
            None,
        )
        .await
        .unwrap();
    let revision = store
        .publish_lab_protocol_revision("lab", &protocol.id, "# Protocol")
        .await
        .unwrap();

    let mut participant =
        LabTransactionRequest::new("lab", "cross-project-participant", LabActorKind::User);
    participant.project_id = Some("project-a".into());
    participant.run_participants.push(LabRunParticipantCreate {
        run_id: run.run_id.clone(),
        material_unit_id: material_id.clone(),
        direction: "input".into(),
        role: "reagent".into(),
        effect: "partially_consumed".into(),
        quantity: Some(LabQuantity::measured("1", "uL")),
        transformation_group: None,
        expected_material_revision: Some(1),
    });
    assert!(store.commit_lab_transaction(participant).await.is_err());

    let mut output = LabTransactionRequest::new("lab", "cross-project-output", LabActorKind::User);
    output.project_id = Some("project-a".into());
    output.run_output_creates.push(LabRunOutputCreate {
        run_id: run.run_id.clone(),
        entity: LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "SMP".into(),
            title: "Cross-project output".into(),
            subtype: None,
            metadata_json: "{}".into(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: "sample".into(),
                quantity: LabQuantity::measured("1", "uL"),
                vessel_description: None,
                availability: "available".into(),
                origin_kind: "prepared".into(),
            }),
            location: None,
            subject: None,
        },
        role: "product".into(),
        effect: "produced".into(),
        quantity: Some(LabQuantity::measured("1", "uL")),
        transformation_group: None,
        initial_location_id: None,
    });
    assert!(store.commit_lab_transaction(output).await.is_err());

    let mut deviation =
        LabTransactionRequest::new("lab", "cross-project-deviation", LabActorKind::User);
    deviation.project_id = Some("project-a".into());
    deviation.run_deviation_creates.push(LabRunDeviationCreate {
        run_id: run.run_id.clone(),
        step_ref: None,
        description: "Should not be attached".into(),
        impact: "unknown".into(),
        disposition: None,
        occurred_at: 10,
    });
    assert!(store.commit_lab_transaction(deviation).await.is_err());

    let mut pin = LabTransactionRequest::new("lab", "cross-project-pin", LabActorKind::User);
    pin.project_id = Some("project-a".into());
    pin.run_protocol_pins.push(LabRunProtocolPin {
        run_id: run.run_id.clone(),
        protocol_revision_id: revision.id,
    });
    assert!(store.commit_lab_transaction(pin).await.is_err());

    let mut reservation =
        LabTransactionRequest::new("lab", "cross-project-reservation", LabActorKind::User);
    reservation.project_id = Some("project-a".into());
    reservation.reservation_creates.push(LabReservationCreate {
        run_id: run.run_id.clone(),
        material_unit_id: material_id.clone(),
        quantity: LabQuantity::measured("1", "uL"),
        expires_at: None,
    });
    assert!(store.commit_lab_transaction(reservation).await.is_err());

    assert!(store
        .list_wet_lab_run_participants(&run.run_id)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_lab_run_deviations(&run.run_id)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_active_material_reservations(&material_id)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .get_wet_lab_run(&run.run_id)
        .await
        .unwrap()
        .unwrap()
        .protocol_revision_id
        .is_none());
    assert_eq!(
        store
            .get_lab_material_unit(&material_id)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        LabQuantity::measured("10", "uL")
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn qc_observations_reject_cross_scope_run_and_method_links() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_cross_scope_qc_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for (project_id, registry_id) in [("project-a", "lab-a"), ("project-b", "lab-b")] {
        store
            .create_project(project_id, project_id, "")
            .await
            .unwrap();
        store
            .create_lab_registry(&LabRegistry::new(registry_id, registry_id, None).unwrap())
            .await
            .unwrap();
        store
            .link_project_lab_registry(project_id, registry_id)
            .await
            .unwrap();
    }
    let tested_entity = store
        .create_lab_entity(
            "lab-a",
            LabEntityKind::ResourceDefinition,
            "AB",
            "Tested entity",
            None,
            None,
        )
        .await
        .unwrap();
    let foreign_run = store
        .create_wet_lab_run(
            "project-b",
            "lab-b",
            "foreign-run",
            "Foreign experiment",
            None,
            None,
        )
        .await
        .unwrap();
    let foreign_protocol = store
        .create_lab_entity(
            "lab-b",
            LabEntityKind::ProtocolSource,
            "PRT",
            "Foreign QC method",
            None,
            None,
        )
        .await
        .unwrap();
    let foreign_revision = store
        .publish_lab_protocol_revision("lab-b", &foreign_protocol.id, "# Foreign method")
        .await
        .unwrap();

    let observation = |command_id: &str, run_id: Option<String>, method_revision_id| {
        let mut request = LabTransactionRequest::new("lab-a", command_id, LabActorKind::User);
        request.project_id = Some("project-a".into());
        request.qc_observation_creates.push(LabQcObservationCreate {
            entity_id: tested_entity.id.clone(),
            run_id,
            method_revision_id,
            measurement_json: "{}".into(),
            evidence_json: "{}".into(),
            observed_at: 10,
        });
        request
    };
    let run_error = store
        .commit_lab_transaction(observation(
            "cross-scope-qc-run",
            Some(foreign_run.run_id),
            None,
        ))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        run_error.contains("another Project or registry"),
        "{run_error}"
    );

    let method_error = store
        .commit_lab_transaction(observation(
            "cross-scope-qc-method",
            None,
            Some(foreign_revision.id),
        ))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        method_error.contains("method revision is missing or belongs to another registry"),
        "{method_error}"
    );
    assert_eq!(
        store
            .lab_entity_provenance(&tested_entity.id)
            .await
            .unwrap()
            .qc_observation_count,
        0
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn evidence_and_amendment_display_ids_are_registry_scoped() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_registry_scoped_aux_ids_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    let mut evidence_display_ids = vec![];
    let mut amendment_display_ids = vec![];

    for suffix in ["a", "b"] {
        let project_id = format!("project-{suffix}");
        let registry_id = format!("lab-{suffix}");
        store
            .create_project(&project_id, &project_id, "")
            .await
            .unwrap();
        store
            .create_lab_registry(&LabRegistry::new(&registry_id, &registry_id, None).unwrap())
            .await
            .unwrap();
        store
            .link_project_lab_registry(&project_id, &registry_id)
            .await
            .unwrap();
        let run = store
            .create_wet_lab_run(
                &project_id,
                &registry_id,
                &format!("run-{suffix}"),
                "Tracked experiment",
                None,
                None,
            )
            .await
            .unwrap();

        let mut evidence = LabTransactionRequest::new(
            &registry_id,
            format!("evidence-{suffix}"),
            LabActorKind::User,
        );
        evidence.project_id = Some(project_id.clone());
        evidence.data_evidence_creates.push(LabDataEvidenceCreate {
            owner_project_id: Some(project_id.clone()),
            owner_registry_id: None,
            producing_run_id: Some(run.run_id.clone()),
            role: "raw_data".into(),
            uri: format!("file:///raw/{suffix}.dat"),
            format: None,
            size_bytes: None,
            checksum_sha256: None,
            origin: "instrument".into(),
            manifest_json: "{}".into(),
        });
        store.commit_lab_transaction(evidence).await.unwrap();
        let evidence_rows = store.list_lab_run_data_evidence(&run.run_id).await.unwrap();
        assert_eq!(evidence_rows[0].registry_id, registry_id);
        evidence_display_ids.push(evidence_rows[0].display_id.clone());

        let mut deviation = LabTransactionRequest::new(
            &registry_id,
            format!("deviation-{suffix}"),
            LabActorKind::User,
        );
        deviation.project_id = Some(project_id.clone());
        deviation.run_deviation_creates.push(LabRunDeviationCreate {
            run_id: run.run_id.clone(),
            step_ref: None,
            description: "Recorded deviation".into(),
            impact: "minor".into(),
            disposition: None,
            occurred_at: 10,
        });
        let deviation_receipt = store.commit_lab_transaction(deviation).await.unwrap();
        store
            .update_wet_lab_run_status(&run.run_id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_wet_lab_run_status(&run.run_id, RunStatus::Failed)
            .await
            .unwrap();

        let mut amendment = LabTransactionRequest::new(
            &registry_id,
            format!("amendment-{suffix}"),
            LabActorKind::User,
        );
        amendment.project_id = Some(project_id);
        amendment.amendment_creates.push(LabAmendmentCreate {
            original_event_id: deviation_receipt.events[0].id.clone(),
            reason: "Correct after review".into(),
            correction_json: r#"{"impact":"major"}"#.into(),
        });
        store.commit_lab_transaction(amendment).await.unwrap();
        amendment_display_ids.push(
            store.list_lab_run_amendments(&run.run_id).await.unwrap()[0]
                .display_id
                .clone(),
        );
    }

    assert_eq!(evidence_display_ids, vec!["DAT-000001", "DAT-000001"]);
    assert_eq!(amendment_display_ids, vec!["AMD-000001", "AMD-000001"]);
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn material_derivation_is_atomic_conserves_quantity_and_creates_new_ids() {
    let tmp = std::env::temp_dir().join(format!("wisp_derivation_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store
        .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
        .await
        .unwrap();
    store.link_project_lab_registry("p", "lab").await.unwrap();
    let run = store
        .create_wet_lab_run("p", "lab", "derive-run", "Aliquot", None, None)
        .await
        .unwrap();
    let mut stock_request = LabTransactionRequest::new("lab", "derive-stock", LabActorKind::User);
    stock_request.entity_creates.push(LabEntityCreate {
        kind: LabEntityKind::MaterialUnit,
        prefix: "MAT".into(),
        title: "Cell suspension".into(),
        subtype: None,
        metadata_json: "{}".into(),
        project_relation: None,
        resource_definition: None,
        aliases: vec![],
        lot: None,
        material_unit: Some(LabMaterialUnitCreate {
            lot_id: None,
            usage_class: "sample".into(),
            quantity: LabQuantity::measured("10", "uL"),
            vessel_description: None,
            availability: "available".into(),
            origin_kind: "prepared".into(),
        }),
        location: None,
        subject: None,
    });
    let stock_id = store
        .commit_lab_transaction(stock_request)
        .await
        .unwrap()
        .created_entities[0]
        .id
        .clone();
    let output = |title: &str, value: &str| LabRunOutputCreate {
        run_id: run.run_id.clone(),
        entity: LabEntityCreate {
            kind: LabEntityKind::MaterialUnit,
            prefix: "SMP".into(),
            title: title.into(),
            subtype: None,
            metadata_json: "{}".into(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: Some(LabMaterialUnitCreate {
                lot_id: None,
                usage_class: "sample".into(),
                quantity: LabQuantity::measured(value, "uL"),
                vessel_description: None,
                availability: "available".into(),
                origin_kind: "prepared".into(),
            }),
            location: None,
            subject: None,
        },
        role: "product".into(),
        effect: "produced".into(),
        quantity: Some(LabQuantity::measured(value, "uL")),
        transformation_group: None,
        initial_location_id: None,
    };
    let input = |revision, value: &str| LabRunParticipantCreate {
        run_id: run.run_id.clone(),
        material_unit_id: stock_id.clone(),
        direction: "input".into(),
        role: "sample".into(),
        effect: "partially_consumed".into(),
        quantity: Some(LabQuantity::measured(value, "uL")),
        transformation_group: None,
        expected_material_revision: Some(revision),
    };
    let mut request = LabTransactionRequest::new("lab", "aliquot-good", LabActorKind::User);
    request.project_id = Some("p".into());
    request
        .derivation_creates
        .push(LabMaterialDerivationCreate {
            run_id: run.run_id.clone(),
            operation: "aliquot".into(),
            inputs: vec![input(1, "4")],
            outputs: vec![output("Aliquot A", "2"), output("Aliquot B", "2")],
        });
    let receipt = store.commit_lab_transaction(request).await.unwrap();
    assert_eq!(receipt.created_entities.len(), 2);
    assert_ne!(
        receipt.created_entities[0].id,
        receipt.created_entities[1].id
    );
    assert_eq!(
        store
            .get_lab_material_unit(&stock_id)
            .await
            .unwrap()
            .unwrap()
            .quantity
            .value
            .as_deref(),
        Some("6")
    );
    assert_eq!(
        store
            .list_material_derivations(&stock_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let mut invalid = LabTransactionRequest::new("lab", "aliquot-invalid", LabActorKind::User);
    invalid.project_id = Some("p".into());
    invalid
        .derivation_creates
        .push(LabMaterialDerivationCreate {
            run_id: run.run_id.clone(),
            operation: "split".into(),
            inputs: vec![input(2, "2")],
            outputs: vec![output("Wrong", "1")],
        });
    assert!(store.commit_lab_transaction(invalid).await.is_err());
    assert_eq!(
        store
            .get_lab_material_unit(&stock_id)
            .await
            .unwrap()
            .unwrap()
            .quantity
            .value
            .as_deref(),
        Some("6")
    );
    assert_eq!(
        store
            .list_material_derivations(&stock_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let mut observed_input = input(2, "2");
    observed_input.effect = "observed".into();
    let mut non_consuming =
        LabTransactionRequest::new("lab", "aliquot-observed-input", LabActorKind::User);
    non_consuming.project_id = Some("p".into());
    non_consuming
        .derivation_creates
        .push(LabMaterialDerivationCreate {
            run_id: run.run_id.clone(),
            operation: "split".into(),
            inputs: vec![observed_input],
            outputs: vec![output("Impossible duplicate", "2")],
        });
    let error = store
        .commit_lab_transaction(non_consuming)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("must consume, transform, or sample"),
        "{error}"
    );
    assert_eq!(
        store
            .get_lab_material_unit(&stock_id)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        LabQuantity::measured("6", "uL")
    );
    assert_eq!(
        store
            .list_material_derivations(&stock_id)
            .await
            .unwrap()
            .len(),
        2,
        "a rejected non-consuming derivation must not create lineage"
    );
    let _ = std::fs::remove_file(tmp);
}
