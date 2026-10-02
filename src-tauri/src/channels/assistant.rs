//! The assistant binding never reads or changes the project IM route.
use super::*;
use crate::research_assistant::{ensure, ASSISTANT_FRAME_ID};

enum Inbound {
    Reply(String),
    Stop,
    Message,
}

fn classify(text: &str) -> Inbound {
    let command = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match command.as_str() {
        "/help" => Inbound::Reply("这里是科研助理，直接用自然语言管理所有可见项目。例如：这周各项目进展如何？在 RNA-seq 项目里安排分析。\n/status — 查看接入状态\n/stop — 停止助理当前回复\n无需 /project 或 /session 切换；派发任务的审批请在桌面项目中处理。".into()),
        "/status" => Inbound::Reply("已接入科研助理，与桌面科研助理共用同一条长期对话，可管理所有可见项目。".into()),
        "/stop" => Inbound::Stop,
        _ if command.starts_with('/') => Inbound::Reply("这里固定连接科研助理，不切换项目或新建助理会话。请直接说明项目和需求；发送 /help 查看帮助。".into()),
        _ => Inbound::Message,
    }
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
    match classify(text) {
        Inbound::Reply(reply) => return reply,
        Inbound::Stop => {
            return match crate::stop_agent(app.state(), Some(ASSISTANT_FRAME_ID.into())).await {
                Ok(()) => "已请求停止科研助理当前回复。已派发的任务请到对应项目处理。".into(),
                Err(error) => format!("停止失败: {error}"),
            }
        }
        Inbound::Message => {}
    }
    let state = app.state::<AppState>();
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
    fn assistant_commands_never_reach_project_routing_or_approval() {
        for command in [
            "/project RNA",
            "/projects",
            "/session 1",
            "/sessions",
            "/new",
            "/approve 123",
            "/reject 123",
            "/approval",
            "/unknown",
        ] {
            assert!(matches!(classify(command), Inbound::Reply(_)), "{command}");
        }
        assert!(matches!(classify("/stop"), Inbound::Stop));
        assert!(matches!(classify("汇报所有项目的进展"), Inbound::Message));
        if let Inbound::Reply(reply) = classify("/status") {
            assert!(reply.contains("科研助理"));
        } else {
            panic!("missing status reply");
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
