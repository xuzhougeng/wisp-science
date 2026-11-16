//! The research assistant's own WeChat, a port of the WebView's
//! `src-tauri/src/channels/assistant.rs`: every message goes to the
//! assistant's one conversation, never to the projects' shared route, and
//! a few short words answer what it asks or change how it runs.

use anyhow::Result;
use tokio::sync::mpsc;

use super::{ChatEvent, TurnOrigin};
use crate::assistant::ASSISTANT_FRAME_ID;
use crate::Server;
use wisp_store::ASSISTANT_PROJECT_ID;

const HELP: &str = "这里是科研助理，直接用自然语言管理所有可见项目。例如：这周各项目进展如何？在 RNA-seq 项目里安排分析。\n\
yes — 批准助理当前操作一次\n\
no — 拒绝助理当前操作\n\
full — 批准本次并开启助理会话完全权限，后续普通工具操作免审批\n\
full off — 关闭助理会话完全权限\n\
/approval — 查看助理待审批请求\n\
/status — 查看接入、模型和权限状态\n\
/model — 查看可用模型；/model <编号或名称> 切换助理模型\n\
/resume — 重跑上一轮失败的请求\n\
/mandates — 查看全部研究职责的状态、KPI 和等你处理的事项\n\
/stop — 停止助理当前回复\n\
完全权限保存在助理会话上，也可在桌面关闭。无需 /project 或 /session 切换；派发任务由助理先判断审批，拿不准时在这里请你确认。";

const FIXED: &str =
    "这里固定连接科研助理，不切换项目或新建助理会话。请直接说明项目和需求；发送 /help 查看帮助。";

#[derive(Debug, PartialEq, Eq)]
enum Inbound {
    Reply(&'static str),
    Stop,
    Approve(bool),
    FullPermission(bool),
    Approvals,
    /// `/approve <code>` or `/reject <code> [why]`, as a waiting request
    /// tells the owner to answer.
    Answer(bool, String),
    Status,
    /// `/model` and its argument; an empty argument lists the models.
    Model(String),
    Resume,
    Mandates,
    Message,
}

fn classify(text: &str) -> Inbound {
    let trimmed = text.trim();
    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "yes" => return Inbound::Approve(true),
        "no" => return Inbound::Approve(false),
        "full" => return Inbound::FullPermission(true),
        "full off" => return Inbound::FullPermission(false),
        "resume" => return Inbound::Resume,
        _ => {}
    }
    let command = normalized.split_whitespace().next().unwrap_or_default();
    // The command is ASCII, so its length also indexes the original text.
    let argument = trimmed.get(command.len()..).unwrap_or_default().trim();
    match command {
        "/help" => Inbound::Reply(HELP),
        "/status" => Inbound::Status,
        "/approval" | "/approvals" => Inbound::Approvals,
        "/approve" => Inbound::Answer(true, argument.into()),
        "/reject" => Inbound::Answer(false, argument.into()),
        "/stop" => Inbound::Stop,
        "/model" | "/models" => Inbound::Model(argument.into()),
        "/resume" => Inbound::Resume,
        "/mandates" | "/mandate" => Inbound::Mandates,
        _ if command.starts_with('/') => Inbound::Reply(FIXED),
        _ => Inbound::Message,
    }
}

/// Words answered at once, outside the turn queue: the turn under way may
/// be waiting on this very answer. A resume runs a turn, so it queues.
pub(crate) fn is_control_text(text: &str) -> bool {
    !matches!(classify(text), Inbound::Message | Inbound::Resume)
}

