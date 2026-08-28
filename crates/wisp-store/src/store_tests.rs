use super::*;

fn nested_test_step(
    id: &str,
    workflow_id: &str,
    allow_delegation: bool,
    max_tokens: u64,
) -> AgentWorkflowStep {
    let mut step = AgentWorkflowStep::new(
        id,
        workflow_id,
        0,
        id,
        "temporary",
        "local",
        "bounded test prompt",
    )
    .unwrap();
    step.spec_json = serde_json::json!({"allow_delegation": allow_delegation}).to_string();
    step.budget_json = serde_json::json!({
        "max_tokens": max_tokens,
        "max_tool_calls": 1,
        "max_cost_microunits": 1,
    })
    .to_string();
    step
}

async fn create_running_nested_test_root(
    store: &Store,
    limits: AgentDelegationRootLimits,
    allow_delegation: bool,
    max_tokens: u64,
) -> AgentWorkflowAttempt {
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("root-frame", "p", "OPERON", "m")
        .await
        .unwrap();
    let mut workflow = AgentWorkflow::new("root", "p", "workspace", "Root batch").unwrap();
    workflow.frame_id = Some("root-frame".into());
    workflow.plan_json = serde_json::json!({"schema_version": 2}).to_string();
    workflow.root_limits_json = serde_json::to_string(&limits).unwrap();
    workflow.max_parallel = i64::from(limits.max_parallel);
    let step = nested_test_step("root-step", "root", allow_delegation, max_tokens);
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("root", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "root",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let mut attempt = AgentWorkflowAttempt::queued(
        "root-attempt",
        "root",
        "root-step",
        1,
        "root-request",
        "local",
        "{}",
    )
    .unwrap();
    attempt.allow_delegation = allow_delegation;
    let AgentWorkflowAttemptStart::Started(attempt) = store
        .try_create_started_agent_workflow_attempt(attempt)
        .await
        .unwrap()
    else {
        panic!("root attempt should start");
    };
    assert!(store
        .set_running_agent_workflow_attempt_provenance("root-request", None, "root-child-frame",)
        .await
        .unwrap());
    store
        .get_agent_workflow_attempt(&attempt.id)
        .await
        .unwrap()
        .unwrap()
}

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
    let sequenced = store.load_messages_with_seq("f1").await.unwrap();
    assert_eq!(
        sequenced.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(sequenced[1].1.content.as_text(), "hello");

    // Durability: close every connection and reopen the same file — the
    // writes must come back from disk, not from the live pool's cache.
    store.pool.close().await;
    let store = Store::open(&tmp).await.unwrap();
    let msgs = store.load_messages("f1").await.unwrap();
    assert_eq!(msgs.len(), 2, "messages must survive close + reopen");
    assert_eq!(msgs[0].content.as_text(), "hi");
    assert_eq!(msgs[1].content.as_text(), "hello");

    // list_sessions derives a title from the first user message and skips
    // untitled frames with no user turn. Named unused drafts are covered in
    // `named_draft_is_listable_but_not_resumable`.
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
async fn named_draft_is_listable_but_not_resumable() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_named_draft_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "proj", "").await.unwrap();
    store
        .create_frame("used", "p1", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("used", 0, &Message::user("hello"))
        .await
        .unwrap();

    store
        .create_frame("untitled", "p1", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("untitled", 0, &Message::system("only system"))
        .await
        .unwrap();
    assert_eq!(
        store.list_sessions("p1").await.unwrap().len(),
        1,
        "untitled empty draft stays hidden"
    );
    assert_eq!(
        store.latest_used_session_id("p1").await.unwrap().as_deref(),
        Some("used")
    );

    store
        .create_frame("draft", "p1", "OPERON", "m")
        .await
        .unwrap();
    store
        .rename_session("draft", "p1", "Named draft")
        .await
        .unwrap();
    let sessions = store.list_sessions("p1").await.unwrap();
    assert_eq!(sessions.len(), 2, "named draft joins the used session");
    assert!(sessions
        .iter()
        .any(|row| row.0 == "draft" && row.1 == "Named draft"));
    assert!(sessions.iter().any(|row| row.0 == "used"));

    let projs = store.list_projects().await.unwrap();
    let p1 = projs.iter().find(|p| p.0 == "p1").unwrap();
    assert_eq!(p1.5, 2, "named draft counts; untitled empty does not");

    let found = store
        .search_sessions(None, "named draft", 10, None, None)
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "named draft must be searchable by title");
    assert_eq!(found[0].id, "draft");
    let all = store
        .search_sessions(None, "", 10, None, None)
        .await
        .unwrap();
    assert!(
        all.iter().all(|s| s.id != "untitled"),
        "untitled message-less frame stays unsearchable"
    );

    assert!(
        store
            .list_recent_sessions_detail(10)
            .await
            .unwrap()
            .iter()
            .all(|row| row.id != "draft"),
        "named unused draft is not recent activity"
    );
    assert_eq!(
        store.latest_used_session_id("p1").await.unwrap().as_deref(),
        Some("used"),
        "rename must not steal resume from a used conversation"
    );

    store.delete_session("used", "p1").await.unwrap();
    assert_eq!(
        store.latest_used_session_id("p1").await.unwrap(),
        None,
        "a project that only has a named unused draft has nothing to resume"
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn duplicate_seq_in_a_frame_is_rejected_and_leaves_rows_unchanged() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_store_dupseq_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "proj", "").await.unwrap();
    store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 0, &Message::user("first"))
        .await
        .unwrap();

    // The schema enforces UNIQUE(frame_id, seq): a second append with the
    // same explicit seq must error instead of silently replacing the row.
    let err = store
        .append_message("f1", 0, &Message::user("imposter"))
        .await
        .expect_err("duplicate (frame_id, seq) must be rejected");
    assert!(
        err.to_string().to_ascii_lowercase().contains("unique"),
        "unexpected error: {err}"
    );

    let msgs = store.load_messages_with_seq("f1").await.unwrap();
    assert_eq!(msgs.len(), 1, "the failed insert must not add a row");
    assert_eq!(msgs[0].0, 0);
    assert_eq!(msgs[0].1.content.as_text(), "first");

    // The same seq in a different frame is fine — uniqueness is per frame.
    store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
    store
        .append_message("f2", 0, &Message::user("other frame"))
        .await
        .unwrap();

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn concurrent_writers_on_two_pools_all_land() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_concurrent_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store_a = Store::open(&tmp).await.unwrap();
    let store_b = Store::open(&tmp).await.unwrap();
    store_a.create_project("p1", "proj", "").await.unwrap();
    store_a
        .create_frame("fa", "p1", "OPERON", "m")
        .await
        .unwrap();
    store_a
        .create_frame("fb", "p1", "OPERON", "m")
        .await
        .unwrap();

    // Two independent pools on the same file, interleaving writes to
    // different frames — exercises WAL + busy_timeout instead of failing
    // with SQLITE_BUSY.
    const PER_WRITER: i64 = 20;
    let writer = |store: Store, frame: &'static str| async move {
        for seq in 0..PER_WRITER {
            store
                .append_message(frame, seq, &Message::user(format!("{frame} {seq}")))
                .await?;
            tokio::task::yield_now().await;
        }
        anyhow::Ok(())
    };
    let task_a = tokio::spawn(writer(store_a.clone(), "fa"));
    let task_b = tokio::spawn(writer(store_b.clone(), "fb"));
    task_a.await.unwrap().unwrap();
    task_b.await.unwrap().unwrap();

    for (store, frame) in [(&store_a, "fb"), (&store_b, "fa")] {
        let msgs = store.load_messages_with_seq(frame).await.unwrap();
        assert_eq!(
            msgs.len(),
            PER_WRITER as usize,
            "every {frame} append must land"
        );
        assert_eq!(
            msgs.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            (0..PER_WRITER).collect::<Vec<_>>()
        );
    }

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn replace_and_load_system_message() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_sysprompt_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p1", "proj", "").await.unwrap();
    store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
    store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::system("old prompt"))
        .await
        .unwrap();
    store
        .append_message("f1", 2, &Message::user("hello"))
        .await
        .unwrap();
    // f2 has no system message.
    store
        .append_message("f2", 1, &Message::user("no system"))
        .await
        .unwrap();

    let map = store
        .load_system_messages(&["f1".into(), "f2".into(), "missing".into()])
        .await
        .unwrap();
    assert_eq!(map.len(), 1, "only f1 has a system message: {map:?}");
    let content: wisp_llm::Content = serde_json::from_str(&map["f1"]).unwrap();
    assert_eq!(content.as_text(), "old prompt");

    assert!(store
        .replace_system_message("f1", &Message::system("new prompt"))
        .await
        .unwrap());
    let msgs = store.load_messages("f1").await.unwrap();
    assert_eq!(msgs[0].content.as_text(), "new prompt");
    assert_eq!(
        msgs[1].content.as_text(),
        "hello",
        "other messages untouched"
    );
    assert!(
        !store
            .replace_system_message("f2", &Message::system("nope"))
            .await
            .unwrap(),
        "frames without a system message report false"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn token_usage_folds_usage_events_into_root_sessions() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_token_usage_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("p", "Workspace A", "/workspace/a")
        .await
        .unwrap();
    store
        .create_project("p2", "Workspace B", "/workspace/b")
        .await
        .unwrap();
    store
        .create_project("scratch:usage", "Scratch", "/tmp/scratch")
        .await
        .unwrap();
    store
        .create_frame("root", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("root", 1, &Message::user("hello usage"))
        .await
        .unwrap();
    store
        .create_child_frame("child", "root", "p", "Sub", "m")
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp();
    let usage = |input: i64, output: i64, model: &str| {
        format!(
            "{{\"kind\":\"Usage\",\"frame_id\":\"x\",\"round\":1,\"model\":\"{model}\",\"created_at\":{now},\"input\":{input},\"output\":{output},\"reasoning\":1,\"cached\":2,\"ctx_tokens\":0,\"max_context\":0}}"
        )
    };
    store
        .append_session_ui_event("root", 1, &usage(100, 10, "model-a"))
        .await
        .unwrap();
    store
        .append_session_ui_event(
            "root",
            2,
            "{\"kind\":\"Text\",\"frame_id\":\"root\",\"delta\":\"hi\"}",
        )
        .await
        .unwrap();
    store
        .append_session_ui_event("child", 1, &usage(50, 5, "model-b"))
        .await
        .unwrap();
    store
        .create_frame("second", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("second", 1, &Message::user("second session"))
        .await
        .unwrap();
    store
        .append_session_ui_event("second", 1, &usage(25, 3, "model-a"))
        .await
        .unwrap();
    store
        .create_frame("other", "p2", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("other", 1, &Message::user("other workspace"))
        .await
        .unwrap();
    store
        .append_session_ui_event("other", 1, &usage(30, 4, "model-b"))
        .await
        .unwrap();
    store
        .create_frame("scratch", "scratch:usage", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_session_ui_event("scratch", 1, &usage(999, 999, "scratch-model"))
        .await
        .unwrap();
    let tool_call = |name: &str, preview: &str| {
        format!(
            "{{\"kind\":\"ToolCall\",\"frame_id\":\"x\",\"name\":\"{name}\",\"preview\":\"{preview}\"}}"
        )
    };
    store
        .append_session_ui_event("root", 3, &tool_call("use_skill", "bear-support"))
        .await
        .unwrap();
    store
        .append_session_ui_event("root", 4, &tool_call("use_skill", "bear-support"))
        .await
        .unwrap();
    store
        .append_session_ui_event("root", 5, &tool_call("use_skill", "bear-map"))
        .await
        .unwrap();
    store
        .append_session_ui_event(
            "root",
            6,
            &tool_call(
                "use_skill",
                "Skipped because 'ask_user' ended the turn before this call. Wait for the user's next message.",
            ),
        )
        .await
        .unwrap();
    store
        .append_session_ui_event("child", 2, &tool_call("mcp:pubmed_search", "query"))
        .await
        .unwrap();
    store
        .append_session_ui_event("other", 2, &tool_call("mcp:pubmed_search", "query"))
        .await
        .unwrap();
    store
        .append_session_ui_event("other", 3, &tool_call("mcp:web_fetch", "url"))
        .await
        .unwrap();
    store
        .append_session_ui_event("other", 4, &tool_call("shell", "ls"))
        .await
        .unwrap();
    store
        .append_session_ui_event("scratch", 2, &tool_call("use_skill", "scratch-skill"))
        .await
        .unwrap();
    // A session with no usage events must not appear at all.
    store
        .create_frame("quiet", "p", "OPERON", "m")
        .await
        .unwrap();

    let workspaces = store.token_usage_by_project().await.unwrap();
    assert_eq!(workspaces.len(), 2, "scratch usage stays out of Settings");
    let workspace = workspaces
        .iter()
        .find(|workspace| workspace.project_id == "p")
        .unwrap();
    assert_eq!(workspace.name, "Workspace A");
    assert_eq!(workspace.workspace_dir, "/workspace/a");
    assert_eq!(workspace.session_count, 2);
    assert_eq!(
        (
            workspace.input,
            workspace.output,
            workspace.reasoning,
            workspace.cached,
        ),
        (175, 18, 3, 6)
    );

    let first_page = store.token_usage_by_session("p", 0, 1).await.unwrap();
    let second_page = store.token_usage_by_session("p", 1, 1).await.unwrap();
    assert_eq!(first_page.total, 2);
    assert_eq!(first_page.items.len(), 1);
    assert_eq!(second_page.items.len(), 1);
    assert_ne!(first_page.items[0].id, second_page.items[0].id);
    let full_page = store.token_usage_by_session("p", 0, 20).await.unwrap();
    let row = full_page.items.iter().find(|row| row.id == "root").unwrap();
    assert_eq!(row.id, "root");
    assert_eq!(row.title, "hello usage");
    assert_eq!(
        (row.input, row.output, row.reasoning, row.cached),
        (150, 15, 2, 4)
    );

    let models = store.token_usage_by_model().await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(
        models
            .iter()
            .find(|model| model.model == "model-a")
            .unwrap()
            .tokens,
        138
    );
    assert_eq!(
        models
            .iter()
            .find(|model| model.model == "model-b")
            .unwrap()
            .tokens,
        89
    );
    assert_eq!(
        store
            .token_usage_activity()
            .await
            .unwrap()
            .iter()
            .map(|day| day.tokens)
            .sum::<i64>(),
        227
    );

    let tools = store.tool_call_usage_ranking().await.unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|row| (row.kind.as_str(), row.name.as_str(), row.calls))
            .collect::<Vec<_>>(),
        vec![
            ("mcp", "pubmed_search", 2),
            ("skill", "bear-support", 2),
            ("mcp", "web_fetch", 1),
            ("skill", "bear-map", 1),
        ]
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn child_agent_frames_stay_out_of_top_level_session_history() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_child_frames_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("root", "p", "OPERON", "model")
        .await
        .unwrap();
    store
        .append_message("root", 1, &Message::user("Parent conversation"))
        .await
        .unwrap();
    store
        .create_child_frame("child", "root", "p", "Research Agent", "model")
        .await
        .unwrap();
    store
        .append_message("child", 1, &Message::user("Delegated task"))
        .await
        .unwrap();
    store
        .create_child_frame("grandchild", "child", "p", "Nested Agent", "model")
        .await
        .unwrap();

    let lineage: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id,parent_frame_id,root_frame_id FROM frames \
         WHERE id IN ('child','grandchild') ORDER BY id",
    )
    .fetch_all(&store.pool)
    .await
    .unwrap();
    assert_eq!(
        lineage,
        vec![
            ("child".into(), "root".into(), "root".into()),
            ("grandchild".into(), "child".into(), "root".into()),
        ]
    );
    assert_eq!(
        store
            .list_sessions("p")
            .await
            .unwrap()
            .into_iter()
            .map(|session| session.0)
            .collect::<Vec<_>>(),
        ["root"]
    );

    store.delete_session("root", "p").await.unwrap();
    assert!(store.frame_project_id("child").await.unwrap().is_none());
    assert!(store
        .frame_project_id("grandchild")
        .await
        .unwrap()
        .is_none());
    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn frame_models_are_session_scoped() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_frame_models_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("first", "p", "OPERON", "m1")
        .await
        .unwrap();
    store
        .create_frame("second", "p", "OPERON", "m1")
        .await
        .unwrap();

    store.set_frame_model("first", "p", "m2").await.unwrap();

    assert_eq!(
        store.frame_model("first").await.unwrap().as_deref(),
        Some("m2")
    );
    assert_eq!(
        store.frame_model("second").await.unwrap().as_deref(),
        Some("m1")
    );
    assert!(store.set_frame_model("first", "other", "m3").await.is_err());
    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn frame_reasoning_effort_is_session_scoped_and_nullable() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_frame_reasoning_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("first", "p", "OPERON", "m1")
        .await
        .unwrap();
    store
        .create_frame("second", "p", "OPERON", "m1")
        .await
        .unwrap();

    store
        .set_frame_reasoning_effort("first", "p", Some("high"))
        .await
        .unwrap();
    store
        .set_frame_service_tier("first", "p", Some("priority"))
        .await
        .unwrap();
    assert_eq!(
        store.frame_reasoning_effort("first").await.unwrap(),
        Some("high".into())
    );
    assert_eq!(store.frame_reasoning_effort("second").await.unwrap(), None);
    assert_eq!(
        store.frame_service_tier("first").await.unwrap(),
        Some("priority".into())
    );
    assert_eq!(store.frame_service_tier("second").await.unwrap(), None);

    sqlx::query("INSERT INTO messages(id,frame_id,seq,role,ts) VALUES('msg','first',1,'user',1)")
        .execute(&store.pool)
        .await
        .unwrap();
    store
        .clone_exploration_frame("first", "exploration", 1, 0)
        .await
        .unwrap();
    assert_eq!(
        store.frame_reasoning_effort("exploration").await.unwrap(),
        Some("high".into()),
        "an exploration branch must preserve the conversation override"
    );
    assert_eq!(
        store.frame_service_tier("exploration").await.unwrap(),
        Some("priority".into()),
        "an exploration branch must preserve the Fast override"
    );

    assert!(store
        .set_frame_reasoning_effort("first", "other", Some("low"))
        .await
        .is_err());

    store
        .set_frame_reasoning_effort("first", "p", Some(""))
        .await
        .unwrap();
    assert_eq!(
        store.frame_reasoning_effort("first").await.unwrap(),
        Some(String::new()),
        "an explicit provider-default override must differ from inheritance"
    );
    store
        .set_frame_reasoning_effort("first", "p", None)
        .await
        .unwrap();
    assert_eq!(store.frame_reasoning_effort("first").await.unwrap(), None);

    store
        .set_frame_service_tier("first", "p", Some(""))
        .await
        .unwrap();
    assert_eq!(
        store.frame_service_tier("first").await.unwrap(),
        Some(String::new()),
        "explicit Fast-off must differ from inheritance"
    );
    store
        .set_frame_service_tier("first", "p", None)
        .await
        .unwrap();
    assert_eq!(store.frame_service_tier("first").await.unwrap(), None);

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_and_steps_roundtrip() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_workflow_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();

    let mut workflow = AgentWorkflow::new("wf", "p", "workspace-1", "review").unwrap();
    assert_eq!(workflow.mode, "manual");
    workflow.description = "Review an implementation with a second agent".into();
    let mut step = AgentWorkflowStep::new(
        "step-1",
        "wf",
        0,
        "reviewer",
        "reviewer",
        "acp",
        "Review {{input}}",
    )
    .unwrap();
    assert!(step.template_id.is_empty());
    step.permissions_json = r#"{"tools":["read_file"]}"#.into();
    store
        .create_agent_workflow_plan(&workflow, &[step.clone()])
        .await
        .unwrap();
    assert_eq!(
        store.list_agent_workflows("p").await.unwrap(),
        vec![workflow.clone()]
    );
    assert_eq!(
        store.list_agent_workflow_steps("wf").await.unwrap(),
        vec![step.clone()]
    );

    assert!(store.delete_agent_workflow("wf").await.unwrap());
    assert!(store.get_agent_workflow("wf").await.unwrap().is_none());
    assert!(store
        .list_agent_workflow_steps("wf")
        .await
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_plan_approval_is_versioned() {
    let tmp = std::env::temp_dir().join(format!("wisp_agent_plan_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    let mut workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegated analysis").unwrap();
    workflow.frame_id = Some("f".into());
    workflow.goal = "Analyze and review the dataset".into();
    workflow.plan_json = r#"{"mode":"manual","max_parallel":2}"#.into();
    let mut step =
        AgentWorkflowStep::new("code", "wf", 0, "code", "coder", "acp", "controlled prompt")
            .unwrap();
    step.spec_json = r#"{"capabilities":["code_run"]}"#.into();
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(!store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    let approved = store.get_agent_workflow("wf").await.unwrap().unwrap();
    assert_eq!(approved.status, AgentWorkflowStatus::Approved);
    assert_eq!(approved.version, 2);
    assert!(approved.approved_at.is_some());
    assert!(store.delete_agent_workflow("wf").await.unwrap());
    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_attempts_persist_cas_lifecycle_and_usage() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_attempt_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegated analysis").unwrap();
    let step = AgentWorkflowStep::new("code", "wf", 0, "code", "coder", "acp", "controlled prompt")
        .unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Running,
            AgentWorkflowStatus::Succeeded,
        )
        .await
        .is_err());

    let mut attempt = AgentWorkflowAttempt::queued(
        "attempt-1",
        "wf",
        "code",
        1,
        "request-1",
        "acp",
        r#"{"input":"data.csv"}"#,
    )
    .unwrap();
    store.create_agent_workflow_attempt(&attempt).await.unwrap();
    assert_eq!(
        store
            .next_agent_workflow_attempt_number("code")
            .await
            .unwrap(),
        2
    );

    attempt.status = AgentWorkflowAttemptStatus::Running;
    attempt.started_at = Some(chrono::Utc::now().timestamp());
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Queued)
        .await
        .unwrap());
    attempt.status = AgentWorkflowAttemptStatus::Succeeded;
    attempt.response_json = Some(r#"{"status":"succeeded"}"#.into());
    attempt.output_json = r#"{"summary":"completed"}"#.into();
    attempt.artifact_ids_json = r#"["artifact-1"]"#.into();
    attempt.evidence_json = r#"[{"kind":"test","summary":"passed"}]"#.into();
    attempt.agent_session_id = Some("agent-session-1".into());
    attempt.child_frame_id = Some("child-frame-1".into());
    attempt.input_tokens = 100;
    attempt.output_tokens = 50;
    attempt.tool_calls = 3;
    attempt.cost_microunits = 25;
    attempt.finished_at = Some(chrono::Utc::now().timestamp());
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Running)
        .await
        .unwrap());
    assert!(!store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Running)
        .await
        .unwrap());
    let persisted = store
        .get_agent_workflow_attempt("attempt-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, AgentWorkflowAttemptStatus::Succeeded);
    assert_eq!(persisted.output_json, attempt.output_json);
    assert_eq!(persisted.artifact_ids_json, attempt.artifact_ids_json);
    assert_eq!(persisted.agent_session_id, attempt.agent_session_id);
    assert_eq!(persisted.tool_calls, 3);

    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Running,
            AgentWorkflowStatus::Succeeded,
        )
        .await
        .unwrap());
    assert_eq!(
        store.list_agent_workflow_attempts("wf").await.unwrap(),
        vec![persisted]
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn interrupted_agent_workflows_recover_to_failed_terminal_state() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_recovery_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegation").unwrap();
    let step = AgentWorkflowStep::new("step", "wf", 0, "step", "coder", "acp", "prompt").unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let attempt =
        AgentWorkflowAttempt::queued("attempt", "wf", "step", 1, "request", "acp", r#"{}"#)
            .unwrap();
    store.create_agent_workflow_attempt(&attempt).await.unwrap();

    assert_eq!(
        store.recover_interrupted_agent_workflows().await.unwrap(),
        (1, 1)
    );
    let recovered = store
        .get_agent_workflow_attempt("attempt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, AgentWorkflowAttemptStatus::Failed);
    assert!(recovered.error.unwrap().contains("stopped"));
    assert_eq!(
        store
            .get_agent_workflow("wf")
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowStatus::Failed
    );
    assert_eq!(
        store.recover_interrupted_agent_workflows().await.unwrap(),
        (0, 0)
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn workflow_run_activity_waits_without_consuming_agent_capacity_and_reconciles() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_workflow_run_activity_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let mut workflow = AgentWorkflow::new("wf", "p", "workspace", "Method search").unwrap();
    workflow.max_parallel = 1;
    workflow.root_limits_json = serde_json::to_string(&AgentDelegationRootLimits {
        max_parallel: 1,
        ..AgentDelegationRootLimits::default()
    })
    .unwrap();
    let mut activity_step = AgentWorkflowStep::new(
        "activity",
        "wf",
        0,
        "activity",
        "run_activity",
        "local",
        "host activity",
    )
    .unwrap();
    activity_step.task_kind = "run_activity".into();
    activity_step.activity_json = serde_json::json!({"activity":"method_search"}).to_string();
    let agent_step =
        AgentWorkflowStep::new("agent", "wf", 1, "agent", "worker", "local", "prompt").unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[activity_step, agent_step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());

    let activity_attempt = AgentWorkflowAttempt::queued(
        "activity-attempt",
        "wf",
        "activity",
        1,
        "activity-request",
        "local",
        "{}",
    )
    .unwrap();
    let activity_attempt = match store
        .try_create_started_agent_workflow_attempt(activity_attempt)
        .await
        .unwrap()
    {
        AgentWorkflowAttemptStart::Started(value) => value,
        other => panic!("activity attempt did not start: {other:?}"),
    };
    let run = RunRecord::new("method-run", "p", "local", "Method search", "method_search");
    let link =
        AgentWorkflowRunActivity::new(&activity_attempt.id, &run.id, "method_search").unwrap();
    store
        .create_agent_workflow_run_activity(&run, &link)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_agent_workflow_attempt(&activity_attempt.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowAttemptStatus::WaitingRun
    );

    let agent_attempt = AgentWorkflowAttempt::queued(
        "agent-attempt",
        "wf",
        "agent",
        1,
        "agent-request",
        "local",
        "{}",
    )
    .unwrap();
    assert!(matches!(
        store
            .try_create_started_agent_workflow_attempt(agent_attempt)
            .await
            .unwrap(),
        AgentWorkflowAttemptStart::Started(_)
    ));

    assert!(store
        .activate_run_lifecycle("method-run", RunStatus::Running, "test-owner", 60)
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("method-run", "test-owner", RunStatus::Succeeded, Some(0))
        .await
        .unwrap());
    assert_eq!(
        store
            .reconcile_agent_workflow_run_activity(&activity_attempt.id)
            .await
            .unwrap(),
        Some(AgentWorkflowAttemptStatus::Succeeded)
    );
    assert_eq!(
        store
            .reconcile_agent_workflow_run_activity(&activity_attempt.id)
            .await
            .unwrap(),
        Some(AgentWorkflowAttemptStatus::Succeeded)
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn workflow_root_cancellation_cancels_linked_draft_run() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_workflow_linked_cancel_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Method search").unwrap();
    let mut activity_step = AgentWorkflowStep::new(
        "activity",
        "wf",
        0,
        "activity",
        "run_activity",
        "local",
        "host activity",
    )
    .unwrap();
    activity_step.task_kind = "run_activity".into();
    activity_step.activity_json = serde_json::json!({"activity":"method_search"}).to_string();
    let descendant =
        AgentWorkflowStep::new("report", "wf", 1, "report", "writer", "local", "report").unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[activity_step, descendant])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let attempt = AgentWorkflowAttempt::queued(
        "activity-attempt",
        "wf",
        "activity",
        1,
        "activity-request",
        "local",
        "{}",
    )
    .unwrap();
    let attempt = match store
        .try_create_started_agent_workflow_attempt(attempt)
        .await
        .unwrap()
    {
        AgentWorkflowAttemptStart::Started(attempt) => attempt,
        other => panic!("activity attempt did not start: {other:?}"),
    };
    let run = RunRecord::new("method-run", "p", "local", "Method search", "method_search");
    let link = AgentWorkflowRunActivity::new(&attempt.id, &run.id, "method_search").unwrap();
    store
        .create_agent_workflow_run_activity(&run, &link)
        .await
        .unwrap();

    assert_eq!(store.request_agent_workflow_cancel("wf").await.unwrap(), 1);
    assert_eq!(
        store.method_search_run_status("method-run").await.unwrap(),
        Some(RunStatus::Cancelled)
    );
    assert_eq!(
        store
            .reconcile_agent_workflow_run_activity(&attempt.id)
            .await
            .unwrap(),
        Some(AgentWorkflowAttemptStatus::Cancelled)
    );
    assert!(store
        .list_agent_workflow_attempts("wf")
        .await
        .unwrap()
        .iter()
        .all(|attempt| attempt.step_id != "report"));

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn workflow_run_activity_recovery_preserves_valid_link_and_fails_missing_link() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_workflow_run_recovery_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();

    for (workflow_id, with_link) in [("valid", true), ("invalid", false)] {
        let workflow = AgentWorkflow::new(workflow_id, "p", "workspace", "Method search").unwrap();
        let mut step = AgentWorkflowStep::new(
            format!("{workflow_id}-step"),
            workflow_id,
            0,
            "activity",
            "run_activity",
            "local",
            "host activity",
        )
        .unwrap();
        step.task_kind = "run_activity".into();
        step.activity_json = serde_json::json!({"activity":"method_search"}).to_string();
        store
            .create_agent_workflow_plan(&workflow, &[step.clone()])
            .await
            .unwrap();
        assert!(store
            .approve_agent_workflow_plan(workflow_id, 1)
            .await
            .unwrap());
        assert!(store
            .transition_agent_workflow_status(
                workflow_id,
                AgentWorkflowStatus::Approved,
                AgentWorkflowStatus::Running,
            )
            .await
            .unwrap());
        let attempt = AgentWorkflowAttempt::queued(
            format!("{workflow_id}-attempt"),
            workflow_id,
            &step.id,
            1,
            format!("{workflow_id}-request"),
            "local",
            "{}",
        )
        .unwrap();
        let mut attempt = match store
            .try_create_started_agent_workflow_attempt(attempt)
            .await
            .unwrap()
        {
            AgentWorkflowAttemptStart::Started(value) => value,
            other => panic!("activity attempt did not start: {other:?}"),
        };
        if with_link {
            let run = RunRecord::new(
                format!("{workflow_id}-run"),
                "p",
                "local",
                "Method search",
                "method_search",
            );
            let link =
                AgentWorkflowRunActivity::new(&attempt.id, &run.id, "method_search").unwrap();
            store
                .create_agent_workflow_run_activity(&run, &link)
                .await
                .unwrap();
        } else {
            attempt.status = AgentWorkflowAttemptStatus::WaitingRun;
            assert!(store
                .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Running)
                .await
                .unwrap());
        }
    }

    assert_eq!(
        store.recover_interrupted_agent_workflows().await.unwrap(),
        (1, 1)
    );
    assert_eq!(
        store
            .get_agent_workflow("valid")
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowStatus::Running
    );
    assert_eq!(
        store
            .get_agent_workflow_attempt("valid-attempt")
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowAttemptStatus::WaitingRun
    );
    assert_eq!(
        store
            .get_agent_workflow_attempt("invalid-attempt")
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowAttemptStatus::Failed
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn workflow_cancellation_is_persisted_and_cleared_for_retry() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_agent_cancel_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegation").unwrap();
    let step = AgentWorkflowStep::new("step", "wf", 0, "step", "coder", "acp", "prompt").unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let mut attempt =
        AgentWorkflowAttempt::queued("attempt", "wf", "step", 1, "request", "acp", r#"{}"#)
            .unwrap();
    store.create_agent_workflow_attempt(&attempt).await.unwrap();
    attempt.status = AgentWorkflowAttemptStatus::Running;
    attempt.started_at = Some(chrono::Utc::now().timestamp());
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Queued)
        .await
        .unwrap());
    assert!(store
        .set_running_agent_workflow_attempt_provenance(
            "request",
            Some("agent-session"),
            "child-frame",
        )
        .await
        .unwrap());
    let running = store
        .get_agent_workflow_attempt("attempt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(running.agent_session_id.as_deref(), Some("agent-session"));
    assert_eq!(running.child_frame_id.as_deref(), Some("child-frame"));

    assert_eq!(store.request_agent_workflow_cancel("wf").await.unwrap(), 1);
    assert!(store.agent_workflow_cancel_requested("wf").await.unwrap());
    attempt.status = AgentWorkflowAttemptStatus::Cancelled;
    attempt.cancel_requested = true;
    attempt.finished_at = Some(chrono::Utc::now().timestamp());
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Running)
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Running,
            AgentWorkflowStatus::Cancelled,
        )
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Cancelled,
            AgentWorkflowStatus::Approved,
        )
        .await
        .unwrap());
    assert!(!store.agent_workflow_cancel_requested("wf").await.unwrap());

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn nested_agent_fanout_lineage_survives_restart_and_root_cancel() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_nested_agent_lineage_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    let limits = AgentDelegationRootLimits {
        max_depth: 2,
        max_tasks: 3,
        max_parallel: 2,
        max_tokens: 1_000,
        max_tool_calls: 20,
        max_cost_microunits: 1_000,
        wall_time_secs: 300,
    };
    let parent = create_running_nested_test_root(&store, limits.clone(), true, 100).await;

    let mut nested = AgentWorkflow::new("nested", "p", "workspace", "Nested batch").unwrap();
    nested.frame_id = Some("root-child-frame".into());
    nested.root_workflow_id = "root".into();
    nested.parent_attempt_id = Some(parent.id.clone());
    nested.depth = parent.depth;
    nested.root_limits_json = serde_json::to_string(&limits).unwrap();
    nested.max_parallel = 2;
    nested.plan_json = serde_json::json!({"schema_version": 2}).to_string();
    let first = nested_test_step("nested-a", "nested", false, 50);
    let mut second = nested_test_step("nested-b", "nested", false, 50);
    second.position = 1;
    store
        .create_agent_workflow_plan(&nested, &[first, second])
        .await
        .unwrap();
    assert_eq!(
        store
            .list_child_agent_workflow_ids(&parent.id)
            .await
            .unwrap(),
        ["nested"]
    );
    assert!(store
        .approve_agent_workflow_plan("nested", 1)
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "nested",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    assert!(store
        .set_agent_workflow_attempt_delegation_slot_yielded(&parent.id, true)
        .await
        .unwrap());
    for (attempt_id, step_id) in [
        ("nested-attempt-a", "nested-a"),
        ("nested-attempt-b", "nested-b"),
    ] {
        let mut attempt = AgentWorkflowAttempt::queued(
            attempt_id,
            "nested",
            step_id,
            1,
            format!("request-{step_id}"),
            "local",
            "{}",
        )
        .unwrap();
        attempt.root_workflow_id = "root".into();
        attempt.parent_attempt_id = Some(parent.id.clone());
        attempt.depth = 2;
        let AgentWorkflowAttemptStart::Started(started) = store
            .try_create_started_agent_workflow_attempt(attempt)
            .await
            .unwrap()
        else {
            panic!("both nested fan-out attempts should reserve root slots");
        };
        assert_eq!(started.depth, 2);
        assert_eq!(
            started.parent_attempt_id.as_deref(),
            Some(parent.id.as_str())
        );
    }

    assert_eq!(
        store.request_agent_workflow_cancel("nested").await.unwrap(),
        3
    );
    for id in ["root-attempt", "nested-attempt-a", "nested-attempt-b"] {
        assert!(
            store
                .get_agent_workflow_attempt(id)
                .await
                .unwrap()
                .unwrap()
                .cancel_requested
        );
    }
    assert!(store
        .agent_workflow_cancel_requested("nested")
        .await
        .unwrap());

    store.pool.close().await;
    let reopened = Store::open(&tmp).await.unwrap();
    let persisted = reopened
        .get_agent_workflow("nested")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.root_workflow_id, "root");
    assert_eq!(persisted.parent_attempt_id.as_deref(), Some("root-attempt"));
    assert_eq!(persisted.depth, 1);
    let attempt = reopened
        .get_agent_workflow_attempt("nested-attempt-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempt.root_workflow_id, "root");
    assert_eq!(attempt.parent_attempt_id.as_deref(), Some("root-attempt"));
    assert_eq!(attempt.depth, 2);
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn nested_agent_task_and_budget_limits_fail_before_workflow_creation() {
    for (name, limits, root_tokens, child_tokens, expected) in [
        (
            "tasks",
            AgentDelegationRootLimits {
                max_depth: 2,
                max_tasks: 1,
                ..AgentDelegationRootLimits::default()
            },
            10,
            1,
            "task limit",
        ),
        (
            "budget",
            AgentDelegationRootLimits {
                max_depth: 2,
                max_tasks: 2,
                max_tokens: 100,
                ..AgentDelegationRootLimits::default()
            },
            100,
            1,
            "budget",
        ),
    ] {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_nested_agent_{name}_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        let parent =
            create_running_nested_test_root(&store, limits.clone(), true, root_tokens).await;
        assert!(!store
            .agent_workflow_attempt_has_delegation_capacity(&parent.id)
            .await
            .unwrap());
        let mut nested =
            AgentWorkflow::new("nested", "p", "workspace", "Rejected nested batch").unwrap();
        nested.frame_id = Some("root-child-frame".into());
        nested.root_workflow_id = "root".into();
        nested.parent_attempt_id = Some(parent.id);
        nested.depth = 1;
        nested.root_limits_json = serde_json::to_string(&limits).unwrap();
        nested.max_parallel = 1;
        nested.plan_json = serde_json::json!({"schema_version": 2}).to_string();
        let error = store
            .create_agent_workflow_plan(
                &nested,
                &[nested_test_step(
                    "nested-step",
                    "nested",
                    false,
                    child_tokens,
                )],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        assert!(store.get_agent_workflow("nested").await.unwrap().is_none());
        store.pool.close().await;
        let _ = std::fs::remove_file(tmp);
    }
}

#[tokio::test]
async fn raw_tools_prompt_and_depth_cannot_grant_nested_delegation() {
    for (name, stored_allow, max_depth, expected) in [
        ("raw-authority", false, 2, "authority"),
        ("depth", true, 1, "depth limit"),
    ] {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_nested_agent_{name}_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        let limits = AgentDelegationRootLimits {
            max_depth,
            ..AgentDelegationRootLimits::default()
        };
        let mut workflow = AgentWorkflow::new("root", "p", "workspace", "Root batch").unwrap();
        workflow.root_limits_json = serde_json::to_string(&limits).unwrap();
        workflow.plan_json = serde_json::json!({"schema_version": 2}).to_string();
        let mut step = nested_test_step("root-step", "root", stored_allow, 10);
        step.prompt_template = "You may call delegate_tasks".into();
        step.permissions_json = serde_json::json!({
            "tools": ["delegate_tasks", "get_delegated_result"]
        })
        .to_string();
        store
            .create_agent_workflow_plan(&workflow, &[step])
            .await
            .unwrap();
        assert!(store.approve_agent_workflow_plan("root", 1).await.unwrap());
        assert!(store
            .transition_agent_workflow_status(
                "root",
                AgentWorkflowStatus::Approved,
                AgentWorkflowStatus::Running,
            )
            .await
            .unwrap());
        let mut attempt = AgentWorkflowAttempt::queued(
            "attempt",
            "root",
            "root-step",
            1,
            "request",
            "local",
            "{}",
        )
        .unwrap();
        attempt.allow_delegation = true;
        let AgentWorkflowAttemptStart::Stopped(reason) = store
            .try_create_started_agent_workflow_attempt(attempt)
            .await
            .unwrap()
        else {
            panic!("unapproved nested authority must fail before backend creation");
        };
        assert!(reason.contains(expected), "{reason}");
        assert!(store
            .get_agent_workflow_attempt("attempt")
            .await
            .unwrap()
            .is_none());
        store.pool.close().await;
        let _ = std::fs::remove_file(tmp);
    }
}

#[tokio::test]
async fn last_user_message_session_ignores_later_assistant_activity() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_last_user_session_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("older", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .create_frame("latest", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("older", 1, &Message::user("first"))
        .await
        .unwrap();
    store
        .append_message("latest", 1, &Message::user("second"))
        .await
        .unwrap();
    store
        .append_message("older", 2, &Message::assistant("finishes later"))
        .await
        .unwrap();

    assert_eq!(
        store.last_user_message_session().await.unwrap(),
        Some(("latest".into(), "p".into()))
    );
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn session_history_and_outline_use_message_times() {
    let tmp = std::env::temp_dir().join(format!("wisp_activity_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .create_frame("older", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .create_frame("newer", "p", "OPERON", "m")
        .await
        .unwrap();
    store.set_frame_timestamps("older", 100, 100).await.unwrap();
    store.set_frame_timestamps("newer", 200, 200).await.unwrap();

    let mut older_question = Message::user("older question");
    older_question.ts = 100;
    let mut older_reply = Message::assistant("older reply");
    older_reply.ts = 110;
    let mut newer_question = Message::user("newer question");
    newer_question.ts = 200;
    let mut newer_reply = Message::assistant("newer reply");
    newer_reply.ts = 210;
    let mut resumed_question = Message::user("resumed question");
    resumed_question.ts = 300;
    let mut resumed_reply = Message::assistant("resumed reply");
    resumed_reply.ts = 310;

    store
        .append_message("older", 1, &older_question)
        .await
        .unwrap();
    store
        .append_message("older", 2, &older_reply)
        .await
        .unwrap();
    store
        .append_message("newer", 1, &newer_question)
        .await
        .unwrap();
    store
        .append_message("newer", 2, &newer_reply)
        .await
        .unwrap();
    store
        .append_message("older", 3, &resumed_question)
        .await
        .unwrap();
    store
        .append_message("older", 4, &resumed_reply)
        .await
        .unwrap();

    let sessions = store.list_sessions("p").await.unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|(id, _, activity_at, ..)| (id.as_str(), *activity_at))
            .collect::<Vec<_>>(),
        [("older", 310), ("newer", 210)]
    );
    assert_eq!(
        store.load_session_user_messages("older").await.unwrap(),
        vec![
            (1, "older question".into(), 100, Some(110)),
            (3, "resumed question".into(), 300, Some(310)),
        ]
    );
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn session_pages_are_stable_when_timestamps_match() {
    let tmp = std::env::temp_dir().join(format!("wisp_pages_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["a", "b", "c"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        let mut message = Message::user(id);
        message.ts = 10;
        store.append_message(id, 1, &message).await.unwrap();
    }

    let first = store.list_sessions_page("p", None, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let cursor = (first[1].2, first[1].0.as_str());
    let second = store
        .list_sessions_page("p", Some(cursor), 2)
        .await
        .unwrap();
    let ids = first
        .iter()
        .chain(&second)
        .map(|row| row.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["c", "b", "a"]);
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn branched_from_survives_listing() {
    let tmp = std::env::temp_dir().join(format!("wisp_branched_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "fork"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user(id))
            .await
            .unwrap();
    }
    store
        .set_session_branched_from("fork", "main")
        .await
        .unwrap();

    let listed = store.list_sessions("p").await.unwrap();
    let source = |id: &str| {
        listed
            .iter()
            .find(|row| row.0 == id)
            .map(|row| row.4.clone())
            .unwrap()
    };
    assert_eq!(source("fork").as_deref(), Some("main"));
    assert_eq!(source("main"), None);

    store.set_session_pinned("fork", "p", true).await.unwrap();
    let pinned = store.list_pinned_sessions("p").await.unwrap();
    assert_eq!(pinned[0].4.as_deref(), Some("main"));
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn main_session_cannot_be_deleted_until_its_conversation_branches_are_deleted() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_branch_delete_guard_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_project("target", "target", "").await.unwrap();
    for id in ["main", "branch"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user(id))
            .await
            .unwrap();
    }
    store
        .set_session_branch_point("branch", "main", 0, "before_user")
        .await
        .unwrap();

    let error = store.delete_session("main", "p").await.unwrap_err();
    assert!(error.to_string().contains("session_has_branches"));
    let move_error = store
        .move_session_to_project("main", "p", "target", "moved-main")
        .await
        .unwrap_err();
    assert!(move_error.to_string().contains("session_has_branches"));
    assert_eq!(
        store.frame_project_id("main").await.unwrap().as_deref(),
        Some("p")
    );

    store.delete_session("branch", "p").await.unwrap();
    store.delete_session("main", "p").await.unwrap();
    assert!(store.frame_project_id("main").await.unwrap().is_none());

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn legacy_branch_lineage_is_not_treated_as_a_mergeable_branch() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_legacy_branch_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "legacy", "new"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user("shared"))
            .await
            .unwrap();
    }
    store
        .set_session_branched_from("legacy", "main")
        .await
        .unwrap();
    store
        .set_session_branch_point("new", "main", 0, "before_user")
        .await
        .unwrap();

    let mergeable = store.list_mergeable_branch_ids("p").await.unwrap();
    assert!(!mergeable.contains("legacy"));
    assert!(mergeable.contains("new"));
    let links = store.list_session_branches("main", "p").await.unwrap();
    assert_eq!(
        links
            .iter()
            .map(|link| link.id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
    assert!(store
        .preview_session_branch_merge("legacy", "p")
        .await
        .is_err());
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn branch_summary_appends_to_the_current_main_tail_without_reading_or_rewriting_it() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_branch_merge_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "branch"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user("shared question"))
            .await
            .unwrap();
        store
            .append_message(id, 2, &Message::assistant("shared answer"))
            .await
            .unwrap();
    }
    store
        .set_session_branch_point("branch", "main", 0, "after_response")
        .await
        .unwrap();
    store
        .append_message("branch", 3, &Message::user("download the dataset"))
        .await
        .unwrap();
    store
        .append_message("branch", 4, &Message::assistant("saved data/counts.csv"))
        .await
        .unwrap();

    let preview = store
        .preview_session_branch_merge("branch", "p")
        .await
        .unwrap();
    assert_eq!(preview.new_message_count, 2);
    assert_eq!(preview.messages[0].text, "download the dataset");

    // Main is free to advance after the checkpoint and even after the branch
    // summary draft was generated. Main changes are not part of the guard.
    store
        .append_message("main", 3, &Message::user("continue the paper review"))
        .await
        .unwrap();
    store
        .append_message("main", 4, &Message::assistant("review continued"))
        .await
        .unwrap();
    let after_main_advanced = store
        .preview_session_branch_merge("branch", "p")
        .await
        .unwrap();
    assert_eq!(after_main_advanced.guard_hash, preview.guard_hash);

    let merged = store
        .merge_session_branch_summary(
            "branch",
            "p",
            &preview.guard_hash,
            "Dataset downloaded to data/counts.csv.",
        )
        .await
        .unwrap();
    assert_eq!(merged.main_session_id, "main");
    assert_eq!(merged.summary_message_seq, 5);
    let main = store.load_messages("main").await.unwrap();
    assert_eq!(
        main.iter()
            .map(|message| message.content.as_text())
            .collect::<Vec<_>>(),
        [
            "shared question",
            "shared answer",
            "continue the paper review",
            "review continued",
            "Dataset downloaded to data/counts.csv.",
        ]
    );
    let transcript = store
        .load_session_transcript_page("main", None, 20)
        .await
        .unwrap();
    assert_eq!(
        transcript.branch_merges,
        [SessionBranchMergeCard {
            summary_message_seq: 5,
            branch_session_id: "branch".into(),
            branch_title: "shared question".into(),
            checkpoint_user_index: 0,
            checkpoint_kind: "after_response".into(),
            summary: "Dataset downloaded to data/counts.csv.".into(),
        }]
    );
    assert_eq!(store.list_sessions("p").await.unwrap().len(), 2);
    assert_eq!(
        store.session_branch_state("branch").await.unwrap(),
        Some("merged")
    );
    assert!(store
        .preview_session_branch_merge("branch", "p")
        .await
        .is_err());
    assert!(store
        .merge_session_branch_summary(
            "branch",
            "p",
            &preview.guard_hash,
            "A second summary must not replace the first.",
        )
        .await
        .is_err());
    let links = store.list_session_branches("main", "p").await.unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].merged);
    assert_eq!(
        links[0].merge_summary.as_deref(),
        Some("Dataset downloaded to data/counts.csv.")
    );

    // Rewinding across the real merge tail revokes it and makes the branch
    // active again. The checkpoint remains valid because the shared turn is
    // still present.
    store.truncate_messages("main", 4).await.unwrap();
    assert_eq!(
        store.session_branch_state("branch").await.unwrap(),
        Some("active")
    );
    assert!(store
        .preview_session_branch_merge("branch", "p")
        .await
        .is_ok());
    assert!(store
        .load_session_transcript_page("main", None, 20)
        .await
        .unwrap()
        .branch_merges
        .is_empty());

    // The artifact-aware turn-undo path must reconcile the same merge tail.
    let preview = store
        .preview_session_branch_merge("branch", "p")
        .await
        .unwrap();
    store
        .merge_session_branch_summary(
            "branch",
            "p",
            &preview.guard_hash,
            "Dataset remains available after review.",
        )
        .await
        .unwrap();
    store.truncate_messages_for_undo("main", 4).await.unwrap();
    assert_eq!(
        store.session_branch_state("branch").await.unwrap(),
        Some("active")
    );
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn rewinding_past_a_branch_checkpoint_keeps_it_as_frozen_history() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_branch_rewind_checkpoint_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "branch"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user("first"))
            .await
            .unwrap();
        store
            .append_message(id, 2, &Message::assistant("answer"))
            .await
            .unwrap();
        store
            .append_message(id, 3, &Message::user("second"))
            .await
            .unwrap();
    }
    store
        .set_session_branch_point("branch", "main", 1, "before_user")
        .await
        .unwrap();

    store.truncate_messages("main", 2).await.unwrap();
    assert_eq!(
        store.session_branch_state("branch").await.unwrap(),
        Some("orphaned")
    );
    assert!(store
        .list_session_branches("main", "p")
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .preview_session_branch_merge("branch", "p")
        .await
        .is_err());
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn after_response_checkpoint_requires_the_reply_to_survive_rewind() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_branch_rewind_response_checkpoint_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "before", "after"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user("question"))
            .await
            .unwrap();
        store
            .append_message(id, 2, &Message::assistant("answer"))
            .await
            .unwrap();
    }
    store
        .set_session_branch_point("before", "main", 0, "before_user")
        .await
        .unwrap();
    store
        .set_session_branch_point("after", "main", 0, "after_response")
        .await
        .unwrap();

    // Retaining the user preserves a before-user anchor, but an
    // after-response anchor is gone once that turn's reply is removed.
    store.truncate_messages("main", 1).await.unwrap();
    assert_eq!(
        store.session_branch_state("before").await.unwrap(),
        Some("active")
    );
    assert_eq!(
        store.session_branch_state("after").await.unwrap(),
        Some("orphaned")
    );
    assert_eq!(
        store
            .list_session_branches("main", "p")
            .await
            .unwrap()
            .into_iter()
            .map(|branch| branch.id)
            .collect::<Vec<_>>(),
        ["before"]
    );
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn branch_summary_guard_changes_only_when_the_branch_changes() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_branch_merge_guard_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["main", "branch"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user("shared"))
            .await
            .unwrap();
    }
    store
        .set_session_branch_point("branch", "main", 0, "before_user")
        .await
        .unwrap();
    let preview = store
        .preview_session_branch_merge("branch", "p")
        .await
        .unwrap();
    store
        .append_message("branch", 2, &Message::assistant("new branch result"))
        .await
        .unwrap();
    let error = store
        .merge_session_branch_summary("branch", "p", &preview.guard_hash, "stale")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("branch changed"));
    assert_eq!(store.load_messages("main").await.unwrap().len(), 1);
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn existing_database_without_branched_from_column_is_repaired() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_branch_lineage_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("saved conversation"))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE frames DROP COLUMN branched_from")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(SESSION_BRANCH_LINEAGE_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&CONTROL_PLANE_MIGRATION.to_string()));
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let sessions = reopened.list_sessions_page("p", None, 100).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].4.is_none());
    reopened.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn pinned_sessions_are_listed_separately_and_toggle() {
    let tmp = std::env::temp_dir().join(format!("wisp_pinned_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["a", "b", "c"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user(id))
            .await
            .unwrap();
    }

    assert!(store.list_pinned_sessions("p").await.unwrap().is_empty());

    store.set_session_pinned("a", "p", true).await.unwrap();
    let pinned = store.list_pinned_sessions("p").await.unwrap();
    assert_eq!(
        pinned.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    // The full listing still contains every session, pinned or not.
    assert_eq!(store.list_sessions("p").await.unwrap().len(), 3);

    store.set_session_pinned("a", "p", false).await.unwrap();
    assert!(store.list_pinned_sessions("p").await.unwrap().is_empty());

    // Pinning a missing session is an error, not a silent no-op.
    assert!(store
        .set_session_pinned("missing", "p", true)
        .await
        .is_err());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn existing_database_without_pinned_column_is_repaired() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_pinned_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("saved conversation"))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE frames DROP COLUMN pinned")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(SESSION_PINNED_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    reopened.set_session_pinned("f", "p", true).await.unwrap();
    assert_eq!(reopened.list_pinned_sessions("p").await.unwrap().len(), 1);
    reopened.pool.close().await;
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
async fn transcript_pages_keep_complete_user_turns_and_matching_events() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_transcript_page_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let messages = [
        Message::system("sys"),
        Message::user("one"),
        Message::assistant("answer one"),
        Message::user("two"),
        Message::assistant("answer two"),
        Message::user("three"),
        Message::assistant("answer three"),
    ];
    for (seq, message) in messages.iter().enumerate() {
        store
            .append_message("f", seq as i64, message)
            .await
            .unwrap();
        store
            .append_session_ui_event(
                "f",
                seq as i64 * 2 + 1,
                &format!(r#"{{"kind":"Text","frame_id":"f","delta":"event {seq}"}}"#),
            )
            .await
            .unwrap();
        store
            .append_session_ui_event(
                "f",
                seq as i64 * 2 + 2,
                &format!(r#"{{"kind":"MessageBoundary","frame_id":"f","seq":{seq}}}"#),
            )
            .await
            .unwrap();
    }
    store
        .upsert_session_review("f", "old-review", 2, "{}")
        .await
        .unwrap();
    store
        .upsert_session_review("f", "new-review", 4, "{}")
        .await
        .unwrap();

    let latest = store
        .load_session_transcript_page("f", None, 2)
        .await
        .unwrap();
    assert_eq!(latest.messages.first().unwrap().0, 3);
    assert_eq!(latest.messages.last().unwrap().0, 6);
    assert_eq!(latest.next_before_seq, Some(3));
    assert_eq!(latest.user_offset, 1);
    assert_eq!(latest.latest_seq, 6);
    assert_eq!(latest.reviews[0].0, 4);
    assert!(latest.ui_events[0].contains(r#""delta":"event 3""#));

    let earlier = store
        .load_session_transcript_page("f", latest.next_before_seq, 2)
        .await
        .unwrap();
    assert_eq!(earlier.messages.first().unwrap().0, 0);
    assert_eq!(earlier.messages.last().unwrap().0, 2);
    assert_eq!(earlier.next_before_seq, None);
    assert_eq!(earlier.user_offset, 0);
    assert_eq!(earlier.reviews[0].0, 2);
    assert!(earlier.ui_events.last().unwrap().contains(r#""seq":2"#));
    let outline = store.load_session_user_messages("f").await.unwrap();
    assert_eq!(
        outline
            .iter()
            .map(|(seq, text, _, _)| (*seq, text.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "one"), (3, "two"), (5, "three"),]
    );
    assert!(outline
        .iter()
        .all(|(_, _, sent_at, response_at)| *sent_at > 0 && response_at.is_some()));
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn max_message_seq_uses_max_not_count_when_seqs_have_gaps() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_max_seq_gap_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    assert_eq!(store.max_message_seq("f").await.unwrap(), 0);
    store
        .append_message("f", 1, &Message::user("first"))
        .await
        .unwrap();
    store
        .append_message("f", 3, &Message::assistant("third"))
        .await
        .unwrap();
    assert_eq!(store.message_count("f").await.unwrap(), 2);
    assert_eq!(store.max_message_seq("f").await.unwrap(), 3);
    let page = store
        .load_session_transcript_page("f", None, 20)
        .await
        .unwrap();
    assert_eq!(page.latest_seq, 3);
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn recent_turn_preview_messages_are_turn_and_content_bounded() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_recent_turns_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut background = Message::user("background completion");
    background.tool_name = Some(AGENT_WORKFLOW_COMPLETION_TOOL.into());
    let large_tool = Message::tool(
        "call-shell",
        "shell",
        "z".repeat(super::sessions::RECENT_TURN_TOOL_PREVIEW_MAX_CHARS + 500),
    );
    let messages = [
        Message::system("sys"),
        Message::user("one"),
        Message::assistant("answer one"),
        Message::user("two"),
        Message::assistant("answer two"),
        large_tool,
        background,
        Message::user("three"),
        Message::assistant("answer three"),
    ];
    for (seq, message) in messages.iter().enumerate() {
        store
            .append_message("f", seq as i64, message)
            .await
            .unwrap();
    }

    let recent = store
        .load_recent_turn_preview_messages("f", 2)
        .await
        .unwrap();
    assert_eq!(recent.first().unwrap().content.as_text(), "two");
    assert_eq!(recent.last().unwrap().content.as_text(), "answer three");
    assert!(recent
        .iter()
        .any(|message| message.content.as_text() == "background completion"));
    assert!(recent
        .iter()
        .all(|message| message.content.as_text() != "one"));
    let tool = recent
        .iter()
        .find(|message| message.tool_name.as_deref() == Some("shell"))
        .unwrap();
    assert_eq!(
        tool.content.as_text().chars().count(),
        super::sessions::RECENT_TURN_TOOL_PREVIEW_MAX_CHARS
    );
    assert!(recent.iter().all(|message| message.tool_calls.is_empty()));
    assert!(recent.iter().all(|message| message.reasoning.is_none()));

    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn transcript_page_caps_legacy_stdout_before_returning_event_json() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_stdout_replay_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("run it"))
        .await
        .unwrap();
    store
        .append_message("f", 2, &Message::assistant("done"))
        .await
        .unwrap();
    let events = [
        serde_json::json!({"kind":"ToolCall","frame_id":"f","name":"shell","preview":"run"}),
        serde_json::json!({"kind":"Stdout","frame_id":"f","chunk":"a".repeat(40_000)}),
        serde_json::json!({"kind":"Stdout","frame_id":"f","chunk":"b".repeat(40_000)}),
        serde_json::json!({"kind":"Stdout","frame_id":"f","chunk":"c".repeat(40_000)}),
        serde_json::json!({"kind":"ToolResult","frame_id":"f","name":"shell","ok":true,"content":"done","duration_ms":1}),
        serde_json::json!({"kind":"ToolCall","frame_id":"f","name":"shell","preview":"next"}),
        serde_json::json!({"kind":"Stdout","frame_id":"f","chunk":"next"}),
        serde_json::json!({"kind":"MessageBoundary","frame_id":"f","seq":2}),
    ];
    for (seq, event) in events.iter().enumerate() {
        store
            .append_session_ui_event("f", seq as i64 + 1, &event.to_string())
            .await
            .unwrap();
    }

    let page = store
        .load_session_transcript_page("f", None, 1)
        .await
        .unwrap();
    let stdout = page
        .ui_events
        .iter()
        .filter_map(|event| serde_json::from_str::<serde_json::Value>(event).ok())
        .filter(|event| event["kind"] == "Stdout")
        .map(|event| event["chunk"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(stdout.len(), 3, "the fully over-budget chunk is omitted");
    assert_eq!(stdout[0].len(), 40_000);
    assert_eq!(
        stdout[1].len(),
        super::sessions::SESSION_UI_STDOUT_REPLAY_MAX_CHARS - 40_000
    );
    assert_eq!(stdout[2], "next", "a new tool receives a fresh budget");

    let _ = std::fs::remove_file(tmp);
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
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: "a2".into(),
            project_id: "p2".into(),
            root_frame_id: "f2".into(),
            filename: "beta.csv".into(),
            content_type: "text/csv".into(),
            storage_path: "/tmp/beta/.wisp/artifacts/sha256/beta.csv".into(),
            logical_key: Some("path:results/beta.csv".into()),
            size_bytes: None,
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Reference,
            capture_timing: ArtifactCaptureTiming::Unknown,
        })
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
    let beta = store
        .search_artifacts(None, "beta", 20, None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(beta.id, "a2");
    assert_eq!(beta.logical_path.as_deref(), Some("results/beta.csv"));

    let sessions = store
        .search_sessions(None, "result", 20, None, None)
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
async fn session_search_prefers_current_project_then_title_then_body() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_ranked_session_search_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("current", "Current", "/tmp/current")
        .await
        .unwrap();
    store
        .create_project("other", "Other", "/tmp/other")
        .await
        .unwrap();

    for (frame, project, title, body) in [
        (
            "current-title",
            "current",
            "Needle title",
            "ordinary response",
        ),
        (
            "current-body",
            "current",
            "Body conversation",
            "needle appears in the assistant body",
        ),
        (
            "other-title",
            "other",
            "Needle in another project",
            "ordinary response",
        ),
    ] {
        store
            .create_frame(frame, project, "OPERON", "m")
            .await
            .unwrap();
        store
            .append_message(frame, 1, &Message::user("initial prompt"))
            .await
            .unwrap();
        store
            .append_message(frame, 2, &Message::assistant(body))
            .await
            .unwrap();
        store.rename_session(frame, project, title).await.unwrap();
    }

    let rows = store
        .search_sessions(None, "needle", 10, None, Some("current"))
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec!["current-title", "current-body", "other-title"]
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
        store
            .load_session_transcript_page("f", None, 100)
            .await
            .unwrap()
            .reviews,
        vec![(2, r#"{"summary":"verified"}"#.into())]
    );

    store.truncate_messages("f", 1).await.unwrap();
    assert!(store
        .load_session_transcript_page("f", None, 100)
        .await
        .unwrap()
        .reviews
        .is_empty());
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
    let app_v1 = r#"{"kind":"ToolPresentation","frame_id":"f","payload":{"version":1}}"#;
    let app_v2 = r#"{"kind":"ToolPresentation","frame_id":"f","payload":{"version":2}}"#;
    store.append_session_ui_event("f", 1, first).await.unwrap();
    store.append_session_ui_event("f", 2, second).await.unwrap();
    store.append_session_ui_event("f", 3, app_v1).await.unwrap();
    store.append_session_ui_event("f", 4, app_v2).await.unwrap();
    assert_eq!(
        store.load_session_ui_events("f").await.unwrap(),
        vec![first, second, app_v1, app_v2]
    );
    assert_eq!(
        store
            .load_latest_session_ui_event("f", "ToolPresentation")
            .await
            .unwrap(),
        Some(app_v2.into())
    );
    assert_eq!(store.next_session_ui_event_seq("f").await.unwrap(), 5);
    store.truncate_messages("f", 1).await.unwrap();
    assert_eq!(
        store.load_session_ui_events("f").await.unwrap(),
        vec![first]
    );
}

#[tokio::test]
async fn session_ui_events_timed_roundtrip() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_ui_events_timed_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    let before = chrono::Utc::now().timestamp_millis();
    store
        .append_session_ui_event("f", 1, r#"{"kind":"User","frame_id":"f","text":"q"}"#)
        .await
        .unwrap();
    store
        .append_session_ui_event("f", 2, r#"{"kind":"Text","frame_id":"f","delta":"a"}"#)
        .await
        .unwrap();
    let after = chrono::Utc::now().timestamp_millis();

    let events = store.load_session_ui_events_timed("f").await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].seq, 2);
    assert!(events[0].event_json.contains("\"kind\":\"User\""));
    for event in &events {
        let created_at = event.created_at.expect("created_at must be stamped");
        assert!(
            (before..=after).contains(&created_at),
            "created_at {created_at} outside [{before}, {after}]"
        );
    }
    store.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn session_ui_events_created_at_backfill_is_idempotent() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_ui_events_created_at_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    assert!(
        Store::has_column(&store.pool, "session_ui_events", "created_at")
            .await
            .unwrap()
    );
    // Simulate a pre-upgrade deployment: the table exists without the column
    // and already holds rows.
    sqlx::query("ALTER TABLE session_ui_events DROP COLUMN created_at")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO session_ui_events(frame_id,seq,event_json) \
         VALUES('f',1,'{\"kind\":\"User\",\"frame_id\":\"f\",\"text\":\"old\"}')",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    assert!(
        Store::has_column(&repaired.pool, "session_ui_events", "created_at")
            .await
            .unwrap()
    );
    let events = repaired.load_session_ui_events_timed("f").await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].created_at, None);
    repaired
        .append_session_ui_event("f", 2, r#"{"kind":"Text","frame_id":"f","delta":"new"}"#)
        .await
        .unwrap();
    let events = repaired.load_session_ui_events_timed("f").await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[1].created_at.is_some());
    // Reopening again must not fail or alter the existing rows.
    repaired.pool.close().await;
    let reopened = Store::open(&tmp).await.unwrap();
    let events = reopened.load_session_ui_events_timed("f").await.unwrap();
    assert_eq!(events.len(), 2);
    reopened.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn interrupted_replace_keeps_the_previous_transcript_and_seq_anchored_rows() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_replace_atomic_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("keep me"))
        .await
        .unwrap();
    store
        .append_message("f", 2, &Message::assistant("kept answer"))
        .await
        .unwrap();
    store
        .save_turn_file_undo(
            "f",
            1,
            "src/main.rs",
            true,
            None,
            Some("before"),
            Some("after"),
            true,
            None,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO message_resource_links(\
         id,frame_id,message_seq,ordinal,original_reference,display_name,\
         resource_kind,mime_type,status,created_at) \
         VALUES('l1','f',1,0,'@r.csv','r.csv','file','text/csv','linked',1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();

    // Simulate a crash mid-replace: the deletes and half the inserts ran, but
    // the transaction is dropped without commit.
    {
        let mut tx = store.begin_write().await.unwrap();
        Store::replace_message_rows_for_test(
            &mut tx,
            "f",
            &[Message::system("half-written checkpoint")],
        )
        .await
        .unwrap();
    }

    let msgs = store.load_messages("f").await.unwrap();
    assert_eq!(msgs.len(), 2, "old transcript must survive an interruption");
    assert_eq!(msgs[0].content.as_text(), "keep me");
    assert_eq!(msgs[1].content.as_text(), "kept answer");
    assert_eq!(
        store.list_turn_file_undo("f", 1).await.unwrap().len(),
        1,
        "undo rows must survive an interrupted replace"
    );
    assert_eq!(
        store
            .list_message_resource_links("f", 1, None)
            .await
            .unwrap()
            .len(),
        1,
        "resource links must survive an interrupted replace"
    );

    // A committed replace applies wholesale and drops the seq-anchored rows,
    // whose anchors the renumbering invalidated.
    store
        .replace_messages(
            "f",
            &[Message::system("checkpoint"), Message::user("recent tail")],
        )
        .await
        .unwrap();
    let msgs = store.load_messages("f").await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content.as_text(), "checkpoint");
    assert_eq!(msgs[1].content.as_text(), "recent tail");
    assert!(store.list_turn_file_undo("f", 1).await.unwrap().is_empty());
    assert!(store
        .list_message_resource_links("f", 1, None)
        .await
        .unwrap()
        .is_empty());
    store.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn side_chat_snapshot_survives_compaction_and_stops_at_completed_boundary() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_side_snapshot_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "P", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    let events = [
        r#"{"kind":"User","frame_id":"f","text":"old decision"}"#,
        r#"{"kind":"MessageBoundary","frame_id":"f","seq":1}"#,
        r#"{"kind":"Text","frame_id":"f","delta":"old answer"}"#,
        r#"{"kind":"MessageBoundary","frame_id":"f","seq":2}"#,
        r#"{"kind":"Text","frame_id":"f","delta":"still streaming"}"#,
    ];
    for (index, event) in events.iter().enumerate() {
        store
            .append_session_ui_event("f", index as i64 + 1, event)
            .await
            .unwrap();
    }
    store
        .append_message("f", 1, &Message::user("old decision"))
        .await
        .unwrap();
    store
        .replace_messages(
            "f",
            &[
                Message::system("compacted checkpoint"),
                Message::user("recent tail"),
            ],
        )
        .await
        .unwrap();

    let snapshot = store.load_session_ui_event_snapshot("f").await.unwrap();
    assert_eq!(snapshot.through_event_seq, 4);
    assert_eq!(snapshot.events.len(), 4);
    assert!(snapshot.events[0].1.contains("old decision"));
    assert!(snapshot.events[2].1.contains("old answer"));
    assert!(snapshot
        .events
        .iter()
        .all(|(_, event)| !event.contains("still streaming")));
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

    // one artifact under "a", none under "b"
    store
        .save_artifact("art1", "a", "f1", "r.csv", "text/csv", "/tmp/r.csv")
        .await
        .unwrap();

    let projs = store.list_projects().await.unwrap();
    assert_eq!(projs.len(), 2);
    // ordered by updated_at desc; "b" created last so it sorts first
    assert_eq!(projs[0].0, "b");
    let a = projs.iter().find(|p| p.0 == "a").unwrap();
    assert_eq!(a.5, 1, "project a has one session");
    assert_eq!(a.7, 1, "project a has one artifact");
    let b = projs.iter().find(|p| p.0 == "b").unwrap();
    assert_eq!(b.5, 0, "project b has no sessions");
    assert_eq!(b.7, 0, "project b has no artifacts");

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

async fn count_bound(store: &Store, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&store.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn delete_project_clears_later_child_tables_and_ignores_orphan_schedules() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_delete_project_cascade_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("gone", "Gone", "/tmp/gone")
        .await
        .unwrap();
    store
        .create_project("keep", "Keep", "/tmp/keep")
        .await
        .unwrap();
    store
        .create_frame("gone-main", "gone", "OPERON", "m")
        .await
        .unwrap();
    store
        .create_frame("gone-explore", "gone", "OPERON", "m")
        .await
        .unwrap();
    store
        .create_frame("keep-main", "keep", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("gone-main", 1, &Message::user("seed"))
        .await
        .unwrap();

    let sha = "a".repeat(64);
    store
        .create_workspace_snapshot(&WorkspaceSnapshotRecord {
            id: "gone-snap".into(),
            project_id: "gone".into(),
            manifest_json: "{}".into(),
            manifest_sha256: sha.clone(),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_context_archive(&ContextArchiveRecord {
            id: "gone-archive".into(),
            project_id: "gone".into(),
            frame_id: "gone-main".into(),
            storage_path: ".wisp/history/gone.json".into(),
            checksum: sha.clone(),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration_family(&ExplorationFamily {
            id: "gone-family".into(),
            project_id: "gone".into(),
            root_frame_id: "gone-main".into(),
            mainline_frame_id: "gone-main".into(),
            generation: 0,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration_checkpoint(&ExplorationCheckpoint {
            id: "gone-checkpoint".into(),
            family_id: "gone-family".into(),
            project_id: "gone".into(),
            source_frame_id: "gone-main".into(),
            source_message_seq: 1,
            source_frame_head_seq: 1,
            source_ui_event_seq: 0,
            source_family_generation: 0,
            source_state_generation: 0,
            workspace_snapshot_id: "gone-snap".into(),
            context_archive_id: "gone-archive".into(),
            guard_hash: sha.clone(),
            entity_hash: "b".repeat(64),
            isolation_summary_json: "{}".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration(&Exploration {
            id: "gone-explore-id".into(),
            checkpoint_id: "gone-checkpoint".into(),
            frame_id: "gone-explore".into(),
            name: "Alt path".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/gone-explore".into(),
            workspace_backend: "copy".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();
    store
        .record_exploration_baseline_entity(&ExplorationBaselineEntity {
            checkpoint_id: "gone-checkpoint".into(),
            entity_kind: "decision".into(),
            entity_id: "gone-decision".into(),
            version_id: None,
            fingerprint: sha.clone(),
        })
        .await
        .unwrap();
    let version_id = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: Some("gone-version".into()),
            artifact_id: "gone-artifact".into(),
            project_id: "gone".into(),
            root_frame_id: "gone-main".into(),
            filename: "result.txt".into(),
            content_type: "text/plain".into(),
            storage_path: "result.txt".into(),
            logical_key: Some("path:result.txt".into()),
            size_bytes: Some(1),
            checksum: Some(sha.clone()),
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    store
        .record_exploration_baseline_artifact_head(&ExplorationBaselineArtifactHead {
            checkpoint_id: "gone-checkpoint".into(),
            logical_key: "path:result.txt".into(),
            artifact_id: "gone-artifact".into(),
            artifact_version_id: version_id.clone(),
            fingerprint: sha.clone(),
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO exploration_effects(\
           id,exploration_id,effect_kind,recoverability,target_summary,metadata_json,created_at\
         ) VALUES('gone-effect','gone-explore-id','run','local_reversible','local','{}',1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    store
        .create_exploration_promotion(&ExplorationPromotion {
            id: "gone-promo".into(),
            exploration_id: "gone-explore-id".into(),
            expected_guard_hash: sha.clone(),
            status: ExplorationPromotionStatus::Prepared,
            diff_json: "{}".into(),
            journal_path: None,
            error: None,
            started_at: 3,
            committed_at: None,
        })
        .await
        .unwrap();
    assert!(store
        .create_project_state_revision(&ProjectStateRevision {
            id: "gone-rev".into(),
            project_id: "gone".into(),
            frame_id: "gone-main".into(),
            turn_index: 0,
            message_seq: 1,
            ui_event_seq: 0,
            parent_revision_id: None,
            workspace_snapshot_id: "gone-snap".into(),
            workspace_manifest_sha256: sha.clone(),
            workspace_delta_json: "[]".into(),
            artifact_heads_json: "[]".into(),
            entities_json: "[]".into(),
            run_ids_json: "[]".into(),
            decision_ids_json: "[]".into(),
            external_effects_json: "[]".into(),
            context_archive_id: "gone-archive".into(),
            state_generation: 0,
            is_full: true,
            created_at: 1,
        })
        .await
        .unwrap());

    let gone_schedule = ScheduleRecord {
        id: "gone-sched".into(),
        project_id: "gone".into(),
        frame_id: Some("gone-main".into()),
        name: "Daily".into(),
        prompt: "Summarize".into(),
        skill: None,
        interval_secs: 60,
        enabled: true,
        next_run_at: 100,
        last_run_at: None,
        created_at: 1,
        updated_at: 1,
    };
    store.create_schedule(&gone_schedule).await.unwrap();
    store
        .record_schedule_run(&ScheduleRunRecord {
            id: "gone-run".into(),
            schedule_id: "gone-sched".into(),
            frame_id: Some("gone-main".into()),
            status: "fired".into(),
            error: None,
            fired_at: 100,
        })
        .await
        .unwrap();
    store
        .upsert_context_storage_prefs(&ContextStoragePrefs {
            project_id: "gone".into(),
            context_id: "local".into(),
            remote_data_root: "~/data".into(),
            remote_workdir_root: ".wisp-science/runs".into(),
            local_results_dir: "results".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .record_remote_staging(&RemoteStagingEntry {
            id: "gone-staging".into(),
            project_id: "gone".into(),
            context_id: "local".into(),
            run_id: None,
            remote_path: "/tmp/staged".into(),
            source: "transfer".into(),
            checksum: None,
            size_bytes: None,
            created_at: 1,
            removed_at: None,
        })
        .await
        .unwrap();
    let plugin = PluginInstallation {
        plugin_id: "cascade-plugin".into(),
        version: "1.0.0".into(),
        display_name: "Cascade".into(),
        description: String::new(),
        author: String::new(),
        license: "MIT".into(),
        source_uri: "https://example.invalid/plugin.zip".into(),
        install_root: "/plugins/cascade/1.0.0".into(),
        archive_sha256: sha.clone(),
        manifest_json: r#"{"schema":"wisp.plugin.v1"}"#.into(),
        trust_state: "checksum_verified".into(),
        installed_at: 1,
        updated_at: 1,
    };
    store.replace_plugin_installation(&plugin).await.unwrap();
    store
        .set_project_plugin("gone", &plugin.plugin_id, &plugin.version, true, "{}")
        .await
        .unwrap();
    store
        .set_project_plugin("keep", &plugin.plugin_id, &plugin.version, true, "{}")
        .await
        .unwrap();
    store
        .insert_ask_user_request("gone-ask", "gone-main", r#"{"q":"ok?"}"#)
        .await
        .unwrap();
    store
        .record_session_import("gone-import", "gone-main", "/tmp/gone.json")
        .await
        .unwrap();
    store
        .record_codex_import("gone-codex", "gone-main", "/tmp/gone.jsonl")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO session_branch_merges(\
           id,source_frame_id,branch_frame_id,checkpoint_user_index,checkpoint_kind,\
           summary_message_seq,guard_hash,created_at\
         ) VALUES('gone-merge','gone-main','gone-explore',0,'before_user',1,?,1)",
    )
    .bind(&sha)
    .execute(&store.pool)
    .await
    .unwrap();
    store
        .insert_global_memory(&GlobalMemory {
            id: "mem-gone".into(),
            content: "from deleted project".into(),
            source_frame_id: Some("gone-main".into()),
            source_turn_index: Some(0),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .insert_global_memory(&GlobalMemory {
            id: "mem-keep".into(),
            content: "from kept project".into(),
            source_frame_id: Some("keep-main".into()),
            source_turn_index: Some(0),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .create_schedule(&ScheduleRecord {
            id: "keep-sched".into(),
            project_id: "keep".into(),
            frame_id: Some("keep-main".into()),
            name: "Keep daily".into(),
            prompt: "Keep going".into(),
            skill: None,
            interval_secs: 60,
            enabled: true,
            next_run_at: 100,
            last_run_at: None,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();

    assert!(!store.due_schedules(500).await.unwrap().is_empty());
    store.delete_project("gone").await.unwrap();

    for (sql, label) in [
        (
            "SELECT COUNT(*) FROM schedules WHERE project_id=?",
            "schedules",
        ),
        (
            "SELECT COUNT(*) FROM schedule_runs WHERE schedule_id='gone-sched'",
            "schedule_runs",
        ),
        (
            "SELECT COUNT(*) FROM context_storage_prefs WHERE project_id=?",
            "context_storage_prefs",
        ),
        (
            "SELECT COUNT(*) FROM remote_staging WHERE project_id=?",
            "remote_staging",
        ),
        (
            "SELECT COUNT(*) FROM project_state_counters WHERE project_id=?",
            "project_state_counters",
        ),
        (
            "SELECT COUNT(*) FROM project_state_revisions WHERE project_id=?",
            "project_state_revisions",
        ),
        (
            "SELECT COUNT(*) FROM workspace_snapshots WHERE project_id=?",
            "workspace_snapshots",
        ),
        (
            "SELECT COUNT(*) FROM context_archives WHERE project_id=?",
            "context_archives",
        ),
        (
            "SELECT COUNT(*) FROM exploration_families WHERE project_id=?",
            "exploration_families",
        ),
        (
            "SELECT COUNT(*) FROM exploration_checkpoints WHERE project_id=?",
            "exploration_checkpoints",
        ),
        (
            "SELECT COUNT(*) FROM explorations WHERE id='gone-explore-id'",
            "explorations",
        ),
        (
            "SELECT COUNT(*) FROM exploration_baseline_entities WHERE checkpoint_id='gone-checkpoint'",
            "exploration_baseline_entities",
        ),
        (
            "SELECT COUNT(*) FROM exploration_baseline_artifact_heads WHERE checkpoint_id='gone-checkpoint'",
            "exploration_baseline_artifact_heads",
        ),
        (
            "SELECT COUNT(*) FROM exploration_effects WHERE exploration_id='gone-explore-id'",
            "exploration_effects",
        ),
        (
            "SELECT COUNT(*) FROM exploration_promotions WHERE exploration_id='gone-explore-id'",
            "exploration_promotions",
        ),
        (
            "SELECT COUNT(*) FROM artifact_heads WHERE project_id=?",
            "artifact_heads",
        ),
        (
            "SELECT COUNT(*) FROM project_plugins WHERE project_id=?",
            "project_plugins",
        ),
        (
            "SELECT COUNT(*) FROM session_branch_merges WHERE id='gone-merge'",
            "session_branch_merges",
        ),
        (
            "SELECT COUNT(*) FROM ask_user_requests WHERE frame_id='gone-main'",
            "ask_user_requests",
        ),
        (
            "SELECT COUNT(*) FROM session_imports WHERE frame_id='gone-main'",
            "session_imports",
        ),
        (
            "SELECT COUNT(*) FROM codex_imports WHERE frame_id='gone-main'",
            "codex_imports",
        ),
        (
            "SELECT COUNT(*) FROM frames WHERE project_id=?",
            "frames",
        ),
    ] {
        let bind = if sql.contains('?') { "gone" } else { "" };
        let count = if sql.contains('?') {
            count_bound(&store, sql, bind).await
        } else {
            sqlx::query_scalar(sql)
                .fetch_one(&store.pool)
                .await
                .unwrap()
        };
        assert_eq!(count, 0, "orphan row remains in {label}");
    }

    assert!(store
        .due_schedules(500)
        .await
        .unwrap()
        .iter()
        .all(|row| { row.project_id != "gone" && row.id != "gone-sched" }));
    assert_eq!(
        store
            .due_schedules(500)
            .await
            .unwrap()
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["keep-sched"]
    );
    assert!(store.get_project("keep").await.unwrap().is_some());
    assert_eq!(
        count_bound(
            &store,
            "SELECT COUNT(*) FROM project_plugins WHERE project_id=?",
            "keep"
        )
        .await,
        1
    );
    assert!(store
        .get_plugin_installation("cascade-plugin", "1.0.0")
        .await
        .unwrap()
        .is_some());
    let memories = store.list_global_memories(10).await.unwrap();
    let mem_gone = memories.iter().find(|m| m.id == "mem-gone").unwrap();
    assert_eq!(mem_gone.source_frame_id, None);
    let mem_keep = memories.iter().find(|m| m.id == "mem-keep").unwrap();
    assert_eq!(mem_keep.source_frame_id.as_deref(), Some("keep-main"));

    store.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn delete_session_clears_later_frame_tables() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_delete_session_cascade_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .create_frame("other", "p", "OPERON", "m")
        .await
        .unwrap();

    let mut workflow = AgentWorkflow::new("wf", "p", "workspace", "Delivery").unwrap();
    workflow.frame_id = Some("f".into());
    let step = nested_test_step("step", "wf", false, 1);
    store
        .create_agent_workflow_plan(&workflow, &[step])
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_workflow_deliveries(\
           id,workflow_id,frame_id,generation,auto_resume,result_json,message_seq,delivered_at,\
           resume_status,resume_error,presented_at,created_at,updated_at\
         ) VALUES('delivery','wf','f',1,0,NULL,NULL,NULL,'disabled',NULL,NULL,1,1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    store
        .insert_ask_user_request("ask-1", "f", r#"{"q":"ok?"}"#)
        .await
        .unwrap();
    store
        .record_session_import("import-1", "f", "/tmp/session.json")
        .await
        .unwrap();
    store
        .record_codex_import("codex-1", "f", "/tmp/codex.jsonl")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO session_branch_merges(\
           id,source_frame_id,branch_frame_id,checkpoint_user_index,checkpoint_kind,\
           summary_message_seq,guard_hash,created_at\
         ) VALUES('merge-1','f','other',0,'before_user',1,'c',1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    store
        .insert_global_memory(&GlobalMemory {
            id: "mem-session".into(),
            content: "session sourced".into(),
            source_frame_id: Some("f".into()),
            source_turn_index: Some(0),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .create_schedule(&ScheduleRecord {
            id: "sched-f".into(),
            project_id: "p".into(),
            frame_id: Some("f".into()),
            name: "After session".into(),
            prompt: "Keep firing".into(),
            skill: None,
            interval_secs: 60,
            enabled: true,
            next_run_at: 100,
            last_run_at: None,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .record_schedule_run(&ScheduleRunRecord {
            id: "sched-run-f".into(),
            schedule_id: "sched-f".into(),
            frame_id: Some("f".into()),
            status: "fired".into(),
            error: None,
            fired_at: 100,
        })
        .await
        .unwrap();

    store.delete_session("f", "p").await.unwrap();

    for (sql, label) in [
        (
            "SELECT COUNT(*) FROM agent_workflow_deliveries WHERE frame_id='f'",
            "agent_workflow_deliveries",
        ),
        (
            "SELECT COUNT(*) FROM session_branch_merges WHERE id='merge-1'",
            "session_branch_merges",
        ),
        (
            "SELECT COUNT(*) FROM ask_user_requests WHERE frame_id='f'",
            "ask_user_requests",
        ),
        (
            "SELECT COUNT(*) FROM session_imports WHERE frame_id='f'",
            "session_imports",
        ),
        (
            "SELECT COUNT(*) FROM codex_imports WHERE frame_id='f'",
            "codex_imports",
        ),
        ("SELECT COUNT(*) FROM frames WHERE id='f'", "frames"),
    ] {
        let count: i64 = sqlx::query_scalar(sql)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "orphan row remains in {label}");
    }
    assert!(store.get_agent_workflow("wf").await.unwrap().is_some());
    assert_eq!(
        store
            .get_schedule("sched-f")
            .await
            .unwrap()
            .unwrap()
            .frame_id,
        None
    );
    assert_eq!(
        store.list_schedule_runs("sched-f", 10).await.unwrap()[0].frame_id,
        None
    );
    assert_eq!(
        store
            .list_global_memories(10)
            .await
            .unwrap()
            .iter()
            .find(|m| m.id == "mem-session")
            .unwrap()
            .source_frame_id,
        None
    );

    store.pool.close().await;
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
async fn mark_frame_seen_clears_unseen_until_new_activity() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_seen_{}.sqlite", uuid::Uuid::new_v4()));
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

    let unseen_of = |rows: Vec<(String, Option<String>, bool)>| {
        rows.into_iter().find(|r| r.0 == "f1").unwrap().2
    };
    assert!(unseen_of(store.list_session_last_roles("p").await.unwrap()));

    store.mark_frame_seen("f1").await.unwrap();
    assert!(!unseen_of(
        store.list_session_last_roles("p").await.unwrap()
    ));
    let found = store
        .search_sessions(None, "", 10, None, None)
        .await
        .unwrap();
    assert!(!found.iter().find(|s| s.id == "f1").unwrap().unseen);

    // New activity after the seen snapshot flips it back. Message ts comes
    // from the wall clock at whole-second resolution, so nudge it forward.
    store
        .append_message("f1", 3, &Message::assistant("more"))
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET ts = ts + 10 WHERE frame_id='f1' AND seq=3")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(unseen_of(store.list_session_last_roles("p").await.unwrap()));
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

/// A v0-era database that already recorded `0000_initial_schema`, so the
/// current 0000_init.sql (folders, extra columns) never runs. Opening it
/// after jumping to HEAD must still list sessions and folders.
#[tokio::test]
async fn upgrade_from_recorded_initial_schema_can_list_sessions() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_jump_{}.sqlite", uuid::Uuid::new_v4()));
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
        sqlx::query("INSERT INTO wisp_schema_migrations(version,applied_at) VALUES(?,1)")
            .bind(INITIAL_SCHEMA_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, description TEXT, \
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE frames (id TEXT PRIMARY KEY, parent_frame_id TEXT, root_frame_id TEXT, \
             agent_name TEXT NOT NULL, status TEXT NOT NULL, project_id TEXT, model TEXT, \
             input_tokens INTEGER, output_tokens INTEGER, created_at INTEGER NOT NULL, \
             updated_at INTEGER NOT NULL, completed_at INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id TEXT PRIMARY KEY, frame_id TEXT NOT NULL, seq INTEGER NOT NULL, \
             role TEXT NOT NULL, content TEXT, tool_calls TEXT, tool_call_id TEXT, tool_name TEXT, \
             reasoning TEXT, ts INTEGER NOT NULL, UNIQUE(frame_id, seq))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE artifacts (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, \
             root_frame_id TEXT NOT NULL, filename TEXT NOT NULL, content_type TEXT NOT NULL, \
             storage_path TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f1", 1, &Message::user("jumped versions"))
        .await
        .unwrap();
    let sessions = store.list_sessions("p").await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].0, "f1");
    assert!(store.list_folders("p").await.unwrap().is_empty());
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

#[tokio::test]
async fn session_transcripts_copy_and_move_between_projects() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_session_transfer_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("source", "Source", "/workspace/source")
        .await
        .unwrap();
    store
        .create_project("target", "Target", "/workspace/target")
        .await
        .unwrap();
    store
        .create_frame("original", "source", "OPERON", "model")
        .await
        .unwrap();
    store
        .append_message("original", 1, &Message::user("transfer this conversation"))
        .await
        .unwrap();
    store
        .append_message("original", 2, &Message::assistant("copied answer"))
        .await
        .unwrap();
    store
        .rename_session("original", "source", "Cross-project analysis")
        .await
        .unwrap();
    store
        .upsert_session_review(
            "original",
            "review-original",
            2,
            r#"{"summary":"looks good"}"#,
        )
        .await
        .unwrap();
    store
        .append_session_ui_event(
            "original",
            1,
            r#"{"kind":"MessageBoundary","frame_id":"original","seq":1}"#,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO runs(\
            id,project_id,frame_id,context_id,title,kind,status,input_refs_json,\
            output_specs_json,created_at,env_snapshot_json\
         ) VALUES('run-original','source','original','local','Run','local','succeeded','[]','[]',1,'{}')",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO artifacts(\
            id,project_id,root_frame_id,filename,content_type,storage_path,created_at\
         ) VALUES('artifact-original','source','original','result.txt','text/plain','results/result.txt',1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();

    store
        .copy_session_to_project("original", "source", "target", "copied")
        .await
        .unwrap();

    assert_eq!(
        store.frame_project_id("copied").await.unwrap().as_deref(),
        Some("target")
    );
    assert_eq!(store.load_messages("copied").await.unwrap().len(), 2);
    assert_eq!(
        store
            .load_session_transcript_page("copied", None, 100)
            .await
            .unwrap()
            .reviews,
        vec![(2, r#"{"summary":"looks good"}"#.into())]
    );
    let copied_events = store.load_session_ui_events("copied").await.unwrap();
    assert_eq!(copied_events.len(), 1);
    assert!(copied_events[0].contains(r#""frame_id":"copied""#));
    let copied = store.list_sessions("target").await.unwrap();
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].1, "Cross-project analysis");
    assert_eq!(store.list_sessions("source").await.unwrap().len(), 1);

    assert!(store
        .copy_session_to_project("original", "source", "source", "same-project")
        .await
        .is_err());
    assert!(store
        .copy_session_to_project("original", "source", "missing", "missing-project")
        .await
        .is_err());

    store
        .move_session_to_project("original", "source", "target", "moved")
        .await
        .unwrap();
    assert!(store.frame_project_id("original").await.unwrap().is_none());
    assert!(store.list_sessions("source").await.unwrap().is_empty());
    assert_eq!(
        store.frame_project_id("moved").await.unwrap().as_deref(),
        Some("target")
    );
    assert_eq!(store.load_messages("moved").await.unwrap().len(), 2);
    assert!(
        store.load_session_ui_events("moved").await.unwrap()[0].contains(r#""frame_id":"moved""#)
    );
    assert_eq!(store.list_sessions("target").await.unwrap().len(), 2);

    let source_review_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_reviews WHERE frame_id='original'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let source_event_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_ui_events WHERE frame_id='original'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(source_review_count.0, 0);
    assert_eq!(source_event_count.0, 0);
    let source_run_frame: (Option<String>,) =
        sqlx::query_as("SELECT frame_id FROM runs WHERE id='run-original'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let source_artifact_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM artifacts WHERE id='artifact-original'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert!(source_run_frame.0.is_none());
    assert_eq!(source_artifact_count.0, 0);

    drop(store);
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
async fn execution_context_selection_is_isolated_per_session() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_session_contexts_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "Project", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
    store.create_frame("f2", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&ExecutionContext::new("ssh:gpu", "GPU").unwrap())
        .await
        .unwrap();

    store
        .set_session_execution_context_enabled("f1", "ssh:gpu", true)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_session_execution_context_ids("f1")
            .await
            .unwrap(),
        vec!["ssh:gpu"]
    );
    assert!(store
        .list_session_execution_context_ids("f2")
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .session_execution_context_enabled("f1", "ssh:gpu")
        .await
        .unwrap());
    assert!(store
        .set_session_execution_context_enabled("f1", "local", true)
        .await
        .unwrap_err()
        .to_string()
        .contains("always available"));

    store
        .set_session_execution_context_enabled("f1", "ssh:gpu", false)
        .await
        .unwrap();
    assert!(store
        .list_session_execution_context_ids("f1")
        .await
        .unwrap()
        .is_empty());

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
            SESSION_REVIEWS_MIGRATION.to_string(),
            SESSION_UI_EVENTS_MIGRATION.to_string(),
            PROJECT_SYNC_STATE_MIGRATION.to_string(),
            SESSION_HISTORY_INDEX_MIGRATION.to_string(),
            MESSAGE_RESOURCE_LINKS_MIGRATION.to_string(),
            SESSION_EXECUTION_CONTEXTS_MIGRATION.to_string(),
            AGENT_WORKFLOWS_MIGRATION.to_string(),
            AGENT_WORKFLOW_CONTRACTS_MIGRATION.to_string(),
            AGENT_WORKFLOW_PLANS_MIGRATION.to_string(),
            AGENT_WORKFLOW_ATTEMPTS_MIGRATION.to_string(),
            RUN_PROGRESS_MIGRATION.to_string(),
            AGENT_WORKFLOW_DELIVERIES_MIGRATION.to_string(),
            AGENT_WORKFLOW_LINEAGE_MIGRATION.to_string(),
            PLUGIN_INSTALLATIONS_MIGRATION.to_string(),
            FRAME_SEEN_MIGRATION.to_string(),
            SESSION_PINNED_MIGRATION.to_string(),
            CODEX_IMPORTS_MIGRATION.to_string(),
            EXTERNAL_SESSION_CACHE_MIGRATION.to_string(),
            TURN_FILE_UNDO_MIGRATION.to_string(),
            SESSION_BRANCH_LINEAGE_MIGRATION.to_string(),
            ASK_USER_REQUESTS_MIGRATION.to_string(),
            RUN_ARTIFACT_LINEAGE_MIGRATION.to_string(),
            PUBLICATION_DOMAIN_MIGRATION.to_string(),
            PUBLICATION_FREEZE_MIGRATION.to_string(),
            PUBLICATION_VERIFICATION_MIGRATION.to_string(),
            AGENT_WORKFLOW_RUN_ACTIVITIES_MIGRATION.to_string(),
            METHOD_SEARCH_MIGRATION.to_string(),
            METHOD_SEARCH_CONTROL_MIGRATION.to_string(),
            SESSION_IMPORTS_MIGRATION.to_string(),
            EXPLORATION_BRANCHES_MIGRATION.to_string(),
            PROJECT_STATE_REVISIONS_MIGRATION.to_string(),
            GLOBAL_MEMORIES_MIGRATION.to_string(),
            SESSION_REASONING_EFFORT_MIGRATION.to_string(),
            SESSION_BRANCH_MERGE_MIGRATION.to_string(),
            EXPLORATION_PROMOTION_RECOVERY_MIGRATION.to_string(),
            RUN_HARVEST_STATE_MIGRATION.to_string(),
            CONTEXT_STORAGE_PREFS_MIGRATION.to_string(),
            RUN_CLEANUP_STATE_MIGRATION.to_string(),
            REMOTE_STAGING_MIGRATION.to_string(),
            RUN_RETENTION_MIGRATION.to_string(),
            SCHEDULES_MIGRATION.to_string(),
            ARTIFACT_SOURCE_DISCARDED_MIGRATION.to_string(),
            RUN_LOG_PULL_MIGRATION.to_string(),
            ORPHAN_FILE_RETENTION_MIGRATION.to_string(),
            RUN_REVIEW_DISMISSED_MIGRATION.to_string(),
            SESSION_SERVICE_TIER_MIGRATION.to_string(),
        ]
    );
    let first_open_migrations = store.schema_migrations().await.unwrap();

    // Idempotency: opening the same file again must neither re-run migrations
    // nor seed a second `local` execution context.
    store.pool.close().await;
    let store = Store::open(&tmp).await.unwrap();
    assert_eq!(
        store.schema_migrations().await.unwrap(),
        first_open_migrations,
        "a second open must leave the recorded migration set unchanged"
    );
    let locals = store
        .list_execution_contexts()
        .await
        .unwrap()
        .into_iter()
        .filter(|ctx| ctx.id == "local")
        .count();
    assert_eq!(locals, 1, "the seeded local context must not be duplicated");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn context_storage_prefs_validate_and_round_trip() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_storage_prefs_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();

    assert!(store
        .get_context_storage_prefs("p", "ssh:gpu")
        .await
        .unwrap()
        .is_none());
    let mut prefs = ContextStoragePrefs {
        project_id: "p".into(),
        context_id: "ssh:gpu".into(),
        remote_data_root: "~/wisp/proj/data".into(),
        remote_workdir_root: ".wisp-science/runs".into(),
        local_results_dir: "remote/gpu".into(),
        created_at: 0,
        updated_at: 0,
    };
    store.upsert_context_storage_prefs(&prefs).await.unwrap();
    let stored = store
        .get_context_storage_prefs("p", "ssh:gpu")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.remote_data_root, "~/wisp/proj/data");

    prefs.remote_data_root = "/data/wisp/proj".into();
    prefs.local_results_dir = "results/from-gpu".into();
    store.upsert_context_storage_prefs(&prefs).await.unwrap();
    let updated = store
        .get_context_storage_prefs("p", "ssh:gpu")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.remote_data_root, "/data/wisp/proj");
    assert_eq!(updated.local_results_dir, "results/from-gpu");
    assert_eq!(updated.created_at, stored.created_at);

    // Validation matrix: traversal, escapes, absolute local dirs all rejected.
    for (field, value) in [
        ("remote_data_root", "~/wisp/../etc"),
        ("remote_data_root", "$HOME/data"),
        ("remote_data_root", "a b"),
        ("remote_data_root", ""),
        ("remote_workdir_root", "/absolute/runs"),
        ("remote_workdir_root", "~/runs"),
        ("remote_workdir_root", "runs/.."),
        ("local_results_dir", "/absolute"),
        ("local_results_dir", "../outside"),
        ("local_results_dir", "a;b"),
    ] {
        let mut bad = updated.clone();
        match field {
            "remote_data_root" => bad.remote_data_root = value.into(),
            "remote_workdir_root" => bad.remote_workdir_root = value.into(),
            _ => bad.local_results_dir = value.into(),
        }
        assert!(
            store.upsert_context_storage_prefs(&bad).await.is_err(),
            "{field}={value} should be rejected"
        );
    }

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn remote_staging_ledger_round_trips_and_counts_external_references() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_remote_staging_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();

    let entry = RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        None,
        "~/wisp/proj/data/input.fasta",
        "transfer",
    );
    store.record_remote_staging(&entry).await.unwrap();
    let mut bad = entry.clone();
    bad.id = "bad".into();
    bad.source = "mystery".into();
    assert!(store.record_remote_staging(&bad).await.is_err());

    let listed = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        store
            .mark_remote_staging_removed(&[entry.id.clone()])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .mark_remote_staging_removed(&[entry.id.clone()])
            .await
            .unwrap(),
        0
    );
    assert!(store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .list_remote_staging("p", "ssh:gpu", true)
            .await
            .unwrap()
            .len(),
        1
    );

    // External-reference audit counts only head versions on that server.
    store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: logical_artifact_id("p", "path:big.bam"),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "big.bam".into(),
            content_type: "data".into(),
            storage_path: "ssh://gpu/scratch/proj/artifacts/r1/big.bam".into(),
            logical_key: Some("path:big.bam".into()),
            size_bytes: None,
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::External,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .count_external_references_on_context("p", "ssh://gpu/")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_external_references_on_context("p", "ssh://other/")
            .await
            .unwrap(),
        0
    );

    let persist = RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        Some("r1".into()),
        "/scratch/proj/artifacts/r1/big.bam",
        "harvest_persist",
    );
    assert!(store.ensure_remote_staging(&persist).await.unwrap());
    assert!(!store.ensure_remote_staging(&persist).await.unwrap());
    assert_eq!(
        store
            .list_live_external_uris_on_context("p", "ssh://gpu/")
            .await
            .unwrap(),
        vec!["ssh://gpu/scratch/proj/artifacts/r1/big.bam".to_string()]
    );

    assert_eq!(
        store
            .mark_external_artifacts_source_discarded("ssh://gpu/")
            .await
            .unwrap(),
        1
    );
    assert!(store
        .ssh_uri_source_discarded("ssh://gpu/scratch/proj/artifacts/r1/big.bam")
        .await
        .unwrap());
    assert_eq!(
        store
            .count_external_references_on_context("p", "ssh://gpu/")
            .await
            .unwrap(),
        0
    );
    assert!(store
        .list_live_external_uris_on_context("p", "ssh://gpu/")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .mark_remote_staging_removed_for_context("ssh:gpu")
            .await
            .unwrap(),
        1
    );
    assert!(store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap()
        .is_empty());

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn auto_harvest_skip_and_lease_hold_are_pure_helpers() {
    assert!(!skip_auto_harvest(None));
    assert!(skip_auto_harvest(Some(1)));
    assert!(require_lifecycle_hold(true, "ok").is_ok());
    assert_eq!(
        require_lifecycle_hold(false, "Run lifecycle lease was lost").unwrap_err(),
        "Run lifecycle lease was lost"
    );
}

#[tokio::test]
async fn run_harvest_state_is_recorded_once_and_survives_reopen() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_run_harvest_state_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let mut run = RunRecord::new("r", "p", "local", "Run", "command");
    run.status = RunStatus::Succeeded;
    store.create_run(&run).await.unwrap();

    assert!(store
        .get_run("r")
        .await
        .unwrap()
        .unwrap()
        .harvested_at
        .is_none());
    assert!(store.mark_run_harvested("r").await.unwrap());
    assert!(store.get_run("r").await.unwrap().unwrap().is_harvested());
    let harvested_at = store
        .get_run("r")
        .await
        .unwrap()
        .unwrap()
        .harvested_at
        .unwrap();
    // Idempotent: a second mark does not rewrite the timestamp.
    assert!(!store.mark_run_harvested("r").await.unwrap());
    drop(store);

    // Reopening (legacy-database repair path) keeps the column and the value.
    let reopened = Store::open(&tmp).await.unwrap();
    assert_eq!(
        reopened.get_run("r").await.unwrap().unwrap().harvested_at,
        Some(harvested_at)
    );
    assert!(reopened
        .record_run_harvest_error("r", "remote artifact registration failed: boom")
        .await
        .unwrap());
    assert_eq!(
        reopened
            .get_run("r")
            .await
            .unwrap()
            .unwrap()
            .last_poll_error
            .as_deref(),
        Some("remote artifact registration failed: boom")
    );

    // Cleanup state: errors are retryable and cleared by a successful clean.
    assert!(reopened
        .record_run_cleanup_error("r", "rm failed: permission denied")
        .await
        .unwrap());
    let run = reopened.get_run("r").await.unwrap().unwrap();
    assert!(run.cleaned_at.is_none());
    assert_eq!(
        run.cleanup_error.as_deref(),
        Some("rm failed: permission denied")
    );
    assert!(reopened.mark_run_cleaned("r").await.unwrap());
    let run = reopened.get_run("r").await.unwrap().unwrap();
    assert!(run.cleaned_at.is_some());
    assert!(run.cleanup_error.is_none());
    // Idempotent, and errors no longer overwrite a cleaned run.
    assert!(!reopened.mark_run_cleaned("r").await.unwrap());
    assert!(!reopened
        .record_run_cleanup_error("r", "late")
        .await
        .unwrap());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn method_search_state_candidates_and_pause_lifecycle_are_durable() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_method_search_store_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("p", "proj", "workspace")
        .await
        .unwrap();
    store
        .create_frame("f", "p", "OPERON", "model")
        .await
        .unwrap();
    let spec_version = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: Some("spec-v1".into()),
            artifact_id: "spec-artifact".into(),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "method-search.json".into(),
            content_type: "application/json".into(),
            storage_path: ".wisp/artifacts/sha256/aa/spec.json".into(),
            logical_key: Some("method-search:spec".into()),
            size_bytes: Some(2),
            checksum: Some("a".repeat(64)),
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    let run = RunRecord::new("run", "p", "local", "Method search", "method_search");
    store.create_run(&run).await.unwrap();
    let state = MethodSearchRunState::new("run", &spec_version, "a".repeat(64)).unwrap();
    store.create_method_search_run_state(&state).await.unwrap();
    assert_eq!(
        store.method_search_run_status("run").await.unwrap(),
        Some(RunStatus::Draft)
    );

    let cancel_run = RunRecord::new(
        "cancel-draft",
        "p",
        "local",
        "Cancelled draft",
        "method_search",
    );
    store.create_run(&cancel_run).await.unwrap();
    store
        .create_method_search_run_state(
            &MethodSearchRunState::new("cancel-draft", &spec_version, "a".repeat(64)).unwrap(),
        )
        .await
        .unwrap();
    assert!(store
        .request_run_cancellation("cancel-draft")
        .await
        .unwrap());
    assert_eq!(
        store
            .method_search_run_status("cancel-draft")
            .await
            .unwrap(),
        Some(RunStatus::Cancelling)
    );
    assert!(store
        .claim_run_lifecycle("cancel-draft", "cancel-owner", 60)
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("cancel-draft", "cancel-owner", RunStatus::Cancelled, None,)
        .await
        .unwrap());

    let blob = MethodCandidateBlob {
        id: "blob".into(),
        run_id: "run".into(),
        kind: "source".into(),
        checksum: "b".repeat(64),
        size_bytes: 12,
        storage_path: ".wisp/method-search/run/blobs/bb/source.py".into(),
        created_at: 1,
    };
    store.save_method_candidate_blob(&blob).await.unwrap();
    let mut candidate = MethodCandidate::proposed(
        "candidate",
        "run",
        0,
        "baseline",
        "baseline",
        "b".repeat(64),
        "c".repeat(64),
    )
    .unwrap();
    store.insert_method_candidate(&candidate).await.unwrap();
    assert!(store
        .transition_method_candidate_to_evaluating("candidate")
        .await
        .unwrap());
    candidate.status = MethodCandidateStatus::Succeeded;
    candidate.primary_score = Some(0.5);
    candidate.utility = Some(0.5);
    candidate.source_blob_id = Some("blob".into());
    candidate.metrics_json = serde_json::json!({"accuracy":0.5}).to_string();
    candidate.finished_at = Some(2);
    assert!(store
        .finish_method_candidate(&candidate, MethodCandidateStatus::Evaluating)
        .await
        .unwrap());
    assert_eq!(
        store.list_method_candidates("run").await.unwrap(),
        vec![candidate]
    );

    assert!(store
        .activate_run_lifecycle("run", RunStatus::Running, "owner", 60)
        .await
        .unwrap());
    assert!(store
        .pause_method_search_run_owned("run", "owner", "user requested pause")
        .await
        .unwrap());
    assert_eq!(
        store.method_search_run_status("run").await.unwrap(),
        Some(RunStatus::Paused)
    );
    assert!(!store.project_has_active_runs("p").await.unwrap());
    assert!(store.resume_method_search_run("run").await.unwrap());
    assert_eq!(store.pause_method_searches_for_shutdown().await.unwrap(), 1);
    let shutdown_run = store.get_run("run").await.unwrap().unwrap();
    assert_eq!(shutdown_run.status, RunStatus::Paused);
    assert!(shutdown_run
        .last_poll_error
        .as_deref()
        .unwrap()
        .contains("graceful application shutdown"));
    assert!(store.resume_method_search_run("run").await.unwrap());
    assert_eq!(
        store
            .recover_interrupted_method_search_runs()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store.method_search_run_status("run").await.unwrap(),
        Some(RunStatus::Paused)
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn run_artifact_lineage_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_run_lineage_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP INDEX IF EXISTS ux_artifacts_project_logical_key")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DROP INDEX IF EXISTS ix_artifacts_project_logical_key")
        .execute(&store.pool)
        .await
        .unwrap();
    for trigger in [
        "trg_evidence_binding_source_project_insert",
        "trg_evidence_binding_source_project_update",
    ] {
        sqlx::query(&format!("DROP TRIGGER {trigger}"))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    for statement in [
        "DROP TABLE run_environment_snapshots",
        "DROP TABLE run_code_snapshots",
        "DROP TABLE run_outputs",
        "DROP TABLE run_inputs",
        "DROP TABLE external_resources",
        "ALTER TABLE artifacts DROP COLUMN logical_key",
        "ALTER TABLE artifact_versions DROP COLUMN materialization",
        "ALTER TABLE artifact_versions DROP COLUMN capture_timing",
        "ALTER TABLE artifact_dependencies DROP COLUMN basis",
        "ALTER TABLE artifact_dependencies DROP COLUMN confidence",
        "ALTER TABLE env_snapshots DROP COLUMN snapshot_json",
        "ALTER TABLE env_snapshots DROP COLUMN hash_algorithm",
    ] {
        sqlx::query(statement).execute(&store.pool).await.unwrap();
    }
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(RUN_ARTIFACT_LINEAGE_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(EXPLORATION_BRANCHES_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    for table in [
        "external_resources",
        "run_inputs",
        "run_outputs",
        "run_code_snapshots",
        "run_environment_snapshots",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        )
        .bind(table)
        .fetch_one(&repaired.pool)
        .await
        .unwrap();
        assert!(exists, "{table}");
    }
    for (table, column) in [
        ("artifacts", "logical_key"),
        ("artifact_versions", "materialization"),
        ("artifact_versions", "capture_timing"),
        ("artifact_dependencies", "basis"),
        ("artifact_dependencies", "confidence"),
        ("env_snapshots", "snapshot_json"),
        ("env_snapshots", "hash_algorithm"),
    ] {
        assert!(Store::has_column(&repaired.pool, table, column)
            .await
            .unwrap());
    }
    repaired.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_domain_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP TRIGGER trg_evidence_bindings_insert_draft")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE capsule_builds")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(PUBLICATION_DOMAIN_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type='table' AND name='capsule_builds')",
    )
    .fetch_one(&repaired.pool)
    .await
    .unwrap();
    let trigger_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type='trigger' AND name='trg_evidence_bindings_insert_draft')",
    )
    .fetch_one(&repaired.pool)
    .await
    .unwrap();
    assert!(table_exists);
    assert!(trigger_exists);

    repaired.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_freeze_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_freeze_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for trigger in [
        "trg_publication_freeze_attempt_revision_insert",
        "trg_publication_freezing_exit",
        "trg_frozen_evidence_artifact_version_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER {trigger}"))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    for statement in [
        "DROP TABLE publication_freeze_attempts",
        "ALTER TABLE publication_readiness_reports DROP COLUMN target_visibility",
        "ALTER TABLE publication_readiness_reports DROP COLUMN policy_json",
    ] {
        sqlx::query(statement).execute(&store.pool).await.unwrap();
    }
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(PUBLICATION_FREEZE_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    assert!(Store::has_column(
        &repaired.pool,
        "publication_readiness_reports",
        "target_visibility"
    )
    .await
    .unwrap());
    assert!(Store::has_column(
        &repaired.pool,
        "publication_readiness_reports",
        "policy_json"
    )
    .await
    .unwrap());
    for (kind, name) in [
        ("table", "publication_freeze_attempts"),
        ("trigger", "trg_publication_freeze_attempt_revision_insert"),
        ("trigger", "trg_publication_freezing_exit"),
        ("trigger", "trg_frozen_evidence_artifact_version_delete"),
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=? AND name=?)",
        )
        .bind(kind)
        .bind(name)
        .fetch_one(&repaired.pool)
        .await
        .unwrap();
        assert!(exists, "{kind} {name}");
    }

    repaired.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_verification_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_verification_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP TABLE reproduction_results")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DROP INDEX ix_reproduction_runs_source")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(PUBLICATION_VERIFICATION_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    for (kind, name) in [
        ("table", "reproduction_runs"),
        ("table", "reproduction_results"),
        ("index", "ix_reproduction_runs_revision"),
        ("index", "ix_reproduction_runs_source"),
        ("index", "ix_reproduction_results_run"),
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=? AND name=?)",
        )
        .bind(kind)
        .bind(name)
        .fetch_one(&repaired.pool)
        .await
        .unwrap();
        assert!(exists, "{kind} {name}");
    }

    repaired.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn turn_file_undo_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_turn_undo_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("ALTER TABLE message_resource_links DROP COLUMN created_artifact")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE message_resource_links DROP COLUMN created_version")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE turn_file_undo")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(TURN_FILE_UNDO_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let columns = sqlx::query("PRAGMA table_info(message_resource_links)")
        .fetch_all(&reopened.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String, _>("name").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert!(columns.contains("created_artifact"));
    assert!(columns.contains("created_version"));
    let undo_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='turn_file_undo'",
    )
    .fetch_one(&reopened.pool)
    .await
    .unwrap();
    assert_eq!(undo_table, 1);
    reopened.pool.close().await;

    Store::open(&tmp).await.unwrap().pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn agent_workflow_contract_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_workflow_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("ALTER TABLE agent_workflow_steps DROP COLUMN budget_json")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(AGENT_WORKFLOW_CONTRACTS_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let columns = sqlx::query("PRAGMA table_info(agent_workflow_steps)")
        .fetch_all(&reopened.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String, _>("name").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert!(columns.contains("input_contract_json"));
    assert!(columns.contains("output_contract_json"));
    assert!(columns.contains("budget_json"));
    assert!(reopened
        .schema_migrations()
        .await
        .unwrap()
        .contains(&AGENT_WORKFLOW_CONTRACTS_MIGRATION.to_string()));
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_plan_migration_repairs_partial_application() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_plan_partial_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("ALTER TABLE agent_workflow_steps DROP COLUMN spec_json")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(AGENT_WORKFLOW_PLANS_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let columns = sqlx::query("PRAGMA table_info(agent_workflow_steps)")
        .fetch_all(&reopened.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String, _>("name").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert!(columns.contains("template_id"));
    assert!(columns.contains("spec_json"));
    assert!(reopened
        .schema_migrations()
        .await
        .unwrap()
        .contains(&AGENT_WORKFLOW_PLANS_MIGRATION.to_string()));
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_attempt_migration_is_retry_safe() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_attempt_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP TABLE agent_workflow_attempts")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(AGENT_WORKFLOW_ATTEMPTS_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_workflow_attempts'",
    )
    .fetch_one(&reopened.pool)
    .await
    .unwrap();
    assert_eq!(table_exists, 1);
    assert!(reopened
        .schema_migrations()
        .await
        .unwrap()
        .contains(&AGENT_WORKFLOW_ATTEMPTS_MIGRATION.to_string()));
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_lineage_migration_is_retry_safe() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_lineage_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP INDEX ix_agent_workflow_attempts_parent")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(AGENT_WORKFLOW_LINEAGE_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let workflow_columns = sqlx::query("PRAGMA table_info(agent_workflows)")
        .fetch_all(&reopened.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    for column in [
        "root_workflow_id",
        "parent_attempt_id",
        "depth",
        "root_limits_json",
    ] {
        assert!(workflow_columns.contains(&column.to_string()));
    }
    let parent_index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' \
         AND name='ix_agent_workflow_attempts_parent'",
    )
    .fetch_one(&reopened.pool)
    .await
    .unwrap();
    assert_eq!(parent_index, 1);
    assert!(reopened
        .schema_migrations()
        .await
        .unwrap()
        .contains(&AGENT_WORKFLOW_LINEAGE_MIGRATION.to_string()));
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn background_agent_completion_is_delivered_and_resumed_exactly_once() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_delivery_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut workflow = AgentWorkflow::new("wf", "p", "workspace", "Background batch").unwrap();
    workflow.frame_id = Some("f".into());
    let step =
        AgentWorkflowStep::new("step", "wf", 0, "worker", "worker", "local", "Do work").unwrap();
    store
        .create_agent_workflow_plan(&workflow, &[step.clone()])
        .await
        .unwrap();
    assert!(store
        .approve_agent_workflow_plan("wf", workflow.version)
        .await
        .unwrap());

    let delivery = store
        .create_agent_workflow_delivery("wf", true)
        .await
        .unwrap();
    assert_eq!(delivery.generation, 1);
    assert_eq!(delivery.resume_status, "pending");
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let mut attempt =
        AgentWorkflowAttempt::queued("attempt-1", "wf", &step.id, 1, "request-1", "local", "{}")
            .unwrap();
    store.create_agent_workflow_attempt(&attempt).await.unwrap();
    attempt.status = AgentWorkflowAttemptStatus::Running;
    attempt.started_at = Some(1);
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Queued)
        .await
        .unwrap());
    attempt.status = AgentWorkflowAttemptStatus::Failed;
    attempt.error = Some("failed once".into());
    attempt.finished_at = Some(2);
    assert!(store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Running)
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Running,
            AgentWorkflowStatus::Failed,
        )
        .await
        .unwrap());
    let result = serde_json::json!({
        "type": "delegated_batch_completion",
        "workflow_id": "wf",
        "generation": 1,
        "result": {"status": "failed"}
    });
    assert!(store
        .complete_agent_workflow_delivery(&delivery.id, &result.to_string())
        .await
        .unwrap());

    // Simulate an application restart after terminal result persistence but
    // before the owning conversation is updated.
    drop(store);
    let store = Store::open(&tmp).await.unwrap();

    let delivered = store.deliver_agent_workflow_completions("f").await.unwrap();
    assert_eq!(delivered.len(), 1);
    assert!(store
        .deliver_agent_workflow_completions("f")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.message_count("f").await.unwrap(), 1);
    let row = sqlx::query("SELECT role,tool_name FROM messages WHERE frame_id='f'")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(row.try_get::<String, _>("role").unwrap(), "internal");
    assert_eq!(
        row.try_get::<String, _>("tool_name").unwrap(),
        AGENT_WORKFLOW_COMPLETION_TOOL
    );
    store
        .create_frame("branch", "p", "OPERON", "m")
        .await
        .unwrap();
    let internal = store.load_messages("f").await.unwrap().remove(0);
    store.append_message("branch", 1, &internal).await.unwrap();
    let branched_role: String =
        sqlx::query_scalar("SELECT role FROM messages WHERE frame_id='branch'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(branched_role, "internal");

    let claimed = store.claim_agent_workflow_auto_resumes("f").await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert!(store
        .claim_agent_workflow_auto_resumes("f")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .finish_agent_workflow_auto_resumes(&[delivery.id.clone()], true, None)
            .await
            .unwrap(),
        1
    );

    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Failed,
            AgentWorkflowStatus::Approved,
        )
        .await
        .unwrap());
    let retry = store
        .create_agent_workflow_delivery("wf", false)
        .await
        .unwrap();
    assert_eq!(retry.generation, 2);
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Approved,
            AgentWorkflowStatus::Running,
        )
        .await
        .unwrap());
    let mut retry_attempt =
        AgentWorkflowAttempt::queued("attempt-2", "wf", &step.id, 2, "request-2", "local", "{}")
            .unwrap();
    store
        .create_agent_workflow_attempt(&retry_attempt)
        .await
        .unwrap();
    retry_attempt.status = AgentWorkflowAttemptStatus::Running;
    retry_attempt.started_at = Some(3);
    assert!(store
        .update_agent_workflow_attempt(&retry_attempt, AgentWorkflowAttemptStatus::Queued)
        .await
        .unwrap());
    retry_attempt.status = AgentWorkflowAttemptStatus::Succeeded;
    retry_attempt.response_json = Some("{}".into());
    retry_attempt.finished_at = Some(4);
    assert!(store
        .update_agent_workflow_attempt(&retry_attempt, AgentWorkflowAttemptStatus::Running)
        .await
        .unwrap());
    assert!(store
        .transition_agent_workflow_status(
            "wf",
            AgentWorkflowStatus::Running,
            AgentWorkflowStatus::Succeeded,
        )
        .await
        .unwrap());
    let retry_result = serde_json::json!({
        "type": "delegated_batch_completion",
        "workflow_id": "wf",
        "generation": 2,
        "result": {"status": "succeeded"}
    });
    assert!(store
        .complete_agent_workflow_delivery(&retry.id, &retry_result.to_string())
        .await
        .unwrap());
    let retry_delivered = store.deliver_agent_workflow_completions("f").await.unwrap();
    assert_eq!(retry_delivered.len(), 1);
    assert_eq!(retry_delivered[0].id, retry.id);
    assert!(store
        .deliver_agent_workflow_completions("f")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.message_count("f").await.unwrap(), 2);

    store.truncate_messages("f", 0).await.unwrap();
    let retained_attempts = store.list_agent_workflow_attempts("wf").await.unwrap();
    assert_eq!(retained_attempts.len(), 2);
    assert_eq!(retained_attempts[0].error.as_deref(), Some("failed once"));
    assert!(store.list_agent_workflow_deliveries("wf").await.unwrap()[0]
        .result_json
        .is_some());

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn agent_workflow_delivery_migration_is_retry_safe() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_delivery_migration_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    sqlx::query("DROP TABLE agent_workflow_deliveries")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(AGENT_WORKFLOW_DELIVERIES_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let reopened = Store::open(&tmp).await.unwrap();
    let table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_workflow_deliveries'",
    )
    .fetch_one(&reopened.pool)
    .await
    .unwrap();
    assert_eq!(table_exists, 1);
    assert!(reopened
        .schema_migrations()
        .await
        .unwrap()
        .contains(&AGENT_WORKFLOW_DELIVERIES_MIGRATION.to_string()));
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn reserved_background_generation_is_failed_instead_of_resumed_after_restart() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_delivery_prestart_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut workflow = AgentWorkflow::new("wf", "p", "workspace", "Background batch").unwrap();
    workflow.frame_id = Some("f".into());
    store
        .create_agent_workflow_plan(&workflow, &[])
        .await
        .unwrap();
    assert!(store
        .approve_agent_workflow_plan("wf", workflow.version)
        .await
        .unwrap());
    store
        .create_agent_workflow_delivery("wf", false)
        .await
        .unwrap();

    assert_eq!(
        store.recover_interrupted_agent_workflows().await.unwrap(),
        (0, 1)
    );
    assert_eq!(
        store
            .get_agent_workflow("wf")
            .await
            .unwrap()
            .unwrap()
            .status,
        AgentWorkflowStatus::Failed
    );
    assert_eq!(
        store
            .list_incomplete_agent_workflow_deliveries()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.recover_interrupted_agent_workflows().await.unwrap(),
        (0, 0)
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
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
    assert_eq!(run.progress_json, "{}");
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&SSH_RUN_CONTROL_MIGRATION.to_string()));
    assert!(store
        .schema_migrations()
        .await
        .unwrap()
        .contains(&RUN_PROGRESS_MIGRATION.to_string()));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn run_manager_roundtrip_and_lifecycle() {
    let tmp = std::env::temp_dir().join(format!("wisp_store_runs_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f1", "p", "OPERON", "m").await.unwrap();
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
    run.progress_json = serde_json::to_string(&RunProgress {
        phase: "uploading".into(),
        direction: "upload".into(),
        completed_bytes: 512,
        total_bytes: 1024,
        files_completed: 0,
        files_total: 1,
        current_file: Some("counts.tsv".into()),
        bytes_per_second: Some(256),
        eta_seconds: Some(2),
        updated_at: 1,
    })
    .unwrap();
    store.create_run(&run).await.unwrap();

    let got = store.get_run("r1").await.unwrap().unwrap();
    assert_eq!(got.status, RunStatus::Draft);
    assert_eq!(got.command.as_deref(), Some("python qc.py"));
    assert_eq!(got.input_refs_json, r#"["data/raw/counts.tsv"]"#);
    assert_eq!(got.timeout_secs, Some(900));
    let progress: RunProgress = serde_json::from_str(&got.progress_json).unwrap();
    assert_eq!(progress.completed_bytes, 512);

    assert!(store
        .activate_run_lifecycle("r1", RunStatus::Submitted, "roundtrip-owner", 60)
        .await
        .unwrap());
    assert!(store
        .set_run_remote_handle_owned(
            "r1",
            "roundtrip-owner",
            r#"{"kind":"ssh_direct","pid":42,"start_time":7}"#,
            "/scratch/wisp/r1",
        )
        .await
        .unwrap());
    assert!(store
        .transition_run_to_running_owned("r1", "roundtrip-owner")
        .await
        .unwrap());
    assert!(store
        .record_run_poll_owned(
            "r1",
            "roundtrip-owner",
            Some("ok stdout"),
            None,
            Some("temporary error"),
        )
        .await
        .unwrap());
    assert!(store
        .record_run_poll_owned("r1", "roundtrip-owner", None, Some("warn stderr"), None,)
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("r1", "roundtrip-owner", RunStatus::Succeeded, Some(0),)
        .await
        .unwrap());

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
async fn run_poll_summaries_omit_large_detail_payloads() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_run_summaries_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let mut run = RunRecord::new("large", "p", "local", "Large", "command");
    run.command = Some(format!("SECRET_COMMAND{}", "c".repeat(32 * 1024)));
    run.stdout_tail = Some("x".repeat(64 * 1024));
    run.stderr_tail = Some("y".repeat(64 * 1024));
    run.env_snapshot_json = format!(r#"{{"SECRET_ENV":"{}"}}"#, "e".repeat(32 * 1024));
    store.create_run(&run).await.unwrap();

    let summaries = store
        .list_run_summaries_in_scope(&StateScope::mainline("p"))
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, "large");
    let json = serde_json::to_string(&summaries).unwrap();
    assert!(!json.contains("SECRET_COMMAND"));
    assert!(!json.contains("SECRET_ENV"));
    assert!(
        json.len() < 5_000,
        "summary payload was {} bytes",
        json.len()
    );

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

    assert!(store
        .activate_run_lifecycle("r1", RunStatus::Submitted, "cancel-owner", 60)
        .await
        .unwrap());
    assert!(store.request_run_cancellation("r1").await.unwrap());
    assert_eq!(
        store.get_run("r1").await.unwrap().unwrap().status,
        RunStatus::Cancelling
    );
    assert!(store
        .finish_active_run_owned("r1", "cancel-owner", RunStatus::TimedOut, None)
        .await
        .unwrap());
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
    for (id, status) in [
        ("submitted", RunStatus::Submitted),
        ("running", RunStatus::Running),
        ("cancelling", RunStatus::Running),
    ] {
        assert!(store
            .activate_run_lifecycle(id, status, "race-owner", 60)
            .await
            .unwrap());
    }
    assert!(store.request_run_cancellation("cancelling").await.unwrap());

    let active = store.list_active_runs().await.unwrap();
    assert_eq!(active.len(), 3);
    assert!(active.iter().any(|run| run.status == RunStatus::Cancelling));
    assert!(store
        .mark_run_lost_owned("running", "race-owner")
        .await
        .unwrap());
    assert!(!store
        .mark_run_lost_owned("running", "race-owner")
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("cancelling", "race-owner", RunStatus::Cancelled, None,)
        .await
        .unwrap());
    assert!(!store
        .finish_active_run_owned("cancelling", "race-owner", RunStatus::TimedOut, None,)
        .await
        .unwrap());
    assert!(!store
        .finish_active_run_owned("draft", "race-owner", RunStatus::Failed, Some(1))
        .await
        .unwrap());
    assert!(store
        .finish_active_run_owned("submitted", "race-owner", RunStatus::Succeeded, Some(0),)
        .await
        .unwrap());
    assert_eq!(
        store.get_run("cancelling").await.unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    assert!(store
        .finish_active_run_owned("draft", "race-owner", RunStatus::Running, None)
        .await
        .is_err());

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
    let progress = RunProgress {
        phase: "uploading".into(),
        direction: "upload".into(),
        completed_bytes: 4,
        total_bytes: 8,
        files_completed: 0,
        files_total: 1,
        current_file: Some("input.dat".into()),
        bytes_per_second: Some(2),
        eta_seconds: Some(2),
        updated_at: chrono::Utc::now().timestamp(),
    };
    assert!(!store
        .update_run_progress_owned("lease", "owner-b", &progress)
        .await
        .unwrap());
    assert!(store
        .update_run_progress_owned("lease", "owner-a", &progress)
        .await
        .unwrap());
    assert!(store
        .get_run("lease")
        .await
        .unwrap()
        .unwrap()
        .progress_json
        .contains("input.dat"));
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
async fn artifacts_keep_version_lineage() {
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
    let latest = store.get_artifact_version(&second).await.unwrap().unwrap();
    let original = store.get_artifact_version(&first).await.unwrap().unwrap();
    assert_eq!(latest.version_number, 2);
    assert_eq!(latest.parent_version_id.as_deref(), Some(first.as_str()));
    assert_eq!(latest.storage_path, "reports/v2.md");
    assert_eq!(original.version_number, 1);

    assert!(store
        .relocate_artifact_storage("a", "durable/isolated-report.md")
        .await
        .unwrap());
    assert!(!store
        .relocate_artifact_storage("missing", "unused")
        .await
        .unwrap());
    assert_eq!(
        store.get_artifact("a").await.unwrap().unwrap().2,
        "durable/isolated-report.md"
    );
    assert_eq!(
        store
            .get_artifact_version(&second)
            .await
            .unwrap()
            .unwrap()
            .storage_path,
        "durable/isolated-report.md"
    );
    assert_eq!(
        store
            .get_artifact_version(&first)
            .await
            .unwrap()
            .unwrap()
            .storage_path,
        "reports/v1.md"
    );

    let graph = store.research_graph("p").await.unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.id == "artifact:a" && node.ref_id.as_deref() == Some("a")));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_revisions_clone_exact_evidence_and_freeze_history() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_domain_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for (project, frame) in [("p", "f"), ("other", "other-frame")] {
        store.create_project(project, project, "").await.unwrap();
        store
            .create_frame(frame, project, "OPERON", "m")
            .await
            .unwrap();
    }
    store
        .create_run(&RunRecord::new("run", "p", "local", "Analysis", "command"))
        .await
        .unwrap();
    store
        .create_run(&RunRecord::new(
            "other-run",
            "other",
            "local",
            "Other",
            "command",
        ))
        .await
        .unwrap();

    let artifact_id = logical_artifact_id("p", "figure:main");
    let mut versions = Vec::new();
    for (checksum, storage) in [
        ("a".repeat(64), "snapshots/v1.png"),
        ("b".repeat(64), "snapshots/v2.png"),
    ] {
        versions.push(
            store
                .save_artifact_version(&ArtifactVersionDraft {
                    version_id: None,
                    artifact_id: artifact_id.clone(),
                    project_id: "p".into(),
                    root_frame_id: "f".into(),
                    filename: "main.png".into(),
                    content_type: "image/png".into(),
                    storage_path: storage.into(),
                    logical_key: Some("figure:main".into()),
                    size_bytes: Some(8),
                    checksum: Some(checksum),
                    producing_run_id: Some("run".into()),
                    env_snapshot_hash: None,
                    materialization: ArtifactMaterialization::Snapshot,
                    capture_timing: ArtifactCaptureTiming::AtCreation,
                })
                .await
                .unwrap(),
        );
    }
    let other_version = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: "other-artifact".into(),
            project_id: "other".into(),
            root_frame_id: "other-frame".into(),
            filename: "other.png".into(),
            content_type: "image/png".into(),
            storage_path: "other.png".into(),
            logical_key: Some("figure:other".into()),
            size_bytes: Some(1),
            checksum: Some("c".repeat(64)),
            producing_run_id: Some("other-run".into()),
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();

    store
        .create_publication("publication", "p", "T cell paper", "Evidence test")
        .await
        .unwrap();
    let revision = store
        .create_publication_revision("revision-1", "publication", None, "Submission v1")
        .await
        .unwrap();
    assert_eq!(revision.revision_number, 1);
    for (id, parent, kind, title, ordinal) in [
        ("section", None, PublicationItemKind::Section, "Results", 0),
        (
            "claim",
            Some("section"),
            PublicationItemKind::Claim,
            "Exhaustion increases",
            0,
        ),
        (
            "figure",
            Some("section"),
            PublicationItemKind::Figure,
            "Figure 2B",
            1,
        ),
        (
            "methods",
            Some("section"),
            PublicationItemKind::Methods,
            "Differential analysis",
            2,
        ),
    ] {
        store
            .save_publication_item(&PublicationItem {
                id: id.into(),
                revision_id: "revision-1".into(),
                parent_item_id: parent.map(str::to_string),
                kind,
                title: title.into(),
                content: String::new(),
                ordinal,
                metadata_json: "{}".into(),
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
    }
    store
        .save_publication_item_link(&PublicationItemLink {
            id: "supports".into(),
            revision_id: "revision-1".into(),
            source_item_id: "figure".into(),
            target_item_id: "claim".into(),
            relation: "supports".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    for binding in [
        EvidenceBindingDraft {
            id: "binding-old".into(),
            revision_id: "revision-1".into(),
            item_id: Some("figure".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: versions[0].clone(),
            purpose: "Figure 2B".into(),
            supported_claim_item_id: Some("claim".into()),
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Public,
        },
        EvidenceBindingDraft {
            id: "binding-new".into(),
            revision_id: "revision-1".into(),
            item_id: Some("figure".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: versions[1].clone(),
            purpose: "Updated Figure 2B".into(),
            supported_claim_item_id: Some("claim".into()),
            selection_state: EvidenceSelectionState::Candidate,
            visibility: EvidenceVisibility::Public,
        },
        EvidenceBindingDraft {
            id: "binding-run".into(),
            revision_id: "revision-1".into(),
            item_id: Some("methods".into()),
            source_kind: EvidenceSourceKind::Run,
            source_id: "run".into(),
            purpose: "Methods execution".into(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Restricted,
        },
    ] {
        store.save_evidence_binding(&binding).await.unwrap();
    }
    assert!(store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "cross-project".into(),
            revision_id: "revision-1".into(),
            item_id: Some("figure".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: other_version.clone(),
            purpose: String::new(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Candidate,
            visibility: EvidenceVisibility::Private,
        })
        .await
        .is_err());
    assert!(sqlx::query(
        "INSERT INTO evidence_bindings(\
               id,revision_id,source_kind,source_id,artifact_version_id,purpose,\
               source_snapshot_json,created_at,updated_at\
             ) VALUES('raw-cross-project','revision-1','artifact_version',?,?,'','{}',1,1)",
    )
    .bind(&other_version)
    .bind(&other_version)
    .execute(&store.pool)
    .await
    .is_err());
    store
        .save_evidence_review(&EvidenceReview {
            id: "review".into(),
            binding_id: "binding-old".into(),
            reviewer: "alice".into(),
            method: "manual_traceability".into(),
            verified_at: 10,
            environment_json: "{}".into(),
            comparator_json: r#"{"kind":"visual"}"#.into(),
            tolerance_json: "{}".into(),
            result: "passed".into(),
            report_json: r#"{"note":"checked"}"#.into(),
            created_at: 10,
        })
        .await
        .unwrap();
    store
        .update_evidence_binding_selection(
            "binding-old",
            EvidenceSelectionState::Selected,
            EvidenceVisibility::Public,
        )
        .await
        .unwrap();
    let reviewed_binding = store
        .get_evidence_binding("binding-old")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviewed_binding.review_state, EvidenceReviewState::Reviewed);
    assert_eq!(
        reviewed_binding.reproduction_state,
        EvidenceReproductionState::NotRun
    );
    store
        .save_evidence_supersession(&EvidenceSupersession {
            id: "supersession".into(),
            revision_id: "revision-1".into(),
            old_binding_id: "binding-old".into(),
            new_binding_id: "binding-new".into(),
            reason: "Updated analysis".into(),
            created_at: 11,
        })
        .await
        .unwrap();
    store
        .save_publication_waiver(&PublicationWaiver {
            id: "waiver".into(),
            revision_id: "revision-1".into(),
            finding_code: "restricted-input".into(),
            author: "alice".into(),
            reason: "DUA requires manifest-only disclosure".into(),
            created_at: 12,
        })
        .await
        .unwrap();

    let clone = store
        .clone_publication_revision("revision-1", "revision-2", "Revision v2")
        .await
        .unwrap();
    assert_eq!(clone.parent_revision_id.as_deref(), Some("revision-1"));
    assert_eq!(clone.state, PublicationRevisionState::Draft);
    assert_eq!(
        store
            .list_publication_items("revision-2")
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        store
            .list_publication_item_links("revision-2")
            .await
            .unwrap()
            .len(),
        1
    );
    let cloned_bindings = store.list_evidence_bindings("revision-2").await.unwrap();
    assert_eq!(cloned_bindings.len(), 3);
    assert!(cloned_bindings.iter().all(|binding| {
        binding.id != "binding-old" && binding.id != "binding-new" && binding.id != "binding-run"
    }));
    let cloned_old = cloned_bindings
        .iter()
        .find(|binding| binding.source_id == versions[0])
        .unwrap();
    let cloned_new = cloned_bindings
        .iter()
        .find(|binding| binding.source_id == versions[1])
        .unwrap();
    assert_eq!(
        cloned_old.source_snapshot_json,
        reviewed_binding.source_snapshot_json
    );
    assert_eq!(
        store
            .list_evidence_reviews(&cloned_old.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let cloned_supersession = store
        .list_evidence_supersessions("revision-2")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(cloned_supersession.old_binding_id, cloned_old.id);
    assert_eq!(cloned_supersession.new_binding_id, cloned_new.id);
    assert_eq!(
        store
            .list_publication_waivers("revision-2")
            .await
            .unwrap()
            .len(),
        1
    );

    let third_version = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id,
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "main.png".into(),
            content_type: "image/png".into(),
            storage_path: "snapshots/v3.png".into(),
            logical_key: Some("figure:main".into()),
            size_bytes: Some(8),
            checksum: Some("d".repeat(64)),
            producing_run_id: Some("run".into()),
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    let cloned_figure = store
        .list_publication_items("revision-2")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.kind == PublicationItemKind::Figure)
        .unwrap();
    let clone_third = store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "clone-third".into(),
            revision_id: "revision-2".into(),
            item_id: Some(cloned_figure.id.clone()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: third_version.clone(),
            purpose: "Revision-only replacement".into(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Public,
        })
        .await
        .unwrap();
    store
        .save_evidence_supersession(&EvidenceSupersession {
            id: "clone-supersession-update".into(),
            revision_id: "revision-2".into(),
            old_binding_id: cloned_old.id.clone(),
            new_binding_id: clone_third.id,
            reason: "Revision-only update".into(),
            created_at: 13,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .list_evidence_supersessions("revision-1")
            .await
            .unwrap()[0]
            .new_binding_id,
        "binding-new"
    );
    assert_eq!(
        store
            .get_evidence_binding("binding-old")
            .await
            .unwrap()
            .unwrap()
            .source_id,
        versions[0]
    );

    let cloned_link_id = store
        .list_publication_item_links("revision-2")
        .await
        .unwrap()[0]
        .id
        .clone();
    store
        .delete_publication_item_link(&cloned_link_id)
        .await
        .unwrap();
    assert!(store
        .list_publication_item_links("revision-2")
        .await
        .unwrap()
        .is_empty());
    store
        .save_publication_item(&PublicationItem {
            id: "temporary-supplement".into(),
            revision_id: "revision-2".into(),
            parent_item_id: Some(cloned_figure.id),
            kind: PublicationItemKind::Supplement,
            title: "Temporary supplement".into(),
            content: String::new(),
            ordinal: 0,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
    store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "temporary-binding".into(),
            revision_id: "revision-2".into(),
            item_id: Some("temporary-supplement".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: versions[0].clone(),
            purpose: "Temporary evidence".into(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Candidate,
            visibility: EvidenceVisibility::Private,
        })
        .await
        .unwrap();
    assert!(store
        .research_graph("p")
        .await
        .unwrap()
        .edges
        .iter()
        .any(|edge| edge.id == "publication-evidence:temporary-binding"));
    store
        .delete_publication_item("temporary-supplement")
        .await
        .unwrap();
    assert!(store
        .get_evidence_binding("temporary-binding")
        .await
        .unwrap()
        .is_none());
    assert!(!store
        .research_graph("p")
        .await
        .unwrap()
        .edges
        .iter()
        .any(|edge| edge.id == "publication-evidence:temporary-binding"));

    sqlx::query("UPDATE publication_revisions SET state='freezing' WHERE id='revision-2'")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store
        .clone_publication_revision("revision-2", "invalid-clone", "Invalid")
        .await
        .is_err());
    sqlx::query("UPDATE publication_revisions SET state='draft' WHERE id='revision-2'")
        .execute(&store.pool)
        .await
        .unwrap();

    sqlx::query(
        "UPDATE publication_revisions SET state='frozen',manifest_json='{}',\
         manifest_sha256=?,frozen_at=20,updated_at=20 WHERE id='revision-1'",
    )
    .bind("e".repeat(64))
    .execute(&store.pool)
    .await
    .unwrap();
    assert!(store
        .update_draft_publication_revision("revision-1", "mutated")
        .await
        .is_err());
    assert!(store
        .update_evidence_binding_selection(
            "binding-old",
            EvidenceSelectionState::Rejected,
            EvidenceVisibility::Private,
        )
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE publication_items SET title='mutated' WHERE id='figure'")
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM evidence_bindings WHERE id='binding-old'")
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert!(store
        .delete_draft_publication_revision("revision-1")
        .await
        .is_err());
    assert!(store.delete_publication("publication").await.is_err());

    let graph = store.research_graph("p").await.unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.id == "publication:publication"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.id == "publication-evidence:binding-old"));
    let drift = store
        .list_publication_evidence_drift("revision-1")
        .await
        .unwrap();
    let old_binding_drift = drift
        .iter()
        .find(|entry| entry.binding_id == "binding-old")
        .unwrap();
    assert!(old_binding_drift.has_drift);
    assert_eq!(old_binding_drift.bound_version_id, versions[0]);
    assert_eq!(old_binding_drift.latest_version_id, third_version);
    assert!(sqlx::query("DELETE FROM artifact_versions WHERE id=?")
        .bind(&versions[0])
        .execute(&store.pool)
        .await
        .is_err());
    store
        .publish_publication_revision("revision-1")
        .await
        .unwrap();
    assert_eq!(
        store
            .get_publication_revision("revision-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        PublicationRevisionState::Published
    );
    store.delete_session("f", "p").await.unwrap();
    assert!(store
        .get_artifact_version(&versions[0])
        .await
        .unwrap()
        .is_some());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn fine_grained_publication_evidence_keeps_immutable_source_snapshots() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_fine_evidence_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("prefix evidence suffix"))
        .await
        .unwrap();
    let mut assistant = Message::assistant("");
    assistant.tool_calls.push(wisp_llm::ToolCall {
        id: "call-1".into(),
        kind: "function".into(),
        function: wisp_llm::FunctionCall {
            name: "read".into(),
            arguments: r#"{"path":"result.txt"}"#.into(),
        },
    });
    store.append_message("f", 2, &assistant).await.unwrap();
    store
        .append_message("f", 3, &Message::tool("call-1", "read", "stable result"))
        .await
        .unwrap();
    store
        .insert_execution_log(&ExecLog {
            id: "execution-1".into(),
            frame_id: "f".into(),
            cell_index: 0,
            tool: "python".into(),
            language: "python".into(),
            source: "print('stable')".into(),
            stdout: "stable\n".into(),
            stderr: String::new(),
            exit_status: "ok".into(),
            wall_s: Some(0.1),
            files_written: vec!["result.txt".into()],
            files_read: vec![],
            env_hash: Some("environment".into()),
        })
        .await
        .unwrap();
    store
        .save_external_resource(&ExternalResource {
            id: "dataset-1".into(),
            project_id: "p".into(),
            kind: "dataset".into(),
            uri: "doi:10.0000/example".into(),
            version: Some("v1".into()),
            checksum: Some("a".repeat(64)),
            size_bytes: Some(42),
            license: Some("CC-BY-4.0".into()),
            visibility: "public".into(),
            access_instructions: Some("Resolve the DOI".into()),
            accessed_at: Some(1),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .create_publication("publication", "p", "Paper", "")
        .await
        .unwrap();
    store
        .create_publication_revision("revision", "publication", None, "Submission")
        .await
        .unwrap();
    store
        .save_publication_item(&PublicationItem {
            id: "item".into(),
            revision_id: "revision".into(),
            parent_item_id: None,
            kind: PublicationItemKind::Methods,
            title: "Methods".into(),
            content: String::new(),
            ordinal: 0,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();

    let message_locator = canonical_json(&serde_json::json!({
        "byte_end": 15,
        "byte_start": 7,
        "frame_id": "f",
        "message_seq": 1,
    }));
    let tool_locator = canonical_json(&serde_json::json!({
        "frame_id": "f",
        "message_seq": 2,
        "tool_call_id": "call-1",
    }));
    for (id, kind, source_id) in [
        (
            "message-binding",
            EvidenceSourceKind::MessageSpan,
            message_locator.as_str(),
        ),
        (
            "tool-binding",
            EvidenceSourceKind::ToolCall,
            tool_locator.as_str(),
        ),
        (
            "execution-binding",
            EvidenceSourceKind::ExecutionLog,
            "execution-1",
        ),
        ("code-binding", EvidenceSourceKind::CodeCell, "execution-1"),
        (
            "external-binding",
            EvidenceSourceKind::ExternalResource,
            "dataset-1",
        ),
    ] {
        store
            .save_evidence_binding(&EvidenceBindingDraft {
                id: id.into(),
                revision_id: "revision".into(),
                item_id: Some("item".into()),
                source_kind: kind,
                source_id: source_id.into(),
                purpose: "Exact source".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Private,
            })
            .await
            .unwrap();
    }

    let before = store
        .list_evidence_bindings("revision")
        .await
        .unwrap()
        .into_iter()
        .map(|binding| (binding.id, binding.source_snapshot_json))
        .collect::<std::collections::BTreeMap<_, _>>();
    for snapshot in before.values() {
        let value: serde_json::Value = serde_json::from_str(snapshot).unwrap();
        let anchor = value.get("anchor").unwrap();
        assert_eq!(
            value["anchor_sha256"],
            canonical_json_sha256(anchor).1.as_str()
        );
    }
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&before["message-binding"]).unwrap()["anchor"]
            ["text_snapshot"],
        "evidence"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&before["tool-binding"]).unwrap()["anchor"]
            ["result"],
        "stable result"
    );

    store.delete_session("f", "p").await.unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM frames WHERE id='f')")
            .fetch_one(&store.pool)
            .await
            .unwrap()
    );
    let after = store
        .list_evidence_bindings("revision")
        .await
        .unwrap()
        .into_iter()
        .map(|binding| (binding.id, binding.source_snapshot_json))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(after, before);
    assert!(store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "new-message-binding".into(),
            revision_id: "revision".into(),
            item_id: Some("item".into()),
            source_kind: EvidenceSourceKind::MessageSpan,
            source_id: message_locator,
            purpose: String::new(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Private,
        })
        .await
        .is_err());

    store.pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_freeze_commit_rolls_back_all_late_captures_on_failure() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_freeze_atomic_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let old_version_id = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: Some("version-old".into()),
            artifact_id: "artifact".into(),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "result.txt".into(),
            content_type: "text/plain".into(),
            storage_path: "result.txt".into(),
            logical_key: Some("result".into()),
            size_bytes: None,
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Reference,
            capture_timing: ArtifactCaptureTiming::Unknown,
        })
        .await
        .unwrap();
    store
        .create_publication("publication", "p", "Paper", "")
        .await
        .unwrap();
    store
        .create_publication_revision("revision", "publication", None, "Submission")
        .await
        .unwrap();
    store
        .save_publication_item(&PublicationItem {
            id: "supplement".into(),
            revision_id: "revision".into(),
            parent_item_id: None,
            kind: PublicationItemKind::Supplement,
            title: "Supplement".into(),
            content: String::new(),
            ordinal: 0,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
    store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "binding".into(),
            revision_id: "revision".into(),
            item_id: Some("supplement".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: old_version_id.clone(),
            purpose: "Supplement bytes".into(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Public,
        })
        .await
        .unwrap();

    let policy = PublicationFreezePolicy {
        phi_pii_reviewed: true,
        redistribution_reviewed: true,
        ..PublicationFreezePolicy::default()
    };
    store
        .begin_publication_freeze("revision", "attempt", &policy)
        .await
        .unwrap();
    let policy_value = serde_json::to_value(&policy).unwrap();
    let (manifest_json, manifest_sha256) = canonical_json_sha256(&serde_json::json!({
        "blockers": [],
        "capability_level": "archived",
        "omissions": [],
        "policy": policy_value.clone(),
        "publication_revision_id": "revision",
        "schema_version": 1,
        "target_visibility": "public",
        "warnings": [],
    }));
    let readiness = PublicationReadiness {
        revision_id: "revision".into(),
        target_visibility: EvidenceVisibility::Public,
        capability_level: PublicationCapabilityLevel::Archived,
        blockers: Vec::new(),
        warnings: Vec::new(),
        omissions: Vec::new(),
        manifest_json,
        manifest_sha256,
        can_freeze: true,
    };
    let capture = |new_version_id: &str, checksum: &str| PublicationLateCapture {
        binding_ids: vec!["binding".into()],
        old_version_id: old_version_id.clone(),
        new_version_id: new_version_id.into(),
        artifact_id: "artifact".into(),
        expected_latest_version_id: Some(old_version_id.clone()),
        version_number: 2,
        content_type: "text/plain".into(),
        storage_path: format!(".wisp/artifacts/sha256/{checksum}"),
        size_bytes: 4,
        checksum: checksum.into(),
        materialization: ArtifactMaterialization::Snapshot,
        source_snapshot_json: canonical_json(&serde_json::json!({
            "capture_timing": "late",
            "historical_content_verified": false,
            "sha256": checksum,
        })),
    };
    let commit = PublicationFreezeCommit {
        revision_id: "revision".into(),
        attempt_id: "attempt".into(),
        policy_json: canonical_json(&policy_value),
        readiness,
        late_captures: vec![
            capture("version-capture-a", &"a".repeat(64)),
            capture("version-capture-b", &"b".repeat(64)),
        ],
    };

    assert!(store.commit_publication_freeze(&commit).await.is_err());
    for version_id in ["version-capture-a", "version-capture-b"] {
        assert!(store
            .get_artifact_version(version_id)
            .await
            .unwrap()
            .is_none());
    }
    assert_eq!(
        store
            .get_evidence_binding("binding")
            .await
            .unwrap()
            .unwrap()
            .source_id,
        old_version_id
    );
    assert_eq!(
        store
            .get_publication_revision("revision")
            .await
            .unwrap()
            .unwrap()
            .state,
        PublicationRevisionState::Freezing
    );
    let attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM publication_freeze_attempts WHERE id='attempt'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    assert!(store
        .get_publication_readiness_report("revision")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .abort_publication_freeze("revision", "attempt")
        .await
        .unwrap());
    store
        .begin_publication_freeze("revision", "interrupted", &policy)
        .await
        .unwrap();
    assert_eq!(
        store
            .recover_stale_publication_freezes(i64::MAX)
            .await
            .unwrap(),
        ["revision"]
    );
    assert_eq!(
        store
            .get_publication_revision("revision")
            .await
            .unwrap()
            .unwrap()
            .state,
        PublicationRevisionState::Draft
    );

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn runs_bind_exact_artifact_versions_code_and_environment() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_exact_run_lineage_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .create_run(&RunRecord::new("run", "p", "local", "Analysis", "command"))
        .await
        .unwrap();

    let input_key = "path:data/input.csv";
    let input_artifact = logical_artifact_id("p", input_key);
    let input_version = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: input_artifact,
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "input.csv".into(),
            content_type: "text/csv".into(),
            storage_path: ".wisp/artifacts/sha256/aa/input.csv".into(),
            logical_key: Some(input_key.into()),
            size_bytes: Some(4),
            checksum: Some("a".repeat(64)),
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    store
        .save_run_input(&RunInput {
            id: "input".into(),
            run_id: "run".into(),
            artifact_version_id: Some(input_version.clone()),
            external_resource_id: None,
            source_ref: "data/input.csv".into(),
            role: "counts".into(),
            required: true,
            basis: LineageBasis::Declared,
            confidence: LineageConfidence::Exact,
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .save_external_resource(&ExternalResource {
            id: "restricted-cohort".into(),
            project_id: "p".into(),
            kind: "dataset".into(),
            uri: "s3://controlled/cohort-v3".into(),
            version: Some("v3".into()),
            checksum: Some("d".repeat(64)),
            size_bytes: Some(1024),
            license: Some("DUA-42".into()),
            visibility: "restricted".into(),
            access_instructions: Some("Request access from the data custodian".into()),
            accessed_at: Some(1),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .save_run_input(&RunInput {
            id: "external-input".into(),
            run_id: "run".into(),
            artifact_version_id: None,
            external_resource_id: Some("restricted-cohort".into()),
            source_ref: "s3://controlled/cohort-v3".into(),
            role: "controlled_cohort".into(),
            required: true,
            basis: LineageBasis::Declared,
            confidence: LineageConfidence::Exact,
            created_at: 2,
        })
        .await
        .unwrap();

    let first_env = serde_json::json!({"context": {"id": "local", "kind": "local"}, "schema": 1});
    let reordered_env =
        serde_json::json!({"schema": 1, "context": {"kind": "local", "id": "local"}});
    assert_eq!(
        canonical_json_sha256(&first_env),
        canonical_json_sha256(&reordered_env)
    );
    let env_hash = store
        .record_run_environment_snapshot("run", Some("local"), &first_env)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_run_environment_snapshot("run")
            .await
            .unwrap()
            .unwrap()
            .hash,
        env_hash
    );
    store
        .save_run_code_snapshot(&RunCodeSnapshot {
            id: "code".into(),
            run_id: "run".into(),
            source_kind: "command".into(),
            source_path: None,
            source_text: "python analysis.py".into(),
            checksum: "b".repeat(64),
            storage_path: None,
            git_commit: Some("deadbeef".into()),
            dirty_patch: None,
            created_at: 1,
        })
        .await
        .unwrap();

    let output_key = "figure:t-cells";
    let output_artifact = logical_artifact_id("p", output_key);
    let output_version = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: output_artifact,
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "figure.png".into(),
            content_type: "image/png".into(),
            storage_path: ".wisp/artifacts/sha256/cc/figure.png".into(),
            logical_key: Some(output_key.into()),
            size_bytes: Some(8),
            checksum: Some("c".repeat(64)),
            producing_run_id: Some("run".into()),
            env_snapshot_hash: Some(env_hash),
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    store
        .save_run_output(&RunOutput {
            id: "output".into(),
            run_id: "run".into(),
            artifact_version_id: output_version.clone(),
            role: "figure".into(),
            logical_output_key: output_key.into(),
            source_path: "results/figure.png".into(),
            created_at: 2,
        })
        .await
        .unwrap();
    store
        .save_artifact_dependency(
            "dependency",
            &output_version,
            &input_version,
            Some("counts"),
            LineageBasis::Declared,
            LineageConfidence::Exact,
        )
        .await
        .unwrap();
    assert!(store
        .save_artifact_dependency(
            "cycle",
            &input_version,
            &output_version,
            None,
            LineageBasis::Inferred,
            LineageConfidence::Uncertain,
        )
        .await
        .is_err());

    assert_eq!(store.list_run_inputs("run").await.unwrap().len(), 2);
    assert_eq!(
        store
            .get_external_resource("restricted-cohort")
            .await
            .unwrap()
            .unwrap()
            .visibility,
        "restricted"
    );
    assert_eq!(store.list_run_outputs("run").await.unwrap().len(), 1);
    assert_eq!(
        store
            .get_run_output_version("run", output_key)
            .await
            .unwrap()
            .unwrap()
            .id,
        output_version
    );
    let dependencies = store
        .list_artifact_dependencies(&output_version)
        .await
        .unwrap();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].depends_on_version_id, input_version);
    assert_eq!(dependencies[0].basis, LineageBasis::Declared);
    assert_eq!(dependencies[0].confidence, LineageConfidence::Exact);
    assert_eq!(
        store.list_run_code_snapshots("run").await.unwrap()[0].source_text,
        "python analysis.py"
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn artifact_versions_reject_cross_project_owners() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_artifact_owner_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    for (project, frame) in [("p1", "f1"), ("p2", "f2")] {
        store.create_project(project, project, "").await.unwrap();
        store
            .create_frame(frame, project, "OPERON", "m")
            .await
            .unwrap();
    }
    store
        .create_run(&RunRecord::new("run-p2", "p2", "local", "Other", "command"))
        .await
        .unwrap();
    let draft = ArtifactVersionDraft {
        version_id: None,
        artifact_id: "artifact".into(),
        project_id: "p1".into(),
        root_frame_id: "f2".into(),
        filename: "result.csv".into(),
        content_type: "text/csv".into(),
        storage_path: "result.csv".into(),
        logical_key: Some("path:result.csv".into()),
        size_bytes: Some(1),
        checksum: Some("a".repeat(64)),
        producing_run_id: None,
        env_snapshot_hash: None,
        materialization: ArtifactMaterialization::Snapshot,
        capture_timing: ArtifactCaptureTiming::AtCreation,
    };
    assert!(store.save_artifact_version(&draft).await.is_err());

    let mut draft = draft;
    draft.root_frame_id = "f1".into();
    draft.producing_run_id = Some("run-p2".into());
    assert!(store.save_artifact_version(&draft).await.is_err());
    assert!(store.get_artifact("artifact").await.unwrap().is_none());

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn deleting_a_session_keeps_artifact_versions_owned_by_run_lineage() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_run_retention_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut run = RunRecord::new("run", "p", "local", "Analysis", "command");
    run.frame_id = Some("f".into());
    store.create_run(&run).await.unwrap();
    let version_id = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: "artifact".into(),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "result.csv".into(),
            content_type: "text/csv".into(),
            storage_path: ".wisp/artifacts/sha256/aa/result.csv".into(),
            logical_key: Some("path:results/result.csv".into()),
            size_bytes: Some(4),
            checksum: Some("a".repeat(64)),
            producing_run_id: Some("run".into()),
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    store
        .save_run_output(&RunOutput {
            id: "output".into(),
            run_id: "run".into(),
            artifact_version_id: version_id.clone(),
            role: "table".into(),
            logical_output_key: "path:results/result.csv".into(),
            source_path: "results/result.csv".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .save_run_artifact_link("compat", "run", "artifact", "table")
        .await
        .unwrap();

    store.delete_session("f", "p").await.unwrap();

    assert!(store.get_run("run").await.unwrap().is_some());
    assert!(store
        .get_artifact_version(&version_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store.list_run_outputs("run").await.unwrap()[0].artifact_version_id,
        version_id
    );
    assert!(store.list_sessions("p").await.unwrap().is_empty());

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
async fn turn_undo_keeps_the_first_preimage_and_removes_owned_artifacts() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_turn_undo_store_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .create_frame("other", "p", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("other", 1, &Message::assistant("shared"))
        .await
        .unwrap();
    for (seq, message) in [
        Message::system("system"),
        Message::user("make a summary"),
        Message::assistant("[summary](summary.md)"),
        Message::assistant("[revised summary](summary.md)"),
    ]
    .iter()
    .enumerate()
    {
        store
            .append_message("f", seq as i64 + 1, message)
            .await
            .unwrap();
    }

    store
        .save_turn_file_undo(
            "f",
            2,
            "notes.md",
            true,
            Some(".wisp/undo/first"),
            Some("before"),
            Some("after-1"),
            true,
            None,
        )
        .await
        .unwrap();
    store
        .save_turn_file_undo(
            "f",
            2,
            "notes.md",
            true,
            Some(".wisp/undo/second"),
            Some("middle"),
            Some("after-2"),
            false,
            Some("the later destination was computed dynamically"),
        )
        .await
        .unwrap();
    let changes = store.list_turn_file_undo("f", 2).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].before_snapshot_path.as_deref(),
        Some(".wisp/undo/first")
    );
    assert_eq!(changes[0].before_checksum.as_deref(), Some("before"));
    assert_eq!(changes[0].after_checksum.as_deref(), Some("after-2"));
    assert!(changes[0].reversible);
    assert!(changes[0].reason.is_none());

    let version_id = store
        .save_artifact(
            "artifact-1",
            "p",
            "f",
            "summary.md",
            "text/markdown",
            ".wisp/artifacts/summary.md",
        )
        .await
        .unwrap();
    let revised_version_id = store
        .save_artifact(
            "artifact-1",
            "p",
            "f",
            "summary.md",
            "text/markdown",
            ".wisp/artifacts/summary-v2.md",
        )
        .await
        .unwrap();
    let shared_version_id = store
        .save_artifact(
            "shared-artifact",
            "p",
            "f",
            "shared.md",
            "text/markdown",
            ".wisp/artifacts/shared.md",
        )
        .await
        .unwrap();
    store
        .replace_message_resource_links(
            "f",
            3,
            &[MessageResourceLink {
                id: "link-1".into(),
                frame_id: "f".into(),
                message_seq: 3,
                ordinal: 0,
                original_reference: "summary.md".into(),
                artifact_id: Some("artifact-1".into()),
                artifact_version_id: Some(version_id),
                display_name: "summary.md".into(),
                resource_kind: "markdown".into(),
                mime_type: "text/markdown".into(),
                status: "ready".into(),
                error: None,
                created_artifact: true,
                created_version: true,
                created_at: 1,
            }],
        )
        .await
        .unwrap();
    store
        .replace_message_resource_links(
            "f",
            4,
            &[
                MessageResourceLink {
                    id: "link-2".into(),
                    frame_id: "f".into(),
                    message_seq: 4,
                    ordinal: 0,
                    original_reference: "summary.md".into(),
                    artifact_id: Some("artifact-1".into()),
                    artifact_version_id: Some(revised_version_id),
                    display_name: "summary.md".into(),
                    resource_kind: "markdown".into(),
                    mime_type: "text/markdown".into(),
                    status: "ready".into(),
                    error: None,
                    created_artifact: false,
                    created_version: true,
                    created_at: 2,
                },
                MessageResourceLink {
                    id: "link-shared-owned".into(),
                    frame_id: "f".into(),
                    message_seq: 4,
                    ordinal: 1,
                    original_reference: "shared.md".into(),
                    artifact_id: Some("shared-artifact".into()),
                    artifact_version_id: Some(shared_version_id.clone()),
                    display_name: "shared.md".into(),
                    resource_kind: "markdown".into(),
                    mime_type: "text/markdown".into(),
                    status: "ready".into(),
                    error: None,
                    created_artifact: true,
                    created_version: true,
                    created_at: 2,
                },
            ],
        )
        .await
        .unwrap();
    store
        .replace_message_resource_links(
            "other",
            1,
            &[MessageResourceLink {
                id: "link-shared-external".into(),
                frame_id: "other".into(),
                message_seq: 1,
                ordinal: 0,
                original_reference: "shared.md".into(),
                artifact_id: Some("shared-artifact".into()),
                artifact_version_id: Some(shared_version_id),
                display_name: "shared.md".into(),
                resource_kind: "markdown".into(),
                mime_type: "text/markdown".into(),
                status: "ready".into(),
                error: None,
                created_artifact: false,
                created_version: false,
                created_at: 3,
            }],
        )
        .await
        .unwrap();

    assert_eq!(
        store.list_owned_message_artifacts("f", 1).await.unwrap(),
        vec![("summary.md".into(), "text/markdown".into())]
    );

    store.truncate_messages_for_undo("f", 1).await.unwrap();
    assert_eq!(store.load_messages("f").await.unwrap().len(), 1);
    assert!(store.list_turn_file_undo("f", 2).await.unwrap().is_empty());
    let remaining = store.list_artifacts("f").await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, "shared-artifact");
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn publication_evidence_retains_message_artifacts_during_undo_and_session_delete() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_publication_artifact_retention_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .append_message("f", 1, &Message::user("prepare the figure"))
        .await
        .unwrap();
    store
        .append_message("f", 2, &Message::assistant("[figure](figure.png)"))
        .await
        .unwrap();
    let version_id = store
        .save_artifact(
            "figure-artifact",
            "p",
            "f",
            "figure.png",
            "image/png",
            ".wisp/artifacts/figure.png",
        )
        .await
        .unwrap();
    store
        .replace_message_resource_links(
            "f",
            2,
            &[MessageResourceLink {
                id: "figure-link".into(),
                frame_id: "f".into(),
                message_seq: 2,
                ordinal: 0,
                original_reference: "figure.png".into(),
                artifact_id: Some("figure-artifact".into()),
                artifact_version_id: Some(version_id.clone()),
                display_name: "figure.png".into(),
                resource_kind: "image".into(),
                mime_type: "image/png".into(),
                status: "ready".into(),
                error: None,
                created_artifact: true,
                created_version: true,
                created_at: 1,
            }],
        )
        .await
        .unwrap();
    store
        .create_publication("publication", "p", "Paper", "")
        .await
        .unwrap();
    store
        .create_publication_revision("revision", "publication", None, "Submission")
        .await
        .unwrap();
    store
        .save_publication_item(&PublicationItem {
            id: "figure".into(),
            revision_id: "revision".into(),
            parent_item_id: None,
            kind: PublicationItemKind::Figure,
            title: "Figure 1".into(),
            content: String::new(),
            ordinal: 0,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
    store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: "binding".into(),
            revision_id: "revision".into(),
            item_id: Some("figure".into()),
            source_kind: EvidenceSourceKind::ArtifactVersion,
            source_id: version_id.clone(),
            purpose: "Figure 1".into(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Private,
        })
        .await
        .unwrap();

    assert!(store
        .list_owned_message_artifacts("f", 1)
        .await
        .unwrap()
        .is_empty());
    store.truncate_messages_for_undo("f", 1).await.unwrap();
    assert!(store
        .get_artifact_version(&version_id)
        .await
        .unwrap()
        .is_some());
    assert!(store
        .get_evidence_binding("binding")
        .await
        .unwrap()
        .is_some());

    store.delete_session("f", "p").await.unwrap();
    assert!(store
        .get_artifact_version(&version_id)
        .await
        .unwrap()
        .is_some());
    assert!(store.list_sessions("p").await.unwrap().is_empty());
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn scratch_projects_hidden_from_user_lists() {
    use crate::{is_scratch_project_id, SCRATCH_PROJECT_PREFIX};

    let tmp = std::env::temp_dir().join(format!(
        "wisp_store_scratch_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("real", "Real", "/tmp/real")
        .await
        .unwrap();
    let scratch_id = format!("{SCRATCH_PROJECT_PREFIX}temp");
    store
        .create_project(&scratch_id, "Scratch", "/tmp/scratch")
        .await
        .unwrap();
    assert!(is_scratch_project_id(&scratch_id));

    store
        .create_frame("f-real", "real", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("f-real", 1, &Message::user("hello"))
        .await
        .unwrap();
    store
        .create_frame("f-scratch", &scratch_id, "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("f-scratch", 1, &Message::user("scratch"))
        .await
        .unwrap();

    let projects = store.list_projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].0, "real");

    let recent = store.list_recent_sessions_detail(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].project_id, "real");

    let hits = store
        .search_sessions(None, "hello", 10, None, None)
        .await
        .unwrap();
    assert!(hits.iter().all(|h| h.project_id == "real"));

    let _ = std::fs::remove_file(&tmp);
}

fn exploration_test_artifact(
    artifact_id: &str,
    frame_id: &str,
    logical_key: &str,
    storage_path: &str,
) -> ArtifactVersionDraft {
    ArtifactVersionDraft {
        version_id: None,
        artifact_id: artifact_id.into(),
        project_id: "p".into(),
        root_frame_id: frame_id.into(),
        filename: storage_path.rsplit('/').next().unwrap().into(),
        content_type: "text/plain".into(),
        storage_path: storage_path.into(),
        logical_key: Some(logical_key.into()),
        size_bytes: Some(4),
        checksum: Some("f".repeat(64)),
        producing_run_id: None,
        env_snapshot_hash: None,
        materialization: ArtifactMaterialization::Snapshot,
        capture_timing: ArtifactCaptureTiming::AtCreation,
    }
}

async fn exploration_store_fixture(label: &str) -> (Store, std::path::PathBuf) {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_exploration_{label}_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store
        .create_project("p", "Project", "/tmp/project")
        .await
        .unwrap();
    store
        .create_frame("main", "p", "OPERON", "model")
        .await
        .unwrap();
    store
        .append_message("main", 1, &Message::user("compare approaches"))
        .await
        .unwrap();
    store
        .append_message("main", 2, &Message::assistant("stable checkpoint"))
        .await
        .unwrap();
    store
        .create_frame("branch", "p", "OPERON", "model")
        .await
        .unwrap();
    (store, tmp)
}

async fn create_exploration_checkpoint_fixture(store: &Store) {
    store
        .create_workspace_snapshot(&WorkspaceSnapshotRecord {
            id: "snapshot".into(),
            project_id: "p".into(),
            manifest_json: r#"{"version":1,"files":[]}"#.into(),
            manifest_sha256: "a".repeat(64),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_context_archive(&ContextArchiveRecord {
            id: "archive".into(),
            project_id: "p".into(),
            frame_id: "main".into(),
            storage_path: ".wisp/history/archive.json".into(),
            checksum: "b".repeat(64),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration_family(&ExplorationFamily {
            id: "family".into(),
            project_id: "p".into(),
            root_frame_id: "main".into(),
            mainline_frame_id: "main".into(),
            generation: 0,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration_checkpoint(&ExplorationCheckpoint {
            id: "checkpoint".into(),
            family_id: "family".into(),
            project_id: "p".into(),
            source_frame_id: "main".into(),
            source_message_seq: 2,
            source_frame_head_seq: 2,
            source_ui_event_seq: 0,
            source_family_generation: 0,
            source_state_generation: 0,
            workspace_snapshot_id: "snapshot".into(),
            context_archive_id: "archive".into(),
            guard_hash: "c".repeat(64),
            entity_hash: "d".repeat(64),
            isolation_summary_json: r#"{"level":"full"}"#.into(),
            created_at: 1,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn current_exploration_checkpoint_ignores_settled_rounds() {
    let (store, tmp) = exploration_store_fixture("settled-checkpoint").await;
    create_exploration_checkpoint_fixture(&store).await;

    // Older databases could retain a terminal exploration tombstone. It must not
    // make a later exploration round reuse that round's stale checkpoint.
    let mut connection = store.pool.acquire().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO explorations(\
           id,checkpoint_id,frame_id,name,status,workspace_dir,workspace_backend,\
           scope_generation,warnings_json,created_at,updated_at\
         ) VALUES('settled','checkpoint','branch','Settled','discarded','/tmp/settled',\
                  'copy',0,'[]',2,2)",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);

    assert!(store
        .current_exploration_checkpoint_for_source("p", "main", "family", 0)
        .await
        .unwrap()
        .is_none());

    sqlx::query("DELETE FROM explorations WHERE id='settled'")
        .execute(&store.pool)
        .await
        .unwrap();
    store
        .create_exploration(&Exploration {
            id: "current".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Current round".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/current".into(),
            workspace_backend: "copy".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 3,
            updated_at: 3,
        })
        .await
        .unwrap();

    let current = store
        .current_exploration_checkpoint_for_source("p", "main", "family", 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, "checkpoint");

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_scope_state_machine_and_generations_are_isolated() {
    let (store, tmp) = exploration_store_fixture("scope").await;
    create_exploration_checkpoint_fixture(&store).await;
    store
        .create_exploration(&Exploration {
            id: "explore".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Alternative normalization".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/explorations/explore/workspace".into(),
            workspace_backend: "snapshot".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .exploration_for_frame("branch")
            .await
            .unwrap()
            .unwrap()
            .id,
        "explore"
    );
    assert_eq!(store.list_explorations("main").await.unwrap().len(), 1);
    let summaries = store.list_project_explorations("p").await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].source_frame_id, "main");
    assert_eq!(summaries[0].checkpoint_user_index, 0);
    assert_eq!(summaries[0].isolation_summary_json, r#"{"level":"full"}"#);
    let frame_scope: Option<String> =
        sqlx::query_scalar("SELECT exploration_id FROM frames WHERE id='branch'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(frame_scope.as_deref(), Some("explore"));
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store.mainline_frame_is_frozen("main").await.unwrap());
    assert!(!store.mainline_frame_is_frozen("branch").await.unwrap());
    assert!(!store
        .project_has_current_exploration_for_other_source("p", "main")
        .await
        .unwrap());
    assert!(store
        .project_has_current_exploration_for_other_source("p", "another-mainline")
        .await
        .unwrap());

    let branch_scope = StateScope::exploration("p", "explore");
    let main_scope = StateScope::mainline("p");
    assert_eq!(store.bump_state_generation(&branch_scope).await.unwrap(), 1);
    assert_eq!(store.state_generation(&branch_scope).await.unwrap(), 1);
    assert_eq!(store.project_state_generation("p").await.unwrap(), 0);
    assert_eq!(store.bump_state_generation(&main_scope).await.unwrap(), 1);
    assert_eq!(store.project_state_generation("p").await.unwrap(), 1);

    assert!(store
        .transition_exploration(
            "explore",
            ExplorationStatus::Creating,
            ExplorationStatus::Active,
        )
        .await
        .unwrap());
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store.mainline_frame_is_frozen("main").await.unwrap());
    assert!(store
        .transition_exploration(
            "explore",
            ExplorationStatus::Active,
            ExplorationStatus::Promoting,
        )
        .await
        .unwrap());
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store
        .transition_exploration(
            "explore",
            ExplorationStatus::Promoting,
            ExplorationStatus::Failed,
        )
        .await
        .unwrap());
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store
        .transition_exploration(
            "explore",
            ExplorationStatus::Failed,
            ExplorationStatus::Active,
        )
        .await
        .is_err());
    assert!(store.bump_state_generation(&branch_scope).await.is_err());
    let family = store
        .get_exploration_family("family")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(family.mainline_frame_id, "main");
    assert_eq!(family.generation, 0);

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_promotion_merges_into_original_main_and_discards_candidate() {
    let (store, tmp) = exploration_store_fixture("promotion").await;
    create_exploration_checkpoint_fixture(&store).await;
    store
        .append_message("branch", 1, &Message::user("compare approaches"))
        .await
        .unwrap();
    store
        .append_message("branch", 2, &Message::assistant("stable checkpoint"))
        .await
        .unwrap();
    store
        .create_exploration(&Exploration {
            id: "explore".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Promote me".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/explorations/explore/workspace".into(),
            workspace_backend: "snapshot".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();
    store
        .transition_exploration(
            "explore",
            ExplorationStatus::Creating,
            ExplorationStatus::Active,
        )
        .await
        .unwrap();
    store
        .append_message("branch", 3, &Message::user("try selected approach"))
        .await
        .unwrap();
    store
        .append_message("branch", 4, &Message::assistant("selected result"))
        .await
        .unwrap();
    store
        .append_session_ui_event(
            "branch",
            1,
            r#"{"kind":"User","frame_id":"branch","text":"try selected approach"}"#,
        )
        .await
        .unwrap();
    store
        .create_frame("ordinary-branch", "p", "OPERON", "model")
        .await
        .unwrap();
    store
        .set_session_branch_point("ordinary-branch", "main", 0, "after_response")
        .await
        .unwrap();
    store
        .append_message("ordinary-branch", 1, &Message::user("ordinary branch"))
        .await
        .unwrap();
    let branch_version = store
        .save_artifact_version(&exploration_test_artifact(
            "artifact-branch",
            "branch",
            "path:result.txt",
            "result.txt",
        ))
        .await
        .unwrap();
    store
        .create_exploration_promotion(&ExplorationPromotion {
            id: "promotion".into(),
            exploration_id: "explore".into(),
            expected_guard_hash: "e".repeat(64),
            status: ExplorationPromotionStatus::Prepared,
            diff_json: r#"{"files":[]}"#.into(),
            journal_path: Some("exploration-promotions/promotion/journal.json".into()),
            error: None,
            started_at: 3,
            committed_at: None,
        })
        .await
        .unwrap();
    store
        .transition_exploration(
            "explore",
            ExplorationStatus::Active,
            ExplorationStatus::Promoting,
        )
        .await
        .unwrap();
    store
        .transition_exploration_promotion(
            "promotion",
            ExplorationPromotionStatus::Prepared,
            ExplorationPromotionStatus::FilesApplied,
            None,
        )
        .await
        .unwrap();
    store
        .commit_exploration_promotion_metadata("promotion")
        .await
        .unwrap();

    assert_eq!(
        store.frame_state_scope("main").await.unwrap(),
        Some(StateScope::mainline("p"))
    );
    assert!(store.frame_state_scope("branch").await.unwrap().is_none());
    assert!(store
        .get_exploration_family("family")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .get_artifact_head("p", MAINLINE_SCOPE_KEY, "path:result.txt")
            .await
            .unwrap()
            .unwrap()
            .artifact_version_id,
        branch_version
    );
    assert_eq!(store.get_exploration("explore").await.unwrap(), None);
    assert_eq!(
        store
            .load_messages_with_seq("main")
            .await
            .unwrap()
            .into_iter()
            .map(|(seq, message)| (seq, message.content.as_text()))
            .collect::<Vec<_>>(),
        vec![
            (1, "compare approaches".into()),
            (2, "stable checkpoint".into()),
            (3, "try selected approach".into()),
            (4, "selected result".into()),
        ]
    );
    assert!(store.load_messages("branch").await.unwrap().is_empty());
    let mut sessions = store.list_sessions("p").await.unwrap();
    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].0, "main");
    assert_eq!(sessions[0].1, "compare approaches");
    assert_eq!(sessions[0].4, None);
    assert_eq!(sessions[1].0, "ordinary-branch");
    assert_eq!(sessions[1].1, "ordinary branch");
    assert_eq!(sessions[1].4.as_deref(), Some("main"));
    assert_eq!(
        store
            .list_session_branches("main", "p")
            .await
            .unwrap()
            .into_iter()
            .map(|branch| branch.id)
            .collect::<Vec<_>>(),
        vec!["ordinary-branch".to_string()]
    );
    assert_eq!(
        store
            .get_exploration_promotion("promotion")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExplorationPromotionStatus::MetadataCommitted
    );
    assert_eq!(store.project_state_generation("p").await.unwrap(), 1);

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_promotion_recovery_migration_removes_legacy_cascade() {
    let (store, tmp) = exploration_store_fixture("promotion-recovery-migration").await;
    sqlx::query("DROP INDEX ix_exploration_promotions_exploration")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE exploration_promotions")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE exploration_promotions (\
           id TEXT PRIMARY KEY,\
           exploration_id TEXT NOT NULL REFERENCES explorations(id) ON DELETE CASCADE,\
           expected_guard_hash TEXT NOT NULL,status TEXT NOT NULL,diff_json TEXT NOT NULL,\
           journal_path TEXT,error TEXT,started_at INTEGER NOT NULL,committed_at INTEGER\
         )",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS ix_exploration_promotions_exploration \
         ON exploration_promotions(exploration_id,started_at DESC)",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(EXPLORATION_PROMOTION_RECOVERY_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    let foreign_key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('exploration_promotions')",
    )
    .fetch_one(&repaired.pool)
    .await
    .unwrap();
    assert_eq!(foreign_key_count, 0);

    create_exploration_checkpoint_fixture(&repaired).await;
    repaired
        .create_exploration(&Exploration {
            id: "explore".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Promote after migration".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/explorations/explore/workspace".into(),
            workspace_backend: "snapshot".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();
    repaired
        .transition_exploration(
            "explore",
            ExplorationStatus::Creating,
            ExplorationStatus::Active,
        )
        .await
        .unwrap();
    repaired
        .create_exploration_promotion(&ExplorationPromotion {
            id: "promotion".into(),
            exploration_id: "explore".into(),
            expected_guard_hash: "e".repeat(64),
            status: ExplorationPromotionStatus::Prepared,
            diff_json: r#"{"files":[]}"#.into(),
            journal_path: Some("exploration-promotions/promotion/journal.json".into()),
            error: None,
            started_at: 3,
            committed_at: None,
        })
        .await
        .unwrap();
    repaired
        .transition_exploration(
            "explore",
            ExplorationStatus::Active,
            ExplorationStatus::Promoting,
        )
        .await
        .unwrap();
    repaired
        .transition_exploration_promotion(
            "promotion",
            ExplorationPromotionStatus::Prepared,
            ExplorationPromotionStatus::FilesApplied,
            None,
        )
        .await
        .unwrap();
    repaired
        .commit_exploration_promotion_metadata("promotion")
        .await
        .unwrap();
    assert!(repaired.get_exploration("explore").await.unwrap().is_none());
    assert_eq!(
        repaired
            .get_exploration_promotion("promotion")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExplorationPromotionStatus::MetadataCommitted
    );

    repaired.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn abandoning_exploration_round_discards_every_candidate_and_releases_mainline() {
    let (store, tmp) = exploration_store_fixture("abandon-round").await;
    store.create_project("target", "target", "").await.unwrap();
    create_exploration_checkpoint_fixture(&store).await;
    for (id, frame_id, created_at) in [("first", "branch", 2), ("second", "branch-two", 3)] {
        if frame_id != "branch" {
            store
                .create_frame(frame_id, "p", "OPERON", "model")
                .await
                .unwrap();
        }
        store
            .create_exploration(&Exploration {
                id: id.into(),
                checkpoint_id: "checkpoint".into(),
                frame_id: frame_id.into(),
                name: id.into(),
                status: ExplorationStatus::Creating,
                workspace_dir: format!("/tmp/explorations/{id}/workspace"),
                workspace_backend: "snapshot".into(),
                scope_generation: 0,
                warnings_json: "[]".into(),
                created_at,
                updated_at: created_at,
            })
            .await
            .unwrap();
        store
            .transition_exploration(id, ExplorationStatus::Creating, ExplorationStatus::Active)
            .await
            .unwrap();
    }
    store.discard_exploration_scope("first").await.unwrap();
    assert!(store.get_exploration("first").await.unwrap().is_none());
    assert!(store.frame_project_id("branch").await.unwrap().is_none());
    assert!(store.project_mainline_is_frozen("p").await.unwrap());
    assert_eq!(store.list_project_explorations("p").await.unwrap().len(), 1);
    let delete_error = store.delete_session("main", "p").await.unwrap_err();
    assert!(delete_error
        .to_string()
        .contains("exploration_mainline_frozen"));
    let move_error = store
        .move_session_to_project("main", "p", "target", "moved-main")
        .await
        .unwrap_err();
    assert!(move_error
        .to_string()
        .contains("exploration_mainline_frozen"));

    let candidates = store.abandon_exploration_round("second").await.unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(!store.project_mainline_is_frozen("p").await.unwrap());
    assert!(store
        .list_project_explorations("p")
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .get_exploration_family("family")
        .await
        .unwrap()
        .is_none());
    store.delete_session("main", "p").await.unwrap();
    assert!(store.frame_project_id("main").await.unwrap().is_none());

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn discarding_exploration_purges_all_private_records() {
    let (store, tmp) = exploration_store_fixture("discard-purge").await;
    create_exploration_checkpoint_fixture(&store).await;
    store
        .create_exploration(&Exploration {
            id: "explore".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Discard me".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/explorations/explore/workspace".into(),
            workspace_backend: "snapshot".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();
    store
        .transition_exploration(
            "explore",
            ExplorationStatus::Creating,
            ExplorationStatus::Active,
        )
        .await
        .unwrap();
    store
        .append_message("branch", 1, &Message::user("private analysis"))
        .await
        .unwrap();
    let version_id = store
        .save_artifact_version(&exploration_test_artifact(
            "artifact-private",
            "branch",
            "path:private.txt",
            "private.txt",
        ))
        .await
        .unwrap();
    let mut run = RunRecord::new("run-private", "p", "local", "Private run", "command");
    run.frame_id = Some("branch".into());
    run.status = RunStatus::Succeeded;
    store.create_run(&run).await.unwrap();
    let decision = ResearchNode::new(
        "decision-private",
        "p",
        ResearchNodeKind::Decision,
        "Private decision",
    )
    .unwrap();
    store
        .save_research_node_in_scope(&decision, &StateScope::exploration("p", "explore"))
        .await
        .unwrap();
    store
        .save_external_resource_in_scope(
            &ExternalResource {
                id: "resource-private".into(),
                project_id: "p".into(),
                kind: "dataset".into(),
                uri: "doi:10.0000/private".into(),
                version: Some("v1".into()),
                checksum: Some("e".repeat(64)),
                size_bytes: Some(4),
                license: None,
                visibility: "restricted".into(),
                access_instructions: None,
                accessed_at: Some(1),
                created_at: 1,
                updated_at: 1,
            },
            &StateScope::exploration("p", "explore"),
        )
        .await
        .unwrap();

    store.discard_exploration_scope("explore").await.unwrap();

    assert!(store.get_exploration("explore").await.unwrap().is_none());
    assert!(store.frame_project_id("branch").await.unwrap().is_none());
    assert!(store.load_messages("branch").await.unwrap().is_empty());
    assert!(store
        .get_artifact("artifact-private")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_artifact_version(&version_id)
        .await
        .unwrap()
        .is_none());
    assert!(store.get_run("run-private").await.unwrap().is_none());
    assert!(store
        .research_graph_owned_by_exploration("explore")
        .await
        .unwrap()
        .nodes
        .is_empty());
    assert!(store
        .list_external_resources_owned_by_exploration("explore")
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_exploration_effects("explore")
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_artifact_heads("p", "explore")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.frame_message_head("main").await.unwrap(), 2);

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_artifact_heads_keep_same_logical_key_private() {
    let (store, tmp) = exploration_store_fixture("artifact-head").await;
    create_exploration_checkpoint_fixture(&store).await;
    store
        .create_exploration(&Exploration {
            id: "explore".into(),
            checkpoint_id: "checkpoint".into(),
            frame_id: "branch".into(),
            name: "Alternative".into(),
            status: ExplorationStatus::Creating,
            workspace_dir: "/tmp/explore".into(),
            workspace_backend: "snapshot".into(),
            scope_generation: 0,
            warnings_json: "[]".into(),
            created_at: 2,
            updated_at: 2,
        })
        .await
        .unwrap();

    let main_version = store
        .save_artifact_version(&exploration_test_artifact(
            "artifact-main",
            "main",
            "path:results/table.tsv",
            "results/table.tsv",
        ))
        .await
        .unwrap();
    let branch_version = store
        .save_artifact_version(&exploration_test_artifact(
            "artifact-branch",
            "branch",
            "path:results/table.tsv",
            "/tmp/explore/results/table.tsv",
        ))
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp();
    store
        .upsert_artifact_head(&ArtifactHead {
            project_id: "p".into(),
            scope_key: MAINLINE_SCOPE_KEY.into(),
            logical_key: "path:results/table.tsv".into(),
            artifact_id: "artifact-main".into(),
            artifact_version_id: main_version.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .upsert_artifact_head(&ArtifactHead {
            project_id: "p".into(),
            scope_key: "explore".into(),
            logical_key: "path:results/table.tsv".into(),
            artifact_id: "artifact-branch".into(),
            artifact_version_id: branch_version.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .record_exploration_baseline_artifact_head(&ExplorationBaselineArtifactHead {
            checkpoint_id: "checkpoint".into(),
            logical_key: "path:results/table.tsv".into(),
            artifact_id: "artifact-main".into(),
            artifact_version_id: main_version.clone(),
            fingerprint: "e".repeat(64),
        })
        .await
        .unwrap();
    store
        .record_exploration_baseline_entity(&ExplorationBaselineEntity {
            checkpoint_id: "checkpoint".into(),
            entity_kind: "run".into(),
            entity_id: "run-baseline".into(),
            version_id: None,
            fingerprint: "a".repeat(64),
        })
        .await
        .unwrap();

    let main = store
        .get_artifact_head("p", MAINLINE_SCOPE_KEY, "path:results/table.tsv")
        .await
        .unwrap()
        .unwrap();
    let branch = store
        .get_artifact_head("p", "explore", "path:results/table.tsv")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(main.artifact_version_id, main_version);
    assert_eq!(branch.artifact_version_id, branch_version);
    assert_eq!(
        store
            .list_artifact_heads("p", MAINLINE_SCOPE_KEY)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .list_artifact_heads("p", "explore")
            .await
            .unwrap()
            .len(),
        1
    );

    let raw_same_key: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM artifacts WHERE project_id='p' AND logical_key='path:results/table.tsv'",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(raw_same_key, 2);

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_checkpoint_rejects_stale_mainline_state() {
    let (store, tmp) = exploration_store_fixture("stale-checkpoint").await;
    store
        .create_workspace_snapshot(&WorkspaceSnapshotRecord {
            id: "snapshot".into(),
            project_id: "p".into(),
            manifest_json: "{}".into(),
            manifest_sha256: "a".repeat(64),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_context_archive(&ContextArchiveRecord {
            id: "archive".into(),
            project_id: "p".into(),
            frame_id: "main".into(),
            storage_path: ".wisp/history/archive.json".into(),
            checksum: "b".repeat(64),
            created_at: 1,
        })
        .await
        .unwrap();
    store
        .create_exploration_family(&ExplorationFamily {
            id: "family".into(),
            project_id: "p".into(),
            root_frame_id: "main".into(),
            mainline_frame_id: "main".into(),
            generation: 0,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .bump_state_generation(&StateScope::mainline("p"))
        .await
        .unwrap();
    let stale = ExplorationCheckpoint {
        id: "checkpoint".into(),
        family_id: "family".into(),
        project_id: "p".into(),
        source_frame_id: "main".into(),
        source_message_seq: 2,
        source_frame_head_seq: 2,
        source_ui_event_seq: 0,
        source_family_generation: 0,
        source_state_generation: 0,
        workspace_snapshot_id: "snapshot".into(),
        context_archive_id: "archive".into(),
        guard_hash: "c".repeat(64),
        entity_hash: "d".repeat(64),
        isolation_summary_json: "{}".into(),
        created_at: 1,
    };
    assert!(store.create_exploration_checkpoint(&stale).await.is_err());

    store
        .append_message("main", 3, &Message::user("mainline moved"))
        .await
        .unwrap();
    let mut wrong_head = stale;
    wrong_head.id = "checkpoint-2".into();
    wrong_head.source_state_generation = 1;
    assert!(store
        .create_exploration_checkpoint(&wrong_head)
        .await
        .is_err());

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn project_state_revisions_are_immutable_and_compaction_safe() {
    let (store, tmp) = exploration_store_fixture("state-revisions").await;
    for (snapshot_id, archive_id, checksum) in [
        ("revision-snapshot-1", "revision-archive-1", "a"),
        ("revision-snapshot-2", "revision-archive-2", "b"),
    ] {
        store
            .create_workspace_snapshot(&WorkspaceSnapshotRecord {
                id: snapshot_id.into(),
                project_id: "p".into(),
                manifest_json: "{}".into(),
                manifest_sha256: checksum.repeat(64),
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .create_context_archive(&ContextArchiveRecord {
                id: archive_id.into(),
                project_id: "p".into(),
                frame_id: "main".into(),
                storage_path: format!("exploration-contexts/{archive_id}.json"),
                checksum: checksum.repeat(64),
                created_at: 1,
            })
            .await
            .unwrap();
    }
    let first = ProjectStateRevision {
        id: "revision-1".into(),
        project_id: "p".into(),
        frame_id: "main".into(),
        // The first revision after an upgrade may start after legacy turns.
        turn_index: 5,
        message_seq: 2,
        ui_event_seq: 10,
        parent_revision_id: None,
        workspace_snapshot_id: "revision-snapshot-1".into(),
        workspace_manifest_sha256: "a".repeat(64),
        workspace_delta_json: r#"{"kind":"full","entries":[]}"#.into(),
        artifact_heads_json: "[]".into(),
        entities_json: "[]".into(),
        run_ids_json: "[]".into(),
        decision_ids_json: "[]".into(),
        external_effects_json: "[]".into(),
        context_archive_id: "revision-archive-1".into(),
        state_generation: 0,
        is_full: true,
        created_at: 1,
    };
    assert!(store.create_project_state_revision(&first).await.unwrap());
    assert!(!store.create_project_state_revision(&first).await.unwrap());

    let mut second = first.clone();
    second.id = "revision-2".into();
    second.turn_index = 6;
    // Compaction can reuse a low model message sequence; turn_index remains
    // the unique stable boundary.
    second.message_seq = 2;
    second.ui_event_seq = 12;
    second.parent_revision_id = Some(first.id.clone());
    second.workspace_snapshot_id = "revision-snapshot-2".into();
    second.workspace_manifest_sha256 = "b".repeat(64);
    second.context_archive_id = "revision-archive-2".into();
    second.is_full = false;
    second.created_at = 2;
    assert!(store.create_project_state_revision(&second).await.unwrap());
    assert_eq!(
        store
            .project_state_revision_for_boundary("main", 2, "revision-snapshot-2")
            .await
            .unwrap()
            .unwrap()
            .turn_index,
        6
    );
    assert_eq!(
        store.list_project_state_revisions("main").await.unwrap(),
        vec![first, second]
    );
    assert_eq!(
        store
            .list_project_state_revision_summaries("main", 6, 6)
            .await
            .unwrap(),
        vec![ProjectStateRevisionSummary {
            frame_id: "main".into(),
            turn_index: 6,
        }]
    );
    assert!(store
        .list_project_state_revision_summaries("main", 0, 201)
        .await
        .is_err());

    store.pool.close().await;
    let reopened = Store::open(&tmp).await.unwrap();
    assert_eq!(
        reopened
            .list_project_state_revisions("main")
            .await
            .unwrap()
            .len(),
        2
    );
    reopened.truncate_messages("main", 0).await.unwrap();
    assert!(reopened
        .list_project_state_revisions("main")
        .await
        .unwrap()
        .is_empty());
    reopened.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_migration_repairs_partial_legacy_state() {
    let (store, tmp) = exploration_store_fixture("migration").await;
    let version = store
        .save_artifact_version(&exploration_test_artifact(
            "artifact-main",
            "main",
            "path:result.txt",
            "result.txt",
        ))
        .await
        .unwrap();
    for table in [
        "project_state_revisions",
        "exploration_promotions",
        "exploration_effects",
        "exploration_baseline_artifact_heads",
        "exploration_baseline_entities",
        "explorations",
        "exploration_checkpoints",
        "exploration_families",
        "context_archives",
        "workspace_snapshots",
        "artifact_heads",
        "project_state_counters",
    ] {
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    for table in [
        "frames",
        "artifacts",
        "runs",
        "research_nodes",
        "research_edges",
        "external_resources",
    ] {
        sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN exploration_id"))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    sqlx::query("DROP INDEX IF EXISTS ix_artifacts_project_logical_key")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE UNIQUE INDEX ux_artifacts_project_logical_key \
         ON artifacts(project_id,logical_key) WHERE logical_key IS NOT NULL",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(EXPLORATION_BRANCHES_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
        .bind(PROJECT_STATE_REVISIONS_MIGRATION)
        .execute(&store.pool)
        .await
        .unwrap();
    store.pool.close().await;

    let repaired = Store::open(&tmp).await.unwrap();
    for table in [
        "project_state_revisions",
        "exploration_families",
        "exploration_checkpoints",
        "explorations",
        "artifact_heads",
        "exploration_promotions",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        )
        .bind(table)
        .fetch_one(&repaired.pool)
        .await
        .unwrap();
        assert!(exists, "missing repaired table {table}");
    }
    for table in [
        "frames",
        "artifacts",
        "runs",
        "research_nodes",
        "research_edges",
        "external_resources",
    ] {
        assert!(Store::has_column(&repaired.pool, table, "exploration_id")
            .await
            .unwrap());
    }
    let old_unique: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' \
         AND name='ux_artifacts_project_logical_key')",
    )
    .fetch_one(&repaired.pool)
    .await
    .unwrap();
    assert!(!old_unique);
    let head = repaired
        .get_artifact_head("p", MAINLINE_SCOPE_KEY, "path:result.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.artifact_version_id, version);
    repaired
        .save_artifact_version(&exploration_test_artifact(
            "artifact-second",
            "branch",
            "path:result.txt",
            "/tmp/explore/result.txt",
        ))
        .await
        .unwrap();

    repaired.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn exploration_schema_has_no_retained_discard_state() {
    let (store, tmp) = exploration_store_fixture("no-archive").await;
    let columns = sqlx::query("PRAGMA table_info(explorations)")
        .fetch_all(&store.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String, _>("name").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert!(!columns.contains("archived_at"));
    assert!(!columns.contains("promoted_at"));
    assert!(!columns.contains("discarded_at"));

    create_exploration_checkpoint_fixture(&store).await;
    for status in ["archived", "promoted", "discarded"] {
        let error = sqlx::query(
            "INSERT INTO explorations(\
               id,checkpoint_id,frame_id,name,status,workspace_dir,workspace_backend,\
               scope_generation,warnings_json,created_at,updated_at\
             ) VALUES(?, 'checkpoint','branch','Retired state',?,'/tmp/retired',\
                      'copy',0,'[]',1,1)",
        )
        .bind(status)
        .bind(status)
        .execute(&store.pool)
        .await
        .unwrap_err();
        assert!(error.to_string().contains("CHECK constraint failed"));
    }

    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}
