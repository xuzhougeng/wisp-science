use super::{
    branch_title, copy_dir_recursive, events_to_items, messages_to_items, parse_disabled_skills,
    parse_enabled_skill_names, parse_skill_tags, parse_ssh_artifact_uri,
    resolve_acp_artifact_references, resolve_composer_references, resolve_workspace,
    session_runtime_status, should_hide_app_on_macos_close, side_chat_prompt,
    update_check_from_release, user_message_start, AgentEvent, ComposerReferenceArg, GithubRelease,
    McpConnection, McpTransport,
};
use std::collections::HashSet;
use std::path::PathBuf;

#[test]
fn update_check_accepts_v_prefixed_newer_release() {
    let result = update_check_from_release(
        "0.9.0",
        GithubRelease {
            tag_name: "v0.10.0".into(),
            html_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.10.0".into(),
        },
    )
    .unwrap();

    assert!(result.update_available);
    assert_eq!(result.latest_version, "0.10.0");
}

#[test]
fn update_check_does_not_downgrade() {
    let result = update_check_from_release(
        "1.2.0",
        GithubRelease {
            tag_name: "v1.1.9".into(),
            html_url: "https://example.invalid/release".into(),
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
    assistant.tool_calls = vec![wisp_llm::ToolCall {
        id: "call-python".into(),
        kind: "function".into(),
        function: wisp_llm::FunctionCall {
            name: "python".into(),
            arguments: r#"{"code":"print(1)"}"#.into(),
        },
    }];
    let result = wisp_llm::Message::tool("call-python", "python", "1");

    let items = messages_to_items(&[assistant, result]);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].tool_name.as_deref(), Some("python"));
    assert_eq!(items[0].input.as_deref(), Some("print(1)"));
    assert_eq!(items[0].text, "1");
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

#[tokio::test]
async fn composer_references_resolve_artifact_session_and_skill() {
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
    ];
    let injected = resolve_composer_references(&store, &refs, "target", &skills)
        .await
        .unwrap()
        .join("\n");
    assert!(injected.contains("data.csv"));
    assert!(injected.contains("prior result"));
    assert!(injected.contains("Use the test workflow"));
    let acp_artifacts = resolve_acp_artifact_references(&store, &refs)
        .await
        .unwrap();
    assert_eq!(acp_artifacts.len(), 1);
    assert_eq!(
        acp_artifacts[0].file_name().and_then(|name| name.to_str()),
        Some("data.csv")
    );
    assert!(acp_artifacts[0].is_file());
    assert!(resolve_composer_references(
        &store,
        &[ComposerReferenceArg::Session {
            id: "target".into()
        }],
        "target",
        &skills,
    )
    .await
    .is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn session_runtime_status_labels() {
    let mut running = HashSet::new();
    running.insert("s1".into());
    let awaiting = HashSet::new();
    assert_eq!(
        session_runtime_status("s1", Some("user"), &running, &awaiting),
        "running"
    );
    assert_eq!(
        session_runtime_status("s2", Some("assistant"), &running, &awaiting),
        "needs_you"
    );
    assert_eq!(
        session_runtime_status("s3", Some("user"), &running, &awaiting),
        "complete"
    );
    let mut awaiting = HashSet::new();
    awaiting.insert("s1".into());
    assert_eq!(
        session_runtime_status("s1", Some("user"), &running, &awaiting),
        "needs_you"
    );
}

#[test]
fn branch_title_marks_new_session_without_long_labels() {
    assert_eq!(
        branch_title(Some("  follow up analysis  ")).unwrap(),
        "Branch: follow up analysis"
    );
    assert_eq!(branch_title(Some("")).is_none(), true);
    assert!(branch_title(Some(&"a".repeat(80))).unwrap().chars().count() <= "Branch: ".len() + 64);
}

#[test]
fn user_message_start_points_at_selected_turn() {
    let msgs = vec![
        wisp_llm::Message::system("sys"),
        wisp_llm::Message::user("first"),
        wisp_llm::Message::assistant("first answer"),
        wisp_llm::Message::tool("call-1", "python", "ok"),
        wisp_llm::Message::user("second"),
        wisp_llm::Message::assistant("second answer"),
    ];
    assert_eq!(user_message_start(&msgs, 0), 1);
    assert_eq!(user_message_start(&msgs, 1), 4);
    assert_eq!(user_message_start(&msgs, 9), msgs.len());
}

#[test]
fn side_chat_prompt_keeps_context_read_only() {
    let p = side_chat_prompt("[USER]\nhi", "what happened?");
    assert!(p.contains("[USER]\nhi"));
    assert!(p.contains("what happened?"));
    assert!(p.contains("Do not continue the main task"));

    let empty = side_chat_prompt("", "summarize");
    assert!(empty.contains("No saved transcript"));
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
    for ok in ["alphafold2", "my-skill", "Skill_1"] {
        assert!(validate_skill_name(ok).is_ok(), "should accept {ok:?}");
    }
}

#[test]
fn parse_disabled_skills_handles_missing_and_valid() {
    assert!(parse_disabled_skills(None).is_empty());
    assert!(parse_disabled_skills(Some("not json")).is_empty());
    let s = parse_disabled_skills(Some(r#"["alphafold2","boltz"]"#));
    assert!(s.contains("alphafold2") && s.contains("boltz") && s.len() == 2);
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

    let base = std::env::temp_dir().join(format!("wisp_ws_test_{}", std::process::id()));
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
        },
    })
    .unwrap();
    assert_eq!(j["transport"]["kind"], "http");
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
        skills: None,
        connectors: None,
        builtin: false,
    };
    let mut prompt = String::from("base prompt");
    let section = crate::specialist_prompt_section(&spec);
    // First append happens; a second pass sees the marker and skips.
    if !prompt.contains("\n\n## Specialist: ") {
        prompt.push_str(&section);
    }
    if !prompt.contains("\n\n## Specialist: ") {
        prompt.push_str(&section);
    }
    assert_eq!(prompt.matches("## Specialist: Paper hunter").count(), 1);
}

#[test]
fn python_bootstrap_success_marks_initialization_complete() {
    let mut status = crate::initial_bootstrap(std::path::Path::new("/tmp/workspace"), 3);
    assert!(status.python_initializing);
    assert!(!status.python_ok);

    crate::finish_python_bootstrap(&mut status, Ok(()));

    assert!(!status.python_initializing);
    assert!(status.python_ok);
}

#[test]
fn python_bootstrap_failure_is_reported_after_initialization() {
    let mut status = crate::initial_bootstrap(std::path::Path::new("/tmp/workspace"), 3);

    crate::finish_python_bootstrap(&mut status, Err("download failed".into()));

    assert!(!status.python_initializing);
    assert!(!status.python_ok);
    assert!(status
        .errors
        .iter()
        .any(|error| error == "Python environment: download failed"));
}

#[test]
fn macos_close_hides_only_main_window_when_not_quitting() {
    assert!(should_hide_app_on_macos_close("main", false));
    assert!(!should_hide_app_on_macos_close("proj-default", false));
    assert!(!should_hide_app_on_macos_close("main", true));
}