impl Server {
    /// One text from the assistant's WeChat. Returns the reply, which may
    /// be empty; what a turn asks for meanwhile goes to `events`.
    pub(crate) async fn handle_assistant_inbound(
        &self,
        text: &str,
        events: Option<mpsc::UnboundedSender<ChatEvent>>,
    ) -> String {
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        match classify(text) {
            Inbound::Reply(reply) => reply.into(),
            Inbound::Stop => {
                self.session(ASSISTANT_FRAME_ID).stop();
                "已请求停止助理当前回复。".into()
            }
            Inbound::Approve(approved) => match self.answer_assistant(approved).await {
                Ok(true) if approved => "已批准，助理继续执行。".into(),
                Ok(true) => "已拒绝，助理会收到拒绝结果。".into(),
                Ok(false) => "助理当前没有待审批的操作。".into(),
                Err(error) => format!("处理审批失败: {error:#}"),
            },
            Inbound::FullPermission(on) => match self.assistant_full_access(on).await {
                Ok(answered) => {
                    let state = if on {
                        "已开启助理会话完全权限，普通工具操作不再逐一审批。"
                    } else {
                        "已关闭助理会话完全权限，工具操作会再请你审批。"
                    };
                    if answered {
                        format!("已批准当前操作。{state}")
                    } else {
                        state.into()
                    }
                }
                Err(error) => format!("切换完全权限失败: {error:#}"),
            },
            Inbound::Approvals => self.assistant_approvals(),
            Inbound::Answer(approved, argument) => {
                let mut words = argument.splitn(2, char::is_whitespace);
                let selector = words.next().unwrap_or_default().trim();
                if selector.is_empty() {
                    return "用法: /approve <审批编号> 或 /reject <审批编号> [原因]".into();
                }
                let feedback = words
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                match self.respond_text_approval(selector, approved, feedback) {
                    Ok(_) if approved => "已批准，助理继续执行。".into(),
                    Ok(_) => "已拒绝，助理会收到拒绝结果。".into(),
                    Err(error) => format!("处理审批失败: {error}"),
                }
            }
            Inbound::Status => self.assistant_status().await,
            Inbound::Model(argument) => match self.assistant_model(&argument).await {
                Ok(reply) | Err(reply) => reply,
            },
            Inbound::Resume => {
                let unstarted = self.0.channels.assistant_unstarted.lock().unwrap().take();
                match unstarted {
                    Some(request) => self.assistant_turn(&request, events).await,
                    None => "没有需要重跑的请求。".into(),
                }
            }
            Inbound::Mandates => match crate::mandates::summary_text(&self.0.store).await {
                Ok(summary) => summary,
                Err(error) => format!("读取研究职责失败: {error:#}"),
            },
            Inbound::Message => self.assistant_turn(text, events).await,
        }
    }

    /// A turn of the assistant's conversation; a request whose turn failed
    /// is kept for `resume`.
    async fn assistant_turn(
        &self,
        text: &str,
        events: Option<mpsc::UnboundedSender<ChatEvent>>,
    ) -> String {
        let turn = async {
            let (project, frame) = self.open_assistant().await?;
            self.chat_turn(&project, &frame, text, events, TurnOrigin::Chat)
                .await
        };
        match turn.await {
            Ok(reply) => reply,
            Err(error) => {
                *self.0.channels.assistant_unstarted.lock().unwrap() = Some(text.to_owned());
                format!("处理失败:{error:#}\n发送 resume 重跑这条请求。")
            }
        }
    }

    /// Answers the assistant's waiting request, if there is one.
    async fn answer_assistant(&self, approved: bool) -> Result<bool> {
        let waiting = {
            let session = self.session(ASSISTANT_FRAME_ID);
            let live = session.live.lock().unwrap();
            let mut ids: Vec<_> = live.approvals.keys().cloned().collect();
            ids.sort();
            ids.into_iter().next()
        };
        let Some(id) = waiting else {
            return Ok(false);
        };
        self.answer_approval(
            ASSISTANT_PROJECT_ID,
            ASSISTANT_FRAME_ID,
            &id,
            approved,
            crate::desktop::Remember::No,
        )
        .await?;
        Ok(true)
    }

    /// Turns the assistant conversation's full access on or off; on also
    /// approves what it is waiting on. Returns whether it answered one.
    async fn assistant_full_access(&self, on: bool) -> Result<bool> {
        let (project, frame) = self.open_assistant().await?;
        let mut options = self.stored_options(&frame).await?;
        options.full_access = on;
        self.set_conversation_options(&project, &frame, options)
            .await?;
        if on {
            self.answer_assistant(true).await
        } else {
            Ok(false)
        }
    }

    fn assistant_approvals(&self) -> String {
        let session = self.session(ASSISTANT_FRAME_ID);
        let live = session.live.lock().unwrap();
        if live.approvals.is_empty() {
            return "助理当前没有待审批的操作。".into();
        }
        let mut waiting: Vec<_> = live
            .approvals
            .iter()
            .map(|(id, (request, _))| super::PendingApproval {
                approval_id: id.clone(),
                frame_id: ASSISTANT_FRAME_ID.into(),
                tool: request.tool.clone(),
                message: request.message.clone(),
                preview: request.preview.clone(),
            })
            .collect();
        waiting.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
        let mut text = waiting
            .iter()
            .map(super::render_approval_request)
            .collect::<Vec<_>>()
            .join("\n\n");
        text.push_str("\n\n也可回复 yes 批准、no 拒绝、full 批准并开启完全权限。");
        text
    }

