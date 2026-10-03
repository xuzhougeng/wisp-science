//! The assistant binding never reads or changes the project IM route.
use super::*;
use crate::research_assistant::{ensure, ASSISTANT_FRAME_ID};

enum Inbound {
    Reply(String),
    Stop,
    Approve(bool),
    FullPermission(bool),
    Approvals,
    Status,
    Message,
}

fn classify(text: &str) -> Inbound {
    let normalized = text.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "yes" => return Inbound::Approve(true),
        "no" => return Inbound::Approve(false),
        "full" => return Inbound::FullPermission(true),
        "full off" => return Inbound::FullPermission(false),
        _ => {}
    }
    let command = normalized.split_whitespace().next().unwrap_or_default();
    match command {
        "/help" => Inbound::Reply("这里是科研助理，直接用自然语言管理所有可见项目。例如：这周各项目进展如何？在 RNA-seq 项目里安排分析。\nyes — 批准助理当前操作一次\nno — 拒绝助理当前操作\nfull — 批准本次并开启助理会话完全权限，后续普通工具操作免审批\nfull off — 关闭助理会话完全权限\n/approval — 查看助理待审批请求\n/status — 查看接入和权限状态\n/stop — 停止助理当前回复\n完全权限仅限助理会话，重启 Wisp 后失效。无需 /project 或 /session 切换；派发任务由助理先判断审批，拿不准时在这里请你确认。".into()),
        "/status" => Inbound::Status,
        "/approval" | "/approvals" => Inbound::Approvals,
        "/stop" => Inbound::Stop,
        _ if command.starts_with('/') => Inbound::Reply("这里固定连接科研助理，不切换项目或新建助理会话。请直接说明项目和需求；发送 /help 查看帮助。".into()),
        _ => Inbound::Message,
    }
}

/// These replies must bypass the turn queue, whose current turn may be
/// blocked on the very confirmation the owner is answering.
pub(super) fn is_control_text(text: &str) -> bool {
    !matches!(classify(text), Inbound::Message)
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
    match classify(text) {
        Inbound::Reply(reply) => return reply,
        Inbound::Status => {
            let permission =
                if crate::approval_commands::session_full_permission(&state, ASSISTANT_FRAME_ID) {
                    "已开启（仅限助理会话，重启后失效；full off 关闭）"
                } else {
                    "未开启（需要确认时回复 yes / no / full）"
                };
            return format!("已接入科研助理，与桌面科研助理共用同一条长期对话，可管理所有可见项目。\n完全权限：{permission}");
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
        Inbound::Message => {}
    }
    let Some(window) = app.workspace_surface("main") else {
        return "桌面端主窗口不可用,无法处理消息。".into();
    };
    let session = match resolve_session(&state.store, &state.app_data).await {
        Ok(session) => session,
        Err(error) => return format!("打开科研助理失败: {error}"),
    };
    send_inbound_turn(app, window.label(), session, text, progress).await
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
