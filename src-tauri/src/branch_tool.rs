//! Agent-opened conversation branches (话中话): a conversation's agent asks
//! for a branch of its own conversation. The branch is an ordinary `/fork`
//! branch (`session_commands::create_branch_from`) cut at the end of the turn
//! that asked for it, nested under this conversation in the sidebar, and
//! handed to the researcher with the agent's brief as its composer draft.
//! Nothing else connects the two: the parent's agent never writes to the
//! branch, is never woken by it, and the branch's permissions are the
//! researcher's, as for any branch. That is the line between this and a
//! subagent (`subagent_tool`), which gets no transcript, is driven by the
//! parent's agent and runs under the parent turn's captured authorization.
//!
//! The tool only records an intent; `realize_pending` runs from the AfterTurn
//! hook once the turn's final messages are persisted, so the branch holds the
//! whole turn, including the reply that announced it.

use crate::session_commands::{
    create_branch_from, last_user_index, require_branchable, BranchCheckpointKind,
    BranchPermissions,
};
use crate::{AgentEvent, AppState};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub(crate) const OPEN_BRANCH_TOOL: &str = "open_branch";
/// Matches `branch_title`'s cap, so the recorded title is the shown title.
const MAX_TITLE_CHARS: usize = 64;
const MAX_BRIEF_CHARS: usize = 4_000;
/// One per concern is the intended use; a turn asking for more is confused.
const MAX_INTENTS_PER_TURN: usize = 3;

/// One `open_branch` call made during a turn, realized after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchIntent {
    pub(crate) title: String,
    pub(crate) brief: String,
    /// Visual user turn the call was made in. Pinning it here keeps the
    /// checkpoint at this turn's end even if a queued turn has started by the
    /// time the branch is built.
    pub(crate) user_index: usize,
}

/// Intents recorded during each conversation's running turn, taken when that
/// turn ends (`turn_hooks::spawn_after_turn`).
#[derive(Default)]
pub(crate) struct BranchIntentRegistry(Mutex<HashMap<String, Vec<BranchIntent>>>);

impl BranchIntentRegistry {
    pub(crate) fn record(&self, frame_id: &str, intent: BranchIntent) -> Result<(), String> {
        let mut intents = self.0.lock().unwrap();
        let pending = intents.entry(frame_id.to_string()).or_default();
        if pending.len() >= MAX_INTENTS_PER_TURN {
            return Err(format!(
                "This turn already opens {MAX_INTENTS_PER_TURN} branches; finish it before opening more."
            ));
        }
        pending.push(intent);
        Ok(())
    }

    pub(crate) fn take(&self, frame_id: &str) -> Vec<BranchIntent> {
        self.0.lock().unwrap().remove(frame_id).unwrap_or_default()
    }
}

/// The `open_branch` tool of one ordinary conversation.
pub(crate) struct OpenBranchTool {
    store: Store,
    intents: Arc<BranchIntentRegistry>,
    frame_id: String,
}

impl OpenBranchTool {
    pub(crate) fn new(
        store: Store,
        intents: Arc<BranchIntentRegistry>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            intents,
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for OpenBranchTool {
    fn name(&self) -> &str {
        OPEN_BRANCH_TOOL
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            OPEN_BRANCH_TOOL,
            "Open a branch of this conversation for the researcher to continue: a new conversation listed under this one in the sidebar that starts with a copy of this conversation up to and including your reply in this turn, with your brief pre-filled in its composer. Use it when a line of work deserves its own conversation (an alternative approach, a tangent, a long follow-up) but needs this context to start from, and the researcher should drive it. You do not run it: the branch is created after this turn ends, its own agent and the researcher continue it, and nothing comes back to you. Prefer dispatch_subagent when you must run the work yourself and act on its result. Call it at most once per concern, then finish your turn and tell the researcher the branch will appear; do not wait or poll for it.",
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short conversation title (up to 64 characters)."
                    },
                    "brief": {
                        "type": "string",
                        "description": "What the branch should pursue and why, written for the researcher and the branch's agent, who both see this conversation's history. It becomes the branch's composer draft; the researcher edits and sends it."
                    },
                    "context": {
                        "type": "string",
                        "enum": ["prefix"],
                        "description": "What the branch starts with. 'prefix' (the default and the only option for now): this conversation's history through this turn."
                    },
                    "autostart": {
                        "type": "boolean",
                        "description": "Whether the branch's agent starts on the brief by itself. Only false is supported for now: the researcher reviews and sends the brief."
                    }
                },
                "required": ["title", "brief"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("title")
            .and_then(Value::as_str)
            .map(|text| text.trim().chars().take(MAX_TITLE_CHARS).collect())
            .unwrap_or_default()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let (title, brief) = match parse_args(args) {
            Ok(parsed) => parsed,
            Err(error) => return ToolResult::fail(error),
        };
        if let Err(error) = require_branchable(&self.store, &self.frame_id).await {
            return ToolResult::fail(error);
        }
        let user_index = match last_user_index(&self.store, &self.frame_id).await {
            Ok(index) => index,
            Err(error) => return ToolResult::fail(error),
        };
        let intent = BranchIntent {
            title,
            brief,
            user_index,
        };
        if let Err(error) = self.intents.record(&self.frame_id, intent.clone()) {
            return ToolResult::fail(error);
        }
        ToolResult::ok(scheduled_text(&intent.title))
    }
}

/// `(title, brief)` once the arguments describe a branch this version builds.
fn parse_args(args: &Value) -> Result<(String, String), String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("'{key}' cannot be empty"))
    };
    let title = text("title")?
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect::<String>();
    let brief = text("brief")?;
    if brief.chars().count() > MAX_BRIEF_CHARS {
        return Err(format!(
            "'brief' is too long (over {MAX_BRIEF_CHARS} characters); the branch sees this conversation's history, so keep the brief to the goal and what differs."
        ));
    }
    if let Some(context) = args.get("context").and_then(Value::as_str) {
        if context != "prefix" {
            return Err(format!(
                "context '{context}' is not supported yet; only 'prefix' (this conversation's history through this turn) is."
            ));
        }
    }
    if args.get("autostart").and_then(Value::as_bool) == Some(true) {
        return Err(
            "autostart true is not supported yet: the researcher reviews and sends the brief. Call again with autostart false or omitted."
                .into(),
        );
    }
    Ok((title, brief.to_string()))
}

