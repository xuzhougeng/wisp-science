//! Contract tests for the invoke/event boundary: backend payloads must
//! deserialize into the shared UI DTOs in `crates/wisp-dto`. The UI cannot
//! link native crates (wasm32), so its DTOs mirror backend shapes by hand;
//! these tests turn silent serde drift into a compile-time/test failure.
//!
//! Pattern: build the backend value → `serde_json::to_value` (what tauri IPC
//! sends) → deserialize into the `wisp_dto` type → assert field fidelity.

use serde_json::json;

fn roundtrip<T: serde::Serialize, D: serde::de::DeserializeOwned>(value: &T) -> D {
    let json = serde_json::to_value(value).expect("backend value must serialize");
    serde_json::from_value(json).expect("UI DTO must accept the backend payload")
}

#[test]
fn ssh_host_contract() {
    let backend = crate::ssh_hosts::SshHost {
        alias: "gpu".into(),
        host_name: Some("10.0.0.5".into()),
        user: Some("alice".into()),
        port: Some(2222),
        identity_file: None,
        notes: Some("lab box".into()),
        auth_method: Some("password".into()),
        has_password: true,
        password: Some("secret".into()),
    };
    let json = serde_json::to_value(&backend).unwrap();
    // Write-only secret must never cross the boundary.
    assert!(json.get("password").is_none());
    // The UI's password placeholder depends on this flag being serialized.
    assert_eq!(json.get("has_password"), Some(&json!(true)));

    let dto: wisp_dto::SshHost = serde_json::from_value(json).unwrap();
    assert_eq!(dto.alias, "gpu");
    assert_eq!(dto.host_name.as_deref(), Some("10.0.0.5"));
    assert_eq!(dto.user.as_deref(), Some("alice"));
    assert_eq!(dto.port, Some(2222));
    assert_eq!(dto.notes.as_deref(), Some("lab box"));
    assert_eq!(dto.auth_method.as_deref(), Some("password"));
    assert!(dto.has_password);
    assert_eq!(dto.password, None);
}

#[test]
fn model_profile_contract() {
    let backend = crate::models::ModelProfile {
        id: "p1".into(),
        label: "Fast".into(),
        provider: "openai".into(),
        api_url: "https://api.example.com".into(),
        endpoint_suffix: "/gateway".into(),
        model: "gpt-test".into(),
        has_api_key: true,
        active: true,
        max_tokens: 4096,
        context_window: 200_000,
        reasoning_effort: "high".into(),
        service_tier: "priority".into(),
        supports_vision: true,
        use_for_vision: true,
        use_for_image_generation: false,
        image_size: String::new(),
        image_quality: String::new(),
        image_aspect_ratio: String::new(),
        image_resolution: String::new(),
        use_for_video_generation: true,
        video_duration_secs: Some(8),
        video_aspect_ratio: Some("9:16".into()),
        video_resolution: Some("720p".into()),
    };
    let dto: wisp_dto::ModelProfile = roundtrip(&backend);
    assert_eq!(dto.id, "p1");
    assert_eq!(dto.label, "Fast");
    assert_eq!(dto.provider, "openai");
    assert_eq!(dto.api_url, "https://api.example.com");
    assert_eq!(dto.endpoint_suffix, "/gateway");
    assert_eq!(dto.model, "gpt-test");
    assert!(dto.has_api_key);
    assert!(dto.active);
    assert_eq!(dto.max_tokens, 4096);
    assert_eq!(dto.context_window, 200_000);
    assert_eq!(dto.reasoning_effort, "high");
    assert_eq!(dto.service_tier, "priority");
    assert!(dto.supports_vision);
    assert!(dto.use_for_vision);
    assert!(!dto.use_for_image_generation);
    assert!(dto.use_for_video_generation);
    assert_eq!(dto.video_duration_secs, Some(8));
    assert_eq!(dto.video_aspect_ratio.as_deref(), Some("9:16"));
    assert_eq!(dto.video_resolution.as_deref(), Some("720p"));
    assert!(wisp_dto::is_video_generation_model(
        "xai/grok-imagine-video-1.5-preview"
    ));
    assert!(!wisp_dto::is_video_generation_model(
        "grok-imagine-video-2.0"
    ));
    assert_eq!(wisp_dto::VIDEO_ASPECT_RATIOS.len(), 5);
    assert_eq!(wisp_dto::VIDEO_RESOLUTIONS.len(), 3);
    assert_eq!(wisp_dto::VIDEO_DURATION_MIN_SECS, 1);
    assert_eq!(wisp_dto::VIDEO_DURATION_MAX_SECS, 15);
}

