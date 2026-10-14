use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{mpsc, oneshot};
use wisp_core::{Output, OutputFuture};
use wisp_dto::PendingToolApproval;
use wisp_llm::Message;
use wisp_tools::{Approval, ConfirmDecision};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolOutcome {
    pub call_id: String,
    pub ok: bool,
    pub duration_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct OutcomeEvent {
    pub kind: String,
    pub outcome: ToolOutcome,
}

pub struct PersistedMessage {
    pub message: Message,
    pub outcome: Option<ToolOutcome>,
}

#[derive(Default)]
pub struct Live {
    pub running: bool,
    /// Changes when a turn starts or finishes, invalidating in-flight store reads.
    pub generation: u64,
    pub messages: Vec<Message>,
    pub next_before_seq: Option<i64>,
    pub draft: String,
    pub error: Option<String>,
    pub tool: String,
    pub preview: String,
    pub last_result: Option<(bool, u64)>,
    pub outcomes: HashMap<String, ToolOutcome>,
    pub approvals: HashMap<String, (PendingToolApproval, oneshot::Sender<ConfirmDecision>)>,
}

pub struct Session {
    pub live: Mutex<Live>,
    pub cancel: Arc<AtomicBool>,
    /// Serializes acceptance, including the durable request-id check.
    pub accept: tokio::sync::Mutex<()>,
    pub runtime: Mutex<Option<wisp_runtime::RuntimeManager>>,
    #[cfg(test)]
    pub snapshot_after_read: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            live: Mutex::new(Live::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            accept: Default::default(),
            runtime: Mutex::new(None),
            #[cfg(test)]
            snapshot_after_read: Mutex::new(None),
        }
    }
}

impl Session {
    pub fn stop(&self) {
        let mut live = self.live.lock().unwrap();
        self.cancel.store(true, Ordering::Relaxed);
        for (_, (_, sender)) in live.approvals.drain() {
            let _ = sender.send(ConfirmDecision::Denied {
                feedback: Some("Turn stopped".into()),
            });
        }
    }
}

pub struct ServerOutput {
    pub session: Arc<Session>,
    pub frame: String,
    pub project: String,
    pub turn: String,
    pub persist: mpsc::UnboundedSender<PersistedMessage>,
}

impl Output for ServerOutput {
    fn pre_tool_use<'a>(
        &'a self,
        tool: &'a str,
        args: &'a serde_json::Value,
    ) -> OutputFuture<'a, wisp_core::PreToolDecision> {
        Box::pin(async move {
            let mut live = self.session.live.lock().unwrap();
            live.tool = tool.into();
            live.preview = serde_json::to_string_pretty(args).unwrap_or_default();
            wisp_core::PreToolDecision::Continue
        })
    }
    fn assistant_text(&self, delta: &str) {
        self.session.live.lock().unwrap().draft.push_str(delta);
    }
    fn tool_result(&self, _: &str, ok: bool, _: &str, duration_ms: u64) {
        self.session.live.lock().unwrap().last_result = Some((ok, duration_ms));
    }
    fn on_message(&self, msg: &Message) {
        let mut live = self.session.live.lock().unwrap();
        if msg.role == wisp_llm::Role::Assistant {
            live.draft.clear();
        }
        live.messages.push(msg.clone());
        let outcome = if msg.role == wisp_llm::Role::Tool {
            live.last_result.take().and_then(|(ok, duration_ms)| {
                msg.tool_call_id.as_ref().map(|id| ToolOutcome {
                    call_id: id.clone(),
                    ok,
                    duration_ms,
                })
            })
        } else {
            None
        };
        if let Some(outcome) = &outcome {
            live.outcomes
                .insert(outcome.call_id.clone(), outcome.clone());
        }
        if self
            .persist
            .send(PersistedMessage {
                message: msg.clone(),
                outcome,
            })
            .is_err()
        {
            self.session.cancel.store(true, Ordering::Relaxed);
            live.error = Some("Conversation could not be saved; stopping the turn".into());
        }
    }
    fn approval_mode(&self, _: &str) -> Approval {
        Approval::Allow
    }
    fn force_ask_mutations(&self) -> bool {
        true
    }
    fn confirm(&self, _: &str) -> bool {
        false
    }
    fn confirm_async<'a>(&'a self, message: &'a str) -> OutputFuture<'a, bool> {
        Box::pin(async move { self.confirm_decision_async(message).await.approved() })
    }
    fn confirm_decision_async<'a>(&'a self, message: &'a str) -> OutputFuture<'a, ConfirmDecision> {
        Box::pin(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let (sender, receiver) = oneshot::channel();
            {
                let mut live = self.session.live.lock().unwrap();
                if self.session.cancel.load(Ordering::Relaxed) {
                    return ConfirmDecision::Denied {
                        feedback: Some("Turn stopped".into()),
                    };
                }
                let tool = live.tool.clone();
                let preview = live.preview.clone();
                live.approvals.insert(
                    id.clone(),
                    (
                        PendingToolApproval {
                            approval_id: id.clone(),
                            frame_id: self.frame.clone(),
                            message: message.into(),
                            tool,
                            preview,
                        },
                        sender,
                    ),
                );
            }
            let result = receiver.await.unwrap_or(ConfirmDecision::Denied {
                feedback: Some("Approval expired".into()),
            });
            self.session.live.lock().unwrap().approvals.remove(&id);
            result
        })
    }
    fn frame_id(&self) -> Option<&str> {
        Some(&self.frame)
    }
    fn project_id(&self) -> Option<&str> {
        Some(&self.project)
    }
    fn turn_id(&self) -> Option<&str> {
        Some(&self.turn)
    }
    fn provenance_scope(&self) -> Option<String> {
        Some(self.frame.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stop_releases_approval_waiters_and_prevents_new_approvals() {
        let session = Arc::new(Session::default());
        let (tx, _rx) = mpsc::unbounded_channel();
        let out = ServerOutput {
            session: session.clone(),
            frame: "s".into(),
            project: "p".into(),
            turn: "t".into(),
            persist: tx,
        };
        assert!(out.force_ask_mutations());
        let approval = out.confirm_decision_async("write file");
        tokio::pin!(approval);
        tokio::select! { _ = &mut approval => panic!("must wait"), _ = tokio::task::yield_now() => {} }
        assert_eq!(session.live.lock().unwrap().approvals.len(), 1);
        session.stop();
        assert!(matches!(approval.await, ConfirmDecision::Denied { .. }));
        assert!(matches!(
            out.confirm_decision_async("later").await,
            ConfirmDecision::Denied { .. }
        ));
    }
}