/// What the model is told once an intent is recorded. The wording closes the
/// loop on purpose: no waiting, no polling, no further role.
fn scheduled_text(title: &str) -> String {
    format!(
        "Branch \"{title}\" is scheduled. It is created when this turn ends, from this conversation's history through your reply in this turn, and appears under this conversation in the sidebar with your brief as its composer draft. The researcher continues it there; nothing from it returns to you. Do not wait, poll or check for it: finish this turn and tell the researcher the branch will appear."
    )
}

/// Build the branch one intent describes: an `after_response` `/fork` at the
/// turn the call was made in, titled by the agent.
pub(crate) async fn create_from_intent(
    store: &Store,
    permissions: BranchPermissions<'_>,
    project_id: &str,
    parent: &str,
    intent: &BranchIntent,
) -> Result<String, String> {
    require_branchable(store, parent).await?;
    create_branch_from(
        store,
        permissions,
        project_id,
        parent,
        Some(intent.user_index),
        BranchCheckpointKind::AfterResponse,
        Some(&intent.title),
    )
    .await
}

/// The notice appended to the parent conversation. It carries the brief, so
/// the hand-over survives even if the branch's unsent draft is lost.
fn opened_notice(locale: &str, title: &str, brief: &str) -> String {
    if locale == "zh" {
        format!(
            "已开启分支《{title}》。它从本轮结束处分出，列在侧栏本对话之下，输入框里已填好这份简报，由你继续推进：\n\n{brief}"
        )
    } else {
        format!(
            "Opened branch “{title}”. It forks from the end of this turn, is listed under this conversation in the sidebar, and its composer holds this brief for you to continue:\n\n{brief}"
        )
    }
}