#[test]
fn runtime_execution_summary_contract() {
    let backend = crate::runtime_commands::RuntimeExecutionSummary {
        text: "[error] object 'x' not found".into(),
        plots: vec!["aGVsbG8=".into(), "d29ybGQ=".into()],
    };
    let dto: wisp_dto::RuntimeExecutionSummary = roundtrip(&backend);
    assert_eq!(dto.text, "[error] object 'x' not found");
    assert_eq!(
        dto.plots,
        vec!["aGVsbG8=".to_string(), "d29ybGQ=".to_string()]
    );
}

#[test]
fn share_social_copy_contract() {
    let backend = crate::share_social::ShareSocialCopy {
        platform: crate::share_social::ShareSocialPlatform::Xiaohongshu,
        highlights: vec![crate::share_social::ShareSocialHighlight {
            title: "Clean peak".into(),
            why: "The 530 nm assignment is unambiguous.".into(),
            message_indexes: vec![1, 3],
        }],
        variants: vec![crate::share_social::ShareSocialVariant {
            title: "Spectrum note".into(),
            body: "主峰在 530 nm。".into(),
            hashtags: vec!["#RNA".into()],
        }],
    };
    let json = serde_json::to_value(&backend).unwrap();
    assert_eq!(json.get("platform"), Some(&json!("xiaohongshu")));
    assert!(json.get("highlights").is_some());
    assert!(json.get("variants").is_some());
    let dto: wisp_dto::ShareSocialCopy = serde_json::from_value(json).unwrap();
    assert_eq!(dto.platform, wisp_dto::ShareSocialPlatform::Xiaohongshu);
    assert_eq!(dto.highlights[0].title, "Clean peak");
    assert_eq!(dto.highlights[0].message_indexes, vec![1, 3]);
    assert_eq!(dto.variants[0].body, "主峰在 530 nm。");
    assert_eq!(dto.variants[0].hashtags, vec!["#RNA".to_string()]);
}

#[test]
fn execution_context_contract() {
    let backend = wisp_store::ExecutionContext {
        id: "ssh:gpu".into(),
        kind: wisp_store::ExecutionContextKind::Ssh,
        label: "GPU box".into(),
        config_json: "{\"alias\":\"gpu\"}".into(),
        capabilities_json: "{}".into(),
        last_probe_at: Some(1_700_000_000),
        last_probe_status: Some("ok".into()),
        last_probe_error: None,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_001,
    };
    let dto: wisp_dto::ExecutionContext = roundtrip(&backend);
    assert_eq!(dto.id, "ssh:gpu");
    // Backend enum serializes lowercase; the UI matches on these strings.
    assert_eq!(dto.kind, "ssh");
    assert_eq!(dto.label, "GPU box");
    assert_eq!(dto.config_json, "{\"alias\":\"gpu\"}");
    assert_eq!(dto.capabilities_json, "{}");
    assert_eq!(dto.last_probe_status.as_deref(), Some("ok"));
    assert_eq!(dto.last_probe_error, None);
}

#[test]
fn run_summary_contract() {
    let backend = wisp_store::RunSummary {
        id: "run-1".into(),
        frame_id: Some("frame-1".into()),
        context_id: "ssh:gpu".into(),
        title: "align reads".into(),
        kind: "shell".into(),
        status: wisp_store::RunStatus::TimedOut,
        created_at: 1,
        started_at: Some(2),
        ended_at: Some(3),
        exit_code: Some(124),
        remote_workdir: Some("/scratch/run-1".into()),
        timeout_secs: Some(60),
        last_polled_at: Some(4),
        last_poll_error: Some("ssh dropped".into()),
        progress_json: "{}".into(),
        harvested_at: None,
        cleaned_at: None,
        cleanup_error: None,
        output_fingerprint: "abc".into(),
    };
    let dto: wisp_dto::RunSummary = roundtrip(&backend);
    assert_eq!(dto.id, "run-1");
    assert_eq!(dto.frame_id.as_deref(), Some("frame-1"));
    assert_eq!(dto.context_id, "ssh:gpu");
    assert_eq!(dto.title, "align reads");
    assert_eq!(dto.kind, "shell");
    // Backend enum serializes snake_case; the UI matches on these strings.
    assert_eq!(dto.status, "timed_out");
    assert_eq!(dto.exit_code, Some(124));
    assert_eq!(dto.remote_workdir.as_deref(), Some("/scratch/run-1"));
    assert_eq!(dto.last_poll_error.as_deref(), Some("ssh dropped"));
    assert_eq!(dto.output_fingerprint, "abc");
}

