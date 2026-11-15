//! Messaging channels: chat apps that talk to Wisp. A port of the
//! WebView's `src-tauri/src/channels/mod.rs`.
//!
//! A text that arrives on a channel drives an ordinary conversation: the
//! turn runs through the same path the desktop's send does, so history,
//! tools and saving behave the same and the conversation shows in the
//! desktop. The turn's answer goes back to the chat when it ends. A turn
//! started from a chat always asks before a tool that changes something,
//! even in a conversation the desktop gives full access; an approval asked
//! for meanwhile is sent to the chat with a code to answer it by.
//!
//! The channels share one target, kept under the WebView's setting
//! `channel_last_message_route`: the project chat messages go to and the
//! conversation they continue there. `/project`, `/session` and `/new`
//! move it; a send from the desktop only records that project's latest
//! conversation and never takes the target. Settings that are not secret
//! live in the store; a bot's token in the keychain.

pub(crate) mod feishu;
mod feishu_card;
pub(crate) mod feishu_registration;
pub(crate) mod lab;
pub(crate) mod lab_mail;
pub(crate) mod pbbp2;
pub(crate) mod weixin;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use wisp_store::Store;

use crate::Server;

/// The longest reply sent to a chat.
pub(crate) const REPLY_MAX_CHARS: usize = 8000;
const APPROVAL_PREVIEW_MAX_CHARS: usize = 1200;
const APPROVAL_CODE_CHARS: usize = 8;
const ROUTE_KEY: &str = "channel_last_message_route";
/// How often a chat's turn is looked at while it runs.
const TURN_POLL: Duration = Duration::from_millis(250);

/// Where a channel's connection stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChannelState {
    #[default]
    Stopped,
    Connecting,
    Running,
    Error,
}

/// Who started a turn: the desktop, a chat app, or another lab member's
/// mail. Only the desktop's turns may run with the conversation's full
/// access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnOrigin {
    Desktop,
    Chat,
    LabMail,
}

/// Which chat app a text came from. Approvals are answered by text only
/// in WeChat's one-to-one chat with its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Weixin,
    Feishu,
}

impl Channel {
    fn key(self) -> &'static str {
        match self {
            Channel::Weixin => "weixin",
            Channel::Feishu => "feishu",
        }
    }
}

/// A running channel: how to stop it, and where it stands.
#[derive(Default)]
pub(crate) struct Worker {
    stop: Mutex<Option<watch::Sender<bool>>>,
    pub(crate) status: Arc<Mutex<(ChannelState, String)>>,
}

impl Worker {
    pub(crate) fn stop(&self) {
        if let Some(stop) = self.stop.lock().unwrap().take() {
            let _ = stop.send(true);
        }
        *self.status.lock().unwrap() = (ChannelState::Stopped, String::new());
    }

    /// Replaces whatever ran before; the receiver ends the new one.
    pub(crate) fn begin(&self) -> watch::Receiver<bool> {
        self.stop();
        let (stop, stopped) = watch::channel(false);
        *self.stop.lock().unwrap() = Some(stop);
        *self.status.lock().unwrap() = (ChannelState::Connecting, String::new());
        stopped
    }

    pub(crate) fn snapshot(&self) -> (ChannelState, String) {
        self.status.lock().unwrap().clone()
    }
}

pub(crate) fn set_status(
    status: &Mutex<(ChannelState, String)>,
    state: ChannelState,
    detail: &str,
) {
    *status.lock().unwrap() = (state, detail.to_owned());
}

/// The channels of one host.
pub(crate) struct Channels {
    pub(crate) weixin: Worker,
    pub(crate) feishu: Worker,
    /// Scans under way that make a Feishu app.
    pub(crate) feishu_flows:
        tokio::sync::Mutex<HashMap<String, feishu_registration::RegistrationFlow>>,
    /// Where Feishu's services are; tests point them at a fake.
    pub(crate) feishu_hosts: Mutex<feishu::Hosts>,
    /// The lab mailbox poller, running while this computer is a member.
    pub(crate) lab: Worker,
    /// Serializes the lab's two small routing settings.
    pub(crate) lab_routes: tokio::sync::Mutex<()>,
    /// The hop of the lab mail whose turn runs in a conversation.
    pub(crate) lab_hops: Mutex<HashMap<String, u32>>,
    /// Serializes binding, enabling and unbinding WeChat.
    pub(crate) weixin_config: tokio::sync::Mutex<()>,
    /// Serializes reading and changing the shared target.
    route: tokio::sync::Mutex<()>,
    /// One turn at a time per chat.
    chats: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Where WeChat's sign-in service is; tests point it at a fake.
    pub(crate) ilink_base: Mutex<String>,
}

