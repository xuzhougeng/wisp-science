//! The assistant binding never reads or changes the project IM route.
use super::*;
use crate::models::ModelProfile;
use crate::research_assistant::{ensure, ASSISTANT_FRAME_ID};

enum Inbound {
    Reply(String),
    Stop,
    Approve(bool),
    FullPermission(bool),
    Approvals,
    Status,
    /// `/model` and its argument; an empty argument lists the models.
    Model(String),
    Resume,
    /// `/mandates`: every visible research mandate at a glance.
    Mandates,
    Message,
}

/// A request whose turn failed before the conversation accepted it. Nothing
/// was persisted for it, so `/resume` has to send the text again.
static UNSTARTED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn classify(text: &str) -> Inbound {
    let normalized = text.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "yes" => return Inbound::Approve(true),
        "no" => return Inbound::Approve(false),
        "full" => return Inbound::FullPermission(true),
        "full off" => return Inbound::FullPermission(false),
        "resume" => return Inbound::Resume,
        _ => {}
    }
    let command = normalized.split_whitespace().next().unwrap_or_default();
    match command {
        "/help" => Inbound::Reply("这里是科研助理，直接用自然语言管理所有可见项目。例如：这周各项目进展如何？在 RNA-seq 项目里安排分析。\nyes — 批准助理当前操作一次\nno — 拒绝助理当前操作\nfull — 批准本次并开启助理会话完全权限，后续普通工具操作免审批\nfull off — 关闭助理会话完全权限\n/approval — 查看助理待审批请求\n/status — 查看接入、模型和权限状态\n/model — 查看可用模型；/model <编号或名称> 切换助理模型\n/resume — 重跑上一轮失败的请求\n/mandates — 查看全部研究职责的状态、KPI 和等你处理的事项\n/stop — 停止助理当前回复\n完全权限仅限助理会话，重启 Wisp 后失效。无需 /project 或 /session 切换；派发任务由助理先判断审批，拿不准时在这里请你确认。".into()),
        "/status" => Inbound::Status,
        "/approval" | "/approvals" => Inbound::Approvals,
        "/stop" => Inbound::Stop,
        // The command is ASCII, so its length also indexes the original text.
        "/model" | "/models" => Inbound::Model(text.trim()[command.len()..].trim().into()),
        "/resume" => Inbound::Resume,
        "/mandates" | "/mandate" => Inbound::Mandates,
        _ if command.starts_with('/') => Inbound::Reply("这里固定连接科研助理，不切换项目或新建助理会话。请直接说明项目和需求；发送 /help 查看帮助。".into()),
        _ => Inbound::Message,
    }
}

/// These replies must bypass the turn queue, whose current turn may be
/// blocked on the very confirmation the owner is answering. A resume runs a
/// turn of its own, so it queues like a message.
pub(super) fn is_control_text(text: &str) -> bool {
    !matches!(classify(text), Inbound::Message | Inbound::Resume)
}

fn model_name(profile: &ModelProfile) -> &str {
    if profile.label.trim().is_empty() {
        &profile.model
    } else {
        &profile.label
    }
}

fn model_list(profiles: &[ModelProfile], current: &str) -> String {
    if profiles.is_empty() {
        return "还没有可用于对话的模型，请先在桌面端的模型设置中添加。".into();
    }
    let mut reply = String::from("科研助理可用的模型：\n");
    for (index, profile) in profiles.iter().enumerate() {
        let name = model_name(profile);
        reply.push_str(&format!("{}. {name}", index + 1));
        if name != profile.model {
            reply.push_str(&format!(" · {}", profile.model));
        }
        if profile.id == current {
            reply.push_str("（当前）");
        }
        reply.push('\n');
    }
    reply.push_str("发送 /model <编号或名称> 切换，只影响科研助理这条对话。");
    reply
}

