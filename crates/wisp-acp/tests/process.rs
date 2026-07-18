use std::{
    io::{BufRead, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use wisp_acp::{
    acp::{
        self,
        schema::{
            v1::{
                AgentCapabilities, AuthMethod, AuthMethodAgent, AuthenticateRequest,
                AuthenticateResponse, CancelNotification, CloseSessionRequest,
                CloseSessionResponse, ContentBlock, ContentChunk, InitializeRequest,
                InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest,
                NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
                PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
                ResumeSessionRequest, ResumeSessionResponse, SessionCapabilities,
                SessionCloseCapabilities, SessionConfigOption, SessionConfigOptionValue,
                SessionMode, SessionModeState, SessionNotification, SessionResumeCapabilities,
                SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
                SetSessionModeRequest, SetSessionModeResponse, StopReason, TextContent,
                ToolCallUpdate, ToolCallUpdateFields,
            },
            ProtocolVersion,
        },
        Agent, Client, ConnectionTo, JsonRpcMessage, JsonRpcNotification, UntypedMessage,
    },
    AcpAgentProfile, AcpError, AcpPermissionKind, AcpSessionEvent, AcpSessionHandle, AcpStopReason,
    AcpUpdateKind,
};

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "app-server") {
        return fake_codex_app_server();
    }
    if args.first().is_some_and(|arg| arg == "--fake-agent") {
        return fake_agent(&args[1..]);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    match runtime.block_on(run_tests()) {
        Ok(()) => {
            println!("wisp-acp process tests passed");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wisp-acp process tests failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run_tests() -> Result<(), String> {
    test_environment_propagation().await?;
    test_installed_codex_acp_effective_policy().await?;
    test_full_lifecycle().await?;
    test_protocol_mismatch().await?;
    test_capability_omission().await?;
    test_child_exit().await?;
    test_stderr_bound_and_drop_cleanup().await?;
    Ok(())
}

async fn test_installed_codex_acp_effective_policy() -> Result<(), String> {
    let Some(command) = find_codex_acp() else {
        println!("codex-acp not installed; skipping adapter conformance check");
        return Ok(());
    };
    let capture = unique_temp_path("wisp-codex-acp-policy");
    let profile = wisp_acp::codex_project_sandbox_profile(AcpAgentProfile::new(
        "codex-policy",
        "Codex policy probe",
        command,
        vec![],
    ))
    .with_env(
        "CODEX_PATH",
        std::env::current_exe()
            .map_err(stringify)?
            .to_string_lossy(),
    )
    .with_env(
        "WISP_FAKE_CODEX_CAPTURE",
        capture.to_string_lossy().to_string(),
    );
    let handle = AcpSessionHandle::launch(profile).await.map_err(stringify)?;
    let start = handle
        .new_session(std::env::current_dir().map_err(stringify)?, vec![])
        .await
        .map_err(stringify)?;
    let prompt = handle.prompt_text(start.session_id, "capture effective policy");
    tokio::pin!(prompt);
    let outcome = loop {
        tokio::select! {
            result = &mut prompt => break result.map_err(stringify)?,
            event = handle.next_event() => match event {
                Some(AcpSessionEvent::Permission(request)) => {
                    let reject = request.options.iter()
                        .find(|option| matches!(option.kind, AcpPermissionKind::RejectOnce | AcpPermissionKind::RejectAlways))
                        .ok_or("Codex escalation offered no reject option")?;
                    handle.respond_permission(request.request_id, Some(reject.id.clone())).map_err(stringify)?;
                }
                Some(AcpSessionEvent::Exited { error }) => return Err(error.unwrap_or_else(|| "codex-acp exited".into())),
                Some(_) => {}
                None => return Err("codex-acp event stream closed".into()),
            }
        }
    };
    check(
        outcome.stop_reason == AcpStopReason::EndTurn,
        "Codex policy probe completed",
    )?;
    let raw = std::fs::read_to_string(&capture).map_err(stringify)?;
    let captured: serde_json::Value = serde_json::from_str(&raw).map_err(stringify)?;
    let turn = &captured["turn"];
    check(
        turn["approvalPolicy"] == "on-request",
        "effective Codex approval policy is on-request",
    )?;
    check(
        turn["sandboxPolicy"]["type"] == "workspaceWrite",
        "effective Codex sandbox is workspace-write",
    )?;
    check(
        turn["sandboxPolicy"]["networkAccess"] == false,
        "effective Codex sandbox network is disabled",
    )?;
    check(
        captured["thread"]["config"]["mcp_servers"] == serde_json::json!({}),
        "Codex session MCP config is empty",
    )?;
    check(
        captured["approval"]["decision"] == "decline",
        "Codex command escalation was rejected",
    )?;
    handle.shutdown(Duration::from_secs(1)).await;
    let _ = std::fs::remove_file(capture);
    Ok(())
}

fn find_codex_acp() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WISP_CODEX_ACP_BIN").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let names: &[&str] = if cfg!(windows) {
        // async-process launches binaries directly; npm's `.cmd` shim needs a
        // shell and is intentionally not used by this no-shell conformance test.
        &["codex-acp.exe"]
    } else {
        &["codex-acp"]
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
            .find(|path| path.is_file())
    })
}

async fn test_environment_propagation() -> Result<(), String> {
    let handle = AcpSessionHandle::launch(
        profile("environment", vec![]).with_env("WISP_ACP_TEST_ENV", "controlled-child"),
    )
    .await
    .map_err(stringify)?;
    check(handle.info().protocol_version == 1, "child environment")?;
    handle.shutdown(Duration::from_secs(1)).await;
    Ok(())
}

fn profile(scenario: &str, extra: Vec<String>) -> AcpAgentProfile {
    let mut args = vec!["--fake-agent".to_string(), scenario.to_string()];
    args.extend(extra);
    AcpAgentProfile::new(
        "fake",
        "Fake ACP Agent",
        std::env::current_exe().expect("current test executable"),
        args,
    )
}

async fn test_full_lifecycle() -> Result<(), String> {
    let exact_args = vec![
        "argument with spaces".to_string(),
        "--literal=$HOME".to_string(),
    ];
    let handle = AcpSessionHandle::launch(profile("full", exact_args))
        .await
        .map_err(|error| error.to_string())?;
    check(handle.info().protocol_version == 1, "ACP v1 handshake")?;
    check(handle.is_alive(), "handle alive after launch")?;
    check(
        handle.info().auth_methods.len() == 1,
        "auth method discovery",
    )?;
    let unauthenticated = handle
        .new_session(std::env::current_dir().map_err(stringify)?, vec![])
        .await
        .err()
        .ok_or("unauthenticated session unexpectedly succeeded")?;
    check(
        unauthenticated.to_string().contains("auth required"),
        "auth-required flow",
    )?;
    handle.authenticate("fake-login").await.map_err(stringify)?;

    let start = handle
        .new_session(std::env::current_dir().map_err(stringify)?, vec![])
        .await
        .map_err(stringify)?;
    check(
        start
            .state
            .config_options
            .as_ref()
            .is_some_and(|options| options.len() == 1),
        "initial session config options",
    )?;
    check(
        start
            .state
            .modes
            .as_ref()
            .and_then(|modes| modes.get("availableModes"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|modes| modes.len() == 2),
        "initial session modes",
    )?;
    let session_id = start.session_id;

    let prompt = handle.prompt_text(session_id.clone(), "permissions");
    tokio::pin!(prompt);
    let mut permission_ids = Vec::new();
    let mut saw_before = false;
    while permission_ids.len() < 2 {
        tokio::select! {
            result = &mut prompt => return Err(format!("prompt finished before permissions: {result:?}")),
            event = handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind: AcpUpdateKind::AgentMessage, .. }) => saw_before = true,
                Some(AcpSessionEvent::Permission(request)) => {
                    check(request.options[0].kind == AcpPermissionKind::AllowOnce, "permission kind")?;
                    permission_ids.push(request.request_id.clone());
                    handle.respond_permission(request.request_id, Some(request.options[0].id.clone())).map_err(stringify)?;
                }
                Some(_) => {}
                None => return Err("event stream closed during permissions".into()),
            }
        }
    }
    check(
        permission_ids[0] != permission_ids[1],
        "concurrent permission IDs",
    )?;
    let outcome = prompt.await.map_err(stringify)?;
    check(
        outcome.stop_reason == AcpStopReason::EndTurn,
        "prompt outcome",
    )?;
    check(saw_before, "prompt streaming update")?;
    let tail = tokio::time::timeout(Duration::from_secs(2), handle.next_event())
        .await
        .map_err(stringify)?;
    check(
        matches!(
            tail,
            Some(AcpSessionEvent::Update {
                kind: AcpUpdateKind::AgentMessage,
                ..
            })
        ),
        "known update after ignored future variant",
    )?;

    let changed = handle
        .set_config(
            session_id.clone(),
            "thinking",
            SessionConfigOptionValue::boolean(true),
        )
        .await
        .map_err(stringify)?;
    check(changed.len() == 1, "set config response")?;
    handle
        .set_mode(session_id.clone(), "full-access")
        .await
        .map_err(stringify)?;
    let bad_mode = handle
        .set_mode(session_id.clone(), "nonexistent")
        .await
        .err()
        .ok_or("unknown mode unexpectedly accepted")?;
    check(
        bad_mode.to_string().contains("unknown mode"),
        "set mode rejects unknown id",
    )?;
    handle
        .resume_session(
            session_id.clone(),
            std::env::current_dir().map_err(stringify)?,
            vec![],
        )
        .await
        .map_err(stringify)?;
    handle
        .load_session(
            session_id.clone(),
            std::env::current_dir().map_err(stringify)?,
            vec![],
        )
        .await
        .map_err(stringify)?;

    let cancelled = handle.prompt_text(session_id.clone(), "cancel");
    tokio::pin!(cancelled);
    let permission = loop {
        tokio::select! {
            result = &mut cancelled => return Err(format!("cancel prompt finished early: {result:?}")),
            event = handle.next_event() => if let Some(AcpSessionEvent::Permission(request)) = event { break request },
        }
    };
    handle.cancel(session_id.clone()).map_err(stringify)?;
    let cancelled_outcome = cancelled.await.map_err(stringify)?;
    check(
        cancelled_outcome.stop_reason == AcpStopReason::Cancelled,
        "cancelled prompt outcome",
    )?;
    let tail = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(AcpSessionEvent::Update { payload, .. }) = handle.next_event().await {
                if payload.to_string().contains("cancel-tail") {
                    return true;
                }
            }
        }
    })
    .await
    .map_err(stringify)?;
    check(tail, "cancellation tail update")?;
    check(
        !permission.request_id.is_empty(),
        "cancelled pending permission",
    )?;

    handle.close_session(session_id).await.map_err(stringify)?;
    handle.shutdown(Duration::from_secs(1)).await;
    check(!handle.is_alive(), "handle dead after shutdown")?;
    Ok(())
}

