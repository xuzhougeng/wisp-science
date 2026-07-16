//! The agent loop: read → think → tool-call → verify, until the model stops
//! or calls `attempt_completion`. Ported from mangopi-cli's `agent_loop`,
//! retuned for streaming + the shared `Output` sink.

use crate::context::{image_content, ContextManager};
use crate::output::{StreamSinkAdapter, ToolEnvAdapter};
use crate::provenance;
use crate::Output;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wisp_llm::{is_retriable, Completion, Content, LlmError, Message, Provider, ToolSchema};
use wisp_tools::{ImageData, Registry, ToolEnv};

const RETRY_DELAYS: [u64; 3] = [1_000, 5_000, 10_000];
const TRUNCATED_OUTPUT_MESSAGE: &str = "模型输出在达到 max_tokens 上限时被截断，任务可能尚未完成——请在设置中调高该模型的 max_tokens，或直接继续对话让我接着做。(output truncated at max_tokens)";

pub async fn agent_loop(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
    vision_provider: Option<&dyn Provider>,
    tools: &Registry,
    root: &Path,
    output: &dyn Output,
    user_input: &str,
    max_iter: usize,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    ctx.append_user(user_input);
    if let Some(m) = ctx.messages.last() {
        output.on_message(m);
    }
    agent_loop_inner(
        ctx,
        provider,
        vision_provider,
        tools,
        root,
        output,
        max_iter,
        cancel,
    )
    .await
}

/// Continue a turn after a transient failure — context already has the user
/// message and any tool results from before the error.
pub async fn agent_loop_continue(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
    vision_provider: Option<&dyn Provider>,
    tools: &Registry,
    root: &Path,
    output: &dyn Output,
    max_iter: usize,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    agent_loop_inner(
        ctx,
        provider,
        vision_provider,
        tools,
        root,
        output,
        max_iter,
        cancel,
    )
    .await
}

async fn agent_loop_inner(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
    vision_provider: Option<&dyn Provider>,
    tools: &Registry,
    root: &Path,
    output: &dyn Output,
    max_iter: usize,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let env = match cancel {
        Some(c) => ToolEnvAdapter::with_cancel(root.to_path_buf(), output, c),
        None => ToolEnvAdapter::new(root.to_path_buf(), output),
    };
    let mut iteration = 0usize;
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            anyhow::bail!("stopped by user");
        }
        iteration += 1;
        let messages = ctx.prepare_for_api(provider, output).await;
        let mut sink = match cancel {
            Some(c) => StreamSinkAdapter::with_cancel(output, c),
            None => StreamSinkAdapter::new(output),
        };
        let comp =
            stream_with_retry(provider, &messages, &tools.schemas(), &mut sink, cancel).await?;
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            anyhow::bail!("stopped by user");
        }
        if is_truncated(comp.finish_reason.as_deref()) {
            anyhow::bail!(TRUNCATED_OUTPUT_MESSAGE);
        }

        ctx.append_assistant(
            comp.content.clone(),
            comp.tool_calls.clone(),
            comp.reasoning.clone(),
        );
        if let Some(m) = ctx.messages.last() {
            output.on_message(m);
        }
        output.usage(
            iteration,
            comp.usage.input_tokens,
            comp.usage.output_tokens,
            ctx.total_tokens(),
            ctx.max_context,
        );

        if comp.tool_calls.is_empty() {
            break;
        }

        let mut completed = false;
        for tc in &comp.tool_calls {
            let name = tc.function.name.clone();
            let args = tc.args_value();
            let producing = provenance::is_producing(&name);
            let root = producing.then(|| env.project_root().to_path_buf());
            let before = if let Some(root) = root.clone() {
                tokio::task::spawn_blocking(move || provenance::snapshot(&root))
                    .await
                    .unwrap_or_default()
            } else {
                Default::default()
            };
            let t0 = std::time::Instant::now();
            let result = tools.run(&name, &args, &env).await;
            let duration_ms = t0.elapsed().as_millis() as u64;
            if let Some(root) = &root {
                let root2 = root.clone();
                let after = tokio::task::spawn_blocking(move || provenance::snapshot(&root2))
                    .await
                    .unwrap_or_default();
                let source = provenance::source_of(&name, &args);
                let (written, read) = provenance::diff(&before, &after, root, &source);
                if !written.is_empty() {
                    output.provenance(&provenance::ProvenanceRecord {
                        tool: name.clone(),
                        language: provenance::language_of(&name),
                        source,
                        output: result.content.clone(),
                        success: result.success,
                        files_written: written,
                        files_read: read,
                    });
                }
            }
            let (content, tool_text, ok) = if let Some(img) = &result.image {
                match vision_provider {
                    Some(vision) => match describe_image(vision, img, &name, &args).await {
                        Ok(text) => (Content::text(text.clone()), text, true),
                        Err(e) => {
                            let text = format!("view_image error: vision model failed: {e}");
                            (Content::text(text.clone()), text, false)
                        }
                    },
                    None => {
                        let text = "view_image error: no vision model is configured. Mark an API model as vision-capable in Settings -> Models and set it for image analysis.".to_string();
                        (Content::text(text.clone()), text, false)
                    }
                }
            } else {
                (
                    Content::text(result.content.clone()),
                    result.content.clone(),
                    result.success,
                )
            };
            output.tool_result(&name, ok, &tool_text, duration_ms);
            ctx.append_tool(&tc.id, &name, content);
            if let Some(m) = ctx.messages.last() {
                output.on_message(m);
            }
            if name == "attempt_completion" {
                completed = true;
                break;
            }
        }
        if completed {
            break;
        }
        if iteration_limit_reached(iteration, max_iter) {
            break;
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            anyhow::bail!("stopped by user");
        }
    }
    Ok(())
}

