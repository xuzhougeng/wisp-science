use std::{
    collections::{HashMap, HashSet},
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

/// Tokens the running or latest turn has used, summed over its model calls,
/// and how full the context was after the last one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cached: u64,
    pub context_tokens: usize,
    pub max_context: usize,
}

/// What one model call used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageCall {
    /// Unix seconds.
    pub created_at: i64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cached: u64,
    pub ctx_tokens: usize,
    pub max_context: usize,
}

/// A model call's usage as it is saved with the conversation: the event
/// the WebView desktop writes, so both count from the same rows. `kind`
/// leads, which is what the usage queries look for.
#[derive(serde::Serialize)]
pub struct UsageEvent<'a> {
    pub kind: &'static str,
    pub frame_id: &'a str,
    pub round: u64,
    pub model: &'a str,
    pub created_at: i64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cached: u64,
    pub ctx_tokens: usize,
    pub max_context: usize,
}

/// The question the registry asks before a tool that needs approval runs.
/// Anything else (a dangerous shell command, for one) is a different question
/// and is never answered by a remembered "allow this tool".
pub fn tool_question(tool: &str) -> String {
    format!("Run tool '{tool}'?")
}

#[derive(Default)]
pub struct Live {
    pub running: bool,
    /// The latest work over many items (`map_items`) the conversation ran,
    /// as it went; kept after its turn until another one starts.
    pub swarm: Option<wisp_dto::SwarmProgress>,
    /// Whether the running turn still reads guidance. It stops once the turn
    /// has looked for the last time, so nothing is put in after that.
    pub guidable: bool,
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
    pub usage: Option<TokenUsage>,
    /// What each model call of the running turn used, in order, to be saved
    /// when the turn ends.
    pub calls: Vec<UsageCall>,
    /// What the model is thinking in the request that is in flight.
    pub reasoning: String,
    /// Files the running turn's tools wrote, to be recorded as artifacts
    /// when it ends.
    pub written: Vec<String>,
    /// Tools the user allowed for the rest of this conversation. Kept in
    /// memory only: a restart asks again.
    pub allowed_tools: HashSet<String>,
    /// Questions suggested after the latest answer, until the next turn.
    pub follow_ups: Vec<String>,
    /// The model is being asked for them right now.
    pub suggesting: bool,
    /// The review that followed the latest turn, until the next turn.
    pub review: Option<Result<crate::review::Review, String>>,
    /// The reviewer is reading the latest turn right now.
    pub reviewing: bool,
    /// Why the latest turn's loop stopped short (`max_iterations`), if it
    /// did.
    pub stop_reason: Option<String>,
    /// What the running turn's tool calls reported doing, to be kept in
    /// the execution log when it ends.
    pub provenance: Vec<wisp_core::provenance::ProvenanceRecord>,
}