async fn test_protocol_mismatch() -> Result<(), String> {
    let error = AcpSessionHandle::launch(profile("mismatch", vec![]))
        .await
        .err()
        .ok_or("protocol mismatch unexpectedly succeeded")?;
    check(
        matches!(error, AcpError::ProtocolMismatch { actual: 0 }),
        "clear protocol mismatch",
    )
}

async fn test_capability_omission() -> Result<(), String> {
    let handle = AcpSessionHandle::launch(profile("no-caps", vec![]))
        .await
        .map_err(stringify)?;
    handle.authenticate("fake-login").await.map_err(stringify)?;
    let start = handle
        .new_session(std::env::current_dir().map_err(stringify)?, vec![])
        .await
        .map_err(stringify)?;
    let cwd = std::env::current_dir().map_err(stringify)?;
    check(
        matches!(
            handle
                .resume_session(start.session_id.clone(), &cwd, vec![])
                .await,
            Err(AcpError::Unsupported("session/resume"))
        ),
        "omitted resume capability",
    )?;
    check(
        matches!(
            handle
                .load_session(start.session_id.clone(), &cwd, vec![])
                .await,
            Err(AcpError::Unsupported("session/load"))
        ),
        "omitted load capability",
    )?;
    check(
        matches!(
            handle.close_session(start.session_id).await,
            Err(AcpError::Unsupported("session/close"))
        ),
        "omitted close capability",
    )?;
    handle.shutdown(Duration::from_secs(1)).await;
    Ok(())
}

