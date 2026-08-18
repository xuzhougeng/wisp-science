use super::app_commands::parse_ssh_artifact_uri;
use super::app_updates::{update_check_from_release, GithubRelease};
use super::desktop_lifecycle::{should_activate_workspace_window, should_hide_workspace_on_close};
use super::session_commands::transcript_page_items;
use super::{
    begin_queued_cutin, branch_title, client_turn_error, coalesce_live_agent_events,
    copy_dir_recursive, enable_referenced_contexts, events_to_items, limit_persisted_ui_event,
    merge_pending_ui_event, message_uses_resource_bindings, messages_to_items, navigation_allowed,
    parse_disabled_skills, parse_enabled_skill_names, parse_follow_up_questions, parse_skill_tags,
    persist_ui_events, provenance_ui_file_changes, receive_confirm_decision,
    reclaim_unconsumed_cutin, resolve_acp_artifact_references, resolve_composer_references,
    resolve_reader_references, resolve_review_backend, resolve_workspace, session_runtime_status,
    should_hide_app_on_macos_close, should_persist_ui_event, user_message_start, AgentEvent,
    ComposerReferenceArg, McpConnection, McpHttpAuth, McpTransport, ProjectActivityLocks,
    QueuedItem, SessionRuntime, SkillInfo, StartupReport, StartupTimeline,
    MAX_PENDING_UI_EVENT_BYTES, UI_STREAM_OUTPUT_MAX_BYTES, UI_TOOL_RESULT_MAX_CHARS,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

#[tokio::test]
async fn exploration_creation_shares_project_activity_but_serializes_round_initialization() {
    let locks = Arc::new(ProjectActivityLocks::default());
    let running_candidate = locks.project("project").read_owned().await;
    let first_creation = locks.exploration_creation("project").lock_owned().await;

    let waiting_locks = locks.clone();
    let second_creation = tokio::spawn(async move {
        let _activity = waiting_locks
            .project("project")
            .try_read_owned()
            .expect("a sibling exploration may share project activity");
        let _creation = waiting_locks
            .exploration_creation("project")
            .lock_owned()
            .await;
    });
    tokio::task::yield_now().await;
    assert!(
        !second_creation.is_finished(),
        "checkpoint initialization must remain serialized"
    );

    drop(first_creation);
    tokio::time::timeout(std::time::Duration::from_secs(1), second_creation)
        .await
        .expect("the second creation should wait instead of reporting ProjectBusy")
        .unwrap();
    assert!(
        locks.project("project").try_write_owned().is_err(),
        "round settlement must stay exclusive while a candidate is active"
    );
    drop(running_candidate);
    assert!(locks.project("project").try_write_owned().is_ok());
}

#[tokio::test]
async fn native_confirmation_waits_for_an_explicit_response() {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let decision = receive_confirm_decision(receiver);
    tokio::pin!(decision);

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut decision)
            .await
            .is_err(),
        "an unanswered permission request must remain blocked"
    );
    sender.send(wisp_tools::ConfirmDecision::Approved).unwrap();
    assert_eq!(decision.await, wisp_tools::ConfirmDecision::Approved);
}

#[test]
fn client_turn_error_prefixes_only_started_turns() {
    assert_eq!(client_turn_error(false, "no model"), "no model");
    assert_eq!(
        client_turn_error(true, "api: 400 max tokens"),
        "[turn-started] api: 400 max tokens"
    );
}

#[test]
fn resource_conflict_confirmation_has_dedicated_ui_payload_and_no_saved_grant() {
    let message = format!(
        "{}Analysis · abc123 is using `plot.R`. Approve to wait.",
        super::resource_leases::CONFIRM_PREFIX
    );
    let (tool, preview) = super::parse_confirm_payload(&message);
    assert_eq!(tool, super::resource_leases::CONFIRM_TOOL);
    assert!(preview.contains("plot.R"));
    assert!(super::approval_grant_key(&message).is_none());
}