#[test]
fn project_transfer_progress_contract() {
    let backend = crate::project_transfer::ProjectTransferProgress {
        direction: "export",
        stage: "copying",
        project_id: Some("proj-1".into()),
        completed_files: 3,
        total_files: Some(10),
        completed_bytes: 1024,
        total_bytes: Some(4096),
        current_path: Some("data/reads.fq".into()),
    };
    let json = serde_json::to_value(&backend).unwrap();
    // Event payload is camelCase on the wire.
    assert!(json.get("projectId").is_some());
    assert!(json.get("completedFiles").is_some());

    let dto: wisp_dto::ProjectTransferProgress = serde_json::from_value(json).unwrap();
    assert_eq!(dto.direction, "export");
    assert_eq!(dto.stage, "copying");
    assert_eq!(dto.project_id.as_deref(), Some("proj-1"));
    assert_eq!(dto.completed_files, 3);
    assert_eq!(dto.total_files, Some(10));
    assert_eq!(dto.completed_bytes, 1024);
    assert_eq!(dto.total_bytes, Some(4096));
    assert_eq!(dto.current_path.as_deref(), Some("data/reads.fq"));
    assert!(!dto.is_complete());
    assert!(!dto.is_failed());
}

#[test]
fn workspace_session_recovery_contract() {
    let preview = wisp_dto::WorkspaceSessionRecoveryPreview {
        workspace_dir: "/study".into(),
        suggested_name: "Study".into(),
        archive_count: 4,
        valid_archive_count: 3,
        recoverable_session_count: 2,
        message_count: 648,
        invalid_archive_count: 1,
        duplicate_archive_count: 1,
        earliest_message_at: Some(10),
        latest_message_at: Some(20),
    };
    let decoded: wisp_dto::WorkspaceSessionRecoveryPreview = roundtrip(&preview);
    assert_eq!(decoded, preview);

    let result = wisp_dto::WorkspaceSessionRecoveryResult {
        project_id: "project".into(),
        project_name: "Study".into(),
        recovered_session_count: 2,
        message_count: 648,
        invalid_archive_count: 1,
        duplicate_archive_count: 1,
    };
    let decoded: wisp_dto::WorkspaceSessionRecoveryResult = roundtrip(&result);
    assert_eq!(decoded, result);
}

#[test]
fn trajectory_snapshot_contract() {
    let backend = crate::trajectory::TrajectorySnapshot {
        frame_id: "frame-1".into(),
        model: Some("gpt-x".into()),
        turns: vec![crate::trajectory::TrajectoryTurn {
            index: 1,
            started_at: Some(1_000_000),
            cells: vec![
                crate::trajectory::TrajectoryCell {
                    kind: "user".into(),
                    summary: "question".into(),
                    detail_input: None,
                    detail_output: Some("full question".into()),
                    ok: None,
                    is_error: false,
                    ts: Some(1_000_000),
                    duration_ms: None,
                    usage: None,
                },
                crate::trajectory::TrajectoryCell {
                    kind: "tool".into(),
                    summary: "read_file {\"path\":\"a.rs\"} → contents".into(),
                    detail_input: Some("{\"path\":\"a.rs\"}".into()),
                    detail_output: Some("contents".into()),
                    ok: Some(true),
                    is_error: false,
                    ts: Some(1_001_000),
                    duration_ms: Some(250),
                    usage: None,
                },
                crate::trajectory::TrajectoryCell {
                    kind: "usage".into(),
                    summary: "round 1 · 100 in / 50 out".into(),
                    detail_input: None,
                    detail_output: None,
                    ok: None,
                    is_error: false,
                    ts: Some(1_004_000),
                    duration_ms: None,
                    usage: Some(crate::trajectory::TrajectoryUsage {
                        round: 1,
                        model: Some("gpt-x".into()),
                        input_tokens: 100,
                        output_tokens: 50,
                        reasoning_tokens: 10,
                        cached_input_tokens: 300,
                    }),
                },
            ],
        }],
        stats: crate::trajectory::TrajectoryStats {
            turns: 1,
            steps: 1,
            llm_ms: 4000,
            tool_ms: 250,
            input_tokens: 100,
            output_tokens: 50,
            cached_input_tokens: 300,
            cache_hit_pct: Some(75.0),
            tokens_per_sec: Some(12.5),
        },
    };
    let json = serde_json::to_value(&backend).unwrap();
    // Command payloads are snake_case on the wire; the UI consumes these keys.
    assert_eq!(json.get("frame_id"), Some(&json!("frame-1")));
    assert!(json["turns"][0].get("started_at").is_some());
    assert!(json["turns"][0]["cells"][1].get("detail_input").is_some());
    assert!(json["turns"][0]["cells"][1].get("duration_ms").is_some());
    assert!(json["turns"][0]["cells"][2]["usage"]
        .get("cached_input_tokens")
        .is_some());
    assert!(json["stats"].get("cache_hit_pct").is_some());
    assert!(json["stats"].get("tokens_per_sec").is_some());

    let dto: wisp_dto::TrajectorySnapshotDto = serde_json::from_value(json).unwrap();
    assert_eq!(dto.frame_id, "frame-1");
    assert_eq!(dto.model.as_deref(), Some("gpt-x"));
    assert_eq!(dto.turns.len(), 1);
    assert_eq!(dto.turns[0].index, 1);
    assert_eq!(dto.turns[0].started_at, Some(1_000_000));
    let cells = &dto.turns[0].cells;
    assert_eq!(cells[0].kind, "user");
    assert_eq!(cells[0].detail_output.as_deref(), Some("full question"));
    assert_eq!(cells[1].kind, "tool");
    assert_eq!(
        cells[1].detail_input.as_deref(),
        Some("{\"path\":\"a.rs\"}")
    );
    assert_eq!(cells[1].ok, Some(true));
    assert!(!cells[1].is_error);
    assert_eq!(cells[1].duration_ms, Some(250));
    let usage = cells[2].usage.as_ref().expect("usage cell carries usage");
    assert_eq!(usage.round, 1);
    assert_eq!(usage.model.as_deref(), Some("gpt-x"));
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 50);
    assert_eq!(usage.reasoning_tokens, 10);
    assert_eq!(usage.cached_input_tokens, 300);
    assert_eq!(dto.stats.turns, 1);
    assert_eq!(dto.stats.steps, 1);
    assert_eq!(dto.stats.llm_ms, 4000);
    assert_eq!(dto.stats.tool_ms, 250);
    assert_eq!(dto.stats.cache_hit_pct, Some(75.0));
    assert_eq!(dto.stats.tokens_per_sec, Some(12.5));
}