async fn test_child_exit() -> Result<(), String> {
    let error = AcpSessionHandle::launch(profile("exit", vec![]))
        .await
        .err()
        .ok_or("exiting child unexpectedly initialized")?;
    check(
        error.to_string().contains("fake early exit"),
        "child stderr surfaced",
    )
}

async fn test_stderr_bound_and_drop_cleanup() -> Result<(), String> {
    let marker = unique_temp_path("wisp-acp-cleanup");
    let handle = AcpSessionHandle::launch_with_stderr_limit(
        profile("cleanup", vec![marker.to_string_lossy().to_string()]),
        128,
    )
    .await
    .map_err(stringify)?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    check(handle.stderr().len() <= 128, "stderr is bounded")?;
    drop(handle);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let first = std::fs::read(&marker).map_err(stringify)?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let second = std::fs::read(&marker).map_err(stringify)?;
    let _ = std::fs::remove_file(marker);
    check(first == second, "dropping handle stops child process")
}

fn unique_temp_path(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
}

fn check(condition: bool, message: &str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| message.to_string())
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn fake_agent(args: &[String]) -> ExitCode {
    let Some(scenario) = args.first().map(String::as_str) else {
        return ExitCode::FAILURE;
    };
    match scenario {
        "exit" => {
            eprintln!("fake early exit");
            return ExitCode::from(17);
        }
        "full"
            if args.get(1).map(String::as_str) != Some("argument with spaces")
                || args.get(2).map(String::as_str) != Some("--literal=$HOME") =>
        {
            eprintln!("argument boundaries changed: {args:?}");
            return ExitCode::FAILURE;
        }
        "cleanup" => {
            eprintln!("{}", "x".repeat(2048));
            let marker = PathBuf::from(args.get(1).expect("cleanup marker"));
            std::thread::spawn(move || {
                let mut value = 0_u64;
                loop {
                    value += 1;
                    let _ = std::fs::write(&marker, value.to_string());
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
        }
        "environment" => {
            if std::env::var("WISP_ACP_TEST_ENV").as_deref() != Ok("controlled-child") {
                eprintln!("controlled child environment was not propagated");
                return ExitCode::FAILURE;
            }
        }
        "full" | "mismatch" | "no-caps" => {}
        _ => return ExitCode::FAILURE,
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("fake agent runtime");
    match runtime.block_on(serve_fake(scenario)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fake agent failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fake_codex_app_server() -> ExitCode {
    let Some(capture_path) = std::env::var_os("WISP_FAKE_CODEX_CAPTURE").map(PathBuf::from) else {
        eprintln!("fake Codex app-server capture path is missing");
        return ExitCode::FAILURE;
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut thread_start = None;
    let mut turn_start = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(method) = message.get("method").and_then(serde_json::Value::as_str) {
            let Some(id) = message.get("id").cloned() else {
                continue;
            };
            let params = message
                .get("params")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let result = match method {
                "initialize" => serde_json::json!({}),
                "account/read" => serde_json::json!({
                    "account": { "type": "chatgpt", "email": "policy@example.invalid" },
                    "requiresOpenaiAuth": false,
                }),
                "config/read" => serde_json::json!({ "config": {} }),
                "skills/extraRoots/set" => serde_json::json!({}),
                "skills/list" => serde_json::json!({ "data": [] }),
                "thread/start" => {
                    thread_start = Some(params);
                    serde_json::json!({
                        "thread": { "id": "fake-thread" },
                        "model": "gpt-5",
                        "reasoningEffort": "medium",
                        "modelProvider": "openai",
                        "serviceTier": null,
                    })
                }
                "model/list" => serde_json::json!({
                    "data": [{
                        "id": "gpt-5",
                        "displayName": "GPT-5",
                        "description": "Policy test model",
                        "isDefault": true,
                        "defaultReasoningEffort": "medium",
                        "supportedReasoningEfforts": [{
                            "reasoningEffort": "medium",
                            "description": "Policy test effort",
                        }],
                        "inputModalities": ["text"],
                    }],
                    "nextCursor": null,
                }),
                "turn/start" => {
                    turn_start = Some(params);
                    serde_json::json!({
                        "turn": {
                            "id": "fake-turn",
                            "status": "inProgress",
                            "items": [],
                            "itemsView": "notLoaded",
                            "error": null,
                        }
                    })
                }
                _ => serde_json::json!({}),
            };
            if write_json_line(
                &mut stdout,
                &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            )
            .is_err()
            {
                return ExitCode::FAILURE;
            }
            if method == "turn/start"
                && write_json_line(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 900,
                        "method": "item/commandExecution/requestApproval",
                        "params": {
                            "threadId": "fake-thread",
                            "turnId": "fake-turn",
                            "itemId": "escape-command",
                            "command": "curl https://example.invalid",
                            "cwd": "/",
                            "reason": "policy escalation probe",
                        }
                    }),
                )
                .is_err()
            {
                return ExitCode::FAILURE;
            }
            continue;
        }
        if message.get("id").and_then(serde_json::Value::as_i64) == Some(900) {
            let approval = message
                .get("result")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let capture = serde_json::json!({
                "thread": thread_start,
                "turn": turn_start,
                "approval": approval,
            });
            if std::fs::write(
                &capture_path,
                serde_json::to_vec_pretty(&capture).expect("capture serializes"),
            )
            .is_err()
            {
                return ExitCode::FAILURE;
            }
            if write_json_line(
                &mut stdout,
                &serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "turn/completed",
                    "params": {
                        "threadId": "fake-thread",
                        "turn": {
                            "id": "fake-turn",
                            "status": "completed",
                            "items": [],
                            "itemsView": "notLoaded",
                            "error": null,
                            "startedAt": null,
                            "completedAt": null,
                            "durationMs": null,
                        }
                    }
                }),
            )
            .is_err()
            {
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn write_json_line(output: &mut impl Write, value: &serde_json::Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}

#[derive(Default)]
struct FakeState {
    authenticated: AtomicBool,
    cancelled: tokio::sync::Notify,
}

async fn serve_fake(scenario: &str) -> acp::Result<()> {
    let mismatch = scenario == "mismatch";
    let full_capabilities = scenario != "no-caps";
    let state = Arc::new(FakeState::default());
    Agent
        .builder()
        .name("wisp-acp-fake")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _cx| {
                let protocol = if mismatch {
                    ProtocolVersion::V0
                } else {
                    request.protocol_version
                };
                let capabilities = if full_capabilities {
                    AgentCapabilities::new()
                        .load_session(true)
                        .session_capabilities(
                            SessionCapabilities::new()
                                .resume(SessionResumeCapabilities::new())
                                .close(SessionCloseCapabilities::new()),
                        )
                } else {
                    AgentCapabilities::new()
                };
                responder.respond(
                    InitializeResponse::new(protocol)
                        .agent_capabilities(capabilities)
                        .auth_methods(vec![AuthMethod::Agent(AuthMethodAgent::new(
                            "fake-login",
                            "Fake login",
                        ))]),
                )
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                async move |_request: AuthenticateRequest, responder, _cx| {
                    state.authenticated.store(true, Ordering::Release);
                    responder.respond(AuthenticateResponse::new())
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                async move |_request: NewSessionRequest, responder, _cx| {
                    if !state.authenticated.load(Ordering::Acquire) {
                        return responder
                            .respond_with_error(acp::util::internal_error("auth required"));
                    }
                    responder.respond(
                        NewSessionResponse::new("fake-session")
                            .config_options(vec![SessionConfigOption::boolean(
                                "thinking", "Thinking", false,
                            )])
                            .modes(SessionModeState::new(
                                "agent",
                                vec![
                                    SessionMode::new("agent", "Agent"),
                                    SessionMode::new("full-access", "Full Access"),
                                ],
                            )),
                    )
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                    let text = request.prompt.iter().find_map(|content| match content {
                        ContentBlock::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    });
                    let session_id = request.session_id;
                    cx.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("before"),
                        ))),
                    ))?;
                    if text == Some("cancel") {
                        let permission = permission_request(session_id.clone(), "cancel-tool");
                        let task_cx = cx.clone();
                        let state = state.clone();
                        cx.spawn(async move {
                            let permission = task_cx.send_request(permission).block_task();
                            let cancelled = state.cancelled.notified();
                            let (permission, ()) = tokio::join!(permission, cancelled);
                            check_cancelled(permission?)?;
                            task_cx.send_notification(SessionNotification::new(
                                session_id,
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new("cancel-tail")),
                                )),
                            ))?;
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        })?;
                    } else {
                        let task_cx = cx.clone();
                        cx.spawn(async move {
                            let first = task_cx
                                .send_request(permission_request(session_id.clone(), "tool-a"))
                                .block_task();
                            let second = task_cx
                                .send_request(permission_request(session_id.clone(), "tool-b"))
                                .block_task();
                            let (first, second) = tokio::join!(first, second);
                            check_selected(first?)?;
                            check_selected(second?)?;
                            task_cx.send_notification(FutureSessionUpdate::new(&session_id))?;
                            task_cx.send_notification(SessionNotification::new(
                                session_id,
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new("after")),
                                )),
                            ))?;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })?;
                    }
                    Ok(())
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let state = state.clone();
                async move |_notification: CancelNotification, _cx| {
                    state.cancelled.notify_waiters();
                    Ok(())
                }
            },
            acp::on_receive_notification!(),
        )
        .on_receive_request(
            async move |_request: SetSessionConfigOptionRequest, responder, _cx| {
                responder.respond(SetSessionConfigOptionResponse::new(vec![
                    SessionConfigOption::boolean("thinking", "Thinking", true),
                ]))
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, _cx| {
                // Reject unknown ids so the client-side round-trip actually
                // asserts the selected mode reached the agent intact.
                match request.mode_id.to_string().as_str() {
                    "agent" | "full-access" => responder.respond(SetSessionModeResponse::new()),
                    other => responder.respond_with_error(acp::util::internal_error(format!(
                        "unknown mode {other}"
                    ))),
                }
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: ResumeSessionRequest, responder, _cx| {
                responder.respond(ResumeSessionResponse::new())
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: LoadSessionRequest, responder, _cx| {
                responder.respond(LoadSessionResponse::new())
            },
            acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: CloseSessionRequest, responder, _cx| {
                responder.respond(CloseSessionResponse::new())
            },
            acp::on_receive_request!(),
        )
        .connect_to(acp::Stdio::new())
        .await
}

fn permission_request(
    session_id: acp::schema::v1::SessionId,
    tool_id: &str,
) -> RequestPermissionRequest {
    RequestPermissionRequest::new(
        session_id,
        ToolCallUpdate::new(
            tool_id.to_string(),
            ToolCallUpdateFields::new().title(format!("Permission for {tool_id}")),
        ),
        vec![PermissionOption::new(
            "allow",
            "Allow once",
            PermissionOptionKind::AllowOnce,
        )],
    )
}

fn check_selected(response: acp::schema::v1::RequestPermissionResponse) -> acp::Result<()> {
    match response.outcome {
        RequestPermissionOutcome::Selected(_) => Ok(()),
        _ => Err(acp::util::internal_error("permission was not selected")),
    }
}

fn check_cancelled(response: acp::schema::v1::RequestPermissionResponse) -> acp::Result<()> {
    match response.outcome {
        RequestPermissionOutcome::Cancelled => Ok(()),
        _ => Err(acp::util::internal_error("permission was not cancelled")),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FutureSessionUpdate {
    session_id: String,
    update: serde_json::Value,
}

impl FutureSessionUpdate {
    fn new(session_id: &acp::schema::v1::SessionId) -> Self {
        Self {
            session_id: session_id.to_string(),
            update: serde_json::json!({ "sessionUpdate": "future_variant", "value": 42 }),
        }
    }
}

impl JsonRpcMessage for FutureSessionUpdate {
    fn matches_method(method: &str) -> bool {
        method == "session/update"
    }

    fn method(&self) -> &'static str {
        "session/update"
    }

    fn to_untyped_message(&self) -> acp::Result<UntypedMessage> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(_method: &str, _params: &impl serde::Serialize) -> acp::Result<Self> {
        Err(acp::util::internal_error("test notification is send-only"))
    }
}

impl JsonRpcNotification for FutureSessionUpdate {}
