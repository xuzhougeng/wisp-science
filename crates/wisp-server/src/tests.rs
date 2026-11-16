use super::*;
use serde_json::json;
use std::{path::Path, sync::atomic::AtomicUsize};
use wisp_llm::{Completion, Message, Provider, StreamSink, ToolCall, ToolSchema};

struct FakeModel {
    calls: Arc<AtomicUsize>,
    write: bool,
}
#[async_trait]
impl Provider for FakeModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Saved result".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.write
            && !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
        {
            return Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "write-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "write".into(),
                        arguments: json!({"path":"answer.txt", "content":"offline result"})
                            .to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        }
        sink.on_text("Saved result");
        self.complete(messages, tools).await
    }
}

/// A model that, asked by a conversation, hands the work to a subagent;
/// as the subagent, answers it; and reviews and handles its report.
struct DispatchingModel;

#[async_trait]
impl Provider for DispatchingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Report: the subagent counted 42 reads.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let text = if said("[From the parent conversation's agent]") {
            "42 reads"
        } else if said("Report: the subagent counted") {
            "Handled the subagent's report."
        } else if messages.iter().any(|m| m.role == wisp_llm::Role::Tool) {
            "Started a subagent."
        } else {
            return Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "dispatch-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "dispatch_subagent".into(),
                        arguments: json!({"instruction": "Count the reads in data.fastq", "title": "Counting"})
                            .to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// A model that, asked to count reads, delegates a two-task batch; asked to
/// run an analysis, delegates a task that runs code; as each task, answers
/// it; and sums up what came back.
struct DelegatingModel;

#[async_trait]
impl Provider for DelegatingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "ok".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let delegate = |args: serde_json::Value| {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "delegate-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "delegate_tasks".into(),
                        arguments: args.to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            })
        };
        let last_tool = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::Tool)
            .map(|m| m.content.as_text());
        let text = if said("[From the parent conversation's agent: a delegated task]") {
            if said("Task: Add up the counts") {
                if said("42 reads") {
                    "{\"summary\": \"total 42\"}"
                } else {
                    "{\"summary\": \"no input\"}"
                }
            } else {
                "Counted.\n```json\n{\"summary\": \"42 reads\", \"evidence\": [\"wc -l\"]}\n```"
            }
        } else if said("<background_agent_completions>") {
            if said("total 42") {
                "Background: total 42"
            } else {
                "Background: it did not finish"
            }
        } else if let Some(result) = last_tool {
            if result.contains("\"completion_policy\":\"background\"") {
                "Started in the background."
            } else if result.contains("\"status\":\"denied\"") {
                "The batch was turned down."
            } else if result.contains("total 42") {
                "Synthesized: total 42"
            } else {
                "The batch failed."
            }
        } else if said("in the background") {
            return delegate(json!({
                "goal": "Count and add up the reads",
                "background": true,
                "tasks": [
                    {"id": "count", "instruction": "Count the reads in data.fastq", "capabilities": ["project_read"]},
                    {"id": "sum", "instruction": "Add up the counts", "capabilities": ["reasoning"], "depends_on": ["count"]},
                ],
            }));
        } else if said("run the analysis") {
            return delegate(json!({
                "goal": "Run the analysis",
                "tasks": [{"id": "run", "instruction": "Run the analysis script", "capabilities": ["code_run"]}],
            }));
        } else {
            return delegate(json!({
                "goal": "Count and add up the reads",
                "tasks": [
                    {"id": "count", "instruction": "Count the reads in data.fastq", "capabilities": ["project_read"]},
                    {"id": "sum", "instruction": "Add up the counts", "capabilities": ["reasoning"], "depends_on": ["count"]},
                ],
            }));
        };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Picks the skill to convert when asked, and writes a workflow of two
/// agents from it.
struct ConvertingModel;

#[async_trait]
impl Provider for ConvertingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
    ) -> wisp_llm::Result<Completion> {
        let system = messages[0].content.as_text();
        let contract = json!({
            "type": "object",
            "required": ["summary", "status", "artifacts"],
            "properties": {
                "summary": {"type": "string"},
                "status": {"const": "succeeded"},
                "artifacts": {"type": "array", "items": {"type": "string"}},
            },
        });
        let content = if system.contains("Select the smallest sufficient set") {
            json!({"skill_ids": ["read-counting"], "rationale": "It counts reads."}).to_string()
        } else if system.contains("Convert the supplied Skill method documents") {
            // It was given the skill's own words.
            assert!(messages[1]
                .content
                .as_text()
                .contains("Divide the line count by four"));
            json!({
                "goal": "Count the reads and report them",
                "context": "Report counts per file.",
                "approval_policy": "review_all",
                "tasks": [
                    {"id": "count_reads", "instruction": "Count the reads in every FASTQ file.",
                     "depends_on": [], "capabilities": ["project_read"], "skill_ids": [],
                     "specialist_id": null, "output_schema": contract, "isolated": false,
                     "model_id": null, "executor": null, "budget": null, "timeout_secs": null},
                    {"id": "report", "instruction": "Write the counts as a table.",
                     "depends_on": ["count_reads"], "capabilities": ["reasoning"], "skill_ids": [],
                     "specialist_id": null, "output_schema": contract, "isolated": false,
                     "model_id": null, "executor": null, "budget": null, "timeout_secs": null},
                ],
            })
            .to_string()
        } else {
            "{}".into()
        };
        Ok(Completion {
            content,
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        _: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        sink.on_text("ok");
        Ok(Completion {
            content: "ok".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Delegates, in the background, a task that hands back a frozen method
/// search contract and a search over it; proposes candidates when asked.
struct SearchingModel {
    spec: Arc<std::sync::Mutex<String>>,
    proposals: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for SearchingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
    ) -> wisp_llm::Result<Completion> {
        // Side calls (a title, follow-ups) are not candidates.
        let proposing = messages
            .iter()
            .any(|m| m.content.as_text().contains("frozen scientific evaluator"));
        if !proposing {
            return Ok(Completion {
                content: "{}".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            });
        }
        let added = self.proposals.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Completion {
            content: json!({
                "replacement": format!("def fit_model(rows):\n    return len(rows) + {added}\n"),
                "rationale": format!("candidate {added}"),
                "family": "linear",
            })
            .to_string(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let last_tool = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::Tool)
            .map(|m| m.content.as_text());
        let text = if said("Task: Freeze the contract") {
            json!({"method_search_spec_artifact_version_id": *self.spec.lock().unwrap()})
                .to_string()
        } else if said("<background_agent_completions>") {
            "Search reported.".to_owned()
        } else if last_tool.is_none() {
            return Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "delegate-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "delegate_tasks".into(),
                        arguments: json!({
                            "goal": "Find a better fit",
                            "background": true,
                            "tasks": [
                                {"id": "prep", "instruction": "Freeze the contract", "capabilities": ["reasoning"],
                                 "output_schema": {"type": "object",
                                    "properties": {"method_search_spec_artifact_version_id": {"type": "string"}},
                                    "required": ["method_search_spec_artifact_version_id"]}},
                                {"id": "search", "instruction": "Search for a better fit", "task_kind": "run_activity",
                                 "depends_on": ["prep"],
                                 "run_activity": {"activity": "method_search", "input_task_id": "prep",
                                    "max_candidates": 3, "max_wall_seconds": 3600,
                                    "max_evaluator_seconds": 30, "max_cost_microunits": 1000000}},
                            ],
                        })
                        .to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        } else {
            "Started in the background.".to_owned()
        };
        sink.on_text(&text);
        Ok(Completion {
            content: text,
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Delegates one isolated task that writes a note in its worktree.
struct IsolatingModel;

#[async_trait]
impl Provider for IsolatingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "ok".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let call = |name: &str, args: serde_json::Value| {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: format!("{name}-1"),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: name.into(),
                        arguments: args.to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            })
        };
        let last_tool = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::Tool)
            .map(|m| m.content.as_text());
        let text = if said("Task: Write the note") {
            match last_tool {
                None => {
                    return call(
                        "write",
                        json!({"path": "note.md", "content": "written in a worktree\n"}),
                    )
                }
                Some(_) => "{\"summary\": \"wrote the note\"}".to_owned(),
            }
        } else {
            match last_tool {
                None => {
                    return call(
                        "delegate_tasks",
                        json!({
                            "goal": "Write the note",
                            "tasks": [{"id": "note", "instruction": "Write the note", "capabilities": ["project_write"], "isolated": true}],
                        }),
                    )
                }
                Some(result) if result.contains("wrote the note") => "Merged.".to_owned(),
                Some(result) => format!("Failed: {result}"),
            }
        };
        sink.on_text(&text);
        Ok(Completion {
            content: text,
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Delegates a task that may delegate: the task hands the count to a task
/// of its own, which may not delegate again.
struct NestingModel;

#[async_trait]
impl Provider for NestingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "ok".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let can_delegate = tools
            .iter()
            .any(|tool| tool.function.name == "delegate_tasks");
        let delegate = |args: serde_json::Value| {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "delegate-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "delegate_tasks".into(),
                        arguments: args.to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            })
        };
        let last_tool = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::Tool)
            .map(|m| m.content.as_text());
        let text = if said("Task: Count the reads in data.fastq") {
            if can_delegate {
                "{\"summary\": \"could delegate again\"}".to_owned()
            } else {
                "{\"summary\": \"42 reads\"}".to_owned()
            }
        } else if said("Task: Lead the read count") {
            match last_tool {
                None if can_delegate => {
                    return delegate(json!({
                        "goal": "Count the reads",
                        "tasks": [{"id": "count", "instruction": "Count the reads in data.fastq", "capabilities": ["reasoning"]}],
                    }));
                }
                None => "{\"summary\": \"no delegation tool\"}".to_owned(),
                Some(result) if result.contains("42 reads") => {
                    "{\"summary\": \"led: 42\"}".to_owned()
                }
                Some(result) => format!(
                    "{{\"summary\": {}}}",
                    serde_json::to_string(&format!("nested failed: {result}")).unwrap()
                ),
            }
        } else {
            match last_tool {
                None => {
                    return delegate(json!({
                        "goal": "Lead the count",
                        "tasks": [{"id": "lead", "instruction": "Lead the read count", "capabilities": ["reasoning", "delegation"]}],
                    }));
                }
                Some(result) if result.contains("led: 42") => "Done: led 42".to_owned(),
                Some(result) => format!("The batch failed: {result}"),
            }
        };
        sink.on_text(&text);
        Ok(Completion {
            content: text,
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// The research assistant's model: lists the projects, dispatches a count to
/// one, and says it started; in the project, counts; asked to review the
/// outcome, reports it.
struct AssistingModel;

#[async_trait]
impl Provider for AssistingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Report: the project counted 42 reads.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let said = |needle: &str| {
            messages
                .iter()
                .any(|m| m.content.as_text().contains(needle))
        };
        let call = |name: &str, args: serde_json::Value| {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: format!("{name}-1"),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: name.into(),
                        arguments: args.to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            })
        };
        let results = messages
            .iter()
            .filter(|m| m.role == wisp_llm::Role::Tool)
            .count();
        let text = if said("[From the research assistant]") {
            "42 reads"
        } else if said("research assistant in Wisp Science") {
            // Only the assistant's own tools are offered to it.
            let names: Vec<&str> = tools
                .iter()
                .map(|tool| tool.function.name.as_str())
                .collect();
            if names
                .iter()
                .any(|name| matches!(*name, "write" | "edit" | "shell" | "read"))
            {
                "I was given project tools."
            } else if results == 0 {
                return call("research_projects", json!({}));
            } else if results == 1 {
                return call(
                    "dispatch_to_project",
                    json!({"project_id": PROJECT, "instruction": "Count the reads in data.fastq", "title": "Counting"}),
                );
            } else {
                "Started the count; I will report back."
            }
        } else {
            "ok"
        };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Says whether the request led with the research assistant's
/// instructions.
struct InstructionsCheckingModel;

#[async_trait]
impl Provider for InstructionsCheckingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "[]".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let told = messages.first().is_some_and(|message| {
            message.role == wisp_llm::Role::System
                && message
                    .content
                    .as_text()
                    .contains("research assistant in Wisp Science")
        });
        let text = if told { "told" } else { "not told" };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// Answers at once, but takes its time over a side question such as what
/// could be asked next.
struct SlowAfterwardsModel;

#[async_trait]
impl Provider for SlowAfterwardsModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        Ok(Completion {
            content: "[]".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        _: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        sink.on_text("An answer");
        Ok(Completion {
            content: "An answer".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

/// A mandate's agent: in a round it asks the researcher for the reference
/// genome; told where it is, it ends the round saying what it did.
struct MandateModel;

#[async_trait]
impl Provider for MandateModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "ok".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let last_user = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::User)
            .map(|m| m.content.as_text())
            .unwrap_or_default();
        let answered = messages
            .iter()
            .rev()
            .take_while(|m| m.role != wisp_llm::Role::User)
            .any(|m| m.role == wisp_llm::Role::Tool);
        let call = |name: &str, args: serde_json::Value| {
            Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: format!("{name}-1"),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: name.into(),
                        arguments: args.to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            })
        };
        let text = if answered {
            "Done for now."
        } else if last_user.contains("data/genome.fa") {
            return call(
                "end_round",
                json!({"done": "Indexed the genome", "next_step": "Align the reads"}),
            );
        } else if last_user.contains("Continue this mandate") {
            return call(
                "request_assistance",
                json!({"kind": "materials", "what": "Upload the reference genome"}),
            );
        } else {
            "ok"
        };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

pub(crate) fn config(root: &Path) -> Config {
    Config {
        data: root.join("data"),
        workspace: root.join("workspace"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: "https://test.invalid/".into(),
        code: wisp_sync::RemoteCode::generate().unwrap(),
        model: Some(crate::config::ModelConfig {
            provider: wisp_llm::ProviderConfig::openai(
                "http://127.0.0.1:9",
                "synthetic",
                "offline",
            ),
            max_context: 128_000,
            supports_vision: false,
        }),
        max_iterations: 5,
        skills: vec![],
        personal_skills: None,
    }
}

pub(crate) fn models(calls: Arc<AtomicUsize>, write: bool) -> ModelFactory {
    Arc::new(move |_| {
        Box::new(FakeModel {
            calls: calls.clone(),
            write,
        })
    })
}

#[tokio::test]
async fn a_new_project_gets_a_folder_named_after_it() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let parent = temp.path().join("research");
    std::fs::create_dir_all(&parent).unwrap();
    let id = server
        .create_local_project(&parent, " RNA: day 3 ", "  Time course  ", true)
        .await
        .unwrap();
    let root = std::fs::canonicalize(&parent).unwrap().join("RNA- day 3");
    assert!(root.is_dir());
    // The standard layout and its manifest, under the project's own name.
    assert!(root.join("data/raw").is_dir());
    assert!(root.join("results/tables").is_dir());
    let manifest = std::fs::read_to_string(root.join(".wisp/project.toml")).unwrap();
    assert!(manifest.contains("name = \"RNA: day 3\""), "{manifest}");
    let settings = server.project_settings(&id).await.unwrap();
    assert_eq!(settings.name, "RNA: day 3");
    assert_eq!(settings.description, "Time course");

    // The same folder again is refused; so is a name with no folder in it.
    assert!(server
        .create_local_project(&parent, "RNA: day 3", "", false)
        .await
        .is_err());
    assert!(server
        .create_local_project(&parent, " ... ", "", false)
        .await
        .is_err());
    assert!(server
        .create_local_project(&parent, "", "", false)
        .await
        .is_err());
    // Without the layout, the folder is left empty.
    let bare = server
        .create_local_project(&parent, "Notes", "", false)
        .await
        .unwrap();
    assert_ne!(bare, id);
    let notes = std::fs::canonicalize(&parent).unwrap().join("Notes");
    assert_eq!(std::fs::read_dir(&notes).unwrap().count(), 0);
    assert_eq!(desktop::project_folder_name("a/b\\c?"), "a-b-c-");
    server.shutdown().await;
}

#[tokio::test]
async fn local_project_registration_preserves_identity_and_is_not_remotely_exposed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let project = temp.path().join("研究 project");
    std::fs::create_dir(&project).unwrap();
    let first = server.register_local_project(&project).await.unwrap();
    assert_eq!(
        server
            .register_local_project(&project.join("."))
            .await
            .unwrap(),
        first
    );
    assert!(server.register_local_project(temp.path()).await.is_err());
    assert!(server
        .register_local_project(&temp.path().join("data"))
        .await
        .is_err());
    assert!(server
        .register_local_project(&temp.path().join("missing"))
        .await
        .is_err());
    assert!(!server.commands().contains(&"register_local_project"));
    let session = call(&server, "native_conversation_create", &first, json!({}))
        .await
        .unwrap();
    let result = call(
        &server,
        "native_conversation_snapshot",
        &first,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    assert_eq!(result["project_id"], first);
    server.shutdown().await;
}

async fn call(server: &Server, command: &str, project: &str, args: Value) -> Result<Value, String> {
    server
        .dispatch(RemoteRequest {
            nonce: "test".into(),
            seq: 1,
            id: uuid::Uuid::new_v4().to_string(),
            command: command.into(),
            project_id: Some(project.into()),
            args,
        })
        .await
}

async fn snapshot(server: &Server, id: &str) -> dto::Snapshot {
    serde_json::from_value(
        call(
            server,
            "native_conversation_snapshot",
            PROJECT,
            json!({"session_id":id}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

async fn wait_for(
    server: &Server,
    id: &str,
    ready: impl Fn(&dto::Snapshot) -> bool,
) -> dto::Snapshot {
    // Generous: a loaded CI runner can take seconds over several turns; a
    // turn that never gets there still fails.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let value = snapshot(server, id).await;
            if ready(&value) {
                return value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("offline turn must make progress")
}

#[tokio::test]
async fn mobile_project_list_omits_a_missing_registered_database() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let store = Store::open_application(&cfg.data.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .set_setting("decentralized_project_storage", "true")
        .await
        .unwrap();
    let workspace = root.path().join("offline");
    store
        .create_project("offline", "Offline", workspace.to_str().unwrap())
        .await
        .unwrap();
    let server = Server::open_with_models(cfg, models(Arc::new(AtomicUsize::new(0)), false))
        .await
        .unwrap();
    // Re-register after releasing the startup reader's handles, including on Windows.
    server.0.store.delete_project("offline").await.unwrap();
    server
        .0
        .store
        .register_project_folder(&workspace)
        .await
        .unwrap();
    let database = workspace.join(wisp_store::PROJECT_DATABASE);
    let saved = root.path().join("saved.sqlite");
    std::fs::rename(&database, &saved).unwrap();

    let projects = call(&server, "list_projects", PROJECT, json!({}))
        .await
        .unwrap();
    let rows = projects.as_array().unwrap();
    assert!(rows.iter().any(|row| row["id"] == PROJECT));
    assert!(rows.iter().all(|row| row["id"] != "offline"));
    assert!(server
        .0
        .store
        .list_projects()
        .await
        .unwrap()
        .iter()
        .any(|row| row.0 == "offline"));
    assert!(!database.exists());

    std::fs::rename(&saved, &database).unwrap();
    let projects = call(&server, "list_projects", PROJECT, json!({}))
        .await
        .unwrap();
    assert!(projects
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "offline"));
    server.shutdown().await;
}

#[tokio::test]
async fn a_document_of_the_project_reads_as_markdown() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), false))
        .await
        .unwrap();
    std::fs::create_dir_all(&cfg.workspace).unwrap();
    std::fs::write(
        cfg.workspace.join("notes.rtf"),
        r"{\rtf1\ansi Reads per sample \b were low\b0 in S1.\par}",
    )
    .unwrap();
    std::fs::write(cfg.workspace.join("plain.txt"), "just text").unwrap();
    let read = server
        .document_preview(PROJECT, "notes.rtf")
        .await
        .unwrap()
        .unwrap();
    assert!(read.contains("Reads per sample"), "{read}");
    assert!(read.contains("were low"), "{read}");
    // Not a document: nothing to convert; outside the project: refused.
    assert_eq!(
        server.document_preview(PROJECT, "plain.txt").await.unwrap(),
        None
    );
    assert!(server.document_preview(PROJECT, "../x.pdf").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_codex_conversation_is_imported_once_and_brought_forward_after() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(tmp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let home = tmp.path().join("home");
    let day = home.join(".codex/sessions/2026/11/14");
    std::fs::create_dir_all(&day).unwrap();
    let file = day.join("rollout-1.jsonl");
    let rollout = r#"{"timestamp":"2026-11-14T08:00:00Z","type":"session_meta","payload":{"id":"thread-1","cwd":"/work/rnaseq"}}
{"timestamp":"2026-11-14T08:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Align the reads"}]}}
{"timestamp":"2026-11-14T08:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Aligned."}]}}
"#;
    std::fs::write(&file, rollout).unwrap();
    let path = file.to_string_lossy().into_owned();
    let listed = server.external_in(PROJECT, home.clone()).await.unwrap();
    assert_eq!(
        (
            listed[0].source,
            listed[0].title.as_str(),
            listed[0].imported
        ),
        ("Codex", "Align the reads", false)
    );
    let (frame, outcome) = server
        .import_external_in(PROJECT, &home, &path)
        .await
        .unwrap();
    assert_eq!(outcome, desktop::ImportOutcome::Imported);
    // Not something waiting for whoever imported it.
    let inbox = call(&server, "native_conversation_inbox", PROJECT, json!({}))
        .await
        .unwrap();
    assert!(!inbox.to_string().contains(&frame), "{inbox}");
    let kept = server.whole_transcript(&frame).await.unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[1].content.as_text(), "Aligned.");
    // In a group named after the tool, and marked imported in the list.
    assert!(server
        .0
        .store
        .list_folders(PROJECT)
        .await
        .unwrap()
        .iter()
        .any(|(_, name, _)| name == "codex"));
    assert!(server.external_in(PROJECT, home.clone()).await.unwrap()[0].imported);
    // Again: nothing new, nothing done.
    let again = server
        .import_external_in(PROJECT, &home, &path)
        .await
        .unwrap();
    assert_eq!(again, (frame.clone(), desktop::ImportOutcome::Unchanged));
    // The conversation went on in Codex: what was added comes in.
    std::fs::write(
        &file,
        format!(
            "{rollout}{}\n",
            r#"{"timestamp":"2026-11-14T09:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Now count them"}]}}"#
        ),
    )
    .unwrap();
    let forward = server
        .import_external_in(PROJECT, &home, &path)
        .await
        .unwrap();
    assert_eq!(forward, (frame.clone(), desktop::ImportOutcome::Updated));
    assert_eq!(server.whole_transcript(&frame).await.unwrap().len(), 3);
    // Nothing outside the tools' folders, and nothing a phone can ask for.
    let stray = home.join("rollout-9.jsonl");
    std::fs::write(&stray, rollout).unwrap();
    assert!(server
        .import_external_in(PROJECT, &home, &stray.to_string_lossy())
        .await
        .is_err());
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("import_external")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_branch_is_brought_back_as_a_summary_the_user_approved() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::open_with_models(cfg, models(calls, false))
        .await
        .unwrap();
    let main = create(&server).await;
    send(&server, &main, "Look at the counts").await.unwrap();
    wait_for(&server, &main, |s| !s.running && !s.items.is_empty()).await;
    // Not a branch: nothing to bring back.
    assert!(server.branch_merge(PROJECT, &main).await.is_err());
    let branch = server
        .branch_conversation(PROJECT, &main, None)
        .await
        .unwrap();
    // Nothing done in it yet: nothing to summarise.
    let fresh = server.branch_merge(PROJECT, &branch).await.unwrap();
    assert_eq!(fresh.source, main);
    assert!(server
        .draft_branch_summary(PROJECT, &branch, &fresh.guard)
        .await
        .is_err());
    send(&server, &branch, "Try a log scale").await.unwrap();
    wait_for(&server, &branch, |s| !s.running && s.items.len() >= 4).await;
    let ready = server.branch_merge(PROJECT, &branch).await.unwrap();
    assert!(ready.new_messages >= 2, "{ready:?}");
    // A stale guard is refused before the model is asked.
    assert!(server
        .draft_branch_summary(PROJECT, &branch, &fresh.guard)
        .await
        .is_err());
    let draft = server
        .draft_branch_summary(PROJECT, &branch, &ready.guard)
        .await
        .unwrap();
    assert_eq!(draft, "Saved result");
    let into = server
        .merge_branch(
            PROJECT,
            &branch,
            &ready.guard,
            "Log scale shows the outlier.",
        )
        .await
        .unwrap();
    assert_eq!(into, main);
    let merged = server.whole_transcript(&main).await.unwrap();
    assert_eq!(
        merged.last().unwrap().content.as_text(),
        "Log scale shows the outlier."
    );
    // The conversation shows it as the branch's card, not as an answer.
    let shown = snapshot(&server, &main).await;
    let card = shown.items.last().unwrap();
    assert_eq!(card.role, "branch_merge");
    assert_eq!(card.text.trim(), "Log scale shows the outlier.");
    assert_eq!(card.input.as_deref(), Some(branch.as_str()));
    // Once only, and none of it is a phone's to do.
    assert!(server
        .merge_branch(PROJECT, &branch, &ready.guard, "Again")
        .await
        .is_err());
    assert!(server.branch_merge("other", &branch).await.is_err());
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("merge")));
    server.shutdown().await;
}

/// Proposes a fresh plan, then answers whatever the plan tool said.
struct Planner;
#[async_trait]
impl Provider for Planner {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Revised".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        if !messages
            .iter()
            .any(|message| message.role == wisp_llm::Role::Tool)
        {
            return Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "plan-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "update_plan".into(),
                        arguments: json!({"steps": [
                            {"step": "Align the reads", "status": "pending"},
                            {"step": "Count and plot", "status": "pending"}
                        ]})
                        .to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        }
        sink.on_text("Revised");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn a_proposed_plan_can_be_sent_back_with_what_should_change() {
    let tmp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(Planner));
    let server = Server::open_with_models(config(tmp.path()), factory)
        .await
        .unwrap();
    let id = create(&server).await;
    send(&server, &id, "Plan the analysis").await.unwrap();
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    let approval = &pending.approvals[0];
    assert!(approval
        .message
        .starts_with(wisp_tools::plan::PLAN_APPROVAL_PREFIX));
    assert!(server
        .answer_with_feedback(PROJECT, &id, &approval.approval_id, "  ")
        .await
        .is_err());
    assert!(server
        .answer_with_feedback(PROJECT, &id, "missing", "Split it")
        .await
        .is_err());
    server
        .answer_with_feedback(
            PROJECT,
            &id,
            &approval.approval_id,
            "Split counting from plotting",
        )
        .await
        .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.error.is_none(), "{:?}", finished.error);
    // The model is told the plan was turned down, and why.
    let refused = finished
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some("update_plan"))
        .unwrap();
    assert!(
        refused
            .text
            .contains("User feedback: Split counting from plotting"),
        "{}",
        refused.text
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_projects_approval_mode_and_tool_rules_decide_what_asks() {
    use desktop::{ApprovalScope, ToolRule};
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(tmp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let start = server.project_approvals(PROJECT).await.unwrap();
    assert_eq!(start.scope, ApprovalScope::Ask);
    assert!(start.rules.is_empty());
    assert!(server
        .set_tool_rule(PROJECT, " ", Some(ToolRule::Ask))
        .await
        .is_err());
    assert!(server
        .set_approval_scope("missing", ApprovalScope::Full)
        .await
        .is_err());

    // A denied tool does not run, and nothing asks about it.
    let rules = server
        .set_tool_rule(PROJECT, "write", Some(ToolRule::Deny))
        .await
        .unwrap();
    assert_eq!(rules.rules.get("write"), Some(&ToolRule::Deny));
    let denied = create(&server).await;
    send(&server, &denied, "Write the result").await.unwrap();
    let done = wait_for(&server, &denied, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    let blocked = done
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some("write"))
        .unwrap();
    assert!(
        blocked.text.contains("blocked by the approval policy"),
        "{}",
        blocked.text
    );
    assert!(!server.0.config.workspace.join("answer.txt").exists());

    // Auto-approve: a change runs without the question before it.
    server.set_tool_rule(PROJECT, "write", None).await.unwrap();
    let auto = server
        .set_approval_scope(PROJECT, ApprovalScope::Auto)
        .await
        .unwrap();
    assert_eq!(auto.scope, ApprovalScope::Auto);
    assert!(auto.rules.is_empty());
    let free = create(&server).await;
    send(&server, &free, "Write the result").await.unwrap();
    let done = wait_for(&server, &free, |s| !s.running).await;
    assert!(done.error.is_none(), "{:?}", done.error);
    assert!(server.0.config.workspace.join("answer.txt").is_file());

    // Kept under the WebView's keys, and its default is read for a project
    // without its own.
    let store = &server.0.store;
    assert_eq!(
        store
            .get_setting(&format!("approval_scope:{PROJECT}"))
            .await
            .unwrap()
            .as_deref(),
        Some("\"auto\"")
    );
    store
        .set_setting("approval_scope", "\"full\"")
        .await
        .unwrap();
    store
        .set_setting("tool_approvals", r#"{"pubmed_search":"ask","odd":"maybe"}"#)
        .await
        .unwrap();
    // A connector set to skip approvals is kept in the WebView's list.
    let connector = server.connectors().await.unwrap()[0].domain.clone();
    let skipped = server
        .set_connector_skip(PROJECT, &connector, true)
        .await
        .unwrap();
    assert!(skipped.skip.contains(&connector));
    assert!(server
        .set_connector_skip(PROJECT, "no-such-connector", true)
        .await
        .is_err());
    assert!(!server
        .set_connector_skip(PROJECT, &connector, false)
        .await
        .unwrap()
        .skip
        .contains(&connector));
    let inherited = crate::approvals::load(store, "another").await;
    assert_eq!(inherited.scope, ApprovalScope::Full);
    assert_eq!(
        inherited.rules.into_iter().collect::<Vec<_>>(),
        [("pubmed_search".to_owned(), ToolRule::Ask)]
    );
    server.shutdown().await;
}

fn command_hook(event: wisp_dto::HookEvent, matcher: &str, command: &str) -> wisp_dto::CommandHook {
    wisp_dto::CommandHook {
        event,
        matcher: matcher.into(),
        command: command.into(),
        enabled: true,
        timeout: Some(20),
    }
}

#[tokio::test]
async fn a_prompt_hook_adds_context_or_refuses_the_message() {
    use wisp_dto::HookEvent;
    let tmp = tempfile::tempdir().unwrap();
    let shown: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Recorder(record.clone())));
    let server = Server::open_with_models(config(tmp.path()), factory)
        .await
        .unwrap();
    assert!(server
        .set_command_hooks(vec![command_hook(HookEvent::PreToolUse, "(", "x")])
        .await
        .is_err());
    // Runs unchanged under sh and PowerShell.
    let saved = server
        .set_command_hooks(vec![command_hook(
            HookEvent::UserPromptSubmit,
            "ignored",
            "  echo queue: gpu  ",
        )])
        .await
        .unwrap();
    assert_eq!(saved[0].command, "echo queue: gpu");
    assert_eq!(saved[0].matcher, "");
    assert_eq!(server.command_hooks().await.unwrap(), saved);
    let id = create(&server).await;
    send(&server, &id, "Count the reads").await.unwrap();
    wait_for(&server, &id, |s| !s.running).await;
    let told = shown.lock().unwrap().last().cloned().unwrap().join("\n");
    assert!(told.contains("queue: gpu"), "{told}");
    // Exit 2 refuses the message, and no turn starts.
    server
        .set_command_hooks(vec![command_hook(
            HookEvent::UserPromptSubmit,
            "",
            "exit 2",
        )])
        .await
        .unwrap();
    let refused = send(&server, &id, "Again").await.unwrap_err();
    assert!(
        refused.contains("Blocked by a UserPromptSubmit hook"),
        "{refused}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_tool_hook_blocks_the_call_and_a_project_file_runs_once_trusted() {
    use wisp_dto::HookEvent;
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(tmp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    // A PreToolUse hook that fails closes the call before any question.
    server
        .set_command_hooks(vec![command_hook(
            HookEvent::PreToolUse,
            "write|edit",
            "exit 3",
        )])
        .await
        .unwrap();
    let id = create(&server).await;
    send(&server, &id, "Write the result").await.unwrap();
    let done = wait_for(&server, &id, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    let blocked = done
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some("write"))
        .unwrap();
    assert!(blocked.text.contains("fail closed"), "{}", blocked.text);
    assert!(!server.0.config.workspace.join("answer.txt").exists());
    let failures = server
        .conversation_status(PROJECT, &id)
        .await
        .unwrap()
        .hook_failures;
    assert_eq!(failures.len(), 1, "{failures:?}");

    // A project's own file is not run until it is trusted as it is.
    server.set_command_hooks(vec![]).await.unwrap();
    assert!(server.project_hooks(PROJECT).await.unwrap().is_none());
    let file = server.0.config.workspace.join(".wisp/hooks.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"{"hooks":{"PreToolUse":[{"matcher":"write","hooks":[{"type":"command","command":"exit 2"}]}]}}"#,
    )
    .unwrap();
    let shown = server.project_hooks(PROJECT).await.unwrap().unwrap();
    assert!(!shown.trusted && shown.error.is_none());
    assert_eq!(shown.hooks.len(), 1);
    let untrusted = create(&server).await;
    send(&server, &untrusted, "Write the result").await.unwrap();
    let pending = wait_for(&server, &untrusted, |s| !s.approvals.is_empty()).await;
    assert!(server
        .conversation_status(PROJECT, &untrusted)
        .await
        .unwrap()
        .hook_failures
        .iter()
        .any(|failure| failure.contains("was not run")));
    drop(pending);
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id": untrusted}),
    )
    .await
    .unwrap();
    wait_for(&server, &untrusted, |s| !s.running).await;
    // Trust is for the content shown: a stale one is refused.
    assert!(server
        .set_project_hooks_trust(PROJECT, Some("stale".into()))
        .await
        .is_err());
    let trusted = server
        .set_project_hooks_trust(PROJECT, Some(shown.sha256.clone()))
        .await
        .unwrap()
        .unwrap();
    assert!(trusted.trusted);
    let guarded = create(&server).await;
    send(&server, &guarded, "Write the result").await.unwrap();
    let done = wait_for(&server, &guarded, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    // Changed, it is no longer trusted.
    std::fs::write(&file, r#"{"hooks":{}}"#).unwrap();
    assert!(
        !server
            .project_hooks(PROJECT)
            .await
            .unwrap()
            .unwrap()
            .trusted
    );
    assert!(
        !server
            .set_project_hooks_trust(PROJECT, None)
            .await
            .unwrap()
            .unwrap()
            .trusted
    );
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_stop_hook_keeps_the_turn_going_once() {
    use wisp_dto::HookEvent;
    let tmp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::open_with_models(config(tmp.path()), models(calls.clone(), false))
        .await
        .unwrap();
    // Blocks while it has not yet asked once, then lets the turn end.
    server
        .set_command_hooks(vec![command_hook(
            HookEvent::Stop,
            "",
            r#"grep -q '"stop_hook_active":false' && { echo "run the tests" >&2; exit 2; }; exit 0"#,
        )])
        .await
        .unwrap();
    let id = create(&server).await;
    send(&server, &id, "Finish up").await.unwrap();
    // The first turn, then the one the hook asked for.
    let done = wait_for(&server, &id, |s| {
        !s.running && calls.load(Ordering::Relaxed) >= 2
    })
    .await;
    assert!(done.error.is_none(), "{:?}", done.error);
    // It stopped asking: nothing failed and no third turn ran.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(server
        .conversation_status(PROJECT, &id)
        .await
        .unwrap()
        .hook_failures
        .is_empty());
    server.shutdown().await;
}

/// Browses the connector tools and loads the skill called "other", then
/// records both results.
struct ToolLister(Arc<std::sync::Mutex<Vec<(String, String)>>>);
#[async_trait]
impl Provider for ToolLister {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "noted".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let result = |tool: &str| {
            messages
                .iter()
                .rev()
                .find(|message| message.tool_name.as_deref() == Some(tool))
                .map(|message| message.content.as_text())
        };
        let (Some(found), Some(skills)) = (result("search_mcp_tools"), result("use_skill")) else {
            let call = |id: &str, name: &str, arguments: serde_json::Value| ToolCall {
                id: id.into(),
                kind: "function".into(),
                function: wisp_llm::FunctionCall {
                    name: name.into(),
                    arguments: arguments.to_string(),
                },
            };
            return Ok(Completion {
                tool_calls: vec![
                    call("browse-1", "search_mcp_tools", json!({"query": "*"})),
                    call("skills-1", "use_skill", json!({"name": "other"})),
                ],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        };
        self.0.lock().unwrap().push((found, skills));
        sink.on_text("noted");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn an_expert_keeps_to_the_skills_and_connectors_it_was_given() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<(String, String)>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(ToolLister(record.clone())));
    let cfg = config(temp.path());
    for (name, about) in [
        ("demo", "Count reads the lab way"),
        ("other", "Plot volcanoes the lab way"),
    ] {
        let folder = cfg.workspace.join("skills").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {about}\n---\nDo it.\n"),
        )
        .unwrap();
    }
    let server = Server::open_with_models(cfg, factory).await.unwrap();
    let experts = server
        .save_expert(desktop::ExpertInput {
            id: String::new(),
            name: "Counter".into(),
            description: String::new(),
            instructions: "Count.".into(),
            model: String::new(),
            skills: Some(vec![" demo ".into(), "demo".into()]),
            connectors: Some(vec![]),
        })
        .await
        .unwrap();
    let counter = experts
        .iter()
        .find(|e| e.name == "Counter")
        .unwrap()
        .clone();
    // Kept trimmed and once each; no connector at all is not every one.
    assert_eq!(counter.skills, Some(vec!["demo".to_owned()]));
    assert_eq!(counter.connectors, Some(vec![]));
    let last = || seen.lock().unwrap().last().cloned().unwrap();
    // Whether browsing the connector tools found any.
    let bundled = |found: &str| found.contains("\"tool_name\"");

    // Without an expert: every skill and the bundled connectors.
    let plain = create(&server).await;
    send(&server, &plain, "hello").await.unwrap();
    wait_for(&server, &plain, |s| !s.running).await;
    let (found, skills) = last();
    assert!(bundled(&found), "{found}");
    // Its instructions, when the skill can be used.
    let loaded = |skills: &str| skills.contains("Do it.");
    assert!(loaded(&skills), "{skills}");

    // As the expert: its skill only, and no connector.
    let session = create(&server).await;
    let options = server
        .conversation_status(PROJECT, &session)
        .await
        .unwrap()
        .options;
    server
        .set_conversation_options(
            PROJECT,
            &session,
            desktop::ConversationOptions {
                expert: counter.id.clone(),
                ..options
            },
        )
        .await
        .unwrap();
    send(&server, &session, "hello").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let (found, skills) = last();
    assert!(!bundled(&found), "{found}");
    assert!(!loaded(&skills), "{skills}");
    server.shutdown().await;
}

#[tokio::test]
async fn guidance_is_read_by_the_running_turn_and_refused_when_none_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::open_with_models(cfg.clone(), models(calls.clone(), true))
        .await
        .unwrap();
    let id = create(&server).await;
    // Nothing runs: the caller sends it as a turn of its own.
    assert!(!server.guide(PROJECT, &id, "Use GRCh38").await.unwrap());
    assert!(server.guide(PROJECT, &id, "  ").await.is_err());
    send(&server, &id, "Write the result").await.unwrap();
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    assert!(server.guide(PROJECT, &id, "Use GRCh38").await.unwrap());
    // Another project's conversation is not this one's.
    assert!(server.guide("other", &id, "Use GRCh38").await.is_err());
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        json!({"session_id": id, "approval_id": pending.approvals[0].approval_id, "approved": true}),
    )
    .await
    .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.error.is_none(), "{:?}", finished.error);
    // Read in the same turn: after the tool it was waiting on, before the
    // answer, with no turn of its own.
    let at = |found: &dyn Fn(&dto::Item) -> bool| finished.items.iter().position(found).unwrap();
    let tool = at(&|item| item.tool_name.as_deref() == Some("write"));
    let guided = at(&|item| item.role == "user" && item.text == "Use GRCh38");
    let answer = at(&|item| item.role == "assistant" && item.text == "Saved result");
    assert!(tool < guided && guided < answer, "{tool} {guided} {answer}");
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    // Once it is over, guidance is the caller's again.
    assert!(!server.guide(PROJECT, &id, "Thanks").await.unwrap());
    // A desktop call, not one a phone can make.
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("guid")));
    server.shutdown().await;
}

#[tokio::test]
async fn queued_messages_go_one_at_a_time_as_turns_end_without_a_window() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(tmp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let id = create(&server).await;
    let queued = |text: &str| dto::SendRequest {
        session_id: id.clone(),
        request_id: String::new(),
        message: text.into(),
        resume: false,
        resume_after: None,
        attachments: vec![],
        references: vec![],
        acp_agent_id: None,
    };
    send(&server, &id, "Write the result").await.unwrap();
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    let first = server
        .queue_message(PROJECT, queued("Now plot it"))
        .await
        .unwrap();
    let second = server
        .queue_message(PROJECT, queued("And summarise"))
        .await
        .unwrap();
    let third = server
        .queue_message(PROJECT, queued("Use GRCh38"))
        .await
        .unwrap();
    assert!(server.queue_message(PROJECT, queued("  ")).await.is_err());
    assert!(server.queue_message("other", queued("x")).await.is_err());
    let order =
        |list: Vec<desktop::QueuedMessage>| list.into_iter().map(|q| q.id).collect::<Vec<_>>();
    assert_eq!(
        order(
            server
                .conversation_status(PROJECT, &id)
                .await
                .unwrap()
                .queued
        ),
        [first, second, third]
    );
    // Moved up, then taken back out.
    assert!(server.move_queued_up(PROJECT, &id, second).await.unwrap());
    assert!(!server.move_queued_up(PROJECT, &id, second).await.unwrap());
    let taken = server
        .take_queued(PROJECT, &id, second)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(taken.message, "And summarise");
    assert!(server
        .take_queued(PROJECT, &id, second)
        .await
        .unwrap()
        .is_none());
    // Words alone can be given to the running turn now.
    assert!(server.guide_with_queued(PROJECT, &id, third).await.unwrap());
    assert_eq!(
        order(server.queued_messages(PROJECT, &id).await.unwrap()),
        [first]
    );
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        json!({"session_id": id, "approval_id": pending.approvals[0].approval_id, "approved": true}),
    )
    .await
    .unwrap();
    // The turn ends and the next message goes by itself, with nobody
    // looking at the conversation.
    let said = |s: &dto::Snapshot, text: &str| {
        s.items
            .iter()
            .any(|item| item.role == "user" && item.text == text)
    };
    let done = wait_for(&server, &id, |s| !s.running && said(s, "Now plot it")).await;
    assert!(done.error.is_none(), "{:?}", done.error);
    assert!(server
        .queued_messages(PROJECT, &id)
        .await
        .unwrap()
        .is_empty());
    // One queued while nothing runs goes at once.
    server
        .queue_message(PROJECT, queued("One more"))
        .await
        .unwrap();
    wait_for(&server, &id, |s| !s.running && said(s, "One more")).await;
    // Nothing waits: there is nothing to send now.
    assert!(!server.send_queued_now(PROJECT, &id).await.unwrap());
    assert!(server.send_queued_now("other", &id).await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn mobile_turn_approval_persistence_and_restart_reuse_the_same_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::open_with_models(cfg.clone(), models(calls.clone(), true))
        .await
        .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let send = json!({"session_id":id,"request_id":"request-1","message":"Write the result"});
    // Drop the submitter's reply. The accepted task lives in the server.
    drop(
        call(&server, "native_conversation_send", PROJECT, send.clone())
            .await
            .unwrap(),
    );
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    let wire = serde_json::to_value(&pending).unwrap();
    assert!(wire.get("exploration").is_none());
    assert!(wire.get("branches").is_none());
    assert!(!cfg.workspace.join("answer.txt").exists());
    assert_eq!(pending.approvals[0].tool, "write");
    assert!(pending.approvals[0].preview.contains("offline result"));
    let approval =
        json!({"session_id":id,"approval_id":pending.approvals[0].approval_id,"approved":true});
    let mut broad = approval.clone();
    broad["scope"] = "global".into();
    assert!(call(&server, "native_conversation_approve", PROJECT, broad)
        .await
        .is_err());
    assert!(call(
        &server,
        "native_conversation_snapshot",
        "other",
        json!({"session_id":id})
    )
    .await
    .is_err());
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        approval.clone(),
    )
    .await
    .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.error.is_none(), "{:?}", finished.error);
    assert!(finished
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    assert_eq!(
        std::fs::read_to_string(cfg.workspace.join("answer.txt")).unwrap(),
        "offline result"
    );
    assert!(finished
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    assert!(
        call(&server, "native_conversation_approve", PROJECT, approval)
            .await
            .is_err()
    );
    let count = calls.load(Ordering::Relaxed);
    call(&server, "native_conversation_send", PROJECT, send.clone())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), count);
    server.shutdown().await;
    drop(server);
    let reopened = Server::open_with_models(cfg, models(calls.clone(), false))
        .await
        .unwrap();
    let restored = snapshot(&reopened, &id).await;
    assert!(restored
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    assert!(restored
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    call(&reopened, "native_conversation_send", PROJECT, send.clone())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), count);
    let mut changed = send;
    changed["message"] = "different".into();
    assert!(
        call(&reopened, "native_conversation_send", PROJECT, changed)
            .await
            .is_err()
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn snapshot_during_turn_completion_includes_the_final_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(
        config(tmp.path()),
        models(Arc::new(AtomicUsize::new(0)), true),
    )
    .await
    .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"snapshot-race","message":"write"}),
    )
    .await
    .unwrap();
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;

    // Hold a snapshot after its transcript read, then finish and persist the turn.
    // A completed snapshot must not reuse the earlier, incomplete transcript.
    let (loaded_tx, loaded_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    *server.session(&id).snapshot_after_read.lock().unwrap() = Some((loaded_tx, resume_rx));
    let reading = {
        let server = server.clone();
        let id = id.clone();
        tokio::spawn(async move { snapshot(&server, &id).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), loaded_rx)
        .await
        .unwrap()
        .unwrap();
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        json!({"session_id":id,"approval_id":pending.approvals[0].approval_id,"approved":true}),
    )
    .await
    .unwrap();
    wait_for(&server, &id, |s| !s.running).await;
    resume_tx.send(()).unwrap();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(5), reading)
        .await
        .unwrap()
        .unwrap();
    assert!(!finished.running);
    assert!(finished.error.is_none(), "{:?}", finished.error);
    assert!(finished
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    assert!(finished
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    server.shutdown().await;
}

#[tokio::test]
async fn stop_during_approval_never_executes_the_write() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::new(AtomicUsize::new(0)), true))
        .await
        .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"r","message":"write"}),
    )
    .await
    .unwrap();
    wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id":id}),
    )
    .await
    .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.approvals.is_empty());
    assert!(!cfg.workspace.join("answer.txt").exists());
    server.shutdown().await;
}

#[tokio::test]
async fn one_server_owns_each_database_and_shutdown_rejects_new_work() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let server =
        Server::open_with_models(cfg.clone(), models(Arc::new(AtomicUsize::new(0)), false))
            .await
            .unwrap();
    assert!(Server::open(cfg).await.is_err());
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    server.shutdown().await;
    assert!(call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"r","message":"hello"})
    )
    .await
    .is_err());
    assert!(call(&server, "arbitrary_command", PROJECT, json!({}))
        .await
        .is_err());
}

async fn send(server: &Server, session: &str, message: &str) -> Result<Value, String> {
    call(
        server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id": session, "request_id": uuid::Uuid::new_v4().to_string(), "message": message}),
    )
    .await
}

async fn create(server: &Server) -> String {
    call(server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn desktop_manages_conversations_without_exposing_it_remotely() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let alpha = create(&server).await;
    let beta = create(&server).await;
    send(&server, &alpha, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &alpha, |s| !s.running).await;

    server
        .rename_conversation(PROJECT, &beta, "  基因组 notes  ")
        .await
        .unwrap();
    assert!(server
        .rename_conversation(PROJECT, &beta, " ")
        .await
        .is_err());
    assert!(server
        .rename_conversation(PROJECT, &beta, &"x".repeat(201))
        .await
        .is_err());
    assert!(server
        .rename_conversation("other-project", &beta, "stolen")
        .await
        .is_err());
    server.pin_conversation(PROJECT, &beta, true).await.unwrap();

    let recent = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(recent.pinned, [beta.clone()]);
    let title = |list: &desktop::ConversationList, id: &str| {
        list.rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.title.clone())
    };
    assert_eq!(title(&recent, &beta).as_deref(), Some("基因组 notes"));
    assert!(title(&recent, &alpha).is_some());

    // Keyword search reads message text, not only titles.
    let found = server
        .conversations(PROJECT, " alignment ", false)
        .await
        .unwrap();
    assert!(title(&found, &alpha).is_some());
    assert!(title(&found, &beta).is_none());

    server
        .archive_conversation(PROJECT, &alpha, true)
        .await
        .unwrap();
    let recent = server.conversations(PROJECT, "", false).await.unwrap();
    assert!(title(&recent, &alpha).is_none());
    let archive = server.conversations(PROJECT, "", true).await.unwrap();
    assert_eq!(archive.rows.len(), 1);
    assert!(archive.rows[0].shelved && archive.rows[0].id == alpha);
    // An archived conversation is still found by keyword and can come back.
    let found = server
        .conversations(PROJECT, "alignment", false)
        .await
        .unwrap();
    assert!(found.rows.iter().any(|row| row.id == alpha && row.shelved));
    server
        .archive_conversation(PROJECT, &alpha, false)
        .await
        .unwrap();
    assert!(title(
        &server.conversations(PROJECT, "", false).await.unwrap(),
        &alpha
    )
    .is_some());

    assert!(server
        .delete_conversation("other-project", &alpha)
        .await
        .is_err());
    server.delete_conversation(PROJECT, &alpha).await.unwrap();
    assert!(server.delete_conversation(PROJECT, &alpha).await.is_err());
    assert!(call(
        &server,
        "native_conversation_snapshot",
        PROJECT,
        json!({"session_id": alpha}),
    )
    .await
    .is_err());
    let left = server.conversations(PROJECT, "", false).await.unwrap();
    assert!(title(&left, &alpha).is_none() && title(&left, &beta).is_some());

    // None of this is a remote command.
    for command in [
        "native_conversation_rename",
        "native_conversation_pin",
        "native_conversation_delete",
        "native_project_session_shelve",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, PROJECT, json!({"session_id": beta}))
            .await
            .is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_running_conversation_is_not_archived_or_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "write the answer").await.unwrap();
    // The write tool waits for approval, which keeps the turn running.
    wait_for(&server, &session, |s| !s.approvals.is_empty()).await;
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    for result in [
        server.archive_conversation(PROJECT, &session, true).await,
        server.delete_conversation(PROJECT, &session).await,
        server
            .transfer_conversation(PROJECT, &session, &lab, false)
            .await
            .map(|_| ()),
        server
            .transfer_conversation(PROJECT, &session, &lab, true)
            .await
            .map(|_| ()),
        server
            .rewind_conversation(
                PROJECT,
                &session,
                &desktop::TurnRef {
                    from_end: 0,
                    text: "write the answer".into(),
                },
            )
            .await
            .map(|_| ()),
        server
            .branch_conversation(PROJECT, &session, None)
            .await
            .map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("Stop the running"));
    }
    // A batch reports the refusal instead of failing as a whole.
    let report = server
        .batch(PROJECT, &[session.clone()], desktop::BatchAction::Delete)
        .await
        .unwrap();
    assert!(report.done.is_empty());
    assert!(report.refused[0].1.contains("Stop the running"));
    server
        .pin_conversation(PROJECT, &session, true)
        .await
        .unwrap();
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    server.delete_conversation(PROJECT, &session).await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_groups_batches_and_moves_conversations_between_projects() {
    use desktop::{BatchAction, Folder};
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let alpha = create(&server).await;
    let beta = create(&server).await;
    let gamma = create(&server).await;
    send(&server, &alpha, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &alpha, |s| !s.running).await;

    // Groups: named, unique per project, in the order the user gives them.
    let screens = server.create_folder(PROJECT, "  Screens ").await.unwrap();
    assert_eq!(screens.name, "Screens");
    let later = server.create_folder(PROJECT, "Later").await.unwrap();
    for name in ["", "  ", "screens", &"x".repeat(61)] {
        assert!(server.create_folder(PROJECT, name).await.is_err(), "{name}");
    }
    assert!(server.create_folder("missing", "Screens").await.is_err());
    server
        .reorder_folders(PROJECT, &[later.id.clone(), screens.id.clone()])
        .await
        .unwrap();
    server
        .rename_folder(PROJECT, &later.id, "Someday")
        .await
        .unwrap();
    // A group may keep its own name, but not take another's.
    server
        .rename_folder(PROJECT, &later.id, "someday")
        .await
        .unwrap();
    assert!(server
        .rename_folder(PROJECT, &later.id, "Screens")
        .await
        .is_err());
    assert!(server
        .rename_folder(PROJECT, "missing", "Other")
        .await
        .is_err());

    server
        .group_conversation(PROJECT, &alpha, Some(&screens.id))
        .await
        .unwrap();
    assert!(server
        .group_conversation(PROJECT, &alpha, Some("missing"))
        .await
        .is_err());
    assert!(server
        .group_conversation("other-project", &alpha, None)
        .await
        .is_err());
    let report = server
        .batch(
            PROJECT,
            &[beta.clone(), gamma.clone(), "missing".into(), beta.clone()],
            BatchAction::Group(Some(later.id.clone())),
        )
        .await
        .unwrap();
    assert_eq!(report.done, [beta.clone(), gamma.clone()]);
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].0, "missing");
    assert!(server
        .batch(
            PROJECT,
            &[beta.clone()],
            BatchAction::Group(Some("missing".into()))
        )
        .await
        .is_err());

    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(
        list.folders,
        [
            Folder {
                id: later.id.clone(),
                name: "someday".into()
            },
            screens.clone()
        ]
    );
    assert_eq!(list.grouped.get(&alpha), Some(&screens.id));
    assert_eq!(list.grouped.get(&beta), Some(&later.id));
    assert_eq!(list.grouped.get(&gamma), Some(&later.id));

    // Taking a conversation out of its group, and deleting a group, keep
    // the conversations.
    server
        .batch(PROJECT, &[gamma.clone()], BatchAction::Group(None))
        .await
        .unwrap();
    server.delete_folder(PROJECT, &later.id).await.unwrap();
    assert!(server.delete_folder(PROJECT, &later.id).await.is_err());
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.folders, [screens.clone()]);
    assert_eq!(list.rows.len(), 3);
    assert_eq!(list.grouped.len(), 1);

    // Archive, restore and delete several at once.
    let both = [beta.clone(), gamma.clone()];
    let report = server
        .batch(PROJECT, &both, BatchAction::Archive)
        .await
        .unwrap();
    assert_eq!(report.done, both);
    let archived = server.conversations(PROJECT, "", true).await.unwrap();
    assert_eq!(archived.rows.len(), 2);
    server
        .batch(PROJECT, &[beta.clone()], BatchAction::Restore)
        .await
        .unwrap();
    let report = server
        .batch(PROJECT, &[gamma.clone()], BatchAction::Delete)
        .await
        .unwrap();
    assert_eq!(report.done, [gamma.clone()]);
    let ids = |list: desktop::ConversationList| {
        let mut ids: Vec<_> = list.rows.into_iter().map(|row| row.id).collect();
        ids.sort();
        ids
    };
    let mut kept = vec![alpha.clone(), beta.clone()];
    kept.sort();
    assert_eq!(
        ids(server.conversations(PROJECT, "", false).await.unwrap()),
        kept
    );
    assert!(server
        .conversations(PROJECT, "", true)
        .await
        .unwrap()
        .rows
        .is_empty());

    // How the sidebar is arranged is remembered, in its own key space.
    assert_eq!(server.preference("sidebar").await.unwrap(), None);
    server.set_preference("sidebar", "date,name").await.unwrap();
    assert_eq!(
        server.preference("sidebar").await.unwrap().as_deref(),
        Some("date,name")
    );
    for name in ["", "desktop.models", "Side Bar", &"x".repeat(41)] {
        assert!(server.set_preference(name, "x").await.is_err(), "{name}");
        assert!(server.preference(name).await.is_err(), "{name}");
    }
    assert!(server
        .set_preference("sidebar", &"x".repeat(2_001))
        .await
        .is_err());

    // Copy and move to another project. The transcript goes; the choice to
    // stop asking for approvals does not.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    server
        .set_conversation_options(
            PROJECT,
            &alpha,
            desktop::ConversationOptions {
                full_access: true,
                plan_first: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(server
        .transfer_conversation(PROJECT, &alpha, PROJECT, true)
        .await
        .is_err());
    assert!(server
        .transfer_conversation(PROJECT, &alpha, "missing", true)
        .await
        .is_err());
    assert!(server
        .transfer_conversation(&lab, &alpha, PROJECT, true)
        .await
        .is_err());
    let transcript = |project: String, id: String| {
        let server = &server;
        async move {
            let value = call(
                server,
                "native_conversation_snapshot",
                &project,
                json!({"session_id": id}),
            )
            .await
            .unwrap();
            serde_json::from_value::<dto::Snapshot>(value)
                .unwrap()
                .items
                .into_iter()
                .map(|item| item.text)
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let original = transcript(PROJECT.into(), alpha.clone()).await;
    assert!(original.contains("sequence alignment plan"));

    let copy = server
        .transfer_conversation(PROJECT, &alpha, &lab, true)
        .await
        .unwrap();
    assert_ne!(copy, alpha);
    assert_eq!(transcript(lab.clone(), copy.clone()).await, original);
    assert_eq!(transcript(PROJECT.into(), alpha.clone()).await, original);
    let options = server
        .conversation_status(&lab, &copy)
        .await
        .unwrap()
        .options;
    assert!(options.plan_first);
    assert!(!options.full_access);
    // The original still has an unread reply; the copy does not ask for
    // attention in its new project.
    let status = |list: desktop::ConversationList, id: &str| {
        list.rows
            .into_iter()
            .find(|row| row.id == id)
            .map(|row| row.status)
    };
    assert_eq!(
        status(
            server.conversations(PROJECT, "", false).await.unwrap(),
            &alpha
        )
        .as_deref(),
        Some("needs_you")
    );
    assert_eq!(
        status(server.conversations(&lab, "", false).await.unwrap(), &copy).as_deref(),
        Some("complete")
    );

    let moved = server
        .transfer_conversation(PROJECT, &alpha, &lab, false)
        .await
        .unwrap();
    assert_eq!(transcript(lab.clone(), moved.clone()).await, original);
    assert!(server.conversation_status(PROJECT, &alpha).await.is_err());
    assert_eq!(
        ids(server.conversations(PROJECT, "", false).await.unwrap()),
        [beta.clone()]
    );
    let mut arrived = vec![copy, moved];
    arrived.sort();
    assert_eq!(
        ids(server.conversations(&lab, "", false).await.unwrap()),
        arrived
    );
    server.shutdown().await;
}

#[tokio::test]
async fn rewind_and_branch_act_on_the_named_message_or_change_nothing() {
    use desktop::TurnRef;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    for message in ["first question", "second question", "third question"] {
        send(&server, &session, message).await.unwrap();
        wait_for(&server, &session, |s| !s.running).await;
    }
    // The transcript as a client sees it: who said what, in order.
    let said = |id: String| {
        let server = &server;
        async move {
            snapshot(server, &id)
                .await
                .items
                .into_iter()
                .map(|item| format!("{}: {}", item.role, item.text))
                .collect::<Vec<_>>()
        }
    };
    let whole = said(session.clone()).await;
    assert_eq!(
        whole,
        [
            "user: first question",
            "assistant: Saved result",
            "user: second question",
            "assistant: Saved result",
            "user: third question",
            "assistant: Saved result",
        ]
    );
    let turn = |from_end: usize, text: &str| TurnRef {
        from_end,
        text: text.into(),
    };

    // A reference that does not match the saved conversation is refused.
    for wrong in [
        turn(0, "second question"),
        turn(1, "second"),
        turn(3, "first question"),
        turn(5_000, "first question"),
    ] {
        for result in [
            server
                .rewind_conversation(PROJECT, &session, &wrong)
                .await
                .map(|_| ()),
            server
                .branch_conversation(PROJECT, &session, Some(&wrong))
                .await
                .map(|_| ()),
        ] {
            assert_eq!(result.unwrap_err().to_string(), desktop::TURN_NOT_FOUND);
        }
    }
    assert!(server
        .rewind_conversation("other-project", &session, &turn(0, "third question"))
        .await
        .is_err());
    assert_eq!(said(session.clone()).await, whole);
    assert_eq!(
        server
            .conversations(PROJECT, "", false)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );

    // A branch copies the conversation through the named message's reply
    // and takes the source's group and choices with it.
    let group = server.create_folder(PROJECT, "Screens").await.unwrap();
    server
        .group_conversation(PROJECT, &session, Some(&group.id))
        .await
        .unwrap();
    server
        .set_conversation_options(
            PROJECT,
            &session,
            desktop::ConversationOptions {
                full_access: true,
                plan_first: true,
                reasoning_effort: "high".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let branch = server
        .branch_conversation(PROJECT, &session, Some(&turn(1, "second question")))
        .await
        .unwrap();
    assert_eq!(said(branch.clone()).await, whole[..4]);
    let full = server
        .branch_conversation(PROJECT, &session, None)
        .await
        .unwrap();
    assert_eq!(said(full.clone()).await, whole);
    // Naming the last message means the whole conversation too.
    let last = server
        .branch_conversation(PROJECT, &session, Some(&turn(0, "third question")))
        .await
        .unwrap();
    assert_eq!(said(last.clone()).await, whole);

    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.rows.len(), 4);
    for id in [&branch, &full, &last] {
        assert_eq!(list.branched.get(id), Some(&session));
        assert_eq!(list.grouped.get(id), Some(&group.id));
        let row = list.rows.iter().find(|row| &row.id == id).unwrap();
        assert!(row.title.ends_with(" (branch)"), "{}", row.title);
        assert_eq!(row.status, "complete");
    }
    assert_eq!(list.branched.get(&session), None);
    let options = server
        .conversation_status(PROJECT, &branch)
        .await
        .unwrap()
        .options;
    assert!(options.full_access && options.plan_first);
    assert_eq!(options.reasoning_effort, "high");

    // A conversation with branches is not deleted or moved from under them.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    for result in [
        server.delete_conversation(PROJECT, &session).await,
        server
            .transfer_conversation(PROJECT, &session, &lab, false)
            .await
            .map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("branches"));
    }
    server.delete_conversation(PROJECT, &last).await.unwrap();
    server.delete_conversation(PROJECT, &full).await.unwrap();

    // Rewinding hands back the message and removes it and what followed.
    // The branch made earlier keeps its copy.
    let text = server
        .rewind_conversation(PROJECT, &session, &turn(1, "second question"))
        .await
        .unwrap();
    assert_eq!(text, "second question");
    assert_eq!(said(session.clone()).await, whole[..2]);
    assert_eq!(said(branch.clone()).await, whole[..4]);
    // The same reference no longer names anything.
    assert!(server
        .rewind_conversation(PROJECT, &session, &turn(1, "second question"))
        .await
        .is_err());

    // The conversation carries on from where it was cut.
    send(&server, &session, "a better question").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        said(session.clone()).await,
        [
            "user: first question",
            "assistant: Saved result",
            "user: a better question",
            "assistant: Saved result",
        ]
    );

    // Back to the very first message leaves an empty conversation that
    // still works.
    server
        .rewind_conversation(PROJECT, &session, &turn(1, "first question"))
        .await
        .unwrap();
    assert!(said(session.clone()).await.is_empty());
    assert!(server
        .branch_conversation(PROJECT, &session, None)
        .await
        .is_err());
    send(&server, &session, "start over").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        said(session.clone()).await,
        ["user: start over", "assistant: Saved result"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_model_profiles_are_validated_saved_and_chosen_per_conversation() {
    use desktop::{ConversationOptions, ModelFields, ModelInput, ModelSource};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let used = Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory = |used: Arc<std::sync::Mutex<Vec<(String, String)>>>| -> ModelFactory {
        Arc::new(move |provider| {
            used.lock()
                .unwrap()
                .push((provider.model.clone(), provider.api_key.clone()));
            Box::new(FakeModel {
                calls: Arc::default(),
                write: false,
            })
        })
    };
    let last_used = |used: &Arc<std::sync::Mutex<Vec<(String, String)>>>| {
        used.lock().unwrap().last().cloned().unwrap()
    };
    let fields = |model: &str| ModelFields {
        provider: "openai".into(),
        api_url: "http://127.0.0.1:9/v1".into(),
        model: model.into(),
        max_context: "32000".into(),
        max_tokens: "4000".into(),
        reasoning_effort: String::new(),
        proxy: String::new(),
    };
    let input = |id: Option<&str>, name: &str, fields: ModelFields, key: Option<&str>| ModelInput {
        id: id.map(Into::into),
        name: name.into(),
        fields,
        api_key: key.map(Into::into),
    };
    let server = Server::open_with(cfg.clone(), factory(used.clone()), secrets.clone())
        .await
        .unwrap();
    let initial = server.model_settings().await.unwrap();
    assert_eq!(initial.source, ModelSource::None);
    assert!(initial.profiles.is_empty());

    let session = create(&server).await;
    let refused = send(&server, &session, "hello").await.unwrap_err();
    assert!(refused.contains("Configure a model"), "{refused}");

    let empty = server
        .save_model_profile(input(None, "", ModelFields::default(), None))
        .await
        .unwrap_err();
    assert_eq!(empty.to_string(), "Enter a model ID");
    // A rejected form changes nothing: no key, an unknown model without
    // limits, a name that is too long, a profile that does not exist.
    for bad in [
        input(None, "", fields("lab-model"), None),
        input(
            None,
            "",
            ModelFields {
                max_context: String::new(),
                ..fields("lab-model")
            },
            Some("first-key"),
        ),
        input(
            None,
            &"n".repeat(61),
            fields("lab-model"),
            Some("first-key"),
        ),
        input(Some("ghost"), "", fields("lab-model"), Some("first-key")),
    ] {
        assert!(server.save_model_profile(bad).await.is_err());
    }
    // A connection test uses the form as typed and saves nothing.
    assert_eq!(
        server
            .test_model(input(None, "", fields("lab-model"), Some("probe-key")))
            .await
            .unwrap(),
        "Saved result"
    );
    assert_eq!(last_used(&used), ("lab-model".into(), "probe-key".into()));
    assert!(server
        .test_model(input(None, "", fields("lab-model"), None))
        .await
        .is_err());
    assert!(server.model_settings().await.unwrap().profiles.is_empty());

    // The first profile becomes the default and is named after its model.
    let saved = server
        .save_model_profile(input(None, " ", fields("lab-model"), Some(" first-key ")))
        .await
        .unwrap();
    assert_eq!(saved.source, ModelSource::Saved);
    let first = saved.profiles[0].clone();
    assert_eq!(first.name, "lab-model");
    assert!(first.is_default && first.has_api_key && first.problem.is_none());
    send(&server, &session, "hello").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(done.model_id, "lab-model");
    assert_eq!(last_used(&used), ("lab-model".into(), "first-key".into()));

    // Editing a profile keeps its stored key when none is typed.
    server
        .save_model_profile(input(
            Some(&first.id),
            "Lab",
            fields("lab-model-2"),
            Some("  "),
        ))
        .await
        .unwrap();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| {
        !s.running && s.model_id == "lab-model-2"
    })
    .await;
    assert_eq!(last_used(&used), ("lab-model-2".into(), "first-key".into()));

    // A second profile has its own key and does not take over.
    let both = server
        .save_model_profile(input(
            None,
            "Deep",
            fields("deep-model"),
            Some("second-key"),
        ))
        .await
        .unwrap();
    assert_eq!(both.profiles.len(), 2);
    let deep = both
        .profiles
        .iter()
        .find(|profile| profile.name == "Deep")
        .unwrap()
        .clone();
    assert!(!deep.is_default);
    let other_session = create(&server).await;

    // One conversation chooses it; the other keeps following the default.
    let choose = |model: &str| ConversationOptions {
        model: model.into(),
        ..Default::default()
    };
    assert!(server
        .set_conversation_options(PROJECT, &session, choose("ghost"))
        .await
        .is_err());
    server
        .set_conversation_options(PROJECT, &session, choose(&deep.id))
        .await
        .unwrap();
    assert_eq!(snapshot(&server, &session).await.model_id, "deep-model");
    assert_eq!(
        snapshot(&server, &other_session).await.model_id,
        "lab-model-2"
    );
    send(&server, &session, "third").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));

    // An expert with a model of its own runs on it, in a conversation that
    // follows the default; one that names a removed model is refused.
    let thinker = desktop::ExpertInput {
        id: String::new(),
        name: "Deep thinker".into(),
        description: String::new(),
        instructions: "Think it through.".into(),
        model: deep.id.clone(),
        skills: None,
        connectors: None,
    };
    assert!(server
        .save_expert(desktop::ExpertInput {
            model: "ghost".into(),
            ..thinker.clone()
        })
        .await
        .is_err());
    let thinker = server
        .save_expert(thinker)
        .await
        .unwrap()
        .into_iter()
        .find(|expert| expert.name == "Deep thinker")
        .unwrap();
    assert_eq!(thinker.model, deep.id);
    let as_expert = create(&server).await;
    server
        .set_conversation_options(
            PROJECT,
            &as_expert,
            ConversationOptions {
                expert: thinker.id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(snapshot(&server, &as_expert).await.model_id, "deep-model");
    send(&server, &as_expert, "think").await.unwrap();
    wait_for(&server, &as_expert, |s| !s.running).await;
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));

    // Sub-agents work on the conversation's model until another is chosen
    // for them: in Settings for all, or by a conversation for itself,
    // which outranks Settings.
    let own = server.0.model.read().unwrap().pick(&deep.id).unwrap();
    let worker = |session: String| {
        let (server, own) = (&server, own.clone());
        async move {
            server
                .subagent_model_for(&session, &own)
                .await
                .provider
                .model
        }
    };
    assert_eq!(server.subagent_model().await.unwrap(), "");
    assert_eq!(worker(session.clone()).await, "deep-model");
    assert!(server.set_subagent_model("ghost").await.is_err());
    assert_eq!(
        server.set_subagent_model(&first.id).await.unwrap(),
        first.id
    );
    assert_eq!(worker(session.clone()).await, "lab-model-2");
    server
        .0
        .store
        .set_frame_subagent_model(&session, PROJECT, Some(&deep.id))
        .await
        .unwrap();
    assert_eq!(worker(session.clone()).await, "deep-model");
    assert_eq!(worker(other_session.clone()).await, "lab-model-2");
    // A choice that names a profile which is gone falls through.
    server
        .0
        .store
        .set_frame_subagent_model(&session, PROJECT, Some("gone"))
        .await
        .unwrap();
    assert_eq!(worker(session.clone()).await, "lab-model-2");
    assert_eq!(server.set_subagent_model("").await.unwrap(), "");
    assert_eq!(worker(session.clone()).await, "deep-model");
    server
        .0
        .store
        .set_frame_subagent_model(&session, PROJECT, None)
        .await
        .unwrap();
    // The conversation's own choice, made through its options: a saved
    // model, its own model over Settings, or Settings again.
    let subagents = |choice: Option<&str>| ConversationOptions {
        subagent_model: choice.map(str::to_owned),
        ..Default::default()
    };
    assert!(server
        .set_conversation_options(PROJECT, &other_session, subagents(Some("ghost")))
        .await
        .is_err());
    let chosen = server
        .set_conversation_options(PROJECT, &other_session, subagents(Some(&first.id)))
        .await
        .unwrap();
    assert_eq!(chosen.subagent_model.as_deref(), Some(first.id.as_str()));
    let lab_own = server.0.model.read().unwrap().pick(&first.id).unwrap();
    let other_worker = || async {
        server
            .subagent_model_for(&other_session, &own)
            .await
            .provider
            .model
    };
    assert_eq!(other_worker().await, "lab-model-2");
    server.set_subagent_model(&first.id).await.unwrap();
    server
        .set_conversation_options(PROJECT, &other_session, subagents(Some("")))
        .await
        .unwrap();
    // Its own model is the one it runs on, whatever Settings says.
    assert_eq!(other_worker().await, "deep-model");
    assert_eq!(
        server
            .subagent_model_for(&other_session, &lab_own)
            .await
            .provider
            .model,
        "lab-model-2"
    );
    let back = server
        .set_conversation_options(PROJECT, &other_session, subagents(None))
        .await
        .unwrap();
    assert_eq!(back.subagent_model, None);
    assert_eq!(other_worker().await, "lab-model-2");
    server.set_subagent_model("").await.unwrap();

    // A review is made with the conversation's model until another is
    // chosen for reviews. (This model writes no report, so the review
    // fails; which model was asked is what is checked.)
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert_eq!(server.review_model().await.unwrap(), "");
    assert!(server.set_review_model("ghost").await.is_err());
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));
    assert_eq!(server.set_review_model(&first.id).await.unwrap(), first.id);
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
    assert_eq!(last_used(&used), ("lab-model-2".into(), "first-key".into()));
    assert_eq!(server.set_review_model("").await.unwrap(), "");
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));

    let switched = server.set_default_model(&deep.id).await.unwrap();
    assert!(switched
        .profiles
        .iter()
        .all(|profile| profile.is_default == (profile.id == deep.id)));
    assert_eq!(
        snapshot(&server, &other_session).await.model_id,
        "deep-model"
    );
    assert!(server.set_default_model("ghost").await.is_err());

    // Deleting the profile sends both conversations to what is left.
    let left = server.delete_model_profile(&deep.id).await.unwrap();
    assert_eq!(left.profiles.len(), 1);
    assert!(left.profiles[0].is_default);
    for id in [&session, &other_session] {
        assert_eq!(snapshot(&server, id).await.model_id, "lab-model-2");
    }
    assert!(server.delete_model_profile(&deep.id).await.is_err());

    // Keys are in the secret store only: not in settings, not in the database.
    let raw = server
        .0
        .store
        .get_setting("desktop.models")
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.contains("first-key") && !raw.contains("second-key"));
    // Another data directory has its own keys, even with the same secret store.
    let other_root = tempfile::tempdir().unwrap();
    let mut other_cfg = config(other_root.path());
    other_cfg.model = None;
    let other = Server::open_with(other_cfg, factory(used.clone()), secrets.clone())
        .await
        .unwrap();
    assert!(other.model_settings().await.unwrap().profiles.is_empty());
    other.shutdown().await;
    server.shutdown().await;
    drop(server);

    // Profiles survive a restart; a lost key is reported, not fatal.
    let reopened = Server::open_with(cfg.clone(), factory(used.clone()), secrets)
        .await
        .unwrap();
    let restored = reopened.model_settings().await.unwrap();
    assert_eq!(restored.source, ModelSource::Saved);
    assert_eq!(restored.profiles[0].name, "Lab");
    assert_eq!(restored.profiles[0].fields.model, "lab-model-2");
    reopened.shutdown().await;
    drop(reopened);
    let without_key = Server::open_with(
        cfg.clone(),
        factory(used.clone()),
        Arc::new(desktop::MemorySecrets::default()),
    )
    .await
    .unwrap();
    let broken = without_key.model_settings().await.unwrap();
    assert_eq!(broken.source, ModelSource::None);
    let profile = &broken.profiles[0];
    assert!(!profile.has_api_key);
    assert!(profile
        .problem
        .as_ref()
        .unwrap()
        .contains("API key is missing"));
    assert_eq!(profile.fields.model, "lab-model-2");
    without_key.shutdown().await;
    drop(without_key);

    // WISP_* variables win and are not editable from the desktop.
    let from_env = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let env = from_env.model_settings().await.unwrap();
    assert_eq!(env.source, ModelSource::Environment);
    assert_eq!(env.profiles.len(), 1);
    assert_eq!(env.profiles[0].fields.model, "offline");
    assert!(env.profiles[0].is_default);
    let id = env.profiles[0].id.clone();
    for refused in [
        from_env
            .save_model_profile(input(None, "", fields("lab-model"), Some("k")))
            .await,
        from_env.set_default_model(&id).await,
        from_env.delete_model_profile(&id).await,
    ] {
        assert!(refused.unwrap_err().to_string().contains("environment"));
    }
    from_env.shutdown().await;
}

#[tokio::test]
async fn the_single_model_saved_by_an_earlier_build_is_kept_as_a_profile() {
    use desktop::{ModelFields, ModelInput, SecretStore};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let fields = ModelFields {
        provider: "openai".into(),
        api_url: "http://127.0.0.1:9/v1".into(),
        model: "old-model".into(),
        max_context: "32000".into(),
        max_tokens: "4000".into(),
        reasoning_effort: String::new(),
        proxy: String::new(),
    };
    // What the earlier build left behind: one settings row and one key.
    {
        let server = Server::open_with(cfg.clone(), models(Arc::default(), false), secrets.clone())
            .await
            .unwrap();
        let store = &server.0.store;
        store
            .set_setting("desktop.model", &serde_json::to_string(&fields).unwrap())
            .await
            .unwrap();
        store
            .set_setting("desktop.model.key_id", "kid")
            .await
            .unwrap();
        secrets.set("desktop.model.api_key.kid", "old-key").unwrap();
        server.shutdown().await;
    }
    let server = Server::open_with(cfg, models(Arc::default(), false), secrets.clone())
        .await
        .unwrap();
    let settings = server.model_settings().await.unwrap();
    assert_eq!(settings.profiles.len(), 1);
    let old = settings.profiles[0].clone();
    assert_eq!(old.name, "old-model");
    assert!(old.is_default && old.has_api_key && old.problem.is_none());
    let session = create(&server).await;
    send(&server, &session, "hello").await.unwrap();
    assert_eq!(
        wait_for(&server, &session, |s| !s.running).await.model_id,
        "old-model"
    );
    // Adding another profile keeps it, still usable with its old key.
    let both = server
        .save_model_profile(ModelInput {
            id: None,
            name: "New".into(),
            fields: ModelFields {
                model: "new-model".into(),
                ..fields
            },
            api_key: Some("new-key".into()),
        })
        .await
        .unwrap();
    assert_eq!(both.profiles.len(), 2);
    let kept = both.profiles.iter().find(|p| p.id == old.id).unwrap();
    assert!(kept.is_default && kept.problem.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_home_lists_recent_work_and_manages_projects() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("data.csv"), "a,b\n1,2\n").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let running = || async {
        let value = call(
            &server,
            "native_conversation_snapshot",
            &lab,
            json!({"session_id": session}),
        )
        .await
        .unwrap();
        serde_json::from_value::<dto::Snapshot>(value).unwrap()
    };
    call(
        &server,
        "native_conversation_send",
        &lab,
        json!({"session_id": session, "request_id": "home-1", "message": "write the answer"}),
    )
    .await
    .unwrap();
    // The write tool waits for approval, which keeps the turn running.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while running().await.approvals.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // Recent work spans projects and says where each conversation lives.
    let recent = server.recent_conversations(10).await.unwrap();
    let row = recent.iter().find(|row| row.id == session).unwrap();
    assert_eq!(row.project_id, lab);
    assert_eq!(row.project_name, "lab");
    assert_eq!(row.status, "needs_you");
    assert!(server.recent_conversations(0).await.unwrap().is_empty());

    let host = &server;
    let starred = |id: String| async move {
        let value = call(host, "list_projects", "", json!({})).await.unwrap();
        serde_json::from_value::<Vec<wisp_dto::ProjectSummary>>(value)
            .unwrap()
            .into_iter()
            .find(|project| project.id == id)
            .map(|project| project.starred)
    };
    server.star_project(&lab, true).await.unwrap();
    assert_eq!(starred(lab.clone()).await, Some(true));
    server.star_project(&lab, false).await.unwrap();
    assert_eq!(starred(lab.clone()).await, Some(false));
    assert!(server.star_project("missing", true).await.is_err());

    let refused = server.remove_project(&lab).await.unwrap_err().to_string();
    assert!(refused.contains("Stop the running"), "{refused}");
    assert!(server.remove_project(PROJECT).await.is_err());
    assert!(server.remove_project("missing").await.is_err());
    call(
        &server,
        "native_conversation_stop",
        &lab,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while running().await.running {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    server.remove_project(&lab).await.unwrap();
    assert_eq!(starred(lab.clone()).await, None);
    assert!(server
        .recent_conversations(10)
        .await
        .unwrap()
        .iter()
        .all(|row| row.project_id != lab));
    // Removing a project never touches its folder.
    assert_eq!(
        std::fs::read_to_string(folder.join("data.csv")).unwrap(),
        "a,b\n1,2\n"
    );
    // The same folder can be opened again as a fresh project.
    let again = server.register_local_project(&folder).await.unwrap();
    assert_ne!(again, lab);

    // None of this is a remote command.
    for command in [
        "set_project_starred",
        "delete_project",
        "list_recent_sessions",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, &again, json!({})).await.is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_search_and_project_files_need_no_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("data")).unwrap();
    std::fs::create_dir_all(folder.join(".wisp")).unwrap();
    std::fs::write(folder.join("data/counts.csv"), "gene,count\nTP53,7\n").unwrap();
    std::fs::write(folder.join(".wisp/counts.sqlite"), "internal").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    // The project has no conversation yet; its files are still reachable.
    let root = server.project_directory(&lab, "").await.unwrap();
    assert_eq!(root.project_id, lab);
    assert_eq!(root.session_id, None);
    let names: Vec<_> = root.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["data"]);
    let file = server.project_file(&lab, "data/counts.csv").await.unwrap();
    assert_eq!(file.requested_path, "data/counts.csv");
    assert_eq!(file.content.text.as_deref(), Some("gene,count\nTP53,7\n"));
    for path in [
        "../lab/data/counts.csv",
        ".wisp/counts.sqlite",
        "/etc/hosts",
    ] {
        assert!(server.project_file(&lab, path).await.is_err(), "{path}");
    }
    assert!(server.project_directory("missing", "").await.is_err());

    // Name search covers every project, or just the one asked for.
    let everywhere = server.find_files(None, "COUNTS", 5).await.unwrap();
    assert_eq!(
        everywhere,
        [desktop::FileMatch {
            project_id: lab.clone(),
            project_name: "lab".into(),
            path: "data/counts.csv".into(),
        }]
    );
    assert!(server
        .find_files(Some(PROJECT), "counts", 5)
        .await
        .unwrap()
        .is_empty());
    assert!(server.find_files(None, " ", 5).await.unwrap().is_empty());

    // Conversation search reads messages in any project, archive included.
    let session = create(&server).await;
    send(&server, &session, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let found = server.find_conversations(" alignment ", 5).await.unwrap();
    assert_eq!(found.len(), 1);
    // The instructions every conversation starts with are not its content:
    // a word that occurs only there must not match every conversation.
    let instructions = server
        .0
        .store
        .load_system_messages(std::slice::from_ref(&session))
        .await
        .unwrap()
        .remove(&session)
        .unwrap();
    let word = instructions
        .split(|c: char| !c.is_ascii_alphabetic())
        .max_by_key(|word| word.len())
        .unwrap();
    assert!(word.len() >= 6, "{word}");
    assert!(server.find_conversations(word, 5).await.unwrap().is_empty());
    assert_eq!(
        (found[0].id.as_str(), found[0].shelved),
        (session.as_str(), false)
    );
    server
        .archive_conversation(PROJECT, &session, true)
        .await
        .unwrap();
    let found = server.find_conversations("alignment", 5).await.unwrap();
    assert_eq!(
        (found[0].id.as_str(), found[0].shelved),
        (session.as_str(), true)
    );
    // With nothing typed it is the recent list, which leaves the archive out.
    assert!(server.find_conversations("", 5).await.unwrap().is_empty());
    assert!(server
        .find_conversations("alignment", 0)
        .await
        .unwrap()
        .is_empty());

    // None of this is a remote command.
    for command in [
        "native_project_file_directory",
        "native_project_file_read",
        "native_project_file_search",
        "search_sessions",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, &lab, json!({"path": ""}))
            .await
            .is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn looking_for_a_newer_version_at_start_can_be_turned_off() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(server.update_check_at_start().await.unwrap());
    server.set_update_check_at_start(false).await.unwrap();
    assert!(!server.update_check_at_start().await.unwrap());
    // Under the WebView desktop's name, so either desktop honours it.
    assert_eq!(
        server
            .0
            .store
            .get_setting("update_check_enabled")
            .await
            .unwrap()
            .as_deref(),
        Some("false")
    );
    server.set_update_check_at_start(true).await.unwrap();
    assert!(server.update_check_at_start().await.unwrap());
    server.shutdown().await;
}

#[tokio::test]
async fn fast_mode_is_asked_for_only_where_the_model_offers_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    // OpenAI's own API; the fake model answers, nothing is sent.
    cfg.model.as_mut().unwrap().provider.base_url = "https://api.openai.com/v1".into();
    let tiers: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let seen = tiers.clone();
    let factory: ModelFactory = Arc::new(move |provider| {
        seen.lock().unwrap().push(provider.service_tier.clone());
        Box::new(FakeModel {
            calls: Arc::default(),
            write: false,
        })
    });
    let server = Server::open_with_models(cfg, factory).await.unwrap();
    let session = create(&server).await;
    let status = server.conversation_status(PROJECT, &session).await.unwrap();
    assert!(status.fast_available);
    assert!(!status.options.fast);

    let fast = desktop::ConversationOptions {
        fast: true,
        ..status.options.clone()
    };
    let saved = server
        .set_conversation_options(PROJECT, &session, fast)
        .await
        .unwrap();
    assert!(saved.fast);
    // Saved the way the WebView desktop saves it.
    assert_eq!(
        server
            .0
            .store
            .frame_service_tier(&session)
            .await
            .unwrap()
            .as_deref(),
        Some(desktop::FAST_TIER)
    );
    send(&server, &session, "hello").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(tiers
        .lock()
        .unwrap()
        .iter()
        .any(|tier| tier.as_deref() == Some(desktop::FAST_TIER)));

    // Off again: the next turn asks for nothing.
    let saved = server
        .set_conversation_options(
            PROJECT,
            &session,
            desktop::ConversationOptions {
                fast: false,
                ..saved
            },
        )
        .await
        .unwrap();
    assert!(!saved.fast);
    tiers.lock().unwrap().clear();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let asked = tiers.lock().unwrap().clone();
    assert!(!asked.is_empty());
    assert!(asked.iter().all(Option::is_none), "{asked:?}");
    server.shutdown().await;
}

#[test]
fn only_openais_own_services_offer_fast_mode() {
    use wisp_llm::{ProviderConfig, ProviderKind};
    let local = ProviderConfig::openai("http://127.0.0.1:9", "k", "m");
    assert!(!desktop::fast_available(&local));
    let openai = ProviderConfig::openai("https://api.openai.com/v1", "k", "m");
    assert!(desktop::fast_available(&openai));
    let mut responses = local.clone();
    responses.kind = ProviderKind::OpenAiResponses;
    assert!(desktop::fast_available(&responses));
    let mut anthropic = openai.clone();
    anthropic.kind = ProviderKind::Anthropic;
    assert!(!desktop::fast_available(&anthropic));
}

#[tokio::test]
async fn desktop_conversation_options_shape_the_next_turn() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let efforts: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let seen = efforts.clone();
    let factory: ModelFactory = Arc::new(move |provider| {
        seen.lock().unwrap().push(provider.reasoning_effort.clone());
        Box::new(FakeModel {
            calls: Arc::default(),
            write: true,
        })
    });
    let server = Server::open_with_models(cfg.clone(), factory)
        .await
        .unwrap();
    let options = |full_access, effort: &str| desktop::ConversationOptions {
        full_access,
        reasoning_effort: effort.into(),
        ..Default::default()
    };
    let asking = create(&server).await;
    assert_eq!(
        server.conversation_status(PROJECT, &asking).await.unwrap(),
        desktop::ConversationStatus::default()
    );

    // Effort is validated, stored, and reaches the model on the next turn.
    assert!(server
        .set_conversation_options(PROJECT, &asking, options(false, "extreme"))
        .await
        .is_err());
    assert!(server
        .set_conversation_options("other", &asking, options(false, "high"))
        .await
        .is_err());
    assert_eq!(
        server
            .set_conversation_options(PROJECT, &asking, options(false, " high "))
            .await
            .unwrap(),
        options(false, "high")
    );
    send(&server, &asking, "write the answer").await.unwrap();
    let pending = wait_for(&server, &asking, |s| !s.approvals.is_empty()).await;
    assert_eq!(
        efforts.lock().unwrap().last().unwrap().as_deref(),
        Some("high")
    );
    let approval = pending.approvals[0].approval_id.clone();
    // "Deny for the conversation" does not exist; the request stays open.
    assert!(server
        .answer_approval(
            PROJECT,
            &asking,
            &approval,
            false,
            desktop::Remember::Conversation
        )
        .await
        .is_err());
    assert!(server
        .answer_approval(PROJECT, &asking, "missing", true, desktop::Remember::No)
        .await
        .is_err());
    server
        .answer_approval(
            PROJECT,
            &asking,
            &approval,
            true,
            desktop::Remember::Conversation,
        )
        .await
        .unwrap();
    wait_for(&server, &asking, |s| !s.running).await;
    assert!(cfg.workspace.join("answer.txt").exists());
    let status = server.conversation_status(PROJECT, &asking).await.unwrap();
    assert_eq!(status.allowed_tools, ["write"]);
    assert_eq!(status.options, options(false, "high"));
    assert!(status.usage.is_some());
    server.revoke_allowed_tools(PROJECT, &asking).await.unwrap();
    assert!(server
        .conversation_status(PROJECT, &asking)
        .await
        .unwrap()
        .allowed_tools
        .is_empty());

    // Full access: the same write runs without a question.
    std::fs::remove_file(cfg.workspace.join("answer.txt")).unwrap();
    let trusted = create(&server).await;
    server
        .set_conversation_options(PROJECT, &trusted, options(true, ""))
        .await
        .unwrap();
    send(&server, &trusted, "write the answer").await.unwrap();
    let done = wait_for(&server, &trusted, |s| !s.running).await;
    assert!(done.approvals.is_empty() && done.error.is_none());
    assert!(cfg.workspace.join("answer.txt").exists());
    assert_eq!(efforts.lock().unwrap().last().unwrap(), &None);
    // It belongs to that conversation only, and goes away with it.
    assert!(
        !server
            .conversation_status(PROJECT, &asking)
            .await
            .unwrap()
            .options
            .full_access
    );
    server.delete_conversation(PROJECT, &trusted).await.unwrap();
    assert_eq!(
        server
            .0
            .store
            .get_setting(&format!("desktop.conversation.full_access.{trusted}"))
            .await
            .unwrap(),
        None
    );

    // None of this is a remote command, and a remote approval is still once.
    for command in [
        "set_session_full_permission",
        "set_session_reasoning_effort",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(
            call(&server, command, PROJECT, json!({"session_id": asking}))
                .await
                .is_err()
        );
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_skill_that_is_turned_off_is_not_offered_to_the_model_or_the_composer() {
    let temp = tempfile::tempdir().unwrap();
    // The host is told about one directory of skills; the project has its own.
    let shared = temp.path().join("shared-skills");
    let skill = |dir: &Path, name: &str, about: &str| {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(
            dir.join(name).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {about}\ntags: [lab]\n---\nDo it.\n"),
        )
        .unwrap();
    };
    skill(&shared, "zebra-count", "Count stripes the lab way");
    let mut settings = config(temp.path());
    settings.skills = vec![shared];
    let server = Server::open_with_models(settings, models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    skill(
        &folder.join("skills"),
        "Alpha-fold-check",
        "Check a predicted fold",
    );
    let lab = server.register_local_project(&folder).await.unwrap();

    // Listed by name whatever the case, each with where it came from.
    let listed = |project: Option<String>| {
        let server = &server;
        async move {
            server
                .skills(project.as_deref())
                .await
                .unwrap()
                .into_iter()
                .map(|row| format!("{} {} {}", row.name, row.source, row.enabled))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        listed(Some(lab.clone())).await,
        ["Alpha-fold-check project true", "zebra-count extra true"]
    );
    // Without a project, only what the host has for every project.
    assert_eq!(listed(None).await, ["zebra-count extra true"]);
    assert!(server.skills(Some("missing")).await.is_err());
    assert_eq!(
        server.skills(Some(&lab)).await.unwrap()[1].tags,
        ["lab".to_string()]
    );

    let offered = |query: &'static str| {
        let (server, lab) = (&server, &lab);
        async move {
            server
                .skill_options(lab, query, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|option| option.label)
                .collect::<Vec<_>>()
        }
    };
    // What a turn's agent is given: the index its skill tools are built on.
    let for_turns = || {
        let (server, folder, lab) = (&server, &folder, &lab);
        async move {
            let index =
                crate::runtime::skill_index(&server.0.config, &server.0.store, lab, folder).await;
            let mut names: Vec<_> = index.all().iter().map(|skill| skill.name.clone()).collect();
            names.sort();
            names
        }
    };
    assert_eq!(offered("").await, ["Alpha-fold-check", "zebra-count"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check", "zebra-count"]);

    // Off: gone from the model's view and from the composer, still listed.
    server
        .set_skill_enabled("zebra-count", false)
        .await
        .unwrap();
    assert!(server.set_skill_enabled(" ", false).await.is_err());
    assert_eq!(offered("").await, ["Alpha-fold-check"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check"]);
    assert_eq!(
        listed(Some(lab.clone())).await,
        ["Alpha-fold-check project true", "zebra-count extra false"]
    );
    // Turning off twice, or a skill that is not installed, is harmless.
    server
        .set_skill_enabled("zebra-count", false)
        .await
        .unwrap();
    server
        .set_skill_enabled("not-installed", false)
        .await
        .unwrap();

    server.set_skill_enabled("zebra-count", true).await.unwrap();
    assert_eq!(offered("zebra").await, ["zebra-count"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check", "zebra-count"]);

    // The researcher's tags take the place of a skill's own, trimmed and
    // once each; none puts its own back.
    let tags = |name: &'static str| {
        let (server, lab) = (&server, &lab);
        async move {
            server
                .skills(Some(lab))
                .await
                .unwrap()
                .into_iter()
                .find(|row| row.name == name)
                .unwrap()
                .tags
        }
    };
    server
        .set_skill_tags(
            "Alpha-fold-check",
            vec![
                " structure ".into(),
                "".into(),
                "protein".into(),
                "structure".into(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(tags("Alpha-fold-check").await, ["protein", "structure"]);
    server
        .set_skill_tags("Alpha-fold-check", vec![" ".into()])
        .await
        .unwrap();
    assert_eq!(tags("Alpha-fold-check").await, ["lab"]);
    assert!(server.set_skill_tags(" ", vec!["x".into()]).await.is_err());
    assert!(server
        .set_skill_tags("zebra-count", vec!["x".repeat(41)])
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_bundled_connector_that_is_turned_off_gives_the_model_no_tools() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();

    // One row per connector, by name, each with the tools it brings.
    let rows = server.connectors().await.unwrap();
    let domains: Vec<_> = rows.iter().map(|row| row.domain.as_str()).collect();
    let mut sorted = domains.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(domains, sorted);
    assert!(rows.len() > 10);
    let pubmed = rows.iter().find(|row| row.domain == "pubmed").unwrap();
    assert!(pubmed.enabled && !pubmed.tools.is_empty());
    assert!(!pubmed.description.is_empty() && !pubmed.links.is_empty());
    let total: usize = rows.iter().map(|row| row.tools.len()).sum();
    assert_eq!(total, wisp_bio::catalog().len());

    // What a turn's agent is given.
    let given = || {
        let server = &server;
        async move {
            crate::runtime::connector_tools(&server.0.store, &[])
                .await
                .iter()
                .map(|tool| tool.name().to_owned())
                .collect::<std::collections::HashSet<_>>()
        }
    };
    let all = given().await;
    assert_eq!(all.len(), total);
    assert!(pubmed.tools.iter().all(|tool| all.contains(tool)));

    server.set_connector_enabled("pubmed", false).await.unwrap();
    assert!(server
        .set_connector_enabled("not-a-connector", false)
        .await
        .is_err());
    let without = given().await;
    assert!(pubmed.tools.iter().all(|tool| !without.contains(tool)));
    assert_eq!(without.len(), total - pubmed.tools.len());
    let rows = server.connectors().await.unwrap();
    assert!(rows
        .iter()
        .all(|row| row.enabled == (row.domain != "pubmed")));

    // A turn still runs with the rest in place.
    let session = create(&server).await;
    send(&server, &session, "hello").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert!(done.error.is_none());

    server.set_connector_enabled("pubmed", true).await.unwrap();
    assert_eq!(given().await, all);
    server.shutdown().await;
}

#[tokio::test]
async fn usage_is_counted_from_each_model_call_and_storage_is_measured() {
    // A model that says what each call cost.
    struct Metered;
    #[async_trait]
    impl Provider for Metered {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "metered-1"
        }
        async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            Ok(Completion {
                content: "counted".into(),
                finish_reason: Some("stop".into()),
                usage: wisp_llm::Usage {
                    input_tokens: 120,
                    output_tokens: 30,
                    reasoning_tokens: 5,
                    cached_input_tokens: 100,
                },
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("counted");
            self.complete(messages, tools).await
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = config(temp.path());
    let data = settings.data.clone();
    let factory: ModelFactory = Arc::new(|_| Box::new(Metered));
    let server = Server::open_with_models(settings, factory).await.unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("uploads")).unwrap();
    std::fs::write(folder.join("uploads/reads.fastq"), vec![b'A'; 4_096]).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let gone = temp.path().join("gone");
    std::fs::create_dir(&gone).unwrap();
    let lost = server.register_local_project(&gone).await.unwrap();
    std::fs::remove_dir(&gone).unwrap();

    // Nothing was used yet. The days run up to today.
    let empty = server.usage().await.unwrap();
    assert!(empty.projects.is_empty() && empty.models.is_empty());
    assert!(empty.days.len() > 300);
    assert!(empty.days.iter().all(|(_, tokens)| *tokens == 0));

    let turn = |project: String, session: String, message: &'static str| {
        let server = &server;
        async move {
            call(
                server,
                "native_conversation_send",
                &project,
                json!({"session_id": session, "request_id": message, "message": message}),
            )
            .await
            .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        &project,
                        json!({"session_id": session}),
                    )
                    .await
                    .unwrap();
                    let snapshot: dto::Snapshot = serde_json::from_value(value).unwrap();
                    if !snapshot.running {
                        return snapshot;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
    };
    let first = server.create_conversation(&lab).await.unwrap();
    let done = turn(lab.clone(), first.clone(), "one").await;
    // The saved usage rows do not show up as anything in the transcript.
    let roles: Vec<_> = done.items.iter().map(|item| item.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant"]);
    turn(lab.clone(), first.clone(), "two").await;
    let other = server.create_conversation(PROJECT).await.unwrap();
    turn(PROJECT.into(), other, "three").await;

    let used = server.usage().await.unwrap();
    let project = |id: &str| {
        used.projects
            .iter()
            .find(|row| row.project_id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        project(&lab),
        desktop::ProjectUsage {
            project_id: lab.clone(),
            name: "lab".into(),
            conversations: 1,
            input: 240,
            output: 60,
            reasoning: 10,
            cached: 200,
        }
    );
    assert_eq!(project(PROJECT).input, 120);
    assert_eq!(used.projects.len(), 2);
    // By model, and on today's row of the calendar: input plus output.
    assert_eq!(used.models, [("metered-1".to_string(), 450)]);
    assert_eq!(used.days.last().unwrap().1, 450);
    assert_eq!(used.days.iter().map(|(_, tokens)| tokens).sum::<u64>(), 450);

    // Storage: the database, no favorites yet, what was installed, and
    // what is kept in each project's folder.
    std::fs::create_dir_all(data.join("toolchain/python")).unwrap();
    std::fs::write(data.join("toolchain/python/bin"), vec![0u8; 2_048]).unwrap();
    std::fs::create_dir_all(data.join("plugins/motif/1.0.0")).unwrap();
    std::fs::write(data.join("plugins/motif/1.0.0/SKILL.md"), vec![0u8; 512]).unwrap();
    std::fs::write(folder.join("reads.fq"), vec![0u8; 1_000]).unwrap();
    let storage = server.storage().await.unwrap();
    assert_eq!(storage.tools_bytes, 2_048);
    assert_eq!(storage.plugins_bytes, 512);
    assert!(
        storage.data_bytes >= storage.database_bytes + storage.tools_bytes + storage.plugins_bytes
    );
    assert_eq!(
        storage.other_bytes(),
        storage.data_bytes
            - storage.database_bytes
            - storage.favorites_bytes
            - storage.tools_bytes
            - storage.plugins_bytes
    );
    // The same folder, whatever links the temp path goes through.
    let same = |shown: &str, wanted: &Path| {
        std::fs::canonicalize(shown).unwrap() == std::fs::canonicalize(wanted).unwrap()
    };
    assert!(same(&storage.data_dir, &data));
    assert!(storage.database_bytes > 0);
    assert_eq!(storage.favorites_bytes, 0);
    let kept = |id: &str| {
        storage
            .projects
            .iter()
            .find(|row| row.project_id == id)
            .unwrap()
            .clone()
    };
    let lab_kept = kept(&lab);
    assert_eq!(lab_kept.conversations, 1);
    assert_eq!(lab_kept.uploads_bytes, 4_096);
    // The whole folder: the researcher's own files as well.
    assert!(lab_kept.workspace_bytes >= 4_096 + 1_000 + lab_kept.wisp_bytes);
    assert!(!lab_kept.workspace_more);
    assert!(!lab_kept.missing);
    assert!(same(&lab_kept.workspace_dir, &folder));
    let lost_kept = kept(&lost);
    assert!(lost_kept.missing);
    assert_eq!((lost_kept.wisp_bytes, lost_kept.uploads_bytes), (0, 0));
    server
        .star_text(&lab, &first, "text", None, "kept")
        .await
        .unwrap();
    assert!(server.storage().await.unwrap().favorites_bytes > 0);
    server.shutdown().await;
}

#[tokio::test]
async fn service_keys_live_in_the_secret_store_and_are_never_read_back() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();

    // Listed, none saved, and nothing is created just by looking.
    let rows = server.credentials().await.unwrap();
    let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        [
            "ncbi_api_key",
            "ncbi_email",
            "openalex_api_key",
            "infinisynapse_api_key",
            "scimaster_api_key"
        ]
    );
    assert!(rows.iter().all(|row| !row.saved));
    assert!(server.service_keys().await.is_empty());
    assert_eq!(
        server
            .0
            .store
            .get_setting("desktop.model.key_id")
            .await
            .unwrap(),
        None
    );

    server
        .set_credential("ncbi_api_key", "  abc123  ")
        .await
        .unwrap();
    server
        .set_credential("ncbi_email", "lab@example.org")
        .await
        .unwrap();
    for (id, value) in [
        ("not-a-key", "x".to_string()),
        ("ncbi_api_key", "two words".to_string()),
        ("ncbi_api_key", "x".repeat(513)),
    ] {
        assert!(server.set_credential(id, &value).await.is_err(), "{id}");
    }
    let saved: Vec<_> = server
        .credentials()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.saved)
        .collect();
    assert_eq!(saved, [true, true, false, false, false]);
    // A turn is given them under the names the services use.
    assert_eq!(
        server.service_keys().await,
        [
            ("NCBI_API_KEY".to_string(), "abc123".to_string()),
            ("NCBI_EMAIL".to_string(), "lab@example.org".to_string()),
        ]
    );

    // The value is in the secret store and nowhere in the database.
    let key_id = server
        .0
        .store
        .get_setting("desktop.model.key_id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        desktop::SecretStore::get(
            secrets.as_ref(),
            &format!("desktop.service.{key_id}.ncbi_api_key")
        )
        .as_deref(),
        Some("abc123")
    );
    server.shutdown().await;
    drop(server);
    for entry in std::fs::read_dir(temp.path().join("data")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap_or_default();
        assert!(!bytes.windows(6).any(|window| window == b"abc123"));
    }

    // Forgetting one leaves the other.
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    server.set_credential("ncbi_api_key", "").await.unwrap();
    assert_eq!(
        server.service_keys().await,
        [("NCBI_EMAIL".to_string(), "lab@example.org".to_string())]
    );

    // Credentials of your own: a name and a variable, the value in the
    // secret store; refused when malformed or one of the service keys.
    assert!(server.custom_credentials().await.unwrap().is_empty());
    for (name, env, value) in [
        ("", "LAB_TOKEN", "t"),
        ("Lab", "9LAB", "t"),
        ("Lab", "LAB-TOKEN", "t"),
        ("Lab", "LAB_TOKEN", " "),
        ("Lab", "ncbi_api_key", "t"),
    ] {
        assert!(
            server
                .add_custom_credential(name, env, value)
                .await
                .is_err(),
            "{env}"
        );
    }
    let mine = server
        .add_custom_credential(" Lab token ", "LAB_TOKEN", " tok-1 ")
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (
            mine[0].name.as_str(),
            mine[0].env_var.as_str(),
            mine[0].saved
        ),
        ("Lab token", "LAB_TOKEN", true)
    );
    // The same variable again: a new value in its place, not a second row.
    let again = server
        .add_custom_credential("Lab", "lab_token", "tok-2")
        .await
        .unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].id, mine[0].id);
    assert_eq!(
        desktop::SecretStore::get(
            secrets.as_ref(),
            &format!("custom_credential:{}", mine[0].id)
        )
        .as_deref(),
        Some("tok-2")
    );
    assert!(server
        .service_keys()
        .await
        .contains(&("LAB_TOKEN".to_string(), "tok-2".to_string())));
    // Kept in the WebView's shape, without the value.
    let saved = server
        .0
        .store
        .get_setting("custom_credentials")
        .await
        .unwrap()
        .unwrap();
    assert!(saved.contains("\"env_var\":\"LAB_TOKEN\""), "{saved}");
    assert!(!saved.contains("tok-2"));
    // Removed, it is gone with its value.
    assert!(server.remove_custom_credential("missing").await.is_err());
    assert!(server
        .remove_custom_credential(&mine[0].id)
        .await
        .unwrap()
        .is_empty());
    assert!(!server
        .service_keys()
        .await
        .iter()
        .any(|(name, _)| name == "LAB_TOKEN"));
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_plugins_mcp_server_runs_in_its_folder_for_the_projects_that_switch_it_on() {
    use std::os::unix::fs::PermissionsExt;
    // A model that calls the plugin's tool once, then says what came back.
    struct Caller;
    #[async_trait]
    impl Provider for Caller {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let back = messages
                .iter()
                .rev()
                .find(|message| message.role == wisp_llm::Role::Tool)
                .map(|message| message.content.as_text());
            Ok(match back {
                Some(text) => Completion {
                    content: format!("It said: {text}"),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                },
                None => Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: wisp_llm::FunctionCall {
                            name: "lab_tools_lab__where".into(),
                            arguments: "{}".into(),
                        },
                    }],
                    finish_reason: Some("tool_calls".into()),
                    ..Default::default()
                },
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), Arc::new(|_| Box::new(Caller)))
        .await
        .unwrap();
    // A plugin whose one MCP server is a script inside it, answering MCP a
    // line at a time with the folder it runs in and the root it was given.
    let package = temp.path().join("lab-plugin");
    std::fs::create_dir_all(package.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(package.join("server")).unwrap();
    std::fs::write(
        package.join(".claude-plugin/plugin.json"),
        r#"{"name":"lab-tools","version":"1.0.0","description":"Lab"}"#,
    )
    .unwrap();
    std::fs::write(
        package.join(".mcp.json"),
        r#"{"lab":{"command":"${CLAUDE_PLUGIN_ROOT}/server/serve.sh","env":{"LAB_ROOT":"${CLAUDE_PLUGIN_ROOT}"}},
            "broken":{"command":"wisp-no-such-program-anywhere"}}"#,
    )
    .unwrap();
    let script = package.join("server/serve.sh");
    std::fs::write(
        &script,
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  [ -z "$id" ] && continue
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"lab","version":"1"}}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"where","description":"Say where","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"cwd=%s root=%s token=%s"}],"isError":false}}\n' "$id" "$(pwd)" "$LAB_ROOT" "${WISP_TEST_SECRET:-none}" ;;
    *) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let installed = server.install_plugin(&package).await.unwrap();
    // The one that cannot start says why; the other has nothing to say.
    assert_eq!(installed.problems.len(), 1, "{:?}", installed.problems);
    assert!(installed.problems[0].contains("broken"));

    // Off: no project's turn gets its tools.
    assert!(server.plugin_mcp_tools(PROJECT).await.is_empty());
    server
        .set_plugin_enabled(PROJECT, "lab-tools", "1.0.0", true)
        .await
        .unwrap();
    let names: Vec<String> = server
        .plugin_mcp_tools(PROJECT)
        .await
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();
    assert_eq!(names, ["lab_tools_lab__where"]);

    // In a turn its tool asks first, then runs in the plugin's folder with
    // the root it was given and nothing of this app's own secrets.
    let session = create(&server).await;
    send(&server, &session, "where").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty()).await;
    server
        .answer_approval(
            PROJECT,
            &session,
            &asked.approvals[0].approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    let said = done
        .items
        .iter()
        .map(|item| item.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let root = dunce::canonicalize(server.0.config.data.join("plugins/lab-tools/1.0.0")).unwrap();
    assert!(said.contains(&format!("cwd={}", root.display())), "{said}");
    assert!(said.contains(&format!("root={}", root.display())), "{said}");
    assert!(said.contains("token=none"), "{said}");
    server.shutdown().await;
}

/// GitHub as a test sees it: one repository at one commit.
struct FakeGithub {
    archive: Vec<u8>,
}
#[async_trait]
impl wisp_skills::distribution::GithubApi for FakeGithub {
    async fn json(&self, path: &str) -> Result<Option<Value>, String> {
        Ok(path
            .starts_with("/repos/author/research/commits/")
            .then(|| json!({"sha": "a".repeat(40)})))
    }
}
#[async_trait]
impl crate::skill_store::Fetch for FakeGithub {
    async fn archive(&self, _: &wisp_dto::SkillInstallSource) -> Result<Vec<u8>, String> {
        Ok(self.archive.clone())
    }
    async fn index(&self) -> Result<Vec<u8>, String> {
        Err("No network here.".into())
    }
}

#[tokio::test]
async fn a_skill_from_github_goes_into_your_own_skills_once() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    let personal = temp.path().join("home/.wisp/skills");
    cfg.personal_skills = Some(personal.clone());
    let server = Server::open_with_models(cfg, models(Arc::default(), false))
        .await
        .unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, text) in [
        (
            "research-main/skills/sample/SKILL.md",
            "---\nname: sample\ndescription: Count reads the lab way\n---\nDo it.\n",
        ),
        ("research-main/skills/sample/references/guide.md", "guide"),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let github = FakeGithub {
        archive: zip.finish().unwrap().into_inner(),
    };
    let found = server
        .preview_with(
            &github,
            PROJECT,
            "https://github.com/author/research",
            "main",
        )
        .await
        .unwrap();
    let sample = found
        .iter()
        .find(|candidate| candidate.name == "sample")
        .unwrap()
        .clone();
    assert!(sample.conflict.is_none(), "{:?}", sample.conflict);
    assert_eq!(sample.source.commit, "a".repeat(40));

    let installed = server
        .install_with(&github, PROJECT, sample.source.clone())
        .await
        .unwrap();
    assert_eq!(installed.name, "sample");
    assert!(personal.join("sample/SKILL.md").is_file());
    // Every project has it, as one of your own.
    let skills = server.skills(Some(PROJECT)).await.unwrap();
    let mine = skills.iter().find(|skill| skill.name == "sample").unwrap();
    assert_eq!(mine.source, "global");
    // Asked again, the name is taken and nothing is replaced.
    std::fs::write(
        personal.join("sample/SKILL.md"),
        "---\nname: sample\ndescription: mine\n---\n",
    )
    .unwrap();
    let again = server
        .install_with(&github, PROJECT, sample.source)
        .await
        .unwrap_err()
        .to_string();
    assert!(again.contains("taken"), "{again}");
    assert!(std::fs::read_to_string(personal.join("sample/SKILL.md"))
        .unwrap()
        .contains("mine"));

    // The community directory: as shipped, and said so when it cannot be
    // read anew.
    let shipped = crate::skill_store::community(&github, false).await.unwrap();
    assert!(!shipped.entries.is_empty());
    assert!(shipped.notice.is_none());
    let offline = crate::skill_store::community(&github, true).await.unwrap();
    assert_eq!(offline.entries.len(), shipped.entries.len());
    assert!(offline.notice.unwrap().contains("No network here."));
    server.shutdown().await;
}

#[tokio::test]
async fn the_local_tools_paths_are_saved_as_the_webview_keeps_them() {
    use std::collections::BTreeMap;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let paths = server.tool_paths().await.unwrap();
    let keys: Vec<_> = paths.iter().map(|path| path.key).collect();
    assert_eq!(keys[..2], ["python_executable", "rscript_executable"]);
    assert!(paths.iter().all(|path| path.saved.is_empty()));
    // A file is needed; nothing is run to check it.
    let python = temp.path().join("python3");
    std::fs::write(&python, "not run").unwrap();
    let one = |key: &str, value: &str| BTreeMap::from([(key.to_owned(), value.to_owned())]);
    assert!(server
        .set_tool_paths(one("python_executable", "/no/such/python"))
        .await
        .is_err());
    assert!(server
        .set_tool_paths(one("ruby_executable", python.to_str().unwrap()))
        .await
        .is_err());
    let saved = server
        .set_tool_paths(one("python_executable", &format!(" {} ", python.display())))
        .await
        .unwrap();
    assert_eq!(saved[0].saved, python.to_string_lossy());
    // Kept on the local context, as the WebView keeps it.
    let context = server
        .0
        .store
        .get_execution_context("local")
        .await
        .unwrap()
        .unwrap();
    let config: Value = serde_json::from_str(&context.config_json).unwrap();
    assert_eq!(
        config["python_executable"],
        python.to_string_lossy().as_ref()
    );
    // Emptied, it is looked for on the search path again.
    let cleared = server
        .set_tool_paths(one("python_executable", ""))
        .await
        .unwrap();
    assert!(cleared[0].saved.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn the_sci_and_agent_infini_keys_are_set_up_where_their_clis_read_them() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        Arc::new(desktop::MemorySecrets::default()),
    )
    .await
    .unwrap();
    // Under test the CLIs' home is inside the data directory.
    let home = temp.path().join("data/home");
    let settings = home.join(".scimaster/config.json");
    server
        .set_credential("scimaster_api_key", "sk-sci-test")
        .await
        .unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(written["apiKey"], "sk-sci-test");
    assert_eq!(written["defaults"]["limit"], 10);
    // Taken away, the file stays without the key.
    server
        .set_credential("scimaster_api_key", "")
        .await
        .unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert!(written.get("apiKey").is_none());
    assert!(server
        .service_keys()
        .await
        .iter()
        .all(|(name, _)| name != "SCIMASTER_API_KEY"));

    // InfiniSynapse needs its CLI, which is told the key.
    let missing = server
        .set_credential("infinisynapse_api_key", "sk-infini-test")
        .await
        .unwrap_err();
    assert!(
        missing.to_string().contains("agent_infini was not found"),
        "{missing}"
    );
    assert!(server.service_keys().await.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = home.join(".infini/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let heard = temp.path().join("heard.txt");
        let program = bin.join("agent_infini");
        std::fs::write(
            &program,
            format!("#!/bin/sh\necho \"$@\" > '{}'\n", heard.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        server
            .set_credential("infinisynapse_api_key", "sk-infini-test")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&heard).unwrap().trim(),
            "init --api-key sk-infini-test"
        );
        assert_eq!(
            server.service_keys().await,
            [(
                "INFINISYNAPSE_API_KEY".to_string(),
                "sk-infini-test".to_string()
            )]
        );
        // A failure says why without the key.
        std::fs::write(&program, "#!/bin/sh\necho \"bad key $3\" >&2\nexit 1\n").unwrap();
        let refused = server
            .set_credential("infinisynapse_api_key", "sk-other")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("bad key <redacted>") && !refused.contains("sk-other"),
            "{refused}"
        );
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_users_mcp_server_gives_turns_its_tools_and_each_asks_before_it_runs() {
    use axum::{http::HeaderMap, response::IntoResponse, routing::post, Json, Router};
    use desktop::{McpServerInput, McpTransport};

    // An MCP server on the loopback interface with one tool, `echo`.
    let tokens: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let heard = tokens.clone();
    let keys: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let heard_keys = keys.clone();
    let app = Router::new().route(
        "/mcp",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let heard = heard.clone();
            let heard_keys = heard_keys.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                heard.lock().unwrap().push(auth);
                if let Some(key) = headers.get("x-lab-key").and_then(|v| v.to_str().ok()) {
                    heard_keys.lock().unwrap().push(key.to_owned());
                }
                let Some(id) = body.get("id").cloned() else {
                    return axum::http::StatusCode::ACCEPTED.into_response();
                };
                let result = match body["method"].as_str().unwrap_or_default() {
                    "initialize" => json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake", "version": "1"}
                    }),
                    "tools/list" => json!({"tools": [{
                        "name": "echo",
                        "description": "Say it back",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}}
                        }
                    }]}),
                    "tools/call" => json!({
                        "content": [{
                            "type": "text",
                            "text": format!("echo: {}", body["params"]["arguments"]["text"].as_str().unwrap_or_default())
                        }],
                        "isError": false
                    }),
                    _ => json!({}),
                };
                Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    // A model that calls the server's tool once, then says what came back.
    struct Caller;
    #[async_trait]
    impl Provider for Caller {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let back = messages
                .iter()
                .rev()
                .find(|message| message.role == wisp_llm::Role::Tool)
                .map(|message| message.content.as_text());
            Ok(match back {
                Some(text) => Completion {
                    content: format!("It said: {text}"),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                },
                None => Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: wisp_llm::FunctionCall {
                            name: "fake__echo".into(),
                            arguments: json!({"text": "hi"}).to_string(),
                        },
                    }],
                    finish_reason: Some("tool_calls".into()),
                    ..Default::default()
                },
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(Caller));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();

    // What is refused before anything is saved.
    let input = |name: &str, url: &str| McpServerInput {
        id: None,
        name: name.into(),
        transport: McpTransport::Http { url: url.into() },
        token: None,
        env: None,
        headers: None,
        proxy: None,
    };
    assert!(server.save_mcp_server(input(" ", &url)).await.is_err());
    assert!(server
        .save_mcp_server(input("Fake", "fake.example"))
        .await
        .is_err());
    assert!(server.mcp_servers().await.unwrap().is_empty());

    let fake = server
        .save_mcp_server(McpServerInput {
            token: Some("tok-1".into()),
            ..input("Fake", &url)
        })
        .await
        .unwrap();
    assert!(fake.enabled);
    // Two servers may not answer to the same name.
    assert!(server.save_mcp_server(input("fake", &url)).await.is_err());
    let rows = server.mcp_servers().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].has_token && rows[0].error.is_none());

    // Asked now, it says what it offers, and it was shown the token.
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );
    assert!(tokens
        .lock()
        .unwrap()
        .iter()
        .all(|auth| auth == "Bearer tok-1"));
    assert!(server.test_mcp_server("missing").await.is_err());

    // A header of its own goes with every request; its value stays in the
    // keychain and only its name is saved.
    let with_header = |headers: Vec<(&str, &str)>| McpServerInput {
        id: Some(fake.id.clone()),
        headers: Some(
            headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        ),
        ..input("Fake", &url)
    };
    assert!(server
        .save_mcp_server(with_header(vec![("X Lab", "k1")]))
        .await
        .is_err());
    assert!(server
        .save_mcp_server(with_header(vec![("X-Lab-Key", "")]))
        .await
        .is_err());
    let saved = server
        .save_mcp_server(with_header(vec![("X-Lab-Key", "k1")]))
        .await
        .unwrap();
    assert_eq!(saved.headers, ["X-Lab-Key"]);
    assert!(!crate::mcp::load(&server.0.store)
        .await
        .iter()
        .any(|server| serde_json::to_string(server).unwrap().contains("k1")));
    server.test_mcp_server(&fake.id).await.unwrap();
    assert_eq!(keys.lock().unwrap().last().map(String::as_str), Some("k1"));
    // Given again with no value, it keeps the one saved; left out, it goes.
    server
        .save_mcp_server(with_header(vec![("X-Lab-Key", " ")]))
        .await
        .unwrap();
    keys.lock().unwrap().clear();
    server.test_mcp_server(&fake.id).await.unwrap();
    assert_eq!(keys.lock().unwrap().last().map(String::as_str), Some("k1"));
    assert!(server
        .save_mcp_server(with_header(vec![]))
        .await
        .unwrap()
        .headers
        .is_empty());
    keys.lock().unwrap().clear();
    server.test_mcp_server(&fake.id).await.unwrap();
    assert!(keys.lock().unwrap().is_empty());

    // In a turn its tool asks first, then runs.
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty()).await;
    // The card marks it as coming from an MCP server.
    assert_eq!(asked.approvals[0].tool, "mcp:fake__echo");
    server
        .answer_approval(
            PROJECT,
            &session,
            &asked.approvals[0].approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    let said = |snapshot: &dto::Snapshot| {
        snapshot
            .items
            .iter()
            .map(|item| item.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(said(&done).contains("It said: echo: hi"), "{}", said(&done));

    // Switched off, the model has no such tool, and nothing asks.
    server
        .set_mcp_server_enabled(&fake.id, false)
        .await
        .unwrap();
    let without = create(&server).await;
    send(&server, &without, "go").await.unwrap();
    let done = wait_for(&server, &without, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    assert!(!said(&done).contains("echo: hi"), "{}", said(&done));

    // A server that cannot be reached says why, and turns still run.
    let dead = server
        .save_mcp_server(input("Dead", "http://127.0.0.1:1/mcp"))
        .await
        .unwrap();
    assert!(server.test_mcp_server(&dead.id).await.is_err());
    let rows = server.mcp_servers().await.unwrap();
    assert!(rows
        .iter()
        .find(|row| row.server.id == dead.id)
        .unwrap()
        .error
        .is_some());
    let beside = create(&server).await;
    send(&server, &beside, "go").await.unwrap();
    wait_for(&server, &beside, |s| !s.running).await;

    // Renaming keeps the switch as it was; forgetting the token is explicit.
    let renamed = server
        .save_mcp_server(McpServerInput {
            id: Some(fake.id.clone()),
            token: Some(String::new()),
            ..input("Echo box", &url)
        })
        .await
        .unwrap();
    assert!(!renamed.enabled);
    assert!(!server.mcp_servers().await.unwrap()[0].has_token);
    server.set_mcp_server_enabled(&fake.id, true).await.unwrap();
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["echo_box__echo"]
    );

    server.delete_mcp_server(&dead.id).await.unwrap();
    assert!(server.delete_mcp_server(&dead.id).await.is_err());
    assert_eq!(server.mcp_servers().await.unwrap().len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_runs_only_on_the_hosts_attached_to_it() {
    use desktop::{ConversationOptions, SshHostRow};
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    for alias in ["gpu", "hpc"] {
        server
            .save_ssh_host(SshHostRow {
                alias: alias.into(),
                host_name: "10.0.0.8".into(),
                identity_file: "~/.ssh/id_lab".into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let session = create(&server).await;
    assert!(server
        .conversation_hosts(PROJECT, &session)
        .await
        .unwrap()
        .is_empty());

    // Attached, its runs may go there; the list says so.
    let attached = server
        .set_conversation_host(PROJECT, &session, "hpc", true)
        .await
        .unwrap();
    assert_eq!(attached, ["hpc"]);
    assert!(server
        .0
        .store
        .session_execution_context_enabled(&session, "ssh:hpc")
        .await
        .unwrap());
    // Choosing where new work runs attaches that host too, and it cannot
    // be taken away while it is the choice.
    let options = ConversationOptions {
        runs_on: "gpu".into(),
        ..server.stored_options(&session).await.unwrap()
    };
    server
        .set_conversation_options(PROJECT, &session, options)
        .await
        .unwrap();
    assert_eq!(
        server.conversation_hosts(PROJECT, &session).await.unwrap(),
        ["gpu", "hpc"]
    );
    let refused = server
        .set_conversation_host(PROJECT, &session, "gpu", false)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("choose another"), "{refused}");
    // Another one goes.
    assert_eq!(
        server
            .set_conversation_host(PROJECT, &session, "hpc", false)
            .await
            .unwrap(),
        ["gpu"]
    );
    // Only a saved host, of a conversation of this project.
    assert!(server
        .set_conversation_host(PROJECT, &session, "ghost", true)
        .await
        .is_err());
    assert!(server
        .set_conversation_host("missing", &session, "hpc", true)
        .await
        .is_err());
    assert!(server.conversation_hosts(PROJECT, "missing").await.is_err());

    // Where the project keeps things there: the defaults until saved.
    let shown = server.host_storage(PROJECT, "gpu").await.unwrap();
    assert!(!shown.saved);
    assert_eq!(shown.workdir_root, ".wisp-science/runs");
    assert!(shown.data_root.starts_with("~/wisp/") && shown.data_root.ends_with("/data"));
    assert_eq!(shown.results_dir, "remote/gpu");
    let saved = server
        .set_host_storage(
            PROJECT,
            "gpu",
            desktop::HostStorage {
                data_root: " /scratch/lab/data ".into(),
                workdir_root: "runs".into(),
                results_dir: "results/gpu".into(),
                saved: false,
            },
        )
        .await
        .unwrap();
    assert!(saved.saved);
    assert_eq!(
        (saved.data_root.as_str(), saved.workdir_root.as_str()),
        ("/scratch/lab/data", "runs")
    );
    // Paths the run manager could not use safely are refused.
    for (data, workdir, results) in [
        ("/scratch/$HOME", "runs", "results"),
        ("/scratch", "/abs/runs", "results"),
        ("/scratch", "runs", "../outside"),
        ("", "runs", "results"),
    ] {
        assert!(server
            .set_host_storage(
                PROJECT,
                "gpu",
                desktop::HostStorage {
                    data_root: data.into(),
                    workdir_root: workdir.into(),
                    results_dir: results.into(),
                    saved: false,
                },
            )
            .await
            .is_err());
    }
    // Said in words, not by the store's column names.
    let said = server
        .set_host_storage(
            PROJECT,
            "gpu",
            desktop::HostStorage {
                data_root: "/scratch".into(),
                workdir_root: "runs".into(),
                results_dir: "../x".into(),
                saved: false,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        said.starts_with("The results folder must not contain"),
        "{said}"
    );
    assert_eq!(server.host_storage(PROJECT, "gpu").await.unwrap(), saved);
    assert!(server.host_storage(PROJECT, "ghost").await.is_err());
    assert!(server.host_storage("missing", "gpu").await.is_err());

    // Files go to and from a saved host that has passed a check, never to
    // one that is not saved or was never reached.
    let place = temp.path().join("copy.csv");
    let unknown = server
        .download_from_host(
            PROJECT,
            Some(&session),
            "ghost",
            "/data/a.csv",
            place.clone(),
        )
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("no longer saved"), "{unknown}");
    assert!(server
        .download_from_host(PROJECT, Some(&session), "gpu", "/data/a.csv", place.clone())
        .await
        .is_err());
    assert!(!place.exists());
    let none = server
        .upload_to_host(PROJECT, None, "gpu", "/data", vec![])
        .await
        .unwrap_err();
    assert!(none.to_string().contains("Choose"), "{none}");
    assert!(server
        .upload_to_host(PROJECT, None, "gpu", "/data", vec!["/tmp/x".into()])
        .await
        .is_err());
    // Only a conversation of the project, and only the project's.
    assert!(server
        .upload_to_host("missing", None, "gpu", "/data", vec!["/tmp/x".into()])
        .await
        .is_err());
    assert!(server
        .download_from_host(PROJECT, Some("missing"), "gpu", "/a", place.clone())
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn ssh_hosts_are_registered_as_execution_contexts_without_their_keys() {
    use desktop::SshHostRow;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let start = server.environments().await.unwrap();
    assert!(start.hosts.is_empty());
    assert_eq!(start.default, None);

    let gpu = SshHostRow {
        alias: " gpu ".into(),
        host_name: "10.0.0.8".into(),
        user: "lab".into(),
        port: Some(2222),
        identity_file: "~/.ssh/id_lab".into(),
        notes: "four A100".into(),
        ..Default::default()
    };
    server.save_ssh_host(gpu.clone()).await.unwrap();
    for wrong in [
        SshHostRow::default(),
        SshHostRow {
            alias: "bad alias; rm -rf".into(),
            ..Default::default()
        },
        SshHostRow {
            alias: "ok".into(),
            notes: "n".repeat(501),
            ..Default::default()
        },
    ] {
        assert!(server.save_ssh_host(wrong).await.is_err());
    }
    let view = server.environments().await.unwrap();
    assert_eq!(
        view.hosts,
        [SshHostRow {
            alias: "gpu".into(),
            ..gpu.clone()
        }]
    );

    // The host is an execution context a run can name, holding the path of
    // the key and not a key.
    let contexts = server.0.store.list_execution_contexts().await.unwrap();
    let context = contexts.iter().find(|ctx| ctx.id == "ssh:gpu").unwrap();
    assert!(context.config_json.contains("10.0.0.8"));
    assert!(context.config_json.contains("id_lab"));
    assert!(!context.config_json.contains("PRIVATE KEY"));

    // Saving the same alias changes the host; it does not add another.
    server
        .save_ssh_host(SshHostRow {
            alias: "gpu".into(),
            host_name: "gpu.lab.example".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let view = server.environments().await.unwrap();
    assert_eq!(view.hosts.len(), 1);
    assert_eq!(view.hosts[0].host_name, "gpu.lab.example");
    assert_eq!(view.hosts[0].user, "");

    // Signing in by password needs one; it is kept as a secret, said to
    // be saved, never listed or written to the database, and dropped when
    // the host goes back to a key.
    let by_password = SshHostRow {
        alias: "gpu".into(),
        host_name: "gpu.lab.example".into(),
        password_sign_in: true,
        ..Default::default()
    };
    assert!(server.save_ssh_host(by_password.clone()).await.is_err());
    server
        .save_ssh_host(SshHostRow {
            password: Some("made-up-pass".into()),
            ..by_password.clone()
        })
        .await
        .unwrap();
    let secret = wisp_runs::ssh_hosts::password_secret_name("gpu");
    assert_eq!(
        server.0.secrets.get(&secret).as_deref(),
        Some("made-up-pass")
    );
    let listed = &server.environments().await.unwrap().hosts[0];
    assert!(listed.password_sign_in && listed.has_password);
    assert_eq!(listed.password, None);
    let saved = server
        .0
        .store
        .get_execution_context("ssh:gpu")
        .await
        .unwrap()
        .unwrap();
    assert!(!saved.config_json.contains("made-up-pass"));
    // Saved again without typing it, the password stays.
    server.save_ssh_host(by_password.clone()).await.unwrap();
    assert!(server.environments().await.unwrap().hosts[0].has_password);
    server
        .save_ssh_host(SshHostRow {
            password_sign_in: false,
            ..by_password.clone()
        })
        .await
        .unwrap();
    assert!(!server.environments().await.unwrap().hosts[0].has_password);
    assert_eq!(server.0.secrets.get(&secret).unwrap_or_default(), "");
    server
        .save_ssh_host(SshHostRow {
            password: Some("made-up-pass".into()),
            ..by_password
        })
        .await
        .unwrap();

    // New work can be sent there by default, and back to this computer.
    server.set_default_environment(Some("gpu")).await.unwrap();
    assert_eq!(
        server.environments().await.unwrap().default.as_deref(),
        Some("gpu")
    );
    assert!(server
        .set_default_environment(Some("nowhere"))
        .await
        .is_err());
    server.set_default_environment(None).await.unwrap();
    assert_eq!(server.environments().await.unwrap().default, None);

    // Removing the host removes its context and a default that named it.
    server.set_default_environment(Some("gpu")).await.unwrap();
    server.remove_ssh_host("gpu").await.unwrap();
    assert!(server.remove_ssh_host("gpu").await.is_err());
    let view = server.environments().await.unwrap();
    assert!(view.hosts.is_empty());
    assert_eq!(view.default, None);
    // Its password goes with it.
    assert_eq!(
        server
            .0
            .secrets
            .get(&wisp_runs::ssh_hosts::password_secret_name("gpu"))
            .unwrap_or_default(),
        ""
    );
    assert!(server
        .0
        .store
        .get_execution_context("ssh:gpu")
        .await
        .unwrap()
        .is_none());
    // Only hosts that are in the SSH config can be imported from it.
    assert!(server
        .import_ssh_hosts(&["surely-not-a-configured-host-name".into()])
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn turn_defaults_and_project_grants_apply_from_the_next_turn() {
    use desktop::Remember;
    let temp = tempfile::tempdir().unwrap();
    // Counts the calls the model was given no tools for: the agent's last
    // word when a turn runs out of steps.
    struct Counting(FakeModel, Arc<AtomicUsize>);
    #[async_trait]
    impl Provider for Counting {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn model(&self) -> &str {
            self.0.model()
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            self.0.complete(messages, tools).await
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if tools.is_empty() {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            self.0.stream(messages, tools, sink).await
        }
    }
    let out_of_steps = Arc::new(AtomicUsize::new(0));
    let counter = out_of_steps.clone();
    let factory: ModelFactory = Arc::new(move |_| {
        Box::new(Counting(
            FakeModel {
                calls: Arc::default(),
                write: true,
            },
            counter.clone(),
        ))
    });
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();

    // Until something is saved, turns run as the host was started.
    let start = server.turn_defaults().await.unwrap();
    assert_eq!(start.max_iterations, 5);
    assert!(start.auto_compact && !start.full_access);
    assert_eq!(start.auto_compact_percent, 80);
    assert_eq!(start.compact_range, (10, 95));
    assert!(!start.auto_continue);
    assert_eq!(start.auto_continue_limit, 10);
    for wrong in [
        desktop::TurnDefaults {
            auto_continue_limit: 0,
            ..start
        },
        desktop::TurnDefaults {
            auto_continue_limit: 101,
            ..start
        },
        desktop::TurnDefaults {
            max_iterations: 0,
            ..start
        },
        desktop::TurnDefaults {
            max_iterations: 1_001,
            ..start
        },
        desktop::TurnDefaults {
            auto_compact_percent: 9,
            ..start
        },
        desktop::TurnDefaults {
            auto_compact_percent: 96,
            ..start
        },
    ] {
        assert!(server.set_turn_defaults(wrong).await.is_err());
    }
    assert_eq!(server.turn_defaults().await.unwrap(), start);
    let saved = server
        .set_turn_defaults(desktop::TurnDefaults {
            max_iterations: 7,
            auto_compact: false,
            auto_compact_percent: 60,
            full_access: true,
            auto_continue: true,
            auto_continue_limit: 3,
            ..start
        })
        .await
        .unwrap();
    assert!(saved.auto_continue);
    assert_eq!(saved.auto_continue_limit, 3);
    assert_eq!(saved.max_iterations, 7);
    assert!(!saved.auto_compact && saved.full_access);
    assert_eq!(saved.auto_compact_percent, 60);
    assert_eq!(server.turn_defaults().await.unwrap(), saved);

    // A conversation made by the desktop starts the way the default says;
    // one made before keeps what it had.
    let trusting = server.create_conversation(PROJECT).await.unwrap();
    let full = |id: String| {
        let server = &server;
        async move {
            server
                .conversation_status(PROJECT, &id)
                .await
                .unwrap()
                .options
                .full_access
        }
    };
    assert!(full(trusting.clone()).await);
    server
        .set_turn_defaults(desktop::TurnDefaults {
            full_access: false,
            ..saved
        })
        .await
        .unwrap();
    let asking = server.create_conversation(PROJECT).await.unwrap();
    assert!(!full(asking.clone()).await);
    assert!(full(trusting).await);
    assert!(server.create_conversation("missing").await.is_err());

    // "Always" cannot be said while denying, and the request stays open.
    send(&server, &asking, "write the answer").await.unwrap();
    let approval = wait_for(&server, &asking, |s| !s.approvals.is_empty())
        .await
        .approvals[0]
        .approval_id
        .clone();
    assert!(server
        .answer_approval(PROJECT, &asking, &approval, false, Remember::Project)
        .await
        .is_err());
    assert!(server.project_grants().await.unwrap().is_empty());
    server
        .answer_approval(PROJECT, &asking, &approval, true, Remember::Project)
        .await
        .unwrap();
    wait_for(&server, &asking, |s| !s.running).await;
    assert_eq!(
        server.project_grants().await.unwrap(),
        [desktop::ProjectGrants {
            project_id: PROJECT.into(),
            project_name: "Server".into(),
            tools: vec!["write".into()],
        }]
    );

    // Another conversation of the project is not asked at all.
    let second = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &second, "write the answer").await.unwrap();
    let done = wait_for(&server, &second, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    assert!(done.items.iter().any(|item| item.role == "tool"));

    // Taken back, the next conversation is asked again.
    server.revoke_project_grant(PROJECT, "write").await.unwrap();
    assert!(server.revoke_project_grant(PROJECT, "write").await.is_err());
    assert!(server.project_grants().await.unwrap().is_empty());
    let third = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &third, "write the answer").await.unwrap();
    let approval = wait_for(&server, &third, |s| !s.approvals.is_empty())
        .await
        .approvals[0]
        .approval_id
        .clone();
    server
        .answer_approval(PROJECT, &third, &approval, true, Remember::No)
        .await
        .unwrap();
    wait_for(&server, &third, |s| !s.running).await;

    // Allowed in every project, in the WebView's list, kept beside what is
    // already there.
    server
        .0
        .store
        .set_setting(
            "approval_grants",
            r#"{"project":{"p":[{"kind":"tool","target":"read"}]}}"#,
        )
        .await
        .unwrap();
    let everywhere = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &everywhere, "write the answer")
        .await
        .unwrap();
    let approval = wait_for(&server, &everywhere, |s| !s.approvals.is_empty())
        .await
        .approvals[0]
        .approval_id
        .clone();
    server
        .answer_approval(PROJECT, &everywhere, &approval, true, Remember::Everywhere)
        .await
        .unwrap();
    wait_for(&server, &everywhere, |s| !s.running).await;
    assert_eq!(server.global_grants().await.unwrap(), ["write"]);
    let saved_grants: serde_json::Value = serde_json::from_str(
        &server
            .0
            .store
            .get_setting("approval_grants")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved_grants["project"]["p"][0]["target"], "read");
    let fourth = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &fourth, "write the answer").await.unwrap();
    assert!(wait_for(&server, &fourth, |s| !s.running)
        .await
        .approvals
        .is_empty());
    server.revoke_global_grant("write").await.unwrap();
    assert!(server.revoke_global_grant("write").await.is_err());
    assert!(server.global_grants().await.unwrap().is_empty());

    // The step limit is read at the start of each turn. Writing and then
    // answering takes two steps; with one, the turn runs out after the
    // write and the agent is asked for a last word without tools.
    assert_eq!(out_of_steps.load(Ordering::Relaxed), 0);
    server
        .set_turn_defaults(desktop::TurnDefaults {
            max_iterations: 1,
            full_access: true,
            ..saved
        })
        .await
        .unwrap();
    let limited = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &limited, "write the answer").await.unwrap();
    wait_for(&server, &limited, |s| !s.running).await;
    assert_eq!(out_of_steps.load(Ordering::Relaxed), 1);
    server.shutdown().await;
}

/// Records every message of every request, whatever its role.
struct Everything(Arc<std::sync::Mutex<Vec<String>>>);
#[async_trait]
impl Provider for Everything {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "noted".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        {
            let mut seen = self.0.lock().unwrap();
            seen.extend(messages.iter().map(|message| message.content.as_text()));
            seen.extend(tools.iter().map(|tool| format!("{tool:?}")));
        }
        sink.on_text("noted");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn memos_are_the_researchers_own_and_never_shown_to_the_model() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    let draft = server
        .add_memo(
            PROJECT,
            "  Prompt draft ",
            " ask about the quokka-7 plasmid ",
        )
        .await
        .unwrap();
    assert_eq!(draft.title, "Prompt draft");
    assert_eq!(draft.body, "ask about the quokka-7 plasmid");
    assert!(!draft.pinned);
    // A title is optional; a body is not, and both have limits.
    let untitled = server.add_memo(PROJECT, "", "ssh lab-gpu").await.unwrap();
    for (title, body) in [
        ("empty", "  ".to_string()),
        ("long body", "x".repeat(20_001)),
        (&"t".repeat(201), "body".to_string()),
    ] {
        assert!(server.add_memo(PROJECT, title, &body).await.is_err());
    }
    assert!(server.add_memo("missing", "", "body").await.is_err());
    assert!(server.memos("missing").await.is_err());

    let titles = |memos: Vec<desktop::ProjectMemo>| {
        memos.into_iter().map(|memo| memo.body).collect::<Vec<_>>()
    };
    // Newest first, until one is pinned.
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ssh lab-gpu", "ask about the quokka-7 plasmid"]
    );
    let patch = |id: &str| desktop::ProjectMemoPatch {
        id: id.into(),
        title: None,
        body: None,
        pinned: None,
    };
    let pinned = server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                pinned: Some(true),
                ..patch(&draft.id)
            },
        )
        .await
        .unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.body, draft.body);
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ask about the quokka-7 plasmid", "ssh lab-gpu"]
    );
    let edited = server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                title: Some("Server".into()),
                body: Some("ssh lab-gpu -p 2222".into()),
                ..patch(&untitled.id)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (edited.title.as_str(), edited.body.as_str()),
        ("Server", "ssh lab-gpu -p 2222")
    );
    assert!(server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                body: Some(" ".into()),
                ..patch(&untitled.id)
            }
        )
        .await
        .is_err());

    // A memo belongs to its project: another one neither lists nor changes it.
    assert!(server.memos(&lab).await.unwrap().is_empty());
    assert!(server.update_memo(&lab, patch(&draft.id)).await.is_err());
    assert!(server.delete_memo(&lab, &draft.id).await.is_err());

    // A whole turn runs without the memo reaching the model in any message
    // or tool description.
    let session = create(&server).await;
    send(&server, &session, "what should I ask next?")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("what should I ask next?"));
    assert!(!seen.contains("quokka-7"), "a memo reached the model");
    assert!(!seen.contains("lab-gpu"), "a memo reached the model");

    server.delete_memo(PROJECT, &draft.id).await.unwrap();
    assert!(server.delete_memo(PROJECT, &draft.id).await.is_err());
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ssh lab-gpu -p 2222"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn memory_reaches_the_model_only_while_it_is_on_and_is_never_saved_with_a_turn() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    // Memory starts on and empty.
    let view = server.memory(&lab).await.unwrap();
    assert!(view.enabled);
    assert!(view.notes.is_empty() && view.habits.is_empty());
    assert!(view.today.ends_with(".md"));
    assert!(server.memory("missing").await.is_err());

    // A remembered line lands in today's note of that project only.
    server
        .remember(&lab, None, "  the queue is called gpu-long  ", false)
        .await
        .unwrap();
    let view = server.memory(&lab).await.unwrap();
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].name, view.today);
    assert_eq!(view.notes[0].preview.trim(), "the queue is called gpu-long");
    assert!(folder.join(".wisp/memory").join(&view.today).is_file());
    assert!(server.memory(PROJECT).await.unwrap().notes.is_empty());
    for (text, why) in [("  ", "empty"), (&"x".repeat(4_001), "too long")] {
        assert!(
            server.remember(&lab, None, text, false).await.is_err(),
            "{why}"
        );
        assert!(server.add_habit(text).await.is_err(), "{why}");
    }

    // A note is edited as a whole file, by a name that stays in the folder.
    server
        .save_memory_note(&lab, "2026-01-02.md", "older finding\n")
        .await
        .unwrap();
    assert_eq!(
        server.memory_note(&lab, "2026-01-02.md").await.unwrap(),
        "older finding\n"
    );
    assert_eq!(server.memory_note(&lab, "2020-01-01.md").await.unwrap(), "");
    for name in [
        "",
        "notes.txt",
        "../escape.md",
        "sub/note.md",
        "sub\\note.md",
        ".hidden.md",
    ] {
        assert!(
            server.save_memory_note(&lab, name, "x").await.is_err(),
            "{name}"
        );
        assert!(server.memory_note(&lab, name).await.is_err(), "{name}");
        assert!(
            server.delete_memory_note(&lab, name).await.is_err(),
            "{name}"
        );
    }
    assert!(server
        .save_memory_note(&lab, "2026-01-02.md", "  ")
        .await
        .is_err());
    let names: Vec<_> = server
        .memory(&lab)
        .await
        .unwrap()
        .notes
        .into_iter()
        .map(|note| note.name)
        .collect();
    assert_eq!(names, [view.today.clone(), "2026-01-02.md".into()]);
    server
        .delete_memory_note(&lab, "2026-01-02.md")
        .await
        .unwrap();
    assert!(server
        .delete_memory_note(&lab, "2026-01-02.md")
        .await
        .is_err());

    // Habits belong to the user, not to a project.
    let habit = server.add_habit(" always report medians ").await.unwrap();
    assert_eq!(habit.content, "always report medians");
    server
        .update_habit(&habit.id, "always report medians and IQR")
        .await
        .unwrap();
    assert!(server.update_habit("missing", "text").await.is_err());
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    server
        .remember(&lab, Some(&session), "prefers vector figures", true)
        .await
        .unwrap();
    // A conversation of another project cannot be named as the source.
    assert!(server
        .remember(PROJECT, Some(&session), "stolen", true)
        .await
        .is_err());
    let habits = |view: desktop::MemoryView| {
        view.habits
            .into_iter()
            .map(|habit| habit.content)
            .collect::<Vec<_>>()
    };
    // Most recently written or changed first; the edit above counts.
    let expected = ["always report medians and IQR", "prefers vector figures"];
    assert_eq!(habits(server.memory(&lab).await.unwrap()), expected);
    assert_eq!(habits(server.memory(PROJECT).await.unwrap()), expected);

    // On: the turn carries the habits and can search the notes.
    let turn = |message: &'static str| {
        let (server, lab, seen) = (&server, &lab, &seen);
        async move {
            seen.lock().unwrap().clear();
            let session = call(server, "native_conversation_create", lab, json!({}))
                .await
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            call(
                server,
                "native_conversation_send",
                lab,
                json!({"session_id": session, "request_id": message, "message": message}),
            )
            .await
            .unwrap();
            let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        lab,
                        json!({"session_id": session}),
                    )
                    .await
                    .unwrap();
                    let snapshot: dto::Snapshot = serde_json::from_value(value).unwrap();
                    if !snapshot.running {
                        return snapshot;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            (seen.lock().unwrap().join("\n"), done)
        }
    };
    // The system prompt always explains the block; this is the block itself.
    const BLOCK: &str = "<global_memory>\nHost-provided user context";
    let (shown, saved) = turn("plot the counts").await;
    assert!(shown.contains(BLOCK));
    assert!(shown.contains("always report medians and IQR"));
    assert!(shown.contains("search_memory"));
    // The habits were for that turn; the saved message is what was typed.
    let user: Vec<_> = saved
        .items
        .iter()
        .filter(|item| item.role == "user")
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(user, ["plot the counts"]);

    // Off: neither reaches the model, and nothing was deleted.
    server.set_memory_enabled(false).await.unwrap();
    let (shown, _) = turn("plot them again").await;
    assert!(shown.contains("plot them again"));
    assert!(!shown.contains(BLOCK));
    assert!(!shown.contains("always report medians"));
    assert!(!shown.contains("search_memory"));
    let view = server.memory(&lab).await.unwrap();
    assert!(!view.enabled);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.habits.len(), 2);

    server.set_memory_enabled(true).await.unwrap();
    server.delete_habit(&habit.id).await.unwrap();
    let (shown, _) = turn("and once more").await;
    assert!(shown.contains("prefers vector figures"));
    assert!(!shown.contains("always report medians"));
    server.shutdown().await;
}

#[tokio::test]
async fn favorites_are_snapshots_that_outlive_the_conversation_they_came_from() {
    let temp = tempfile::tempdir().unwrap();
    let settings = config(temp.path());
    let library = settings.data.join("library.sqlite");
    let server = Server::open_with_models(settings, models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("figs")).unwrap();
    std::fs::create_dir_all(folder.join(".wisp")).unwrap();
    let picture = b"\x89PNG\r\n\x1a\nnot really a picture".to_vec();
    std::fs::write(folder.join("figs/plot.png"), &picture).unwrap();
    std::fs::write(folder.join(".wisp/secret.png"), &picture).unwrap();
    std::fs::write(folder.join("notes.txt"), "plain").unwrap();
    std::fs::write(temp.path().join("outside.png"), &picture).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    // A host nobody saved anything on has no library file.
    assert!(!library.exists());

    let finding = "  The knockout halves viability.\nIt holds in both lines.";
    let text = server
        .star_text(&lab, &session, "text", None, finding)
        .await
        .unwrap();
    assert_eq!(text.kind, "text");
    assert_eq!(text.title, "The knockout halves viability.");
    assert_eq!(text.code, finding);
    assert_eq!(text.source_project_name, "lab");
    assert_eq!(text.source_session_id, session);
    assert!(library.exists());
    // Saving the same passage again is the same item.
    let again = server
        .star_text(&lab, &session, "text", None, finding)
        .await
        .unwrap();
    assert_eq!(again.id, text.id);
    let code = server
        .star_text(
            &lab,
            &session,
            "code",
            Some(" python "),
            "import scanpy as sc\n",
        )
        .await
        .unwrap();
    assert_eq!(code.language.as_deref(), Some("python"));
    for (kind, body) in [
        ("figure", "x".to_string()),
        ("text", "  \n".to_string()),
        ("text", "x".repeat(2 * 1024 * 1024 + 1)),
    ] {
        assert!(server
            .star_text(&lab, &session, kind, None, &body)
            .await
            .is_err());
    }
    assert!(server
        .star_text(PROJECT, &session, "text", None, "stolen")
        .await
        .is_err());

    // A figure is a copy of an image inside the project.
    let figure = server
        .star_figure(&lab, &session, "figs/plot.png")
        .await
        .unwrap();
    assert_eq!(figure.kind, "figure");
    assert_eq!(figure.title, "plot.png");
    assert_eq!(figure.content_type.as_deref(), Some("image/png"));
    assert_eq!(figure.source_path.as_deref(), Some("figs/plot.png"));
    for path in [
        "notes.txt",
        "../outside.png",
        ".wisp/secret.png",
        "figs/missing.png",
    ] {
        assert!(
            server.star_figure(&lab, &session, path).await.is_err(),
            "{path}"
        );
    }
    std::fs::remove_file(folder.join("figs/plot.png")).unwrap();
    let kept = server.favorite(&figure.id).await.unwrap();
    assert_eq!(kept.image.as_deref(), Some(picture.as_slice()));
    assert_eq!(server.favorite(&text.id).await.unwrap().image, None);

    let ids = |items: Vec<desktop::LibraryItemSummary>| {
        items.into_iter().map(|item| item.id).collect::<Vec<_>>()
    };
    let newest_first = vec![figure.id.clone(), code.id.clone(), text.id.clone()];
    assert_eq!(ids(server.favorites("", None).await.unwrap()), newest_first);
    assert_eq!(
        ids(server.favorites(" KNOCKOUT ", None).await.unwrap()),
        [text.id.clone()]
    );
    assert_eq!(
        ids(server.favorites("", Some("figure")).await.unwrap()),
        [figure.id.clone()]
    );
    // The project's name finds what came from it.
    assert_eq!(
        ids(server.favorites("lab", Some("code")).await.unwrap()),
        [code.id.clone()]
    );
    assert!(server
        .favorites("nothing like it", None)
        .await
        .unwrap()
        .is_empty());
    let here: Vec<_> = server
        .conversation_favorites(&lab, &session)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(here, newest_first);
    assert!(server
        .conversation_favorites(PROJECT, &session)
        .await
        .is_err());

    // The conversation goes; what was saved from it stays.
    server.delete_conversation(&lab, &session).await.unwrap();
    assert_eq!(ids(server.favorites("", None).await.unwrap()), newest_first);
    assert_eq!(server.favorite(&text.id).await.unwrap().item.code, finding);

    server.unstar(&text.id).await.unwrap();
    assert!(server.unstar(&text.id).await.is_err());
    assert!(server.favorite(&text.id).await.is_err());
    assert_eq!(
        ids(server.favorites("", None).await.unwrap()),
        [figure.id, code.id]
    );
    server.shutdown().await;
}

/// Records what the model was shown, turn by turn.
struct Recorder(Arc<std::sync::Mutex<Vec<Vec<String>>>>);
#[async_trait]
impl Provider for Recorder {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "noted".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        self.0.lock().unwrap().push(
            messages
                .iter()
                .filter(|message| message.role == wisp_llm::Role::User)
                .map(|message| message.content.as_text())
                .collect(),
        );
        sink.on_text("noted");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn a_model_that_can_see_is_given_attached_pictures() {
    for sees in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = config(temp.path());
        cfg.model.as_mut().unwrap().supports_vision = sees;
        let uploads = cfg.workspace.join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        image::RgbImage::from_pixel(4, 3, image::Rgb([20, 120, 200]))
            .save(uploads.join("plot.png"))
            .unwrap();
        let server = Server::open_with_models(cfg, models(Arc::default(), false))
            .await
            .unwrap();
        let session = create(&server).await;
        server
            .send(
                PROJECT,
                dto::SendRequest {
                    session_id: session.clone(),
                    request_id: "r1".into(),
                    message: "What does this show?".into(),
                    resume: false,
                    resume_after: None,
                    attachments: vec!["uploads/plot.png".into()],
                    references: vec![],
                    acp_agent_id: None,
                },
            )
            .await
            .unwrap();
        wait_for(&server, &session, |s| !s.running).await;
        let saved = server.0.store.load_messages(&session).await.unwrap();
        let asked = saved
            .iter()
            .find(|message| message.role == wisp_llm::Role::User)
            .unwrap();
        // The path is in the message either way; the picture only for a
        // model that can see it.
        assert!(asked.content.as_text().contains("uploads/plot.png"));
        let pictures = match &asked.content {
            wisp_llm::Content::Parts(parts) => parts
                .iter()
                .filter(|part| matches!(part, wisp_llm::Part::Image { .. }))
                .count(),
            wisp_llm::Content::Text(_) => 0,
        };
        assert_eq!(pictures, usize::from(sees), "{:?}", asked.content);
        server.shutdown().await;
    }
}

/// Says what a picture shows, as a model that can see would.
struct Describer;
#[async_trait]
impl Provider for Describer {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "gpt-4o"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "A blue square.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        sink.on_text("A blue square.");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn a_model_that_cannot_see_is_told_what_a_saved_one_saw() {
    use desktop::{ModelFields, ModelInput};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let uploads = cfg.workspace.join("uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    image::RgbImage::from_pixel(4, 3, image::Rgb([20, 120, 200]))
        .save(uploads.join("plot.png"))
        .unwrap();
    let shown: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = shown.clone();
    // No request leaves the test: every model is a fake, by its ID.
    let factory: ModelFactory = Arc::new(move |provider| match provider.model.as_str() {
        "gpt-4o" => Box::new(Describer),
        _ => Box::new(Recorder(record.clone())),
    });
    let server = Server::open_with(cfg, factory, Arc::new(desktop::MemorySecrets::default()))
        .await
        .unwrap();
    let input = |name: &str, api_url: &str, model: &str| ModelInput {
        id: None,
        name: name.into(),
        fields: ModelFields {
            provider: "openai".into(),
            api_url: api_url.into(),
            model: model.into(),
            max_context: "32000".into(),
            max_tokens: "4000".into(),
            reasoning_effort: String::new(),
            proxy: String::new(),
        },
        api_key: Some("made-up-key".into()),
    };
    // The conversation's model is the first saved, and cannot see.
    let saved = server
        .save_model_profile(input("Text", "http://127.0.0.1:9/v1", "lab-model"))
        .await
        .unwrap();
    let text = saved.profiles[0].clone();
    assert!(!text.sees);
    let session = create(&server).await;
    let ask = |request: &str| dto::SendRequest {
        session_id: session.clone(),
        request_id: request.into(),
        message: "What does this show?".into(),
        resume: false,
        resume_after: None,
        attachments: vec!["uploads/plot.png".into()],
        references: vec![],
        acp_agent_id: None,
    };
    let last_shown = || {
        shown
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
            .join("\n")
    };
    // With no model that can see, the path is all it gets, and the turn
    // goes on.
    server.send(PROJECT, ask("r1")).await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let first = last_shown();
    assert!(first.contains("uploads/plot.png"), "{first}");
    assert!(!first.contains("image_observations"), "{first}");

    // A saved model that can see describes the picture for it.
    let saved = server
        .save_model_profile(input("Eyes", "https://api.openai.com/v1", "gpt-4o"))
        .await
        .unwrap();
    let eyes = saved
        .profiles
        .iter()
        .find(|profile| profile.name == "Eyes")
        .unwrap()
        .clone();
    assert!(eyes.sees);
    server.send(PROJECT, ask("r2")).await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let second = last_shown();
    assert!(second.contains("image_observations"), "{second}");
    assert!(second.contains("A blue square."), "{second}");

    // Only a model that can see can be chosen to read pictures.
    assert_eq!(server.picture_model().await.unwrap(), "");
    assert!(server.set_picture_model(&text.id).await.is_err());
    assert!(server.set_picture_model("gone").await.is_err());
    assert_eq!(server.set_picture_model(&eyes.id).await.unwrap(), eyes.id);
    assert_eq!(server.set_picture_model("").await.unwrap(), "");
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_messages_carry_project_files_and_turn_scoped_references() {
    use wisp_dto::ComposerReferenceArg as Reference;
    let temp = tempfile::tempdir().unwrap();
    let shown: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Recorder(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("data")).unwrap();
    std::fs::create_dir_all(folder.join("skills/demo")).unwrap();
    std::fs::write(folder.join("data/counts.csv"), "gene,count\n").unwrap();
    std::fs::write(
        folder.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Count reads the lab way\n---\nAlways report medians.\n",
    )
    .unwrap();
    let outside = temp.path().join("notes.txt");
    std::fs::write(&outside, "field notes").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let new_conversation = || async {
        call(&server, "native_conversation_create", &lab, json!({}))
            .await
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    };
    let request = |session: &str, id: &str, message: &str| dto::SendRequest {
        session_id: session.into(),
        request_id: id.into(),
        message: message.into(),
        resume: false,
        resume_after: None,
        attachments: vec![],
        references: vec![],
        acp_agent_id: None,
    };
    let finished = |session: String| {
        let (server, lab) = (&server, &lab);
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        lab,
                        json!({"session_id": session}),
                    )
                    .await
                    .unwrap();
                    let snapshot: dto::Snapshot = serde_json::from_value(value).unwrap();
                    if !snapshot.running {
                        return snapshot;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
    };

    // Picking files: one outside the project is copied in, one inside is not.
    let files = server
        .import_attachments(&lab, &[folder.join("data/counts.csv"), outside])
        .await
        .unwrap();
    assert_eq!(files, ["data/counts.csv", "uploads/notes.txt"]);
    let skills = server.skill_options(&lab, "lab COUNT", 5).await.unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].label, "demo");
    assert!(server
        .skill_options(&lab, "absent", 5)
        .await
        .unwrap()
        .is_empty());

    let earlier = new_conversation().await;
    server
        .send(&lab, request(&earlier, "r1", "alpha finding about TP53"))
        .await
        .unwrap();
    finished(earlier.clone()).await;

    let session = new_conversation().await;
    // Nothing that cannot be honoured is sent, and nothing is saved for it.
    for (attachments, references) in [
        (vec!["../notes.txt".to_string()], vec![]),
        (vec!["data".to_string()], vec![]),
        (
            vec![],
            vec![Reference::Skill {
                name: "ghost".into(),
            }],
        ),
        (vec![], vec![Reference::Session { id: "ghost".into() }]),
        (vec![], vec![Reference::Artifact { id: "a".into() }]),
    ] {
        let mut bad = request(&session, "bad", "use these");
        bad.attachments = attachments;
        bad.references = references;
        assert!(server.send(&lab, bad).await.is_err());
    }
    assert!(finished(session.clone()).await.items.is_empty());
    assert!(server
        .send("other", request(&session, "bad", "hello"))
        .await
        .is_err());

    let mut full = request(&session, "r2", "use these");
    full.attachments = files.clone();
    full.references = vec![
        Reference::Skill {
            name: "demo".into(),
        },
        Reference::Session {
            id: earlier.clone(),
        },
        Reference::Skill {
            name: "demo".into(),
        },
    ];
    server.send(&lab, full).await.unwrap();
    let done = finished(session.clone()).await;
    // The files are part of the saved message and listed on it.
    let asked = &done.items[0];
    assert_eq!(
        asked.text,
        "use these\n\nUploaded files: data/counts.csv, uploads/notes.txt"
    );
    assert_eq!(asked.attachments, files);
    // The model saw the references ahead of the request, each once.
    let turn = shown.lock().unwrap().last().unwrap().clone();
    let position = |needle: &str| turn.iter().position(|text| text.contains(needle));
    let request_at = position("use these").unwrap();
    assert!(position("alpha finding about TP53").unwrap() < request_at);
    assert!(position("Always report medians.").unwrap() < request_at);
    assert_eq!(
        turn.iter()
            .filter(|text| text.contains("# Skill: demo"))
            .count(),
        1
    );
    // They belong to that turn only: not saved, and not shown to the next.
    let saved = server.0.store.load_messages(&session).await.unwrap();
    assert!(saved
        .iter()
        .all(|message| !message.content.as_text().contains("Always report medians.")));
    server
        .send(&lab, request(&session, "r3", "and now?"))
        .await
        .unwrap();
    finished(session.clone()).await;
    let next = shown.lock().unwrap().last().unwrap().clone();
    assert!(next.iter().any(|text| text.contains("and now?")));
    assert!(next
        .iter()
        .all(|text| !text.contains("Always report medians.")));

    // A message can be files alone; it is named after the first one.
    let files_only = new_conversation().await;
    let mut bare = request(&files_only, "r4", "  ");
    bare.attachments = vec!["data/counts.csv".into()];
    server.send(&lab, bare).await.unwrap();
    let bare = finished(files_only.clone()).await;
    assert_eq!(bare.items[0].text, "Uploaded files: data/counts.csv");
    assert_eq!(
        server
            .conversations(&lab, "", false)
            .await
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == files_only)
            .unwrap()
            .title,
        "data/counts.csv"
    );

    // The remote send command still takes text only.
    let refused = call(
        &server,
        "native_conversation_send",
        &lab,
        json!({"session_id": session, "request_id": "r5", "message": "x",
               "attachments": ["data/counts.csv"]}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("text messages"), "{refused}");
    server.shutdown().await;
}

#[tokio::test]
async fn a_message_can_carry_artifacts_environments_runtimes_and_workflows() {
    use wisp_dto::ComposerReferenceArg as Reference;
    let temp = tempfile::tempdir().unwrap();
    let shown: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Recorder(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("data")).unwrap();
    std::fs::write(folder.join("data/counts.csv"), "gene,count\n").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    server
        .save_ssh_host(desktop::SshHostRow {
            alias: "gpu".into(),
            host_name: "10.0.0.8".into(),
            identity_file: "~/.ssh/id_lab".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    server
        .0
        .store
        .save_artifact(
            "art-counts",
            &lab,
            &session,
            "counts.csv",
            "text/csv",
            "data/counts.csv",
        )
        .await
        .unwrap();
    let workflow = server.workflows().await.unwrap().remove(0);
    let request = |id: &str, references: Vec<Reference>| dto::SendRequest {
        session_id: session.clone(),
        request_id: id.into(),
        message: "use these".into(),
        resume: false,
        resume_after: None,
        attachments: vec![],
        references,
        acp_agent_id: None,
    };

    // What cannot be honoured is refused, and nothing is attached for it.
    for bad in [
        Reference::Artifact {
            id: "missing".into(),
        },
        Reference::Context {
            id: "ssh:absent".into(),
        },
        // Interpreters are kept on this computer only.
        Reference::Runtime {
            context_id: "ssh:gpu".into(),
            language: "python".into(),
        },
        Reference::Runtime {
            context_id: "local".into(),
            language: "julia".into(),
        },
        Reference::Workflow { id: "gone".into() },
        Reference::Project { id: lab.clone() },
    ] {
        let refused = server
            .send(
                &lab,
                request(
                    "bad",
                    vec![
                        Reference::Context {
                            id: "ssh:gpu".into(),
                        },
                        bad,
                    ],
                ),
            )
            .await;
        assert!(refused.is_err());
    }
    assert!(!server
        .0
        .store
        .session_execution_context_enabled(&session, "ssh:gpu")
        .await
        .unwrap());

    let references = vec![
        Reference::Artifact {
            id: "art-counts".into(),
        },
        Reference::Context {
            id: "ssh:gpu".into(),
        },
        Reference::Runtime {
            context_id: "local".into(),
            language: "python".into(),
        },
        Reference::Workflow {
            id: workflow.id.clone(),
        },
        Reference::Artifact {
            id: "art-counts".into(),
        },
    ];
    server.send(&lab, request("r1", references)).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let value = call(
                &server,
                "native_conversation_snapshot",
                &lab,
                json!({"session_id": session}),
            )
            .await
            .unwrap();
            if !serde_json::from_value::<dto::Snapshot>(value)
                .unwrap()
                .running
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let turn = shown.lock().unwrap().last().unwrap().join("\n");
    // The artifact by name and where its file is, once.
    assert_eq!(turn.matches("- counts.csv: ").count(), 1, "{turn}");
    let counts = format!("lab{0}data{0}counts.csv", std::path::MAIN_SEPARATOR);
    assert!(turn.contains(&counts), "{turn}");
    // The environment by its id and kind; work may now run there.
    assert!(
        turn.contains("- gpu (context_id: ssh:gpu, kind: ssh)"),
        "{turn}"
    );
    assert!(server
        .0
        .store
        .session_execution_context_enabled(&session, "ssh:gpu")
        .await
        .unwrap());
    assert!(
        turn.contains("- python runtime on Local (context_id: local): call the `python` tool"),
        "{turn}"
    );
    assert!(
        turn.contains(&format!("Template id: {}", workflow.id)),
        "{turn}"
    );
    // None of it is saved with the conversation.
    let saved = server.0.store.load_messages(&session).await.unwrap();
    assert!(saved.iter().all(|message| {
        let text = message.content.as_text();
        !text.contains("context_id:") && !text.contains("selected_workflow_template")
    }));
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_in_plan_mode_is_told_so_and_cannot_change_anything() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), true))
        .await
        .unwrap();
    let session = create(&server).await;
    let instructions = || async {
        server
            .0
            .store
            .load_system_messages(std::slice::from_ref(&session))
            .await
            .unwrap()
            .remove(&session)
            .unwrap()
    };
    let plan = |plan_first| desktop::ConversationOptions {
        plan_first,
        ..Default::default()
    };
    assert_eq!(snapshot(&server, &session).await.plan_mode, Some(false));
    assert!(
        server
            .set_conversation_options(PROJECT, &session, plan(true))
            .await
            .unwrap()
            .plan_first
    );
    assert_eq!(snapshot(&server, &session).await.plan_mode, Some(true));

    // The model tries to write. Plan mode refuses before any question is
    // asked, and nothing is written.
    send(&server, &session, "write the answer").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert!(done.approvals.is_empty() && done.error.is_none());
    assert!(!cfg.workspace.join("answer.txt").exists());
    let refused = done
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some("write"))
        .unwrap();
    assert_eq!(refused.ok, Some(false));
    assert!(refused.text.contains("plan"), "{}", refused.text);
    assert!(instructions().await.contains("<plan_mode>"));

    // Leaving plan mode takes the section out again from the next turn, and
    // the switch goes away with the conversation.
    server
        .set_conversation_options(PROJECT, &session, plan(false))
        .await
        .unwrap();
    send(&server, &session, "carry on").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(!instructions().await.contains("<plan_mode>"));
    server
        .set_conversation_options(PROJECT, &session, plan(true))
        .await
        .unwrap();
    server.delete_conversation(PROJECT, &session).await.unwrap();
    assert_eq!(
        server
            .0
            .store
            .get_setting(&format!("frame_plan_mode:{session}"))
            .await
            .unwrap(),
        None
    );
    server.shutdown().await;
}

#[test]
fn a_file_a_later_turn_wrote_again_is_marked_under_the_earlier_answer() {
    let row = |call: &str, artifact: &str| desktop::ReplyArtifact {
        call_id: call.into(),
        artifact: artifact.into(),
        version: format!("{call}-{artifact}"),
        name: artifact.into(),
        kind: "image/png".into(),
        path: Some(format!("figs/{artifact}")),
        superseded: false,
    };
    let rows = desktop::mark_superseded(vec![
        row("turn-1", "a.png"),
        row("turn-1", "b.png"),
        row("turn-2", "a.png"),
        row("turn-3", "c.png"),
    ]);
    let marked: Vec<_> = rows.iter().map(|row| row.superseded).collect();
    assert_eq!(marked, [true, false, false, false]);
}

#[tokio::test]
async fn files_a_turn_writes_become_artifacts_of_its_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), true))
        .await
        .unwrap();
    let writer = create(&server).await;
    let idle = create(&server).await;
    assert!(server
        .artifacts(PROJECT, None, "", 10)
        .await
        .unwrap()
        .is_empty());

    // The turn writes answer.txt through the write tool.
    server
        .set_conversation_options(
            PROJECT,
            &writer,
            desktop::ConversationOptions {
                full_access: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    send(&server, &writer, "write the answer").await.unwrap();
    wait_for(&server, &writer, |s| !s.running).await;
    assert!(cfg.workspace.join("answer.txt").exists());

    let all = server.artifacts(PROJECT, None, "", 10).await.unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    let artifact = &all[0];
    assert_eq!(artifact.name, "answer.txt");
    assert_eq!(artifact.path.as_deref(), Some("answer.txt"));
    assert_eq!(artifact.session_id, writer);
    assert_eq!(artifact.size_bytes, Some("offline result".len() as u64));
    assert!(!artifact.kind.is_empty());

    // Listed for its own conversation only, and found by name.
    assert_eq!(
        server
            .artifacts(PROJECT, Some(&writer), "", 10)
            .await
            .unwrap(),
        all
    );
    assert!(server
        .artifacts(PROJECT, Some(&idle), "", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        server
            .artifacts(PROJECT, None, " ANSWER ", 10)
            .await
            .unwrap(),
        all
    );
    assert!(server
        .artifacts(PROJECT, None, "absent", 10)
        .await
        .unwrap()
        .is_empty());
    assert!(server
        .artifacts(PROJECT, None, "", 0)
        .await
        .unwrap()
        .is_empty());
    assert!(server.artifacts("missing", None, "", 10).await.is_err());

    // Under which answer: after the turn's write call, the version it made.
    let replies = server.reply_artifacts(PROJECT, &writer).await.unwrap();
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0].call_id, "write-1");
    assert_eq!(replies[0].artifact, artifact.id);
    assert_eq!(replies[0].name, "answer.txt");
    assert_eq!(replies[0].path.as_deref(), Some("answer.txt"));
    assert!(!replies[0].superseded);
    let shown = snapshot(&server, &writer).await;
    assert!(shown
        .items
        .iter()
        .any(|item| item.role == "tool" && item.call_id.as_deref() == Some("write-1")));
    assert!(server
        .reply_artifacts(PROJECT, &idle)
        .await
        .unwrap()
        .is_empty());
    assert!(server.reply_artifacts("missing", &writer).await.is_err());

    // Its versions, and a copy of one saved elsewhere from its snapshot,
    // even after the file in the project changed.
    let versions = server
        .artifact_versions(PROJECT, &artifact.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].number, 1);
    assert_eq!(versions[0].size_bytes, Some("offline result".len() as u64));
    std::fs::write(cfg.workspace.join("answer.txt"), "changed by hand").unwrap();
    let copy = temp.path().join("copy.txt");
    server
        .save_artifact_version(PROJECT, &versions[0].id, &copy)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "offline result");
    // Only through its own project; an unknown artifact has none.
    let other = temp.path().join("other-project");
    std::fs::create_dir_all(&other).unwrap();
    let other = server.register_local_project(&other).await.unwrap();
    assert!(server
        .artifact_versions(&other, &artifact.id)
        .await
        .is_err());
    assert!(server
        .save_artifact_version(&other, &versions[0].id, &temp.path().join("x"))
        .await
        .is_err());
    assert!(server.artifact_versions(PROJECT, "ghost").await.is_err());

    // Moved with its artifacts: what the preview listed goes along, and a
    // preview that no longer matches moves nothing.
    // A file changed by hand since it was written stays where it is.
    let changed = server
        .preview_move_with_artifacts(PROJECT, &writer, &other)
        .await
        .unwrap();
    assert!(changed.artifacts.is_empty(), "{changed:?}");
    assert_eq!(changed.retained[0].reason, "changed");
    std::fs::write(cfg.workspace.join("answer.txt"), "offline result").unwrap();
    let preview = server
        .preview_move_with_artifacts(PROJECT, &writer, &other)
        .await
        .unwrap();
    assert_eq!(preview.artifacts, ["answer.txt"], "{preview:?}");
    assert!(preview.files.contains(&"answer.txt".to_string()));
    assert!(server
        .preview_move_with_artifacts(PROJECT, &writer, PROJECT)
        .await
        .is_err());
    assert!(server
        .move_conversation_with_artifacts(PROJECT, &writer, &other, "stale")
        .await
        .is_err());
    assert_eq!(
        server.artifacts(PROJECT, None, "", 10).await.unwrap().len(),
        1
    );
    let moved = server
        .move_conversation_with_artifacts(PROJECT, &writer, &other, &preview.fingerprint)
        .await
        .unwrap();
    assert!(server
        .artifacts(PROJECT, None, "", 10)
        .await
        .unwrap()
        .is_empty());
    let there = server.artifacts(&other, None, "", 10).await.unwrap();
    assert_eq!(there.len(), 1, "{there:?}");
    assert_eq!(there[0].name, "answer.txt");
    assert_eq!(there[0].session_id, moved);
    // The file went to the same place in the other project's folder.
    assert_eq!(
        std::fs::read_to_string(temp.path().join("other-project/answer.txt")).unwrap(),
        "offline result"
    );

    // A run that does not exist, or is not this project's, is not cancelled.
    assert!(server.cancel_run(PROJECT, "ghost").await.is_err());
    assert!(server.cancel_run("missing", "ghost").await.is_err());

    for command in ["list_artifacts", "cancel_run"] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, PROJECT, json!({})).await.is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_finished_runs_workspace_is_cleaned_only_when_nothing_would_be_lost() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let store = &server.0.store;
    let run = |id: &str, kind: &str, status| {
        let mut run = wisp_store::RunRecord::new(id, PROJECT, "local", "Align", kind);
        run.status = status;
        run
    };
    // Unknown runs and other projects are refused.
    assert!(server.clean_run(PROJECT, "ghost", false).await.is_err());
    assert!(server.clean_run("missing", "ghost", false).await.is_err());

    // A run still going keeps its workspace.
    store
        .create_run(&run("going", "ssh_direct", wisp_store::RunStatus::Running))
        .await
        .unwrap();
    let refused = server.clean_run(PROJECT, "going", false).await.unwrap_err();
    assert!(refused.to_string().contains("still running"), "{refused}");

    // Outputs it declared and never brought back are not lost by default.
    let mut finished = run("finished", "ssh_direct", wisp_store::RunStatus::Succeeded);
    finished.output_specs_json =
        r#"[{"glob":"results/*.csv","kind":"table","residency":"local"}]"#.into();
    store.create_run(&finished).await.unwrap();
    let refused = server
        .clean_run(PROJECT, "finished", false)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("never harvested"), "{refused}");
    // Accepting the loss gets as far as the workspace, which this one lacks.
    let refused = server
        .clean_run(PROJECT, "finished", true)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("no server workspace"),
        "{refused}"
    );

    // A search ends through its own coordinator and keeps no workspace.
    store
        .create_run(&run(
            "search",
            "method_search",
            wisp_store::RunStatus::Succeeded,
        ))
        .await
        .unwrap();
    assert!(server.clean_run(PROJECT, "search", true).await.is_err());

    // One already cleaned is left as it is.
    store
        .create_run(&run("cleaned", "ssh_direct", wisp_store::RunStatus::Failed))
        .await
        .unwrap();
    store.mark_run_cleaned("cleaned").await.unwrap();
    server.clean_run(PROJECT, "cleaned", false).await.unwrap();
    assert!(store
        .get_run("cleaned")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_some());
    // It is the desktop's own.
    assert!(!server.commands().iter().any(|name| name.contains("clean")));

    // Only the workspace of a finished run on an SSH host is looked at.
    let refused = server
        .run_workspace(PROJECT, "going", "", "", 0)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("still running"), "{refused}");
    let refused = server
        .run_workspace(PROJECT, "finished", "", "", 0)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("no server workspace"),
        "{refused}"
    );
    assert!(server
        .run_workspace(PROJECT, "ghost", "", "", 0)
        .await
        .is_err());
    // Something has to be chosen, inside the workspace.
    let none = server
        .download_run_files(PROJECT, "finished", vec![], vec![])
        .await
        .unwrap_err();
    assert!(none.to_string().contains("Choose"), "{none}");
    for outside in ["../secrets", "/etc/passwd", "a//b"] {
        assert!(server
            .download_run_files(PROJECT, "finished", vec![outside.into()], vec![])
            .await
            .is_err());
    }
    assert!(server
        .delete_run_files(PROJECT, "finished", vec![])
        .await
        .is_err());
    let root = server
        .delete_run_files(PROJECT, "finished", vec![String::new()])
        .await
        .unwrap_err();
    assert!(root.to_string().contains("cleaning the run up"), "{root}");
    assert!(server
        .delete_run_files(PROJECT, "finished", vec!["../x".into()])
        .await
        .is_err());

    // A run on a host has only its last lines here, until its workspace
    // is cleaned and its log saved into the project.
    let refused = server.run_log(PROJECT, "cleaned").await.unwrap_err();
    assert!(refused.to_string().contains("on the machine"), "{refused}");
    let saved = config(temp.path()).workspace.join("runs/cleaned");
    std::fs::create_dir_all(&saved).unwrap();
    std::fs::write(saved.join("stdout.log"), "aligned 1000 reads\n").unwrap();
    store
        .mark_run_logs_saved("cleaned", "runs/cleaned")
        .await
        .unwrap();
    let log = server.run_log(PROJECT, "cleaned").await.unwrap();
    assert_eq!(log.stdout, "aligned 1000 reads\n");
    assert!(log.stderr.is_empty() && !log.cut);
    // A saved place that is not the run's own is not read.
    store
        .mark_run_logs_saved("finished", "runs/cleaned")
        .await
        .unwrap();
    assert!(server.run_log(PROJECT, "finished").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_model_with_a_route_of_its_own_takes_it_over_the_network_page() {
    use desktop::{ConversationOptions, ModelFields, ModelInput};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    // Which route each model was built with, by its ID.
    let routes: Arc<std::sync::Mutex<Vec<(String, Option<String>)>>> = Arc::default();
    let heard = routes.clone();
    let factory: ModelFactory = Arc::new(move |provider| {
        heard
            .lock()
            .unwrap()
            .push((provider.model.clone(), provider.proxy.clone()));
        Box::new(FakeModel {
            calls: Arc::default(),
            write: false,
        })
    });
    let server = Server::open_with(cfg, factory, Arc::new(desktop::MemorySecrets::default()))
        .await
        .unwrap();
    let input = |model: &str, proxy: &str| ModelInput {
        id: None,
        name: model.into(),
        fields: ModelFields {
            provider: "openai".into(),
            api_url: "http://127.0.0.1:9/v1".into(),
            model: model.into(),
            max_context: "32000".into(),
            max_tokens: "4000".into(),
            reasoning_effort: String::new(),
            proxy: proxy.into(),
        },
        api_key: Some("made-up-key".into()),
    };
    let refused = server
        .save_model_profile(input("lab-bad", "localhost"))
        .await
        .unwrap_err();
    assert!(refused.to_string().starts_with("Route"), "{refused}");
    server
        .save_model_profile(input("lab-plain", ""))
        .await
        .unwrap();
    let saved = server
        .save_model_profile(input("lab-direct", " none "))
        .await
        .unwrap();
    let direct = saved
        .profiles
        .iter()
        .find(|profile| profile.fields.model == "lab-direct")
        .unwrap()
        .clone();
    assert_eq!(direct.fields.proxy, "none");
    server
        .set_network(wisp_dto::NetworkSettings {
            model_proxy_url: "http://127.0.0.1:7890".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let route_of = |model: &str| {
        routes
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(id, _)| id == model)
            .and_then(|(_, route)| route.clone())
    };
    // The default model follows the Network page.
    let plain = create(&server).await;
    send(&server, &plain, "hello").await.unwrap();
    wait_for(&server, &plain, |s| !s.running).await;
    assert_eq!(
        route_of("lab-plain").as_deref(),
        Some("http://127.0.0.1:7890")
    );
    // The one with its own route keeps it.
    let own = create(&server).await;
    let options = server
        .conversation_status(PROJECT, &own)
        .await
        .unwrap()
        .options;
    server
        .set_conversation_options(
            PROJECT,
            &own,
            ConversationOptions {
                model: direct.id.clone(),
                ..options
            },
        )
        .await
        .unwrap();
    send(&server, &own, "hello").await.unwrap();
    wait_for(&server, &own, |s| !s.running).await;
    assert_eq!(route_of("lab-direct").as_deref(), Some("none"));
    server.shutdown().await;
}

#[tokio::test]
async fn network_routes_reach_the_model_and_mcp_servers_and_mirrors_reach_the_prompt() {
    use axum::{response::IntoResponse, Json, Router};
    use desktop::{McpServerInput, McpTransport};
    use wisp_dto::NetworkSettings;

    // A proxy on the loopback interface that answers MCP itself, whatever
    // host it was asked for.
    let app = Router::new().fallback(|Json(body): Json<Value>| async move {
        let Some(id) = body.get("id").cloned() else {
            return axum::http::StatusCode::ACCEPTED.into_response();
        };
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake", "version": "1"}
            }),
            "tools/list" => json!({"tools": [{
                "name": "echo",
                "description": "Say it back",
                "inputSchema": {"type": "object"}
            }]}),
            _ => json!({}),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    // A model that records the route it was built with and what it is shown.
    let routes: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let factory = |routes: &Arc<std::sync::Mutex<Vec<Option<String>>>>,
                   shown: &Arc<std::sync::Mutex<Vec<String>>>|
     -> ModelFactory {
        let (routes, shown) = (routes.clone(), shown.clone());
        Arc::new(move |cfg| {
            routes.lock().unwrap().push(cfg.proxy.clone());
            Box::new(Everything(shown.clone()))
        })
    };
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory(&routes, &shown))
        .await
        .unwrap();
    assert_eq!(server.network().await.unwrap(), NetworkSettings::default());

    // A wrong field is named and nothing is saved, the good fields included.
    let refused = server
        .set_network(NetworkSettings {
            model_proxy_url: "http://127.0.0.1:7890".into(),
            command_proxy_url: "http://user:secret@proxy.test:1".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(refused.to_string().starts_with("Code proxy"), "{refused}");
    assert_eq!(server.network().await.unwrap(), NetworkSettings::default());

    // An MCP server whose name does not resolve: it answers only when the
    // request goes through the proxy.
    let fake = server
        .save_mcp_server(McpServerInput {
            id: None,
            name: "Fake".into(),
            transport: McpTransport::Http {
                url: "http://mcp.invalid/mcp".into(),
            },
            token: None,
            env: None,
            headers: None,
            proxy: None,
        })
        .await
        .unwrap();
    let saved = server
        .set_network(NetworkSettings {
            model_proxy_url: " http://127.0.0.1:7890 ".into(),
            mcp_proxy_url: proxy.clone(),
            pip_index_url: "https://pypi.mirror.test/simple".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(saved.model_proxy_url, "http://127.0.0.1:7890");
    assert_eq!(server.network().await.unwrap(), saved);
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );

    // The next turn's model is built with the route, and is told where
    // packages come from.
    let last_route = || routes.lock().unwrap().last().cloned().flatten();
    let prompt = || shown.lock().unwrap().join("\n");
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_route().as_deref(), Some("http://127.0.0.1:7890"));
    assert!(prompt().contains("<wisp-package-mirrors>"));
    assert!(prompt().contains("\"https://pypi.mirror.test/simple\""));

    // `none` is passed on as it is; with the mirror gone the same
    // conversation is no longer told about it.
    let direct = server
        .set_network(NetworkSettings {
            model_proxy_url: "none".into(),
            mcp_proxy_url: proxy.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    shown.lock().unwrap().clear();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_route().as_deref(), Some("none"));
    assert!(!prompt().contains("<wisp-package-mirrors>"), "{}", prompt());

    // The routes are kept and apply again when the host next opens.
    server.shutdown().await;
    drop(server);
    let server = Server::open_with_models(config(temp.path()), factory(&routes, &shown))
        .await
        .unwrap();
    assert_eq!(server.network().await.unwrap(), direct);
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );

    // A server's own route wins over the page's: with the page saying
    // direct, only the server that names the proxy is reached.
    server
        .set_network(NetworkSettings {
            mcp_proxy_url: "none".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let change = |route: Option<&str>| McpServerInput {
        id: Some(fake.id.clone()),
        name: "Fake".into(),
        transport: McpTransport::Http {
            url: "http://mcp.invalid/mcp".into(),
        },
        token: None,
        env: None,
        headers: None,
        proxy: route.map(str::to_owned),
    };
    assert!(server
        .save_mcp_server(change(Some("localhost")))
        .await
        .is_err());
    server.save_mcp_server(change(Some(""))).await.unwrap();
    assert!(server.test_mcp_server(&fake.id).await.is_err());
    let own = server
        .save_mcp_server(change(Some(&format!(" {proxy} "))))
        .await
        .unwrap();
    assert_eq!(own.proxy, proxy);
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );
    // Left as it is when the form does not say.
    assert_eq!(
        server.save_mcp_server(change(None)).await.unwrap().proxy,
        proxy
    );
    server.shutdown().await;
}

#[tokio::test]
async fn checking_an_ssh_host_records_what_it_is_and_tells_the_model() {
    use desktop::SshHostRow;
    use wisp_runs::context_probe::{ProbeCommand, ProbeCommandOutput, ProbeRunner};

    // Stands in for OpenSSH: answers the probe's one command with what the
    // host "is", or fails the way a refused login does.
    struct Host {
        answers: Result<Vec<(&'static str, &'static str)>, &'static str>,
        asked: Vec<ProbeCommand>,
    }
    impl ProbeRunner for Host {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            self.asked.push(command.clone());
            Ok(match &self.answers {
                Ok(values) => ProbeCommandOutput {
                    status: 0,
                    stdout: values
                        .iter()
                        .map(|(key, value)| {
                            format!(
                                "__WISP_PROBE_VALUE_BEGIN__:{key}\n{value}\n\
                                 __WISP_PROBE_VALUE_END__:{key}\n"
                            )
                        })
                        .collect::<String>()
                        .into_bytes(),
                    stderr: String::new(),
                },
                Err(why) => ProbeCommandOutput {
                    status: 255,
                    stdout: vec![],
                    stderr: (*why).into(),
                },
            })
        }
    }

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(seen.clone())));
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    server
        .save_ssh_host(SshHostRow {
            alias: "lab-probe".into(),
            host_name: "10.0.0.9".into(),
            user: "lab".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Never checked: there is nothing to show, and no host of that name
    // is checked by mistake.
    assert!(server.environments().await.unwrap().checks.is_empty());
    let mut nobody = Host {
        answers: Ok(vec![]),
        asked: vec![],
    };
    assert!(server
        .check_environment_with("missing", &mut nobody)
        .await
        .is_err());
    assert!(nobody.asked.is_empty());

    // A refused login is recorded, not raised, and says what to look at.
    let mut refusing = Host {
        answers: Err("lab@10.0.0.9: Permission denied (publickey)."),
        asked: vec![],
    };
    let failed = server
        .check_environment_with("lab-probe", &mut refusing)
        .await
        .unwrap();
    assert!(failed.found.is_none());
    let why = failed.error.clone().unwrap();
    assert!(why.contains("key authentication failed"), "{why}");
    // It asked the saved host, as the saved user, without a prompt.
    assert_eq!(refusing.asked.len(), 1);
    assert_eq!(refusing.asked[0].program, "ssh");
    let args = refusing.asked[0].args.join(" ");
    assert!(
        args.contains("BatchMode=yes") && args.contains("lab@10.0.0.9"),
        "{args}"
    );
    assert_eq!(server.environments().await.unwrap().checks, [failed]);

    // The model is told not to use a host whose check failed.
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let prompt = || shown.lock().unwrap().join("\n");
    assert!(prompt().contains("ssh:lab-probe"));
    assert!(prompt().contains("last probe failed"), "{}", prompt());

    // Checked again, it answers: what it is is kept and shown.
    let mut answering = Host {
        answers: Ok(vec![
            ("os", "Linux"),
            ("arch", "x86_64"),
            ("hostname", "lab-node-7"),
            ("cpu_count", "64"),
            ("gpu_summary", "NVIDIA A100 x4"),
            ("scheduler", "/usr/bin/sbatch"),
            ("python_executable", "/opt/conda/bin/python3"),
            ("python_version", "3.11.9"),
        ]),
        asked: vec![],
    };
    let passed = server
        .check_environment_with("lab-probe", &mut answering)
        .await
        .unwrap();
    assert_eq!(passed.error, None);
    let found = passed.found.clone().unwrap();
    assert_eq!(found.hostname.as_deref(), Some("lab-node-7"));
    assert_eq!(found.cpu_count, Some(64));
    assert_eq!(found.scheduler.as_deref(), Some("slurm"));
    assert_eq!(
        found.python_executable.as_deref(),
        Some("/opt/conda/bin/python3")
    );
    assert_eq!(server.environments().await.unwrap().checks, [passed]);

    // Now that it has passed a check, its folders can be looked at.
    struct Listing(std::sync::Mutex<Vec<wisp_runs::RunCommand>>);
    #[async_trait]
    impl wisp_runs::RunCommandRunner for Listing {
        async fn run(
            &self,
            command: wisp_runs::RunCommand,
            _: std::time::Duration,
        ) -> Result<wisp_runs::RunCommandOutput, String> {
            self.0.lock().unwrap().push(command);
            Ok(wisp_runs::RunCommandOutput {
                exit_code: 0,
                stdout: "motd\nWISP_REMOTE_DIR_V1\0/home/lab/data\0\
                         f\x0012\x001794636300\x00reads.fq\0d\x000\x000\x00results\0"
                    .into(),
                stderr: String::new(),
            })
        }
    }
    let lister = Listing(Default::default());
    let listed = server
        .remote_directory_with("lab-probe", "~/data", &lister)
        .await
        .unwrap();
    assert_eq!(listed.path, "/home/lab/data");
    let names: Vec<_> = listed.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["results", "reads.fq"]);
    {
        let asked = lister.0.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].program, "ssh");
        assert!(asked[0].args.join(" ").contains("lab@10.0.0.9"));
        // The folder goes to the host's shell as quoted data on stdin.
        let script = asked[0].stdin.as_deref().unwrap();
        assert!(script.contains("dir=\"$HOME\"/'data'"), "{script}");
    }
    assert!(server
        .remote_directory_with("missing", "", &lister)
        .await
        .is_err());
    assert_eq!(lister.0.lock().unwrap().len(), 1);
    shown.lock().unwrap().clear();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(!prompt().contains("last probe failed"), "{}", prompt());

    // Changing the host's notes does not forget that it was checked.
    server
        .save_ssh_host(SshHostRow {
            alias: "lab-probe".into(),
            host_name: "10.0.0.9".into(),
            user: "lab".into(),
            notes: "four A100".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let after = server.environments().await.unwrap();
    assert_eq!(after.checks.len(), 1);
    assert!(after.checks[0].found.is_some());

    // A saved host whose execution context is missing can still be checked.
    server
        .0
        .store
        .delete_execution_context("ssh:lab-probe")
        .await
        .unwrap();
    assert!(server.environments().await.unwrap().checks.is_empty());
    let again = server
        .check_environment_with("lab-probe", &mut answering)
        .await
        .unwrap();
    assert!(again.found.is_some());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_chooses_where_its_work_runs() {
    use desktop::{ConversationOptions, SshHostRow, RUNS_LOCALLY};

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(seen.clone())));
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    for alias in ["run-a", "run-b"] {
        server
            .save_ssh_host(SshHostRow {
                alias: alias.into(),
                host_name: "10.0.0.8".into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    server.set_default_environment(Some("run-a")).await.unwrap();

    // What the model is told its work goes to, after one more turn.
    let told = |session: String, message: &'static str| {
        let (server, shown) = (server.clone(), shown.clone());
        async move {
            shown.lock().unwrap().clear();
            send(&server, &session, message).await.unwrap();
            wait_for(&server, &session, |s| !s.running).await;
            let prompt = shown.lock().unwrap().join("\n");
            ["run-a", "run-b"]
                .into_iter()
                .filter(|alias| {
                    prompt.contains(&format!("Default analysis environment: `ssh:{alias}`"))
                })
                .collect::<Vec<_>>()
        }
    };
    let runs_on = |session: String| {
        let server = server.clone();
        async move {
            server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options
                .runs_on
        }
    };
    let choose = |session: String, runs_on: &'static str| {
        let server = server.clone();
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    ConversationOptions {
                        runs_on: runs_on.into(),
                        ..options
                    },
                )
                .await
        }
    };

    // With no choice of its own a conversation follows Settings.
    let session = create(&server).await;
    assert_eq!(runs_on(session.clone()).await, "");
    assert_eq!(told(session.clone(), "go").await, ["run-a"]);

    // Its own choice applies from its next turn, and only to it.
    let saved = choose(session.clone(), "run-b").await.unwrap();
    assert_eq!(saved.runs_on, "run-b");
    assert_eq!(told(session.clone(), "again").await, ["run-b"]);
    let other = create(&server).await;
    assert_eq!(told(other.clone(), "go").await, ["run-a"]);

    // This computer is a choice too: no host is named as where work goes.
    choose(session.clone(), RUNS_LOCALLY).await.unwrap();
    assert!(told(session.clone(), "once more").await.is_empty());

    // A host that is not saved is refused and nothing changes.
    assert!(choose(session.clone(), "nowhere").await.is_err());
    assert_eq!(runs_on(session.clone()).await, RUNS_LOCALLY);

    // Changing another option keeps the choice, and a branch carries it.
    let options = server
        .conversation_status(PROJECT, &session)
        .await
        .unwrap()
        .options;
    server
        .set_conversation_options(
            PROJECT,
            &session,
            ConversationOptions {
                plan_first: true,
                ..options
            },
        )
        .await
        .unwrap();
    assert_eq!(runs_on(session.clone()).await, RUNS_LOCALLY);
    let branch = server
        .branch_conversation(PROJECT, &session, None)
        .await
        .unwrap();
    assert_eq!(runs_on(branch).await, RUNS_LOCALLY);
    // So does a copy in another project: hosts belong to the app.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let copy = server
        .transfer_conversation(PROJECT, &session, &lab, true)
        .await
        .unwrap();
    let copied = server.conversation_status(&lab, &copy).await.unwrap();
    assert_eq!(copied.options.runs_on, RUNS_LOCALLY);

    // Back to following Settings; and a host that is removed is no longer
    // any conversation's choice.
    choose(session.clone(), "").await.unwrap();
    assert_eq!(runs_on(session.clone()).await, "");
    choose(other.clone(), "run-b").await.unwrap();
    server.remove_ssh_host("run-b").await.unwrap();
    assert_eq!(runs_on(other.clone()).await, "");
    assert_eq!(told(other, "after").await, ["run-a"]);
    server.shutdown().await;
}

#[tokio::test]
async fn project_settings_name_the_project_and_give_the_model_its_rules() {
    use desktop::{ProjectSettings, PROJECT_NAME_LIMIT, PROJECT_RULES_LIMIT};

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let models = |shown: &Arc<std::sync::Mutex<Vec<String>>>| -> ModelFactory {
        let shown = shown.clone();
        Arc::new(move |_| Box::new(Everything(shown.clone())))
    };
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(&shown))
        .await
        .unwrap();
    let rules_file = temp.path().join("workspace/.wisp/WISP.md");
    let start = server.project_settings(PROJECT).await.unwrap();
    assert_eq!((start.name.as_str(), start.rules.as_str()), ("Server", ""));
    assert!(server.project_settings("missing").await.is_err());

    // What the model is shown after one more turn of `session`.
    let told = |session: String, message: &'static str| {
        let (server, shown) = (server.clone(), shown.clone());
        async move {
            shown.lock().unwrap().clear();
            send(&server, &session, message).await.unwrap();
            wait_for(&server, &session, |s| !s.running).await;
            shown.lock().unwrap().join("\n")
        }
    };
    // A conversation that starts before the project has rules.
    let session = create(&server).await;
    assert!(told(session.clone(), "go")
        .await
        .contains("No user-defined rules."));

    // What cannot be saved changes nothing, the rules file included.
    let with = |change: fn(&mut ProjectSettings)| {
        let mut settings = ProjectSettings {
            rules: "Never saved.".into(),
            ..start.clone()
        };
        change(&mut settings);
        settings
    };
    for refused in [
        with(|s| s.name = "  ".into()),
        with(|s| s.name = "two\nlines".into()),
        with(|s| s.name = "n".repeat(PROJECT_NAME_LIMIT + 1)),
        with(|s| s.rules = "r".repeat(PROJECT_RULES_LIMIT + 1)),
        with(|s| s.retention.succeeded = Some(0)),
        with(|s| s.retention.orphaned = Some(desktop::RETENTION_DAYS_LIMIT + 1)),
    ] {
        assert!(server.set_project_settings(PROJECT, refused).await.is_err());
    }
    assert_eq!(server.project_settings(PROJECT).await.unwrap(), start);
    assert!(!rules_file.exists());

    // Saved, they are trimmed; the rules are a file in the project.
    let saved = server
        .set_project_settings(
            PROJECT,
            ProjectSettings {
                name: " Screens 2026 ".into(),
                description: " CRISPR screens ".into(),
                rules: "Always report effect sizes.\n".into(),
                retention: desktop::RunRetention {
                    succeeded: Some(7),
                    failed: None,
                    orphaned: Some(30),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.name, "Screens 2026");
    // The cleanup windows are the run manager's, kept with the project.
    assert_eq!(
        server.0.store.project_run_retention(PROJECT).await.unwrap(),
        (Some(7), None, Some(30))
    );
    assert_eq!(saved.retention.orphaned, Some(30));
    assert_eq!(saved.description, "CRISPR screens");
    assert_eq!(
        std::fs::read_to_string(&rules_file).unwrap(),
        "Always report effect sizes."
    );

    // The conversation that was already open is told from its next turn.
    let prompt = told(session.clone(), "again").await;
    assert!(prompt.contains("Always report effect sizes."), "{prompt}");
    assert!(!prompt.contains("No user-defined rules."));
    // The file is the truth: an edit made outside the app is read as well.
    std::fs::write(&rules_file, "Use SI units.").unwrap();
    assert_eq!(
        server.project_settings(PROJECT).await.unwrap().rules,
        "Use SI units."
    );
    let prompt = told(session.clone(), "third").await;
    assert!(prompt.contains("Use SI units.") && !prompt.contains("effect sizes"));

    // Emptying the rules removes the file, and the model is told there are none.
    server
        .set_project_settings(
            PROJECT,
            ProjectSettings {
                rules: "  ".into(),
                ..saved.clone()
            },
        )
        .await
        .unwrap();
    assert!(!rules_file.exists());
    assert!(told(session.clone(), "fourth")
        .await
        .contains("No user-defined rules."));

    // The name is the user's: opening the host again does not put it back.
    server.shutdown().await;
    drop(server);
    let server = Server::open_with_models(config(temp.path()), models(&shown))
        .await
        .unwrap();
    let after = server.project_settings(PROJECT).await.unwrap();
    assert_eq!(
        (after.name.as_str(), after.description.as_str()),
        ("Screens 2026", "CRISPR screens")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn the_desktop_makes_renames_and_trashes_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let names = |path: &'static str| {
        let server = server.clone();
        async move {
            let mut names: Vec<String> = server
                .project_directory(PROJECT, path)
                .await
                .unwrap()
                .entries
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            names.sort();
            names
        }
    };
    assert_eq!(
        server
            .create_project_entry(PROJECT, "", "figures", true)
            .await
            .unwrap(),
        "figures"
    );
    let note = server
        .create_project_entry(PROJECT, "figures", "notes.md", false)
        .await
        .unwrap();
    assert_eq!(note, "figures/notes.md");
    assert!(server
        .create_project_entry("missing", "", "x", true)
        .await
        .is_err());
    let plan = server
        .rename_project_entry(PROJECT, &note, "plan.md")
        .await
        .unwrap();
    assert_eq!(names("figures").await, ["plan.md"]);

    // Trashed, it is gone from the listing and kept on disk; the trash is
    // part of the project's own records, which the browser does not show.
    let gone = server.trash_project_entry(PROJECT, &plan).await.unwrap();
    assert!(gone.starts_with(".wisp/trash/") && gone.ends_with("/plan.md"));
    assert!(names("figures").await.is_empty());
    assert!(temp.path().join("workspace").join(&gone).is_file());
    assert_eq!(names("").await, ["figures"]);

    // A picture is handed over whole, with what kind it is; other files,
    // and anything outside the project, are not.
    let workspace = temp.path().join("workspace");
    std::fs::write(workspace.join("figures/plot.PNG"), b"\x89PNG fake").unwrap();
    std::fs::write(workspace.join("figures/notes.txt"), "words").unwrap();
    let (kind, bytes) = server
        .project_image(PROJECT, "figures/plot.PNG")
        .await
        .unwrap();
    assert_eq!(
        (kind.as_str(), bytes.as_slice()),
        ("image/png", &b"\x89PNG fake"[..])
    );
    for path in [
        "figures/notes.txt",
        "figures",
        "figures/missing.png",
        "../x.png",
    ] {
        assert!(server.project_image(PROJECT, path).await.is_err(), "{path}");
    }
    // A PDF is handed over whole for its pages to be drawn; nothing else is.
    std::fs::write(workspace.join("figures/paper.PDF"), b"%PDF-1.4 fake").unwrap();
    assert_eq!(
        server
            .project_pdf(PROJECT, "figures/paper.PDF")
            .await
            .unwrap(),
        b"%PDF-1.4 fake"
    );
    for path in ["figures/notes.txt", "figures/plot.PNG", "../x.pdf"] {
        assert!(server.project_pdf(PROJECT, path).await.is_err(), "{path}");
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_projects_journey_lists_what_happened_and_what_the_researcher_recorded() {
    use wisp_dto::ResearchJournalInput;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (from, until) = (now - 86_400, now + 86_400);
    assert!(server
        .journey(PROJECT, from, until)
        .await
        .unwrap()
        .entries
        .is_empty());

    // What was asked in a conversation is part of the history.
    let session = create(&server).await;
    send(&server, &session, "align the reads").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    // So is what the researcher writes down, dated when it happened.
    let record = |title: &str, category: &str, occurred_at: i64| ResearchJournalInput {
        title: title.into(),
        body: "Mapping rate 98%.".into(),
        category: category.into(),
        occurred_at,
    };
    let id = server
        .add_journal_entry(PROJECT, record(" Reads are clean ", "finding", now - 3_600))
        .await
        .unwrap();
    for refused in [
        record("", "finding", now),
        record("Title", "gossip", now),
        record("Title", "finding", -5),
    ] {
        assert!(server.add_journal_entry(PROJECT, refused).await.is_err());
    }
    assert!(server
        .add_journal_entry("missing", record("Title", "finding", now))
        .await
        .is_err());

    let journey = server.journey(PROJECT, from, until).await.unwrap();
    let kinds: Vec<_> = journey
        .entries
        .iter()
        .map(|entry| (entry.kind.as_str(), entry.title.as_str(), entry.manual))
        .collect();
    // Newest first: the question just asked, then the record dated an hour ago.
    assert_eq!(
        kinds,
        [
            ("session", "align the reads", false),
            ("finding", "Reads are clean", true)
        ]
    );
    assert_eq!(journey.entries[1].source_id, id);
    assert_eq!(journey.entries[1].summary, "Mapping rate 98%.");
    assert_eq!(
        journey.entries[0].frame_id.as_deref(),
        Some(session.as_str())
    );

    // Only what falls in the range, and never more than a month at a time.
    let earlier = server
        .journey(PROJECT, now - 10 * 86_400, now - 86_400)
        .await
        .unwrap();
    assert!(earlier.entries.is_empty());
    assert!(server
        .journey(PROJECT, now - 40 * 86_400, now)
        .await
        .is_err());
    assert!(server.journey("missing", from, until).await.is_err());

    // The calendar is the same history for every project at once.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    server
        .add_journal_entry(&lab, record("Ordered primers", "progress", now))
        .await
        .unwrap();
    let calendar = server.calendar(from, until).await.unwrap();
    let mut seen: Vec<_> = calendar
        .iter()
        .map(|project| {
            assert_eq!(project.error, None);
            (project.project_id.as_str(), project.history.entries.len())
        })
        .collect();
    seen.sort();
    let mut expected = [(PROJECT, 2), (lab.as_str(), 1)];
    expected.sort();
    assert_eq!(seen, expected);
    assert!(server.calendar(now - 40 * 86_400, now).await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_is_read_back_as_a_trajectory() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // A model that runs one tool, then answers.
    let server = Server::open_with_models(config(temp.path()), models(calls, true))
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing asked yet: nothing to show, and nothing made up.
    let empty = server.trajectory(PROJECT, &session).await.unwrap();
    assert!(empty.turns.is_empty());
    assert_eq!(empty.stats, wisp_dto::TrajectoryStatsDto::default());

    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;

    let trajectory = server.trajectory(PROJECT, &session).await.unwrap();
    assert_eq!(trajectory.frame_id, session);
    assert_eq!(trajectory.turns.len(), 1);
    let turn = &trajectory.turns[0];
    assert_eq!(turn.index, 1);
    assert!(turn.started_at.is_some());
    let cells = |kind: &str| -> Vec<_> { turn.cells.iter().filter(|c| c.kind == kind).collect() };
    assert_eq!(turn.cells[0].kind, "user");
    assert_eq!(turn.cells[0].summary, "write the file");
    // The tool is one step: what it was given, what it answered, how it went.
    let tools = cells("tool");
    assert_eq!(tools.len(), 1, "{turn:?}");
    assert!(tools[0].summary.starts_with("write {"), "{turn:?}");
    assert!(tools[0]
        .detail_input
        .as_deref()
        .unwrap()
        .contains("answer.txt"));
    assert!(tools[0].detail_output.is_some());
    assert_eq!((tools[0].ok, tools[0].is_error), (Some(true), false));
    assert_eq!(cells("assistant").len(), 1, "{turn:?}");
    // Two model calls: the one that asked for the tool and the one that answered.
    assert_eq!(cells("usage").len(), 2, "{turn:?}");
    assert_eq!((trajectory.stats.turns, trajectory.stats.steps), (1, 2));
    // The turn's calls added up, for the line under its answer.
    let tokens = server.turn_tokens(PROJECT, &session).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        (tokens[0].from_end, tokens[0].text.as_str(), tokens[0].calls),
        (0, "write the file", 2)
    );
    assert_eq!(
        tokens[0].input + tokens[0].output,
        (trajectory.stats.input_tokens + trajectory.stats.output_tokens) as u64
    );

    // Only the conversation's own project may read it.
    assert!(server.trajectory("missing", &session).await.is_err());
    assert!(server.turn_tokens("missing", &session).await.is_err());
    assert!(server.trajectory(PROJECT, "missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_turn_that_asks_to_remember_has_a_memory_drafted_to_confirm() {
    // A model that answers, and drafts a memory when asked on the side.
    struct Drafting {
        drafts: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Provider for Drafting {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let drafting = messages[0]
                .content
                .as_text()
                .starts_with("You prepare a memory draft");
            if drafting {
                self.drafts.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Completion {
                content: if drafting {
                    r#"{"scope":"global","content":"Plot on a log scale."}"#
                } else {
                    "[]"
                }
                .into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("Noted.");
            Ok(Completion {
                content: "Noted.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let drafts = Arc::new(AtomicUsize::new(0));
    let factory: ModelFactory = {
        let drafts = drafts.clone();
        Arc::new(move |_| {
            Box::new(Drafting {
                drafts: drafts.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let session = create(&server).await;
    let after = |message: &'static str| {
        let (server, session) = (&server, session.clone());
        async move {
            send(server, &session, message).await.unwrap();
            wait_for(server, &session, |s| !s.running).await;
            server.after_turn_settled(PROJECT, &session).await.unwrap();
            server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .memory_proposal
        }
    };
    // An ordinary turn has nothing drafted.
    assert_eq!(after("count the reads").await, None);
    assert_eq!(drafts.load(Ordering::Relaxed), 0);
    // Asked to remember: a draft to confirm, with its scope.
    let proposal = after("Remember: always plot on a log scale").await.unwrap();
    assert_eq!(proposal.trigger, desktop::ProposalTrigger::Explicit);
    assert_eq!(proposal.scope, "global");
    assert_eq!(proposal.content, "Plot on a log scale.");
    assert_eq!(drafts.load(Ordering::Relaxed), 1);
    // Put away, it is gone; only through the conversation's own project.
    assert!(server
        .dismiss_memory_proposal("missing", &session)
        .await
        .is_err());
    server
        .dismiss_memory_proposal(PROJECT, &session)
        .await
        .unwrap();
    assert_eq!(
        server
            .conversation_status(PROJECT, &session)
            .await
            .unwrap()
            .memory_proposal,
        None
    );

    // The thresholds for tool failures, under the WebView's key; off
    // until turned on.
    let start = server.failure_analysis().await.unwrap();
    assert!(!start.enabled);
    let saved = server
        .set_failure_analysis(desktop::AutoFailureAnalysisSettings {
            enabled: true,
            failure_rate_threshold: 0,
            minimum_failures: 3,
        })
        .await
        .unwrap();
    assert_eq!(
        (
            saved.enabled,
            saved.failure_rate_threshold,
            saved.minimum_failures
        ),
        (true, 1, 3)
    );
    assert_eq!(server.failure_analysis().await.unwrap(), saved);
    assert!(server
        .0
        .store
        .get_setting("auto_failure_analysis")
        .await
        .unwrap()
        .is_some());
    server.shutdown().await;
}

#[tokio::test]
async fn an_answer_is_followed_by_questions_to_ask_next() {
    // A model that answers a turn, and suggests when asked on the side.
    struct Advising {
        suggested: Arc<AtomicUsize>,
        hold: Arc<AtomicBool>,
    }
    #[async_trait]
    impl Provider for Advising {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            self.suggested.fetch_add(1, Ordering::Relaxed);
            Ok(Completion {
                content: r#"["Why?", "How many?", "What next?"]"#.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            while self.hold.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            sink.on_text("The answer.");
            Ok(Completion {
                content: "The answer.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let suggested = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let factory: ModelFactory = {
        let (suggested, hold) = (suggested.clone(), hold.clone());
        Arc::new(move |_| {
            Box::new(Advising {
                suggested: suggested.clone(),
                hold: hold.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // On until it is turned off.
    let defaults = server.turn_defaults().await.unwrap();
    assert!(defaults.follow_ups);

    let session = create(&server).await;
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());
    // Nothing to wait for before anything was asked.
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert!(server
        .after_turn_settled("missing", &session)
        .await
        .is_err());
    send(&server, &session, "one").await.unwrap();
    // Whoever sees the turn over can wait for the questions.
    wait_for(&server, &session, |s| !s.running).await;
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    let after = snapshot(&server, &session).await;
    assert!(!after.running);
    assert_eq!(after.follow_ups, ["Why?", "How many?", "What next?"]);
    assert_eq!(suggested.load(Ordering::Relaxed), 1);

    // They are about that answer: the next turn starts without them.
    hold.store(true, Ordering::Relaxed);
    send(&server, &session, "two").await.unwrap();
    let during = wait_for(&server, &session, |s| s.running).await;
    assert!(during.follow_ups.is_empty());
    hold.store(false, Ordering::Relaxed);
    wait_for(&server, &session, |s| !s.follow_ups.is_empty()).await;

    // Going back removes the answer they were about, and them with it.
    let latest = desktop::TurnRef {
        from_end: 0,
        text: "two".into(),
    };
    server
        .rewind_conversation(PROJECT, &session, &latest)
        .await
        .unwrap();
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());

    // Put away, they stay away; only the conversation's project may.
    assert!(server
        .dismiss_follow_ups("missing", &session)
        .await
        .is_err());
    server.dismiss_follow_ups(PROJECT, &session).await.unwrap();
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());

    // Turned off, the model is not asked at all.
    let off = server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    assert!(!off.follow_ups);
    // The key and value are the WebView desktop's.
    assert_eq!(
        server
            .0
            .store
            .get_setting("follow_up_questions")
            .await
            .unwrap()
            .as_deref(),
        Some("false")
    );
    let before = suggested.load(Ordering::Relaxed);
    send(&server, &session, "three").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());
    assert_eq!(suggested.load(Ordering::Relaxed), before);
    server.shutdown().await;
}

#[tokio::test]
async fn a_question_asked_aside_is_answered_without_touching_the_conversation() {
    // A model that answers a turn, and records what it is asked aside.
    struct Recording {
        asked: Arc<Mutex<Vec<Vec<Message>>>>,
    }
    #[async_trait]
    impl Provider for Recording {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(tools.is_empty(), "a side question has no tools");
            self.asked.lock().unwrap().push(messages.to_vec());
            Ok(Completion {
                content: " Because the file has 400 lines. ".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("There are 100 reads.");
            Ok(Completion {
                content: "There are 100 reads.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let factory: ModelFactory = {
        let asked = asked.clone();
        Arc::new(move |_| {
            Box::new(Recording {
                asked: asked.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // Asking aside is all this test's model is asked on the side.
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "count the reads").await.unwrap();
    let before = wait_for(&server, &session, |s| !s.running).await;

    let earlier = [desktop::AsideExchange {
        question: "which file?".into(),
        answer: "reads.fq".into(),
    }];
    let answer = server
        .ask_aside(PROJECT, &session, &earlier, "  why 100?  ")
        .await
        .unwrap();
    assert_eq!(answer.text, "Because the file has 400 lines.");
    // The one turn is quoted whole, so nothing earlier goes along.
    assert!(answer.sources.is_empty());
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        let sent: Vec<_> = asked[0]
            .iter()
            .map(|message| message.content.as_text())
            .collect();
        // The conversation as quoted data, the earlier exchange, the question.
        assert!(sent[1].contains("[USER]\ncount the reads"), "{sent:?}");
        assert!(
            sent[1].contains("[ASSISTANT]\nThere are 100 reads."),
            "{sent:?}"
        );
        assert_eq!(sent[3..], ["which file?", "reads.fq", "why 100?"]);
    }
    // Nothing was added to the conversation.
    let after = snapshot(&server, &session).await;
    assert_eq!(after.items.len(), before.items.len());
    assert!(!after.running);

    // A question is needed, of a sensible length, in the conversation's own project.
    for (project, question) in [
        (PROJECT, "   ".to_owned()),
        (PROJECT, "x".repeat(4_001)),
        ("missing", "why?".to_owned()),
    ] {
        assert!(server
            .ask_aside(project, &session, &[], &question)
            .await
            .is_err());
    }
    assert!(server
        .ask_aside(PROJECT, "missing", &[], "why?")
        .await
        .is_err());
    assert_eq!(asked.lock().unwrap().len(), 1);

    // About one of the project's runs: its live snapshot goes along.
    let mut run =
        wisp_store::RunRecord::new("run-aside", PROJECT, "local", "Align reads", "command");
    run.status = wisp_store::RunStatus::Failed;
    run.stderr_tail = Some("index not found".into());
    server.0.store.create_run(&run).await.unwrap();
    server
        .ask_aside_about_run(PROJECT, &session, "run-aside", &[], "why did it fail?")
        .await
        .unwrap();
    {
        let asked = asked.lock().unwrap();
        let sent = asked.last().unwrap()[1].content.as_text();
        assert!(
            sent.contains("<run>\nRun run-aside \"Align reads\""),
            "{sent}"
        );
        assert!(sent.contains("stderr tail:\nindex not found"), "{sent}");
    }
    assert!(server
        .ask_aside_about_run(PROJECT, &session, "ghost", &[], "why?")
        .await
        .is_err());
    // It is the desktop's own: a paired device cannot ask it.
    assert!(!server.commands().iter().any(|name| name.contains("aside")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_project_and_its_folder_are_deleted_only_when_its_name_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("results")).unwrap();
    std::fs::write(folder.join("results/counts.csv"), "gene,count\n").unwrap();
    let neighbour = temp.path().join("neighbour");
    std::fs::create_dir(&neighbour).unwrap();
    std::fs::write(neighbour.join("keep.txt"), "mine").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let kept = server.register_local_project(&neighbour).await.unwrap();
    let session = server.create_conversation(&lab).await.unwrap();
    let listed = |id: String| {
        let server = &server;
        async move {
            let value = call(server, "list_projects", "", json!({})).await.unwrap();
            serde_json::from_value::<Vec<wisp_dto::ProjectSummary>>(value)
                .unwrap()
                .iter()
                .any(|project| project.id == id)
        }
    };

    // The question says what would go.
    let asked = server.project_folder(&lab).await.unwrap();
    assert_eq!(asked.name, "lab");
    assert!(asked.path.ends_with("lab"), "{asked:?}");
    assert_eq!(
        asked.contents,
        desktop::FolderContents::Files {
            files: 1,
            bytes: 11,
            more: false
        }
    );

    // Without the name, with another project's name, or for a project
    // that cannot leave, nothing is touched.
    for (project, typed) in [
        (lab.as_str(), ""),
        (lab.as_str(), "neighbour"),
        (lab.as_str(), "LAB"),
        (PROJECT, "Server"),
        ("missing", "lab"),
    ] {
        assert!(server
            .delete_project_and_folder(project, typed)
            .await
            .is_err());
    }
    assert!(server.project_folder(PROJECT).await.is_err());
    assert!(folder.join("results/counts.csv").exists());
    assert!(listed(lab.clone()).await);

    server
        .delete_project_and_folder(&lab, " lab ")
        .await
        .unwrap();
    assert!(!folder.exists());
    assert!(!listed(lab.clone()).await);
    // Its conversation went with it; the neighbour and its files did not.
    assert!(server.conversation_status(&lab, &session).await.is_err());
    assert!(listed(kept.clone()).await);
    assert_eq!(
        std::fs::read_to_string(neighbour.join("keep.txt")).unwrap(),
        "mine"
    );

    // A folder that is already gone leaves only records to remove.
    std::fs::remove_dir_all(&neighbour).unwrap();
    let asked = server.project_folder(&kept).await.unwrap();
    assert_eq!(asked.contents, desktop::FolderContents::Missing);
    server
        .delete_project_and_folder(&kept, "neighbour")
        .await
        .unwrap();
    assert!(!listed(kept).await);
    // It is the desktop's own: a paired device cannot ask for it.
    assert!(!server
        .commands()
        .iter()
        .any(|name| name.contains("delete_project")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_review_reports_what_the_tool_output_does_not_support() {
    // A model that runs a tool and answers, and reviews when asked to.
    struct Reviewing {
        read: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl Provider for Reviewing {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "reviewer-1"
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(tools.is_empty(), "a reviewer has no tools");
            assert!(messages[0]
                .content
                .as_text()
                .starts_with("You are a REVIEWER"));
            self.read
                .lock()
                .unwrap()
                .push(messages[1].content.as_text());
            Ok(Completion {
                content: r#"{"summary":"Checked the write.","findings":[{"message_index":3,
                    "claim":"Saved result","evidence":"the tool wrote answer.txt",
                    "fix":"Name the file","verdict":"warn","severity":"low"}]}"#
                    .into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
            {
                return Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: wisp_llm::FunctionCall {
                            name: "write".into(),
                            arguments: json!({"path":"answer.txt", "content":"offline result"})
                                .to_string(),
                        },
                    }],
                    finish_reason: Some("tool_calls".into()),
                    ..Default::default()
                });
            }
            sink.on_text("Saved result");
            Ok(Completion {
                content: "Saved result".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let read = Arc::new(Mutex::new(Vec::new()));
    let factory: ModelFactory = {
        let read = read.clone();
        Arc::new(move |_| Box::new(Reviewing { read: read.clone() }))
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    let session = create(&server).await;
    // Before anything was said there is nothing to review, and the model
    // is not asked.
    let empty = server
        .review_conversation(PROJECT, &session)
        .await
        .unwrap_err()
        .to_string();
    assert!(empty.contains("nothing to review"), "{empty}");
    assert!(read.lock().unwrap().is_empty());

    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    // Half a turn cannot be judged.
    if asked.running {
        let early = server
            .review_conversation(PROJECT, &session)
            .await
            .unwrap_err()
            .to_string();
        assert!(early.contains("Wait for the turn"), "{early}");
    }
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    let before = wait_for(&server, &session, |s| !s.running).await;

    let review = server.review_conversation(PROJECT, &session).await.unwrap();
    assert_eq!(review.summary, "Checked the write.");
    assert_eq!(review.model, "reviewer-1");
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].claim, "Saved result");
    assert_eq!(review.findings[0].verdict, "warn");
    assert!(review.correction().contains("1. Claim: Saved result"));
    // The reviewer read the question, what the tool was given and what it
    // answered, and the reply.
    let transcript = read.lock().unwrap()[0].clone();
    assert!(
        transcript.starts_with("[msg:0 USER]\nwrite the file"),
        "{transcript}"
    );
    assert!(transcript.contains("TOOL:write]\nargs:\n"), "{transcript}");
    assert!(transcript.contains("answer.txt"), "{transcript}");
    assert!(
        transcript.contains("ASSISTANT]\nSaved result"),
        "{transcript}"
    );
    // No message was added; the report is kept with the conversation, as
    // a row after what it read, in the WebView's shape.
    let after = snapshot(&server, &session).await;
    assert_eq!(after.items.len(), before.items.len() + 1);
    let kept = after.items.last().unwrap();
    assert_eq!(kept.role, "review");
    let report: serde_json::Value = serde_json::from_str(&kept.text).unwrap();
    assert_eq!(report["summary"], "Checked the write.");
    assert_eq!(report["review_status"], "failed");
    assert_eq!(report["findings"][0]["claim"], "Saved result");
    assert_eq!(
        crate::review::Review::from_report_json(&kept.text).unwrap(),
        review
    );

    // One turn reviewed by itself: the reviewer reads up to its end and
    // nothing that came after.
    send(&server, &session, "and thanks").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let first = desktop::TurnRef {
        from_end: 1,
        text: "write the file".into(),
    };
    server.review_turn(PROJECT, &session, &first).await.unwrap();
    let earlier = read.lock().unwrap().last().unwrap().clone();
    assert!(earlier.contains("write the file"), "{earlier}");
    assert!(!earlier.contains("and thanks"), "{earlier}");
    // That review is kept after the first turn, before the next question.
    let rows = snapshot(&server, &session).await.items;
    let next = rows
        .iter()
        .position(|row| row.role == "user" && row.text == "and thanks")
        .unwrap();
    assert_eq!(rows[next - 1].role, "review");
    assert_eq!(rows.iter().filter(|row| row.role == "review").count(), 2);
    server.review_conversation(PROJECT, &session).await.unwrap();
    assert!(read.lock().unwrap().last().unwrap().contains("and thanks"));
    // A turn that is not there is refused, and nobody is asked.
    let asked = read.lock().unwrap().len();
    let wrong = desktop::TurnRef {
        from_end: 1,
        text: "something else".into(),
    };
    assert!(server.review_turn(PROJECT, &session, &wrong).await.is_err());
    assert!(server
        .review_turn("missing", &session, &first)
        .await
        .is_err());
    assert_eq!(read.lock().unwrap().len(), asked);

    assert!(server
        .review_conversation("missing", &session)
        .await
        .is_err());
    assert!(server
        .review_conversation(PROJECT, "missing")
        .await
        .is_err());
    assert!(!server.commands().iter().any(|name| name.contains("review")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversations_interpreters_are_listed_and_can_be_stopped() {
    // An interpreter that starts at once and does nothing.
    struct Idle;
    #[async_trait]
    impl wisp_runtime::RuntimeKernel for Idle {
        async fn execute(
            &mut self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &[String],
            _: &wisp_runtime::RuntimeOutput,
        ) -> anyhow::Result<wisp_runtime::KernelResp> {
            Ok(Default::default())
        }
        async fn inspect(&mut self, _: &str) -> anyhow::Result<wisp_runtime::RuntimeObjectList> {
            Ok(Default::default())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    struct Launcher;
    #[async_trait]
    impl wisp_runtime::RuntimeLauncher for Launcher {
        async fn launch(
            &self,
            key: &wisp_runtime::RuntimeKey,
            _: &Path,
        ) -> anyhow::Result<wisp_runtime::LaunchedRuntime> {
            let python = key.language == wisp_runtime::RuntimeLanguage::Python;
            Ok(wisp_runtime::LaunchedRuntime::new(
                Box::new(Idle),
                wisp_runtime::RuntimeMetadata {
                    interpreter: Some(
                        if python {
                            "/opt/py/bin/python3"
                        } else {
                            "/opt/r/bin/Rscript"
                        }
                        .into(),
                    ),
                    version: Some(if python { "3.12.4" } else { "4.4.1" }.into()),
                    process_id: python.then_some(4242),
                },
            ))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    let runtimes = |id: String| {
        let server = &server;
        async move {
            server
                .conversation_status(PROJECT, &id)
                .await
                .unwrap()
                .runtimes
        }
    };
    // A conversation that ran no code keeps no interpreter.
    assert!(runtimes(session.clone()).await.is_empty());
    assert!(server
        .stop_runtime(PROJECT, &session, "none")
        .await
        .is_err());

    let manager = wisp_runtime::RuntimeManager::new(Arc::new(Launcher));
    *server.session(&session).runtime.lock().unwrap() = Some(manager.clone());
    for key in [
        wisp_runtime::RuntimeKey::local_python(PROJECT),
        wisp_runtime::RuntimeKey::r(PROJECT, wisp_runtime::LOCAL_CONTEXT_ID),
    ] {
        manager
            .start(key.with_session(session.clone()), temp.path().to_path_buf())
            .await
            .unwrap();
    }
    let listed = runtimes(session.clone()).await;
    let seen: Vec<_> = listed
        .iter()
        .map(|row| (row.language, row.state, row.version.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                desktop::RuntimeLanguage::Python,
                desktop::RuntimeState::Ready,
                Some("3.12.4")
            ),
            (
                desktop::RuntimeLanguage::R,
                desktop::RuntimeState::Ready,
                Some("4.4.1")
            ),
        ]
    );
    assert_eq!(
        listed[0].interpreter.as_deref(),
        Some("/opt/py/bin/python3")
    );
    // Its process, which one of the conversation's interpreters it is,
    // and when it started and last ran code.
    assert_eq!(listed[0].process_id, Some(4242));
    assert_eq!(listed[1].process_id, None);
    assert_eq!(listed[0].generation, 1);
    assert!(listed[0].started_at_ms > 0);
    assert!(listed[0].last_activity_at_ms >= listed[0].started_at_ms);

    // Stopped, it stays listed as stopped; the other is untouched.
    let python = listed[0].id.clone();
    server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .unwrap();
    let listed = runtimes(session.clone()).await;
    assert_eq!(listed[0].state, desktop::RuntimeState::Stopped);
    assert_eq!(listed[1].state, desktop::RuntimeState::Ready);
    // Asked again, the stopped one is taken away.
    server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .unwrap();
    let listed = runtimes(session.clone()).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].language, desktop::RuntimeLanguage::R);
    assert!(server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .is_err());
    // Only through the conversation's own project.
    assert!(server
        .stop_runtime("missing", &session, &listed[0].id)
        .await
        .is_err());
    assert_eq!(runtimes(session.clone()).await.len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn the_model_view_is_what_the_next_turn_would_be_sent() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing has been sent, so nothing is saved to send again.
    let empty = server.model_view(PROJECT, &session).await.unwrap();
    assert!(empty.rows.is_empty() && !empty.compacted);

    send(&server, &session, "count the reads").await.unwrap();
    let shown = wait_for(&server, &session, |s| !s.running).await;
    let view = server.model_view(PROJECT, &session).await.unwrap();
    let kinds: Vec<_> = view.rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        [
            desktop::ModelRowKind::System,
            desktop::ModelRowKind::User,
            desktop::ModelRowKind::Assistant,
        ]
    );
    // The instructions are in it though the transcript never shows them.
    assert!(view.rows[0].chars > 200, "{:?}", view.rows[0].chars);
    assert!(shown.items.iter().all(|item| item.role != "system"));
    assert_eq!(view.rows[1].text, "count the reads");
    assert_eq!(view.rows[2].text, "Saved result");
    assert_eq!(
        view.chars,
        view.rows.iter().map(|row| row.chars).sum::<usize>()
    );

    assert!(server.model_view("missing", &session).await.is_err());
    assert!(server.model_view(PROJECT, "missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_that_asks_for_it_is_reviewed_after_each_turn_worth_it() {
    // A model that runs one tool per conversation and answers briefly, and
    // reports one finding whenever it is asked to review.
    struct Checked {
        reviews: Arc<AtomicUsize>,
        corrected: Arc<AtomicBool>,
    }
    #[async_trait]
    impl Provider for Checked {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let content = if messages[0]
                .content
                .as_text()
                .starts_with("You are a REVIEWER")
            {
                self.reviews.fetch_add(1, Ordering::Relaxed);
                r#"{"summary":"Checked.","findings":[{"claim":"Saved result",
                    "evidence":"the file is answer.txt","fix":"Name it","verdict":"fail"}]}"#
            } else {
                "[]"
            };
            Ok(Completion {
                content: content.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if messages.iter().any(|message| {
                message
                    .content
                    .as_text()
                    .starts_with("An independent reviewer found")
            }) {
                self.corrected.store(true, Ordering::Relaxed);
            }
            if !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
            {
                return Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: wisp_llm::FunctionCall {
                            name: "write".into(),
                            arguments: json!({"path":"answer.txt", "content":"offline result"})
                                .to_string(),
                        },
                    }],
                    finish_reason: Some("tool_calls".into()),
                    ..Default::default()
                });
            }
            sink.on_text("Saved result");
            Ok(Completion {
                content: "Saved result".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let reviews = Arc::new(AtomicUsize::new(0));
    let corrected = Arc::new(AtomicBool::new(false));
    let factory: ModelFactory = {
        let (reviews, corrected) = (reviews.clone(), corrected.clone());
        Arc::new(move |_| {
            Box::new(Checked {
                reviews: reviews.clone(),
                corrected: corrected.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // One turn with a tool, approved, and everything after it settled.
    let turn = |session: String, message: &'static str| {
        let server = &server;
        async move {
            send(server, &session, message).await.unwrap();
            let asked = wait_for(server, &session, |s| !s.approvals.is_empty() || !s.running).await;
            for approval in &asked.approvals {
                server
                    .answer_approval(
                        PROJECT,
                        &session,
                        &approval.approval_id,
                        true,
                        desktop::Remember::No,
                    )
                    .await
                    .unwrap();
            }
            wait_for(server, &session, |s| !s.running).await;
            server.after_turn_settled(PROJECT, &session).await.unwrap();
            server.conversation_status(PROJECT, &session).await.unwrap()
        }
    };
    // Once `count` reviews were asked for and what follows them settled.
    let settled = |session: String, count: usize| {
        let (server, reviews) = (&server, reviews.clone());
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while reviews.load(Ordering::Relaxed) < count {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("the reviews must be asked for");
            wait_for(server, &session, |s| !s.running).await;
            server.after_turn_settled(PROJECT, &session).await.unwrap();
            server.conversation_status(PROJECT, &session).await.unwrap()
        }
    };
    let choose = |session: String, auto_review: bool| {
        let server = &server;
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    desktop::ConversationOptions {
                        auto_review,
                        ..options
                    },
                )
                .await
                .unwrap()
                .auto_review
        }
    };

    // Off until it is asked for: a turn with a tool is not reviewed.
    let defaults = server.turn_defaults().await.unwrap();
    assert!(!defaults.auto_review);
    let quiet = create(&server).await;
    let status = turn(quiet.clone(), "write the file").await;
    assert!(!status.options.auto_review);
    assert_eq!(status.review, None);
    assert_eq!(reviews.load(Ordering::Relaxed), 0);

    // Asked for in one conversation: its tool turn is reviewed, and what
    // the review found is corrected once, as the WebView does: the turn is
    // resumed with the findings, nobody says anything new, and the
    // correction is reviewed in its turn without a second correction.
    let checked = create(&server).await;
    assert!(choose(checked.clone(), true).await);
    turn(checked.clone(), "write the file").await;
    let status = settled(checked.clone(), 2).await;
    let Some(desktop::ReviewOutcome::Done(review)) = status.review else {
        panic!("no review: {:?}", status.review);
    };
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].verdict, "fail");
    assert!(corrected.load(Ordering::Relaxed));
    let rows = snapshot(&server, &checked).await.items;
    let count = |role: &str| rows.iter().filter(|row| row.role == role).count();
    assert_eq!(count("user"), 1, "{rows:?}");
    assert_eq!(count("review"), 2, "{rows:?}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(reviews.load(Ordering::Relaxed), 2);
    // A short answer without tools is not worth a review, and the last
    // turn's review does not linger over it.
    let status = turn(checked.clone(), "thanks").await;
    assert_eq!(status.review, None);
    assert_eq!(reviews.load(Ordering::Relaxed), 2);
    // The other conversation was not switched on by this one.
    assert!(
        !server
            .conversation_status(PROJECT, &quiet)
            .await
            .unwrap()
            .options
            .auto_review
    );

    // On by default from now on: conversations that never chose follow,
    // one that chose keeps its choice.
    // (Saying what it already is writes nothing; turning it on and off
    // again is a choice.)
    assert!(!choose(quiet.clone(), false).await);
    assert!(choose(quiet.clone(), true).await);
    assert!(!choose(quiet.clone(), false).await);
    let saved = server
        .set_turn_defaults(desktop::TurnDefaults {
            auto_review: true,
            ..defaults
        })
        .await
        .unwrap();
    assert!(saved.auto_review);
    let fresh = create(&server).await;
    turn(fresh.clone(), "write the file").await;
    let status = settled(fresh.clone(), 4).await;
    assert!(status.options.auto_review);
    assert!(matches!(
        status.review,
        Some(desktop::ReviewOutcome::Done(_))
    ));
    assert!(
        !server
            .conversation_status(PROJECT, &quiet)
            .await
            .unwrap()
            .options
            .auto_review
    );

    // Put away, it is gone; only through the conversation's own project.
    assert!(server.dismiss_review("missing", &fresh).await.is_err());
    server.dismiss_review(PROJECT, &fresh).await.unwrap();
    assert_eq!(
        server
            .conversation_status(PROJECT, &fresh)
            .await
            .unwrap()
            .review,
        None
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_started_as_an_expert_works_under_its_instructions() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let experts = server
        .save_expert(desktop::ExpertInput {
            id: String::new(),
            name: "Statistician".into(),
            description: "Numbers".into(),
            instructions: "Always report medians with the interquartile range.".into(),
            model: String::new(),
            skills: None,
            connectors: None,
        })
        .await
        .unwrap();
    let stats = experts
        .iter()
        .find(|expert| expert.name == "Statistician")
        .unwrap()
        .clone();
    assert_eq!(server.experts().await.unwrap(), experts);

    let choose = |session: String, expert: String| {
        let server = &server;
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    desktop::ConversationOptions { expert, ..options },
                )
                .await
        }
    };
    let prompt = |session: String| {
        let server = &server;
        async move {
            server.model_view(PROJECT, &session).await.unwrap().rows[0]
                .text
                .clone()
        }
    };
    let session = create(&server).await;
    assert!(choose(session.clone(), "ghost".into()).await.is_err());
    assert_eq!(
        choose(session.clone(), stats.id.clone())
            .await
            .unwrap()
            .expert,
        stats.id
    );
    send(&server, &session, "summarise the counts")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let instructed = prompt(session.clone()).await;
    assert!(
        instructed.ends_with(
            "## Specialist: Statistician\nAlways report medians with the interquartile range."
        ) || instructed.contains(
            "## Specialist: Statistician\nAlways report medians with the interquartile range."
        ),
        "{instructed}"
    );
    // Once it has started, the conversation is what it is.
    let late = choose(session.clone(), String::new()).await.unwrap_err();
    assert!(
        late.to_string().contains("before the first message"),
        "{late}"
    );
    // Removing the expert does not rewrite the conversation.
    server.delete_expert(&stats.id).await.unwrap();
    send(&server, &session, "and again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(prompt(session.clone())
        .await
        .contains("## Specialist: Statistician"));
    assert_eq!(
        server
            .conversation_status(PROJECT, &session)
            .await
            .unwrap()
            .options
            .expert,
        ""
    );

    // A conversation with no expert has no such section.
    let plain = create(&server).await;
    send(&server, &plain, "hello").await.unwrap();
    wait_for(&server, &plain, |s| !s.running).await;
    assert!(!prompt(plain).await.contains("## Specialist"));
    assert!(!server.commands().iter().any(|name| name.contains("expert")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_file_is_traced_back_to_the_call_that_wrote_it() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // A model that writes answer.txt, then answers.
    let server = Server::open_with_models(config(temp.path()), models(calls, true))
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing has written it yet.
    assert_eq!(
        server
            .file_origin(PROJECT, Some(&session), "answer.txt")
            .await
            .unwrap(),
        None
    );
    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;

    let origin = server
        .file_origin(PROJECT, Some(&session), "answer.txt")
        .await
        .unwrap()
        .expect("the write was kept");
    assert_eq!(origin.session, session);
    assert_eq!(origin.title, "write the file");
    assert_eq!(origin.tool, "write");
    // Found from another conversation of the project too, and by the
    // file's full path.
    let other = create(&server).await;
    let full = temp.path().join("workspace/answer.txt");
    let again = server
        .file_origin(PROJECT, Some(&other), &full.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(again.map(|origin| origin.session), Some(session.clone()));
    // A file nobody wrote here has no origin; a path outside is refused.
    assert_eq!(
        server
            .file_origin(PROJECT, None, "notes.txt")
            .await
            .unwrap(),
        None
    );
    assert!(server
        .file_origin(PROJECT, None, "../outside.txt")
        .await
        .is_err());
    assert!(server
        .file_origin("missing", None, "answer.txt")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_sample_project_is_made_where_the_user_says() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let parent = temp.path().join("somewhere");
    std::fs::create_dir(&parent).unwrap();
    let project = server.create_sample_project(&parent).await.unwrap();
    let folder = parent.join("Wisp sample project");
    assert!(folder.join("data/counts.csv").is_file());
    assert!(folder.join("scripts/summary.py").is_file());
    // It is an ordinary project: listed, with its files, and conversable.
    let listed = server.project_directory(&project, "").await.unwrap();
    assert!(listed.entries.iter().any(|entry| entry.name == "README.md"));
    server.create_conversation(&project).await.unwrap();
    // A second one goes beside the first.
    let second = server.create_sample_project(&parent).await.unwrap();
    assert_ne!(second, project);
    assert!(parent.join("Wisp sample project 2").is_dir());
    // Not in Wisp's own data, and not in a folder that is not there.
    assert!(server
        .create_sample_project(&temp.path().join("data"))
        .await
        .is_err());
    assert!(server
        .create_sample_project(&temp.path().join("missing"))
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn an_example_is_read_and_copied_into_a_project_to_go_on_with() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let examples = server.demos().await.unwrap();
    assert_eq!(examples.len(), 6);
    let rnaseq = &examples[2];
    assert_eq!(rnaseq.id, "manifest_esr1_03_rnaseq");
    let read = server.demo(&rnaseq.id).await.unwrap();
    assert_eq!(read.title, rnaseq.title);
    assert!(read
        .rows
        .iter()
        .any(|row| row.role == desktop::DemoRole::Tool));
    assert!(server.demo("../manifest_esr1_03_rnaseq").await.is_err());

    let parent = temp.path().join("somewhere");
    std::fs::create_dir(&parent).unwrap();
    let project = server.create_sample_project(&parent).await.unwrap();
    let folder = parent.join("Wisp sample project");
    std::fs::write(folder.join("GSE153250_counts_matrix.tsv"), "mine").unwrap();
    let session = server.copy_demo(&rnaseq.id, &project).await.unwrap();
    // A conversation of its own, named as the example, to go on with.
    let listed = server.conversations(&project, "", false).await.unwrap();
    let row = listed.rows.iter().find(|row| row.id == session).unwrap();
    assert_eq!(row.title, rnaseq.title);
    let saved = server.0.store.load_messages(&session).await.unwrap();
    assert!(saved.len() > 50, "{}", saved.len());
    assert!(saved[0].content.as_text().contains("GSE153250"));
    // Its files, and nothing of the project's written over.
    assert!(folder.join("GSE153250_sample_groups.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(folder.join("GSE153250_counts_matrix.tsv")).unwrap(),
        "mine"
    );
    // Copied again, it is another conversation.
    let again = server.copy_demo(&rnaseq.id, &project).await.unwrap();
    assert_ne!(again, session);
    assert!(server
        .copy_demo("manifest_nothing", &project)
        .await
        .is_err());
    assert!(server
        .copy_demo(&rnaseq.id, "no-such-project")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversations_research_is_drafted_reviewed_and_frozen() {
    // A model that answers a turn, and drafts an archive when asked to.
    struct Archivist;
    #[async_trait]
    impl Provider for Archivist {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let system = messages[0].content.as_text();
            let content = if system.starts_with("Prepare a research notebook archive") {
                assert!(messages[1].content.as_text().contains("count the reads"));
                r###"{"title":"Read counts","report":"## Findings\n100 reads.",
                    "scripts":[{"filename":"count.sh","content":"wc -l reads.fq"}]}"###
            } else {
                "[]"
            };
            Ok(Completion {
                content: content.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("There are 100 reads.");
            Ok(Completion {
                content: "There are 100 reads.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(Archivist));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing said yet: nothing to archive.
    assert!(server
        .draft_research_archive(PROJECT, &session)
        .await
        .is_err());
    send(&server, &session, "count the reads").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        server.research_archive(PROJECT, &session).await.unwrap(),
        None
    );

    // The draft is the model's; nothing is saved by drafting.
    let mut draft = server
        .draft_research_archive(PROJECT, &session)
        .await
        .unwrap();
    assert_eq!(draft.title, "Read counts");
    assert_eq!(draft.scripts.len(), 1);
    assert_eq!(
        server.research_archive(PROJECT, &session).await.unwrap(),
        None
    );
    assert!(!snapshot(&server, &session).await.read_only);
    assert_eq!(snapshot(&server, &session).await.read_only_reason, None);

    // What is frozen is what the researcher made of it.
    draft.report.push_str("\n\nChecked by hand.");
    let refused = desktop::ArchiveDraft {
        title: " ".into(),
        ..draft.clone()
    };
    assert!(server
        .freeze_research_archive(PROJECT, &session, refused)
        .await
        .is_err());
    let frozen = server
        .freeze_research_archive(PROJECT, &session, draft)
        .await
        .unwrap();
    assert!(frozen.frozen_at.is_some());
    let folder = temp
        .path()
        .join("workspace/.wisp/research-archives")
        .join(&frozen.id);
    assert_eq!(
        std::fs::read_to_string(folder.join("REPORT.md")).unwrap(),
        "# Read counts\n\n## Findings\n100 reads.\n\nChecked by hand.\n"
    );
    assert!(folder.join("scripts/count.sh").is_file());
    assert_eq!(
        server.research_archive(PROJECT, &session).await.unwrap(),
        Some(frozen.clone())
    );

    // From now on it is a record: read, not continued or archived again.
    let record = snapshot(&server, &session).await;
    assert!(record.read_only);
    assert_eq!(record.read_only_reason, Some(dto::ReadOnlyReason::Archived));
    let late = send(&server, &session, "one more thing").await.unwrap_err();
    assert!(late.contains("read-only"), "{late}");
    assert!(server
        .draft_research_archive(PROJECT, &session)
        .await
        .is_err());
    // Research goes on in a new conversation that starts from the archive.
    let continued = server.continue_research(PROJECT, &session).await.unwrap();
    assert_ne!(continued, session);
    let opening = server.whole_transcript(&continued).await.unwrap();
    let said = opening[0].content.as_text();
    assert!(
        said.starts_with("Continue research from the archived milestone: "),
        "{said}"
    );
    assert!(said.contains(&format!(".wisp/research-archives/{}/REPORT.md", frozen.id)));
    assert!(!snapshot(&server, &continued).await.read_only);
    // Only from a frozen archive.
    assert!(server.continue_research(PROJECT, &continued).await.is_err());
    // Another conversation of the project is untouched.
    let next = create(&server).await;
    send(&server, &next, "count the reads again").await.unwrap();
    wait_for(&server, &next, |s| !s.running).await;
    assert!(server.research_archive("missing", &session).await.is_err());
    assert!(!server
        .commands()
        .iter()
        .any(|name| name.contains("archive_research")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_is_exported_whole_as_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // A model that runs one tool, then answers.
    let server = Server::open_with_models(config(temp.path()), models(calls, true))
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;
    send(&server, &session, "and again").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;

    let text = server.export_conversation(PROJECT, &session).await.unwrap();
    // The title, the project, and both questions in the order they were asked.
    assert!(
        text.starts_with("# write the file\n\nProject: Server\n"),
        "{text}"
    );
    let first = text.find("## You\n\nwrite the file").unwrap();
    let second = text.find("## You\n\nand again").unwrap();
    assert!(first < second, "{text}");
    // What the model did and said is between them.
    let between = &text[first..second];
    assert!(between.contains("**Tool `"), "{text}");
    assert!(between.contains("## Wisp"), "{text}");
    // A page to share has what was said, in order, and no tools.
    let shared = server.share_rows(PROJECT, &session).await.unwrap();
    assert_eq!(
        shared.first().map(|row| row.text.as_str()),
        Some("write the file")
    );
    assert!(shared.iter().any(|row| row.role == "assistant"));
    assert!(shared
        .iter()
        .all(|row| matches!(row.role.as_str(), "user" | "assistant")));
    assert!(server.share_rows("missing", &session).await.is_err());
    // Only the conversation's own project may export it.
    assert!(server
        .export_conversation("missing", &session)
        .await
        .is_err());
    assert!(server
        .export_conversation(PROJECT, "missing")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn capabilities_count_what_a_project_can_draw_on() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let start = server.capabilities(PROJECT).await.unwrap();
    // Every bundled connector is on to begin with; nothing else is set up.
    assert!(start.connectors.1 > 0 && start.connectors.0 == start.connectors.1);
    assert_eq!(start.mcp_servers, (0, 0));
    assert_eq!(start.hosts, (0, 0));
    assert_eq!((start.memory_notes, start.habits), (0, 0));
    assert!(start.memory_on);
    // Each tool looked for is listed, found or not.
    assert_eq!(start.tools.len(), 8);
    assert!(start.tools.iter().any(|tool| tool.name == "git"));
    for tool in &start.tools {
        if let Some(path) = &tool.path {
            assert!(std::path::Path::new(path).is_file(), "{path}");
        }
    }

    // What is switched off or added shows in the counts.
    let connector = server.connectors().await.unwrap()[0].domain.clone();
    server
        .set_connector_enabled(&connector, false)
        .await
        .unwrap();
    server
        .save_ssh_host(desktop::SshHostRow {
            alias: "cap-host".into(),
            host_name: "10.0.0.9".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    server.add_habit("Report effect sizes").await.unwrap();
    server.set_memory_enabled(false).await.unwrap();
    let after = server.capabilities(PROJECT).await.unwrap();
    assert_eq!(
        after.connectors,
        (start.connectors.1 - 1, start.connectors.1)
    );
    // Saved, but not checked: it cannot run anything yet.
    assert_eq!(after.hosts, (0, 1));
    assert_eq!(after.habits, 1);
    assert!(!after.memory_on);
    assert!(server.capabilities("missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_project_is_exported_as_an_archive_and_imported_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let source = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    // A project with a conversation and a file of its own.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("counts.csv"), "gene,count\nA,1\n").unwrap();
    let lab = source.register_local_project(&folder).await.unwrap();
    let session = source.create_conversation(&lab).await.unwrap();
    source
        .0
        .store
        .rename_session(&session, &lab, "Alignment")
        .await
        .unwrap();

    let archive = temp.path().join("lab.zip");
    // Where to write must be said in full.
    assert!(source
        .export_project(&lab, std::path::Path::new("lab.zip"))
        .await
        .is_err());
    assert!(source.export_project("missing", &archive).await.is_err());
    source.export_project(&lab, &archive).await.unwrap();
    assert!(archive.is_file());
    // The project is already here: its own archive is not imported twice.
    let parent = temp.path().join("imports");
    std::fs::create_dir(&parent).unwrap();
    let again = source.import_project(&archive, &parent).await.unwrap_err();
    assert!(again.to_string().contains("already present"), "{again}");
    source.shutdown().await;
    drop(source);

    // Another host, with a database of its own, reads it back whole.
    let other = tempfile::tempdir().unwrap();
    let target = Server::open_with_models(config(other.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let parent = other.path().join("imports");
    std::fs::create_dir(&parent).unwrap();
    // Not inside the host's own data, and only from a real archive.
    assert!(target
        .import_project(&archive, &other.path().join("data"))
        .await
        .is_err());
    assert!(target
        .import_project(&other.path().join("missing.zip"), &parent)
        .await
        .is_err());
    let imported = target.import_project(&archive, &parent).await.unwrap();
    assert_eq!(imported, lab);
    let settings = target.project_settings(&imported).await.unwrap();
    assert_eq!(settings.name, "lab");
    let sessions = target.0.store.list_sessions(&imported).await.unwrap();
    assert_eq!(sessions.len(), 1);
    // Its files came along, under the folder that was chosen.
    let (_, _, root) = target
        .0
        .store
        .get_project_meta(&imported)
        .await
        .unwrap()
        .unwrap();
    assert!(std::path::Path::new(&root).starts_with(std::fs::canonicalize(&parent).unwrap()));
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&root).join("counts.csv")).unwrap(),
        "gene,count\nA,1\n"
    );
    target.shutdown().await;
}

#[tokio::test]
async fn a_project_is_exported_as_a_folder_package_and_opened_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let source = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("counts.csv"), "gene,count\nA,1\n").unwrap();
    let lab = source.register_local_project(&folder).await.unwrap();
    source.create_conversation(&lab).await.unwrap();

    let parent = temp.path().join("exports");
    std::fs::create_dir(&parent).unwrap();
    // Only into a folder that is there, said in full.
    assert!(source
        .export_project_folder(&lab, std::path::Path::new("exports"))
        .await
        .is_err());
    assert!(source
        .export_project_folder(&lab, &parent.join("missing"))
        .await
        .is_err());
    let package = source.export_project_folder(&lab, &parent).await.unwrap();
    assert!(package.starts_with(&parent) && package.is_dir());
    // How far it got is only kept while it goes on.
    assert_eq!(source.archive_progress(), None);
    // A second export makes a second folder rather than writing over it.
    let second = source.export_project_folder(&lab, &parent).await.unwrap();
    assert_ne!(second, package);
    // It is this host's own project already.
    assert!(source.import_project_folder(&package).await.is_err());
    source.shutdown().await;
    drop(source);

    let other = tempfile::tempdir().unwrap();
    let target = Server::open_with_models(config(other.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(target
        .import_project_folder(&other.path().join("missing"))
        .await
        .is_err());
    let opened = target.import_project_folder(&package).await.unwrap();
    assert_eq!(opened, lab);
    assert_eq!(
        target.0.store.list_sessions(&opened).await.unwrap().len(),
        1
    );
    let (_, _, root) = target
        .0
        .store
        .get_project_meta(&opened)
        .await
        .unwrap()
        .unwrap();
    assert!(
        std::path::Path::new(&root).join("counts.csv").is_file(),
        "{root}"
    );
    target.shutdown().await;
}

#[tokio::test]
async fn conversations_map_over_items_and_decide_only_with_a_decision_model() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // The settings are the desktop's own, not commands a remote client has.
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("decision")));
    let map_items = |seen: &Arc<std::sync::Mutex<Vec<String>>>| {
        let mut seen = seen.lock().unwrap();
        let tool = seen
            .iter()
            .rev()
            .find(|entry| entry.contains("name: \"map_items\""))
            .cloned()
            .expect("map_items is offered");
        seen.clear();
        tool
    };

    let session = create(&server).await;
    send(&server, &session, "screen the abstracts")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(!map_items(&seen).contains("\"decide\": Object"));

    // A self-hosted decision model needs no key; from the next turn the
    // tool offers its decide step.
    let start = server.decision_model().await.unwrap();
    assert!(!start.enabled);
    let local = wisp_dto::DecisionModelSettings {
        enabled: true,
        ..wisp_dto::DecisionModelSettings::from_preset(
            wisp_dto::DecisionPreset::find("startlux_local").unwrap(),
        )
    };
    let saved = server.save_decision_model(local, None).await.unwrap();
    assert!(saved.enabled && saved.stored_keys.is_empty());
    send(&server, &session, "and decide which to keep")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(map_items(&seen).contains("\"decide\": Object"));

    // Forgetting a key that was never kept is no error and changes nothing.
    let after = server.clear_decision_model_key("typesafe").await.unwrap();
    assert_eq!(after, saved);
}

#[tokio::test]
async fn a_paper_keeps_its_figures_with_the_results_and_runs_they_rest_on() {
    use desktop::{PaperInput, PaperItemInput, PaperItemKind, PaperSourceKind};
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("paper") || command.contains("publication")));
    assert!(server.papers(PROJECT).await.unwrap().is_empty());
    assert!(server.papers("missing").await.is_err());

    // A paper needs a title; its first version has the usual name.
    let blank = PaperInput::default();
    assert!(server.create_paper(PROJECT, blank).await.is_err());
    let paper = server
        .create_paper(
            PROJECT,
            PaperInput {
                title: "  Plasmid copy number ".into(),
                description: " draft for the lab meeting ".into(),
                version: String::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(paper.title, "Plasmid copy number");
    assert_eq!(paper.description, "draft for the lab meeting");
    assert_eq!(paper.versions.len(), 1);
    assert_eq!(paper.versions[0].label, "Initial submission");
    assert_eq!(paper.versions[0].state, desktop::PaperVersionState::Draft);
    assert!(paper.editable && paper.items.is_empty());
    let version = paper.shown.clone();
    assert_eq!(server.papers(PROJECT).await.unwrap()[0].id, paper.id);

    // Figures and tables are added in order and can be changed.
    let item = |kind, title: &str| PaperItemInput {
        id: String::new(),
        kind,
        title: title.into(),
        note: String::new(),
    };
    server
        .save_paper_item(PROJECT, &version, item(PaperItemKind::Figure, "Figure 1"))
        .await
        .unwrap();
    server
        .save_paper_item(PROJECT, &version, item(PaperItemKind::Table, "Table 1"))
        .await
        .unwrap();
    assert!(server
        .save_paper_item(PROJECT, &version, item(PaperItemKind::Figure, " "))
        .await
        .is_err());
    assert!(server
        .save_paper_item(PROJECT, "missing", item(PaperItemKind::Figure, "Figure 2"))
        .await
        .is_err());
    let shown = server.paper(PROJECT, &paper.id, None).await.unwrap();
    let titles: Vec<_> = shown.items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(titles, ["Figure 1", "Table 1"]);
    let figure = shown.items[0].id.clone();
    server
        .save_paper_item(
            PROJECT,
            &version,
            PaperItemInput {
                id: figure.clone(),
                kind: PaperItemKind::Figure,
                title: "Figure 1: copy number by strain".into(),
                note: "qPCR, three replicates".into(),
            },
        )
        .await
        .unwrap();

    // A run of the project is offered as evidence and bound with a reason.
    let run = wisp_store::RunRecord::new("run-qpcr", PROJECT, "local", "qpcr_fit.py", "script");
    server.0.store.create_run(&run).await.unwrap();
    let (runs, more) = server
        .paper_source_choices(PROJECT, PaperSourceKind::Run, "QPCR", 0)
        .await
        .unwrap();
    assert!(!more);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].name, "qpcr_fit.py");
    assert!(server
        .paper_source_choices(PROJECT, PaperSourceKind::Other, "", 0)
        .await
        .is_err());
    server
        .bind_paper_source(
            PROJECT,
            &version,
            &figure,
            PaperSourceKind::Run,
            &runs[0].id,
            " the fit behind panel B ",
        )
        .await
        .unwrap();
    // A source that is not there, or an item of no version, is refused.
    for (item, source) in [(figure.as_str(), "run-absent"), ("item-absent", "run-qpcr")] {
        assert!(server
            .bind_paper_source(PROJECT, &version, item, PaperSourceKind::Run, source, "")
            .await
            .is_err());
    }

    // A file a turn wrote is a result of the project, bound at the
    // version it has now.
    let session = create(&server).await;
    server
        .set_conversation_options(
            PROJECT,
            &session,
            desktop::ConversationOptions {
                full_access: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    send(&server, &session, "write the answer").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let (results, _) = server
        .paper_source_choices(PROJECT, PaperSourceKind::Result, "", 0)
        .await
        .unwrap();
    assert_eq!(results.len(), 1, "{results:?}");
    server
        .bind_paper_source(
            PROJECT,
            &version,
            &figure,
            PaperSourceKind::Result,
            &results[0].id,
            "",
        )
        .await
        .unwrap();

    let shown = server
        .paper(PROJECT, &paper.id, Some(&version))
        .await
        .unwrap();
    assert_eq!(shown.items[0].title, "Figure 1: copy number by strain");
    assert_eq!(shown.items[0].note, "qPCR, three replicates");
    let sources = &shown.items[0].sources;
    assert_eq!(sources.len(), 2);
    let of_run = sources
        .iter()
        .find(|source| source.kind == PaperSourceKind::Run)
        .unwrap();
    assert_eq!(of_run.name, "qpcr_fit.py");
    assert_eq!(of_run.purpose, "the fit behind panel B");
    let of_result = sources
        .iter()
        .find(|source| source.kind == PaperSourceKind::Result)
        .unwrap();
    assert_eq!(of_result.name, results[0].name);
    assert!(shown.items[1].sources.is_empty() && shown.loose.is_empty());

    // Evidence and items are removed one at a time.
    server
        .unbind_paper_source(PROJECT, &version, &of_run.id)
        .await
        .unwrap();
    assert!(server
        .unbind_paper_source(PROJECT, &version, &of_run.id)
        .await
        .is_err());
    let table = shown.items[1].id.clone();
    server
        .delete_paper_item(PROJECT, &version, &table)
        .await
        .unwrap();
    assert!(server
        .delete_paper_item(PROJECT, &version, &table)
        .await
        .is_err());
    let shown = server.paper(PROJECT, &paper.id, None).await.unwrap();
    assert_eq!(shown.items.len(), 1);
    assert_eq!(shown.items[0].sources.len(), 1);

    // A paper belongs to its project.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    assert!(server.paper(&lab, &paper.id, None).await.is_err());
    assert!(server.delete_paper(&lab, &paper.id).await.is_err());
    assert!(server
        .save_paper_item(&lab, &version, item(PaperItemKind::Claim, "Claim"))
        .await
        .is_err());

    server
        .rename_paper(PROJECT, &paper.id, "Copy number control", "")
        .await
        .unwrap();
    assert!(server
        .rename_paper(PROJECT, &paper.id, "", "")
        .await
        .is_err());
    assert_eq!(
        server.papers(PROJECT).await.unwrap()[0].title,
        "Copy number control"
    );
    server.delete_paper(PROJECT, &paper.id).await.unwrap();
    assert!(server.papers(PROJECT).await.unwrap().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn a_paper_version_is_checked_waived_finalized_and_continued() {
    use desktop::{
        PaperAudience, PaperFinding, PaperInput, PaperItemInput, PaperItemKind, PaperPolicy,
        PaperSourceKind, PaperVersionState,
    };
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    assert!(!server.commands().iter().any(|command| {
        command.contains("final") || command.contains("waive") || command.contains("freeze")
    }));
    let paper = server
        .create_paper(
            PROJECT,
            PaperInput {
                title: "Copy number".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let version = paper.shown.clone();
    let figure = PaperItemInput {
        id: String::new(),
        kind: PaperItemKind::Figure,
        title: "Figure 1".into(),
        note: String::new(),
    };
    server
        .save_paper_item(PROJECT, &version, figure.clone())
        .await
        .unwrap();
    let figure_id = server.paper(PROJECT, &paper.id, None).await.unwrap().items[0]
        .id
        .clone();
    let codes = |found: &[PaperFinding]| -> Vec<String> {
        found.iter().map(|finding| finding.code.clone()).collect()
    };

    // With nothing bound the check says so, and finalizing is refused.
    let private = PaperPolicy::default();
    assert_eq!(private.audience, PaperAudience::Private);
    let check = server
        .check_paper(PROJECT, &version, private)
        .await
        .unwrap();
    assert!(!check.can_finalize && !check.finalized);
    let blockers = codes(&check.blockers);
    assert!(
        blockers.contains(&"no_selected_evidence".to_owned()),
        "{blockers:?}"
    );
    assert!(blockers.contains(&"publication_item_missing_evidence".to_owned()));
    let refused = server
        .finalize_paper(PROJECT, &version, private)
        .await
        .unwrap();
    assert!(!refused.finalized);
    let shown = server.paper(PROJECT, &paper.id, None).await.unwrap();
    assert_eq!(shown.versions[0].state, PaperVersionState::Draft);
    assert!(shown.versions[0].fingerprint.is_empty());

    // For the public, both reviews have to be confirmed.
    let public = PaperPolicy {
        audience: PaperAudience::Public,
        ..private
    };
    let blockers = codes(
        &server
            .check_paper(PROJECT, &version, public)
            .await
            .unwrap()
            .blockers,
    );
    assert!(blockers.contains(&"sensitive_data_review_required".to_owned()));
    assert!(blockers.contains(&"redistribution_review_required".to_owned()));

    // A finished run is bound; what it lacks can stand once a reason is
    // given.
    let mut run = wisp_store::RunRecord::new("run-fit", PROJECT, "local", "fit.py", "script");
    run.status = wisp_store::RunStatus::Succeeded;
    server.0.store.create_run(&run).await.unwrap();
    server
        .bind_paper_source(
            PROJECT,
            &version,
            &figure_id,
            PaperSourceKind::Run,
            "run-fit",
            "the fit",
        )
        .await
        .unwrap();
    let check = server
        .check_paper(PROJECT, &version, private)
        .await
        .unwrap();
    assert!(!check.can_finalize && !check.blockers.is_empty());
    assert!(
        check.blockers.iter().all(|finding| finding.waivable),
        "{:?}",
        check.blockers
    );
    let first = check.blockers[0].code.clone();
    assert!(server
        .waive_paper_finding(PROJECT, &version, &first, "  ")
        .await
        .is_err());
    for finding in &check.blockers {
        server
            .waive_paper_finding(
                PROJECT,
                &version,
                &finding.code,
                " kept in the lab notebook ",
            )
            .await
            .unwrap();
    }
    let check = server
        .check_paper(PROJECT, &version, private)
        .await
        .unwrap();
    assert!(check.can_finalize, "{:?}", check.blockers);
    assert!(check
        .blockers
        .iter()
        .all(|finding| finding.waived.as_deref() == Some("kept in the lab notebook")));

    // Finalizing locks the version under a fingerprint.
    let done = server
        .finalize_paper(PROJECT, &version, private)
        .await
        .unwrap();
    assert!(done.finalized);
    assert_eq!(done.fingerprint.len(), 64);
    let shown = server.paper(PROJECT, &paper.id, None).await.unwrap();
    assert_eq!(shown.versions[0].state, PaperVersionState::Finalized);
    assert_eq!(shown.versions[0].fingerprint, done.fingerprint);
    assert!(!shown.editable);
    assert!(server
        .save_paper_item(PROJECT, &version, figure.clone())
        .await
        .is_err());
    assert!(server
        .check_paper(PROJECT, &version, private)
        .await
        .is_err());
    assert!(server
        .waive_paper_finding(PROJECT, &version, &first, "again")
        .await
        .is_err());
    assert!(server.delete_paper(PROJECT, &paper.id).await.is_err());

    // A new version is a draft with the same items and evidence, named
    // after its number unless a name is given.
    let next = server
        .new_paper_version(PROJECT, &paper.id, &version, "")
        .await
        .unwrap();
    assert_eq!(next.versions.len(), 2);
    assert_eq!(next.shown, next.versions[0].id);
    assert_eq!(next.versions[0].label, "Version 2");
    assert_eq!(next.versions[0].state, PaperVersionState::Draft);
    assert!(next.editable);
    assert_eq!(next.items[0].title, "Figure 1");
    assert_eq!(next.items[0].sources.len(), 1);
    server
        .save_paper_item(PROJECT, &next.shown, figure)
        .await
        .unwrap();
    let named = server
        .new_paper_version(PROJECT, &paper.id, &version, " Reply to reviewers ")
        .await
        .unwrap();
    assert_eq!(named.versions[0].label, "Reply to reviewers");

    // Another project's paper is refused.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    assert!(server
        .check_paper(&lab, &next.shown, private)
        .await
        .is_err());
    assert!(server
        .waive_paper_finding(&lab, &next.shown, &first, "reason")
        .await
        .is_err());
    assert!(server
        .new_paper_version(&lab, &paper.id, &version, "")
        .await
        .is_err());
    server.shutdown().await;
}

/// A stand-in ACP agent: answers `initialize` with a name and one way to
/// sign in, and stops when its input closes.
#[cfg(unix)]
const FAKE_ACP_AGENT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | grep -o '"id":[^,}]*' | head -n 1 | cut -d: -f2-)
  method=$(printf '%s\n' "$line" | grep -o '"method":"[^"]*"' | head -n 1 | cut -d'"' -f4)
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"login","name":"Sign in"},{"type":"terminal","id":"tui","name":"Sign in in a terminal","args":["auth","login"],"env":{"FAKE_AGENT_AUTH":"1"}}],"agentInfo":{"name":"fake-agent","title":"Fake agent","version":"0.1.0"}}}\n' "$id"
      ;;
    authenticate)
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      ;;
  esac
done
"#;

#[cfg(unix)]
pub(crate) fn fake_acp_agent(dir: &Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("fake-acp-agent");
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// A stand-in ACP agent that holds one session. A prompt is answered with
/// a thought, a tool call that completes, a plan and a line of text; one
/// that says "ask" first asks permission and says how it was answered; one
/// that says "wait" answers only once it is cancelled.
#[cfg(unix)]
const FAKE_ACP_CONVERSATION: &str = r#"#!/bin/sh
say() { printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":%s}}\n' "$1"; }
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | grep -o '"id":[^,}]*' | head -n 1 | cut -d: -f2-)
  method=$(printf '%s\n' "$line" | grep -o '"method":"[^"]*"' | head -n 1 | cut -d'"' -f4)
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"mcpCapabilities":{"http":true}},"authMethods":[],"agentInfo":{"name":"fake-agent","version":"0.1.0"}}}\n' "$id"
      ;;
    session/new)
      mcp=$(printf '%s\n' "$line" | grep -o '"url":"[^"]*"' | head -n 1 | cut -d'"' -f4)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"s1","modes":{"currentModeId":"default","availableModes":[{"id":"default","name":"Default"},{"id":"plan","name":"Plan","description":"Plan before changing anything"}]},"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"fast","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}]}}\n' "$id"
      ;;
    session/set_mode)
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      ;;
    session/set_config_option)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"deep","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}]}}\n' "$id"
      ;;
    session/prompt)
      case "$line" in
        *wait*)
          while IFS= read -r next; do
            case "$next" in *session/cancel*) break ;; esac
          done
          printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"cancelled"}}\n' "$id"
          continue
          ;;
        *tools*)
          say "{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$mcp\"}}"
          ;;
        *ask*)
          printf '{"jsonrpc":"2.0","id":900,"method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"t2","title":"Write notes.txt","rawInput":{"path":"notes.txt"}},"options":[{"optionId":"yes","name":"Allow","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}]}}\n'
          IFS= read -r answer
          case "$answer" in
            *'"yes"'*) say '{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"You allowed it."}}' ;;
            *) say '{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"You turned it down."}}' ;;
          esac
          ;;
        *)
          say '{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"compact","description":"Summarise the session"},{"name":"init","description":"Write an AGENTS.md","input":{"hint":"what to cover"}}]}'
          say '{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Looking at the data"}}'
          say '{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read data.csv","kind":"read","status":"in_progress","rawInput":{"path":"data.csv"}}'
          say '{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","rawOutput":"3 rows"}'
          say '{"sessionUpdate":"plan","entries":[{"content":"Read the data","priority":"high","status":"completed"},{"content":"Fit the model","priority":"medium","status":"in_progress"}]}'
          say '{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello from the agent."}}'
          ;;
      esac
      printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
      ;;
  esac
done
"#;

#[cfg(unix)]
#[tokio::test]
async fn a_conversation_given_to_an_acp_agent_streams_asks_and_stops_through_it() {
    use desktop::{AcpAgent, ConversationOptions};
    let temp = tempfile::tempdir().unwrap();
    let calls: Arc<AtomicUsize> = Arc::default();
    let server = Server::open_with_models(config(temp.path()), models(calls.clone(), false))
        .await
        .unwrap();
    let script = fake_acp_agent(temp.path(), FAKE_ACP_CONVERSATION);
    let agent = server
        .save_acp_agent(AcpAgent {
            label: "Fake agent".into(),
            command: script.to_string_lossy().into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let id = create(&server).await;
    let give = |acp_agent: &str| ConversationOptions {
        acp_agent: acp_agent.into(),
        ..Default::default()
    };
    assert!(server
        .set_conversation_options(PROJECT, &id, give("missing"))
        .await
        .is_err());
    let options = server
        .set_conversation_options(PROJECT, &id, give(&agent))
        .await
        .unwrap();
    assert_eq!(options.acp_agent, agent);
    // The choice is for good: neither Wisp nor another agent takes over.
    assert!(server
        .set_conversation_options(PROJECT, &id, give(""))
        .await
        .is_err());

    // Its modes and options are known once it has started.
    assert!(server
        .conversation_status(PROJECT, &id)
        .await
        .unwrap()
        .acp
        .is_none());
    assert!(server.set_acp_mode(PROJECT, &id, "plan").await.is_err());
    send(&server, &id, "look at the data").await.unwrap();
    let done = wait_for(&server, &id, |s| !s.running).await;
    assert!(done.error.is_none(), "{:?}", done.error);
    assert_eq!(done.acp_agent_id.as_deref(), Some(agent.as_str()));
    let roles: Vec<_> = done.items.iter().map(|item| item.role.as_str()).collect();
    assert_eq!(
        roles,
        ["user", "tool", "tool", "reasoning", "assistant"],
        "{:?}",
        done.items
    );
    let read = &done.items[1];
    assert_eq!(read.tool_name.as_deref(), Some("Read data.csv"));
    assert!(read
        .input
        .as_deref()
        .unwrap_or_default()
        .contains("data.csv"));
    assert_eq!(read.text, "3 rows");
    assert_eq!(read.ok, Some(true));
    let steps = done.items[2]
        .plan_steps
        .as_ref()
        .expect("the plan is a checklist");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].content, "Fit the model");
    assert_eq!(done.items[3].text, "Looking at the data");
    assert_eq!(done.items[4].text, "Hello from the agent.");
    let binding = server.0.store.get_acp_session(&id).await.unwrap().unwrap();
    assert_eq!(binding.agent_profile_id, agent);
    assert_eq!(binding.agent_session_id, "s1");
    // Wisp's own model was never asked.
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let controls = server
        .conversation_status(PROJECT, &id)
        .await
        .unwrap()
        .acp
        .expect("the agent's mode and options are known");
    let current = |setting: &desktop::AcpSetting| match &setting.value {
        desktop::AcpValue::Choice { current, choices } => (
            current.clone(),
            choices
                .iter()
                .map(|choice| choice.id.clone())
                .collect::<Vec<_>>(),
        ),
        desktop::AcpValue::Flag(on) => (on.to_string(), vec![]),
    };
    let mode = controls.mode.as_ref().unwrap();
    assert_eq!(
        current(mode),
        ("default".into(), vec!["default".into(), "plan".into()])
    );
    assert_eq!(controls.options[0].name, "Model");
    assert_eq!(current(&controls.options[0]).0, "fast");
    // And the commands it accepts at the start of a message.
    let commands: Vec<_> = controls
        .commands
        .iter()
        .map(|command| (command.name.as_str(), command.hint.as_str()))
        .collect();
    assert_eq!(commands, [("compact", ""), ("init", "what to cover")]);
    let controls = server.set_acp_mode(PROJECT, &id, "plan").await.unwrap();
    assert_eq!(current(controls.mode.as_ref().unwrap()).0, "plan");
    let controls = server
        .set_acp_option(
            PROJECT,
            &id,
            "model",
            desktop::AcpPick::Choice("deep".into()),
        )
        .await
        .unwrap();
    assert_eq!(current(&controls.options[0]).0, "deep");
    let kept = server
        .conversation_status(PROJECT, &id)
        .await
        .unwrap()
        .acp
        .unwrap();
    assert_eq!(current(kept.mode.as_ref().unwrap()).0, "plan");
    assert_eq!(current(&kept.options[0]).0, "deep");

    // A permission question waits for the user, as a tool approval does.
    send(&server, &id, "ask before writing").await.unwrap();
    let asking = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    assert_eq!(asking.approvals[0].tool, "Write notes.txt");
    assert!(asking.approvals[0].preview.contains("notes.txt"));
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        json!({"session_id": id, "approval_id": asking.approvals[0].approval_id, "approved": false}),
    )
    .await
    .unwrap();
    let answered = wait_for(&server, &id, |s| !s.running).await;
    assert!(answered.error.is_none(), "{:?}", answered.error);
    assert_eq!(answered.items.last().unwrap().text, "You turned it down.");

    // Stop reaches the agent, and the conversation goes on afterwards.
    send(&server, &id, "wait for me").await.unwrap();
    wait_for(&server, &id, |s| s.running).await;
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id": id}),
    )
    .await
    .unwrap();
    let stopped = wait_for(&server, &id, |s| !s.running).await;
    assert!(stopped.error.is_some());
    // The agent was handed Wisp's tools over MCP at a loopback address.
    send(&server, &id, "which tools").await.unwrap();
    let told = wait_for(&server, &id, |s| !s.running).await;
    let url = told.items.last().unwrap().text.clone();
    assert!(
        url.starts_with("http://127.0.0.1:") && url.contains("/mcp/"),
        "{url}"
    );
    let listed: Value = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(&url)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "run_in_context"));

    // Nothing is saved for the stopped answer.
    assert_eq!(stopped.items.last().unwrap().role, "user");
    assert_eq!(stopped.model_id, "Fake agent");
    send(&server, &id, "look again").await.unwrap();
    let again = wait_for(&server, &id, |s| !s.running).await;
    assert!(again.error.is_none(), "{:?}", again.error);
    assert_eq!(again.items.last().unwrap().text, "Hello from the agent.");
    // The agent used the same call id again: each turn keeps its own row.
    let reads = again
        .items
        .iter()
        .filter(|item| item.tool_name.as_deref() == Some("Read data.csv"))
        .count();
    assert_eq!(reads, 2);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn the_first_run_guide_sets_up_deepseek_and_is_shown_once() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let server = Server::open_with_models(cfg, models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("onboarding")));
    assert!(!server.onboarding_done().await.unwrap());
    assert!(server.set_up_deepseek("  ").await.is_err());
    let settings = server.set_up_deepseek(" sk-test-key ").await.unwrap();
    let saved: Vec<_> = settings
        .profiles
        .iter()
        .map(|profile| profile.fields.model.as_str())
        .collect();
    assert_eq!(saved, ["deepseek-v4-pro", "deepseek-v4-flash"]);
    let flash = &settings.profiles[1];
    assert!(flash.is_default && flash.has_api_key);
    assert_eq!(flash.fields.api_url, "https://api.deepseek.com");
    assert!(settings.profiles[0].has_api_key);
    // The WebView reads the same setting.
    server.finish_onboarding().await.unwrap();
    assert!(server.onboarding_done().await.unwrap());
    assert_eq!(
        server
            .0
            .store
            .get_setting("onboarding_done")
            .await
            .unwrap()
            .as_deref(),
        Some("1")
    );
    server.shutdown().await;

    // Models set by the environment are not changed from the guide.
    let other = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(other.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(server.set_up_deepseek("sk-test-key").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn the_bridge_offers_wisps_science_tools_over_mcp_and_a_relay_carries_stdio() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("bridge") || command.contains("mcp_relay")));
    let id = create(&server).await;
    let root = server.project_root(PROJECT).await.unwrap();
    let offered = server.bridge_offer(PROJECT, &root, &id).await.unwrap();
    let bridge = offered.bridge.clone();
    let hidden = offered.hidden.len();
    assert!(hidden > 0, "the bundled connectors are found by search");
    bridge.offer(
        "token-1",
        std::path::Path::new(&root),
        offered.tools,
        offered.hidden,
    );
    let url = bridge.url("token-1");
    assert!(url.starts_with("http://127.0.0.1:"));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let ask = |body: Value| {
        let (client, url) = (client.clone(), url.clone());
        async move { client.post(&url).json(&body).send().await.unwrap() }
    };

    let hello: Value = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(hello["result"]["serverInfo"]["name"], "wisp-science");
    assert_eq!(hello["result"]["protocolVersion"], "2025-06-18");
    // A notification is taken without an answer.
    let noted = ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!(noted.status(), reqwest::StatusCode::ACCEPTED);

    let listed: Value = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await
        .json()
        .await
        .unwrap();
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    for wanted in [
        "use_skill",
        "list_skill_catalog",
        "run_in_context",
        "get_run",
        "wisp_search_tools",
        "wisp_use_tool",
    ] {
        assert!(
            names.iter().any(|name| name == wanted),
            "{wanted} in {names:?}"
        );
    }
    // The connectors are not listed one by one; a search finds them.
    assert!(!names.iter().any(|name| name == "search_articles"));
    let found: Value = ask(json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": {"name": "wisp_search_tools", "arguments": {"query": "search articles", "limit": 3}}}))
        .await
        .json()
        .await
        .unwrap();
    let found: Value =
        serde_json::from_str(found["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(found["results"][0]["tool_name"], "search_articles");
    assert!(found["results"].as_array().unwrap().len() <= 3);
    assert_eq!(found["total_hidden_tools"], hidden);
    // The listed tools are reached through wisp_use_tool too.
    let used: Value = ask(json!({"jsonrpc": "2.0", "id": 11, "method": "tools/call",
        "params": {"name": "wisp_use_tool", "arguments": {"tool_name": "list_skill_catalog", "tool_input": {}}}}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(used["result"]["isError"], false, "{used}");
    // The agent has its own file and shell tools.
    for kept_out in ["shell", "write", "edit", "read"] {
        assert!(!names.iter().any(|name| name == kept_out), "{kept_out}");
    }
    let called: Value = ask(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "list_skill_catalog", "arguments": {}}}))
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert_eq!(called["result"]["content"][0]["type"], "text");
    let unknown: Value = ask(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "nope", "arguments": {}}}))
    .await
    .json()
    .await
    .unwrap();
    assert!(unknown["error"]["message"]
        .as_str()
        .unwrap()
        .contains("nope"));
    // Another path offers nothing, and a withdrawn one stops.
    let stranger = client
        .post(bridge.url("guess"))
        .json(&json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), reqwest::StatusCode::NOT_FOUND);

    // The relay carries newline-delimited messages both ways and passes
    // nothing back for a notification.
    let input = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n\n\
                 {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/list\"}\n";
    let mut output = Vec::new();
    crate::acp_bridge::carry(
        &url,
        tokio::io::BufReader::new(input.as_bytes()),
        &mut output,
    )
    .await
    .unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["id"], 7);
    assert_eq!(lines[1]["id"], 8);
    bridge.withdraw("token-1");
    let gone = client
        .post(&url)
        .json(&json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), reqwest::StatusCode::NOT_FOUND);
    server.shutdown().await;
}

#[tokio::test]
async fn a_subscription_sign_in_is_kept_in_the_keychain_and_runs_its_models() {
    use desktop::{SecretStore, SignInStatus, Subscription};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(cfg, models(Arc::default(), false), secrets.clone())
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("subscription") || command.contains("sign_in")));
    assert!(server
        .subscriptions()
        .iter()
        .all(|row| row.account.is_none()));
    // Sign in with ChatGPT has no device code; that is refused at once.
    assert!(server
        .start_subscription_sign_in(Subscription::ChatGpt, true)
        .await
        .is_err());

    // A finished sign-in (the exchange itself needs the network).
    let creds = crate::subscriptions::Credentials::Codex(wisp_llm::codex_auth::CodexCredentials {
        access_token: "test-access-1".into(),
        refresh_token: "test-refresh".into(),
        expires_at_ms: wisp_llm::codex_auth::now_ms() + 3_600_000,
        account_id: "acct-test".into(),
    });
    let id = server.0.sign_ins.finished(creds);
    let state = server.subscription_sign_in(&id).unwrap();
    assert_eq!(state.status, SignInStatus::Done);
    assert_eq!(state.account, "acct-test");
    let settings = server.save_subscription_sign_in(&id).await.unwrap();
    assert!(server.save_subscription_sign_in(&id).await.is_err());
    assert_eq!(settings.profiles.len(), 1);
    let profile = &settings.profiles[0];
    assert_eq!(profile.fields.provider, "openai_codex");
    assert!(profile.has_api_key && profile.is_default && profile.problem.is_none());
    // The WebView's keychain name, so either desktop sees the sign-in.
    assert!(secrets
        .get("codex_subscription")
        .unwrap()
        .contains("test-access-1"));
    let row = server
        .subscriptions()
        .into_iter()
        .find(|row| row.subscription == Subscription::Codex)
        .unwrap();
    assert_eq!(row.account.as_deref(), Some("acct-test"));

    // Its model can be changed in the form without a key.
    let mut fields = profile.fields.clone();
    fields.model = "gpt-test-model".into();
    let changed = server
        .save_model_profile(desktop::ModelInput {
            id: Some(profile.id.clone()),
            name: String::new(),
            fields,
            api_key: None,
        })
        .await
        .unwrap();
    assert_eq!(changed.profiles[0].fields.model, "gpt-test-model");
    assert!(changed.profiles[0].problem.is_none());

    // A turn runs on the access token, refreshed only when due.
    let mut model = server.0.model.read().unwrap().pick("").unwrap();
    assert!(matches!(
        model.provider.kind,
        wisp_llm::ProviderKind::OpenAiCodex
    ));
    assert_eq!(model.provider.api_key, "test-access-1");
    assert_eq!(
        model.provider.base_url,
        wisp_llm::codex_auth::DEFAULT_BASE_URL
    );
    model.provider.api_key.clear();
    server
        .fresh_subscription(&mut model, "openai_codex")
        .await
        .unwrap();
    assert_eq!(model.provider.api_key, "test-access-1");

    // Signing out keeps the model, waiting for the next sign-in.
    let settings = server
        .sign_out_subscription(Subscription::Codex)
        .await
        .unwrap();
    assert!(settings.profiles[0]
        .problem
        .as_deref()
        .unwrap()
        .contains("Not signed in"));
    assert!(server.0.model.read().unwrap().pick("").is_err());

    // A browser sign-in takes only an address of its own attempt.
    let attempt = server
        .0
        .sign_ins
        .browser_for_test(Subscription::Codex, "state-1");
    assert!(server
        .submit_subscription_redirect(
            &attempt,
            "http://localhost:1455/auth/callback?code=abc&state=other"
        )
        .await
        .is_err());
    assert!(server
        .submit_subscription_redirect(&attempt, "   ")
        .await
        .is_err());
    server.cancel_subscription_sign_in(&attempt);
    assert!(server.subscription_sign_in(&attempt).is_err());
    server.shutdown().await;
}

/// Tencent's iLink bot service as far as WeChat binding and chatting
/// use it, on loopback.
#[derive(Default)]
struct FakeIlink {
    base: String,
    status_polls: usize,
    inbox: Vec<serde_json::Value>,
    sent: Vec<(String, String)>,
}

async fn fake_ilink() -> Arc<std::sync::Mutex<FakeIlink>> {
    use axum::{extract::State, routing::get, routing::post, Json, Router};
    type Shared = Arc<std::sync::Mutex<FakeIlink>>;
    let fake: Shared = Default::default();
    let app = Router::new()
        .route(
            "/ilink/bot/get_bot_qrcode",
            get(|| async {
                Json(json!({"ret": 0, "qrcode": "qr-1", "qrcode_img_content": "https://example.test/qr-1"}))
            }),
        )
        .route(
            "/ilink/bot/get_qrcode_status",
            get(|State(fake): State<Shared>| async move {
                let mut fake = fake.lock().unwrap();
                fake.status_polls += 1;
                Json(if fake.status_polls == 1 {
                    json!({"ret": 0, "status": "wait"})
                } else {
                    json!({
                        "ret": 0,
                        "status": "confirmed",
                        "bot_token": "test-bot-token",
                        "baseurl": fake.base,
                        "ilink_bot_id": "bot-1",
                        "ilink_user_id": "owner-1",
                    })
                })
            }),
        )
        .route(
            "/ilink/bot/getupdates",
            post(|State(fake): State<Shared>| async move {
                let msgs: Vec<_> = fake.lock().unwrap().inbox.drain(..).collect();
                Json(json!({"ret": 0, "longpolling_timeout_ms": 0, "get_updates_buf": "cursor-1", "msgs": msgs}))
            }),
        )
        .route(
            "/ilink/bot/sendmessage",
            post(
                |State(fake): State<Shared>, Json(body): Json<serde_json::Value>| async move {
                    let msg = &body["msg"];
                    fake.lock().unwrap().sent.push((
                        msg["to_user_id"].as_str().unwrap_or_default().to_owned(),
                        msg["item_list"][0]["text_item"]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    ));
                    Json(json!({"ret": 0}))
                },
            ),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    fake.lock().unwrap().base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    fake
}

fn say(fake: &std::sync::Mutex<FakeIlink>, from: &str, text: &str) {
    fake.lock().unwrap().inbox.push(json!({
        "from_user_id": from,
        "to_user_id": "bot-1",
        "context_token": "ctx",
        "item_list": [{"type": 1, "text_item": {"text": text}}],
    }));
}

/// The next reply after the `seen` first ones that `wanted` accepts.
async fn wait_sent(
    fake: &std::sync::Mutex<FakeIlink>,
    seen: &mut usize,
    wanted: impl Fn(&str) -> bool,
) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let found = {
                let fake = fake.lock().unwrap();
                fake.sent
                    .iter()
                    .enumerate()
                    .skip(*seen)
                    .find(|(_, (_, text))| wanted(text))
                    .map(|(at, (to, text))| (at, to.clone(), text.clone()))
            };
            if let Some((at, to, text)) = found {
                assert_eq!(to, "owner-1", "only the owner is answered");
                *seen = at + 1;
                return text;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the bot must reply")
}

#[tokio::test]
async fn wechat_binds_by_qr_code_and_runs_the_owners_messages_as_turns() {
    use desktop::{ChannelState, SecretStore, WeixinScan};
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), true),
        secrets.clone(),
    )
    .await
    .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("weixin") || command.contains("channel")));
    let fake = fake_ilink().await;
    *server.0.channels.ilink_base.lock().unwrap() = fake.lock().unwrap().base.clone();

    // Nothing is bound; WeChat cannot be turned on yet.
    let status = server
        .weixin_status(desktop::WeixinDestination::Projects)
        .await;
    assert!(!status.bound && !status.enabled);
    assert!(server
        .set_weixin_enabled(desktop::WeixinDestination::Projects, true)
        .await
        .is_err());
    let qr = server.start_weixin_bind().await.unwrap();
    assert_eq!(qr.qrcode, "qr-1");
    assert_eq!(qr.matrix.dark.len(), qr.matrix.width * qr.matrix.width);
    assert_eq!(
        server
            .poll_weixin_bind(desktop::WeixinDestination::Projects, "qr-1")
            .await
            .unwrap(),
        WeixinScan::Waiting
    );
    assert_eq!(
        server
            .poll_weixin_bind(desktop::WeixinDestination::Projects, "qr-1")
            .await
            .unwrap(),
        WeixinScan::Confirmed
    );
    // The WebView's keychain name, so either desktop uses the binding.
    assert_eq!(
        secrets.get("weixin_bot_token").as_deref(),
        Some("test-bot-token")
    );
    assert!(
        server
            .weixin_status(desktop::WeixinDestination::Projects)
            .await
            .bound
    );

    let folder = temp.path().join("chat project");
    std::fs::create_dir(&folder).unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    assert!(
        server
            .set_weixin_enabled(desktop::WeixinDestination::Projects, true)
            .await
            .unwrap()
            .enabled
    );

    let mut seen = 0;
    say(&fake, "owner-1", "/help");
    wait_sent(&fake, &mut seen, |text| text.contains("/project <序号")).await;
    // Someone else is not answered (wait_sent checks every reply's
    // recipient), and with no target yet a message is not run.
    say(&fake, "stranger", "/help");
    say(&fake, "owner-1", "hello");
    wait_sent(&fake, &mut seen, |text| text.contains("还没有 IM 目标项目")).await;
    say(&fake, "owner-1", &format!("/project {project}"));
    wait_sent(&fake, &mut seen, |text| {
        text.contains("IM 目标已切换到项目")
    })
    .await;

    // A turn from the chat asks before writing; the code answers it.
    say(&fake, "owner-1", "please write the answer");
    let asked = wait_sent(&fake, &mut seen, |text| text.contains("Wisp 等待审批")).await;
    assert!(asked.contains("工具: write"));
    let code = asked
        .split("/approve ")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(code.len(), 8);
    say(&fake, "owner-1", &format!("/approve {code}"));
    wait_sent(&fake, &mut seen, |text| text.starts_with("已批准")).await;
    let answer = wait_sent(&fake, &mut seen, |text| text.contains("Saved result")).await;
    assert_eq!(answer, "Saved result");
    assert!(folder.join("answer.txt").exists());
    // The conversation it started is the target from now on.
    say(&fake, "owner-1", "/status");
    let status = wait_sent(&fake, &mut seen, |text| text.contains("最近 IM 会话")).await;
    assert!(status.contains("chat project"), "{status}");
    assert!(
        server
            .weixin_status(desktop::WeixinDestination::Projects)
            .await
            .state
            == ChannelState::Running
    );

    // Unbinding forgets the token and stops the bot.
    let status = server
        .unbind_weixin(desktop::WeixinDestination::Projects)
        .await
        .unwrap();
    assert!(!status.bound && !status.enabled);
    assert_eq!(status.state, ChannelState::Stopped);
    assert_eq!(secrets.get("weixin_bot_token").as_deref(), Some(""));
    server.shutdown().await;
}

#[tokio::test]
async fn the_research_assistant_has_a_wechat_of_its_own() {
    use desktop::{ChannelState, SecretStore, WeixinDestination, WeixinScan};
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    let fake = fake_ilink().await;
    *server.0.channels.ilink_base.lock().unwrap() = fake.lock().unwrap().base.clone();

    // Bound for the assistant, under the WebView's names; the projects'
    // WeChat stays as it was.
    let assistant = WeixinDestination::Assistant;
    assert!(server.set_weixin_enabled(assistant, true).await.is_err());
    let qr = server.start_weixin_bind().await.unwrap();
    assert_eq!(
        server
            .poll_weixin_bind(assistant, &qr.qrcode)
            .await
            .unwrap(),
        WeixinScan::Waiting
    );
    assert_eq!(
        server
            .poll_weixin_bind(assistant, &qr.qrcode)
            .await
            .unwrap(),
        WeixinScan::Confirmed
    );
    assert_eq!(
        secrets.get("assistant_weixin_bot_token").as_deref(),
        Some("test-bot-token")
    );
    assert!(server.weixin_status(assistant).await.bound);
    assert!(
        !server
            .weixin_status(WeixinDestination::Projects)
            .await
            .bound
    );
    // One bot serves one of the two.
    let refused = server
        .poll_weixin_bind(WeixinDestination::Projects, &qr.qrcode)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("research assistant"), "{refused}");
    assert!(
        server
            .set_weixin_enabled(assistant, true)
            .await
            .unwrap()
            .enabled
    );

    let mut seen = 0;
    say(&fake, "owner-1", "/help");
    wait_sent(&fake, &mut seen, |text| text.contains("这里是科研助理")).await;
    // The projects' routing words go nowhere here.
    say(&fake, "owner-1", "/project 1");
    wait_sent(&fake, &mut seen, |text| text.contains("固定连接科研助理")).await;
    say(&fake, "owner-1", "yes");
    wait_sent(&fake, &mut seen, |text| text.contains("没有待审批")).await;
    // A message is a turn of the assistant's conversation.
    say(&fake, "owner-1", "how are the projects going?");
    let answer = wait_sent(&fake, &mut seen, |text| text.contains("Saved result")).await;
    assert_eq!(answer, "Saved result");
    let (_, frame) = server.open_assistant().await.unwrap();
    assert!(texts_of(&server, &frame)
        .await
        .iter()
        .any(|text| text == "how are the projects going?"));
    say(&fake, "owner-1", "/model");
    wait_sent(&fake, &mut seen, |text| text.contains("助理当前模型")).await;
    say(&fake, "owner-1", "full");
    wait_sent(&fake, &mut seen, |text| {
        text.contains("已开启助理会话完全权限")
    })
    .await;
    assert!(server.stored_options(&frame).await.unwrap().full_access);
    say(&fake, "owner-1", "full off");
    wait_sent(&fake, &mut seen, |text| text.contains("已关闭")).await;
    assert!(!server.stored_options(&frame).await.unwrap().full_access);
    say(&fake, "owner-1", "/mandates");
    wait_sent(&fake, &mut seen, |text| text.contains("研究职责")).await;

    // A notice of the assistant's goes out with the owner's next message.
    server
        .announce("职责需要你 / A mandate needs you")
        .await
        .unwrap();
    let mut notices = seen;
    say(&fake, "owner-1", "/status");
    wait_sent(&fake, &mut seen, |text| {
        text.contains("科研助理微信：已接入")
    })
    .await;
    wait_sent(&fake, &mut notices, |text| text.starts_with("职责需要你")).await;
    assert_eq!(
        server.weixin_status(assistant).await.state,
        ChannelState::Running
    );

    let status = server.unbind_weixin(assistant).await.unwrap();
    assert!(!status.bound && !status.enabled);
    assert_eq!(
        secrets.get("assistant_weixin_bot_token").as_deref(),
        Some("")
    );
    server.shutdown().await;
}

/// Feishu's REST, accounts and long-connection services, on loopback.
#[derive(Default)]
struct FakeFeishu {
    base: String,
    socket: String,
    polls: usize,
    events: Vec<Vec<u8>>,
    acks: usize,
    texts: Vec<String>,
    cards: Vec<String>,
    closed: usize,
}

async fn fake_feishu() -> Arc<std::sync::Mutex<FakeFeishu>> {
    use axum::{
        extract::State,
        routing::{get, patch, post, put},
        Form, Json, Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    type Shared = Arc<std::sync::Mutex<FakeFeishu>>;
    let fake: Shared = Default::default();
    let ok = || Json(json!({"code": 0}));
    let app = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post(|| async {
                Json(json!({"code": 0, "tenant_access_token": "t-test", "expire": 7200}))
            }),
        )
        .route(
            "/open-apis/bot/v3/info",
            get(|| async { Json(json!({"code": 0, "bot": {"open_id": "ou_bot"}})) }),
        )
        .route(
            "/callback/ws/endpoint",
            post(|State(fake): State<Shared>| async move {
                let url = format!("{}/ws?service_id=7", fake.lock().unwrap().socket);
                Json(
                    json!({"code": 0, "data": {"URL": url, "ClientConfig": {"PingInterval": 120}}}),
                )
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post(
                |State(fake): State<Shared>, Json(body): Json<serde_json::Value>| async move {
                    if body["msg_type"] == "text" {
                        let content: serde_json::Value =
                            serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
                        fake.lock()
                            .unwrap()
                            .texts
                            .push(content["text"].as_str().unwrap().to_owned());
                    }
                    Json(json!({"code": 0, "data": {"message_id": "om_1"}}))
                },
            ),
        )
        .route(
            "/open-apis/cardkit/v1/cards",
            post(|| async { Json(json!({"code": 0, "data": {"card_id": "card-1"}})) }),
        )
        .route(
            "/open-apis/cardkit/v1/cards/{card}/elements/md/content",
            put(
                move |State(fake): State<Shared>, Json(body): Json<serde_json::Value>| async move {
                    fake.lock()
                        .unwrap()
                        .cards
                        .push(body["content"].as_str().unwrap_or_default().to_owned());
                    ok()
                },
            ),
        )
        .route(
            "/open-apis/cardkit/v1/cards/{card}/settings",
            patch(move |State(fake): State<Shared>| async move {
                fake.lock().unwrap().closed += 1;
                ok()
            }),
        )
        .route(
            "/oauth/v1/app/registration",
            post(
                |State(fake): State<Shared>,
                 Form(form): Form<std::collections::HashMap<String, String>>| async move {
                    if form.get("action").map(String::as_str) == Some("begin") {
                        return Json(json!({
                            "device_code": "dc-1",
                            "verification_uri_complete": "https://example.test/verify",
                            "interval": 1,
                            "expire_in": 600,
                        }));
                    }
                    let mut fake = fake.lock().unwrap();
                    fake.polls += 1;
                    Json(if fake.polls == 1 {
                        json!({"error": "authorization_pending"})
                    } else {
                        json!({
                            "client_id": "cli_test",
                            "client_secret": "test-feishu-secret",
                            "user_info": {"open_id": "ou_owner", "tenant_brand": "feishu"},
                        })
                    })
                },
            ),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sockets = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    {
        let mut fake = fake.lock().unwrap();
        fake.base = format!("http://{}", listener.local_addr().unwrap());
        fake.socket = format!("ws://{}", sockets.local_addr().unwrap());
    }
    tokio::spawn(async move { axum::serve(listener, app).await });
    let shared = fake.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = sockets.accept().await {
            let fake = shared.clone();
            tokio::spawn(async move {
                let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut sink, mut stream) = socket.split();
                loop {
                    let queued: Vec<Vec<u8>> = fake.lock().unwrap().events.drain(..).collect();
                    for (at, payload) in queued.into_iter().enumerate() {
                        let frame = crate::channels::pbbp2::Frame {
                            seqid: at as u64 + 1,
                            service: 7,
                            method: 1,
                            headers: vec![("type".into(), "event".into())],
                            payload,
                            ..Default::default()
                        };
                        let bytes = crate::channels::pbbp2::encode(&frame);
                        if sink.send(WsMessage::Binary(bytes.into())).await.is_err() {
                            return;
                        }
                    }
                    match tokio::time::timeout(std::time::Duration::from_millis(50), stream.next())
                        .await
                    {
                        Ok(Some(Ok(WsMessage::Binary(data)))) => {
                            let frame = crate::channels::pbbp2::decode(&data).unwrap();
                            if frame.method == 1 && frame.payload == b"{\"code\":200}" {
                                fake.lock().unwrap().acks += 1;
                            }
                        }
                        Ok(None) | Ok(Some(Err(_))) => return,
                        _ => {}
                    }
                }
            });
        }
    });
    fake
}

fn feishu_event(id: &str, sender: &str, text: &str) -> Vec<u8> {
    json!({
        "header": {"event_type": "im.message.receive_v1", "event_id": id},
        "event": {
            "sender": {"sender_id": {"open_id": sender}},
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "text",
                "content": json!({"text": text}).to_string(),
            },
        },
    })
    .to_string()
    .into_bytes()
}

async fn until<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Some(found) = ready() {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

#[tokio::test]
async fn feishu_makes_its_app_by_scan_and_answers_only_its_owner() {
    use crate::channels::{feishu::Hosts, feishu_registration::Accounts};
    use desktop::{ChannelState, FeishuBindPoll, SecretStore};
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("feishu")));
    let fake = fake_feishu().await;
    let base = fake.lock().unwrap().base.clone();
    *server.0.channels.feishu_hosts.lock().unwrap() = Hosts {
        feishu: base.clone(),
        lark: base.clone(),
        accounts: Accounts {
            feishu: base.clone(),
            lark: base,
        },
    };
    assert!(!server.feishu_status().await.bound);
    assert!(server.set_feishu(true, false, "", None).await.is_err());

    // Scanning makes the app; whoever scanned is its owner.
    let bind = server.start_feishu_bind(false).await.unwrap();
    assert_eq!(
        bind.matrix.dark.len(),
        bind.matrix.width * bind.matrix.width
    );
    let mut app = None;
    for _ in 0..40 {
        match server.poll_feishu_bind(&bind.flow).await.unwrap() {
            FeishuBindPoll::Confirmed { app_id } => {
                app = Some(app_id);
                break;
            }
            FeishuBindPoll::Pending { .. } => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(app.as_deref(), Some("cli_test"));
    assert!(server.poll_feishu_bind(&bind.flow).await.is_err());
    assert_eq!(
        secrets.get("feishu_app_secret").as_deref(),
        Some("test-feishu-secret")
    );
    let status = server.feishu_status().await;
    assert!(status.bound && status.owner == "ou_owner" && !status.enabled);

    let folder = temp.path().join("feishu project");
    std::fs::create_dir(&folder).unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    assert!(
        server
            .set_feishu(true, false, "cli_test", None)
            .await
            .unwrap()
            .enabled
    );
    let connected = || {
        let server = server.clone();
        async move { server.feishu_status().await.state == ChannelState::Running }
    };
    for _ in 0..200 {
        if connected().await {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(connected().await);

    let push = |event: Vec<u8>| fake.lock().unwrap().events.push(event);
    let said = |wanted: &'static str| {
        let fake = fake.clone();
        move || {
            fake.lock()
                .unwrap()
                .texts
                .iter()
                .any(|text| text.contains(wanted))
                .then_some(())
        }
    };
    push(feishu_event(
        "ev-1",
        "ou_owner",
        &format!("/project {project}"),
    ));
    // Delivered twice: answered once.
    push(feishu_event(
        "ev-1",
        "ou_owner",
        &format!("/project {project}"),
    ));
    until("the switch", said("IM 目标已切换到项目")).await;
    push(feishu_event("ev-2", "ou_stranger", "hello"));
    until("the refusal", said("只响应已绑定的所有者")).await;

    // A turn shows on a card that ends as the answer.
    push(feishu_event("ev-3", "ou_owner", "what is the answer?"));
    until("the answer card", || {
        let fake = fake.lock().unwrap();
        (fake.closed == 1 && fake.cards.last().map(String::as_str) == Some("Saved result"))
            .then_some(())
    })
    .await;

    // With no owner, the first to write waits for the desktop to confirm.
    assert!(server.set_feishu_owner("").await.unwrap().owner.is_empty());
    push(feishu_event("ev-4", "ou_new", "hello"));
    until("the pairing note", said("尚未绑定所有者")).await;
    let status = server.feishu_status().await;
    assert_eq!(status.pending_owner, "ou_new");
    let status = server.confirm_feishu_owner().await.unwrap();
    assert!(status.owner == "ou_new" && status.pending_owner.is_empty());
    assert!(server.confirm_feishu_owner().await.is_err());

    {
        let fake = fake.lock().unwrap();
        assert_eq!(
            fake.texts
                .iter()
                .filter(|text| text.contains("IM 目标已切换"))
                .count(),
            1
        );
        // Every frame was acknowledged, the repeated one too.
        assert_eq!(fake.acks, 5);
    }
    let status = server.unbind_feishu().await.unwrap();
    assert!(!status.bound && !status.enabled && status.owner.is_empty());
    assert_eq!(status.state, ChannelState::Stopped);
    assert_eq!(secrets.get("feishu_app_secret").as_deref(), Some(""));
    server.shutdown().await;
}

/// A relay hosting a lab, on loopback.
async fn lab_relay(root: &Path) -> String {
    let relay = wisp_sync::FileRelay::open(root.join("relay"))
        .await
        .unwrap();
    let state = wisp_sync::RelayHttpState::new(relay, "test-relay-token")
        .unwrap()
        .with_lab(
            wisp_sync::Lab::open(root.join("relay").join("lab"))
                .await
                .unwrap(),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, wisp_sync::relay_router(state)).await });
    url
}

/// Looks in a computer's lab mailbox once and waits for the turns it
/// started.
async fn deliver_lab_mail(server: &Server) {
    let identity = server.lab_identity().unwrap();
    let client = server.lab_client(&identity).unwrap();
    let seen = crate::channels::lab_mail::SeenMails::default();
    for turn in server
        .poll_lab_mail(&identity, &client, &seen)
        .await
        .unwrap()
    {
        turn.await.unwrap();
    }
}

async fn texts_of(server: &Server, frame: &str) -> Vec<String> {
    server
        .0
        .store
        .load_messages(frame)
        .await
        .unwrap()
        .iter()
        .map(|message| message.content.as_text())
        .collect()
}

#[tokio::test]
async fn lab_members_join_by_invite_and_their_agents_mail_each_other() {
    let temp = tempfile::tempdir().unwrap();
    let url = lab_relay(temp.path()).await;
    let computer = |name: &str| {
        let root = temp.path().join(name);
        async move {
            Server::open_with(
                config(&root),
                models(Arc::default(), false),
                Arc::new(desktop::MemorySecrets::default()),
            )
            .await
            .unwrap()
        }
    };
    let ada = computer("ada").await;
    let lin = computer("lin").await;
    assert!(!ada.commands().iter().any(|command| command.contains("lab")));
    assert_eq!(ada.lab_status().await.state, "none");
    // A name is needed, and a token.
    assert!(ada
        .join_lab(&url, None, "Ada", "Genomics", "")
        .await
        .is_err());
    assert!(ada
        .join_lab(&url, Some("test-relay-token".into()), " ", "", "")
        .await
        .is_err());

    // The first computer founds the lab and leads it.
    let status = ada
        .join_lab(&url, Some("test-relay-token".into()), "Ada", "Genomics", "")
        .await
        .unwrap();
    assert_eq!(status.state, "active");
    assert!(status.leader && status.lab_name == "Genomics");
    // The mailbox is looked at by hand below.
    ada.0.channels.lab.stop();
    assert!(ada
        .join_lab(&url, Some("test-relay-token".into()), "Ada", "", "")
        .await
        .is_err());

    // Another joins with an invite and waits for the leader.
    let invite = ada.create_lab_invite().await.unwrap();
    assert!(lin
        .join_lab(
            "",
            Some("test-relay-token".into()),
            "Lin",
            "",
            "not-an-invite"
        )
        .await
        .is_err());
    let status = lin
        .join_lab("", Some("test-relay-token".into()), "Lin", "", &invite)
        .await
        .unwrap();
    lin.0.channels.lab.stop();
    assert_eq!(status.state, "pending");
    assert!(!status.leader);
    let waiting = ada
        .lab_status()
        .await
        .members
        .into_iter()
        .find(|member| member.pending)
        .unwrap();
    assert_eq!(waiting.name, "Lin");
    // Only the leader approves.
    assert!(lin.approve_lab_member(&waiting.id).await.is_err());
    ada.approve_lab_member(&waiting.id).await.unwrap();
    let status = lin.lab_status().await;
    assert_eq!(status.state, "active");
    assert_eq!(status.members.len(), 2);

    // Lin's computer answers lab mail from one of its projects.
    let lin_folder = temp.path().join("lin-project");
    std::fs::create_dir(&lin_folder).unwrap();
    let lin_project = lin.register_local_project(&lin_folder).await.unwrap();
    assert!(lin.set_lab_inbox_project("missing").await.is_err());
    lin.set_lab_inbox_project(&lin_project).await.unwrap();
    assert_eq!(lin.lab_status().await.inbox_project_id, lin_project);

    // Ada's agent asks; Lin's agent answers in a conversation kept for Ada,
    // and the answer comes back to the conversation that asked.
    let ada_folder = temp.path().join("ada-project");
    std::fs::create_dir(&ada_folder).unwrap();
    let ada_project = ada.register_local_project(&ada_folder).await.unwrap();
    let asking = ada.create_conversation(&ada_project).await.unwrap();
    let sent = ada
        .lab_send(
            &asking,
            &json!({"to": "lin", "message": "Which reference genome?"}),
            &ada_folder,
        )
        .await
        .unwrap();
    assert!(sent.starts_with("Sent to Lin."));
    // Only the project's own files leave the computer.
    assert!(ada
        .lab_send(
            &asking,
            &json!({"to": "Lin", "message": "see this", "files": ["../secret.txt"]}),
            &ada_folder,
        )
        .await
        .is_err());
    deliver_lab_mail(&lin).await;
    let kept = lin
        .0
        .store
        .list_sessions_page(&lin_project, None, 10)
        .await
        .unwrap();
    let (lab_frame, title, ..) = kept.first().unwrap().clone();
    assert_eq!(title, "Lab: Ada");
    let asked = texts_of(&lin, &lab_frame).await;
    assert!(asked.iter().any(|text| text.starts_with(
        "[Lab message from Ada (lab leader)]
Which reference genome?"
    )));
    deliver_lab_mail(&ada).await;
    let answered = texts_of(&ada, &asking).await;
    assert!(
        answered.iter().any(|text| text.starts_with(
            "[Lab reply from Lin]
Saved result"
        )),
        "{answered:?}"
    );

    // With no project for lab mail, a question is told so at once.
    lin.set_lab_inbox_project("").await.unwrap();
    ada.lab_send(
        &asking,
        &json!({"to": "Lin", "message": "And the annotation?"}),
        &ada_folder,
    )
    .await
    .unwrap();
    deliver_lab_mail(&lin).await;
    deliver_lab_mail(&ada).await;
    assert!(texts_of(&ada, &asking)
        .await
        .iter()
        .any(|text| text.contains("does not answer lab messages")));

    // Removed by the leader, Lin's computer can only forget the lab.
    ada.remove_lab_member(&waiting.id).await.unwrap();
    assert_eq!(lin.lab_status().await.state, "removed");
    lin.leave_lab(true).await.unwrap();
    assert_eq!(lin.lab_status().await.state, "none");
    assert!(!lin.is_lab_member().await);
    ada.shutdown().await;
    lin.shutdown().await;
}

#[tokio::test]
async fn the_desk_display_reads_the_agent_and_the_pet_only_with_its_token() {
    use desktop::{DeviceBridgeState, SecretStore};
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("device")));
    // Off, and only one concrete address.
    let bridge = server.device_bridge().await;
    assert!(!bridge.enabled && bridge.state == DeviceBridgeState::Stopped);
    assert!(server
        .set_device_bridge(true, "0.0.0.0", 18766)
        .await
        .is_err());
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bridge = server
        .set_device_bridge(true, "127.0.0.1", u32::from(port))
        .await
        .unwrap();
    assert_eq!(bridge.state, DeviceBridgeState::Listening);
    let url = bridge.url.clone().unwrap();
    assert!(bridge.has_token);
    let token = server.device_bridge_token().unwrap();
    // The WebView's keychain name.
    assert_eq!(secrets.get("sticks3_device_bridge_token").unwrap(), token);

    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |path: &str, token: &str| {
        http.get(format!("{url}{path}"))
            .header("x-wisp-device-token", token)
            .send()
    };
    let health: serde_json::Value = http
        .get(format!("{url}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["service"], "wisp-device-bridge");
    assert_eq!(
        get("/state", "wrong").await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let state: serde_json::Value = get("/state", &token).await.unwrap().json().await.unwrap();
    assert_eq!(state["type"], "pet_state");
    assert_eq!(state["state"], "idle");

    // A turn ends: the device shows it done until it is put away.
    let folder = temp.path().join("desk");
    std::fs::create_dir(&folder).unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    let session = server.create_conversation(&project).await.unwrap();
    server
        .send(
            &project,
            dto::SendRequest {
                session_id: session.clone(),
                request_id: "r1".into(),
                message: "hello".into(),
                resume: false,
                resume_after: None,
                attachments: vec![],
                references: vec![],
                acp_agent_id: None,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while server.session(&session).live.lock().unwrap().running {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let state: serde_json::Value = get("/state", &token).await.unwrap().json().await.unwrap();
    assert_eq!(state["state"], "done", "{state}");
    assert_eq!(state["sessionId"], session.as_str());
    let post = |body: serde_json::Value, token: &str| {
        http.post(format!("{url}/action"))
            .header("x-wisp-device-token", token)
            .json(&body)
            .send()
    };
    let acknowledged: serde_json::Value =
        post(json!({"id": "a1", "action": "acknowledge"}), &token)
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(acknowledged["acknowledged"], true);
    let state: serde_json::Value = get("/state", &token).await.unwrap().json().await.unwrap();
    assert_eq!(state["state"], "idle");

    // Asking for a conversation hands it to the window.
    let waiting = tokio::spawn({
        let server = server.clone();
        async move { server.next_device_focus().await }
    });
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let focused = post(
        json!({"id": "a2", "action": "focus_session", "sessionId": session}),
        &token,
    )
    .await
    .unwrap();
    assert!(focused.status().is_success());
    assert_eq!(waiting.await.unwrap(), (project.clone(), session.clone()));
    assert_eq!(
        post(
            json!({"id": "a3", "action": "focus_session", "sessionId": "gone"}),
            &token
        )
        .await
        .unwrap()
        .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(json!({"id": "a4", "action": "run_shell"}), &token)
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let actions: serde_json::Value = get("/actions", &token).await.unwrap().json().await.unwrap();
    assert_eq!(actions["actions"].as_array().unwrap().len(), 4);

    // The pet is drawn frame by frame once there is one.
    let manifest: serde_json::Value = get("/pet/manifest", &token)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(manifest["enabled"], false);
    let pet = temp.path().join("pet");
    std::fs::create_dir(&pet).unwrap();
    let sheet = image::RgbaImage::from_pixel(1536, 2288, image::Rgba([200, 80, 40, 255]));
    sheet.save(pet.join("sheet.png")).unwrap();
    std::fs::write(
        pet.join("pet.json"),
        r#"{"id":"mochi","displayName":"Mochi","spriteVersionNumber":2,"spritesheetPath":"sheet.png"}"#,
    )
    .unwrap();
    crate::pet::save(&server.0.store, true, pet.to_str().unwrap())
        .await
        .unwrap();
    let manifest: serde_json::Value = get("/pet/manifest", &token)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(manifest["enabled"], true);
    assert_eq!(manifest["id"], "mochi");
    assert_eq!(manifest["frameCounts"]["idle"], 7);
    let revision = manifest["revision"].as_str().unwrap().to_owned();
    let frame = get(
        &format!("/pet/frame?revision={revision}&state=idle&frame=0"),
        &token,
    )
    .await
    .unwrap();
    assert_eq!(frame.headers()["content-type"], "image/png");
    let png = frame.bytes().await.unwrap();
    let drawn = image::load_from_memory(&png).unwrap();
    assert_eq!((drawn.width(), drawn.height()), (120, 130));
    assert_eq!(
        get("/pet/frame?revision=old&state=idle&frame=0", &token)
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );

    // A new token locks the old one out; off forgets it.
    let fresh = server.rotate_device_bridge_token().unwrap();
    assert_ne!(fresh, token);
    assert_eq!(
        get("/state", &token).await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(get("/state", &fresh).await.unwrap().status().is_success());
    let bridge = server
        .set_device_bridge(false, "127.0.0.1", u32::from(port))
        .await
        .unwrap();
    assert!(!bridge.enabled && !bridge.has_token);
    assert_eq!(bridge.state, DeviceBridgeState::Stopped);
    server.shutdown().await;
}

#[tokio::test]
async fn projects_sync_between_devices_and_a_lab_shares_its_knowledge_base() {
    use desktop::SecretStore;
    let temp = tempfile::tempdir().unwrap();
    let url = lab_relay(temp.path()).await;
    let computer = |name: &str| {
        let root = temp.path().join(name);
        async move {
            let secrets = Arc::new(desktop::MemorySecrets::default());
            let server = Server::open_with(
                config(&root),
                models(Arc::default(), false),
                secrets.clone(),
            )
            .await
            .unwrap();
            (server, secrets)
        }
    };
    let (ada, ada_secrets) = computer("ada").await;
    let (lin, _) = computer("lin").await;
    assert!(!ada
        .commands()
        .iter()
        .any(|command| command.contains("sync")));

    // Nothing is set up: a sync says where to set it up.
    let folder = temp.path().join("study");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "first draft").unwrap();
    let project = ada.register_local_project(&folder).await.unwrap();
    assert!(ada.sync_project(&project).await.is_err());
    assert!(ada.project_sync(&project).await.unwrap().is_none());
    assert!(ada
        .set_sync_settings("relay", "http://example.com", None, "")
        .await
        .is_err());
    assert!(ada
        .set_sync_settings("folder", "", None, "relative/path")
        .await
        .is_err());
    for server in [&ada, &lin] {
        let settings = server
            .set_sync_settings("relay", &url, Some("test-relay-token".into()), "")
            .await
            .unwrap();
        assert!(settings.has_token && settings.backend == "relay");
    }

    // The first sync publishes; a second has nothing to do.
    let pushed = ada.sync_project(&project).await.unwrap();
    assert_eq!(
        (pushed.status.as_str(), pushed.direction.as_str()),
        ("synced", "push")
    );
    assert!(ada.sync_project(&project).await.unwrap().direction == "none");
    assert!(ada.project_sync(&project).await.unwrap().unwrap().synced);
    // The project key is in the keychain, never in the store.
    assert!(ada_secrets
        .get(&format!("sync_project_key:{project}"))
        .is_some_and(|key| !key.is_empty()));
    let code = ada.project_sync_code(&project).await.unwrap();
    assert!(code.starts_with("wisp-sync:"));

    // The other device joins with the code and gets the files.
    let parent = temp.path().join("lin-projects");
    std::fs::create_dir(&parent).unwrap();
    assert!(lin
        .join_synced_project("not a code", &parent)
        .await
        .is_err());
    let joined = lin.join_synced_project(&code, &parent).await.unwrap();
    assert_eq!(joined, project);
    let (_, lin_root) = lin.0.store.get_project(&joined).await.unwrap().unwrap();
    let lin_root = std::path::PathBuf::from(lin_root);
    assert_eq!(
        std::fs::read_to_string(lin_root.join("notes.txt")).unwrap(),
        "first draft"
    );
    assert!(lin.join_synced_project(&code, &parent).await.is_err());

    // A change on one device reaches the other.
    std::fs::write(lin_root.join("notes.txt"), "second draft").unwrap();
    assert_eq!(lin.sync_project(&joined).await.unwrap().direction, "push");
    assert_eq!(ada.sync_project(&project).await.unwrap().direction, "pull");
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "second draft"
    );

    // Both changed: nothing is overwritten until one side is chosen.
    std::fs::write(folder.join("notes.txt"), "ada's edit").unwrap();
    std::fs::write(lin_root.join("notes.txt"), "lin's edit").unwrap();
    lin.sync_project(&joined).await.unwrap();
    let conflict = ada.sync_project(&project).await.unwrap_err().to_string();
    assert!(conflict.contains("conflict"), "{conflict}");
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "ada's edit"
    );
    let resolved = ada.resolve_project_sync(&project, true).await.unwrap();
    assert_eq!(resolved.direction, "pull");
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "lin's edit"
    );

    // A lab on the same relay publishes a synced project as its knowledge
    // base; a member downloads it and their agents are pointed at it.
    ada.join_lab(&url, Some("test-relay-token".into()), "Ada", "Genomics", "")
        .await
        .unwrap();
    ada.0.channels.lab.stop();
    let invite = ada.create_lab_invite().await.unwrap();
    lin.join_lab("", Some("test-relay-token".into()), "Lin", "", &invite)
        .await
        .unwrap();
    lin.0.channels.lab.stop();
    let waiting = ada
        .lab_status()
        .await
        .members
        .into_iter()
        .find(|member| member.pending)
        .unwrap();
    ada.approve_lab_member(&waiting.id).await.unwrap();
    let protocols = temp.path().join("protocols");
    std::fs::create_dir(&protocols).unwrap();
    std::fs::write(protocols.join("pcr.md"), "Anneal at 58 °C.").unwrap();
    let base = ada.register_local_project(&protocols).await.unwrap();
    // Not synchronized yet: there is no device code to publish.
    assert!(ada.publish_lab_knowledge_base(&base).await.is_err());
    ada.sync_project(&base).await.unwrap();
    ada.publish_lab_knowledge_base(&base).await.unwrap();
    let shown = lin.lab_status().await.knowledge_base.unwrap();
    assert!(!shown.local);
    let kb = lin.join_lab_knowledge_base(&parent).await.unwrap();
    assert_eq!(kb, base);
    assert!(lin.lab_status().await.knowledge_base.unwrap().local);
    let root = lin.lab_knowledge_base_root().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&root).join("pcr.md")).unwrap(),
        "Anneal at 58 °C."
    );
    assert!(crate::channels::lab_mail::turn_context(Some(&root)).contains(&root));
    ada.shutdown().await;
    lin.shutdown().await;
}

fn send_text(session: &str, id: &str, text: &str) -> dto::SendRequest {
    dto::SendRequest {
        session_id: session.into(),
        request_id: id.into(),
        message: text.into(),
        resume: false,
        resume_after: None,
        attachments: vec![],
        references: vec![],
        acp_agent_id: None,
    }
}

async fn turn_done(server: &Server, session: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while server.session(session).live.lock().unwrap().running {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the turn must end");
}

#[tokio::test]
async fn explorations_branch_a_turn_into_copies_and_keep_one() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("exploration")));
    let folder = temp.path().join("study");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "v1").unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    let source = server.create_conversation(&project).await.unwrap();
    server
        .send(&project, send_text(&source, "r1", "what now?"))
        .await
        .unwrap();
    turn_done(&server, &source).await;
    assert!(server.explorations(&project).await.unwrap().is_empty());

    // A first candidate freezes the source and copies the project.
    let first = server
        .start_exploration(&project, &source, None, "Try A")
        .await
        .unwrap();
    let rows = server.explorations(&project).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_frame_id, source);
    assert_eq!(rows[0].exploration.frame_id, first.session);
    let copy = std::path::PathBuf::from(&rows[0].exploration.workspace_dir);
    assert_eq!(
        std::fs::read_to_string(copy.join("notes.txt")).unwrap(),
        "v1"
    );
    let state = server.exploration_state(&project, &source).await.unwrap();
    assert!(state.source_frozen);
    // The source is frozen; its history is the exploration's too.
    let refused = server
        .send(&project, send_text(&source, "r2", "more"))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("exploration_mainline_frozen"), "{refused}");
    assert!(server.delete_conversation(&project, &source).await.is_err());
    let cloned = texts_of(&server, &first.session).await;
    assert!(cloned.iter().any(|text| text == "what now?"));
    // The exploration's turns work in its copy.
    server
        .send(&project, send_text(&first.session, "e1", "try it"))
        .await
        .unwrap();
    turn_done(&server, &first.session).await;
    assert!(texts_of(&server, &first.session)
        .await
        .iter()
        .any(|text| text == "try it"));
    std::fs::write(copy.join("notes.txt"), "v2").unwrap();
    std::fs::write(copy.join("new.txt"), "found").unwrap();
    // Mainline files are untouched meanwhile.
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "v1"
    );
    // An exploration's conversation is kept or discarded, not deleted.
    assert!(server
        .delete_conversation(&project, &first.session)
        .await
        .is_err());

    // A second candidate shares the checkpoint; another conversation cannot
    // start a round of its own.
    let second = server
        .start_exploration(&project, &source, None, "Try B")
        .await
        .unwrap();
    let rows = server.explorations(&project).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].exploration.checkpoint_id,
        rows[1].exploration.checkpoint_id
    );
    let other = server.create_conversation(&project).await.unwrap();
    server
        .send(&project, send_text(&other, "o1", "hello"))
        .await
        .unwrap();
    turn_done(&server, &other).await;
    let busy = server
        .start_exploration(&project, &other, None, "Elsewhere")
        .await
        .unwrap_err()
        .to_string();
    assert!(busy.contains("exploration_round_active"), "{busy}");

    // Keeping one brings its files in and ends the round.
    let preview = server
        .preview_exploration(&project, &first.id)
        .await
        .unwrap();
    assert!(preview.eligible, "{:?}", preview.blockers);
    let changed: Vec<(&str, &str)> = preview
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();
    assert!(changed.contains(&("notes.txt", "modified")));
    assert!(changed.contains(&("new.txt", "added")));
    assert!(server
        .promote_exploration(&project, &first.id, "stale")
        .await
        .is_err());
    let mainline = server
        .promote_exploration(&project, &first.id, &preview.guard)
        .await
        .unwrap();
    assert_eq!(mainline, source);
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "v2"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("new.txt")).unwrap(),
        "found"
    );
    assert!(server.explorations(&project).await.unwrap().is_empty());
    assert!(!copy.exists());
    let _ = second;
    // The kept exploration's turn continues the source conversation.
    assert!(texts_of(&server, &source)
        .await
        .iter()
        .any(|text| text == "try it"));
    server
        .send(&project, send_text(&source, "r3", "and next"))
        .await
        .unwrap();
    turn_done(&server, &source).await;

    // Discarding the only candidate, and abandoning a round, keep the
    // mainline as it is.
    let third = server
        .start_exploration(&project, &source, None, "Try C")
        .await
        .unwrap();
    server
        .discard_exploration(&project, &third.id)
        .await
        .unwrap();
    assert!(server.explorations(&project).await.unwrap().is_empty());
    assert!(server
        .abandon_exploration_round(&project, &source)
        .await
        .is_err());
    let fourth = server
        .start_exploration(&project, &source, None, "Try D")
        .await
        .unwrap();
    let rows = server.explorations(&project).await.unwrap();
    std::fs::write(
        std::path::Path::new(&rows[0].exploration.workspace_dir).join("notes.txt"),
        "never kept",
    )
    .unwrap();
    server
        .abandon_exploration_round(&project, &source)
        .await
        .unwrap();
    assert!(server.explorations(&project).await.unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "v2"
    );
    let _ = fourth;
    server.shutdown().await;
}

#[tokio::test]
async fn a_subagent_works_in_its_own_conversation_and_reports_back() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(DispatchingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("reads");
    std::fs::create_dir(&folder).unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    // Dispatching asks first, like any tool that sets work going, unless
    // the conversation has full access.
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            full_access: true,
            ..defaults
        })
        .await
        .unwrap();
    let parent = server.create_conversation(&project).await.unwrap();
    server
        .send(&project, send_text(&parent, "p1", "count the reads please"))
        .await
        .unwrap();
    turn_done(&server, &parent).await;

    // The subagent has its own conversation, under its parent.
    let list = server.conversations(&project, "", false).await.unwrap();
    let (child, owner) = list.dispatched.iter().next().unwrap();
    assert_eq!(owner, &parent);
    let child = child.clone();
    let titled = list.rows.iter().find(|row| row.id == child).unwrap();
    assert_eq!(titled.title, "Counting");
    turn_done(&server, &child).await;
    let asked = texts_of(&server, &child).await;
    assert!(asked.iter().any(
        |text| text.starts_with("[From the parent conversation's agent]")
            && text.ends_with("Count the reads in data.fastq")
    ));
    // It is watched, not written to.
    let refused = server
        .send(&project, send_text(&child, "c1", "hello"))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("only takes instructions"), "{refused}");

    // Its outcome comes back as a report, and the parent handles it.
    let handled = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let texts = texts_of(&server, &parent).await;
            if texts
                .iter()
                .any(|text| text == "Handled the subagent's report.")
            {
                return texts;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the parent must handle the report");
    assert!(handled
        .iter()
        .any(|text| text == "Report: the subagent counted 42 reads."));
    // The resumed turn added no message of the user's.
    let users = server
        .0
        .store
        .load_messages(&parent)
        .await
        .unwrap()
        .iter()
        .filter(|message| message.role == wisp_llm::Role::User)
        .count();
    assert_eq!(users, 1);
    let status = server
        .subagent_status(&project, &parent, None)
        .await
        .unwrap();
    assert!(status.contains(&child) && status.contains("Counting"));
    assert!(server
        .subagent_status(&project, &child, Some(&parent))
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_delegated_method_search_waits_for_start_then_runs_and_reports_back() {
    use crate::method_search::control::testing::{prepare, ScriptedEvaluator};
    let temp = tempfile::tempdir().unwrap();
    let spec = Arc::new(std::sync::Mutex::new(String::new()));
    let proposals = Arc::new(AtomicUsize::new(0));
    let factory: ModelFactory = {
        let (spec, proposals) = (spec.clone(), proposals.clone());
        Arc::new(move |_| {
            Box::new(SearchingModel {
                spec: spec.clone(),
                proposals: proposals.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    server.set_method_search_evaluator(Arc::new(ScriptedEvaluator));
    let root = std::path::PathBuf::from(server.project_root(PROJECT).await.unwrap());
    let parent = create(&server).await;
    // The contract the preparing task hands back, frozen beforehand.
    let prepared = prepare(&server.0.store, PROJECT, &root, &parent, 3).await;
    *spec.lock().unwrap() = prepared.method_search_spec_artifact_version_id.clone();

    send(&server, &parent, "search for a better fit")
        .await
        .unwrap();
    // A method search is always approved first.
    let pending = wait_for(&server, &parent, |s| !s.approvals.is_empty()).await;
    assert!(
        pending.approvals[0].message.contains("search"),
        "{}",
        pending.approvals[0].message
    );
    server
        .answer_approval(
            PROJECT,
            &parent,
            &pending.approvals[0].approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    turn_done(&server, &parent).await;

    // The search waits in Draft, linked to its step, until it is started.
    let store = &server.0.store;
    let run_id = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let found = store
                .list_runs_by_project(PROJECT)
                .await
                .unwrap()
                .into_iter()
                .find(|run| run.kind == "method_search");
            if let Some(run) = found {
                return run.id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the batch must make the search's Run");
    let details = server.method_search_run(PROJECT, &run_id).await.unwrap();
    assert_eq!(details.run.status, wisp_store::RunStatus::Draft);
    assert!(details.activity.is_some());
    // Its step's driver holds it from the moment it can be seen, so Start
    // leaves the search to that driver instead of running a second one.
    assert!(crate::method_search::coordinator::method_search_is_active(
        &run_id
    ));
    assert_eq!(proposals.load(Ordering::SeqCst), 0);
    server.start_method_search(PROJECT, &run_id).await.unwrap();

    // It proposes and scores its candidates, promotes the best, and the
    // batch's result comes back to the conversation.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let delivered = texts_of(&server, &parent)
                .await
                .iter()
                .any(|text| text == "Search reported.");
            if delivered && !server.session(&parent).live.lock().unwrap().running {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the search must end and its result be delivered");
    let details = server.method_search_run(PROJECT, &run_id).await.unwrap();
    assert_eq!(details.run.status, wisp_store::RunStatus::Succeeded);
    assert!(proposals.load(Ordering::SeqCst) >= 1);
    assert!(!details.candidates.is_empty());
    assert!(!details.outputs.is_empty());
    // The card reads the details through the shared DTO.
    let card: wisp_dto::MethodSearchRunDetails =
        serde_json::from_value(serde_json::to_value(&details).unwrap()).unwrap();
    assert_eq!(card.run.status, "succeeded");
    assert_eq!(card.candidates.len(), details.candidates.len());
    assert_eq!(card.spec.target.symbol, "fit_model");
    // Its promoted files read back as text; nothing else does.
    let selected = details
        .outputs
        .iter()
        .find(|output| output.role == "selected_method")
        .unwrap();
    let source = server
        .method_search_output(PROJECT, &run_id, &selected.artifact_version_id)
        .await
        .unwrap();
    assert!(source.starts_with("def fit_model(rows):"));
    assert!(server
        .method_search_output(PROJECT, &run_id, &details.state.spec_artifact_version_id)
        .await
        .is_err());
    let batch = &server.delegated_batches(PROJECT, &parent).await.unwrap()[0];
    assert_eq!(batch.status, "succeeded");
    let search = batch.tasks.iter().find(|task| task.id == "search").unwrap();
    assert_eq!(search.status, "succeeded");
    server.shutdown().await;
}

#[tokio::test]
async fn an_isolated_task_works_in_a_worktree_and_its_changes_are_merged() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(IsolatingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // A clean Git project, so a task can have a worktree of its own.
    let root = std::path::PathBuf::from(server.project_root(PROJECT).await.unwrap());
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    git(&["init"]);
    git(&["config", "core.autocrlf", "false"]);
    std::fs::write(root.join(".gitignore"), ".wisp/\n").unwrap();
    std::fs::write(root.join("README.md"), "project\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=Wisp Test",
        "-c",
        "user.email=wisp-test@localhost",
        "commit",
        "-m",
        "base",
    ]);
    let before = git(&["rev-list", "--count", "HEAD"]);

    let parent = create(&server).await;
    send(&server, &parent, "write the note in isolation")
        .await
        .unwrap();
    // A task that changes the project is approved first.
    let pending = wait_for(&server, &parent, |s| !s.approvals.is_empty()).await;
    server
        .answer_approval(
            PROJECT,
            &parent,
            &pending.approvals[0].approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    turn_done(&server, &parent).await;
    let texts = texts_of(&server, &parent).await;
    assert!(texts.iter().any(|text| text == "Merged."), "{texts:?}");

    // The note was written in the worktree and cherry-picked into the
    // project as a commit of its own; the worktree is gone.
    assert_eq!(
        std::fs::read_to_string(root.join("note.md")).unwrap(),
        "written in a worktree\n"
    );
    assert_ne!(git(&["rev-list", "--count", "HEAD"]), before);
    let worktrees = server.0.config.data.join("agent-worktrees");
    assert!(std::fs::read_dir(&worktrees)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true));
    let batch = &server.delegated_batches(PROJECT, &parent).await.unwrap()[0];
    let full = server
        .delegated_result(
            &parent,
            &json!({"workflow_id": batch.id, "task_id": "note"}),
        )
        .await
        .unwrap();
    let evidence = full["response"]["evidence"].to_string();
    assert!(evidence.contains("workspace_merge"), "{evidence}");
    server.shutdown().await;
}

#[tokio::test]
async fn a_task_granted_delegation_runs_a_batch_of_its_own_one_level_down() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(NestingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let parent = create(&server).await;
    send(&server, &parent, "lead the count").await.unwrap();
    // Granting a task delegation is approved first, on the plan card.
    let pending = wait_for(&server, &parent, |s| !s.approvals.is_empty()).await;
    let card = &pending.approvals[0];
    assert!(
        card.message.contains("[ ] lead: Lead the read count"),
        "{}",
        card.message
    );
    server
        .answer_approval(
            PROJECT,
            &parent,
            &card.approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    turn_done(&server, &parent).await;
    let texts = texts_of(&server, &parent).await;
    assert!(texts.iter().any(|text| text == "Done: led 42"), "{texts:?}");

    let batches = server.delegated_batches(PROJECT, &parent).await.unwrap();
    assert_eq!(batches.len(), 1);
    let root = &batches[0];
    assert!(!root.nested);
    assert_eq!(root.status, "succeeded");
    assert_eq!(root.tasks[0].summary.as_deref(), Some("led: 42"));

    // The leading task's own batch is listed in its conversation, one
    // level down and named after it; its task could not delegate again.
    let lead = root.tasks[0].conversation.clone().unwrap();
    let own = server.delegated_batches(PROJECT, &lead).await.unwrap();
    assert_eq!(own.len(), 1);
    let nested = &own[0];
    assert!(nested.nested);
    assert_eq!(nested.status, "succeeded");
    assert_eq!(nested.tasks[0].id, "lead/count");
    assert_eq!(nested.tasks[0].summary.as_deref(), Some("42 reads"));
    let workflow = server
        .0
        .store
        .get_agent_workflow(&nested.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(workflow.depth, 1);
    assert_eq!(workflow.root_workflow_id, root.id);
    assert!(workflow.parent_attempt_id.is_some());
    // Its task's conversation is under the leading task's.
    let counting = nested.tasks[0].conversation.clone().unwrap();
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.dispatched.get(&counting), Some(&lead));
    // Only a root batch runs again on its own.
    assert!(server
        .retry_delegated_batch(&lead, &nested.id, &std::collections::HashMap::new())
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_delegated_batch_runs_its_tasks_in_order_and_returns_their_results() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(DelegatingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let parent = create(&server).await;
    send(&server, &parent, "count the reads please")
        .await
        .unwrap();
    turn_done(&server, &parent).await;
    let texts = texts_of(&server, &parent).await;
    assert!(
        texts.iter().any(|text| text == "Synthesized: total 42"),
        "{texts:?}"
    );

    // The batch and its tasks are kept, each task with its conversation.
    let batches = server.delegated_batches(PROJECT, &parent).await.unwrap();
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.goal, "Count and add up the reads");
    assert_eq!(batch.status, "succeeded");
    assert!(!batch.running());
    let ids: Vec<_> = batch.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["count", "sum"]);
    assert_eq!(batch.tasks[1].depends_on, ["count"]);
    assert!(batch.tasks.iter().all(|task| task.status == "succeeded"));
    // The dependent task was given what the first one found.
    assert_eq!(batch.tasks[0].summary.as_deref(), Some("42 reads"));
    assert_eq!(batch.tasks[1].summary.as_deref(), Some("total 42"));

    // Each task worked in a conversation of its own under the parent,
    // which is watched, not written to, and is no subagent.
    let child = batch.tasks[0].conversation.clone().unwrap();
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.dispatched.get(&child), Some(&parent));
    let row = list.rows.iter().find(|row| row.id == child).unwrap();
    assert_eq!(row.title, "count · Count the reads in data.fastq");
    let asked = texts_of(&server, &child).await;
    assert!(asked.iter().any(|text| text
        .starts_with("[From the parent conversation's agent: a delegated task]")
        && text.contains("Task: Count the reads in data.fastq")));
    assert!(send(&server, &child, "hello").await.is_err());
    let status = server
        .subagent_status(PROJECT, &parent, None)
        .await
        .unwrap();
    assert!(!status.contains(&child), "{status}");
    assert_eq!(
        server.delegated_batch_of(&child).await,
        Some(batch.id.clone())
    );

    // The full result of a task is read back, by its own conversation only.
    let full = server
        .delegated_result(
            &parent,
            &json!({"workflow_id": batch.id, "task_id": "count"}),
        )
        .await
        .unwrap();
    assert_eq!(full["status"], "succeeded");
    assert_eq!(full["response"]["output"]["summary"], "42 reads");
    let other = create(&server).await;
    assert!(server
        .delegated_result(
            &other,
            &json!({"workflow_id": batch.id, "task_id": "count"})
        )
        .await
        .is_err());

    // A batch that runs code is approved first; turned down, nothing runs.
    send(&server, &other, "run the analysis").await.unwrap();
    let pending = wait_for(&server, &other, |s| !s.approvals.is_empty()).await;
    let approval = &pending.approvals[0];
    assert!(approval
        .message
        .starts_with(wisp_tools::plan::PLAN_APPROVAL_PREFIX));
    assert!(approval
        .message
        .contains("[ ] run: Run the analysis script"));
    // It waits on its plan card, not in the Agents tab.
    let waiting = &server.delegated_batches(PROJECT, &other).await.unwrap()[0];
    assert!(!waiting.launched);
    assert!(server
        .approve_delegated_batch(&other, &waiting.id)
        .await
        .is_err());
    server
        .answer_with_feedback(PROJECT, &other, &approval.approval_id, "Not now")
        .await
        .unwrap();
    turn_done(&server, &other).await;
    assert!(texts_of(&server, &other)
        .await
        .iter()
        .any(|text| text == "The batch was turned down."));
    // Nothing ran, so nothing is kept.
    assert!(server
        .delegated_batches(PROJECT, &other)
        .await
        .unwrap()
        .is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn a_background_batch_is_delivered_later_and_resumes_its_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(DelegatingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let parent = create(&server).await;
    send(&server, &parent, "count the reads in the background")
        .await
        .unwrap();
    // The turn ends with the batch still going.
    turn_done(&server, &parent).await;
    assert!(texts_of(&server, &parent)
        .await
        .iter()
        .any(|text| text == "Started in the background."));
    // Its result comes back on its own, and the agent sums it up.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if texts_of(&server, &parent)
                .await
                .iter()
                .any(|text| text == "Background: total 42")
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the result must be delivered and summed up");
    turn_done(&server, &parent).await;
    // The result reads as what came back, not as the researcher's words.
    let shown = snapshot(&server, &parent).await;
    let delivered = shown
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL))
        .expect("the delivered result is in the transcript");
    assert_eq!(delivered.role, "tool");
    assert!(delivered.text.contains("total 42"));
    assert!(!shown
        .items
        .iter()
        .any(|item| item.role == "user" && item.text.contains("delegated_batch_completion")));
    let batches = server.delegated_batches(PROJECT, &parent).await.unwrap();
    assert_eq!(batches[0].status, "succeeded");
    server.shutdown().await;
}

#[tokio::test]
async fn a_cua_driver_server_gets_the_autopilot_once_a_decision_model_is_set() {
    use axum::{response::IntoResponse, routing::post, Json, Router};
    use desktop::McpTransport;
    // A server whose tools look like Cua Driver's.
    let app = Router::new().route(
        "/mcp",
        post(|Json(body): Json<Value>| async move {
            let Some(id) = body.get("id").cloned() else {
                return axum::http::StatusCode::ACCEPTED.into_response();
            };
            let result = match body["method"].as_str().unwrap_or_default() {
                "initialize" => json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "cua", "version": "1"}
                }),
                "tools/list" => json!({"tools": [
                    {"name": "get_window_state", "description": "State",
                     "inputSchema": {"type": "object"}},
                    {"name": "click", "description": "Click",
                     "inputSchema": {"type": "object", "properties": {
                        "element_token": {"type": "string"},
                        "target": {"type": "string"}}}}
                ]}),
                _ => json!({}),
            };
            Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    server
        .save_mcp_server(desktop::McpServerInput {
            id: None,
            name: "Cua".into(),
            transport: McpTransport::Http { url },
            token: None,
            env: None,
            headers: None,
            proxy: None,
        })
        .await
        .unwrap();
    let names = |tools: Vec<Box<dyn wisp_tools::Tool>>| {
        tools
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect::<Vec<_>>()
    };
    // Without a decision model the agent drives the driver itself.
    let plain = names(server.mcp_tools().await);
    assert!(
        plain.iter().any(|name| name.ends_with("click")),
        "{plain:?}"
    );
    assert!(!plain.iter().any(|name| name == "desktop_autopilot"));
    let local = wisp_dto::DecisionModelSettings {
        enabled: true,
        ..wisp_dto::DecisionModelSettings::from_preset(
            wisp_dto::DecisionPreset::find("startlux_local").unwrap(),
        )
    };
    server.save_decision_model(local, None).await.unwrap();
    let piloted = names(server.mcp_tools().await);
    assert_eq!(
        piloted
            .iter()
            .filter(|name| *name == "desktop_autopilot")
            .count(),
        1,
        "{piloted:?}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn privacy_mode_hides_chosen_projects_where_the_webview_reads_it() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let start = server.privacy_mode().await.unwrap();
    assert!(!start.active && start.project_ids.is_empty());
    // Nothing chosen is nothing to hide; a project that is not here is
    // left out.
    assert!(server.set_privacy_mode(true, vec![]).await.is_err());
    server
        .set_privacy_mode(
            true,
            vec![PROJECT.into(), "elsewhere".into(), PROJECT.into()],
        )
        .await
        .unwrap();
    let on = server.privacy_mode().await.unwrap();
    assert!(on.active);
    assert_eq!(on.project_ids, [PROJECT]);
    // The store keeps it under the WebView's keys.
    assert_eq!(
        server
            .0
            .store
            .get_setting("wisp-privacy-mode-active")
            .await
            .unwrap()
            .as_deref(),
        Some("1")
    );
    server.set_privacy_mode(false, vec![]).await.unwrap();
    assert!(!server.privacy_mode().await.unwrap().active);
    server.shutdown().await;
}

#[tokio::test]
async fn separate_project_storage_is_the_webviews_switch() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server.separate_project_storage().await.unwrap());
    server.set_separate_project_storage(true).await.unwrap();
    assert!(server.separate_project_storage().await.unwrap());
    // The store reads it where projects are made.
    assert!(server
        .0
        .store
        .decentralized_project_storage()
        .await
        .unwrap());
    server.set_separate_project_storage(false).await.unwrap();
    assert!(!server.separate_project_storage().await.unwrap());
    server.shutdown().await;
}

#[tokio::test]
async fn a_quick_action_drafts_its_workflow_on_what_was_typed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let library = server
        .save_workflow(desktop::WorkflowInput {
            name: "Evidence review".into(),
            goal: "Weigh the evidence".into(),
            tasks: vec![desktop::WorkflowTask {
                title: "Search".into(),
                instructions: "Find the papers.".into(),
                ..Default::default()
            }],
            approval: desktop::WorkflowApproval::ReviewAll,
            ..Default::default()
        })
        .await
        .unwrap();
    let actions = server
        .save_quick_action(desktop::QuickActionInput {
            name: "Review it".into(),
            prompt: "Focus on human studies.".into(),
            workflow: library[0].id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let action = actions.last().unwrap().id.clone();
    let drafted = server
        .start_quick_action(PROJECT, &action, "  Does drug X lower LDL?  ")
        .await
        .unwrap();
    assert!(!drafted.started);
    let batches = server
        .delegated_batches(PROJECT, &drafted.session)
        .await
        .unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].status, "draft");
    // An action that puts a request into the composer starts nothing.
    let request = actions
        .iter()
        .find(|action| action.workflow.is_empty())
        .unwrap()
        .id
        .clone();
    let refused = server
        .start_quick_action(PROJECT, &request, "x")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("starts no workflow"),
        "{refused}"
    );
    assert!(server
        .start_quick_action(PROJECT, "ghost", "x")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_workflow_runs_as_agents_once_it_is_approved() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(DelegatingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let task = |title: &str, instructions: &str, after: &[usize], capabilities: &[&str]| {
        desktop::WorkflowTask {
            title: title.into(),
            instructions: instructions.into(),
            after: after.to_vec(),
            capabilities: capabilities.iter().map(|name| name.to_string()).collect(),
            agent: Default::default(),
        }
    };
    let save = |name: &str, approval, capability: &'static str| {
        let server = server.clone();
        let name = name.to_owned();
        async move {
            let saved = server
                .save_workflow(desktop::WorkflowInput {
                    id: String::new(),
                    name,
                    goal: "Count and add up the reads".into(),
                    tasks: vec![
                        task(
                            "Count the reads",
                            "Count the reads in data.fastq",
                            &[],
                            &[capability],
                        ),
                        task(
                            "Add up the counts",
                            "Add up the counts",
                            &[0],
                            &["reasoning"],
                        ),
                    ],
                    approval,
                    context: String::new(),
                })
                .await
                .unwrap();
            saved.last().unwrap().id.clone()
        }
    };
    let delivered = |session: String| {
        let server = server.clone();
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                loop {
                    if texts_of(&server, &session)
                        .await
                        .iter()
                        .any(|text| text == "Background: total 42")
                    {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("the workflow's result must reach its conversation");
            turn_done(&server, &session).await;
        }
    };

    // Under review it waits as a draft, in a conversation of its own.
    let reviewed = save(
        "Read count",
        desktop::WorkflowApproval::ReviewAll,
        "project_read",
    )
    .await;
    let drafted = server.draft_workflow(PROJECT, &reviewed).await.unwrap();
    assert!(!drafted.started);
    let batches = server
        .delegated_batches(PROJECT, &drafted.session)
        .await
        .unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].status, "draft");
    assert!(batches[0].launched);
    let ids: Vec<&str> = batches[0]
        .tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect();
    assert_eq!(ids, ["count-the-reads", "add-up-the-counts"]);
    assert_eq!(batches[0].tasks[1].depends_on, ["count-the-reads"]);
    assert_eq!(batches[0].tasks[0].capabilities, ["project_read"]);

    // A second draft is thrown away, and only a draft of its own
    // conversation can be.
    let other = server.draft_workflow(PROJECT, &reviewed).await.unwrap();
    assert!(server
        .discard_delegated_batch(&other.session, &drafted.batch)
        .await
        .is_err());
    server
        .discard_delegated_batch(&other.session, &other.batch)
        .await
        .unwrap();
    assert!(server
        .delegated_batches(PROJECT, &other.session)
        .await
        .unwrap()
        .is_empty());

    // Approved, its agents run and the result comes back to it.
    server
        .approve_delegated_batch(&drafted.session, &drafted.batch)
        .await
        .unwrap();
    delivered(drafted.session.clone()).await;
    let batches = server
        .delegated_batches(PROJECT, &drafted.session)
        .await
        .unwrap();
    assert_eq!(batches[0].status, "succeeded");
    assert!(server
        .approve_delegated_batch(&drafted.session, &drafted.batch)
        .await
        .is_err());

    // Under auto-safe approval, one that only reads starts at once, while
    // one that runs code still waits.
    let safe = save(
        "Safe count",
        desktop::WorkflowApproval::AutoSafe,
        "project_read",
    )
    .await;
    let started = server.draft_workflow(PROJECT, &safe).await.unwrap();
    assert!(started.started);
    delivered(started.session.clone()).await;
    let risky = save("Run count", desktop::WorkflowApproval::AutoSafe, "code_run").await;
    let waiting = server.draft_workflow(PROJECT, &risky).await.unwrap();
    assert!(!waiting.started);

    // A task can be done as one of the researcher's experts; one that is
    // gone cannot, and leaves nothing behind.
    let expert = server.experts().await.unwrap()[0].clone();
    let mut tasks = vec![task(
        "Count the reads",
        "Count the reads",
        &[],
        &["project_read"],
    )];
    tasks[0].agent.expert = Some(expert.id.clone());
    let saved = server
        .save_workflow(desktop::WorkflowInput {
            id: String::new(),
            name: "As an expert".into(),
            goal: "Count the reads".into(),
            tasks: tasks.clone(),
            approval: desktop::WorkflowApproval::ReviewAll,
            context: String::new(),
        })
        .await
        .unwrap();
    let as_expert = server
        .draft_workflow(PROJECT, &saved.last().unwrap().id)
        .await
        .unwrap();
    let drafted = server
        .delegated_batches(PROJECT, &as_expert.session)
        .await
        .unwrap();
    assert_eq!(
        drafted[0].tasks[0].expert.as_deref(),
        Some(expert.name.as_str())
    );
    tasks[0].agent.expert = Some("nobody".into());
    let saved = server
        .save_workflow(desktop::WorkflowInput {
            id: String::new(),
            name: "Without the expert".into(),
            goal: "Count the reads".into(),
            tasks,
            approval: desktop::WorkflowApproval::ReviewAll,
            context: String::new(),
        })
        .await
        .unwrap();
    let before = server
        .conversations(PROJECT, "", false)
        .await
        .unwrap()
        .rows
        .len();
    assert!(server
        .draft_workflow(PROJECT, &saved.last().unwrap().id)
        .await
        .is_err());
    assert_eq!(
        server
            .conversations(PROJECT, "", false)
            .await
            .unwrap()
            .rows
            .len(),
        before
    );
    server.shutdown().await;
}

#[tokio::test]
async fn the_research_assistant_dispatches_to_a_project_and_gets_the_report() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(AssistingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // Dispatching asks first, like other tools that set work going,
    // unless the conversation has full access.
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            full_access: true,
            ..defaults
        })
        .await
        .unwrap();
    let (project, frame) = server.open_assistant().await.unwrap();
    assert_eq!(
        server.open_assistant().await.unwrap(),
        (project.clone(), frame.clone())
    );
    // Its project is not one of the researcher's.
    let listed = server.0.store.list_projects().await.unwrap();
    assert!(!listed.iter().any(|row| row.0 == project));
    server
        .send(
            &project,
            send_text(&frame, "a1", "count the reads in my project"),
        )
        .await
        .unwrap();
    turn_done(&server, &frame).await;
    let texts = texts_of(&server, &frame).await;
    assert!(texts[0].starts_with("You are the researcher's research assistant"));
    assert!(
        texts
            .iter()
            .any(|text| text == "Started the count; I will report back."),
        "{texts:?}"
    );

    // The work runs in a conversation of the project, told who asked.
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    let counting = list
        .rows
        .iter()
        .find(|row| row.title == "Counting")
        .unwrap();
    turn_done(&server, &counting.id).await;
    let asked = texts_of(&server, &counting.id).await;
    assert!(asked
        .iter()
        .any(|text| text.starts_with("[From the research assistant]")));
    // Its outcome comes back to the assistant, reviewed.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if texts_of(&server, &frame)
                .await
                .iter()
                .any(|text| text == "Report: the project counted 42 reads.")
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the outcome must be reported to the assistant");

    // The plan records where the work went, and can be ticked off.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let plan = server.assistant_plan(&today).await.unwrap();
    let item = plan
        .iter()
        .find(|item| item.session_id.as_deref() == Some(counting.id.as_str()))
        .expect("the dispatch is on today's plan");
    assert_eq!(item.status, "open");
    assert!(item.project_name.is_some());
    assert!(server
        .set_assistant_plan_item(&item.id, "finished")
        .await
        .is_err());
    server
        .set_assistant_plan_item(&item.id, "done")
        .await
        .unwrap();
    assert_eq!(
        server.assistant_plan(&today).await.unwrap()[0].status,
        "done"
    );
    assert!(server.assistant_plan("not a day").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn the_research_assistant_keeps_its_instructions_and_name_when_told_something_first() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(InstructionsCheckingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // A mandate's notice can reach it before the researcher's first message.
    server
        .announce("职责需要你 / A mandate needs you")
        .await
        .unwrap();
    let (project, frame) = server.open_assistant().await.unwrap();
    server
        .send(&project, send_text(&frame, "a1", "what needs me?"))
        .await
        .unwrap();
    turn_done(&server, &frame).await;
    let texts = texts_of(&server, &frame).await;
    assert!(
        texts[0].starts_with("You are the researcher's research assistant"),
        "{texts:?}"
    );
    assert_eq!(texts[1], "职责需要你 / A mandate needs you");
    assert_eq!(texts.last().map(String::as_str), Some("told"), "{texts:?}");
    // The first message does not rename it.
    assert_eq!(
        server
            .0
            .store
            .session_title(&frame)
            .await
            .unwrap()
            .as_deref(),
        Some("Research assistant")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn an_exploration_starts_from_an_answer_while_follow_ups_are_still_asked_for() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(SlowAfterwardsModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("study");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "v1").unwrap();
    let project = server.register_local_project(&folder).await.unwrap();
    let source = server.create_conversation(&project).await.unwrap();
    server
        .send(&project, send_text(&source, "r1", "what now?"))
        .await
        .unwrap();
    turn_done(&server, &source).await;
    // The follow-up questions are still being asked for...
    assert!(server.session(&source).live.lock().unwrap().suggesting);
    // ...but the turn's work is over, so the project is not busy.
    server
        .start_exploration(&project, &source, None, "Try A")
        .await
        .unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn a_mandate_asks_for_what_it_needs_and_goes_on_once_answered() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(MandateModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let mandate = server
        .create_mandate(wisp_dto::MandateDraft {
            project_id: PROJECT.into(),
            goal: "Keep the alignment up to date.".into(),
            interval_secs: 86_400,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(server
        .create_mandate(wisp_dto::MandateDraft {
            project_id: "missing".into(),
            goal: "x".into(),
            interval_secs: 86_400,
            ..Default::default()
        })
        .await
        .is_err());
    server.run_mandate_now(&mandate.id).await.unwrap();
    // The round asks for the genome: the mandate waits, and the research
    // assistant is told.
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let all = server.mandates().await.unwrap();
            // The round that asked is recorded from its answer.
            if all[0].request.is_some()
                && all[0].mandate.status == "waiting"
                && all[0].last_round.is_some()
            {
                let (_, frame) = server.open_assistant().await.unwrap();
                if texts_of(&server, &frame)
                    .await
                    .iter()
                    .any(|text| text.starts_with("职责需要你 / A mandate needs you"))
                {
                    return all;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the mandate must ask and the assistant be told");
    let frame = waiting[0].mandate.frame_id.clone().unwrap();
    turn_done(&server, &frame).await;
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    let row = list.rows.iter().find(|row| row.id == frame).unwrap();
    assert_eq!(row.title, waiting[0].mandate.name);
    // Its turns are given the brief.
    assert_eq!(
        waiting[0].request.as_ref().unwrap().what,
        "Upload the reference genome"
    );

    // Answered, it goes on and ends the round with what it did.
    let replied = server
        .reply_to_mandate(&mandate.id, "It is in data/genome.fa")
        .await
        .unwrap();
    assert_eq!(replied.status, "active");
    let done = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let all = server.mandates().await.unwrap();
            if all[0]
                .last_round
                .as_ref()
                .is_some_and(|round| round.done == "Indexed the genome")
            {
                return all;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the round must be recorded");
    assert!(done[0].request.is_none());
    turn_done(&server, &frame).await;

    // Closed, it runs no more and its conversation is an ordinary one. A
    // report covers whole seconds before it, so let one pass.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let closed = server
        .set_mandate_status(&mandate.id, "done")
        .await
        .unwrap();
    assert_eq!(closed.status, "done");
    // Closing writes one last report, on the card and to the assistant.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let all = server.mandates().await.unwrap();
            let (_, frame) = server.open_assistant().await.unwrap();
            let told = texts_of(&server, &frame)
                .await
                .iter()
                .any(|text| text.starts_with("职责汇报 / Mandate report"));
            if all[0].reports.len() == 1 && told {
                // The round that asked, recorded from its answer, and the
                // round that reported.
                assert_eq!(all[0].reports[0].rounds, 2);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the closing report must be written and announced");
    assert!(server.run_mandate_now(&mandate.id).await.is_err());
    assert!(server.reply_to_mandate(&mandate.id, "more").await.is_err());
    assert!(server
        .set_mandate_status(&mandate.id, "gone")
        .await
        .is_err());
    server.delete_mandate(&mandate.id).await.unwrap();
    assert!(server.mandates().await.unwrap().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn a_project_file_is_kept_as_an_artifact_of_the_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    std::fs::create_dir_all(cfg.workspace.join("figures")).unwrap();
    std::fs::write(cfg.workspace.join("figures/umap.svg"), "<svg/>").unwrap();
    server
        .register_artifact(PROJECT, &session, "figures/umap.svg")
        .await
        .unwrap();
    // Kept again, it is a new version of the same artifact.
    std::fs::write(cfg.workspace.join("figures/umap.svg"), "<svg></svg>").unwrap();
    server
        .register_artifact(PROJECT, &session, "figures/umap.svg")
        .await
        .unwrap();
    let kept = server.0.store.list_artifacts(&session).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].1, "umap.svg");
    // Nothing outside the project, nothing missing, nothing of Wisp's own.
    let outside = temp.path().join("secret.txt");
    std::fs::write(&outside, "x").unwrap();
    assert!(server
        .register_artifact(PROJECT, &session, &outside.to_string_lossy())
        .await
        .is_err());
    assert!(server
        .register_artifact(PROJECT, &session, "figures/gone.svg")
        .await
        .is_err());
    std::fs::create_dir_all(cfg.workspace.join(".wisp")).unwrap();
    std::fs::write(cfg.workspace.join(".wisp/state.json"), "{}").unwrap();
    assert!(server
        .register_artifact(PROJECT, &session, ".wisp/state.json")
        .await
        .is_err());
    assert!(server
        .register_artifact(PROJECT, "missing", "figures/umap.svg")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversations_request_is_exported_as_json() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    assert!(server.debug_request(PROJECT, &session).await.is_err());
    send(&server, &session, "summarize the sheet")
        .await
        .unwrap();
    turn_done(&server, &session).await;
    let (name, json) = server.debug_request(PROJECT, &session).await.unwrap();
    assert!(name.starts_with("wisp-debug-request-") && name.ends_with(".json"));
    // The conversation's id is one plain component of the name.
    assert!(!name["wisp-debug-request-".len()..].contains('-'));
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["source"], "stored-messages");
    assert_eq!(value["messages"][0]["role"], "system");
    assert!(value["system_prompt_est_tokens"].as_u64().unwrap() > 0);
    assert_eq!(value["message_count"], 3);
    assert_eq!(value["model"], "offline");
    assert_eq!(value["termination_reason"], "completed");
    assert_eq!(value["configured_max_iter"], 5);
    assert!(value["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["text"] == "summarize the sheet"));
    assert!(server.debug_request("other", &session).await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_terminal_on_an_ssh_host_opens_with_its_settings_and_folder() {
    use desktop::SshHostRow;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    server
        .save_ssh_host(SshHostRow {
            alias: "gpu".into(),
            host_name: "10.0.0.8".into(),
            user: "lab".into(),
            port: Some(2222),
            ..Default::default()
        })
        .await
        .unwrap();
    let launch = server
        .terminal_launch(PROJECT, "ssh:gpu", None)
        .await
        .unwrap();
    assert_eq!(launch.program, "ssh");
    assert!(launch.args.windows(2).any(|pair| pair == ["-p", "2222"]));
    assert_eq!(launch.args.last().unwrap(), "lab@10.0.0.8");
    // In a folder: the folder is one quoted word of the remote command.
    let launch = server
        .terminal_launch(PROJECT, "ssh:gpu", Some("/data/it's; rm -rf ~"))
        .await
        .unwrap();
    assert_eq!(
        launch.args.last().unwrap(),
        "cd '/data/it'\\''s; rm -rf ~' && exec \"${SHELL:-/bin/sh}\" -l"
    );
    assert!(server
        .terminal_launch(PROJECT, "ssh:gone", None)
        .await
        .is_err());
    assert!(server
        .terminal_launch(PROJECT, "local", None)
        .await
        .is_err());
    server.shutdown().await;
}

/// A ChatGPT access token as Codex CLI keeps it (unsigned; claims only).
fn chatgpt_token(account: &str, exp: i64, email: &str) -> String {
    use base64::Engine;
    let encode = |value: String| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    let claims = serde_json::json!({
        "exp": exp,
        "https://api.openai.com/profile": {"email": email},
        "https://api.openai.com/auth": {"chatgpt_account_id": account, "chatgpt_plan_type": "plus"},
    });
    format!("{}.{}.sig", encode("{}".into()), encode(claims.to_string()))
}

#[tokio::test]
async fn codex_accounts_are_pooled_imported_switched_and_removed() {
    use desktop::{SecretStore, Subscription};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(cfg, models(Arc::default(), false), secrets.clone())
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("codex")));
    assert!(server.codex_accounts().unwrap().is_empty());

    // Sign-ins of Codex CLI and CLIProxyAPI on this computer.
    let home = temp.path().join("home");
    let exp = wisp_llm::codex_auth::now_ms() / 1000 + 3600;
    let cli = |account: &str, exp: i64, email: &str| {
        serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": chatgpt_token(account, exp, email),
                "refresh_token": format!("refresh-{account}-{exp}"),
                "id_token": "id",
            },
        })
        .to_string()
    };
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        cli("acct-a", exp, "a@example.test"),
    )
    .unwrap();
    let proxy = home.join(".cli-proxy-api");
    std::fs::create_dir_all(&proxy).unwrap();
    // A newer copy of the same account, and a second account.
    std::fs::write(
        proxy.join("codex-a.json"),
        cli("acct-a", exp + 60, "a@example.test"),
    )
    .unwrap();
    std::fs::write(
        proxy.join("codex-b.json"),
        cli("acct-b", exp, "b@example.test"),
    )
    .unwrap();
    std::fs::write(
        proxy.join("claude-c.json"),
        cli("acct-c", exp, "c@example.test"),
    )
    .unwrap();
    let found = crate::subscriptions::import_codex_accounts(secrets.as_ref(), &home, None).unwrap();
    assert_eq!(found, 2);
    let accounts = server.codex_accounts().unwrap();
    let ids: Vec<&str> = accounts.iter().map(|account| account.id.as_str()).collect();
    assert_eq!(ids, ["acct-a", "acct-b"]);
    assert_eq!(accounts[0].email, "a@example.test");
    assert_eq!(accounts[0].plan, "plus");
    assert!(accounts[0].active && !accounts[1].active);
    // The newest copy of an account is the one kept.
    assert!(secrets
        .get("codex_subscription")
        .unwrap()
        .contains(&format!("refresh-acct-a-{}", exp + 60)));
    // A copy in the pool refreshed later (by the WebView, say) is the one
    // a turn runs on, and it becomes the sign-in again.
    let later = wisp_llm::codex_auth::CodexCredentials {
        access_token: chatgpt_token("acct-a", exp + 600, "a@example.test"),
        refresh_token: "refresh-acct-a-later".into(),
        expires_at_ms: (exp + 600) * 1000,
        account_id: "acct-a".into(),
    };
    secrets
        .set("codex_account:acct-a", &later.to_json())
        .unwrap();
    let access = crate::subscriptions::fresh_access(
        secrets.as_ref(),
        Subscription::Codex,
        &reqwest::Client::new(),
    )
    .await
    .unwrap();
    assert_eq!(access, later.access_token);
    assert!(secrets
        .get("codex_subscription")
        .unwrap()
        .contains("refresh-acct-a-later"));
    // Nothing to import is said plainly.
    let empty = temp.path().join("empty-home");
    assert!(crate::subscriptions::import_codex_accounts(secrets.as_ref(), &empty, None).is_err());

    // Switching makes another account the subscription's sign-in.
    server.switch_codex_account("acct-b").await.unwrap();
    assert!(secrets
        .get("codex_subscription")
        .unwrap()
        .contains("refresh-acct-b"));
    assert!(server.codex_accounts().unwrap()[1].active);
    assert!(server.switch_codex_account("acct-missing").await.is_err());
    let row = server
        .subscriptions()
        .into_iter()
        .find(|row| row.subscription == Subscription::Codex)
        .unwrap();
    assert_eq!(row.account.as_deref(), Some("acct-b"));

    // Removing the active account hands over to the next one.
    server.remove_codex_account("acct-b").await.unwrap();
    let accounts = server.codex_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].active && accounts[0].id == "acct-a");
    // Removing the last signs out.
    server.remove_codex_account("acct-a").await.unwrap();
    assert!(server.codex_accounts().unwrap().is_empty());
    assert!(server
        .subscriptions()
        .into_iter()
        .all(|row| row.account.is_none()));
    server.shutdown().await;
}

#[tokio::test]
async fn acp_agents_are_kept_in_the_shared_list_tested_and_removed() {
    use desktop::AcpAgent;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("acp")));
    assert!(server.acp_agents().await.is_empty());
    let agent = |label: &str, command: &str| AcpAgent {
        label: label.into(),
        command: command.into(),
        ..Default::default()
    };
    // A name and a program are needed, and a change needs an agent.
    assert!(server.save_acp_agent(agent(" ", "x")).await.is_err());
    assert!(server.save_acp_agent(agent("A", " ")).await.is_err());
    assert!(server
        .save_acp_agent(AcpAgent {
            id: "missing".into(),
            ..agent("A", "x")
        })
        .await
        .is_err());
    assert!(server
        .save_acp_agent(AcpAgent {
            args: vec!["a\nb".into()],
            ..agent("A", "x")
        })
        .await
        .is_err());

    let id = server
        .save_acp_agent(AcpAgent {
            args: vec!["--acp".into(), " ".into()],
            ..agent(" Missing ", " wisp-no-such-agent ")
        })
        .await
        .unwrap();
    let rows = server.acp_agents().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent.id, id);
    assert_eq!(rows[0].agent.label, "Missing");
    assert_eq!(rows[0].agent.command, "wisp-no-such-agent");
    assert_eq!(rows[0].agent.args, ["--acp"]);
    assert!(!rows[0].found);
    // The WebView desktop reads the same setting in the same shape.
    let raw = server
        .0
        .store
        .get_setting("acp_agent_profiles")
        .await
        .unwrap()
        .unwrap();
    let kept: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(kept[0]["id"], id.as_str());
    assert_eq!(kept[0]["command"], "wisp-no-such-agent");
    assert_eq!(kept[0]["args"], json!(["--acp"]));
    let missing = server.test_acp_agent(&id).await.unwrap_err().to_string();
    assert_eq!(
        missing,
        "The agent's program was not found on this computer"
    );
    assert!(server.test_acp_agent("missing").await.is_err());

    server
        .save_acp_agent(AcpAgent {
            id: id.clone(),
            ..agent("Renamed", "wisp-no-such-agent")
        })
        .await
        .unwrap();
    let rows = server.acp_agents().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent.label, "Renamed");
    assert!(rows[0].agent.args.is_empty());

    #[cfg(unix)]
    {
        let script = fake_acp_agent(temp.path(), FAKE_ACP_AGENT);
        let fake = server
            .save_acp_agent(agent("Fake", &script.to_string_lossy()))
            .await
            .unwrap();
        let rows = server.acp_agents().await;
        assert!(rows.iter().any(|row| row.agent.id == fake && row.found));
        let report = server.test_acp_agent(&fake).await.unwrap();
        assert_eq!(report.protocol, 1);
        assert_eq!(report.name, "Fake agent");
        assert_eq!(report.version, "0.1.0");
        let ways: Vec<_> = report
            .sign_in
            .iter()
            .map(|way| (way.id.as_str(), way.kind))
            .collect();
        assert_eq!(
            ways,
            [
                ("login", desktop::AcpSignInKind::Agent),
                ("tui", desktop::AcpSignInKind::Terminal)
            ]
        );
        // The agent signs in by itself, or names a command for a terminal:
        // its program with its own arguments and then the method's.
        assert_eq!(
            server.sign_in_acp_agent(&fake, "login").await.unwrap(),
            desktop::AcpSignInOutcome::Done
        );
        assert_eq!(
            server.sign_in_acp_agent(&fake, "tui").await.unwrap(),
            desktop::AcpSignInOutcome::Terminal {
                program: script.to_string_lossy().into(),
                args: vec!["auth".into(), "login".into()],
                env: vec![("FAKE_AGENT_AUTH".into(), "1".into())],
            }
        );
        assert!(server.sign_in_acp_agent(&fake, "gone").await.is_err());
    }

    server.remove_acp_agent(&id).await.unwrap();
    assert!(server.remove_acp_agent(&id).await.is_err());
    assert!(server
        .acp_agents()
        .await
        .iter()
        .all(|row| row.agent.id != id));
    server.shutdown().await;
}

#[tokio::test]
async fn a_scheduled_task_fires_once_per_time_into_a_new_conversation() {
    use desktop::ScheduleInput;
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("schedule")));
    assert!(server.schedules().await.unwrap().is_empty());

    let task = |prompt: &str, start_at| ScheduleInput {
        project: PROJECT.into(),
        name: String::new(),
        prompt: prompt.into(),
        interval_secs: 3_600,
        start_at,
        session: None,
    };
    // A project that is not there, and a form that is not filled in.
    let elsewhere = ScheduleInput {
        project: "missing".into(),
        ..task("anything", None)
    };
    assert!(server.create_schedule(elsewhere).await.is_err());
    assert!(server.create_schedule(task(" ", None)).await.is_err());

    let now = chrono::Utc::now().timestamp();
    // One whose time passed three hours ago, one that is not due yet.
    server
        .create_schedule(task("summarise the queue", Some(now - 3 * 3_600 - 10)))
        .await
        .unwrap();
    server
        .create_schedule(task("weekly report", None))
        .await
        .unwrap();
    let listed = server.schedules().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "summarise the queue");
    assert!(listed.iter().all(|row| row.enabled && row.runs.is_empty()));
    let due = listed[0].id.clone();
    let later = listed[1].id.clone();

    // However long it was due, it fires once and keeps its place on the clock.
    assert_eq!(server.fire_due_schedules(now).await.unwrap(), 1);
    assert_eq!(server.fire_due_schedules(now).await.unwrap(), 0);
    let listed = server.schedules().await.unwrap();
    let fired = listed.iter().find(|row| row.id == due).unwrap();
    assert_eq!(fired.runs.len(), 1);
    assert!(fired.runs[0].error.is_none());
    assert_eq!(fired.next_run_at, now - 10 + 3_600);
    let session = fired.runs[0].session.clone().unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let sent = seen.lock().unwrap().join("\n");
    assert!(sent.contains("[Scheduled task: summarise the queue]\n\nsummarise the queue"));
    // The conversation is named after the task, not after its message.
    assert_eq!(
        server
            .0
            .store
            .session_title(&session)
            .await
            .unwrap()
            .as_deref(),
        Some("summarise the queue")
    );

    // Off, a task whose time comes does not fire; on again, it does once.
    server.set_schedule_enabled(&later, false).await.unwrap();
    let far = now + 10 * 3_600;
    assert_eq!(
        server.fire_due_schedules(far).await.unwrap(),
        1,
        "the first"
    );
    server.set_schedule_enabled(&later, true).await.unwrap();
    assert_eq!(server.fire_due_schedules(far).await.unwrap(), 1);
    assert_eq!(server.fire_due_schedules(far).await.unwrap(), 0);

    // Run now sends it at once and leaves the cadence where it was.
    let before = server.schedules().await.unwrap();
    let cadence = before
        .iter()
        .find(|row| row.id == later)
        .unwrap()
        .next_run_at;
    let ran = server.run_schedule_now(&later).await.unwrap();
    wait_for(&server, &ran, |s| !s.running).await;
    let after = server.schedules().await.unwrap();
    let row = after.iter().find(|row| row.id == later).unwrap();
    assert_eq!(row.next_run_at, cadence);
    assert_eq!(row.runs.len(), 2);

    // A conversation's own timer is the WebView's: not listed, not fired,
    // and not this page's to change.
    let timer = wisp_store::ScheduleRecord {
        id: "timer".into(),
        project_id: PROJECT.into(),
        frame_id: Some(session.clone()),
        replace_previous_turn: true,
        name: "timer".into(),
        prompt: "check".into(),
        skill: None,
        interval_secs: 3_600,
        enabled: true,
        next_run_at: now - 60,
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };
    server.0.store.create_schedule(&timer).await.unwrap();
    assert_eq!(server.schedules().await.unwrap().len(), 2);
    assert_eq!(server.fire_due_schedules(now).await.unwrap(), 0);
    assert!(server.delete_schedule("timer").await.is_err());
    assert!(server.run_schedule_now("timer").await.is_err());

    server.delete_schedule(&due).await.unwrap();
    assert!(server.delete_schedule(&due).await.is_err());
    assert_eq!(server.schedules().await.unwrap().len(), 1);

    // A conversation's timer is sent into that conversation each time,
    // and goes when the conversation's timers are cancelled.
    let bound = ScheduleInput {
        session: Some(session.clone()),
        ..task("check the queue again", Some(now - 5))
    };
    let stranger = ScheduleInput {
        session: Some("absent".into()),
        ..task("nowhere", None)
    };
    assert!(server.create_schedule(stranger).await.is_err());
    server.create_schedule(bound).await.unwrap();
    let before = server.0.store.list_sessions(PROJECT).await.unwrap().len();
    assert_eq!(server.fire_due_schedules(now).await.unwrap(), 1);
    let listed = server.schedules().await.unwrap();
    let timer = listed
        .iter()
        .find(|row| row.session.as_deref() == Some(session.as_str()))
        .unwrap();
    assert_eq!(timer.runs[0].session.as_deref(), Some(session.as_str()));
    assert!(timer.runs[0].error.is_none());
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        server.0.store.list_sessions(PROJECT).await.unwrap().len(),
        before,
        "no new conversation"
    );
    assert!(seen
        .lock()
        .unwrap()
        .join("\n")
        .contains("[Scheduled task: check the queue again]"));
    assert_eq!(
        server
            .cancel_conversation_schedules(PROJECT, &session)
            .await
            .unwrap(),
        1
    );
    assert_eq!(server.schedules().await.unwrap().len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn a_subagents_conversation_says_it_is_read_only_because_it_is_a_subagents() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let parent = server.create_conversation(PROJECT).await.unwrap();
    let child = server.create_conversation(PROJECT).await.unwrap();
    server
        .0
        .store
        .set_session_dispatched_from(&child, &parent)
        .await
        .unwrap();
    let watched = snapshot(&server, &child).await;
    assert!(watched.read_only);
    assert_eq!(
        watched.read_only_reason,
        Some(dto::ReadOnlyReason::Subagent)
    );
    let open = snapshot(&server, &parent).await;
    assert!(!open.read_only && open.read_only_reason.is_none());
    // An older client reads the snapshot as before: the reason is one
    // more field, and absent when there is none.
    let sent = serde_json::to_value(&open).unwrap();
    assert!(sent.get("read_only_reason").is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_keeps_one_timer_that_waits_while_the_conversation_is_busy() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let session = server.create_conversation(PROJECT).await.unwrap();
    assert!(server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .is_none());
    assert!(server
        .set_conversation_timer(PROJECT, "absent", "check", 600)
        .await
        .is_err());
    assert!(server
        .set_conversation_timer(PROJECT, &session, "check", 30)
        .await
        .is_err());

    // Setting it again changes the one timer, and a paused one stays paused.
    server
        .set_conversation_timer(PROJECT, &session, "check the queue", 600)
        .await
        .unwrap();
    let first = server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .unwrap();
    assert!(first.enabled);
    server.set_schedule_enabled(&first.id, false).await.unwrap();
    let before = chrono::Utc::now().timestamp();
    server
        .set_conversation_timer(PROJECT, &session, "look again", 1_800)
        .await
        .unwrap();
    let timer = server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.schedules().await.unwrap().len(), 1);
    assert_eq!(
        (timer.prompt.as_str(), timer.interval_secs, timer.enabled),
        ("look again", 1_800, false)
    );
    assert!(timer.next_run_at >= before + 1_800);
    assert_eq!(timer.session.as_deref(), Some(session.as_str()));
    let due = timer.next_run_at;
    // Paused, it does not fire.
    assert_eq!(server.fire_due_schedules(due).await.unwrap(), 0);
    server.set_schedule_enabled(&timer.id, true).await.unwrap();

    // While a turn runs there, or a message waits for one, it stays due
    // and nothing is recorded.
    let live = server.session(&session);
    live.live.lock().unwrap().running = true;
    assert_eq!(server.fire_due_schedules(due).await.unwrap(), 0);
    live.live.lock().unwrap().running = false;
    let queued = live
        .queue
        .lock()
        .unwrap()
        .push(PROJECT, "after this turn", vec![], vec![])
        .unwrap();
    assert_eq!(server.fire_due_schedules(due).await.unwrap(), 0);
    let waiting = server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.next_run_at, due);
    assert!(waiting.runs.is_empty());

    // Idle again, it fires into the conversation.
    live.queue.lock().unwrap().take(queued).unwrap();
    assert_eq!(server.fire_due_schedules(due).await.unwrap(), 1);
    wait_for(&server, &session, |s| !s.running).await;
    let fired = server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fired.next_run_at, due + 1_800);
    assert!(fired.runs[0].error.is_none());
    assert!(seen
        .lock()
        .unwrap()
        .join("\n")
        .contains("[Scheduled task: look again]"));

    assert_eq!(
        server
            .cancel_conversation_schedules(PROJECT, &session)
            .await
            .unwrap(),
        1
    );
    assert!(server
        .conversation_timer(PROJECT, &session)
        .await
        .unwrap()
        .is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn a_plugin_is_installed_once_and_brings_its_skills_to_the_projects_that_switch_it_on() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), false))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("plugin")));
    assert!(server.plugins(Some(PROJECT)).await.unwrap().is_empty());
    assert!(server.plugins(Some("missing")).await.is_err());

    let package = temp.path().join("motif");
    crate::plugins::tests::write_plugin(&package, "motif-tools", "1.2.0");
    assert!(server
        .install_plugin(&temp.path().join("absent"))
        .await
        .is_err());
    let installed = server.install_plugin(&package).await.unwrap();
    assert_eq!(installed.name, "Motif tools");
    assert_eq!(installed.skills, ["find-motifs"]);
    assert_eq!(installed.servers, ["uvx motif-mcp --stdio"]);
    assert!(!installed.enabled);
    assert!(cfg
        .data
        .join("plugins/motif-tools/1.2.0/skills/find-motifs/SKILL.md")
        .is_file());

    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let names = |project: String, root: std::path::PathBuf| {
        let server = &server;
        async move {
            crate::runtime::skill_index(&server.0.config, &server.0.store, &project, &root)
                .await
                .all()
                .iter()
                .map(|skill| skill.name.clone())
                .collect::<Vec<_>>()
        }
    };
    // Installed is not switched on: no project has the skill yet.
    assert!(!names(lab.clone(), folder.clone())
        .await
        .contains(&"find-motifs".to_owned()));

    // Switched on for one project, its skill is that project's only.
    assert!(server
        .set_plugin_enabled(&lab, "motif-tools", "9.9.9", true)
        .await
        .is_err());
    server
        .set_plugin_enabled(&lab, "motif-tools", "1.2.0", true)
        .await
        .unwrap();
    assert!(server.plugins(Some(&lab)).await.unwrap()[0].enabled);
    assert!(!server.plugins(Some(PROJECT)).await.unwrap()[0].enabled);
    // Asked about no project, it is listed and reads as off.
    assert!(!server.plugins(None).await.unwrap()[0].enabled);
    assert!(names(lab.clone(), folder.clone())
        .await
        .contains(&"find-motifs".to_owned()));
    assert!(!names(PROJECT.into(), cfg.workspace.clone())
        .await
        .contains(&"find-motifs".to_owned()));

    // A newer version replaces the files and the record; the project
    // chose the older one, so it is off until switched on again.
    crate::plugins::tests::write_plugin(&package, "motif-tools", "1.3.0");
    let newer = server.install_plugin(&package).await.unwrap();
    assert_eq!(newer.version, "1.3.0");
    let listed = server.plugins(Some(&lab)).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].enabled);
    assert!(!cfg.data.join("plugins/motif-tools/1.2.0").exists());
    assert!(!names(lab.clone(), folder.clone())
        .await
        .contains(&"find-motifs".to_owned()));
    server
        .set_plugin_enabled(&lab, "motif-tools", "1.3.0", true)
        .await
        .unwrap();
    server
        .set_plugin_enabled(&lab, "motif-tools", "1.3.0", false)
        .await
        .unwrap();
    assert!(!server.plugins(Some(&lab)).await.unwrap()[0].enabled);
    server
        .set_plugin_enabled(&lab, "motif-tools", "1.3.0", true)
        .await
        .unwrap();

    // Removing it takes it from every project, with its files.
    assert!(server.remove_plugin("motif-tools", "1.2.0").await.is_err());
    assert!(server.remove_plugin("../outside", "1.3.0").await.is_err());
    server.remove_plugin("motif-tools", "1.3.0").await.unwrap();
    assert!(server.plugins(Some(&lab)).await.unwrap().is_empty());
    assert!(!cfg.data.join("plugins/motif-tools").exists());
    assert!(!names(lab, folder).await.contains(&"find-motifs".to_owned()));
    // The package it was installed from is untouched.
    assert!(package.join("skills/find-motifs/SKILL.md").is_file());
    server.shutdown().await;
}

#[tokio::test]
async fn remote_access_dials_a_relay_and_answers_a_browser_that_holds_the_code() {
    use desktop::RemoteState;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use wisp_sync::{open_frame, seal_frame, RemoteCode, CLIENT_TO_HOST, HOST_TO_CLIENT};

    let temp = tempfile::tempdir().unwrap();
    // A relay of our own on this machine: nothing leaves it.
    let relay = wisp_sync::FileRelay::open(&temp.path().join("relay"))
        .await
        .unwrap();
    let app = wisp_sync::relay_router(wisp_sync::RelayHttpState::new(relay, "made-up").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("http://{addr}");

    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    // Setting it up is the desktop's own, not something a browser can ask.
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("remote_access")));
    assert_eq!(
        server.remote_access().await.unwrap(),
        desktop::RemoteAccess::default()
    );

    // It is not turned on without a relay and a token, and a relay that
    // a browser could not use is refused.
    assert!(server.set_remote_access(true, "", None).await.is_err());
    assert!(server.set_remote_access(true, &url, None).await.is_err());
    assert!(server
        .set_remote_access(false, "http://relay.example", None)
        .await
        .is_err());
    assert!(server
        .set_remote_access(false, &url, Some("two words".into()))
        .await
        .is_err());
    assert!(!server.remote_access().await.unwrap().enabled);

    let state_is = |wanted: RemoteState| {
        let server = &server;
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let now = server.remote_access().await.unwrap();
                    if now.state == wanted {
                        return now;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("the tunnel reaches the state")
        }
    };

    // A token the relay does not know: the tunnel says why it is not up.
    server
        .set_remote_access(true, &url, Some("wrong".into()))
        .await
        .unwrap();
    let refused = state_is(RemoteState::Failed).await;
    assert!(!refused.detail.is_empty());

    let on = server
        .set_remote_access(true, &format!(" {url} "), Some("made-up".into()))
        .await
        .unwrap();
    assert!(on.enabled && on.has_token);
    assert_eq!(on.relay_url, url);
    let shown = on.code.clone().unwrap();
    assert_eq!(
        on.link.as_deref(),
        Some(format!("{url}/remote#{shown}").as_str())
    );
    // The link as a QR code for a phone: a square of modules.
    let qr = on.qr.clone().expect("the link comes as a QR code too");
    assert!(qr.width >= 21);
    assert_eq!(qr.dark.len(), qr.width * qr.width);
    assert_eq!(
        Some(qr),
        crate::channels::weixin::QrMatrix::of(on.link.as_deref().unwrap()).ok()
    );
    state_is(RemoteState::Connected).await;
    // The token and the code are in the secret store only.
    for key in ["remote_relay_token", "remote_access_code"] {
        assert_eq!(server.0.store.get_setting(key).await.unwrap(), None);
    }

    // A browser that holds the code is greeted and answered.
    let code = RemoteCode::parse(&shown).unwrap();
    let key = code.key();
    let client = format!("ws://{addr}/v1/remote/client/{}", code.sid());
    let (mut browser, _) = tokio_tungstenite::connect_async(&client).await.unwrap();
    async fn frame<S>(socket: &mut S, key: &[u8; 32]) -> Value
    where
        S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
    {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                // The relay's keep-alive pings are not frames of the host.
                match socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => {
                        let plain = open_frame(key, HOST_TO_CLIENT, &text).unwrap();
                        return serde_json::from_slice(&plain).unwrap();
                    }
                    Message::Ping(_) | Message::Pong(_) => {}
                    other => panic!("expected a sealed frame, got {other:?}"),
                }
            }
        })
        .await
        .expect("the host answers")
    }
    let hello = frame(&mut browser, &key).await;
    assert_eq!(hello["type"], "hello");
    assert!(hello["commands"]
        .as_array()
        .unwrap()
        .contains(&json!("list_projects")));
    let nonce = hello["nonce"].as_str().unwrap().to_owned();
    assert_eq!(server.remote_access().await.unwrap().clients, 1);

    let ask = |seq: u64, id: &str, command: &str| {
        let request = json!({
            "nonce": nonce, "seq": seq, "id": id, "command": command, "args": {}
        });
        Message::Text(
            seal_frame(&key, CLIENT_TO_HOST, request.to_string().as_bytes())
                .unwrap()
                .into(),
        )
    };
    browser
        .send(ask(1, "first", "list_projects"))
        .await
        .unwrap();
    let answer = frame(&mut browser, &key).await;
    assert_eq!(answer["type"], "response");
    assert_eq!(answer["response"]["id"], "first");
    assert!(answer["response"]["error"].is_null(), "{answer}");
    assert!(answer["response"]["result"].to_string().contains(PROJECT));

    // A frame sent again is dropped; a command the host does not offer
    // remotely is refused by name.
    browser
        .send(ask(1, "replayed", "list_projects"))
        .await
        .unwrap();
    browser
        .send(ask(2, "second", "set_remote_access"))
        .await
        .unwrap();
    let answer = frame(&mut browser, &key).await;
    assert_eq!(answer["response"]["id"], "second");
    assert_eq!(
        answer["response"]["error"],
        "This command is not available on this host"
    );

    // A new code: the old link is dead and the tunnel comes back on the new one.
    let reset = server.reset_remote_access_code().await.unwrap();
    assert_ne!(reset.code, Some(shown));
    state_is(RemoteState::Connected).await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match browser.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the old browser is dropped");

    // Off: the tunnel goes, the token stays for the next time.
    let off = server.set_remote_access(false, &url, None).await.unwrap();
    assert!(!off.enabled && off.has_token);
    assert_eq!((off.state, off.clients), (RemoteState::Off, 0));
    server.shutdown().await;
}

#[tokio::test]
async fn a_workflow_runs_as_one_conversation_given_its_staged_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("workflow")));
    let workflows = server.workflows().await.unwrap();
    assert_eq!(workflows.len(), 2);
    let review = workflows[0].clone();
    assert_eq!(review.shape(), (3, 2));

    assert!(server.run_workflow(PROJECT, "absent").await.is_err());
    assert!(server.run_workflow("missing", &review.id).await.is_err());
    let session = server.run_workflow(PROJECT, &review.id).await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let sent = seen.lock().unwrap().join("\n");
    assert!(sent.contains("[Workflow: Literature evidence review]"));
    assert!(sent.contains("- Task 3 — Weigh the evidence (after task 1, task 2): "));
    assert!(sent.find("\nStage 1\n").unwrap() < sent.find("\nStage 3\n").unwrap());
    // The conversation is named after the workflow, not after its message.
    assert_eq!(
        server
            .0
            .store
            .session_title(&session)
            .await
            .unwrap()
            .as_deref(),
        Some("Literature evidence review")
    );

    let kept = server.delete_workflow(&review.id).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert!(server.run_workflow(PROJECT, &review.id).await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn the_browser_tools_are_offered_only_while_the_bridge_is_switched_on() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("browser")));
    // Off until someone switches it on, and nothing is listening.
    let start = server.browser_settings().await.unwrap();
    assert!(!start.enabled && !start.listening && start.problem.is_none());
    assert_eq!(start.status, desktop::BrowserStatus::default());

    let offered = |seen: &Arc<std::sync::Mutex<Vec<String>>>| {
        let mut seen = seen.lock().unwrap();
        let names: Vec<&str> = [
            "browser_setup",
            "web_open_tab",
            "web_scan",
            "web_execute_js",
        ]
        .into_iter()
        .filter(|name| {
            seen.iter()
                .any(|entry| entry.contains(&format!("name: \"{name}\"")))
        })
        .collect();
        seen.clear();
        names.len()
    };
    let session = create(&server).await;
    send(&server, &session, "what is new on the preprint server?")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(offered(&seen), 0);

    // On, the bridge listens on this computer and the next turn has the tools.
    server.0.browser.listen_on("127.0.0.1:0");
    let on = server.set_browser_enabled(true).await.unwrap();
    assert!(on.enabled && on.listening && on.problem.is_none());
    assert!(!on.status.connected);
    send(&server, &session, "and today?").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(offered(&seen), 4);

    // The lists are kept with it.
    let rule = |host: &str| desktop::BrowserHostRule {
        host: host.into(),
        reason: String::new(),
    };
    let kept = server
        .save_browser_filters(desktop::BrowserFilters {
            block: vec![rule("Example.com")],
            prefer: vec![rule("https://pubmed.ncbi.nlm.nih.gov/")],
        })
        .await
        .unwrap();
    assert_eq!(kept.filters.block, [rule("example.com")]);
    assert_eq!(kept.filters.prefer, [rule("pubmed.ncbi.nlm.nih.gov")]);
    let bad = desktop::BrowserFilters {
        block: vec![rule("not a host")],
        prefer: vec![],
    };
    assert!(server.save_browser_filters(bad).await.is_err());

    // Off again: nothing listens and the tools are gone from the next turn.
    let off = server.set_browser_enabled(false).await.unwrap();
    assert!(!off.enabled && !off.listening);
    send(&server, &session, "never mind").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(offered(&seen), 0);

    // A port another program holds is reported, not fought over.
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    server
        .0
        .browser
        .listen_on(&taken.local_addr().unwrap().to_string());
    let blocked = server.set_browser_enabled(true).await.unwrap();
    assert!(blocked.enabled && !blocked.listening);
    assert!(blocked.problem.unwrap().contains("cannot listen"));
    server.shutdown().await;
}

/// A snapshot as a client that draws compaction rows asks for it.
async fn snapshot_with_compactions(server: &Server, id: &str) -> dto::Snapshot {
    serde_json::from_value(
        call(
            server,
            "native_conversation_snapshot",
            PROJECT,
            json!({"session_id": id, "compaction_rows": true}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

/// The compaction rows of a transcript, with where each one is.
fn compaction_cards(items: &[dto::Item]) -> Vec<(usize, wisp_dto::ContextCompactionDto)> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.role == "compaction")
        .map(|(at, item)| (at, serde_json::from_str(&item.text).unwrap()))
        .collect()
}

#[tokio::test]
async fn a_conversation_is_compacted_on_request_and_its_history_archived() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(cfg.clone(), factory)
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("compact")));
    let session = create(&server).await;
    // Nothing was said yet, and a conversation of another project is not ours.
    assert!(server
        .compact_conversation(PROJECT, &session, desktop::CompactKind::Regular)
        .await
        .is_err());
    assert!(server
        .compact_conversation("missing", &session, desktop::CompactKind::Regular)
        .await
        .is_err());

    for message in ["align the reads", "now call variants", "and summarise"] {
        send(&server, &session, message).await.unwrap();
        wait_for(&server, &session, |s| !s.running).await;
    }
    let shown_before = snapshot(&server, &session).await.items.len();
    let done = server
        .compact_conversation(PROJECT, &session, desktop::CompactKind::Regular)
        .await
        .unwrap();
    assert!(done.after <= done.before, "{done:?}");
    // Regular compaction keeps every turn; it only trims old tool output.
    assert!(!done.summarised);
    // The whole history is archived in the project.
    let archives: Vec<_> = std::fs::read_dir(cfg.workspace.join(".wisp/history"))
        .unwrap()
        .collect();
    assert_eq!(archives.len(), 1);
    // The transcript on screen keeps every row it had, and a compaction
    // row after them says what happened and that it can be undone.
    let shown = snapshot_with_compactions(&server, &session).await.items;
    assert_eq!(shown.len(), shown_before + 1);
    let marker = compaction_cards(&shown);
    assert_eq!(marker.len(), 1, "{shown:?}");
    let (at, card) = &marker[0];
    assert_eq!(*at, shown.len() - 1);
    // A client that did not ask (a phone, the web client) gets none: it
    // would show the row's raw text.
    let plain = snapshot(&server, &session).await.items;
    assert_eq!(plain.len(), shown_before);
    assert!(compaction_cards(&plain).is_empty());
    assert_eq!(card.strategy, "manual");
    assert!(card.can_undo && card.undo_reason.is_none(), "{card:?}");
    assert!(card.checkpoint.is_none());
    assert_eq!((card.before, card.after), (done.before, done.after));

    // Undone, the model is sent everything again; only the compaction
    // that was looked at, and only once.
    let compacted = server.model_view(PROJECT, &session).await.unwrap();
    assert!(compacted.epoch > 0);
    let whole_before = compacted.chars;
    assert!(server
        .undo_compaction(PROJECT, &session, compacted.epoch + 1)
        .await
        .is_err());
    assert!(server
        .undo_compaction("missing", &session, compacted.epoch)
        .await
        .is_err());
    server
        .undo_compaction(PROJECT, &session, compacted.epoch)
        .await
        .unwrap();
    let restored = server.model_view(PROJECT, &session).await.unwrap();
    assert_eq!(restored.epoch, 0);
    // Its row is gone with it.
    let shown = snapshot_with_compactions(&server, &session).await.items;
    assert!(compaction_cards(&shown).is_empty());
    assert_eq!(shown.len(), shown_before);
    assert!(!restored.compacted);
    assert!(restored.chars >= whole_before);
    assert!(server
        .undo_compaction(PROJECT, &session, compacted.epoch)
        .await
        .is_err());

    // Compacted again, into a summary this time, and the conversation goes
    // on from what was kept; then it can no longer be undone.
    let summarised = server
        .compact_conversation(PROJECT, &session, desktop::CompactKind::Semantic)
        .await
        .unwrap();
    assert!(summarised.summarised, "{summarised:?}");
    let again = server.model_view(PROJECT, &session).await.unwrap().epoch;
    let shown = snapshot_with_compactions(&server, &session).await.items;
    let (_, card) = &compaction_cards(&shown)[0];
    assert_eq!(card.epoch, again as u64);
    assert!(card.checkpoint.is_some(), "{card:?}");
    send(&server, &session, "what did we find?").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(seen
        .lock()
        .unwrap()
        .join("\n")
        .contains("what did we find?"));
    let refused = server
        .undo_compaction(PROJECT, &session, again)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("went on after"), "{refused}");
    // The row stays where the compaction was, before the question asked
    // after it, and no longer offers undo.
    let shown = snapshot_with_compactions(&server, &session).await.items;
    let marker = compaction_cards(&shown);
    assert_eq!(marker.len(), 1, "{shown:?}");
    let (at, card) = &marker[0];
    assert_eq!(shown[at + 1].role, "user");
    assert_eq!(shown[at + 1].text, "what did we find?");
    assert!(!card.can_undo);
    assert_eq!(card.undo_reason.as_deref(), Some("has_new_turns"));
    assert!(compaction_cards(&snapshot(&server, &session).await.items).is_empty());
    server.shutdown().await;
}

/// Writes a handoff note when asked in the WebView's words, keeping the
/// user's instruction in it, and otherwise answers as the offline model.
struct HandoffWriter;
#[async_trait]
impl Provider for HandoffWriter {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
    ) -> wisp_llm::Result<Completion> {
        let texts: Vec<String> = messages.iter().map(|m| m.content.as_text()).collect();
        let content = if texts.first().is_some_and(|t| t.contains("handoff note")) {
            let mut note = "## Goal and current stage\nAlign the reads.".to_owned();
            if let Some(asked) = texts
                .iter()
                .find(|t| t.starts_with("<handoff-instruction>"))
            {
                note.push_str(&format!("\n\nAsked: {asked}"));
            }
            if texts.iter().any(|t| t.contains("align the reads")) {
                note.push_str("\n\nFrom the transcript.");
            }
            note
        } else {
            "Saved result".to_owned()
        };
        Ok(Completion {
            content,
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        _: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        sink.on_text("Saved result");
        Ok(Completion {
            content: "Saved result".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn a_conversation_is_handed_off_to_a_new_one_that_starts_from_the_note() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(HandoffWriter));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let ask = |session: &str, instruction: Option<String>| dto::HandoffRequest {
        session_id: session.into(),
        instruction,
    };
    let source = create(&server).await;
    // Nothing was said yet: there is nothing to hand off.
    assert!(server
        .draft_handoff(PROJECT, ask(&source, None))
        .await
        .is_err());
    send(&server, &source, "align the reads").await.unwrap();
    wait_for(&server, &source, |s| !s.running && s.items.len() >= 2).await;
    let said = |messages: Vec<Message>| {
        messages
            .iter()
            .map(|message| (message.role.clone(), message.content.as_text()))
            .collect::<Vec<_>>()
    };
    let before = said(server.whole_transcript(&source).await.unwrap());

    // Not this project's conversation, or too long an instruction.
    assert!(server
        .draft_handoff("missing", ask(&source, None))
        .await
        .is_err());
    assert!(server
        .draft_handoff(PROJECT, ask(&source, Some("研".repeat(8001))))
        .await
        .is_err());
    // The model writes the note from the transcript, as the WebView asks.
    let drafted = server
        .draft_handoff(PROJECT, ask(&source, Some("keep the QC thresholds".into())))
        .await
        .unwrap();
    assert_eq!(drafted.project_id, PROJECT);
    assert_eq!(drafted.session_id, source);
    assert!(
        drafted.note.starts_with("## Goal and current stage"),
        "{}",
        drafted.note
    );
    assert!(drafted.note.contains("keep the QC thresholds"));
    assert!(drafted.note.contains("From the transcript."));

    // An empty note, or another project, starts nothing.
    assert!(server
        .start_from_handoff(PROJECT, &source, "  \n ")
        .await
        .is_err());
    assert!(server
        .start_from_handoff("missing", &source, "note")
        .await
        .is_err());
    // The note as the user changed it is the new conversation's first
    // message, in the same project.
    let edited = format!("{}\n\nAlso re-run QC.", drafted.note);
    let fresh = server
        .start_from_handoff(PROJECT, &source, &edited)
        .await
        .unwrap();
    assert_ne!(fresh, source);
    assert_eq!(
        server.0.store.frame_project_id(&fresh).await.unwrap(),
        Some(PROJECT.to_owned())
    );
    let started = wait_for(&server, &fresh, |s| !s.running && s.items.len() >= 2).await;
    assert!(started.error.is_none(), "{:?}", started.error);
    assert_eq!(started.items[0].role, "user");
    assert_eq!(started.items[0].text, edited);
    let opened = said(server.whole_transcript(&fresh).await.unwrap());
    assert_eq!(
        opened
            .iter()
            .find(|(role, _)| *role == wisp_llm::Role::User),
        Some(&(wisp_llm::Role::User, edited.clone()))
    );
    // The conversation handed off is as it was.
    assert_eq!(
        said(server.whole_transcript(&source).await.unwrap()),
        before
    );
    server.shutdown().await;
}

/// Refuses every turn as too long for its context window while `full` is
/// set, as a provider does once a conversation has outgrown it; writes a
/// summary when asked for one, and otherwise answers. Records each turn's
/// request.
struct Outgrown {
    full: Arc<std::sync::atomic::AtomicBool>,
    seen: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}
#[async_trait]
impl Provider for Outgrown {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Summary of the earlier turns".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        self.seen
            .lock()
            .unwrap()
            .push(messages.iter().map(|m| m.content.as_text()).collect());
        if self.full.load(Ordering::Relaxed) {
            return Err(wisp_llm::LlmError::Api {
                status: 400,
                body: r#"{"error":{"message":"This model's maximum context length is 8192 tokens. However, your messages resulted in 9000 tokens.","code":"context_length_exceeded"}}"#.into(),
            });
        }
        sink.on_text("Continued");
        Ok(Completion {
            content: "Continued".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn a_turn_refused_as_too_long_is_compacted_and_continued_or_taken_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let full: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let seen: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let (outgrown, record) = (full.clone(), seen.clone());
    let factory: ModelFactory = Arc::new(move |_| {
        Box::new(Outgrown {
            full: outgrown.clone(),
            seen: record.clone(),
        })
    });
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let session = create(&server).await;
    for message in ["align the reads", "now call variants"] {
        send(&server, &session, message).await.unwrap();
        let ended = wait_for(&server, &session, |s| !s.running && s.items.len() >= 2).await;
        assert!(ended.error.is_none() && ended.error_kind.is_none());
    }

    // The conversation has outgrown the window: the turn fails, even after
    // its own compaction, and the snapshot says why in a way a client can
    // offer a way out of; the text stays for those that only show it.
    full.store(true, Ordering::Relaxed);
    send(&server, &session, "and summarise").await.unwrap();
    let failed = wait_for(&server, &session, |s| !s.running && s.error.is_some()).await;
    assert_eq!(
        failed.error_kind,
        Some(dto::TurnErrorKind::ContextTooLong),
        "{:?}",
        failed.error
    );
    assert!(
        failed
            .error
            .as_deref()
            .unwrap()
            .contains("maximum context length"),
        "{:?}",
        failed.error
    );
    let sent = serde_json::to_value(&failed).unwrap();
    assert_eq!(sent["error_kind"], "context_too_long");

    // Taken elsewhere: a new conversation is sent the WebView's prompt
    // with this one referenced, and reads it from there.
    full.store(false, Ordering::Relaxed);
    assert!(server
        .continue_in_new_conversation("missing", &session, "Continue.")
        .await
        .is_err());
    let elsewhere = server
        .continue_in_new_conversation(
            PROJECT,
            &session,
            "Continue the last unfinished request from the attached conversation.",
        )
        .await
        .unwrap();
    assert_eq!(
        server.0.store.frame_project_id(&elsewhere).await.unwrap(),
        Some(PROJECT.to_owned())
    );
    let there = wait_for(&server, &elsewhere, |s| !s.running && s.items.len() >= 2).await;
    assert!(there.error.is_none(), "{:?}", there.error);
    assert!(seen
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .iter()
        .any(|text| text.contains("now call variants")));
    // The conversation it came from still says what happened.
    let still = snapshot(&server, &session).await;
    assert_eq!(still.error_kind, Some(dto::TurnErrorKind::ContextTooLong));

    // Compacted and continued: the same compaction the marker shows, then
    // the interrupted turn goes on from what was saved, with no new
    // message and nothing said twice.
    let done = server
        .compact_conversation(PROJECT, &session, desktop::CompactKind::Semantic)
        .await
        .unwrap();
    assert!(done.summarised, "{done:?}");
    let marked = compaction_cards(&snapshot_with_compactions(&server, &session).await.items);
    assert!(
        marked.iter().any(|(_, card)| card.strategy == "manual"),
        "{marked:?}"
    );
    seen.lock().unwrap().clear();
    // As the desktop asks for it.
    server
        .send(
            PROJECT,
            dto::SendRequest {
                session_id: session.clone(),
                request_id: uuid::Uuid::new_v4().to_string(),
                message: String::new(),
                resume: true,
                resume_after: None,
                attachments: vec![],
                references: vec![],
                acp_agent_id: None,
            },
        )
        .await
        .unwrap();
    let resumed = wait_for(&server, &session, |s| {
        !s.running && s.items.last().is_some_and(|row| row.text == "Continued")
    })
    .await;
    assert!(resumed.error.is_none() && resumed.error_kind.is_none());
    let asked = seen.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0]
        .iter()
        .any(|text| text.contains("Summary of the earlier turns")));
    assert_eq!(
        server
            .whole_transcript(&session)
            .await
            .unwrap()
            .iter()
            .filter(|message| message.content.as_text() == "and summarise")
            .count(),
        1
    );
    server.shutdown().await;
}

#[tokio::test]
async fn undoing_the_latest_turn_puts_its_files_back() {
    use desktop::TurnRef;
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), true))
        .await
        .unwrap();
    assert!(!server
        .commands()
        .iter()
        .any(|command| command.contains("undo")));
    let answer = cfg.workspace.join("answer.txt");
    let write = |session: String| {
        let server = &server;
        async move {
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    desktop::ConversationOptions {
                        full_access: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            send(server, &session, "write the answer").await.unwrap();
            wait_for(server, &session, |s| !s.running).await;
        }
    };
    let turn = TurnRef {
        from_end: 0,
        text: "write the answer".into(),
    };

    // A file the turn created goes to the project's trash.
    let created = create(&server).await;
    write(created.clone()).await;
    assert_eq!(std::fs::read_to_string(&answer).unwrap(), "offline result");
    let earlier = TurnRef {
        from_end: 1,
        ..turn.clone()
    };
    assert!(server.undo_turn(PROJECT, &created, &earlier).await.is_err());
    let (text, files) = server.undo_turn(PROJECT, &created, &turn).await.unwrap();
    assert_eq!(text, "write the answer");
    assert_eq!(files.removed, ["answer.txt"]);
    assert!(
        files.restored.is_empty() && files.left.is_empty(),
        "{files:?}"
    );
    assert!(!answer.exists());
    let trash: Vec<_> = std::fs::read_dir(cfg.workspace.join(".wisp/trash"))
        .unwrap()
        .collect();
    assert_eq!(trash.len(), 1);
    assert!(snapshot(&server, &created).await.items.is_empty());

    // A file the turn overwrote gets its earlier content back.
    std::fs::write(&answer, "what the researcher wrote").unwrap();
    let overwrote = create(&server).await;
    write(overwrote.clone()).await;
    assert_eq!(std::fs::read_to_string(&answer).unwrap(), "offline result");
    let (_, files) = server.undo_turn(PROJECT, &overwrote, &turn).await.unwrap();
    assert_eq!(files.restored, ["answer.txt"]);
    assert_eq!(
        std::fs::read_to_string(&answer).unwrap(),
        "what the researcher wrote"
    );

    // A file changed again since the turn is left as it is, and named.
    let changed = create(&server).await;
    write(changed.clone()).await;
    std::fs::write(&answer, "edited by hand afterwards").unwrap();
    let (_, files) = server.undo_turn(PROJECT, &changed, &turn).await.unwrap();
    assert!(files.restored.is_empty() && files.removed.is_empty());
    assert_eq!(files.left.len(), 1);
    assert_eq!(files.left[0].0, "answer.txt");
    assert!(files.left[0].1.contains("changed again"), "{files:?}");
    assert_eq!(
        std::fs::read_to_string(&answer).unwrap(),
        "edited by hand afterwards"
    );
    // The conversation is rewound all the same.
    assert!(snapshot(&server, &changed).await.items.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_is_named_after_its_first_message_on_one_line() {
    assert_eq!(
        crate::title_of("  [Scheduled task: report]\n\nSummarise\tthe   week  "),
        "[Scheduled task: report] Summarise the week"
    );
    assert_eq!(crate::title_of(&"字".repeat(100)).chars().count(), 80);
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    send(
        &server,
        &session,
        "align the reads\nagainst GRCh38\n\nthen count",
    )
    .await
    .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        server
            .0
            .store
            .session_title(&session)
            .await
            .unwrap()
            .as_deref(),
        Some("align the reads against GRCh38 then count")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_skill_is_turned_into_a_workflow_to_review() {
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(ConvertingModel));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let root = std::path::PathBuf::from(server.project_root(PROJECT).await.unwrap());
    let skill = |name: &str, body: &str| {
        let folder = root.join("skills").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} method\n---\n{body}\n"),
        )
        .unwrap();
        folder
    };
    skill(
        "read-counting",
        "# Read counting\n\nDivide the line count by four.",
    );
    let scripted = skill("scripted", "# Scripted\n\nRun the script.");
    std::fs::create_dir_all(scripted.join("scripts")).unwrap();
    std::fs::write(scripted.join("scripts/run.py"), "print(1)\n").unwrap();
    let before = server.workflows().await.unwrap();

    // With no skill picked, the model picks one; the workflow it writes
    // comes back as a template to review.
    let drafted = server
        .workflow_from_skills(PROJECT, "Count the reads in my FASTQ files", &[])
        .await
        .unwrap();
    assert!(drafted.id.is_empty());
    assert_eq!(drafted.name, "From read-counting");
    assert_eq!(drafted.goal, "Count the reads and report them");
    assert_eq!(drafted.context, "Report counts per file.");
    let titles: Vec<&str> = drafted
        .tasks
        .iter()
        .map(|task| task.title.as_str())
        .collect();
    assert_eq!(titles, ["Count reads", "Report"]);
    assert_eq!(drafted.tasks[1].after, [0]);
    assert_eq!(drafted.tasks[0].capabilities, ["project_read"]);
    assert!(drafted.tasks[0].agent.output_schema.is_some());
    // Nothing is kept until it is saved.
    assert_eq!(server.workflows().await.unwrap(), before);
    let saved = server.save_workflow(drafted).await.unwrap();
    assert_eq!(saved.len(), before.len() + 1);

    // A skill that runs scripts of its own cannot be turned into one, nor
    // can one that is not there.
    for picked in ["scripted", "absent"] {
        assert!(server
            .workflow_from_skills(PROJECT, "Run it", &[picked.to_string()])
            .await
            .is_err());
    }
    assert!(server
        .workflow_from_skills(PROJECT, "  ", &["read-counting".to_string()])
        .await
        .is_err());
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_terminal_runs_in_the_app_and_answers_what_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let root = std::path::PathBuf::from(server.project_root(PROJECT).await.unwrap());
    let script = "printf 'hello from %s\\n' \"$(basename \"$PWD\")\"; read line; printf 'got %s\\n' \"$line\"";
    let info = server
        .start_terminal(
            PROJECT,
            "local",
            "Test".into(),
            crate::terminals::Launch {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                cwd: Some(root.clone()),
                display_cwd: root.display().to_string(),
                envs: Vec::new(),
            },
        )
        .unwrap();
    assert!(info.running);
    assert_eq!(server.terminals(PROJECT).len(), 1);
    assert!(server.terminals("other").is_empty());

    // Reads by offset until what is wanted has been written.
    use base64::Engine as _;
    let decode = |output: &wisp_dto::native_conversations::TerminalOutput| {
        String::from_utf8_lossy(
            &base64::engine::general_purpose::STANDARD
                .decode(&output.base64)
                .unwrap(),
        )
        .into_owned()
    };
    let until = |wanted: &'static str| {
        let server = server.clone();
        let id = info.id.clone();
        async move {
            let mut seen = String::new();
            let mut cursor = None;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let output = server
                        .read_terminal(PROJECT, &id, cursor, std::time::Duration::from_millis(500))
                        .await
                        .unwrap();
                    seen.push_str(&decode(&output));
                    cursor = Some(output.end);
                    if seen.contains(wanted) {
                        return output;
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("never saw {wanted:?} in {seen:?}"))
        }
    };
    let folder = root.file_name().unwrap().to_string_lossy().into_owned();
    let first = until("hello from").await;
    assert!(decode(&first).contains(&folder) || first.reset);
    server.resize_terminal(PROJECT, &info.id, 40, 120).unwrap();
    assert!(server.resize_terminal(PROJECT, &info.id, 0, 120).is_err());
    server.write_terminal(PROJECT, &info.id, b"abc\r").unwrap();
    until("got abc").await;
    // It ends by itself; reading again from nothing redraws all of it.
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let output = server
                .read_terminal(
                    PROJECT,
                    &info.id,
                    None,
                    std::time::Duration::from_millis(200),
                )
                .await
                .unwrap();
            if output.exit_code.is_some() {
                return output;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ended.exit_code, Some(0));
    assert!(ended.reset);
    assert!(decode(&ended).contains("got abc"));
    assert!(!server.terminals(PROJECT)[0].running);
    assert!(server.write_terminal(PROJECT, &info.id, b"more").is_err());
    // Only its own project reads or closes it.
    assert!(server
        .read_terminal("other", &info.id, None, std::time::Duration::ZERO)
        .await
        .is_err());
    server.close_terminal(PROJECT, &info.id).unwrap();
    assert!(server.terminals(PROJECT).is_empty());
    assert!(server.close_terminal(PROJECT, &info.id).is_err());

    // A local terminal opens only inside the project's folder.
    assert!(server
        .open_terminal(PROJECT, None, Some("../elsewhere"))
        .await
        .is_err());
    assert!(server
        .open_terminal(PROJECT, None, Some("missing"))
        .await
        .is_err());
    server.shutdown().await;
}

/// Answers in two halves: the first cut at the output limit.
struct TruncatingModel(Arc<AtomicUsize>);

#[async_trait]
impl Provider for TruncatingModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "{}".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        _: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        let asked = messages
            .iter()
            .rev()
            .find(|m| m.role == wisp_llm::Role::User)
            .map(|m| m.content.as_text())
            .unwrap_or_default();
        let call = self.0.fetch_add(1, Ordering::SeqCst);
        let (text, finish) = if call == 0 || asked.contains("write it out") {
            ("The first half, cut", "length")
        } else {
            ("and the second half.", "stop")
        };
        sink.on_text(text);
        Ok(Completion {
            content: text.into(),
            finish_reason: Some(finish.into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn an_answer_cut_at_the_output_limit_carries_on_when_asked_to() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(TruncatingModel(counted.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let start = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            auto_continue: true,
            ..start
        })
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "Tell me").await.unwrap();
    turn_done(&server, &session).await;
    // It was cut once, and the turn went on by itself to the end.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(texts_of(&server, &session)
        .await
        .iter()
        .any(|text| text.contains("and the second half.")));

    // Off, the cut answer is where the turn stops.
    server
        .set_turn_defaults(desktop::TurnDefaults {
            auto_continue: false,
            ..start
        })
        .await
        .unwrap();
    calls.store(0, Ordering::SeqCst);
    let other = create(&server).await;
    send(&server, &other, "Tell me").await.unwrap();
    turn_done(&server, &other).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn usage_ranks_the_skills_and_mcp_tools_the_model_called() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let store = &server.0.store;
    crate::mcp::save(
        store,
        &[crate::mcp::McpServer {
            id: "lab".into(),
            name: "Lab".into(),
            enabled: true,
            transport: crate::mcp::McpTransport::Http {
                url: "https://lab.example/mcp".into(),
            },
            env: vec![],
            headers: vec![],
            proxy: String::new(),
        }],
    )
    .await
    .unwrap();
    let calls = |names: &[(&str, &str)]| {
        let mut message = Message::assistant("");
        message.tool_calls = names
            .iter()
            .enumerate()
            .map(|(index, (name, arguments))| ToolCall {
                id: format!("call-{index}"),
                kind: "function".into(),
                function: wisp_llm::FunctionCall {
                    name: (*name).into(),
                    arguments: (*arguments).into(),
                },
            })
            .collect();
        message
    };
    let session = create(&server).await;
    store
        .append_message(
            &session,
            1,
            &calls(&[
                ("use_skill", r#"{"name": "read-counting"}"#),
                ("lab__search", "{}"),
                ("read", r#"{"path": "a.txt"}"#),
            ]),
        )
        .await
        .unwrap();
    store
        .append_message(
            &session,
            2,
            &calls(&[
                ("use_skill", r#"{"name": "read-counting"}"#),
                ("use_skill", "not json"),
                ("gone__tool", "{}"),
            ]),
        )
        .await
        .unwrap();
    let usage = server.usage().await.unwrap();
    let ranked: Vec<(&str, &str, i64)> = usage
        .tools
        .iter()
        .map(|tool| (tool.kind.as_str(), tool.name.as_str(), tool.calls))
        .collect();
    // This host's own tools, a skill call that named none and a server
    // that is no longer set up are left out.
    assert_eq!(
        ranked,
        [("skill", "read-counting", 2), ("mcp", "Lab/search", 1)]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_memory_is_drafted_from_a_turn_for_the_researcher_to_keep() {
    /// Answers, and drafts a memory when asked for one.
    struct Drafting;
    #[async_trait]
    impl Provider for Drafting {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let drafting = messages[0]
                .content
                .as_text()
                .contains("You prepare a memory draft");
            let evidence = messages
                .last()
                .map(|m| m.content.as_text())
                .unwrap_or_default();
            let content = if drafting && evidence.contains("log scale") {
                r#"{"scope": "global", "content": "Plot counts on a log scale."}"#
            } else {
                "{}"
            };
            Ok(Completion {
                content: content.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("Noted.");
            Ok(Completion {
                content: "Noted.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(Drafting));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "Always plot counts on a log scale")
        .await
        .unwrap();
    turn_done(&server, &session).await;
    let draft = server
        .suggest_memory(PROJECT, &session, None)
        .await
        .unwrap();
    assert_eq!(draft.scope, "global");
    assert_eq!(draft.content, "Plot counts on a log scale.");
    let named = desktop::TurnRef {
        from_end: 0,
        text: "Always plot counts on a log scale".into(),
    };
    assert_eq!(
        server
            .suggest_memory(PROJECT, &session, Some(&named))
            .await
            .unwrap(),
        draft
    );
    // Nothing was kept, a turn that is not there has no draft, and only
    // the conversation's project asks.
    assert!(server.memory(PROJECT).await.unwrap().habits.is_empty());
    let absent = desktop::TurnRef {
        from_end: 3,
        text: "Never said".into(),
    };
    assert!(server
        .suggest_memory(PROJECT, &session, Some(&absent))
        .await
        .is_err());
    assert!(server
        .suggest_memory("other", &session, None)
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_new_project_can_keep_its_own_database_and_such_a_folder_opens_in_place() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    server.set_separate_project_storage(true).await.unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    server.create_conversation(&lab).await.unwrap();
    // Its records are in its folder, not in the host's database.
    assert!(folder.join(".wisp/project.sqlite").is_file());
    assert!(folder.join(".wisp/project.json").is_file());
    // A project made before the choice, or with it off, stays where it is.
    server.set_separate_project_storage(false).await.unwrap();
    let plain = temp.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    server.register_local_project(&plain).await.unwrap();
    assert!(!plain.join(".wisp/project.sqlite").exists());
    server.shutdown().await;
    drop(server);

    // Another host opens the folder where it is, its conversation with it.
    let other = tempfile::tempdir().unwrap();
    let target = Server::open_with_models(config(other.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let opened = target.import_project_folder(&folder).await.unwrap();
    assert_eq!(opened, lab);
    assert_eq!(
        target.0.store.list_sessions(&opened).await.unwrap().len(),
        1
    );
    target.shutdown().await;
}

#[tokio::test]
async fn the_webview_desktops_projects_are_listed_and_brought_over_from_a_copy() {
    let temp = tempfile::tempdir().unwrap();
    // The WebView's data: a project kept in its database, one that keeps
    // its own in its folder, one whose folder is gone, and a hidden one.
    let database = temp.path().join("webview").join("wisp.sqlite");
    let kept = temp.path().join("kept");
    let own = temp.path().join("own");
    let hidden = temp.path().join("hidden");
    for folder in [&kept, &own, &hidden] {
        std::fs::create_dir(folder).unwrap();
    }
    std::fs::write(kept.join("counts.csv"), "gene,count\nA,1\n").unwrap();
    {
        let webview = wisp_store::Store::open_application(&database)
            .await
            .unwrap();
        webview
            .create_project("kept", "Kept study", kept.to_str().unwrap())
            .await
            .unwrap();
        webview
            .create_frame("kept-1", "kept", "agent", "model")
            .await
            .unwrap();
        webview
            .append_message("kept-1", 1, &Message::user("saved conversation"))
            .await
            .unwrap();
        // Never begun, so not counted.
        webview
            .create_frame("kept-empty", "kept", "agent", "model")
            .await
            .unwrap();
        webview.set_project_starred("kept", true).await.unwrap();
        webview
            .create_project("own", "Own study", own.to_str().unwrap())
            .await
            .unwrap();
        webview
            .create_frame("own-1", "own", "agent", "model")
            .await
            .unwrap();
        webview
            .append_message("own-1", 1, &Message::user("kept in its folder"))
            .await
            .unwrap();
        webview.migrate_project_storage("own").await.unwrap();
        webview
            .create_project(
                "gone",
                "Gone study",
                temp.path().join("gone").to_str().unwrap(),
            )
            .await
            .unwrap();
        webview
            .create_project("assistant:x", "Hidden", hidden.to_str().unwrap())
            .await
            .unwrap();
        // Settings of its own, of the copied project and of its
        // conversation.
        for (key, value) in [
            ("memory_enabled", "false"),
            ("skill_tags", r#"{"stats":["mine"]}"#),
            ("follow_up_questions", "false"),
            ("approval_scope:kept", "project"),
            ("frame_plan_mode:kept-1", "true"),
        ] {
            webview.set_setting(key, value).await.unwrap();
        }
    }
    // Made with the project, then moved away.
    std::fs::remove_dir_all(temp.path().join("gone")).unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    // A setting made here is never replaced.
    server
        .0
        .store
        .set_setting("follow_up_questions", "true")
        .await
        .unwrap();

    let data = server.webview_data(&database).await.unwrap();
    assert_eq!(data.settings, ["skills", "memory"]);
    assert!(data.models.is_empty());
    // Models are not taken while the environment sets the model.
    assert!(server
        .import_webview(&database, &[], &[], &["any".to_owned()])
        .await
        .is_err());
    let found = data.projects;
    let ids: Vec<&str> = found.iter().map(|project| project.id.as_str()).collect();
    // Starred first, then by name; the hidden one is left out.
    assert_eq!(ids, ["kept", "gone", "own"]);
    assert_eq!(found[0].conversations, Some(1));
    assert!(found[0].starred && found[0].folder_found && !found[0].own_database);
    assert!(!found[0].here);
    assert!(!found[1].folder_found);
    assert_eq!(found[2].conversations, None);
    assert!(found[2].own_database && found[2].folder_found);
    // Only a Wisp database that is there is read.
    let other = temp.path().join("other.sqlite");
    std::fs::write(&other, b"").unwrap();
    assert!(server.webview_data(&other).await.is_err());
    assert!(server
        .webview_data(&temp.path().join("missing.sqlite"))
        .await
        .is_err());

    let chosen: Vec<String> = ["kept", "own", "gone", "nothing"]
        .into_iter()
        .map(String::from)
        .collect();
    let settings: Vec<String> = ["memory", "follow_ups", "nothing"]
        .into_iter()
        .map(String::from)
        .collect();
    let outcome = server
        .import_webview(&database, &chosen, &settings, &[])
        .await
        .unwrap();
    assert_eq!(outcome.settings, ["memory"]);
    let setting = |key: &'static str| {
        let store = server.0.store.clone();
        async move { store.get_setting(key).await.unwrap() }
    };
    assert_eq!(setting("memory_enabled").await.as_deref(), Some("false"));
    assert_eq!(
        setting("follow_up_questions").await.as_deref(),
        Some("true")
    );
    assert_eq!(setting("skill_tags").await, None);
    let done = outcome.projects;
    let outcome: Vec<(&str, bool)> = done
        .iter()
        .map(|done| (done.id.as_str(), done.error.is_none()))
        .collect();
    assert!(done[1].error.is_none(), "{:?}", done[1].error);
    assert_eq!(
        outcome,
        [
            ("kept", true),
            ("own", true),
            ("gone", false),
            ("nothing", false)
        ]
    );
    assert!(
        done[2]
            .error
            .as_deref()
            .unwrap()
            .contains("not on this computer"),
        "{:?}",
        done[2].error
    );
    // The copied project uses its folder where it is, starred as it was.
    assert_eq!(server.0.store.list_sessions("kept").await.unwrap().len(), 1);
    let (_, _, root) = server
        .0
        .store
        .get_project_meta("kept")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(&root),
        std::fs::canonicalize(&kept).unwrap()
    );
    assert!(server
        .0
        .store
        .starred_project_ids()
        .await
        .unwrap()
        .contains("kept"));
    // Its settings and its conversation's came with it.
    assert_eq!(
        setting("approval_scope:kept").await.as_deref(),
        Some("project")
    );
    assert_eq!(
        setting("frame_plan_mode:kept-1").await.as_deref(),
        Some("true")
    );
    // The one with its own database is opened in its folder.
    assert_eq!(server.0.store.list_sessions("own").await.unwrap().len(), 1);
    let again = server.webview_data(&database).await.unwrap();
    assert_eq!(again.settings, ["skills"]);
    let here: Vec<String> = again
        .projects
        .into_iter()
        .filter(|project| project.here)
        .map(|project| project.id)
        .collect();
    assert_eq!(here, ["kept", "own"]);
    // A second time is refused, and the copy is not left behind.
    let again = server
        .import_webview(&database, &["kept".to_owned()], &[], &[])
        .await
        .unwrap();
    assert!(again.projects[0].error.is_some());
    assert!(!server.0.config.data.join("webview-import").exists());
    server.shutdown().await;

    // The WebView still has all it had.
    let webview = wisp_store::Store::open_application(&database)
        .await
        .unwrap();
    assert_eq!(webview.list_sessions("kept").await.unwrap().len(), 1);
    assert_eq!(webview.list_sessions("own").await.unwrap().len(), 1);
    assert!(webview.get_project("gone").await.unwrap().is_some());
}

#[tokio::test]
async fn the_webview_desktops_saved_models_come_with_their_keys() {
    use desktop::SecretStore;
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("webview").join("wisp.sqlite");
    {
        let webview = wisp_store::Store::open_application(&database)
            .await
            .unwrap();
        let profiles = json!([
            {"id": "wv-flash", "label": "Flash", "provider": "openai",
             "api_url": "https://api.example.invalid", "model": "flash-model",
             "max_tokens": 0, "context_window": 0},
            {"id": "wv-local", "label": "", "provider": "openai",
             "api_url": "http://127.0.0.1:9/", "endpoint_suffix": "/v1/",
             "model": "local-model", "max_tokens": 4000, "context_window": 32000,
             "proxy_url": "none", "has_api_key": true},
            // One it cannot read does not hide the rest.
            {"id": 5},
        ]);
        webview
            .set_setting("model_profiles", &profiles.to_string())
            .await
            .unwrap();
        webview
            .set_setting("active_model_id", "wv-local")
            .await
            .unwrap();
    }
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    secrets.set("model_key:wv-local", "made-up-key").unwrap();
    let server = Server::open_with(cfg, models(Arc::default(), false), secrets.clone())
        .await
        .unwrap();

    let data = server.webview_data(&database).await.unwrap();
    let listed: Vec<(&str, &str, bool, bool, bool)> = data
        .models
        .iter()
        .map(|model| {
            (
                model.id.as_str(),
                model.name.as_str(),
                model.has_key,
                model.active,
                model.here,
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("wv-flash", "Flash", false, false, false),
            ("wv-local", "local-model", true, true, false)
        ]
    );

    let chosen = ["wv-local".to_owned(), "wv-flash".to_owned()];
    let outcome = server
        .import_webview(&database, &[], &[], &chosen)
        .await
        .unwrap();
    // In the WebView's order.
    assert_eq!(outcome.models, ["wv-flash", "wv-local"]);
    let settings = server.model_settings().await.unwrap();
    let profile = |id: &str| {
        settings
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .unwrap()
            .clone()
    };
    let local = profile("wv-local");
    assert_eq!(local.fields.api_url, "http://127.0.0.1:9/v1");
    assert_eq!(
        (
            local.fields.max_context.as_str(),
            local.fields.max_tokens.as_str(),
            local.fields.proxy.as_str()
        ),
        ("32000", "4000", "none")
    );
    // The WebView's everyday model is the default, usable with its key.
    assert!(local.has_api_key && local.is_default, "{local:?}");
    assert_eq!(local.problem, None);
    let flash = profile("wv-flash");
    assert_eq!(flash.name, "Flash");
    assert!(!flash.has_api_key && flash.fields.max_context.is_empty());
    // The WebView's own key is left where it was.
    assert_eq!(
        secrets.get("model_key:wv-local").as_deref(),
        Some("made-up-key")
    );
    // Both are here now; bringing one again adds nothing.
    let again = server.webview_data(&database).await.unwrap();
    assert!(again.models.iter().all(|model| model.here));
    let twice = server
        .import_webview(&database, &[], &[], &["wv-local".to_owned()])
        .await
        .unwrap();
    assert!(twice.models.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn the_sidebar_pages_through_more_conversations_than_one_list_holds() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let page = desktop::CONVERSATION_PAGE;
    // Pinned first, so every conversation made after it is newer.
    let pinned = create(&server).await;
    server
        .pin_conversation(PROJECT, &pinned, true)
        .await
        .unwrap();
    let mut newest_first = Vec::new();
    for _ in 0..page + 5 {
        newest_first.insert(0, create(&server).await);
    }
    let ids = |list: &desktop::ConversationList| {
        list.rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>()
    };

    // One page: the newest hundred, the pinned one although it is older,
    // and word that there is more.
    let first = server.conversations(PROJECT, "", false).await.unwrap();
    assert!(first.more);
    assert_eq!(first.rows.len(), page + 1);
    assert_eq!(ids(&first)[..page], newest_first[..page]);
    assert_eq!(ids(&first)[page], pinned);
    // Two pages: everything, once each, the older ones under the first page.
    let both = server
        .conversation_pages(PROJECT, "", false, 2)
        .await
        .unwrap();
    assert!(!both.more);
    assert_eq!(both.rows.len(), page + 6);
    assert_eq!(ids(&both)[..page], newest_first[..page]);
    let mut rest = newest_first[page..].to_vec();
    rest.push(pinned.clone());
    assert_eq!(ids(&both)[page..], rest[..]);
    // Asking for more pages than there are is the same as asking for all.
    let all = server
        .conversation_pages(PROJECT, "", false, 5)
        .await
        .unwrap();
    assert_eq!((ids(&all), all.more), (ids(&both), false));

    // A search is cut off and paged the same way, so it misses nothing.
    let found = server
        .conversations(PROJECT, "new conversation", false)
        .await
        .unwrap();
    assert!(found.more);
    assert_eq!(found.rows.len(), page);
    let found = server
        .conversation_pages(PROJECT, "new conversation", false, 2)
        .await
        .unwrap();
    assert!(!found.more);
    assert_eq!(found.rows.len(), page + 6);

    // The phone and web clients' list is what it was: the newest hundred,
    // with no pinned row added and nothing said about more.
    let remote = call(&server, "remote_sessions", PROJECT, json!({}))
        .await
        .unwrap();
    let remote: Vec<String> = remote
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(remote, newest_first[..page]);
    server.shutdown().await;
}