impl Default for Channels {
    fn default() -> Self {
        Self {
            weixin: Worker::default(),
            feishu: Worker::default(),
            feishu_flows: Default::default(),
            feishu_hosts: Default::default(),
            lab: Worker::default(),
            lab_routes: Default::default(),
            lab_hops: Default::default(),
            weixin_config: Default::default(),
            route: Default::default(),
            chats: Default::default(),
            ilink_base: Mutex::new(weixin::DEFAULT_BASE_URL.into()),
        }
    }
}

impl Channels {
    pub(crate) fn stop_all(&self) {
        self.weixin.stop();
        self.feishu.stop();
        self.lab.stop();
    }

    pub(crate) fn chat_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.chats
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_default()
            .clone()
    }
}

pub(crate) async fn get_setting(store: &Store, key: &str) -> String {
    store
        .get_setting(key)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub(crate) fn truncate_reply(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}\n……(内容过长已截断,完整内容请在桌面端查看)")
}

/// Where a chat's turn has got, for a channel that shows it as it runs:
/// what the model is writing and the latest tool calls (name, and how it
/// went once finished). Reasoning and tool output are never part of it:
/// they can hold secrets and do not belong in another app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnProgress {
    pub active: bool,
    pub assistant: String,
    pub tools: Vec<(String, Option<(bool, u64)>)>,
}

/// What a channel hears while a chat's turn runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChatEvent {
    /// An approval the turn waits for, as text to send.
    Approval(String),
    Progress(TurnProgress),
}

/// The latest tool calls a chat is shown.
const PROGRESS_TOOLS: usize = 4;

/// The turn's progress from the conversation's live state; `from` is how
/// many messages it held before the turn.
fn progress_of(live: &crate::output::Live, from: usize) -> TurnProgress {
    let new = live.messages.get(from..).unwrap_or_default();
    let mut tools: Vec<(String, Option<(bool, u64)>)> = new
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|call| {
            let outcome = live
                .outcomes
                .get(&call.id)
                .map(|outcome| (outcome.ok, outcome.duration_ms));
            (call.function.name.clone(), outcome)
        })
        .collect();
    if tools.len() > PROGRESS_TOOLS {
        tools.drain(..tools.len() - PROGRESS_TOOLS);
    }
    let assistant = if live.draft.trim().is_empty() {
        new.iter()
            .rev()
            .find(|message| message.role == wisp_llm::Role::Assistant)
            .map(|message| message.content.as_text())
            .unwrap_or_default()
    } else {
        live.draft.clone()
    };
    TurnProgress {
        active: !new.is_empty() || !live.draft.is_empty() || !live.reasoning.is_empty(),
        assistant,
        tools,
    }
}

/// A tool waiting for an answer, as a chat is told about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingApproval {
    pub approval_id: String,
    pub frame_id: String,
    pub tool: String,
    pub message: String,
    pub preview: String,
}