pub struct Session {
    pub live: Mutex<Live>,
    pub cancel: Arc<AtomicBool>,
    /// Serializes acceptance, including the durable request-id check.
    pub accept: tokio::sync::Mutex<()>,
    /// Woken whenever `suggesting` or `reviewing` is cleared.
    pub suggested: tokio::sync::Notify,
    pub runtime: Mutex<Option<wisp_runtime::RuntimeManager>>,
    /// The ACP agent this conversation talks to, kept between its turns.
    pub acp: tokio::sync::Mutex<Option<crate::acp_turn::AcpLive>>,
    /// What the user said while a turn ran, for the turn to read at its
    /// next step instead of after it ends.
    pub guidance: wisp_core::GuidanceQueue,
    /// Numbers each piece of guidance.
    pub guided: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    pub snapshot_after_read: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            live: Mutex::new(Live::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            accept: Default::default(),
            suggested: Default::default(),
            runtime: Mutex::new(None),
            acp: Default::default(),
            guidance: Default::default(),
            guided: Default::default(),
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
    /// The conversation runs tools that change things without asking.
    pub full_access: bool,
    /// The conversation plans: tools that change things refuse to run.
    pub plan_mode: bool,
    /// Tools the user allowed for good in this project, as of the turn's
    /// start. They answer the standard tool question only.
    pub always: HashSet<String>,
    /// Another lab member's mail started the turn: it asks before every
    /// tool outside a short list and reads only inside the project.
    pub lab_mail: bool,
    /// An exploration round froze the project: this conversation keeps to
    /// tools that change nothing.
    pub write_locked: bool,
    /// The turn runs in an exploration: local code may not name the
    /// mainline's folder.
    pub isolation: Option<wisp_runs::exploration_isolation::ExplorationIsolationBoundary>,
}

impl Output for ServerOutput {
    fn tool_presentation(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        _server: Option<std::sync::Arc<dyn wisp_tools::McpAppServer>>,
    ) {
        if kind != wisp_dto::SWARM_PRESENTATION {
            return;
        }
        let Ok(progress) = serde_json::from_value::<wisp_dto::SwarmProgress>(payload.clone())
        else {
            return;
        };
        let mut live = self.session.live.lock().unwrap();
        match &mut live.swarm {
            Some(current) => current.merge(progress),
            None => live.swarm = Some(progress),
        }
    }
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
    fn reasoning(&self, delta: &str) {
        self.session.live.lock().unwrap().reasoning.push_str(delta);
    }
    fn plan_mode(&self) -> bool {
        self.plan_mode
    }
    fn file_changed(&self, path: &str) {
        self.session.live.lock().unwrap().written.push(path.into());
    }
    fn provenance(&self, record: &wisp_core::provenance::ProvenanceRecord) {
        let mut live = self.session.live.lock().unwrap();
        if record.success {
            live.written.extend(record.files_written.iter().cloned());
        }
        live.provenance.push(record.clone());
    }
    fn tool_result(&self, _: &str, ok: bool, _: &str, duration_ms: u64) {
        self.session.live.lock().unwrap().last_result = Some((ok, duration_ms));
    }
    fn usage(
        &self,
        _round: usize,
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
        _: wisp_core::ContextUsage,
    ) {
        let mut live = self.session.live.lock().unwrap();
        live.calls.push(UsageCall {
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs() as i64)
                .unwrap_or_default(),
            input,
            output,
            reasoning,
            cached,
            ctx_tokens,
            max_context,
        });
        let usage = live.usage.get_or_insert_with(TokenUsage::default);
        usage.input += input;
        usage.output += output;
        usage.reasoning += reasoning;
        usage.cached += cached;
        usage.context_tokens = ctx_tokens;
        usage.max_context = max_context;
    }
    fn on_message(&self, msg: &Message) {
        let mut live = self.session.live.lock().unwrap();
        if msg.role == wisp_llm::Role::Assistant {
            live.draft.clear();
            // The thought belonged to the request this message answers.
            live.reasoning.clear();
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
        !self.full_access || self.lab_mail
    }
    fn unattended_tools(&self) -> Option<&'static [&'static str]> {
        self.lab_mail
            .then_some(crate::channels::lab_mail::UNATTENDED_TOOLS)
    }
    fn restrict_read_paths_to_project(&self) -> bool {
        self.lab_mail || self.isolation.is_some()
    }
    fn project_write_locked(&self) -> bool {
        self.write_locked
    }
    fn preflight_local_execution(&self, source: &str) -> Result<(), String> {
        match &self.isolation {
            Some(boundary) => boundary.check_local_source(source),
            None => Ok(()),
        }
    }
    fn approval_bypass(&self) -> bool {
        // Dangerous shell commands keep their own question either way.
        self.full_access
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
                let allowed = live.allowed_tools.contains(&tool) || self.always.contains(&tool);
                if allowed && message == tool_question(&tool) {
                    return ConfirmDecision::Approved;
                }
                // A delegated batch puts its own plan to the researcher when
                // any task can change something, so it is not asked about
                // twice. Another member's mail still asks first.
                if !self.lab_mail
                    && tool == crate::delegation::DELEGATE
                    && message == tool_question(&tool)
                {
                    return ConfirmDecision::Approved;
                }
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
            full_access: false,
            plan_mode: false,
            always: Default::default(),
            lab_mail: false,
            write_locked: false,
            isolation: None,
        };
        assert!(out.force_ask_mutations());
        assert!(!out.approval_bypass());
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

    fn output(session: &Arc<Session>, full_access: bool) -> ServerOutput {
        ServerOutput {
            session: session.clone(),
            frame: "s".into(),
            project: "p".into(),
            turn: "t".into(),
            persist: mpsc::unbounded_channel().0,
            full_access,
            plan_mode: false,
            always: Default::default(),
            lab_mail: false,
            write_locked: false,
            isolation: None,
        }
    }