/// The profile `/model <argument>` names: its number in the list, else a
/// case-insensitive label, id, or model name, matched whole before in part.
fn pick_model<'a>(
    profiles: &'a [ModelProfile],
    argument: &str,
) -> Result<&'a ModelProfile, String> {
    if let Ok(number) = argument.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|index| profiles.get(index))
            .ok_or_else(|| format!("没有编号为 {number} 的模型；发送 /model 查看列表。"));
    }
    let wanted = argument.to_lowercase();
    let is = |value: &str| value.to_lowercase() == wanted;
    if let Some(profile) = profiles
        .iter()
        .find(|p| is(&p.label) || is(&p.id) || is(&p.model))
    {
        return Ok(profile);
    }
    let has = |value: &str| value.to_lowercase().contains(&wanted);
    let partial: Vec<_> = profiles
        .iter()
        .filter(|p| has(&p.label) || has(&p.model))
        .collect();
    match partial.as_slice() {
        [profile] => Ok(profile),
        [] => Err(format!(
            "没有名称包含“{argument}”的模型；发送 /model 查看列表。"
        )),
        several => Err(format!(
            "“{argument}”匹配到多个模型：{}。请改用编号。",
            several
                .iter()
                .map(|p| model_name(p))
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

async fn current_model(state: &AppState) -> (Vec<ModelProfile>, String) {
    (
        crate::models::delegation_profiles(&state.store).await,
        crate::models::session_profile_id(&state.store, ASSISTANT_FRAME_ID).await,
    )
}

async fn model_reply(state: &AppState, argument: &str) -> String {
    if let Err(error) = resolve_session(&state.store, &state.app_data).await {
        return format!("打开科研助理失败: {error}");
    }
    let (profiles, current) = current_model(state).await;
    if argument.is_empty() {
        return model_list(&profiles, &current);
    }
    let profile = match pick_model(&profiles, argument) {
        Ok(profile) => profile,
        Err(reply) => return reply,
    };
    let name = model_name(profile);
    if profile.id == current {
        return format!("科研助理已在使用 {name}。");
    }
    match crate::models::set_session_model(state, ASSISTANT_FRAME_ID, &profile.id).await {
        Ok(()) => {
            format!("已将科研助理的模型切换为 {name}，下一轮回复起生效；只影响科研助理这条对话。")
        }
        Err(error) => format!("切换模型失败: {error}"),
    }
}

/// What `/resume` runs again.
#[derive(Debug, PartialEq)]
enum Rerun {
    /// The request never reached the conversation: send it again.
    Resend(String),
    /// The turn started and then failed: continue it without a new message.
    Continue,
}

fn rerun(unstarted: Option<String>, last_outcome: Option<&str>) -> Option<Rerun> {
    match unstarted {
        Some(text) => Some(Rerun::Resend(text)),
        None => (last_outcome == Some("Error")).then_some(Rerun::Continue),
    }
}

/// The reply for a failed turn, and the request to remember when the turn
/// never started and so left nothing in the conversation to continue from.
fn turn_failure(text: &str, error: &str) -> (Option<String>, String) {
    let (started, message) = crate::split_turn_error(error);
    (
        (!started && !text.is_empty()).then(|| text.to_string()),
        format!("处理失败：{message}\n发送 /resume 重跑这一轮；模型不可用时可先用 /model 切换。"),
    )
}

pub(super) fn approval_message(request: &crate::ConfirmRequest) -> String {
    if request.tool == "project_approval" {
        return format!("科研助理需要你确认项目操作。\n{}\n\n回复 yes 批准本次，no 拒绝。选择会转交给原项目的这条审批。", super::approval_preview(request));
    }
    format!(
        "科研助理等待审批。\n工具: {}\n{}\n\n回复 yes 批准本次，no 拒绝，full 批准并开启助理会话完全权限（后续普通工具操作免审批）。\n完全权限仅限助理会话，项目审批由助理判断，拿不准时再请你确认；重启 Wisp 后失效，回复 full off 可关闭。",
        request.tool,
        super::approval_preview(request),
    )
}

fn pending_approval(state: &AppState) -> Option<crate::ConfirmRequest> {
    state
        .confirms
        .lock()
        .unwrap()
        .get(ASSISTANT_FRAME_ID)
        .map(|pending| pending.request.clone())
}

async fn resolve_session(store: &Store, app_data: &std::path::Path) -> Result<String, String> {
    ensure(store, app_data).await?;
    Ok(ASSISTANT_FRAME_ID.into())
}

pub(super) async fn handle_inbound(
    app: &AppHandle,
    text: &str,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> String {
    let state = app.state::<AppState>();
    let inbound = classify(text);
    let resuming = matches!(inbound, Inbound::Resume);
    match inbound {
        Inbound::Reply(reply) => return reply,
        Inbound::Status => {
            let permission =
                if crate::approval_commands::session_full_permission(&state, ASSISTANT_FRAME_ID) {
                    "已开启（仅限助理会话，重启后失效；full off 关闭）"
                } else {
                    "未开启（需要确认时回复 yes / no / full）"
                };
            let (profiles, current) = current_model(&state).await;
            let model = profiles
                .iter()
                .find(|profile| profile.id == current)
                .map_or("未配置", model_name);
            return format!("已接入科研助理，与桌面科研助理共用同一条长期对话，可管理所有可见项目。\n模型：{model}（/model 切换）\n完全权限：{permission}");
        }
        Inbound::Model(argument) => return model_reply(&state, &argument).await,
        Inbound::Mandates => {
            return crate::mandates::summary_text(&state.store)
                .await
                .unwrap_or_else(|error| format!("读取研究职责失败: {error}"))
        }
        Inbound::Approvals => {
            return pending_approval(&state)
                .map(|request| approval_message(&request))
                .unwrap_or_else(|| "科研助理当前没有待审批请求。".into());
        }
        Inbound::Approve(approved) => {
            let Some(pending) = pending_approval(&state) else {
                return "科研助理当前没有待审批请求；它可能已经处理或失效。".into();
            };
            // Match both the assistant project and exact approval ID, so a
            // desktop response winning this race cannot approve a later call.
            let request = wisp_dto::native_conversations::ApprovalRequest {
                session_id: ASSISTANT_FRAME_ID.into(),
                approval_id: pending.approval_id,
                approved,
                feedback: None,
                scope: Default::default(),
            };
            let project_approval = pending.tool == "project_approval";
            return match crate::approval_commands::respond_native_confirmation(
                &state,
                wisp_store::ASSISTANT_PROJECT_ID,
                &request,
            )
            .await
            {
                Ok(()) if project_approval => format!(
                    "已提交项目操作的{}选择，科研助理正在转交原审批。",
                    if approved { "批准" } else { "拒绝" }
                ),
                Ok(()) if approved => "已批准本次操作，科研助理将继续执行。".into(),
                Ok(()) => "已拒绝本次操作，科研助理将收到拒绝结果。".into(),
                Err(error) => format!("审批失败: {error}"),
            };
        }
        Inbound::FullPermission(enabled) => {
            return match crate::approval_commands::set_session_full_permission_inner(
                &state, ASSISTANT_FRAME_ID, enabled,
            ) {
                Ok(true) => "已开启科研助理会话完全权限：当前审批（如有）已批准，后续普通工具操作免审批。仅限助理会话，项目审批由助理判断，拿不准时再请你确认；重启 Wisp 后失效，回复 full off 可关闭。".into(),
                Ok(false) => "已关闭科研助理会话完全权限，后续需要确认的操作会再次请求审批。".into(),
                Err(error) => format!("设置权限失败: {error}"),
            };
        }
        Inbound::Stop => {
            return match crate::stop_agent(app.state(), Some(ASSISTANT_FRAME_ID.into())).await {
                Ok(()) => "已请求停止科研助理当前回复。已派发的任务请到对应项目处理。".into(),
                Err(error) => format!("停止失败: {error}"),
            }
        }
        Inbound::Message | Inbound::Resume => {}
    }
    let Some(window) = app.workspace_surface("main") else {
        return "桌面端主窗口不可用,无法处理消息。".into();
    };
    let session = match resolve_session(&state.store, &state.app_data).await {
        Ok(session) => session,
        Err(error) => return format!("打开科研助理失败: {error}"),
    };
    let (text, resume) = if resuming {
        if state
            .running_turns
            .lock()
            .await
            .contains(ASSISTANT_FRAME_ID)
        {
            return "科研助理正在回复，等这一轮结束后再发送 /resume。".into();
        }
        let unstarted = UNSTARTED.lock().unwrap().take();
        let outcome = state.store.last_turn_outcome(ASSISTANT_FRAME_ID).await;
        match rerun(unstarted, outcome.ok().flatten().as_deref()) {
            Some(Rerun::Resend(text)) => (text, false),
            Some(Rerun::Continue) => (String::new(), true),
            None => return "科研助理上一轮没有失败，没有需要重跑的请求。".into(),
        }
    } else {
        (text.to_string(), false)
    };
    let result = run_inbound_turn(app, window.label(), session, &text, resume, progress).await;
    let (unstarted, reply) = match result {
        Ok(reply) => (None, reply),
        Err(error) => turn_failure(&text, &error),
    };
    *UNSTARTED.lock().unwrap() = unstarted;
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_commands_never_reach_project_routing() {
        for command in [
            "/project RNA",
            "/projects",
            "/session 1",
            "/sessions",
            "/new",
            "/approve 123",
            "/reject 123",
            "/unknown",
        ] {
            assert!(matches!(classify(command), Inbound::Reply(_)), "{command}");
        }
        assert!(matches!(classify("/stop"), Inbound::Stop));
        assert!(matches!(classify("汇报所有项目的进展"), Inbound::Message));
        assert!(matches!(classify("/status"), Inbound::Status));
        // A read of every mandate, answered at once like /status.
        for command in ["/mandates", " /Mandates ", "/mandate"] {
            assert!(matches!(classify(command), Inbound::Mandates), "{command}");
            assert!(is_control_text(command));
        }
    }

    fn profile(id: &str, label: &str, model: &str) -> ModelProfile {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": label, "provider": "openai", "api_url": "", "model": model
        }))
        .unwrap()
    }

    #[test]
    fn model_command_lists_and_picks_by_number_or_name() {
        assert!(matches!(classify("/model"), Inbound::Model(a) if a.is_empty()));
        assert!(matches!(classify("/models"), Inbound::Model(a) if a.is_empty()));
        assert!(
            matches!(classify(" /MODEL  DeepSeek V4 "), Inbound::Model(a) if a == "DeepSeek V4")
        );
        assert!(is_control_text("/model 2"));

        let profiles = [
            profile("m1", "GPT-5.5", "gpt-5.5"),
            profile("m2", "", "deepseek-v4-flash"),
            profile("m3", "DeepSeek Pro", "deepseek-v4-pro"),
        ];
        let picked = |argument| pick_model(&profiles, argument).map(|p| p.id.as_str());
        assert_eq!(picked("2"), Ok("m2"));
        assert_eq!(picked("gpt-5.5"), Ok("m1"));
        assert_eq!(picked("M3"), Ok("m3"));
        assert_eq!(picked("flash"), Ok("m2"));
        assert!(picked("deepseek").unwrap_err().contains("DeepSeek Pro"));
        assert!(picked("0").is_err());
        assert!(picked("4").is_err());
        assert!(picked("claude").is_err());

        assert_eq!(
            model_list(&profiles, "m3"),
            "科研助理可用的模型：\n1. GPT-5.5 · gpt-5.5\n2. deepseek-v4-flash\n\
             3. DeepSeek Pro · deepseek-v4-pro（当前）\n\
             发送 /model <编号或名称> 切换，只影响科研助理这条对话。"
        );
    }

    #[test]
    fn resume_queues_as_a_turn_and_reruns_only_a_failed_request() {
        for text in ["/resume", " /Resume ", "resume", "RESUME"] {
            assert!(matches!(classify(text), Inbound::Resume), "{text}");
            assert!(!is_control_text(text), "{text}");
        }
        assert!(matches!(classify("resume the analysis"), Inbound::Message));

        assert_eq!(rerun(None, Some("Error")), Some(Rerun::Continue));
        assert_eq!(rerun(None, Some("Done")), None);
        assert_eq!(rerun(None, None), None);
        assert_eq!(
            rerun(Some("汇报进展".into()), Some("Done")),
            Some(Rerun::Resend("汇报进展".into()))
        );

        // A started turn left its request in the conversation; only a turn
        // that never started needs the text kept for the rerun.
        let (unstarted, reply) = turn_failure("汇报进展", "[turn-started] api: 504");
        assert_eq!(unstarted, None);
        assert!(reply.starts_with("处理失败：api: 504\n") && reply.contains("/resume"));
        assert_eq!(
            turn_failure("汇报进展", "No API key").0.as_deref(),
            Some("汇报进展")
        );
        assert_eq!(turn_failure("", "project is busy").0, None);
    }

    #[test]
    fn approval_replies_are_exact_case_insensitive_control_commands() {
        for text in ["yes", " YES \n", "Yes"] {
            assert!(matches!(classify(text), Inbound::Approve(true)), "{text}");
            assert!(is_control_text(text));
        }
        assert!(matches!(classify(" NO "), Inbound::Approve(false)));
        assert!(matches!(classify(" FULL "), Inbound::FullPermission(true)));
        assert!(matches!(
            classify("Full Off"),
            Inbound::FullPermission(false)
        ));
        for text in ["/approval", "/approvals"] {
            assert!(matches!(classify(text), Inbound::Approvals));
        }
        for text in [
            "yes please",
            "no changes",
            "full analysis",
            "say yes",
            "yesterday",
        ] {
            assert!(matches!(classify(text), Inbound::Message), "{text}");
            assert!(!is_control_text(text));
        }
    }

    #[tokio::test]
    async fn first_remote_message_reuses_singleton_without_touching_project_route() {
        let dir =
            std::env::temp_dir().join(format!("wisp-assistant-channel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("wisp.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &dir.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("session", "project", "OPERON", "")
            .await
            .unwrap();
        let route = SharedRoute {
            project_id: Some("project".into()),
            session_id: Some("session".into()),
            ..Default::default()
        };
        set_route(&store, &route).await.unwrap();
        let before = get_setting(&store, LAST_MESSAGE_ROUTE_KEY).await;
        let (first, second) =
            tokio::join!(resolve_session(&store, &dir), resolve_session(&store, &dir));
        assert_eq!(first.unwrap(), ASSISTANT_FRAME_ID);
        assert_eq!(second.unwrap(), ASSISTANT_FRAME_ID);
        record_last_message_session(&store, ASSISTANT_FRAME_ID)
            .await
            .unwrap();
        assert_eq!(get_setting(&store, LAST_MESSAGE_ROUTE_KEY).await, before);
        assert_eq!(resolve_message_session(&store).await.unwrap(), "session");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
