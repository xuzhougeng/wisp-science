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
    store.create_agent_workflow(&workflow).await.unwrap();
    assert_eq!(
        store.list_agent_workflows("p").await.unwrap(),
        vec![workflow.clone()]
    );
    workflow.name = "review-v2".into();
    assert!(store.update_agent_workflow(&workflow).await.unwrap());
    let updated_workflow = store.get_agent_workflow("wf").await.unwrap().unwrap();
    assert_eq!(updated_workflow.name, "review-v2");
    assert_eq!(updated_workflow.version, 2);

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
    store.create_agent_workflow_step(&step).await.unwrap();
    assert_eq!(
        store.list_agent_workflow_steps("wf").await.unwrap(),
        vec![step.clone()]
    );

    step.position = 1;
    assert!(store.update_agent_workflow_step(&step).await.unwrap());
    assert_eq!(
        store
            .get_agent_workflow_step("step-1")
            .await
            .unwrap()
            .unwrap()
            .position,
        1
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
async fn agent_workflow_plan_edit_and_approval_are_versioned() {
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
        .create_agent_workflow_plan(&workflow, &[step.clone()])
        .await
        .unwrap();

    workflow.name = "Edited delegated analysis".into();
    workflow.plan_json = r#"{"mode":"manual","max_parallel":1}"#.into();
    workflow.max_parallel = 1;
    assert!(store
        .replace_agent_workflow_plan(&workflow, &[step], 1)
        .await
        .unwrap());
    assert!(!store
        .replace_agent_workflow_plan(&workflow, &[], 1)
        .await
        .unwrap());
    let (edited, steps) = store.get_agent_workflow_plan("wf").await.unwrap().unwrap();
    assert_eq!(edited.version, 2);
    assert_eq!(edited.max_parallel, 1);
    assert_eq!(steps.len(), 1);
    assert!(store.approve_agent_workflow_plan("wf", 2).await.unwrap());
    assert!(!store.approve_agent_workflow_plan("wf", 2).await.unwrap());
    let approved = store.get_agent_workflow("wf").await.unwrap().unwrap();
    assert_eq!(approved.status, AgentWorkflowStatus::Approved);
    assert_eq!(approved.version, 3);
    assert!(approved.approved_at.is_some());
    assert!(store.update_agent_workflow_step(&steps[0]).await.is_err());
    assert!(store.delete_agent_workflow_step("code").await.is_err());
    let mut reverted = approved;
    reverted.status = AgentWorkflowStatus::Draft;
    assert!(store.update_agent_workflow(&reverted).await.is_err());
    assert!(store.delete_agent_workflow("wf").await.unwrap());
    store.pool.close().await;
    let _ = std::fs::remove_file(tmp);
}

#[tokio::test]
async fn legacy_step_mutations_invalidate_the_reviewed_plan_version() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_agent_plan_step_cas_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();

    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegated analysis").unwrap();
    store.create_agent_workflow(&workflow).await.unwrap();
    let mut step =
        AgentWorkflowStep::new("code", "wf", 0, "code", "coder", "acp", "controlled prompt")
            .unwrap();

    store.create_agent_workflow_step(&step).await.unwrap();
    assert!(!store.approve_agent_workflow_plan("wf", 1).await.unwrap());
    assert_eq!(
        store
            .get_agent_workflow("wf")
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );

    step.position = 1;
    assert!(store.update_agent_workflow_step(&step).await.unwrap());
    assert!(!store.approve_agent_workflow_plan("wf", 2).await.unwrap());
    assert_eq!(
        store
            .get_agent_workflow("wf")
            .await
            .unwrap()
            .unwrap()
            .version,
        3
    );

    assert!(store.delete_agent_workflow_step("code").await.unwrap());
    assert!(!store.approve_agent_workflow_plan("wf", 3).await.unwrap());
    assert_eq!(
        store
            .get_agent_workflow("wf")
            .await
            .unwrap()
            .unwrap()
            .version,
        4
    );
    assert!(store.approve_agent_workflow_plan("wf", 4).await.unwrap());

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
    store.create_agent_workflow(&workflow).await.unwrap();
    let step = AgentWorkflowStep::new("code", "wf", 0, "code", "coder", "acp", "controlled prompt")
        .unwrap();
    store.create_agent_workflow_step(&step).await.unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 2).await.unwrap());
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
    store.create_agent_workflow(&workflow).await.unwrap();
    let step = AgentWorkflowStep::new("step", "wf", 0, "step", "coder", "acp", "prompt").unwrap();
    store.create_agent_workflow_step(&step).await.unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 2).await.unwrap());
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
async fn workflow_cancellation_is_persisted_and_cleared_for_retry() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_agent_cancel_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let workflow = AgentWorkflow::new("wf", "p", "workspace", "Delegation").unwrap();
    store.create_agent_workflow(&workflow).await.unwrap();
    let step = AgentWorkflowStep::new("step", "wf", 0, "step", "coder", "acp", "prompt").unwrap();
    store.create_agent_workflow_step(&step).await.unwrap();
    assert!(store.approve_agent_workflow_plan("wf", 2).await.unwrap());
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
async fn session_pages_are_stable_when_timestamps_match() {
    let tmp = std::env::temp_dir().join(format!("wisp_pages_{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    for id in ["a", "b", "c"] {
        store.create_frame(id, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(id, 1, &Message::user(id))
            .await
            .unwrap();
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
    assert_eq!(
        store.load_session_user_messages("f").await.unwrap(),
        vec![
            (1, "one".to_string()),
            (3, "two".to_string()),
            (5, "three".to_string()),
        ]
    );
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
    let found = store.search_sessions(None, "", 10, None).await.unwrap();
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
        store.load_session_reviews("copied").await.unwrap(),
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
        ]
    );

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
    store.create_agent_workflow(&workflow).await.unwrap();
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
    let relocated = store.list_artifact_versions("a").await.unwrap();
    assert_eq!(relocated[0].storage_path, "durable/isolated-report.md");
    assert_eq!(relocated[1].storage_path, "reports/v1.md");

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