fn approval_code(approval_id: &str) -> String {
    approval_id
        .chars()
        .take(APPROVAL_CODE_CHARS)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn approval_preview(request: &PendingApproval) -> String {
    let raw = if request.preview.trim().is_empty() {
        request.message.trim()
    } else {
        request.preview.trim()
    };
    if raw.chars().count() <= APPROVAL_PREVIEW_MAX_CHARS {
        raw.to_owned()
    } else {
        format!(
            "{}\n……(审批详情过长已截断)",
            raw.chars()
                .take(APPROVAL_PREVIEW_MAX_CHARS)
                .collect::<String>()
        )
    }
}

pub(crate) fn render_approval_request(request: &PendingApproval) -> String {
    let code = approval_code(&request.approval_id);
    let tool = match request.tool.trim() {
        "" => "操作",
        tool => tool,
    };
    let preview = approval_preview(request);
    let detail = if preview.is_empty() {
        String::new()
    } else {
        format!("\n详情:\n{preview}\n")
    };
    format!(
        "⚠️ Wisp 等待审批\n\n编号: {code}\n会话: {}\n工具: {tool}\n{detail}\n回复:\n/approve {code}\n/reject {code} 原因",
        short_id(&request.frame_id)
    )
}

/// The chats' target and each project's latest conversation. A target
/// project with no conversation is an explicit `/project` or `/new`: the
/// next ordinary message starts one there.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct SharedRoute {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    last_session_by_project: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    id: String,
    label: String,
}

/// Picks by number (from 1), id, exact label, or a unique id prefix.
fn select_choice<'a>(
    choices: &'a [Choice],
    selector: &str,
    kind: &str,
) -> Result<&'a Choice, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(format!("请提供{kind}序号、名称或 ID。"));
    }
    if let Ok(number) = selector.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|index| choices.get(index))
            .ok_or_else(|| format!("没有序号为 {number} 的{kind}。"));
    }
    if let Some(choice) = choices
        .iter()
        .find(|choice| choice.id.eq_ignore_ascii_case(selector))
    {
        return Ok(choice);
    }
    let labelled: Vec<&Choice> = choices
        .iter()
        .filter(|choice| choice.label.eq_ignore_ascii_case(selector))
        .collect();
    match labelled.as_slice() {
        [choice] => return Ok(choice),
        [_, _, ..] => return Err(format!("找到多个同名{kind}，请使用序号或 ID。")),
        _ => {}
    }
    let lower = selector.to_ascii_lowercase();
    let prefixed: Vec<&Choice> = choices
        .iter()
        .filter(|choice| choice.id.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match prefixed.as_slice() {
        [choice] => Ok(choice),
        [_, _, ..] => Err(format!("ID 前缀不唯一，请输入更长的{kind} ID。")),
        _ => Err(format!("未找到{kind}“{selector}”。")),
    }
}

fn format_list(choices: &[Choice], current: Option<&str>, project: bool) -> String {
    let (empty, title, limit, more, hint) = if project {
        (
            "还没有可用项目，请先在桌面端创建项目。",
            "项目列表:",
            20,
            "个项目未显示",
            "发送 /project <序号|名称|ID> 切换项目。",
        )
    } else {
        (
            "当前项目还没有已有会话。发送普通消息即可创建，或发送 /new。",
            "最近会话:",
            10,
            "个会话未显示",
            "发送 /session <序号|标题|ID> 切换，/new 开启新会话。",
        )
    };
    if choices.is_empty() {
        return empty.into();
    }
    let mut lines = vec![title.to_owned()];
    for (index, choice) in choices.iter().take(limit).enumerate() {
        let marker = if current == Some(choice.id.as_str()) {
            " ← 当前"
        } else {
            ""
        };
        lines.push(format!(
            "{}. {} · {}{marker}",
            index + 1,
            choice.label,
            short_id(&choice.id)
        ));
    }
    if choices.len() > limit {
        lines.push(format!("…另有 {} {more}", choices.len() - limit));
    }
    lines.push(hint.into());
    lines.join("\n")
}

/// The turn's answer: the last `attempt_completion` result or assistant
/// text after the latest message from the user.
pub(crate) fn final_answer(messages: &[wisp_llm::Message]) -> Option<String> {
    use wisp_llm::Role;
    let start = messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .map_or(0, |at| at + 1);
    messages[start..]
        .iter()
        .rev()
        .filter(|message| {
            message.role == Role::Assistant
                || (message.role == Role::Tool
                    && message.tool_name.as_deref() == Some("attempt_completion"))
        })
        .map(|message| message.content.as_text())
        .find(|text| !text.trim().is_empty())
}

