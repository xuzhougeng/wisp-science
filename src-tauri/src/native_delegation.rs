use async_trait::async_trait;
use std::{
    collections::HashSet,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use wisp_core::{
    agent_loop, AgentBudget, AgentDelegationRequest, AgentUsage, ContextManager, Output,
};
use wisp_llm::{Completion, LlmError, Message, Provider, StreamSink, ToolSchema};
use wisp_store::{ExecLog, Store};
use wisp_tools::{Approval, Registry};

pub(crate) struct NativeAgentRun {
    pub(crate) result: Result<String, String>,
    pub(crate) usage: AgentUsage,
}

#[derive(Default)]
struct UsageTracker(Mutex<AgentUsage>);

impl UsageTracker {
    fn record(&self, completion: &Completion, budget: &AgentBudget) -> Result<(), LlmError> {
        let mut usage = self.0.lock().unwrap();
        usage.input_tokens = usage
            .input_tokens
            .saturating_add(completion.usage.input_tokens);
        usage.output_tokens = usage
            .output_tokens
            .saturating_add(completion.usage.output_tokens);
        usage.tool_calls = usage
            .tool_calls
            .saturating_add(completion.tool_calls.len() as u64);
        if let Some(reason) = budget_violation(&usage, budget) {
            return Err(LlmError::Config(reason));
        }
        Ok(())
    }

    fn snapshot(&self) -> AgentUsage {
        self.0.lock().unwrap().clone()
    }
}

struct BudgetedProvider<'a> {
    inner: &'a dyn Provider,
    budget: &'a AgentBudget,
    usage: Arc<UsageTracker>,
}

#[async_trait]
impl Provider for BudgetedProvider<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> wisp_llm::Result<Completion> {
        let completion = self.inner.complete(messages, tools).await?;
        self.usage.record(&completion, self.budget)?;
        Ok(completion)
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let completion = self.inner.stream(messages, tools, sink).await?;
        self.usage.record(&completion, self.budget)?;
        Ok(completion)
    }
}

struct NativeOutput {
    allowed_tools: HashSet<String>,
    messages: tokio::sync::mpsc::UnboundedSender<Message>,
    provenance: tokio::sync::mpsc::UnboundedSender<wisp_core::ProvenanceRecord>,
}

impl Output for NativeOutput {
    fn confirm(&self, _message: &str) -> bool {
        false
    }

    fn approval_mode(&self, tool: &str) -> Approval {
        if self.allowed_tools.contains(tool) {
            Approval::Allow
        } else {
            Approval::Deny
        }
    }

    fn restrict_read_paths_to_project(&self) -> bool {
        true
    }

    fn on_message(&self, message: &Message) {
        let _ = self.messages.send(message.clone());
    }

    fn provenance(&self, record: &wisp_core::ProvenanceRecord) {
        let _ = self.provenance.send(record.clone());
    }

    fn preflight_shell(&self, _cmd: &str) -> Result<(), String> {
        Err("direct shell is not available to Native delegated Agents".into())
    }
}