#[test]
fn browser_tab_cleanup_prompt_contract() {
    let prompt = wisp_dto::BrowserTabCleanupPrompt {
        turn_id: "turn-1".into(),
        frame_id: "frame-1".into(),
        tabs: vec![wisp_dto::BrowserTabCleanupItem {
            session: "shared".into(),
            tab_id: 42,
            url: "https://example.com/paper".into(),
            title: "Paper".into(),
            initial_url: "https://example.com".into(),
        }],
    };
    let dto: wisp_dto::BrowserTabCleanupPrompt = roundtrip(&prompt);
    assert_eq!(dto.turn_id, "turn-1");
    assert_eq!(dto.frame_id, "frame-1");
    assert_eq!(dto.tabs[0].session, "shared");
    assert_eq!(dto.tabs[0].tab_id, 42);
    assert_eq!(dto.tabs[0].url, "https://example.com/paper");
    assert_eq!(dto.tabs[0].title, "Paper");
    assert_eq!(dto.tabs[0].initial_url, "https://example.com");
}

#[test]
fn mcp_connection_list_contract_redacts_secrets() {
    let backend = crate::McpConnection {
        id: "conn-1".into(),
        name: "remote".into(),
        enabled: true,
        transport: crate::McpTransport::Http {
            url: "https://example.test/mcp".into(),
            headers: vec![wisp_dto::McpSecretEntry::plaintext(
                "Authorization",
                "secret-value",
            )],
            auth: crate::McpHttpAuth::None,
        },
    };
    let json = serde_json::to_value(&backend).expect("backend value must serialize");
    assert!(json["transport"]["headers"][0].get("value").is_none());
    assert!(!json.to_string().contains("secret-value"));
    assert_eq!(json["transport"]["headers"][0]["name"], "Authorization");
    assert_eq!(json["transport"]["headers"][0]["has_value"], true);

    let dto: wisp_dto::ConnRow = serde_json::from_value(json).unwrap();
    match dto.transport {
        wisp_dto::ConnTransport::Http { headers, .. } => {
            assert_eq!(headers[0].name, "Authorization");
            assert!(headers[0].has_value);
            assert_eq!(headers[0].value, None);
        }
        _ => panic!("expected http"),
    }
}

#[test]
fn channels_status_contract_includes_feishu_owner() {
    let backend = crate::channels::ChannelsStatus {
        feishu_enabled: true,
        feishu_bound: true,
        feishu_international: false,
        feishu_app_id: "cli_1".into(),
        feishu_has_secret: true,
        feishu_state: "running".into(),
        feishu_detail: String::new(),
        feishu_owner_open_id: "ou_owner".into(),
        feishu_pending_owner_open_id: "ou_pending".into(),
        weixin_enabled: false,
        weixin_bound: false,
        weixin_state: "stopped".into(),
        weixin_detail: String::new(),
        device: crate::device_bridge::DeviceBridgeSettingsStatus {
            enabled: false,
            mode: crate::device_bridge::DeviceBridgeMode::Lan,
            has_token: false,
            runtime: crate::device_bridge::DeviceBridgeRuntimeStatus::default(),
        },
    };
    let dto: wisp_dto::ChannelsStatus = roundtrip(&backend);
    assert_eq!(dto.feishu_owner_open_id, "ou_owner");
    assert_eq!(dto.feishu_pending_owner_open_id, "ou_pending");
    assert_eq!(dto.feishu_app_id, "cli_1");
}