const HELP_TEXT: &str = "可用命令:\n/status — 查看 IM 目标项目和会话\n/project — 列出项目\n/project <序号|名称|ID> — 切换 IM 目标项目\n/session — 列出当前 IM 项目的最近会话\n/session <序号|标题|ID> — 切换 IM 会话\n/new — 在当前 IM 项目开启新会话\n/approval — 查看待审批请求（微信）\n/approve <编号> — 批准一次（微信）\n/reject <编号> [原因] — 拒绝（微信）\n/stop — 停止当前任务\n/help — 显示本帮助\n\n微信和飞书共用同一个 IM 目标项目。桌面端在某个项目里发言只会更新该项目的最近会话，不会把 IM 目标抢走。/project、/session 和 /new 会显式切换这个 IM 目标。";
const TEXT_APPROVALS_ONLY: &str = "文本审批命令目前仅支持绑定 owner 的微信一对一会话。";
const NO_PROJECT: &str = "还没有 IM 目标项目。请先发送 /project 选择。";

impl Server {
    async fn route_unlocked(&self) -> SharedRoute {
        serde_json::from_str(&get_setting(&self.0.store, ROUTE_KEY).await).unwrap_or_default()
    }

    async fn set_route_unlocked(&self, route: &SharedRoute) -> Result<()> {
        self.0
            .store
            .set_setting(ROUTE_KEY, &serde_json::to_string(route)?)
            .await
    }

    /// The target with conversations and projects that are gone dropped.
    async fn validated_route_unlocked(&self) -> Result<SharedRoute> {
        let store = &self.0.store;
        let original = self.route_unlocked().await;
        let mut route = original.clone();
        if let Some(session) = route.session_id.clone() {
            match store.frame_project_id(&session).await? {
                Some(owner) => {
                    route.project_id = Some(owner.clone());
                    route
                        .last_session_by_project
                        .entry(owner)
                        .or_insert(session);
                }
                None => {
                    route.session_id = None;
                    if let Some(project) = route.project_id.clone() {
                        if let Some(fallback) = route.last_session_by_project.get(&project).cloned()
                        {
                            if store.frame_project_id(&fallback).await?.as_deref() == Some(&project)
                            {
                                route.session_id = Some(fallback);
                            } else {
                                route.last_session_by_project.remove(&project);
                            }
                        }
                    }
                }
            }
        }
        if route.session_id.is_none() {
            if let Some(project) = route.project_id.as_deref() {
                if store.get_project(project).await?.is_none() {
                    route.project_id = None;
                }
            }
        }
        if route != original {
            self.set_route_unlocked(&route).await?;
        }
        Ok(route)
    }

    async fn validated_route(&self) -> Result<SharedRoute> {
        let _route = self.0.channels.route.lock().await;
        self.validated_route_unlocked().await
    }

    async fn set_route(&self, route: &SharedRoute) -> Result<()> {
        let _route = self.0.channels.route.lock().await;
        self.set_route_unlocked(route).await
    }

    /// Every accepted send records its project's latest conversation.
    pub(crate) async fn record_last_message_session(&self, frame: &str) -> Result<()> {
        let _route = self.0.channels.route.lock().await;
        let project = self
            .0
            .store
            .frame_project_id(frame)
            .await?
            .context("The conversation no longer exists")?;
        if wisp_store::is_assistant_project_id(&project) {
            return Ok(());
        }
        let mut route = self.validated_route_unlocked().await?;
        route.last_session_by_project.insert(project, frame.into());
        self.set_route_unlocked(&route).await
    }

    /// The conversation an ordinary chat message goes to; one is started
    /// in the target project when there is none, before the turn, so a
    /// turn that fails is not retried into yet another conversation.
    async fn message_session(&self) -> Result<(String, String), String> {
        let _route = self.0.channels.route.lock().await;
        let mut route = self
            .validated_route_unlocked()
            .await
            .map_err(|error| format!("{error:#}"))?;
        let Some(project) = route.project_id.clone() else {
            return Err(NO_PROJECT.into());
        };
        if route.session_id.is_none() {
            let session = self
                .create_conversation(&project)
                .await
                .map_err(|error| format!("创建会话失败: {error:#}"))?;
            route
                .last_session_by_project
                .insert(project.clone(), session.clone());
            route.session_id = Some(session);
            self.set_route_unlocked(&route)
                .await
                .map_err(|error| format!("保存共享路由失败: {error:#}"))?;
        }
        Ok((project, route.session_id.unwrap_or_default()))
    }