fn iteration_limit_reached(iteration: usize, max_iter: usize) -> bool {
    max_iter != 0 && iteration >= max_iter
}

async fn describe_image(
    provider: &dyn Provider,
    img: &ImageData,
    tool_name: &str,
    args: &serde_json::Value,
) -> std::result::Result<String, LlmError> {
    let question = args
        .get("question")
        .or_else(|| args.get("prompt"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Describe the image carefully. Extract visible text, labels, plots, UI state, notable scientific content, and uncertainties.");
    let user = Message {
        role: wisp_llm::Role::User,
        content: image_content(
            &format!("Tool: {tool_name}\n{}\n\nTask: {question}", img.label),
            &img.data_url,
        ),
        tool_calls: vec![],
        tool_call_id: None,
        tool_name: None,
        reasoning: None,
        ts: chrono::Utc::now().timestamp(),
        model_name: None,
    };
    let comp = provider
        .complete(
            &[
                Message::system("You are Wisp's vision subagent. Return concise, factual observations for a non-visual main agent. Do not invent details that are not visible."),
                user,
            ],
            &[],
        )
        .await?;
    let observed = comp.content.trim();
    if observed.is_empty() {
        return Err(LlmError::Incomplete);
    }
    Ok(format!(
        "{}\nVision model: {}\n\n{}",
        img.label,
        provider.model(),
        observed
    ))
}

async fn stream_with_retry(
    provider: &dyn Provider,
    messages: &[Message],
    schemas: &[ToolSchema],
    sink: &mut StreamSinkAdapter<'_>,
    cancel: Option<&AtomicBool>,
) -> Result<Completion, LlmError> {
    let mut last = None;
    for attempt in 0..=RETRY_DELAYS.len() {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(LlmError::Config("stopped by user".into()));
        }
        match provider.stream(messages, schemas, sink).await {
            Ok(c) => return Ok(c),
            Err(e) => {
                if !is_retriable(&e) || attempt == RETRY_DELAYS.len() {
                    return Err(e);
                }
                tracing::warn!("LLM stream failed (attempt {}), retrying: {e}", attempt + 1);
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(RETRY_DELAYS[attempt])).await;
            }
        }
    }
    Err(last.expect("retry loop always returns or breaks"))
}

fn is_truncated(finish_reason: Option<&str>) -> bool {
    matches!(finish_reason, Some("length") | Some("max_tokens"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::NullOutput;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use wisp_llm::{FunctionCall, ToolCall};
    use wisp_tools::{Registry, Tool, ToolEnv, ToolResult};

    #[test]
    fn truncation_detected_across_providers() {
        assert!(is_truncated(Some("length")));
        assert!(is_truncated(Some("max_tokens")));
        assert!(!is_truncated(Some("stop")));
        assert!(!is_truncated(Some("tool_calls")));
        assert!(!is_truncated(None));
    }

    #[test]
    fn zero_max_iter_disables_the_iteration_limit() {
        assert!(!iteration_limit_reached(usize::MAX, 0));
        assert!(!iteration_limit_reached(99, 100));
        assert!(iteration_limit_reached(100, 100));
    }

    struct FixedProvider {
        completion: Completion,
    }

    #[async_trait]
    impl Provider for FixedProvider {
        fn name(&self) -> &str {
            "fixed"
        }

        fn model(&self) -> &str {
            "fixed"
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            Ok(self.completion.clone())
        }

        async fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
            _sink: &mut dyn wisp_llm::StreamSink,
        ) -> wisp_llm::Result<Completion> {
            Ok(self.completion.clone())
        }
    }

    static SPY_RAN: AtomicBool = AtomicBool::new(false);

    struct SpyTool;

    #[async_trait]
    impl Tool for SpyTool {
        fn name(&self) -> &str {
            "spy"
        }

        fn schema(&self) -> ToolSchema {
            ToolSchema::new(
                "spy",
                "test spy tool",
                serde_json::json!({"type": "object"}),
            )
        }

        async fn run(&self, _args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
            SPY_RAN.store(true, Ordering::SeqCst);
            ToolResult::ok("ran")
        }
    }

    #[tokio::test]
    async fn truncated_tool_call_is_not_executed() {
        SPY_RAN.store(false, Ordering::SeqCst);
        let provider = FixedProvider {
            completion: Completion {
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "spy".into(),
                        arguments: r#"{"cmd":"ssh CPU3 'cd /tmp && awk \"NR==1 || $1==\"AT1G324"}"#
                            .into(),
                    },
                }],
                finish_reason: Some("length".into()),
                ..Completion::default()
            },
        };
        let mut tools = Registry::builtins();
        tools.add(Box::new(SpyTool));
        let mut ctx = ContextManager::new(100_000);
        let root = std::env::temp_dir().join(format!(
            "wisp-core-truncated-tool-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();

        let err = agent_loop(
            &mut ctx,
            &provider,
            None,
            &tools,
            &root,
            &NullOutput,
            "run a command",
            1,
            None,
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("output truncated at max_tokens"),
            "unexpected error: {err}"
        );
        assert!(!SPY_RAN.load(Ordering::SeqCst), "truncated tool ran");
        assert_eq!(ctx.messages.len(), 1, "only the user message is persisted");
        std::fs::remove_dir_all(root).ok();
    }
}