#[test]
fn mcp_app_context_is_latest_only_and_session_scoped() {
    let first = super::normalize_mcp_app_context(
        "Motif for Claude Science",
        serde_json::json!({
            "content": [{"type": "text", "text": "Active record: pET-28a(+)"}],
            "structuredContent": {"recordId": "pet-28a", "length": 5369}
        }),
    )
    .unwrap();
    let runtime = super::SessionRuntime::new();
    let other_runtime = super::SessionRuntime::new();
    runtime.set_mcp_app_context("mcp-app:session-a:motif".into(), first);

    let injection = runtime.mcp_app_context_injection().unwrap();
    assert!(injection.contains("Motif for Claude Science"));
    assert!(injection.contains("Active record: pET-28a(+)"));
    assert!(injection.contains(r#""length":5369"#));
    assert!(other_runtime.mcp_app_context_injection().is_none());

    let replacement = super::normalize_mcp_app_context(
        "Motif for Claude Science",
        serde_json::json!({
            "content": [{"type": "text", "text": "Active record: pBR322"}]
        }),
    )
    .unwrap();
    runtime.set_mcp_app_context("mcp-app:session-a:motif".into(), replacement);
    let injection = runtime.mcp_app_context_injection().unwrap();
    assert!(injection.contains("Active record: pBR322"));
    assert!(!injection.contains("pET-28a"));

    runtime.set_mcp_app_context("mcp-app:session-a:motif".into(), None);
    assert!(runtime.mcp_app_context_injection().is_none());
}

#[test]
fn mcp_app_context_rejects_unsupported_and_oversized_payloads() {
    let unsupported = super::normalize_mcp_app_context(
        "Motif",
        serde_json::json!({
            "content": [{"type": "image", "data": "AA==", "mimeType": "image/png"}]
        }),
    )
    .unwrap_err();
    assert!(unsupported.contains("only text"));

    let oversized = super::normalize_mcp_app_context(
        "Motif",
        serde_json::json!({
            "content": [{
                "type": "text",
                "text": "A".repeat(super::MAX_MCP_APP_CONTEXT_BYTES)
            }]
        }),
    )
    .unwrap_err();
    assert!(oversized.contains("64 KiB"));
}

#[test]
fn mcp_app_instance_id_carries_its_session() {
    assert_eq!(
        super::mcp_app_frame_id("mcp-app:session-a:ui://motif/workbench.html").unwrap(),
        "session-a"
    );
    assert!(super::mcp_app_frame_id("not-an-app").is_err());
}

#[test]
fn image_helper_loads_supported_extension_for_model_input() {
    let root = std::env::temp_dir().join(format!("wisp_message_images_{}", uuid::Uuid::new_v4()));
    let uploads = root.join("uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("plot.PNG"), b"image bytes").unwrap();
    std::fs::write(uploads.join("notes.txt"), b"notes").unwrap();

    // Small images do not need the UI confirmation path; exercise the shared
    // loader directly through its image helper here.
    let result = wisp_tools::image::view_image(&uploads.join("plot.PNG").to_string_lossy());
    let images = vec![result.image.unwrap()];

    assert_eq!(images.len(), 1);
    assert!(images[0].data_url.starts_with("data:image/png;base64,"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn image_resize_confirmation_has_dedicated_card_kind() {
    assert_eq!(
        super::parse_confirm_payload(&format!(
            "{}Resize {}",
            wisp_tools::image::RESIZE_CONFIRM_PREFIX,
            "plot.png"
        )),
        ("image_resize".into(), "Resize plot.png".into())
    );
}

#[test]
fn configured_image_generation_tool_is_available_without_a_specialist() {
    let root = std::env::temp_dir().join(format!("wisp_image_tool_agent_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let skills = Arc::new(wisp_skills::SkillIndex::load(&[]));
    let memory = Arc::new(wisp_core::MemoryManager::new(&root));
    let mut agent = wisp_core::Agent::new(
        wisp_llm::ProviderConfig::openai("http://127.0.0.1:9/v1", "sk-chat-test", "chat-model"),
        skills,
        memory,
        root.clone(),
        128_000,
        4,
        false,
        None,
    );

    super::add_configured_image_generation_tool(
        &mut agent,
        Some((
            "https://api.openai.com/v1".into(),
            "gpt-image-2".into(),
            "sk-image-test".into(),
        )),
        Some("none".into()),
    );

    assert!(agent.tools.get("generate_image").is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_agent_settings_refresh_max_iter_on_reused_agent() {
    // Mid-session Settings changes must not stay stuck at construction time.
    let root = std::env::temp_dir().join(format!("wisp_live_max_iter_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let skills = Arc::new(wisp_skills::SkillIndex::load(&[]));
    let memory = Arc::new(wisp_core::MemoryManager::new(&root));
    let mut agent = wisp_core::Agent::new(
        wisp_llm::ProviderConfig::openai("http://127.0.0.1:9/v1", "sk-chat-test", "chat-model"),
        skills,
        memory,
        root.clone(),
        128_000,
        100,
        false,
        None,
    );
    assert_eq!(agent.max_iter, 100);

    super::apply_live_agent_settings(&mut agent, 0, true, true, 7);
    assert_eq!(agent.max_iter, 0);

    super::apply_live_agent_settings(&mut agent, 50, false, false, 10);
    assert_eq!(agent.max_iter, 50);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn llm_dispatch_debug_detects_cached_model_mismatch() {
    assert!(!super::llm_model_mismatch("glm-5.2", "GLM-5.2"));
    assert!(super::llm_model_mismatch("gpt-5.6-luna", "glm-5.2"));
}

fn reviewer_with_backend(
    backend: Option<crate::review::ReviewBackendConfig>,
) -> crate::specialists::Specialist {
    let mut reviewer = crate::specialists::builtin_reviewer();
    reviewer.review_backend = backend;
    reviewer
}

#[test]
fn reviewer_follow_session_resolves_acp_or_default_http() {
    let reviewer = reviewer_with_backend(Some(crate::review::ReviewBackendConfig::FollowSession));
    assert_eq!(
        resolve_review_backend(&reviewer, Some("acp-codex")),
        Some(crate::review::ReviewBackendConfig::AcpAgent {
            profile_id: "acp-codex".into(),
        })
    );
    assert_eq!(
        resolve_review_backend(&reviewer, None),
        Some(crate::review::ReviewBackendConfig::HttpModel {
            profile_id: String::new(),
        })
    );
}

#[test]
fn reviewer_explicit_backend_does_not_follow_session() {
    let reviewer = reviewer_with_backend(Some(crate::review::ReviewBackendConfig::HttpModel {
        profile_id: "http-reviewer".into(),
    }));
    assert_eq!(
        resolve_review_backend(&reviewer, Some("acp-codex")),
        Some(crate::review::ReviewBackendConfig::HttpModel {
            profile_id: "http-reviewer".into(),
        })
    );
}

#[tokio::test]
async fn auto_review_is_off_by_default_and_persists_changes() {
    let dir = std::env::temp_dir().join(format!("wisp_auto_review_{}", uuid::Uuid::new_v4()));
    let store = wisp_store::Store::open(&dir.join("wisp.sqlite"))
        .await
        .unwrap();

    assert!(!super::load_auto_review_enabled(&store).await);
    super::save_auto_review_enabled(&store, true).await.unwrap();
    assert!(super::load_auto_review_enabled(&store).await);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn update_check_accepts_v_prefixed_newer_release() {
    let result = update_check_from_release(
        "0.9.0",
        GithubRelease {
            tag_name: "v0.10.0".into(),
            html_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.10.0".into(),
            body: "## What's new\n- release notes".into(),
        },
    )
    .unwrap();

    assert!(result.update_available);
    assert_eq!(result.latest_version, "0.10.0");
    assert_eq!(result.notes, "## What's new\n- release notes");
    assert!(!result.install_supported);
}

#[test]
fn update_check_does_not_downgrade() {
    let result = update_check_from_release(
        "1.2.0",
        GithubRelease {
            tag_name: "v1.1.9".into(),
            html_url: "https://example.invalid/release".into(),
            body: String::new(),
        },
    )
    .unwrap();

    assert!(!result.update_available);
}

#[cfg(target_os = "macos")]
#[test]
fn mac_menu_locale_uses_saved_zh_labels() {
    let labels = super::mac_menu_labels(super::AppMenuLocale::from_tag("zh-CN"));
    assert_eq!(labels.help, "帮助");
    assert_eq!(labels.check_updates, "检查更新…");
    assert_eq!(labels.copy, "复制");
    assert_eq!(labels.paste, "粘贴");
    assert_eq!(labels.select_all, "全选");
}

#[cfg(target_os = "macos")]
#[test]
fn mac_menu_locale_includes_english_edit_labels() {
    let labels = super::mac_menu_labels(super::AppMenuLocale::from_tag("en"));
    assert_eq!(labels.undo, "Undo");
    assert_eq!(labels.redo, "Redo");
    assert_eq!(labels.cut, "Cut");
    assert_eq!(labels.copy, "Copy");
    assert_eq!(labels.paste, "Paste");
    assert_eq!(labels.select_all, "Select All");
}

#[cfg(target_os = "macos")]
#[test]
fn mac_menu_action_maps_update_and_settings_ids() {
    assert_eq!(
        super::mac_menu_action("action.check-updates"),
        Some("check-updates")
    );
    assert_eq!(super::mac_menu_action("action.star-us"), Some("star-us"));
    assert_eq!(super::mac_menu_action("action.settings"), Some("settings"));
    assert_eq!(super::mac_menu_action("action.unknown"), None);
}

#[test]
fn reloaded_tool_items_keep_notebook_source() {
    let mut assistant = wisp_llm::Message::assistant("");
    assistant.tool_calls = vec![
        wisp_llm::ToolCall {
            id: "call-python".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "python".into(),
                arguments: r#"{"code":"print(1)"}"#.into(),
            },
        },
        wisp_llm::ToolCall {
            id: "call-r".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "r".into(),
                arguments: r#"{"code":"summary(data)"}"#.into(),
            },
        },
    ];
    let result = wisp_llm::Message::tool("call-python", "python", "1");
    let r_result = wisp_llm::Message::tool("call-r", "r", "summary");

    let items = messages_to_items(&[assistant, result, r_result]);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].tool_name.as_deref(), Some("python"));
    assert_eq!(items[0].input.as_deref(), Some("print(1)"));
    assert_eq!(items[0].text, "1");
    assert_eq!(items[1].tool_name.as_deref(), Some("r"));
    assert_eq!(items[1].input.as_deref(), Some("summary(data)"));
    assert_eq!(items[1].text, "summary");
}

#[test]
fn legacy_tool_replay_is_bounded_but_complete_cards_are_not_truncated() {
    let ordinary = wisp_llm::Message::tool(
        "call-shell",
        "shell",
        "x".repeat(UI_TOOL_RESULT_MAX_CHARS + 500),
    );
    let completion_body = "y".repeat(UI_TOOL_RESULT_MAX_CHARS + 500);
    let completion = wisp_llm::Message::tool(
        "call-completion",
        "attempt_completion",
        completion_body.clone(),
    );

    let items = messages_to_items(&[ordinary, completion]);

    assert_eq!(items[0].text.chars().count(), UI_TOOL_RESULT_MAX_CHARS);
    assert!(items[0].text.ends_with("… output truncated …"));
    assert_eq!(items[1].role, "assistant");
    assert_eq!(items[1].text, completion_body);
}

#[test]
fn reloaded_propose_plan_result_rebuilds_the_plan_card() {
    let plan = wisp_llm::Message::tool(
        "call-plan",
        wisp_tools::plan::PROPOSE_PLAN,
        r#"{"v":1,"source":"native","entries":[{"content":"Read the loader","status":"pending","priority":"high"}]}"#,
    );

    let items = messages_to_items(&[plan]);

    assert_eq!(items.len(), 1);
    // "plan" is what LoadedItem::into_chat turns back into a plan card; a
    // generic tool row here would render the raw JSON instead.
    assert_eq!(items[0].role, "plan");
    assert!(items[0].text.contains("\"source\":\"native\""));
}

#[test]
fn reloaded_ask_user_result_rebuilds_the_question_card() {
    let question = wisp_llm::Message::tool(
        "call-ask",
        wisp_tools::ask_user::ASK_USER,
        r#"{"v":1,"source":"native","question":"Which aligner?","options":[{"label":"STAR","description":""}],"allow_freeform":true}"#,
    );

    let items = messages_to_items(&[question]);

    assert_eq!(items.len(), 1);
    // "question" is what LoadedItem::into_chat turns back into a question
    // card; a generic tool row here would render the raw JSON instead.
    assert_eq!(items[0].role, "question");
    assert!(items[0].text.contains("Which aligner?"));
}

#[test]
fn reloaded_background_completion_keeps_terminal_status() {
    let mut completion = wisp_llm::Message::user(
        r#"{"type":"delegated_batch_completion","result":{"status":"cancelled"}}"#,
    );
    completion.tool_name = Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL.into());

    let items = messages_to_items(&[completion]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].role, "tool");
    assert_eq!(items[0].tool_name.as_deref(), Some("delegate_tasks"));
    assert_eq!(items[0].ok, Some(false));
    assert_eq!(items[0].kind.as_deref(), Some("background_completion"));
}

#[test]
fn ssh_artifact_uri_maps_to_execution_context_and_remote_path() {
    assert_eq!(
        parse_ssh_artifact_uri("ssh://CPU/home/xzg/results.tar.gz"),
        Some(("ssh:CPU".into(), "/home/xzg/results.tar.gz".into()))
    );
    assert_eq!(
        parse_ssh_artifact_uri("ssh://CPU/~/results.tar.gz"),
        Some(("ssh:CPU".into(), "~/results.tar.gz".into()))
    );
    assert_eq!(parse_ssh_artifact_uri("ssh://CPU"), None);
}

#[test]
fn persisted_ui_events_keep_live_step_order_and_boundaries() {
    let frame_id = "f".to_string();
    let events = vec![
        AgentEvent::User {
            frame_id: frame_id.clone(),
            text: "question".into(),
        },
        AgentEvent::MessageBoundary {
            frame_id: frame_id.clone(),
            seq: 1,
        },
        AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: "I will check.".into(),
        },
        AgentEvent::Reasoning {
            frame_id: frame_id.clone(),
            delta: "thinking".into(),
        },
        AgentEvent::ToolCall {
            frame_id: frame_id.clone(),
            name: "shell".into(),
            preview: "pwd".into(),
        },
        AgentEvent::MessageBoundary {
            frame_id: frame_id.clone(),
            seq: 2,
        },
        AgentEvent::ToolResult {
            frame_id: frame_id.clone(),
            name: "shell".into(),
            ok: true,
            content: "/tmp".into(),
            duration_ms: 12,
        },
        AgentEvent::MessageBoundary { frame_id, seq: 3 },
    ];

    let (items, boundaries) = events_to_items(&events);
    assert_eq!(
        items
            .iter()
            .map(|item| item.role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "assistant", "reasoning", "tool"]
    );
    assert_eq!(items[3].text, "/tmp");
    assert_eq!(boundaries.get(&2), Some(&4));
}

#[test]
fn persisted_stdout_replay_folds_progress_and_stays_bounded() {
    let frame_id = "f".to_string();
    let events = vec![
        AgentEvent::ToolCall {
            frame_id: frame_id.clone(),
            name: "shell".into(),
            preview: "run".into(),
        },
        AgentEvent::Stdout {
            frame_id: frame_id.clone(),
            chunk: "10%\r90%\n".into(),
        },
        AgentEvent::Stdout {
            frame_id,
            chunk: "x".repeat(UI_STREAM_OUTPUT_MAX_BYTES + 1_000),
        },
    ];

    let (items, _) = events_to_items(&events);
    assert_eq!(items.len(), 1);
    assert!(items[0].text.len() <= UI_STREAM_OUTPUT_MAX_BYTES);
    assert!(!items[0].text.contains("10%"));
    assert!(items[0].text.ends_with('x'));
}

#[test]
fn persisted_file_changes_restore_structured_artifact_evidence() {
    let event = AgentEvent::FileChanged {
        frame_id: "f".into(),
        path: "results/new.csv".into(),
    };
    assert!(should_persist_ui_event(&event));

    let (items, _) = events_to_items(&[event]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].role, "file_changed");
    assert_eq!(items[0].text, "results/new.csv");
    assert!(items[0].tool_name.is_none());
}

#[test]
fn provenance_for_execution_tools_becomes_file_change_evidence_without_direct_tool_duplicates() {
    let mut record = wisp_core::ProvenanceRecord {
        tool: "python".into(),
        files_written: vec!["results/new.csv".into()],
        ..Default::default()
    };
    assert_eq!(provenance_ui_file_changes(&record), ["results/new.csv"]);

    record.tool = "write".into();
    assert!(provenance_ui_file_changes(&record).is_empty());
}

#[test]
fn persisted_stdout_budget_caps_each_tool_and_resets_at_boundaries() {
    let mut bytes = 0usize;
    let first = limit_persisted_ui_event(
        AgentEvent::Stdout {
            frame_id: "f".into(),
            chunk: "a".repeat(UI_STREAM_OUTPUT_MAX_BYTES - 2),
        },
        &mut bytes,
    )
    .unwrap();
    assert!(matches!(first, AgentEvent::Stdout { .. }));
    let clipped = limit_persisted_ui_event(
        AgentEvent::Stdout {
            frame_id: "f".into(),
            chunk: "界界".into(),
        },
        &mut bytes,
    );
    assert!(
        clipped.is_none(),
        "a partial UTF-8 scalar must not be saved"
    );
    assert_eq!(bytes, UI_STREAM_OUTPUT_MAX_BYTES - 2);
    assert!(limit_persisted_ui_event(
        AgentEvent::ToolResult {
            frame_id: "f".into(),
            name: "shell".into(),
            ok: true,
            content: "done".into(),
            duration_ms: 1,
        },
        &mut bytes,
    )
    .is_some());
    assert_eq!(bytes, 0);
    assert!(limit_persisted_ui_event(
        AgentEvent::Stdout {
            frame_id: "f".into(),
            chunk: "next".into(),
        },
        &mut bytes,
    )
    .is_some());
    assert_eq!(bytes, 4);
}

#[test]
fn persisted_ui_events_restore_native_plan_and_question_cards() {
    let frame_id = "f".to_string();
    let events = vec![
        AgentEvent::ToolCall {
            frame_id: frame_id.clone(),
            name: wisp_tools::plan::PROPOSE_PLAN.into(),
            preview: "1 steps".into(),
        },
        AgentEvent::ToolResult {
            frame_id: frame_id.clone(),
            name: wisp_tools::plan::PROPOSE_PLAN.into(),
            ok: true,
            content: r#"{"v":1,"source":"native","entries":[{"content":"Fix replay","status":"pending","priority":"high"}]}"#.into(),
            duration_ms: 1,
        },
        AgentEvent::ToolCall {
            frame_id: frame_id.clone(),
            name: wisp_tools::ask_user::ASK_USER.into(),
            preview: "Which option?".into(),
        },
        AgentEvent::ToolResult {
            frame_id,
            name: wisp_tools::ask_user::ASK_USER.into(),
            ok: true,
            content: r#"{"v":1,"source":"native","question":"Which option?","options":[]}"#.into(),
            duration_ms: 1,
        },
    ];

    let (items, _) = events_to_items(&events);
    assert_eq!(
        items
            .iter()
            .map(|item| item.role.as_str())
            .collect::<Vec<_>>(),
        vec!["plan", "question"]
    );
    assert!(items[0].text.contains("Fix replay"));
    assert!(items[1].text.contains("Which option?"));
    assert!(items.iter().all(|item| item.tool_name.is_none()));
}

#[test]
fn persisted_usage_folds_per_turn_and_floats_to_tail() {
    let frame_id = "f".to_string();
    let usage = |round, input, output, cached| AgentEvent::Usage {
        frame_id: frame_id.clone(),
        round,
        model: "model".into(),
        created_at: 1,
        input,
        output,
        reasoning: 0,
        cached,
        ctx_tokens: input as usize,
        max_context: 1_000,
        context_usage: wisp_core::ContextUsage {
            conversation: input as usize,
            ..wisp_core::ContextUsage::default()
        },
    };
    let events = vec![
        AgentEvent::User {
            frame_id: frame_id.clone(),
            text: "q1".into(),
        },
        AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: "a1".into(),
        },
        usage(1, 100, 10, 80), // round 1
        usage(2, 200, 20, 0),  // round 2, same turn
        AgentEvent::User {
            frame_id: frame_id.clone(),
            text: "q2".into(),
        },
        AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: "a2".into(),
        },
        usage(1, 50, 5, 0),
    ];

    let (items, _) = events_to_items(&events);
    assert_eq!(
        items
            .iter()
            .map(|item| item.role.as_str())
            .collect::<Vec<_>>(),
        // one usage row per turn, each at its turn's tail
        vec!["user", "assistant", "usage", "user", "assistant", "usage"]
    );
    let first: serde_json::Value = serde_json::from_str(&items[2].text).unwrap();
    assert_eq!(first["input"], 300); // 100 + 200 folded
    assert_eq!(first["output"], 30);
    assert_eq!(first["cached"], 80);
    assert_eq!(first["ctx_tokens"], 200); // latest round snapshot, not a sum
    assert_eq!(first["max_context"], 1_000);
    assert_eq!(first["context_usage"]["conversation"], 200);
    let second: serde_json::Value = serde_json::from_str(&items[5].text).unwrap();
    assert_eq!(second["input"], 50);
}

#[test]
fn persisted_ui_events_ignore_ephemeral_reviewer_handoffs() {
    let frame_id = "f".to_string();
    let events = vec![
        AgentEvent::ReviewStarted {
            frame_id: frame_id.clone(),
        },
        AgentEvent::CorrectionStarted {
            frame_id,
            model: "main-model".into(),
        },
    ];

    let (items, _) = events_to_items(&events);
    assert!(items.is_empty());
}

#[test]
fn persisted_ui_events_restore_context_compaction_flags() {
    let event = AgentEvent::Compaction {
        frame_id: "f".into(),
        before: 812_000,
        after: 236_000,
        strategy: "auto".into(),
    };
    assert!(should_persist_ui_event(&event));
    let events = vec![event];

    let (items, _) = events_to_items(&events);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].role, "compaction");
    let payload: serde_json::Value = serde_json::from_str(&items[0].text).unwrap();
    assert_eq!(payload["before"], 812_000);
    assert_eq!(payload["after"], 236_000);
    assert_eq!(payload["strategy"], "auto");
}

#[test]
fn mcp_app_presentations_are_persisted_for_session_restore() {
    let presentation = AgentEvent::ToolPresentation {
        frame_id: "f".into(),
        presentation_id: "presentation-1".into(),
        presentation_kind: "mcp_app".into(),
        payload: serde_json::json!({"resource": {"uri": "ui://motif/workbench.html"}}),
    };
    assert!(should_persist_ui_event(&presentation));
    assert!(!should_persist_ui_event(&AgentEvent::Diff {
        frame_id: "f".into(),
        path: "temporary.txt".into(),
    }));
}

#[test]
fn terminal_turn_events_are_persisted_for_diagnostics() {
    assert!(should_persist_ui_event(&AgentEvent::Done {
        frame_id: "f".into(),
        stop_reason: None,
        effective_max_iter: None,
    }));
    assert!(should_persist_ui_event(&AgentEvent::Error {
        frame_id: "f".into(),
        message: "api: 524 gateway timeout".into(),
        effective_max_iter: None,
    }));

    let raw = vec![
        serde_json::to_string(&AgentEvent::Text {
            frame_id: "f".into(),
            delta: "partial".into(),
        })
        .unwrap(),
        serde_json::to_string(&AgentEvent::Error {
            frame_id: "f".into(),
            message: "api: 524 gateway timeout".into(),
            effective_max_iter: None,
        })
        .unwrap(),
    ];
    let terminal = super::terminal_ui_events(&raw);
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["kind"], "Error");
    assert_eq!(terminal[0]["message"], "api: 524 gateway timeout");

    let terminal = super::terminal_ui_events(&[serde_json::to_string(&AgentEvent::Done {
        frame_id: "f".into(),
        stop_reason: Some("max_iterations".into()),
        effective_max_iter: Some(20),
    })
    .unwrap()]);
    assert_eq!(terminal[0]["stop_reason"], "max_iterations");
    assert_eq!(terminal[0]["effective_max_iter"], 20);

    let events = vec![AgentEvent::Error {
        frame_id: "f".into(),
        message: "api: 524 gateway timeout".into(),
        effective_max_iter: None,
    }];
    let (items, _) = events_to_items(&events);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].role, "assistant");
    assert_eq!(items[0].text, "Error: api: 524 gateway timeout");
}

