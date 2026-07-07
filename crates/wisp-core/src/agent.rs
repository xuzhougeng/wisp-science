//! The agent loop: read → think → tool-call → verify, until the model stops
//! or calls `attempt_completion`. Ported from mangopi-cli's `agent_loop`,
//! retuned for streaming + the shared `Output` sink.

use crate::context::{image_content, ContextManager};
use crate::output::{StreamSinkAdapter, ToolEnvAdapter};
use crate::Output;
use crate::provenance;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wisp_llm::{Completion, Content, LlmError, Message, Provider, ToolSchema, is_retriable};
use wisp_tools::{Registry, ToolEnv};

const RETRY_DELAYS: [u64; 3] = [1_000, 5_000, 10_000];

pub async fn agent_loop(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
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
    agent_loop_inner(ctx, provider, tools, root, output, max_iter, cancel).await
}

/// Continue a turn after a transient failure — context already has the user
/// message and any tool results from before the error.
pub async fn agent_loop_continue(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
    tools: &Registry,
    root: &Path,
    output: &dyn Output,
    max_iter: usize,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    agent_loop_inner(ctx, provider, tools, root, output, max_iter, cancel).await
}

async fn agent_loop_inner(
    ctx: &mut ContextManager,
    provider: &dyn Provider,
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
        let comp = stream_with_retry(provider, &messages, &tools.schemas(), &mut sink, cancel).await?;
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            anyhow::bail!("stopped by user");
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
            if is_truncated(comp.finish_reason.as_deref()) {
                anyhow::bail!("模型输出在达到 max_tokens 上限时被截断，任务可能尚未完成——请在设置中调高该模型的 max_tokens，或直接继续对话让我接着做。(output truncated at max_tokens)");
            }
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
            let content = if let Some(img) = &result.image {
                image_content(&img.label, &img.data_url)
            } else {
                Content::text(result.content.clone())
            };
            output.tool_result(&name, result.success, &result.content, duration_ms);
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
        if iteration >= max_iter {
            break;
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            anyhow::bail!("stopped by user");
        }
    }
    Ok(())
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
    use super::is_truncated;

    #[test]
    fn truncation_detected_across_providers() {
        assert!(is_truncated(Some("length")));
        assert!(is_truncated(Some("max_tokens")));
        assert!(!is_truncated(Some("stop")));
        assert!(!is_truncated(Some("tool_calls")));
        assert!(!is_truncated(None));
    }
}
