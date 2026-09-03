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
    fn tool_presentation(
        &self,
        _kind: &str,
        _payload: &Value,
        _server: Option<std::sync::Arc<dyn wisp_tools::McpAppServer>>,
    ) {
    }
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
    /// When true, mutating tools require Ask even if the host policy is Allow
    /// and even if [`Self::approval_bypass`] is set. Used for unattended IM
    /// turns that share the desktop approval UI.
    fn force_ask_mutations(&self) -> bool {
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
    /// Fired once per message appended to the context during a turn (user,
    /// assistant, tool). Lets the host persist incrementally so a crash or a
    /// mid-turn "new session" doesn't lose the whole turn. Default: no-op.
    fn on_message(&self, _msg: &wisp_llm::Message) {}
    /// Fired once per producing tool call that wrote ≥1 file, with the code,
    /// result text, and diffed inputs/outputs. Default: no-op (CLI ignores it).
    fn provenance(&self, _rec: &crate::provenance::ProvenanceRecord) {}
    /// Opaque identity of the conversation tree this loop belongs to (a root
    /// frame id). Producing-tool windows of the same scope — one conversation
    /// and its subagents — are not foreign to each other when disambiguating
    /// concurrent writes (#911). Default `None`: every other window is foreign.
    fn provenance_scope(&self) -> Option<String> {
        None
    }
    /// Host-owned id of the in-flight user-visible turn. Default `None`.
    fn turn_id(&self) -> Option<&str> {
        None
    }
    /// Conversation frame this loop belongs to. Default `None`.
    fn frame_id(&self) -> Option<&str> {
        None
    }
    /// Host-owned project this loop belongs to. Default `None`.
    fn project_id(&self) -> Option<&str> {
        None
    }
    /// Hard host-owned boundary checked before free-form source reaches a
    /// local shell or language runtime.
    fn preflight_local_execution(&self, _source: &str) -> Result<(), String> {
        Ok(())
    }
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
    /// Mid-turn guidance queue (`GuidanceQueue`). Typed as the mutex so this
    /// module does not depend on `agent`.
    guidance: Option<&'a std::sync::Mutex<Vec<(u64, String)>>>,
    /// Kernel-reported writes for the in-flight tool call.
    ///
    /// Tool calls within one agent loop run strictly sequentially, so
    /// drain-per-call is race-free; parallel subagent loops each construct
    /// their own adapter, so reports cannot cross loops. The buffer must be
    /// drained unconditionally after every tool call so a stale report can
    /// never leak into the next call's record.
    reported_writes: std::sync::Mutex<Vec<String>>,
}