#[tokio::test]
async fn busy_agent_invalidation_survives_until_the_next_lock_owner() {
    let runtime = SessionRuntime::new();
    let mut busy = runtime.agent.lock().await;

    runtime.invalidate_cached_agent();
    assert!(runtime.discard_stale_agent(&mut busy));
    assert!(!runtime.discard_stale_agent(&mut busy));

    // A second invalidation after the first was consumed must create a new
    // generation instead of being erased by the earlier lock owner.
    runtime.invalidate_cached_agent();
    assert!(runtime.discard_stale_agent(&mut busy));
}

#[test]
fn pending_ui_event_merge_stays_bounded() {
    let frame_id = "f".to_string();
    let mut pending = Some(AgentEvent::Text {
        frame_id: frame_id.clone(),
        delta: "a".repeat(MAX_PENDING_UI_EVENT_BYTES - 1),
    });
    assert!(merge_pending_ui_event(
        &mut pending,
        AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: "b".into(),
        }
    )
    .is_none());
    let flushed = merge_pending_ui_event(
        &mut pending,
        AgentEvent::Text {
            frame_id,
            delta: "c".into(),
        },
    )
    .unwrap();
    assert!(
        matches!(flushed, AgentEvent::Text { delta, .. } if delta.len() == MAX_PENDING_UI_EVENT_BYTES)
    );
    assert!(matches!(pending, Some(AgentEvent::Text { ref delta, .. }) if delta == "c"));
}

