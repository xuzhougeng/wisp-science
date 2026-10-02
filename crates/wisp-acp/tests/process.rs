use std::{
    collections::HashMap,
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
                AgentCapabilities, AuthMethod, AuthMethodAgent, AuthMethodTerminal,
                AuthenticateRequest, AuthenticateResponse, CancelNotification, CloseSessionRequest,
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
    AcpAgentProfile, AcpAuthMethodKind, AcpError, AcpPermissionKind, AcpSessionEvent,
    AcpSessionHandle, AcpStopReason, AcpUpdateKind,
};

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
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
    test_environment_propagation()
        .await
        .map_err(|error| format!("environment propagation: {error}"))?;
    test_terminal_auth_discovery()
        .await
        .map_err(|error| format!("terminal auth discovery: {error}"))?;
    test_full_lifecycle()
        .await
        .map_err(|error| format!("full lifecycle: {error}"))?;
    test_protocol_mismatch()
        .await
        .map_err(|error| format!("protocol mismatch: {error}"))?;
    test_capability_omission()
        .await
        .map_err(|error| format!("capability omission: {error}"))?;
    test_child_exit()
        .await
        .map_err(|error| format!("child exit: {error}"))?;
    test_stderr_bound_and_drop_cleanup()
        .await
        .map_err(|error| format!("stderr and cleanup: {error}"))?;
    test_cancel_during_initialize()
        .await
        .map_err(|error| format!("initialize cancellation: {error}"))?;
    Ok(())
}

async fn test_terminal_auth_discovery() -> Result<(), String> {
    let handle = AcpSessionHandle::launch(profile("terminal-auth", vec![]))
        .await
        .map_err(stringify)?;
    let method = handle
        .info()
        .auth_methods
        .first()
        .ok_or("terminal auth method was not advertised")?;
    check(method.id == "terminal-login", "terminal auth method id")?;
    match &method.kind {
        AcpAuthMethodKind::Terminal { args, env } => {
            check(
                args == &["--cli", "auth", "login", "argument with spaces"],
                "terminal auth argument boundaries",
            )?;
            check(
                env.get("WISP_ACP_AUTH_TEST").map(String::as_str) == Some("enabled"),
                "terminal auth environment",
            )?;
        }
        kind => return Err(format!("expected terminal auth method, got {kind:?}")),
    }
    handle.shutdown(Duration::from_secs(1)).await;
    Ok(())
}

async fn test_environment_propagation() -> Result<(), String> {
    let mut profile = profile("environment", vec![]);
    profile
        .env
        .insert("WISP_ACP_TEST_ENV".into(), "controlled-child".into());
    let handle = AcpSessionHandle::launch(profile).await.map_err(stringify)?;
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

async fn test_cancel_during_initialize() -> Result<(), String> {
    let marker = unique_temp_path("wisp-acp-initialize-cleanup");
    let launch = tokio::spawn(AcpSessionHandle::launch(profile(
        "initialize-hang",
        vec![marker.to_string_lossy().to_string()],
    )));
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    launch.abort();
    check(
        matches!(launch.await, Err(error) if error.is_cancelled()),
        "initialize task was not cancelled",
    )?;
    started.map_err(|_| "child did not reach its initialization wait")?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let first = std::fs::read(&marker).map_err(stringify)?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let second = std::fs::read(&marker).map_err(stringify)?;
    let _ = std::fs::remove_file(marker);
    check(
        first == second,
        "cancelling initialize left its child alive",
    )
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
        "cleanup" | "initialize-hang" => {
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
            if scenario == "initialize-hang" {
                loop {
                    std::thread::park();
                }
            }
        }
        "environment" => {
            if std::env::var("WISP_ACP_TEST_ENV").as_deref() != Ok("controlled-child") {
                eprintln!("controlled child environment was not propagated");
                return ExitCode::FAILURE;
            }
        }
        "full" | "mismatch" | "no-caps" | "terminal-auth" => {}
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

#[derive(Default)]
struct FakeState {
    authenticated: AtomicBool,
    cancelled: tokio::sync::Notify,
}

async fn serve_fake(scenario: &str) -> acp::Result<()> {
    let mismatch = scenario == "mismatch";
    let full_capabilities = scenario != "no-caps";
    let terminal_auth = scenario == "terminal-auth";
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
                let auth_methods = if terminal_auth {
                    if request.client_capabilities.auth.terminal {
                        vec![AuthMethod::Terminal(
                            AuthMethodTerminal::new("terminal-login", "Terminal login")
                                .args(vec![
                                    "--cli".into(),
                                    "auth".into(),
                                    "login".into(),
                                    "argument with spaces".into(),
                                ])
                                .env(HashMap::from([(
                                    "WISP_ACP_AUTH_TEST".into(),
                                    "enabled".into(),
                                )])),
                        )]
                    } else {
                        vec![]
                    }
                } else {
                    vec![AuthMethod::Agent(AuthMethodAgent::new(
                        "fake-login",
                        "Fake login",
                    ))]
                };
                responder.respond(
                    InitializeResponse::new(protocol)
                        .agent_capabilities(capabilities)
                        .auth_methods(auth_methods),
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