impl<'a> ToolEnvAdapter<'a> {
    pub fn new(root: std::path::PathBuf, out: &'a dyn Output) -> Self {
        Self {
            root,
            out,
            cancel: None,
            guidance: None,
            reported_writes: std::sync::Mutex::new(Vec::new()),
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
            guidance: None,
            reported_writes: std::sync::Mutex::new(Vec::new()),
        }
    }
    /// Drain kernel-reported writes accumulated during the current tool call.
    pub(crate) fn take_reported_writes(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .reported_writes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
    /// Let long-running tools see that the host has queued mid-turn guidance
    /// without draining it. The agent loop still injects at the iteration
    /// boundary.
    pub fn with_guidance(mut self, queue: &'a std::sync::Mutex<Vec<(u64, String)>>) -> Self {
        self.guidance = Some(queue);
        self
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
    fn project_id(&self) -> Option<&str> {
        self.out.project_id()
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
    fn force_ask_mutations(&self) -> bool {
        self.out.force_ask_mutations()
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
    fn guidance_pending(&self) -> bool {
        self.guidance
            .and_then(|queue| queue.lock().ok())
            .is_some_and(|pending| !pending.is_empty())
    }
    fn cancel_flag(&self) -> Option<&std::sync::atomic::AtomicBool> {
        self.cancel
    }
    async fn preflight_local_execution(&self, source: &str) -> Result<(), String> {
        self.out.preflight_local_execution(source)
    }
    async fn preflight_shell(&self, cmd: &str) -> Result<(), String> {
        self.out.preflight_shell(cmd)
    }
    fn note_shell_outcome(&self, cmd: &str, success: bool, detail: &str) {
        self.out.note_shell_outcome(cmd, success, detail);
    }
    fn report_written_paths(&self, paths: &[String]) {
        self.reported_writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(paths.iter().cloned());
    }
    fn turn_id(&self) -> Option<&str> {
        self.out.turn_id()
    }
    fn frame_id(&self) -> Option<&str> {
        self.out.frame_id()
    }
    async fn emit(&self, event: wisp_tools::ToolEvent) {
        match event {
            wisp_tools::ToolEvent::Call { name, preview } => self.out.tool_call(&name, &preview),
            wisp_tools::ToolEvent::Diff { path, old, new } => self.out.diff(&path, &old, &new),
            wisp_tools::ToolEvent::FileChanged { path } => self.out.file_changed(&path),
            wisp_tools::ToolEvent::Stdout { chunk } => self.out.stdout_chunk(&chunk),
            wisp_tools::ToolEvent::Presentation {
                kind,
                payload,
                server,
            } => self.out.tool_presentation(&kind, &payload, server),
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
}
impl<'a> StreamSinkAdapter<'a> {
    pub fn new(out: &'a dyn Output) -> Self {
        Self { out, cancel: None }
    }
    /// Like `new`, but the streaming loop can poll `is_cancelled()` to stop
    /// token generation mid-stream when the user hits Stop.
    pub fn with_cancel(out: &'a dyn Output, cancel: &'a std::sync::atomic::AtomicBool) -> Self {
        Self {
            out,
            cancel: Some(cancel),
        }
    }
}
impl<'a> wisp_llm::StreamSink for StreamSinkAdapter<'a> {
    fn on_text(&mut self, delta: &str) {
        self.out.assistant_text(delta);
    }
    fn on_reasoning(&mut self, delta: &str) {
        self.out.reasoning(delta);
    }
    fn on_tool_call(&mut self, _i: usize, _name: &str, _args: &str) {}
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

    #[test]
    fn tool_env_adapter_reports_pending_guidance_without_draining() {
        let out = NullOutput;
        let queue = std::sync::Mutex::new(Vec::<(u64, String)>::new());
        let env = ToolEnvAdapter::new(std::path::PathBuf::from("."), &out).with_guidance(&queue);
        assert!(
            !wisp_tools::ToolEnv::guidance_pending(&env),
            "empty queue is not pending"
        );
        queue.lock().unwrap().push((1, "how far?".into()));
        assert!(
            wisp_tools::ToolEnv::guidance_pending(&env),
            "queued guidance must be visible to long waits"
        );
        assert_eq!(
            queue.lock().unwrap().len(),
            1,
            "peeking must not drain the queue; the agent loop injects at the iteration boundary"
        );
        assert!(!wisp_tools::ToolEnv::guidance_pending(
            &ToolEnvAdapter::new(std::path::PathBuf::from("."), &out)
        ));
    }

    #[test]
    fn reported_writes_accumulate_then_drain_empty() {
        let out = NullOutput;
        let env = ToolEnvAdapter::new(std::path::PathBuf::from("."), &out);
        wisp_tools::ToolEnv::report_written_paths(&env, &["a.txt".into(), "b.txt".into()]);
        wisp_tools::ToolEnv::report_written_paths(&env, &["c.txt".into()]);
        assert_eq!(
            env.take_reported_writes(),
            vec![
                "a.txt".to_string(),
                "b.txt".to_string(),
                "c.txt".to_string()
            ]
        );
        assert!(env.take_reported_writes().is_empty());
    }
}