#[tokio::test]
async fn live_agent_events_merge_deltas_and_preserve_order() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let handle = tokio::spawn(coalesce_live_agent_events(
        rx,
        std::time::Duration::from_millis(5),
        move |event| sink.lock().unwrap().push(event),
    ));

    let frame_id = "f".to_string();
    for delta in ["a", "b", "c"] {
        tx.send(AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: delta.into(),
        })
        .unwrap();
    }
    tx.send(AgentEvent::Stdout {
        frame_id: frame_id.clone(),
        chunk: "out".into(),
    })
    .unwrap();
    // A non-delta event must flush pending output before itself, keeping
    // the tool boundary behind the stream it terminates.
    tx.send(AgentEvent::Done {
        frame_id,
        stop_reason: None,
        effective_max_iter: None,
    })
    .unwrap();
    drop(tx);
    handle.await.unwrap();

    let emitted = emitted.lock().unwrap();
    assert_eq!(emitted.len(), 3, "token flood must be coalesced");
    assert!(matches!(&emitted[0], AgentEvent::Text { delta, .. } if delta == "abc"));
    assert!(matches!(&emitted[1], AgentEvent::Stdout { chunk, .. } if chunk == "out"));
    assert!(matches!(&emitted[2], AgentEvent::Done { .. }));
}

#[tokio::test]
async fn ui_events_are_persisted_before_the_turn_ends() {
    let base = std::env::temp_dir().join(format!("wisp_ui_flush_{}", uuid::Uuid::new_v4()));
    let store = wisp_store::Store::open(&base.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "Project", &base.to_string_lossy())
        .await
        .unwrap();
    store
        .create_frame("f", "p", "OPERON", "model")
        .await
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tokio::spawn(persist_ui_events(
        store.clone(),
        "f".into(),
        1,
        rx,
        std::time::Duration::from_millis(5),
    ));
    tx.send(AgentEvent::Text {
        frame_id: "f".into(),
        delta: "still running".into(),
    })
    .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !store.load_session_ui_events("f").await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(!handle.is_finished());
    drop(tx);
    handle.await.unwrap();
    let _ = std::fs::remove_dir_all(base);
}

