//! UI/CLI output abstraction. The agent loop drives this; the headless CLI
//! prints to the terminal and the Tauri host forwards each call as an event.
//!
//! All methods take `&self` so a single shared `Output` can be borrowed by the
//! tool environment and the stream sink simultaneously. Interactive state
//! (confirmation prompts) is guarded with interior mutability in impls.

use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

/// Object-safe async hook used by interactive outputs. Most headless outputs
/// keep the synchronous defaults below; desktop outputs return a future that
/// yields while the UI sends its decision back.
pub type OutputFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait Output: Send + Sync {
    fn assistant_text(&self, _delta: &str) {}
    fn reasoning(&self, _delta: &str) {}
    fn tool_call(&self, _name: &str, _preview: &str) {}
    fn tool_result(&self, _name: &str, _ok: bool, _content: &str, _duration_ms: u64) {}
    fn usage(
        &self,
        _round: usize,
        _input: u64,
        _output: u64,
        _reasoning: u64,
        _cached: u64,
        _ctx_tokens: usize,
        _max_context: usize,
        _context_usage: crate::ContextUsage,
    ) {
    }
    fn compaction(&self, _before: usize, _after: usize, _strategy: &str) {}
    fn compaction_started(&self, _strategy: &str) {}
    /// Fired once when the context estimate crosses the warning threshold and
    /// automatic compaction is disabled or could not bring it back under.
    fn context_warning(&self, _ctx_tokens: usize, _max_context: usize) {}
    fn diff(&self, _path: &str, _old: &str, _new: &str) {}
    fn file_changed(&self, _path: &str) {}
    fn stdout_chunk(&self, _chunk: &str) {}
    fn tool_presentation(&self, _kind: &str, _payload: &Value) {}
    /// Blocking confirmation prompt for destructive actions.
    fn confirm(&self, _message: &str) -> bool {
        true
    }
    /// Confirmation prompt that can carry rejection feedback.
    fn confirm_decision(&self, message: &str) -> wisp_tools::ConfirmDecision {
        if self.confirm(message) {
            wisp_tools::ConfirmDecision::Approved
        } else {
            wisp_tools::ConfirmDecision::Denied { feedback: None }
        }
    }
    /// Async confirmation path used by [`ToolEnvAdapter`]. The default bridges
    /// existing CLI/test outputs to their synchronous implementation; GUI
    /// hosts should override it so waiting never blocks their command runtime.
    fn confirm_async<'a>(&'a self, message: &'a str) -> OutputFuture<'a, bool> {
        Box::pin(async move { self.confirm(message) })
    }
    /// Async variant carrying rejection feedback.
    fn confirm_decision_async<'a>(
        &'a self,
        message: &'a str,
    ) -> OutputFuture<'a, wisp_tools::ConfirmDecision> {
        Box::pin(async move { self.confirm_decision(message) })
    }
    /// Approval mode for a tool about to run. Default `Allow` preserves the old
    /// auto-run behaviour; the Tauri host overrides it from its saved policy.
    fn approval_mode(&self, _tool: &str) -> wisp_tools::Approval {
        wisp_tools::Approval::Allow
    }
    /// Desktop hosts can coordinate project resources across conversations.
    /// The default keeps CLI and test execution independent.
    fn acquire_tool_resources<'a>(
        &'a self,
        _tool: &'a str,
        _args: &'a Value,
    ) -> OutputFuture<'a, Result<Option<wisp_tools::ToolResourceLease>, String>> {
        Box::pin(async { Ok(None) })
    }
    /// Whether this conversation bypasses approval prompts. Explicit blocks
    /// and the tool registry's plan-mode gate remain authoritative.
    fn approval_bypass(&self) -> bool {
        false
    }
    fn restrict_read_paths_to_project(&self) -> bool {
        false
    }
    /// True when the approval scope is "full" — dangerous shell commands skip
    /// their confirm prompt. Default `false`; the Tauri host overrides it.
    fn danger_auto_approve(&self) -> bool {
        false
    }
    /// True while the session is in plan mode, so the tool registry refuses
    /// everything outside its read-only set. Default `false`.
    fn plan_mode(&self) -> bool {
        false
    }
    /// True when project state is temporarily frozen but the conversation may
    /// continue with read-only tools.
    fn project_write_locked(&self) -> bool {
        false
    }
    /// Structured run/round/tool lifecycle event. Default no-op keeps
    /// CLI/test outputs unchanged; hosts that render live tool state (the
    /// Tauri shell) override it and project the event onto their own
    /// protocol. Existing granular callbacks (`assistant_text`, `tool_call`,
    /// ...) keep firing alongside it during the compatibility period.
    fn runtime_event(&self, _event: &crate::runtime_event::AgentRuntimeEvent) {}
    /// Fired once per message appended to the context during a turn (user,
    /// assistant, tool). Lets the host persist incrementally so a crash or a
    /// mid-turn "new session" doesn't lose the whole turn. Default: no-op.
    fn on_message(&self, _msg: &wisp_llm::Message) {}
    /// Fired once per producing tool call that wrote ≥1 file, with the code,
    /// result text, and diffed inputs/outputs. Default: no-op (CLI ignores it).
    fn provenance(&self, _rec: &crate::provenance::ProvenanceRecord) {}
    /// Optional shell preflight (e.g. block free-form SSH after a prior failure).
    fn preflight_shell(&self, _cmd: &str) -> Result<(), String> {
        Ok(())
    }
    /// Optional shell postflight so the host can open an SSH connectivity gate.
    fn note_shell_outcome(&self, _cmd: &str, _success: bool, _detail: &str) {}
}