    #[test]
    fn work_over_many_items_is_followed_as_it_goes() {
        let session = Arc::new(Session::default());
        let out = output(&session, true);
        let task = |id: &str, status: &str, revision: u64, done: usize| {
            serde_json::json!({
                "id": id, "title": id, "instruction": "", "status": status,
                "revision": revision, "completed_units": done, "total_units": 1,
                "failed_units": 0, "log": [], "result": "",
            })
        };
        let progress = |id: &str, revision: u64, phase: &str, tasks: Vec<serde_json::Value>| {
            serde_json::json!({
                "id": id, "source": "map_items", "title": "Read the papers",
                "model": "m", "started_at_ms": revision as i64, "revision": revision,
                "phase": phase, "total": 2, "tasks": tasks, "summary": "",
            })
        };
        // Something else shown by a tool is not work over items.
        out.tool_presentation("mcp_app", &serde_json::json!({}), None);
        assert!(session.live.lock().unwrap().swarm.is_none());
        out.tool_presentation(
            wisp_dto::SWARM_PRESENTATION,
            &progress(
                "s1",
                1,
                "running",
                vec![task("a", "queued", 0, 0), task("b", "queued", 0, 0)],
            ),
            None,
        );
        // A live update carries the tasks that changed.
        out.tool_presentation(
            wisp_dto::SWARM_PRESENTATION,
            &progress("s1", 2, "running", vec![task("a", "completed", 1, 1)]),
            None,
        );
        let swarm = session.live.lock().unwrap().swarm.clone().unwrap();
        assert_eq!(swarm.tasks.len(), 2);
        assert_eq!(swarm.tasks[0].status, wisp_dto::SwarmTaskStatus::Completed);
        assert_eq!(swarm.tasks[1].status, wisp_dto::SwarmTaskStatus::Queued);
        // A newer run replaces it.
        out.tool_presentation(
            wisp_dto::SWARM_PRESENTATION,
            &progress("s2", 5, "starting", vec![task("c", "queued", 0, 0)]),
            None,
        );
        let swarm = session.live.lock().unwrap().swarm.clone().unwrap();
        assert_eq!((swarm.id.as_str(), swarm.tasks.len()), ("s2", 1));
    }

    #[tokio::test]
    async fn a_tool_allowed_for_the_conversation_is_not_asked_about_again() {
        let session = Arc::new(Session::default());
        let out = output(&session, false);
        {
            let mut live = session.live.lock().unwrap();
            live.tool = "write".into();
            live.allowed_tools.insert("write".into());
        }
        assert!(matches!(
            out.confirm_decision_async(&tool_question("write")).await,
            ConfirmDecision::Approved
        ));
        assert!(session.live.lock().unwrap().approvals.is_empty());
        // A different question about the same tool still reaches the user,
        // and so does the standard question about another tool.
        for (tool, message) in [
            ("write", "This command deletes files. Run it?".to_string()),
            ("shell", tool_question("shell")),
        ] {
            session.live.lock().unwrap().tool = tool.into();
            let asked = out.confirm_decision_async(&message);
            tokio::pin!(asked);
            tokio::select! { _ = &mut asked => panic!("must wait"), _ = tokio::task::yield_now() => {} }
            assert_eq!(session.live.lock().unwrap().approvals.len(), 1);
            session.live.lock().unwrap().approvals.clear();
        }
    }

    #[test]
    fn full_access_stops_asking_and_usage_adds_up_over_a_turn() {
        let session = Arc::new(Session::default());
        let out = output(&session, true);
        assert!(!out.force_ask_mutations() && out.approval_bypass());
        // Dangerous commands are a separate switch that stays off.
        assert!(!out.danger_auto_approve());
        out.usage(1, 100, 20, 5, 40, 900, 64_000, Default::default());
        out.usage(2, 150, 30, 0, 90, 1_200, 64_000, Default::default());
        assert_eq!(
            session.live.lock().unwrap().usage,
            Some(TokenUsage {
                input: 250,
                output: 50,
                reasoning: 5,
                cached: 130,
                context_tokens: 1_200,
                max_context: 64_000,
            })
        );
    }
}