#[tokio::test]
async fn composer_references_resolve_non_reader_context() {
    let base = std::env::temp_dir().join(format!("wisp_refs_{}", uuid::Uuid::new_v4()));
    let root_a = base.join("alpha");
    let root_b = base.join("beta");
    std::fs::create_dir_all(root_a.join("uploads")).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    std::fs::write(root_a.join("uploads/data.csv"), "x,y\n1,2\n").unwrap();
    let store = wisp_store::Store::open(&base.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("a", "Alpha", &root_a.to_string_lossy())
        .await
        .unwrap();
    store
        .create_project("b", "Beta", &root_b.to_string_lossy())
        .await
        .unwrap();
    store
        .create_frame("target", "a", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("target", 1, &wisp_llm::Message::user("current"))
        .await
        .unwrap();
    store
        .create_frame("source", "b", "OPERON", "m")
        .await
        .unwrap();
    store
        .append_message("source", 1, &wisp_llm::Message::user("prior result"))
        .await
        .unwrap();
    store
        .save_artifact(
            "artifact",
            "a",
            "target",
            "data.csv",
            "text/csv",
            &root_a.join("uploads/data.csv").to_string_lossy(),
        )
        .await
        .unwrap();
    let skill_dir = base.join("skills/test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: test\n---\nUse the test workflow.",
    )
    .unwrap();
    let skills = wisp_skills::SkillIndex::load(&[base.join("skills")]);
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
        .await
        .unwrap();
    let refs = vec![
        ComposerReferenceArg::Artifact {
            id: "artifact".into(),
        },
        ComposerReferenceArg::Session {
            id: "source".into(),
        },
        ComposerReferenceArg::Skill {
            name: "test-skill".into(),
        },
        ComposerReferenceArg::Workflow {
            id: "roundtable".into(),
        },
        ComposerReferenceArg::Context {
            id: "ssh:gpu".into(),
        },
        ComposerReferenceArg::Runtime {
            context_id: "ssh:gpu".into(),
            language: "r".into(),
        },
    ];
    let injected = resolve_composer_references(&store, &refs, "target", &root_a, &skills)
        .await
        .unwrap()
        .join("\n");
    assert!(injected.contains("data.csv"));
    assert!(!injected.contains("prior result"));
    assert!(injected.contains("Use the test workflow"));
    assert!(injected.contains("selected the reusable Workflow “Roundtable”"));
    assert!(injected.contains("\"chair_synthesis\""));
    assert!(injected.contains("GPU (context_id: ssh:gpu, kind: ssh)"));
    assert!(injected.contains("r runtime on GPU (context_id: ssh:gpu)"));
    assert!(resolve_composer_references(
        &store,
        &[ComposerReferenceArg::Context {
            id: "ssh:missing".into()
        }],
        "target",
        &root_a,
        &skills,
    )
    .await
    .is_err());
    let acp_artifacts = resolve_acp_artifact_references(&store, &refs)
        .await
        .unwrap();
    assert_eq!(acp_artifacts.len(), 1);
    assert_eq!(
        acp_artifacts[0].file_name().and_then(|name| name.to_str()),
        Some("data.csv")
    );
    assert!(acp_artifacts[0].is_file());
    let cancel = AtomicBool::new(false);
    assert!(resolve_reader_references(
        &store,
        &[ComposerReferenceArg::Session {
            id: "target".into()
        }],
        "target",
        "question",
        &cancel,
    )
    .await
    .is_err());
    let empty_project = resolve_reader_references(
        &store,
        &[ComposerReferenceArg::Project { id: "a".into() }],
        "target",
        "question",
        &cancel,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(empty_project.contains("No other saved sessions"));
    assert!(resolve_reader_references(
        &store,
        &[ComposerReferenceArg::Project { id: "b".into() }],
        "target",
        "question",
        &cancel,
    )
    .await
    .is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn at_mentioning_a_server_turns_it_on_for_the_session() {
    let base = std::env::temp_dir().join(format!("wisp_ctx_on_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&base).unwrap();
    let store = wisp_store::Store::open(&base.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "P", &base.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:cpu1", "CPU1").unwrap())
        .await
        .unwrap();
    assert!(store
        .list_session_execution_context_ids("f")
        .await
        .unwrap()
        .is_empty());

    // A runtime reference enables the server it lives on, same as naming the
    // server directly. Local needs no toggle, and a stale id must not error.
    enable_referenced_contexts(
        &store,
        &[
            ComposerReferenceArg::Runtime {
                context_id: "ssh:cpu1".into(),
                language: "r".into(),
            },
            ComposerReferenceArg::Context { id: "local".into() },
            ComposerReferenceArg::Context {
                id: "ssh:gone".into(),
            },
        ],
        "f",
    )
    .await;
    assert_eq!(
        store.list_session_execution_context_ids("f").await.unwrap(),
        vec!["ssh:cpu1".to_string()]
    );

    // The prompt's compute section is rendered from that stored set, so the
    // just-enabled server has to appear in it this same turn.
    let compute = super::ssh_hosts::stored_compute_section(&store, "f")
        .await
        .unwrap();
    assert!(compute.contains("ssh:cpu1"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn session_runtime_status_labels() {
    let mut running = HashSet::new();
    running.insert("s1".into());
    let awaiting = HashSet::new();
    assert_eq!(
        session_runtime_status("s1", Some("user"), true, &running, &awaiting),
        "running"
    );
    assert_eq!(
        session_runtime_status("s2", Some("assistant"), true, &running, &awaiting),
        "needs_you"
    );
    assert_eq!(
        session_runtime_status("s4", Some("internal"), true, &running, &awaiting),
        "needs_you"
    );
    assert_eq!(
        session_runtime_status("s3", Some("user"), true, &running, &awaiting),
        "complete"
    );
    // A viewed assistant reply no longer needs you — only unseen ones do.
    assert_eq!(
        session_runtime_status("s2", Some("assistant"), false, &running, &awaiting),
        "complete"
    );
    let mut awaiting = HashSet::new();
    awaiting.insert("s1".into());
    assert_eq!(
        session_runtime_status("s1", Some("user"), true, &running, &awaiting),
        "needs_you"
    );
    // Blocked sessions stay flagged even after being viewed.
    assert_eq!(
        session_runtime_status("s1", Some("user"), false, &running, &awaiting),
        "needs_you"
    );
}

#[test]
fn branch_title_uses_the_draft_without_long_labels() {
    assert_eq!(
        branch_title(Some("  follow up analysis  ")).unwrap(),
        "follow up analysis"
    );
    assert_eq!(branch_title(Some("")).is_none(), true);
    assert!(branch_title(Some(&"a".repeat(80))).unwrap().chars().count() <= 64);
}

#[test]
fn user_message_start_points_at_selected_turn() {
    let mut completion = wisp_llm::Message::user("background completion");
    completion.tool_name = Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL.into());
    let msgs = vec![
        wisp_llm::Message::system("sys"),
        wisp_llm::Message::user("first"),
        wisp_llm::Message::assistant("first answer"),
        wisp_llm::Message::tool("call-1", "python", "ok"),
        completion,
        wisp_llm::Message::user("second"),
        wisp_llm::Message::assistant("second answer"),
    ];
    assert_eq!(user_message_start(&msgs, 0), 1);
    assert_eq!(user_message_start(&msgs, 1), 5);
    assert_eq!(user_message_start(&msgs, 9), msgs.len());
}

#[test]
fn transcript_page_reconstructs_legacy_prefix_before_persisted_events() {
    let events = [
        AgentEvent::Text {
            frame_id: "f".into(),
            delta: "new answer".into(),
        },
        AgentEvent::MessageBoundary {
            frame_id: "f".into(),
            seq: 2,
        },
    ];
    let page = wisp_store::SessionTranscriptPage {
        messages: vec![
            (1, wisp_llm::Message::user("legacy question")),
            (2, wisp_llm::Message::assistant("fallback answer")),
        ],
        branch_merges: vec![],
        reviews: vec![],
        resources: vec![],
        ui_events: events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect(),
        next_before_seq: None,
        user_offset: 0,
        latest_seq: 2,
    };

    let items = transcript_page_items(&page).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].role, "user");
    assert_eq!(items[0].text, "legacy question");
    assert_eq!(items[1].role, "assistant");
    assert_eq!(items[1].text, "new answer");
}

#[test]
fn branch_merge_projection_never_relabels_the_previous_answer() {
    let page = wisp_store::SessionTranscriptPage {
        messages: vec![
            (1, wisp_llm::Message::user("question")),
            (2, wisp_llm::Message::assistant("original answer")),
            (3, wisp_llm::Message::assistant("branch summary")),
        ],
        branch_merges: vec![wisp_store::SessionBranchMergeCard {
            summary_message_seq: 3,
            branch_session_id: "branch".into(),
            branch_title: "focused work".into(),
            checkpoint_user_index: 0,
            checkpoint_kind: "after_response".into(),
            summary: "branch summary".into(),
        }],
        reviews: vec![],
        resources: vec![],
        ui_events: vec![],
        next_before_seq: None,
        user_offset: 0,
        latest_seq: 3,
    };

    let items = transcript_page_items(&page).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].role, "assistant");
    assert_eq!(items[1].text, "original answer");
}

#[test]
fn resource_bindings_cover_messages_rendered_as_assistant_output() {
    assert!(message_uses_resource_bindings(
        &wisp_llm::Message::assistant("answer")
    ));
    let completion = wisp_llm::Message::tool("call-1", "attempt_completion", "result");
    assert!(message_uses_resource_bindings(&completion));
    let ordinary_tool = wisp_llm::Message::tool("call-2", "read_file", "result");
    assert!(!message_uses_resource_bindings(&ordinary_tool));
}

#[test]
fn scope_gates_per_tool_modes() {
    use super::{ApprovalMode, ApprovalPolicy, Scope};
    use std::collections::HashMap;
    use wisp_tools::Approval;

    let policy = |scope: Scope| {
        let mut tools = HashMap::new();
        tools.insert("asker".to_string(), ApprovalMode::Ask);
        tools.insert("blocked".to_string(), ApprovalMode::Deny);
        ApprovalPolicy {
            scope,
            tools,
            ..Default::default()
        }
    };

    // Ask (current behaviour): per-tool modes pass through unchanged.
    let ask = policy(Scope::Ask);
    assert_eq!(ask.mode_for("asker"), Approval::Ask);
    assert_eq!(ask.mode_for("blocked"), Approval::Deny);
    assert_eq!(ask.mode_for("unset"), Approval::Allow);
    assert!(!ask.full());

    // Auto: per-tool Ask is silenced to Allow, but an explicit Deny still
    // blocks and dangerous commands are NOT auto-approved.
    let auto = policy(Scope::Auto);
    assert_eq!(auto.mode_for("asker"), Approval::Allow);
    assert_eq!(auto.mode_for("blocked"), Approval::Deny);
    assert!(!auto.full());

    // Full: everything Allow except an explicit Deny; dangerous commands
    // auto-approve (full() == true).
    let full = policy(Scope::Full);
    assert_eq!(full.mode_for("asker"), Approval::Allow);
    assert_eq!(full.mode_for("blocked"), Approval::Deny);
    assert!(full.full());
}

#[test]
fn approval_grants_respect_scope_and_persistence() {
    use super::{ApprovalGrantKey, ApprovalGrants};

    let key = ApprovalGrantKey {
        kind: "command".into(),
        target: "shell".into(),
    };
    let mut grants = ApprovalGrants::default();
    assert!(!grants.allows("s1", "p1", &key));

    grants.grant("session", "s1", "p1", key.clone());
    assert!(grants.allows("s1", "p2", &key));
    assert!(!grants.allows("s2", "p1", &key));

    grants.grant("project", "s2", "p1", key.clone());
    assert!(grants.allows("s2", "p1", &key));
    assert!(!grants.allows("s2", "p2", &key));

    let persisted = grants.persisted();
    let loaded = ApprovalGrants::from_persisted(persisted);
    assert!(!loaded.allows("s1", "p2", &key));
    assert!(loaded.allows("s3", "p1", &key));

    grants.grant("global", "s3", "p2", key.clone());
    assert!(grants.allows("any", "any", &key));
}

#[test]
fn approval_grant_key_skips_plan_and_normalizes_shell() {
    use super::{approval_grant_key, ApprovalGrantKey};

    assert_eq!(
        approval_grant_key("Dangerous command detected: rm -rf /tmp/x"),
        Some(ApprovalGrantKey {
            kind: "command".into(),
            target: "shell".into(),
        })
    );
    assert_eq!(
        approval_grant_key("Run tool 'python'?"),
        Some(ApprovalGrantKey {
            kind: "tool".into(),
            target: "python".into(),
        })
    );
    assert_eq!(
        approval_grant_key(&format!(
            "{}[ ] Inspect",
            wisp_tools::plan::PLAN_APPROVAL_PREFIX
        )),
        None
    );
}

#[test]
fn copy_dir_recursive_copies_nested_files() {
    let base = std::env::temp_dir().join(format!(
        "wisp_copy_dir_test_{}_{}",
        std::process::id(),
        line!()
    ));
    let from = base.join("from");
    let to = base.join("to");
    std::fs::create_dir_all(from.join("scripts")).unwrap();
    std::fs::write(from.join("SKILL.md"), "---\nname: x\n---\nbody").unwrap();
    std::fs::write(from.join("scripts").join("run.py"), "print(1)").unwrap();

    copy_dir_recursive(&from, &to).unwrap();

    assert!(to.join("SKILL.md").is_file());
    assert!(to.join("scripts").join("run.py").is_file());
    assert_eq!(
        std::fs::read_to_string(to.join("SKILL.md")).unwrap(),
        "---\nname: x\n---\nbody"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn validate_skill_name_rejects_traversal() {
    use super::validate_skill_name;
    for bad in [
        "",
        "  ",
        "..",
        "../../etc",
        "/etc/passwd",
        "a/b",
        "..\\x",
        "foo/../bar",
    ] {
        assert!(validate_skill_name(bad).is_err(), "should reject {bad:?}");
    }
    for ok in ["literature-review", "my-skill", "Skill_1"] {
        assert!(validate_skill_name(ok).is_ok(), "should accept {ok:?}");
    }
}

#[test]
fn parse_disabled_skills_handles_missing_and_valid() {
    assert!(parse_disabled_skills(None).is_empty());
    assert!(parse_disabled_skills(Some("not json")).is_empty());
    let s = parse_disabled_skills(Some(r#"["literature-review","analysis-workflow"]"#));
    assert!(s.contains("literature-review") && s.contains("analysis-workflow") && s.len() == 2);
}

#[test]
fn resolve_workspace_prefers_env_then_setting_then_default() {
    let default = PathBuf::from("/nonexistent/wisp/default");
    // Blank/whitespace candidates are skipped → default wins (never created).
    assert_eq!(
        resolve_workspace(Some("   ".into()), Some(String::new()), default.clone()),
        default
    );
    assert!(!default.exists());

    let base = std::env::temp_dir().join(format!("wisp_ws_test_{}", uuid::Uuid::new_v4()));
    let env_dir = base.join("env");
    let set_dir = base.join("set");
    // A creatable env path wins over the setting, and gets created.
    assert_eq!(
        resolve_workspace(
            Some(env_dir.to_string_lossy().into_owned()),
            Some(set_dir.to_string_lossy().into_owned()),
            default.clone(),
        ),
        env_dir
    );
    assert!(env_dir.exists());
    // Falls through to the setting when env is absent.
    assert_eq!(
        resolve_workspace(None, Some(set_dir.to_string_lossy().into_owned()), default),
        set_dir
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn parse_skill_tags_normalizes_global_tag_json() {
    let tags = parse_skill_tags(Some(
        serde_json::json!({
            "alpha": [" compute ", "protein", "compute", ""],
            "beta": [],
            "gamma": "bad"
        })
        .to_string(),
    ));

    assert_eq!(
        tags.get("alpha").unwrap(),
        &vec!["compute".to_string(), "protein".to_string()]
    );
    assert!(!tags.contains_key("beta"));
    assert!(!tags.contains_key("gamma"));
}

#[test]
fn parse_enabled_skill_names_uses_none_as_all_enabled() {
    assert!(parse_enabled_skill_names(None).is_none());

    let enabled =
        parse_enabled_skill_names(Some(r#"["alpha", " beta ", "", "alpha"]"#.into())).unwrap();
    assert!(enabled.contains("alpha"));
    assert!(enabled.contains("beta"));
    assert_eq!(enabled.len(), 2);

    assert!(parse_enabled_skill_names(Some("not json".into()))
        .unwrap()
        .is_empty());
}

#[test]
fn mcp_connection_serde_roundtrip() {
    let stdio = McpConnection {
        id: "1".into(),
        name: "local".into(),
        enabled: true,
        transport: McpTransport::Stdio {
            command: "python".into(),
            args: vec!["s.py".into()],
            env: vec![("K".into(), "V".into())],
            cwd: None,
        },
    };
    let http = McpConnection {
        id: "2".into(),
        name: "remote".into(),
        enabled: false,
        transport: McpTransport::Http {
            url: "https://x/mcp".into(),
            headers: vec![("Authorization".into(), "Bearer t".into())],
            auth: McpHttpAuth::OAuth,
        },
    };
    for c in [stdio, http] {
        let json = serde_json::to_string(&c).unwrap();
        let back: McpConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }
    // tag shape
    let j = serde_json::to_value(&McpConnection {
        id: "3".into(),
        name: "n".into(),
        enabled: true,
        transport: McpTransport::Http {
            url: "u".into(),
            headers: vec![],
            auth: McpHttpAuth::None,
        },
    })
    .unwrap();
    assert_eq!(j["transport"]["kind"], "http");
    assert_eq!(j["transport"]["auth"], "none");
}

#[test]
fn specialist_prompt_section_appends_identity() {
    let spec = crate::specialists::Specialist {
        id: "sp1".into(),
        name: "Paper hunter".into(),
        icon: String::new(),
        color: String::new(),
        description: "ignored".into(),
        instructions: "You hunt papers.".into(),
        model_id: String::new(),
        review_backend: None,
        skills: None,
        connectors: None,
        builtin: false,
    };
    let s = crate::specialist_prompt_section(&spec);
    assert!(s.starts_with("\n\n## Specialist: Paper hunter\n"));
    assert!(s.contains("You hunt papers."));
    assert!(
        !s.contains("ignored"),
        "description must not enter the prompt"
    );
}

#[test]
fn specialist_section_marker_detects_prior_append() {
    let spec = crate::specialists::Specialist {
        id: "sp1".into(),
        name: "Paper hunter".into(),
        icon: String::new(),
        color: String::new(),
        description: String::new(),
        instructions: "You hunt papers.".into(),
        model_id: String::new(),
        review_backend: None,
        skills: None,
        connectors: None,
        builtin: false,
    };
    let mut prompt = String::from("base prompt");
    let section = crate::specialist_prompt_section(&spec);
    // First append happens; a second pass sees the marker and skips.
    crate::append_specialist_section_once(&mut prompt, &section);
    crate::append_specialist_section_once(&mut prompt, &section);
    assert_eq!(prompt.matches("## Specialist: Paper hunter").count(), 1);
    assert!(prompt.starts_with("base prompt"));
}

#[test]
fn python_bootstrap_success_marks_initialization_complete() {
    let mut status =
        crate::app_commands::initial_bootstrap(std::path::Path::new("/tmp/workspace"), 3);
    assert!(status.python_initializing);
    assert!(!status.python_ok);

    crate::app_commands::finish_python_bootstrap(&mut status, Ok(()));

    assert!(!status.python_initializing);
    assert!(status.python_ok);
}

#[test]
fn python_bootstrap_failure_is_reported_after_initialization() {
    let mut status =
        crate::app_commands::initial_bootstrap(std::path::Path::new("/tmp/workspace"), 3);

    crate::app_commands::finish_python_bootstrap(&mut status, Err("download failed".into()));

    assert!(!status.python_initializing);
    assert!(!status.python_ok);
    assert!(status
        .errors
        .iter()
        .any(|error| error == "Python environment: download failed"));
}

#[test]
fn capability_skill_counts_use_enabled_bundled_vs_project_added_inventory() {
    let skill = |name: &str, scope: &str, enabled: bool| SkillInfo {
        name: name.into(),
        description: String::new(),
        tags: vec![],
        scope: scope.into(),
        enabled,
        builtin: scope == "bundled",
        managed: false,
        managed_by: None,
        dir: format!("/{scope}/{name}"),
    };
    let skills = vec![
        skill("bundled-on", "bundled", true),
        skill("bundled-off", "bundled", false),
        skill("project", "project", true),
        skill("global", "global", true),
        skill("plugin", "plugin", true),
    ];

    let counts = crate::app_commands::capability_skill_counts(&skills);

    assert_eq!(counts.bundled, 1);
    assert_eq!(counts.project, 3);
    assert_eq!(counts.total(), 4);
}

#[test]
fn macos_close_hides_only_main_window_when_not_quitting() {
    assert!(should_hide_app_on_macos_close("main", false));
    assert!(!should_hide_app_on_macos_close("proj-default", false));
    assert!(!should_hide_app_on_macos_close("main", true));
}

#[test]
fn windows_close_to_tray_applies_only_to_the_main_window() {
    assert!(should_hide_workspace_on_close("main"));
    assert!(!should_hide_workspace_on_close("proj-default"));
    assert!(!should_hide_workspace_on_close("pet"));
}

#[test]
fn windows_path_repair_recovers_trailing_user_entries() {
    let inherited = r"C:\Windows\System32;C:\TOOLS\;C:\";
    let user = r"C:\IgnoredWithoutSlash;C:\tools\;C:\Users\Ada\AppData\Local\pixi\bin\;D:\";

    assert_eq!(
        super::repair_windows_path(inherited, user),
        r"C:\Windows\System32;C:\TOOLS;C:\;C:\Users\Ada\AppData\Local\pixi\bin;D:\"
    );
}

#[test]
fn project_window_url_carries_the_target_session() {
    assert_eq!(
        super::project_commands::project_window_url("abc", None),
        "index.html?project=abc"
    );
    assert_eq!(
        super::project_commands::project_window_url("abc", Some("s1")),
        "index.html?project=abc&session=s1"
    );
}

#[test]
fn centered_window_position_centers_over_the_anchor() {
    assert_eq!(
        super::project_commands::centered_window_position((100, 50), (1600, 1000), (1100, 760)),
        (350, 170)
    );
    // A window larger than its anchor overflows symmetrically.
    assert_eq!(
        super::project_commands::centered_window_position((100, 50), (800, 600), (1100, 760)),
        (-50, -30)
    );
    // Anchors on monitors left of/above the primary keep negative origins.
    assert_eq!(
        super::project_commands::centered_window_position((-1600, -50), (1600, 1000), (1100, 760)),
        (-1350, 70)
    );
}

#[tokio::test]
async fn project_workspace_data_deletion_removes_only_the_resolved_project_directory() {
    let root = std::env::temp_dir().join(format!("wisp_project_delete_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("data").join("results.csv"), b"value\n1\n").unwrap();

    let target =
        super::project_commands::project_workspace_delete_target(root.to_string_lossy().as_ref())
            .unwrap()
            .unwrap();
    super::project_commands::delete_project_workspace_data(target)
        .await
        .unwrap();

    assert!(!root.exists());
}

#[test]
fn project_workspace_data_deletion_rejects_filesystem_root() {
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = temp.ancestors().last().unwrap();
    let error =
        super::project_commands::project_workspace_delete_target(root.to_string_lossy().as_ref())
            .unwrap_err();
    assert_eq!(error, "Refusing to delete a filesystem root.");
}

#[test]
fn app_activation_restores_workspace_windows_but_not_the_pet() {
    assert!(should_activate_workspace_window("main"));
    assert!(should_activate_workspace_window("proj-default"));
    assert!(!should_activate_workspace_window("pet"));
}

#[test]
fn window_focus_tracking_survives_unordered_focus_handoff() {
    let assert_reset = || assert!(!super::app_has_focus());
    assert_reset();
    super::record_window_focus("main", true);
    assert!(super::app_has_focus());
    // Focus moves main → project window; gain may arrive before loss.
    super::record_window_focus("proj-a", true);
    super::record_window_focus("main", false);
    assert!(super::app_has_focus());
    // Destroyed window must not pin the app as focused forever.
    super::record_window_focus("proj-a", false);
    assert_reset();
}

// Click-to-open (#434, #499): a notification arms one navigation for its
// window, and native activation/focus consumes the exact payload only once.
#[test]
fn pending_notify_target_fires_once_per_window() {
    let target = serde_json::json!({ "projectId": "project-499", "sessionId": "s1" });
    super::pending_notify_targets()
        .lock()
        .unwrap()
        .insert("proj-434".into(), target.clone());
    assert!(super::claim_notify_activation("proj-434", true));
    assert!(!super::claim_notify_activation("proj-434", true));
    assert!(super::claim_notify_activation("unresolved-project", false));
}

#[test]
fn session_notification_stays_in_its_origin_window_with_same_project_peers() {
    let active_projects = HashMap::from([
        ("main".to_string(), "workspace".to_string()),
        ("proj-workspace".to_string(), "workspace".to_string()),
    ]);
    let active_frames = HashMap::from([
        ("main".to_string(), "session-a".to_string()),
        ("proj-workspace".to_string(), "session-b".to_string()),
    ]);

    assert_eq!(
        super::select_notification_window(
            Some("proj-workspace"),
            "session-b",
            Some("workspace"),
            &active_projects,
            &active_frames,
        )
        .map(|selection| (selection.label, selection.arm_focus_navigation)),
        Some(("proj-workspace".to_string(), true)),
    );
}

#[test]
fn session_notification_falls_back_to_the_window_viewing_the_session() {
    let active_projects = HashMap::from([
        ("main".to_string(), "workspace".to_string()),
        ("proj-workspace".to_string(), "workspace".to_string()),
    ]);
    let active_frames = HashMap::from([
        ("main".to_string(), "session-a".to_string()),
        ("proj-workspace".to_string(), "session-b".to_string()),
    ]);

    assert_eq!(
        super::select_notification_window(
            Some("closed-window"),
            "session-b",
            Some("workspace"),
            &active_projects,
            &active_frames,
        )
        .map(|selection| (selection.label, selection.arm_focus_navigation)),
        Some(("proj-workspace".to_string(), true)),
    );
}

#[test]
fn session_notification_ignores_an_origin_window_that_switched_projects() {
    let active_projects = HashMap::from([
        ("main".to_string(), "project-b".to_string()),
        ("proj-a".to_string(), "project-a".to_string()),
    ]);
    let active_frames = HashMap::from([
        ("main".to_string(), "session-b".to_string()),
        ("proj-a".to_string(), "session-a".to_string()),
    ]);

    assert_eq!(
        super::select_notification_window(
            Some("main"),
            "session-a",
            Some("project-a"),
            &active_projects,
            &active_frames,
        )
        .map(|selection| (selection.label, selection.arm_focus_navigation)),
        Some(("proj-a".to_string(), true)),
    );
}

#[test]
fn foreign_project_notification_fallback_never_arms_focus_navigation() {
    let active_projects = HashMap::from([("main".to_string(), "project-b".to_string())]);
    let active_frames = HashMap::from([("main".to_string(), "session-b".to_string())]);

    assert_eq!(
        super::select_notification_window(
            Some("main"),
            "session-a",
            Some("project-a"),
            &active_projects,
            &active_frames,
        )
        .map(|selection| (selection.label, selection.arm_focus_navigation)),
        Some(("main".to_string(), false)),
    );
}

// Queue (#433): the enqueue/driver protocol must (a) claim exactly one driver,
// (b) drain FIFO, and (c) let a later enqueue re-claim after the queue empties —
// otherwise an item enqueued just as the driver exits would strand with no runner.
#[test]
fn queue_driver_claim_is_single_and_reclaimable() {
    use std::sync::atomic::Ordering;
    let item = |id: u64| QueuedItem {
        id,
        message: format!("m{id}"),
        attachments: vec![],
        references: vec![],
    };
    let rt = SessionRuntime::new();

    // First enqueue claims the driver slot; a concurrent second must not.
    rt.queued.lock().unwrap().push(item(1));
    assert!(
        !rt.draining.swap(true, Ordering::SeqCst),
        "first enqueue claims the driver"
    );
    rt.queued.lock().unwrap().push(item(2));
    assert!(
        rt.draining.swap(true, Ordering::SeqCst),
        "second enqueue sees a driver already running"
    );

    // The driver drains FIFO from the front.
    assert_eq!(rt.queued.lock().unwrap().remove(0).id, 1);
    assert_eq!(rt.queued.lock().unwrap().remove(0).id, 2);

    // Empty → the driver clears the flag under the queued lock and exits.
    {
        let q = rt.queued.lock().unwrap();
        assert!(q.is_empty());
        rt.draining.store(false, Ordering::SeqCst);
    }

    // A later enqueue re-claims the slot rather than stranding.
    rt.queued.lock().unwrap().push(item(3));
    assert!(
        !rt.draining.swap(true, Ordering::SeqCst),
        "post-drain enqueue re-claims the driver"
    );
}

#[test]
fn unconsumed_cutin_returns_to_the_front_of_the_queue() {
    let rt = SessionRuntime::new();
    rt.queued.lock().unwrap().push(QueuedItem {
        id: 7,
        message: "close tabs".into(),
        attachments: vec![],
        references: vec![],
    });

    let (guidance_id, item) = begin_queued_cutin(&rt, 7).unwrap();
    assert!(rt.queued.lock().unwrap().is_empty());
    assert!(reclaim_unconsumed_cutin(&rt, guidance_id, item));
    assert_eq!(rt.queued.lock().unwrap()[0].message, "close tabs");
    assert!(rt.pending_guidance.lock().unwrap().is_empty());
}

#[test]
fn consumed_cutin_is_not_queued_again() {
    let rt = SessionRuntime::new();
    rt.queued.lock().unwrap().push(QueuedItem {
        id: 8,
        message: "use tab.close".into(),
        attachments: vec![],
        references: vec![],
    });

    let (guidance_id, item) = begin_queued_cutin(&rt, 8).unwrap();
    rt.pending_guidance.lock().unwrap().clear();
    assert!(!reclaim_unconsumed_cutin(&rt, guidance_id, item));
    assert!(rt.queued.lock().unwrap().is_empty());
}

// Reorder (#433): move swaps with the neighbour and clamps at both ends, so the
// driver (which drains front-first) runs items in the user's chosen order.
#[test]
fn queue_reorder_swaps_and_clamps() {
    let item = |id: u64| QueuedItem {
        id,
        message: format!("m{id}"),
        attachments: vec![],
        references: vec![],
    };
    let ids = |q: &[QueuedItem]| q.iter().map(|it| it.id).collect::<Vec<_>>();

    let mut q = vec![item(1), item(2), item(3)]; // A, B, C
    super::swap_queued_toward(&mut q, 3, true); // C up → A, C, B
    assert_eq!(ids(&q), [1, 3, 2]);
    super::swap_queued_toward(&mut q, 1, false); // A down → C, A, B
    assert_eq!(ids(&q), [3, 1, 2]);
    super::swap_queued_toward(&mut q, 3, true); // C already first → no-op
    assert_eq!(ids(&q), [3, 1, 2]);
    super::swap_queued_toward(&mut q, 2, false); // B already last → no-op
    assert_eq!(ids(&q), [3, 1, 2]);
    super::swap_queued_toward(&mut q, 99, true); // unknown id → no-op
    assert_eq!(ids(&q), [3, 1, 2]);
}

#[test]
fn follow_up_questions_parse_exactly_three_distinct_options() {
    assert_eq!(
        parse_follow_up_questions("```json\n[\"One?\", \"Two?\", \"Three?\"]\n```").unwrap(),
        ["One?", "Two?", "Three?"]
    );
    assert!(parse_follow_up_questions("[\"Same?\", \"Same?\", \"Third?\"]").is_err());
}

#[test]
fn startup_timeline_returns_phase_results_and_names_the_slowest_phase() {
    let mut timeline = StartupTimeline::default();
    let fast = timeline.record("fast", || 1_u32);
    let slow = timeline.record("slow", || "store");
    assert_eq!(fast, 1);
    assert_eq!(slow, "store");

    // Pin the measured durations instead of racing wall-clock sleeps, so the
    // ordering assertions below are deterministic.
    timeline.phases[0].1 = std::time::Duration::from_millis(1);
    timeline.phases[1].1 = std::time::Duration::from_millis(20);

    let summary = timeline.summary();
    assert!(summary.starts_with("total=21ms"), "{summary}");
    let slow_at = summary.find("slow=20ms").expect("slow phase reported");
    let fast_at = summary.find("fast=1ms").expect("fast phase reported");
    assert!(
        slow_at < fast_at,
        "slowest phase must come first: {summary}"
    );
    assert_eq!(timeline.total(), std::time::Duration::from_millis(21));
}

#[test]
fn startup_timeline_summary_holds_without_any_phase() {
    let timeline = StartupTimeline::default();
    assert_eq!(timeline.summary(), "total=0ms");
    assert_eq!(timeline.total(), std::time::Duration::ZERO);
}

#[test]
fn startup_report_grows_as_the_launch_progresses() {
    let mut report = StartupReport::default();
    assert_eq!(report.summary(), "");

    report.setup = "total=120ms store=90ms".into();
    assert_eq!(report.summary(), "total=120ms store=90ms");

    // A blank window that outlives `setup` by minutes points away from the
    // backend, so the report must keep both numbers side by side.
    report.window_ready_ms = Some(600_000);
    report.deferred_ms = Some(4_200);
    assert_eq!(
        report.summary(),
        "total=120ms store=90ms window_ready=600000ms deferred=4200ms"
    );
}

#[test]
fn navigation_guard_allows_only_app_origins() {
    let allowed = [
        "tauri://localhost/",
        "tauri://localhost/index.html",
        "http://tauri.localhost/",
        "https://tauri.localhost/",
        "http://localhost:1421/",
        "about:blank",
    ];
    for url in allowed {
        assert!(
            navigation_allowed(&tauri::Url::parse(url).unwrap()),
            "{url} should be allowed"
        );
    }

    let blocked = [
        "https://example.com/page",
        "http://evil.example/figures/plot.png",
        // Host-suffix lookalikes must not pass as the dev server.
        "http://localhost.evil.example/",
        "https://not-tauri.localhost/",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ];
    for url in blocked {
        assert!(
            !navigation_allowed(&tauri::Url::parse(url).unwrap()),
            "{url} should be blocked"
        );
    }
}

#[test]
fn desktop_app_icon_is_full_bleed_with_an_inset_mark() {
    let svg = include_str!("../icons/app-icon.svg");
    let rounded = include_str!("../icons/app-icon-rounded.svg");
    let script = include_str!("../gen-icons.ps1");
    assert!(
        !svg.contains("<clipPath"),
        "macOS master must be full-bleed; Dock applies the squircle mask"
    );
    assert!(
        svg.contains("scale(0.60)"),
        "keep the DNA mark inset so Dock/Launchpad does not fill the tile"
    );
    assert!(
        rounded.contains("<clipPath") && rounded.contains("rx=\"58\""),
        "Windows/Linux launchers draw the bitmap as-is and need baked rounding"
    );
    assert!(
        rounded.contains("scale(0.60)"),
        "rounded launcher icon must keep the same inset mark"
    );
    assert!(
        script
            .lines()
            .any(|line| line.contains("Resolve-Path") && line.contains("icons/app-icon.svg")),
        "icon generation must use the desktop master, not the in-app logo"
    );
    assert!(
        script.lines().any(
            |line| line.contains("Resolve-Path") && line.contains("icons/app-icon-rounded.svg")
        ),
        "Windows/Linux icons must come from the rounded master"
    );
    assert!(
        !script.lines().any(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#') && line.contains("ui/logo.svg")
        }),
        "do not pass the in-app logo (canvas-filling badge) to cargo tauri icon"
    );
}