pub(crate) async fn run_native_agent(
    provider: &dyn Provider,
    vision_provider: Option<&dyn Provider>,
    store: &Store,
    project_id: &str,
    child_frame_id: &str,
    project_root: &Path,
    tools: &Registry,
    request: &AgentDelegationRequest,
    system: String,
    prompt: String,
    cancel: &AtomicBool,
) -> anyhow::Result<NativeAgentRun> {
    let (message_tx, mut message_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let message_store = store.clone();
    let message_project_id = project_id.to_string();
    let message_frame_id = child_frame_id.to_string();
    let message_project_root = project_root.to_path_buf();
    let model = provider.model().to_string();
    let message_task = tokio::spawn(async move {
        let mut sequence = 0i64;
        while let Some(mut message) = message_rx.recv().await {
            if message.role == wisp_llm::Role::Assistant && message.model_name.is_none() {
                message.model_name = Some(model.clone());
            }
            sequence += 1;
            message_store
                .append_message(&message_frame_id, sequence, &message)
                .await?;
            if message.role == wisp_llm::Role::Assistant {
                crate::resource_refs::bind_new_message_resources(
                    &message_store,
                    &message_project_root,
                    &message_project_id,
                    &message_frame_id,
                    sequence,
                    &message.content.as_text(),
                )
                .await;
            }
        }
        anyhow::Ok(())
    });

    let (provenance_tx, mut provenance_rx) =
        tokio::sync::mpsc::unbounded_channel::<wisp_core::ProvenanceRecord>();
    let provenance_store = store.clone();
    let provenance_frame_id = child_frame_id.to_string();
    let provenance_task = tokio::spawn(async move {
        while let Some(record) = provenance_rx.recv().await {
            let log = ExecLog {
                id: uuid::Uuid::new_v4().to_string(),
                frame_id: provenance_frame_id.clone(),
                cell_index: provenance_store
                    .next_cell_index(&provenance_frame_id)
                    .await?,
                tool: record.tool,
                language: record.language,
                source: record.source,
                stdout: record.output,
                stderr: String::new(),
                exit_status: if record.success {
                    "ok".into()
                } else {
                    "error".into()
                },
                wall_s: None,
                files_written: record.files_written,
                files_read: record.files_read,
                env_hash: None,
            };
            provenance_store.insert_execution_log(&log).await?;
        }
        anyhow::Ok(())
    });

    let output = NativeOutput {
        allowed_tools: tools.names().into_iter().map(str::to_string).collect(),
        messages: message_tx,
        provenance: provenance_tx,
    };
    let usage = Arc::new(UsageTracker::default());
    let vision_provider = vision_provider.map(|inner| BudgetedProvider {
        inner,
        budget: &request.spec.budget,
        usage: usage.clone(),
    });
    let provider = BudgetedProvider {
        inner: provider,
        budget: &request.spec.budget,
        usage: usage.clone(),
    };
    let max_context = request
        .spec
        .context_policy
        .max_tokens
        .or(request.spec.budget.max_tokens)
        .unwrap_or(32_000) as usize;
    let max_iterations = request
        .spec
        .budget
        .max_tool_calls
        .map(|limit| limit as usize + 1)
        .unwrap_or(100);
    let mut context = ContextManager::new(max_context.max(1));
    context.append_system(system);
    let run = agent_loop(
        &mut context,
        &provider,
        vision_provider
            .as_ref()
            .map(|provider| provider as &dyn Provider),
        tools,
        project_root,
        &output,
        &prompt,
        max_iterations,
        Some(cancel),
    )
    .await;
    let content = context
        .messages
        .iter()
        .rev()
        .find(|message| message.role == wisp_llm::Role::Assistant)
        .map(|message| message.content.as_text());
    drop(output);
    message_task.await??;
    provenance_task.await??;

    Ok(NativeAgentRun {
        result: run
            .map(|_| content.unwrap_or_default())
            .map_err(|error| error.to_string()),
        usage: usage.snapshot(),
    })
}

fn budget_violation(usage: &AgentUsage, budget: &AgentBudget) -> Option<String> {
    let total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
    if budget
        .max_tokens
        .is_some_and(|limit| total_tokens > u64::from(limit))
    {
        return Some(format!(
            "Agent exceeded its token budget ({total_tokens} tokens)"
        ));
    }
    if budget
        .max_tool_calls
        .is_some_and(|limit| usage.tool_calls > u64::from(limit))
    {
        return Some(format!(
            "Agent exceeded its tool-call budget ({} calls)",
            usage.tool_calls
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, path::PathBuf, sync::atomic::Ordering};
    use wisp_llm::{FunctionCall, ToolCall, Usage};

    struct SequenceProvider {
        completions: Mutex<VecDeque<Completion>>,
        schemas: Mutex<Vec<Vec<String>>>,
    }

    impl SequenceProvider {
        fn new(completions: Vec<Completion>) -> Self {
            Self {
                completions: Mutex::new(completions.into()),
                schemas: Mutex::new(Vec::new()),
            }
        }

        fn pop(&self, tools: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            self.schemas.lock().unwrap().push(
                tools
                    .iter()
                    .map(|schema| schema.function.name.clone())
                    .collect(),
            );
            self.completions
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("fake provider ran out of completions".into()))
        }
    }

    #[async_trait]
    impl Provider for SequenceProvider {
        fn name(&self) -> &str {
            "sequence"
        }

        fn model(&self) -> &str {
            "sequence-model"
        }

        async fn complete(
            &self,
            _messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            self.pop(tools)
        }

        async fn stream(
            &self,
            _messages: &[Message],
            tools: &[ToolSchema],
            _sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.pop(tools)
        }
    }

    struct BlockingProvider;

    #[async_trait]
    impl Provider for BlockingProvider {
        fn name(&self) -> &str {
            "blocking"
        }

        fn model(&self) -> &str {
            "blocking-model"
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            Err(LlmError::Config("complete should not be called".into()))
        }

        async fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            loop {
                if sink.is_cancelled() {
                    return Err(LlmError::Config("stream cancelled".into()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
    }

    fn completion(content: &str, tool_calls: Vec<ToolCall>, input: u64, output: u64) -> Completion {
        Completion {
            content: content.into(),
            reasoning: None,
            finish_reason: Some(if tool_calls.is_empty() {
                "stop".into()
            } else {
                "tool_calls".into()
            }),
            tool_calls,
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                reasoning_tokens: 0,
            },
        }
    }

    fn tool_call(id: &str, name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: args.to_string(),
            },
        }
    }

    fn request(max_tokens: u32, max_tool_calls: u32) -> AgentDelegationRequest {
        AgentDelegationRequest {
            request_id: "request-1".into(),
            workflow_id: "workflow-1".into(),
            step_id: "step-1".into(),
            spec: serde_json::from_value(serde_json::json!({
                "agent_id": "child-agent",
                "name": "Child Agent",
                "goal": "Complete the test task",
                "role": "analyst",
                "backend": "local",
                "prompt_template": "Work independently.",
                "permissions": {
                    "tools": ["read", "write"],
                    "paths": ["project://**"],
                    "write": true
                },
                "context_policy": {"max_tokens": 32000},
                "budget": {
                    "max_tokens": max_tokens,
                    "max_tool_calls": max_tool_calls
                }
            }))
            .unwrap(),
            input: serde_json::json!({}),
            lineage: None,
        }
    }

    async fn fixture() -> (Store, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "wisp_native_agent_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = Store::open(&base.join("db/store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &workspace.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("child", "project", "Child Agent", "sequence-model")
            .await
            .unwrap();
        (store, base, workspace)
    }

    async fn run_sequence(
        provider: &SequenceProvider,
        store: &Store,
        workspace: &Path,
        tools: &Registry,
        request: &AgentDelegationRequest,
    ) -> NativeAgentRun {
        run_native_agent(
            provider,
            None,
            store,
            "project",
            "child",
            workspace,
            tools,
            request,
            "Test system prompt".into(),
            "Test user prompt".into(),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn native_agent_sees_only_granted_tools_and_persists_messages() {
        let (store, base, workspace) = fixture().await;
        std::fs::write(workspace.join("evidence.txt"), "verified evidence").unwrap();
        let provider = SequenceProvider::new(vec![
            completion(
                "",
                vec![tool_call(
                    "call-read",
                    "read",
                    serde_json::json!({"path": "evidence.txt"}),
                )],
                11,
                3,
            ),
            completion(r#"{"summary":"done"}"#, vec![], 7, 2),
        ]);
        let tools = Registry::builtins().filtered(&["read".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 4)).await;

        assert_eq!(run.result.unwrap(), r#"{"summary":"done"}"#);
        assert_eq!(run.usage.input_tokens, 18);
        assert_eq!(run.usage.output_tokens, 5);
        assert_eq!(run.usage.tool_calls, 1);
        assert!(provider
            .schemas
            .lock()
            .unwrap()
            .iter()
            .all(|schemas| schemas == &["read"]));
        let messages = store.load_messages("child").await.unwrap();
        assert_eq!(messages.len(), 4);
        assert!(messages.iter().any(|message| {
            message.role == wisp_llm::Role::Tool
                && message.content.as_text().contains("verified evidence")
        }));
        assert!(messages
            .iter()
            .filter(|message| message.role == wisp_llm::Role::Assistant)
            .all(|message| message.model_name.as_deref() == Some("sequence-model")));
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_child_file_links_become_durable_artifact_references() {
        let (store, base, workspace) = fixture().await;
        let provider = SequenceProvider::new(vec![
            completion(
                "",
                vec![tool_call(
                    "call-write",
                    "write",
                    serde_json::json!({
                        "path": "report.md",
                        "content": "durable report"
                    }),
                )],
                1,
                1,
            ),
            completion(
                r#"{"summary":"Created [report](report.md)","files_changed":["report.md"],"diff_summary":"created report","artifacts":[],"evidence":[],"tests":[],"risks":[]}"#,
                vec![],
                1,
                1,
            ),
        ]);
        let tools = Registry::builtins().filtered(&["write".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 4)).await;

        assert!(run.result.is_ok(), "{:?}", run.result);
        let artifacts = store.list_artifacts("child").await.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].1, "report.md");
        assert!(artifacts[0].3.contains(".wisp/artifacts/sha256"));
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_cannot_call_an_ungranted_tool() {
        let (store, base, workspace) = fixture().await;
        let provider = SequenceProvider::new(vec![
            completion(
                "",
                vec![tool_call(
                    "call-write",
                    "write",
                    serde_json::json!({"path": "blocked.txt", "content": "no"}),
                )],
                1,
                1,
            ),
            completion("done", vec![], 1, 1),
        ]);
        let tools = Registry::builtins().filtered(&["read".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 4)).await;

        assert!(run.result.is_ok());
        assert!(!workspace.join("blocked.txt").exists());
        let messages = store.load_messages("child").await.unwrap();
        assert!(messages.iter().any(|message| {
            message.role == wisp_llm::Role::Tool
                && message.content.as_text().contains("unknown tool 'write'")
        }));
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_read_scope_rejects_parent_escape() {
        let (store, base, workspace) = fixture().await;
        std::fs::write(base.join("outside.txt"), "secret").unwrap();
        let provider = SequenceProvider::new(vec![
            completion(
                "",
                vec![tool_call(
                    "call-read",
                    "read",
                    serde_json::json!({"path": "../outside.txt"}),
                )],
                1,
                1,
            ),
            completion("done", vec![], 1, 1),
        ]);
        let tools = Registry::builtins().filtered(&["read".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 4)).await;

        assert!(run.result.is_ok());
        let messages = store.load_messages("child").await.unwrap();
        assert!(messages.iter().any(|message| {
            message.role == wisp_llm::Role::Tool
                && message.content.as_text().contains("outside project root")
        }));
        assert!(!messages
            .iter()
            .any(|message| message.content.as_text().contains("secret")));
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_writes_without_acp_and_persists_provenance() {
        let (store, base, workspace) = fixture().await;
        let provider = SequenceProvider::new(vec![
            completion(
                "",
                vec![tool_call(
                    "call-write",
                    "write",
                    serde_json::json!({"path": "result.txt", "content": "native output"}),
                )],
                1,
                1,
            ),
            completion("done", vec![], 1, 1),
        ]);
        let tools = Registry::builtins().filtered(&["write".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 4)).await;

        assert!(run.result.is_ok());
        assert_eq!(
            std::fs::read_to_string(workspace.join("result.txt")).unwrap(),
            "native output"
        );
        let provenance = store
            .find_provenance_by_path("child", "result.txt")
            .await
            .unwrap()
            .expect("Native write should be auditable without an ACP session");
        assert_eq!(provenance.tool, "write");
        assert_eq!(provenance.source, "result.txt");
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_stops_before_tools_when_completion_exceeds_budget() {
        let (store, base, workspace) = fixture().await;
        let provider = SequenceProvider::new(vec![completion(
            "",
            vec![tool_call(
                "call-write",
                "write",
                serde_json::json!({"path": "over-budget.txt", "content": "no"}),
            )],
            8,
            5,
        )]);
        let tools = Registry::builtins().filtered(&["write".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(10, 4)).await;

        assert!(run
            .result
            .unwrap_err()
            .contains("exceeded its token budget"));
        assert_eq!(run.usage.input_tokens, 8);
        assert_eq!(run.usage.output_tokens, 5);
        assert_eq!(run.usage.tool_calls, 1);
        assert!(!workspace.join("over-budget.txt").exists());
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_stops_before_tools_when_call_count_exceeds_budget() {
        let (store, base, workspace) = fixture().await;
        let provider = SequenceProvider::new(vec![completion(
            "",
            vec![
                tool_call(
                    "call-write-1",
                    "write",
                    serde_json::json!({"path": "first.txt", "content": "no"}),
                ),
                tool_call(
                    "call-write-2",
                    "write",
                    serde_json::json!({"path": "second.txt", "content": "no"}),
                ),
            ],
            1,
            1,
        )]);
        let tools = Registry::builtins().filtered(&["write".into()]);

        let run = run_sequence(&provider, &store, &workspace, &tools, &request(100, 1)).await;

        assert!(run
            .result
            .unwrap_err()
            .contains("exceeded its tool-call budget"));
        assert_eq!(run.usage.tool_calls, 2);
        assert!(!workspace.join("first.txt").exists());
        assert!(!workspace.join("second.txt").exists());
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }

    #[tokio::test]
    async fn native_agent_stream_honors_targeted_cancellation() {
        let (store, base, workspace) = fixture().await;
        let provider = BlockingProvider;
        let tools = Registry::builtins().filtered(&["read".into()]);
        let request = request(100, 4);
        let cancel = Arc::new(AtomicBool::new(false));
        let running = run_native_agent(
            &provider,
            None,
            &store,
            "project",
            "child",
            &workspace,
            &tools,
            &request,
            "Test system prompt".into(),
            "Test user prompt".into(),
            cancel.as_ref(),
        );
        let stop = async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        };

        let (run, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(running, stop)
        })
        .await
        .expect("targeted cancellation should stop the Native Agent promptly");

        assert!(run
            .unwrap()
            .result
            .unwrap_err()
            .contains("stream cancelled"));
        drop(store);
        std::fs::remove_dir_all(base).ok();
    }
}