    async fn project_choices(&self) -> Result<Vec<Choice>> {
        Ok(self
            .0
            .store
            .list_projects()
            .await?
            .into_iter()
            .filter(|(id, ..)| !wisp_store::is_assistant_project_id(id))
            .map(|(id, name, ..)| Choice {
                id,
                label: if name.trim().is_empty() {
                    "未命名项目".into()
                } else {
                    name
                },
            })
            .collect())
    }

    async fn session_choices(&self, project: &str) -> Result<Vec<Choice>> {
        Ok(self
            .0
            .store
            .list_sessions_page(project, None, usize::MAX)
            .await?
            .into_iter()
            .map(|(id, title, ..)| Choice { id, label: title })
            .collect())
    }

    async fn project_name(&self, project: &str) -> String {
        self.0
            .store
            .get_project(project)
            .await
            .ok()
            .flatten()
            .map(|(name, _)| name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| short_id(project))
    }

    async fn route_status_text(&self, route: &SharedRoute) -> String {
        let Some(project) = route.project_id.as_deref() else {
            return "还没有 IM 目标项目。下一条普通消息前请先发送 /project 选择。".into();
        };
        let name = self.project_name(project).await;
        let session = match route.session_id.as_deref() {
            Some(session) => {
                let title = self
                    .0
                    .store
                    .get_session_reference(session)
                    .await
                    .ok()
                    .flatten()
                    .map(|item| item.title)
                    .unwrap_or_else(|| "新会话".into());
                format!("{title} · {}", short_id(session))
            }
            None => "新会话（下一条普通消息创建）".into(),
        };
        format!("IM 目标项目: {name}\n最近 IM 会话: {session}")
    }