/// A silent output for tests / non-interactive runs that auto-approves.
pub struct NullOutput;
impl Output for NullOutput {}

/// Adapter exposing `Output` as a `wisp_tools::ToolEnv`.
pub struct ToolEnvAdapter<'a> {
    root: std::path::PathBuf,
    out: &'a dyn Output,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// Invocation identity injected by the agent loop (never by the tool) via
    /// [`ToolEnvAdapter::for_invocation`]. Structured progress is forwarded
    /// only while this is set and the invocation is still active.
    invocation: Option<crate::runtime_event::ToolInvocationKey>,
    invocation_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<'a> ToolEnvAdapter<'a> {
    pub fn new(root: std::path::PathBuf, out: &'a dyn Output) -> Self {
        Self {
            root,
            out,
            cancel: None,
            invocation: None,
            invocation_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    /// Like `new`, but tools can poll `is_cancelled()` to stop mid-execution.
    pub fn with_cancel(
        root: std::path::PathBuf,
        out: &'a dyn Output,
        cancel: &'a std::sync::atomic::AtomicBool,
    ) -> Self {
        Self {
            root,
            out,
            cancel: Some(cancel),
            invocation: None,
            invocation_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    /// Per-call clone carrying the invocation key the agent loop assigned, so
    /// `ToolEvent::Progress` can be projected onto the runtime-event stream
    /// with a stable identity. Progress is forwarded only until
    /// [`ToolEnvAdapter::finish_invocation`] runs.
    pub fn for_invocation(&self, key: crate::runtime_event::ToolInvocationKey) -> Self {
        Self {
            root: self.root.clone(),
            out: self.out,
            cancel: self.cancel,
            invocation: Some(key),
            invocation_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }
    /// Mark the invocation finished: progress emitted afterwards (e.g. by a
    /// detached task the tool spawned) is silently ignored.
    pub fn finish_invocation(&self) {
        self.invocation_active
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[async_trait::async_trait]
impl<'a> wisp_tools::ToolEnv for ToolEnvAdapter<'a> {
    fn project_root(&self) -> &std::path::Path {
        &self.root
    }
    fn restrict_read_paths_to_project(&self) -> bool {
        self.out.restrict_read_paths_to_project()
    }
    async fn confirm(&self, message: &str) -> bool {
        self.out.confirm_async(message).await
    }
    async fn confirm_decision(&self, message: &str) -> wisp_tools::ConfirmDecision {
        self.out.confirm_decision_async(message).await
    }
    async fn approval_mode(&self, tool: &str) -> wisp_tools::Approval {
        self.out.approval_mode(tool)
    }
    async fn acquire_tool_resources(
        &self,
        tool: &str,
        args: &Value,
    ) -> Result<Option<wisp_tools::ToolResourceLease>, String> {
        self.out.acquire_tool_resources(tool, args).await
    }
    fn approval_bypass(&self) -> bool {
        self.out.approval_bypass()
    }
    fn danger_auto_approve(&self) -> bool {
        self.out.danger_auto_approve()
    }
    fn plan_mode(&self) -> bool {
        self.out.plan_mode()
    }
    fn project_write_locked(&self) -> bool {
        self.out.project_write_locked()
    }
    fn is_cancelled(&self) -> bool {
        self.cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    }
    fn cancel_flag(&self) -> Option<&std::sync::atomic::AtomicBool> {
        self.cancel
    }
    async fn preflight_shell(&self, cmd: &str) -> Result<(), String> {
        self.out.preflight_shell(cmd)
    }
    fn note_shell_outcome(&self, cmd: &str, success: bool, detail: &str) {
        self.out.note_shell_outcome(cmd, success, detail);
    }
    async fn emit(&self, event: wisp_tools::ToolEvent) {
        match event {
            wisp_tools::ToolEvent::Call { name, preview } => self.out.tool_call(&name, &preview),
            wisp_tools::ToolEvent::Diff { path, old, new } => self.out.diff(&path, &old, &new),
            wisp_tools::ToolEvent::FileChanged { path } => self.out.file_changed(&path),
            wisp_tools::ToolEvent::Stdout { chunk } => self.out.stdout_chunk(&chunk),
            wisp_tools::ToolEvent::Presentation { kind, payload } => {
                self.out.tool_presentation(&kind, &payload)
            }
            wisp_tools::ToolEvent::Progress { details } => {
                // Structured progress rides the runtime-event stream only while
                // the owning invocation is live; anything emitted after the
                // tool finished (or without an invocation key) is dropped.
                let active = self
                    .invocation_active
                    .load(std::sync::atomic::Ordering::Relaxed);
                if active {
                    if let Some(key) = &self.invocation {
                        self.out.runtime_event(
                            &crate::runtime_event::AgentRuntimeEvent::ToolExecutionUpdated {
                                key: key.clone(),
                                details: crate::runtime_event::bound_tool_details(details),
                            },
                        );
                    }
                }
            }
            wisp_tools::ToolEvent::Result { ok: _ } => {}
        }
        let _ = Value::Null;
    }
}

/// Adapter exposing `Output` as a `wisp_llm::StreamSink` (text + reasoning
/// deltas only; usage/tool-call deltas are handled by the agent loop).
pub struct StreamSinkAdapter<'a> {
    out: &'a dyn Output,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// Round (agent-loop iteration) this sink streams; stamped onto the
    /// runtime events forwarded with each delta.
    round: usize,
}
impl<'a> StreamSinkAdapter<'a> {
    pub fn new(out: &'a dyn Output) -> Self {
        Self {
            out,
            cancel: None,
            round: 0,
        }
    }
    /// Like `new`, but the streaming loop can poll `is_cancelled()` to stop
    /// token generation mid-stream when the user hits Stop.
    pub fn with_cancel(out: &'a dyn Output, cancel: &'a std::sync::atomic::AtomicBool) -> Self {
        Self {
            out,
            cancel: Some(cancel),
            round: 0,
        }
    }
    /// Stamp runtime events with the agent-loop round being streamed.
    pub fn for_round(mut self, round: usize) -> Self {
        self.round = round;
        self
    }
}
impl<'a> wisp_llm::StreamSink for StreamSinkAdapter<'a> {
    fn on_text(&mut self, delta: &str) {
        self.out.assistant_text(delta);
        self.out.runtime_event(
            &crate::runtime_event::AgentRuntimeEvent::AssistantTextDelta {
                round: self.round,
                delta: delta.to_string(),
            },
        );
    }
    fn on_reasoning(&mut self, delta: &str) {
        self.out.reasoning(delta);
        self.out.runtime_event(
            &crate::runtime_event::AgentRuntimeEvent::AssistantReasoningDelta {
                round: self.round,
                delta: delta.to_string(),
            },
        );
    }
    fn on_tool_call(&mut self, snapshot: &wisp_llm::ToolCallSnapshot) {
        self.out.runtime_event(
            &crate::runtime_event::AgentRuntimeEvent::AssistantToolCallUpdated(
                crate::runtime_event::ToolCallDraft {
                    key: crate::runtime_event::ToolInvocationKey::draft(self.round, snapshot.index),
                    id: snapshot.id.clone(),
                    name: snapshot.name.clone(),
                    arguments_so_far: snapshot.arguments_so_far.clone(),
                },
            ),
        );
    }
    fn on_usage(&mut self, _u: wisp_llm::Usage) {}
    fn is_cancelled(&self) -> bool {
        self.cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use wisp_llm::StreamSink;

    // The streaming loops break on `sink.is_cancelled()`; this proves the Stop
    // flag is actually threaded through the sink and read (the wiring that was
    // missing in #58, leaving Stop dead during token streaming).
    #[test]
    fn stream_sink_adapter_polls_cancel_flag() {
        let out = NullOutput;
        let flag = AtomicBool::new(false);
        let sink = StreamSinkAdapter::with_cancel(&out, &flag);
        assert!(!sink.is_cancelled(), "not cancelled before Stop");
        flag.store(true, Ordering::Relaxed);
        assert!(
            sink.is_cancelled(),
            "reflects the flag once Stop is pressed"
        );
        // A sink built without a cancel flag never reports cancelled.
        assert!(!StreamSinkAdapter::new(&out).is_cancelled());
    }

    #[test]
    fn tool_call_snapshots_become_live_draft_events() {
        use crate::runtime_event::{AgentRuntimeEvent, ToolInvocationKey};
        use wisp_llm::ToolCallSnapshot;

        struct Recorder(Mutex<Vec<AgentRuntimeEvent>>);
        impl Output for Recorder {
            fn runtime_event(&self, event: &AgentRuntimeEvent) {
                self.0.lock().unwrap().push(event.clone());
            }
        }

        let out = Recorder(Mutex::new(Vec::new()));
        let mut sink = StreamSinkAdapter::new(&out).for_round(3);
        // Fragments arrive; each emission is a full snapshot keyed by
        // (round, index). The id appears once the stream reveals it.
        sink.on_tool_call(&ToolCallSnapshot {
            index: 1,
            id: None,
            name: "read".into(),
            arguments_so_far: "{\"pa".into(),
        });
        sink.on_tool_call(&ToolCallSnapshot {
            index: 1,
            id: Some("call-9".into()),
            name: "read".into(),
            arguments_so_far: "{\"path\":\"a.txt\"}".into(),
        });

        let events = out.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        let mut drafts = events.iter().map(|event| {
            let AgentRuntimeEvent::AssistantToolCallUpdated(draft) = event else {
                panic!("expected a draft event, got {event:?}");
            };
            assert_eq!(draft.key, ToolInvocationKey::draft(3, 1));
            assert_eq!(draft.name, "read");
            draft
        });
        let first = drafts.next().unwrap();
        assert_eq!(first.id, None);
        assert_eq!(first.arguments_so_far, "{\"pa");
        let second = drafts.next().unwrap();
        assert_eq!(second.id.as_deref(), Some("call-9"));
        assert_eq!(second.arguments_so_far, "{\"path\":\"a.txt\"}");
    }

    struct AsyncConfirmOutput {
        receiver: Mutex<Option<tokio::sync::oneshot::Receiver<wisp_tools::ConfirmDecision>>>,
        sync_called: AtomicBool,
    }

    impl Output for AsyncConfirmOutput {
        fn confirm_decision(&self, _message: &str) -> wisp_tools::ConfirmDecision {
            self.sync_called.store(true, Ordering::SeqCst);
            wisp_tools::ConfirmDecision::Denied { feedback: None }
        }

        fn confirm_decision_async<'a>(
            &'a self,
            _message: &'a str,
        ) -> OutputFuture<'a, wisp_tools::ConfirmDecision> {
            let receiver = self.receiver.lock().unwrap().take().unwrap();
            Box::pin(async move {
                receiver
                    .await
                    .unwrap_or(wisp_tools::ConfirmDecision::Denied { feedback: None })
            })
        }
    }

    #[tokio::test]
    async fn tool_env_yields_while_async_confirmation_is_pending() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let output = AsyncConfirmOutput {
            receiver: Mutex::new(Some(receiver)),
            sync_called: AtomicBool::new(false),
        };
        let env = ToolEnvAdapter::new(std::path::PathBuf::from("."), &output);
        let decision = wisp_tools::ToolEnv::confirm_decision(&env, "Run tool?");
        tokio::pin!(decision);

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut decision)
                .await
                .is_err(),
            "a pending UI decision must keep the tool call suspended"
        );
        sender.send(wisp_tools::ConfirmDecision::Approved).unwrap();
        assert_eq!(decision.await, wisp_tools::ConfirmDecision::Approved);
        assert!(
            !output.sync_called.load(Ordering::SeqCst),
            "the adapter must not fall back to the runtime-blocking sync hook"
        );
    }

    #[tokio::test]
    async fn progress_is_forwarded_only_for_an_active_invocation() {
        use crate::runtime_event::{AgentRuntimeEvent, ToolInvocationKey};

        struct Recorder(Mutex<Vec<AgentRuntimeEvent>>);
        impl Output for Recorder {
            fn runtime_event(&self, event: &AgentRuntimeEvent) {
                self.0.lock().unwrap().push(event.clone());
            }
        }

        let out = Recorder(Mutex::new(Vec::new()));
        let env = ToolEnvAdapter::new(std::path::PathBuf::from("."), &out);
        // No invocation key: progress has nowhere to land and is dropped.
        wisp_tools::ToolEnv::emit(
            &env,
            wisp_tools::ToolEvent::Progress {
                details: serde_json::json!({"pct": 1}),
            },
        )
        .await;
        assert!(out.0.lock().unwrap().is_empty());

        let key = ToolInvocationKey::ready(1, 0, "call-1");
        let invocation = env.for_invocation(key.clone());
        wisp_tools::ToolEnv::emit(
            &invocation,
            wisp_tools::ToolEvent::Progress {
                details: serde_json::json!({"pct": 50}),
            },
        )
        .await;
        // Once the invocation is finished, late progress is silently ignored.
        invocation.finish_invocation();
        wisp_tools::ToolEnv::emit(
            &invocation,
            wisp_tools::ToolEvent::Progress {
                details: serde_json::json!({"pct": 100}),
            },
        )
        .await;

        let events = out.0.lock().unwrap();
        assert_eq!(
            events.as_slice(),
            &[AgentRuntimeEvent::ToolExecutionUpdated {
                key,
                details: serde_json::json!({"pct": 50}),
            }]
        );
    }

    #[tokio::test]
    async fn oversized_progress_details_are_bounded() {
        use crate::runtime_event::{AgentRuntimeEvent, ToolInvocationKey};

        struct Recorder(Mutex<Vec<AgentRuntimeEvent>>);
        impl Output for Recorder {
            fn runtime_event(&self, event: &AgentRuntimeEvent) {
                self.0.lock().unwrap().push(event.clone());
            }
        }

        let out = Recorder(Mutex::new(Vec::new()));
        let env = ToolEnvAdapter::new(std::path::PathBuf::from("."), &out)
            .for_invocation(ToolInvocationKey::ready(1, 0, "call-1"));
        let big = serde_json::json!({"blob": "x".repeat(32 * 1024)});
        wisp_tools::ToolEnv::emit(&env, wisp_tools::ToolEvent::Progress { details: big }).await;

        let events = out.0.lock().unwrap();
        let [AgentRuntimeEvent::ToolExecutionUpdated { details, .. }] = events.as_slice() else {
            panic!("expected one progress event, got {events:?}");
        };
        assert_eq!(details["truncated"], serde_json::json!(true));
        let original = details["original_bytes"].as_u64().unwrap() as usize;
        assert!(original > crate::runtime_event::TOOL_DETAILS_MAX_BYTES);
        assert!(serde_json::to_string(details).unwrap().len() <= 256);
    }
}