    async fn assistant_status(&self) -> String {
        let options = match self.stored_options(ASSISTANT_FRAME_ID).await {
            Ok(options) => options,
            Err(error) => return format!("读取助理状态失败: {error:#}"),
        };
        let model = self.assistant_model_name(&options.model);
        let running = self
            .session(ASSISTANT_FRAME_ID)
            .live
            .lock()
            .unwrap()
            .running;
        format!(
            "科研助理微信：已接入\n模型：{model}\n完全权限：{}\n当前回复：{}",
            if options.full_access { "开" } else { "关" },
            if running { "进行中" } else { "空闲" }
        )
    }

    fn assistant_model_name(&self, choice: &str) -> String {
        let model = self.0.model.read().unwrap();
        let settings = model.settings();
        let profile = if choice.is_empty() {
            settings
                .profiles
                .iter()
                .find(|profile| profile.is_default)
                .or_else(|| settings.profiles.first())
        } else {
            settings
                .profiles
                .iter()
                .find(|profile| profile.id == choice)
        };
        match profile {
            Some(profile) => profile_name(profile),
            None => "默认模型".into(),
        }
    }

    /// Lists the models, or switches the assistant to one by its number or
    /// name.
    async fn assistant_model(&self, argument: &str) -> Result<String, String> {
        let (project, frame) = self
            .open_assistant()
            .await
            .map_err(|error| format!("打开助理会话失败: {error:#}"))?;
        let mut options = self
            .stored_options(&frame)
            .await
            .map_err(|error| format!("读取助理设置失败: {error:#}"))?;
        let profiles: Vec<(String, String)> = {
            let model = self.0.model.read().unwrap();
            model
                .settings()
                .profiles
                .iter()
                .map(|profile| (profile.id.clone(), profile_name(profile)))
                .collect()
        };
        if profiles.is_empty() {
            return Err("还没有可用的模型，请先在桌面设置里添加。".into());
        }
        let current = self.assistant_model_name(&options.model);
        if argument.is_empty() {
            let mut lines = vec![format!("助理当前模型：{current}")];
            for (index, (_, name)) in profiles.iter().enumerate() {
                lines.push(format!("{}. {name}", index + 1));
            }
            lines.push("发送 /model <编号或名称> 切换。".into());
            return Ok(lines.join("\n"));
        }
        let picked = argument
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| profiles.get(index))
            .or_else(|| {
                profiles
                    .iter()
                    .find(|(id, name)| id == argument || name.eq_ignore_ascii_case(argument))
            })
            .ok_or_else(|| format!("没有找到模型“{argument}”。发送 /model 查看列表。"))?;
        options.model = picked.0.clone();
        self.set_conversation_options(&project, &frame, options)
            .await
            .map_err(|error| format!("切换模型失败: {error:#}"))?;
        Ok(format!(
            "助理已切换到模型：{}，下一条消息起生效。",
            picked.1
        ))
    }
}

/// A model as the owner reads it: its name, or its model id.
fn profile_name(profile: &crate::desktop::ModelProfile) -> String {
    if profile.name.trim().is_empty() {
        profile.fields.model.clone()
    } else {
        profile.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_few_words_answer_or_steer_and_the_rest_is_a_request() {
        assert_eq!(classify(" YES "), Inbound::Approve(true));
        assert_eq!(classify("no"), Inbound::Approve(false));
        assert_eq!(classify("full"), Inbound::FullPermission(true));
        assert_eq!(classify("Full Off"), Inbound::FullPermission(false));
        assert_eq!(classify("/help"), Inbound::Reply(HELP));
        assert_eq!(classify("/model 2"), Inbound::Model("2".into()));
        assert_eq!(classify("/models"), Inbound::Model(String::new()));
        assert_eq!(
            classify("/reject ABCD1234 not now"),
            Inbound::Answer(false, "ABCD1234 not now".into())
        );
        // The projects' routing words do not reach a project from here.
        assert_eq!(classify("/project 1"), Inbound::Reply(FIXED));
        assert_eq!(classify("/session new"), Inbound::Reply(FIXED));
        assert_eq!(classify("这周各项目进展如何？"), Inbound::Message);
        // Answers skip the turn queue; requests and resume wait in it.
        assert!(is_control_text("yes"));
        assert!(is_control_text("/stop"));
        assert!(is_control_text("/mandates"));
        assert!(!is_control_text("resume"));
        assert!(!is_control_text("plan my week"));
    }
}