    /// Tools waiting for an answer in every conversation, oldest code first.
    pub(crate) fn pending_approvals(&self) -> Vec<PendingApproval> {
        let sessions: Vec<(String, Arc<crate::output::Session>)> = self
            .0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        let mut pending: Vec<PendingApproval> = sessions
            .iter()
            .flat_map(|(frame, session)| {
                let live = session.live.lock().unwrap();
                live.approvals
                    .iter()
                    .map(|(id, (request, _))| PendingApproval {
                        approval_id: id.clone(),
                        frame_id: frame.clone(),
                        tool: request.tool.clone(),
                        message: request.message.clone(),
                        preview: request.preview.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        pending.sort_by(|a, b| a.approval_id.cmp(&b.approval_id));
        pending
    }

    fn render_pending_approvals(&self) -> String {
        let pending = self.pending_approvals();
        if pending.is_empty() {
            return "当前没有待审批请求。".into();
        }
        let mut text = format!("当前有 {} 个待审批请求:\n", pending.len());
        for request in pending {
            text.push('\n');
            text.push_str(&render_approval_request(&request));
            text.push('\n');
        }
        truncate_reply(text.trim_end(), REPLY_MAX_CHARS)
    }

    /// Answers the one pending approval whose id starts with `selector`.
    fn respond_text_approval(
        &self,
        selector: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> Result<PendingApproval, String> {
        let selector = selector.trim().to_ascii_lowercase();
        if selector.len() < 6 || !selector.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("审批编号至少需要 6 位十六进制字符。".into());
        }
        let matches: Vec<PendingApproval> = self
            .pending_approvals()
            .into_iter()
            .filter(|request| request.approval_id.starts_with(&selector))
            .collect();
        let request = match matches.as_slice() {
            [] => return Err("未找到该待审批请求；它可能已经处理或失效。".into()),
            [request] => request.clone(),
            _ => return Err("审批编号前缀不唯一，请输入更多位。".into()),
        };
        let session = self.session(&request.frame_id);
        let sender = session
            .live
            .lock()
            .unwrap()
            .approvals
            .remove(&request.approval_id)
            .map(|(_, sender)| sender)
            .ok_or_else(|| "未找到该待审批请求；它可能已经处理或失效。".to_string())?;
        sender
            .send(if approved {
                wisp_tools::ConfirmDecision::Approved
            } else {
                wisp_tools::ConfirmDecision::Denied { feedback }
            })
            .map_err(|_| "审批已失效。".to_string())?;
        Ok(request)
    }

    /// One text from a chat: a command is answered here, anything else is
    /// sent to the target conversation. Returns the reply, which may be
    /// empty. What the turn asks for and does meanwhile goes to `events`.
    pub(crate) async fn handle_inbound(
        &self,
        channel: Channel,
        chat: &str,
        text: &str,
        events: Option<mpsc::UnboundedSender<ChatEvent>>,
    ) -> String {
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        let lock = self
            .0
            .channels
            .chat_lock(&format!("{}:{chat}", channel.key()));
        let guard = lock.lock().await;
        let mut parts = text.splitn(2, char::is_whitespace);
        let command = parts.next().unwrap_or_default().to_ascii_lowercase();
        let argument = parts.next().unwrap_or_default().trim();
        let failed = |what: &str, error: anyhow::Error| format!("{what}: {error:#}");
        match command.as_str() {
            "/help" => return HELP_TEXT.into(),
            "/approval" | "/approvals" => {
                if channel != Channel::Weixin {
                    return TEXT_APPROVALS_ONLY.into();
                }
                return self.render_pending_approvals();
            }
            "/approve" => {
                if channel != Channel::Weixin {
                    return TEXT_APPROVALS_ONLY.into();
                }
                let mut words = argument.split_whitespace();
                let Some(selector) = words.next() else {
                    return "用法: /approve <审批编号>".into();
                };
                if words.next().is_some() {
                    return "用法: /approve <审批编号>。微信审批目前只允许本次。".into();
                }
                return match self.respond_text_approval(selector, true, None) {
                    Ok(resolved) => format!(
                        "已批准 {}，会话 {} 将继续执行。",
                        approval_code(&resolved.approval_id),
                        short_id(&resolved.frame_id)
                    ),
                    Err(error) => format!("批准失败: {error}"),
                };
            }
            "/reject" => {
                if channel != Channel::Weixin {
                    return TEXT_APPROVALS_ONLY.into();
                }
                let mut words = argument.splitn(2, char::is_whitespace);
                let selector = words.next().unwrap_or_default().trim();
                if selector.is_empty() {
                    return "用法: /reject <审批编号> [原因]".into();
                }
                let feedback = words
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                return match self.respond_text_approval(selector, false, feedback) {
                    Ok(resolved) => format!(
                        "已拒绝 {}，会话 {} 将收到拒绝结果。",
                        approval_code(&resolved.approval_id),
                        short_id(&resolved.frame_id)
                    ),
                    Err(error) => format!("拒绝失败: {error}"),
                };
            }
            "/status" => {
                return match self.validated_route().await {
                    Ok(route) => self.route_status_text(&route).await,
                    Err(error) => failed("读取共享路由失败", error),
                };
            }
            "/project" | "/projects" => {
                let mut route = match self.validated_route().await {
                    Ok(route) => route,
                    Err(error) => return failed("读取共享路由失败", error),
                };
                let choices = match self.project_choices().await {
                    Ok(choices) => choices,
                    Err(error) => return failed("读取项目失败", error),
                };
                if argument.is_empty() {
                    return format_list(&choices, route.project_id.as_deref(), true);
                }
                let selected = match select_choice(&choices, argument, "项目") {
                    Ok(selected) => selected,
                    Err(error) => return error,
                };
                route.project_id = Some(selected.id.clone());
                route.session_id = None;
                if let Err(error) = self.set_route(&route).await {
                    return failed("切换共享路由失败", error);
                }
                return format!(
                    "IM 目标已切换到项目“{}”。下一条微信或飞书普通消息会在这里创建新会话；也可发送 /session 选择已有会话。",
                    selected.label
                );
            }
            "/session" | "/sessions" | "/new" => {
                let mut route = match self.validated_route().await {
                    Ok(route) => route,
                    Err(error) => return failed("读取共享路由失败", error),
                };
                let Some(project) = route.project_id.clone() else {
                    return NO_PROJECT.into();
                };
                if command == "/new" || argument.eq_ignore_ascii_case("new") {
                    route.session_id = None;
                    let name = self.project_name(&project).await;
                    if let Err(error) = self.set_route(&route).await {
                        return failed("切换共享路由失败", error);
                    }
                    return format!(
                        "已准备在项目“{name}”中开启共享的新会话。请从微信或飞书发送下一条普通消息。"
                    );
                }
                let choices = match self.session_choices(&project).await {
                    Ok(choices) => choices,
                    Err(error) => return failed("读取会话失败", error),
                };
                if argument.is_empty() {
                    let name = self.project_name(&project).await;
                    return format!(
                        "项目: {name}\n{}",
                        format_list(&choices, route.session_id.as_deref(), false)
                    );
                }
                let selected = match select_choice(&choices, argument, "会话") {
                    Ok(selected) => selected,
                    Err(error) => return error,
                };
                route.session_id = Some(selected.id.clone());
                if let Err(error) = self.set_route(&route).await {
                    return failed("切换共享路由失败", error);
                }
                return format!(
                    "IM 目标已切换到会话“{}” · {}。后续微信和飞书消息都会继续这个会话。",
                    selected.label,
                    short_id(&selected.id)
                );
            }
            "/stop" => {
                let route = match self.validated_route().await {
                    Ok(route) => route,
                    Err(error) => return failed("读取共享路由失败", error),
                };
                let Some(session) = route.session_id else {
                    return "当前没有最近发言会话。".into();
                };
                self.session(&session).stop();
                return "已请求停止当前任务。".into();
            }
            _ => {}
        }
        if text.starts_with('/') {
            return format!("未知命令“{command}”。发送 /help 查看可用命令。");
        }
        let (project, session) = match self.message_session().await {
            Ok(target) => target,
            Err(error) => return format!("路由消息失败: {error}"),
        };
        // The long turn runs without the chat's lock, so /stop gets through.
        drop(guard);
        match self
            .chat_turn(&project, &session, text, events, TurnOrigin::Chat)
            .await
        {
            Ok(reply) => reply,
            Err(error) => format!("处理失败:{error:#}"),
        }
    }

    /// Runs one turn for a chat or a lab mail and returns its answer.
    pub(crate) async fn chat_turn(
        &self,
        project: &str,
        session: &str,
        text: &str,
        events: Option<mpsc::UnboundedSender<ChatEvent>>,
        origin: TurnOrigin,
    ) -> Result<String> {
        let root = self.project_root(project).await?;
        let request = wisp_dto::native_conversations::SendRequest {
            session_id: session.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message: text.into(),
            resume: false,
            resume_after: None,
            attachments: Vec::new(),
            references: Vec::new(),
            acp_agent_id: None,
        };
        let live = self.session(session);
        self.start_turn(project, &root, request, origin).await?;
        // The history the turn started from; what follows is this turn's.
        let from = live.live.lock().unwrap().messages.len();
        let mut told = std::collections::HashSet::new();
        let mut shown = TurnProgress::default();
        let error = loop {
            let (running, error, progress, waiting) = {
                let live = live.live.lock().unwrap();
                (
                    live.running,
                    live.error.clone(),
                    progress_of(&live, from),
                    live.approvals
                        .iter()
                        .filter(|(id, _)| !told.contains(*id))
                        .map(|(id, (request, _))| PendingApproval {
                            approval_id: id.clone(),
                            frame_id: session.into(),
                            tool: request.tool.clone(),
                            message: request.message.clone(),
                            preview: request.preview.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
            };
            for request in waiting {
                told.insert(request.approval_id.clone());
                if let Some(events) = &events {
                    let _ = events.send(ChatEvent::Approval(render_approval_request(&request)));
                }
            }
            if running && progress != shown {
                if let Some(events) = &events {
                    let _ = events.send(ChatEvent::Progress(progress.clone()));
                }
                shown = progress;
            }
            if !running {
                break error;
            }
            tokio::time::sleep(TURN_POLL).await;
        };
        if let Some(error) = error {
            anyhow::bail!(error);
        }
        let messages = self.0.store.load_messages(session).await?;
        Ok(match final_answer(&messages) {
            Some(answer) => truncate_reply(&answer, REPLY_MAX_CHARS),
            None => "(本轮完成,但没有文本回复)".into(),
        })
    }

    /// Starts the channels that were left on. The desktop calls this once
    /// it has opened the host; a headless server does not.
    pub async fn start_channels(&self) {
        if get_setting(&self.0.store, feishu::ENABLED).await == "true" {
            self.start_feishu().await;
        }
        if self.is_lab_member().await {
            self.start_lab();
        }
        if get_setting(&self.0.store, weixin::ENABLED).await == "true" {
            let _config = self.0.channels.weixin_config.lock().await;
            self.start_weixin().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Vec<Choice> {
        [
            ("a1b2c3d4-0000", "Alpha"),
            ("a1b2ffff-0000", "Beta"),
            ("c0ffee00-0000", "Beta"),
        ]
        .into_iter()
        .map(|(id, label)| Choice {
            id: id.into(),
            label: label.into(),
        })
        .collect()
    }

    #[test]
    fn choices_are_picked_by_number_label_id_or_unique_prefix() {
        let choices = choices();
        assert_eq!(select_choice(&choices, "1", "项目").unwrap().label, "Alpha");
        assert!(select_choice(&choices, "4", "项目")
            .unwrap_err()
            .contains('4'));
        assert_eq!(
            select_choice(&choices, "alpha", "项目").unwrap().id,
            "a1b2c3d4-0000"
        );
        // Two of that name: the number or id has to be used.
        assert!(select_choice(&choices, "Beta", "项目")
            .unwrap_err()
            .contains("多个"));
        assert_eq!(
            select_choice(&choices, "c0ff", "项目").unwrap().id,
            "c0ffee00-0000"
        );
        assert!(select_choice(&choices, "a1b2", "项目")
            .unwrap_err()
            .contains("不唯一"));
        assert!(select_choice(&choices, "zzz", "项目").is_err());
        assert!(select_choice(&choices, " ", "项目").is_err());
    }

    #[test]
    fn lists_mark_the_current_choice_and_say_how_to_switch() {
        let list = format_list(&choices(), Some("c0ffee00-0000"), true);
        assert!(list.starts_with("项目列表:"));
        assert!(list.contains("3. Beta · c0ffee00 ← 当前"));
        assert!(list.ends_with("发送 /project <序号|名称|ID> 切换项目。"));
        assert!(format_list(&[], None, false).contains("/new"));
    }

    #[test]
    fn approvals_read_with_a_short_code_and_how_to_answer() {
        let request = PendingApproval {
            approval_id: "1a2b3c4d-5e6f-0000-0000-000000000000".into(),
            frame_id: "frame-123456789".into(),
            tool: "write".into(),
            message: "Run tool 'write'?".into(),
            preview: "x".repeat(APPROVAL_PREVIEW_MAX_CHARS + 5),
        };
        let text = render_approval_request(&request);
        assert!(text.contains("编号: 1A2B3C4D"));
        assert!(text.contains("会话: frame-12"));
        assert!(text.contains("工具: write"));
        assert!(text.contains("/approve 1A2B3C4D"));
        assert!(text.contains("审批详情过长已截断"));
    }

    #[test]
    fn long_replies_are_cut_with_a_note() {
        assert_eq!(truncate_reply("short", 10), "short");
        let cut = truncate_reply(&"字".repeat(12), 10);
        assert!(cut.starts_with(&"字".repeat(10)));
        assert!(cut.contains("已截断"));
    }

    #[test]
    fn the_answer_is_from_the_latest_turn_only() {
        use wisp_llm::Message;
        let mut messages = vec![
            Message::user("first"),
            Message::assistant("old answer"),
            Message::user("second"),
        ];
        // Nothing yet in this turn: the old answer is not repeated.
        assert_eq!(final_answer(&messages), None);
        messages.push(Message::assistant(""));
        messages.push(Message::tool("c1", "attempt_completion", "There are 100."));
        assert_eq!(final_answer(&messages).as_deref(), Some("There are 100."));
    }
}