/// AfterTurn: build every branch the finished turn asked for, tell the
/// project's windows (sidebar refresh plus the composer draft) and leave a
/// notice in the parent conversation. A failure is reported as a hook failure
/// and never touches the finished turn.
pub(crate) async fn realize_pending(
    app: &AppHandle,
    frame_id: &str,
    project_id: &str,
    intents: Vec<BranchIntent>,
) {
    let state = app.state::<AppState>();
    let locale = crate::load_locale(&state.store).await;
    for intent in intents {
        let created = create_from_intent(
            &state.store,
            BranchPermissions::of(&state),
            project_id,
            frame_id,
            &intent,
        )
        .await;
        match created {
            Ok(branch_id) => {
                crate::emit_agent_event_in(
                    app,
                    AgentEvent::BranchOpened {
                        frame_id: frame_id.to_string(),
                        branch_id,
                        title: intent.title.clone(),
                        brief: intent.brief.clone(),
                    },
                    Some(project_id),
                );
                let notice = opened_notice(&locale, &intent.title, &intent.brief);
                match crate::dispatch::post_reply(app, frame_id, &notice).await {
                    Ok(true) => {}
                    Ok(false) => tracing::info!(
                        "open_branch notice for {frame_id} dropped: the conversation is gone"
                    ),
                    Err(error) => {
                        tracing::warn!("open_branch notice for {frame_id} failed: {error}")
                    }
                }
            }
            Err(message) => {
                tracing::warn!("open_branch for {frame_id} failed: {message}");
                crate::emit_agent_event_in(
                    app,
                    AgentEvent::HookFailed {
                        frame_id: frame_id.to_string(),
                        hook: OPEN_BRANCH_TOOL.into(),
                        message,
                    },
                    Some(project_id),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApprovalGrants;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::RwLock;
    use wisp_llm::Message;

    struct NoEnv(PathBuf);
    #[async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    /// Append one completed visual turn (`user`, `assistant`) to `frame` the
    /// way a real turn persists it: messages plus the User/MessageBoundary UI
    /// events that checkpoints are mapped through.
    async fn persist_turn(store: &Store, frame: &str, text: &str) {
        let seq = store.max_message_seq(frame).await.unwrap() + 1;
        store
            .append_message(frame, seq, &Message::user(text))
            .await
            .unwrap();
        store
            .append_message(frame, seq + 1, &Message::assistant(format!("re {text}")))
            .await
            .unwrap();
        let mut event_seq = store.next_session_ui_event_seq(frame).await.unwrap();
        for event in [
            format!(r#"{{"kind":"User","frame_id":"{frame}","text":"{text}"}}"#),
            format!(r#"{{"kind":"MessageBoundary","frame_id":"{frame}","seq":{seq}}}"#),
            format!(r#"{{"kind":"Text","frame_id":"{frame}","delta":"re {text}"}}"#),
            format!(
                r#"{{"kind":"MessageBoundary","frame_id":"{frame}","seq":{}}}"#,
                seq + 1
            ),
        ] {
            store
                .append_session_ui_event(frame, event_seq, &event)
                .await
                .unwrap();
            event_seq += 1;
        }
    }

    async fn store_with_turns(dir: &Path, frame: &str, turns: &[&str]) -> Store {
        let store = Store::open(&dir.join("wisp.sqlite")).await.unwrap();
        store.create_project("p", "P", "").await.unwrap();
        store.create_frame(frame, "p", "OPERON", "m").await.unwrap();
        store
            .append_message(frame, 1, &Message::system("sys"))
            .await
            .unwrap();
        for text in turns {
            persist_turn(&store, frame, text).await;
        }
        store
    }

    fn texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .map(|message| message.content.as_text())
            .collect()
    }

    #[tokio::test]
    async fn schema_offers_prefix_context_and_no_autostart() {
        let registry = Arc::new(BranchIntentRegistry::default());
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        let tool = OpenBranchTool::new(store, registry, "main");
        assert_eq!(tool.name(), "open_branch");
        let schema = tool.schema();
        assert_eq!(schema.function.name, "open_branch");
        let parameters = &schema.function.parameters;
        assert_eq!(parameters["required"], json!(["title", "brief"]));
        assert_eq!(
            parameters["properties"]["context"]["enum"],
            json!(["prefix"])
        );
        assert_eq!(parameters["properties"]["autostart"]["type"], "boolean");
        assert!(schema
            .function
            .description
            .contains("do not wait or poll for it"));
        assert_eq!(
            tool.preview(&json!({"title": "  Try method B  ", "brief": "x"})),
            "Try method B"
        );
    }

    #[tokio::test]
    async fn open_branch_records_the_turn_it_was_called_in_and_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_turns(dir.path(), "main", &["q1", "q2"]).await;
        let registry = Arc::new(BranchIntentRegistry::default());
        let tool = OpenBranchTool::new(store.clone(), registry.clone(), "main");
        let env = NoEnv(dir.path().to_path_buf());

        let result = tool
            .run(
                &json!({"title": "Try method B", "brief": "Compare B.", "context": "prefix", "autostart": false}),
                &env,
            )
            .await;
        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("when this turn ends"));
        assert!(result.content.contains("Do not wait, poll or check"));
        // Nothing is created during the turn.
        assert!(store
            .list_session_branches("main", "p")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            registry.take("main"),
            vec![BranchIntent {
                title: "Try method B".into(),
                brief: "Compare B.".into(),
                user_index: 1,
            }]
        );
        // Taking an intent consumes it; the next turn starts clean.
        assert!(registry.take("main").is_empty());

        for (args, expected) in [
            (json!({"brief": "x"}), "'title' cannot be empty"),
            (
                json!({"title": "t", "brief": "  "}),
                "'brief' cannot be empty",
            ),
            (
                json!({"title": "t", "brief": "x", "context": "handoff"}),
                "context 'handoff' is not supported yet",
            ),
            (
                json!({"title": "t", "brief": "x", "autostart": true}),
                "autostart true is not supported yet",
            ),
            (
                json!({"title": "t", "brief": "x".repeat(MAX_BRIEF_CHARS + 1)}),
                "'brief' is too long",
            ),
        ] {
            let result = tool.run(&args, &env).await;
            assert!(!result.success, "{args}");
            assert!(result.content.contains(expected), "{}", result.content);
        }
        assert!(registry.take("main").is_empty());

        // A long title is cut to what the sidebar shows.
        let long = "t".repeat(MAX_TITLE_CHARS + 5);
        assert!(
            tool.run(&json!({"title": long, "brief": "x"}), &env)
                .await
                .success
        );
        assert_eq!(
            registry.take("main")[0].title.chars().count(),
            MAX_TITLE_CHARS
        );

        // A turn asking for too many branches is told to finish first.
        for _ in 0..MAX_INTENTS_PER_TURN {
            assert!(
                tool.run(&json!({"title": "t", "brief": "x"}), &env)
                    .await
                    .success
            );
        }
        let result = tool.run(&json!({"title": "t", "brief": "x"}), &env).await;
        assert!(!result.success);
        assert!(result.content.contains("finish it before opening more"));
        assert_eq!(registry.take("main").len(), MAX_INTENTS_PER_TURN);
    }

    #[tokio::test]
    async fn open_branch_refuses_from_a_branch_and_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_turns(dir.path(), "main", &["q1"]).await;
        let grants = Mutex::new(ApprovalGrants::default());
        let full_permission = RwLock::new(HashSet::new());
        let branch = create_from_intent(
            &store,
            BranchPermissions {
                approval_grants: &grants,
                full_permission_sessions: &full_permission,
            },
            "p",
            "main",
            &BranchIntent {
                title: "first".into(),
                brief: "x".into(),
                user_index: 0,
            },
        )
        .await
        .unwrap();
        let registry = Arc::new(BranchIntentRegistry::default());
        let tool = OpenBranchTool::new(store.clone(), registry.clone(), branch.clone());
        let result = tool
            .run(
                &json!({"title": "t", "brief": "x"}),
                &NoEnv(dir.path().to_path_buf()),
            )
            .await;
        assert!(!result.success);
        assert!(result
            .content
            .contains("Conversation branches cannot be branched again"));
        assert!(registry.take(&branch).is_empty());
    }

    #[tokio::test]
    async fn a_realized_intent_is_a_fork_at_the_end_of_its_own_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_turns(dir.path(), "main", &["q1", "q2"]).await;
        let registry = Arc::new(BranchIntentRegistry::default());
        let tool = OpenBranchTool::new(store.clone(), registry.clone(), "main");
        assert!(
            tool.run(
                &json!({"title": "Try method B", "brief": "Compare B."}),
                &NoEnv(dir.path().to_path_buf())
            )
            .await
            .success
        );
        // A queued follow-up may already have run by the time AfterTurn gets
        // to the intent; the branch still ends with the turn that asked.
        persist_turn(&store, "main", "q3").await;

        let grants = Mutex::new(ApprovalGrants::default());
        let full_permission = RwLock::new(HashSet::from(["main".to_string()]));
        let intents = registry.take("main");
        assert_eq!(intents.len(), 1);
        let branch = create_from_intent(
            &store,
            BranchPermissions {
                approval_grants: &grants,
                full_permission_sessions: &full_permission,
            },
            "p",
            "main",
            &intents[0],
        )
        .await
        .unwrap();

        assert_eq!(
            texts(&store.load_messages(&branch).await.unwrap()),
            ["sys", "q1", "re q1", "q2", "re q2"]
        );
        let links = store.list_session_branches("main", "p").await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].id, branch);
        assert_eq!(links[0].title, "Try method B");
        assert_eq!(links[0].checkpoint_user_index, 1);
        assert_eq!(links[0].checkpoint_kind, "after_response");
        // An ordinary branch: nobody's subagent, and the researcher's
        // permissions follow as for /fork.
        assert_eq!(store.session_dispatched_from(&branch).await.unwrap(), None);
        assert!(full_permission.read().unwrap().contains(&branch));
        assert_eq!(
            store.session_branch_state(&branch).await.unwrap(),
            Some("active")
        );
    }

    #[test]
    fn the_parent_notice_names_the_branch_and_carries_the_brief() {
        let zh = opened_notice("zh", "方法 B", "比较 B 与基线。");
        assert!(zh.starts_with("已开启分支《方法 B》"));
        assert!(zh.ends_with("比较 B 与基线。"));
        let en = opened_notice("en", "Method B", "Compare B with the baseline.");
        assert!(en.starts_with("Opened branch “Method B”"));
        assert!(en.ends_with("Compare B with the baseline."));
        assert!(scheduled_text("Method B").contains("\"Method B\""));
    }
}
