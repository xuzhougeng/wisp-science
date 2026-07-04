//! The agent loop: read → think → tool-call → verify, until the model stops
//! or calls `attempt_completion`. Ported from mangopi-cli's `agent_loop`,
//! retuned for streaming + the shared `Output` sink.

use crate::context::{image_content, ContextManager};
use crate::output::{StreamSinkAdapter, ToolEnvAdapter};
use crate::Output;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use wisp_llm::{Content, Provider};
use wisp_tools::Registry;

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
        let mut sink = StreamSinkAdapter::new(output);
        let comp = provider
            .stream(&messages, &tools.schemas(), &mut sink)
            .await?;

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
            // A turn truncated at the model's token cap also arrives with no tool
            // calls, looking identical to a clean finish — so the run would stop
            // mid-task with no explanation (#55, seen with GLM-5.2 hitting
            // max_tokens). Surface it instead of silently cutting progress off.
            if is_truncated(comp.finish_reason.as_deref()) {
                anyhow::bail!("模型输出在达到 max_tokens 上限时被截断，任务可能尚未完成——请在设置中调高该模型的 max_tokens，或直接继续对话让我接着做。(output truncated at max_tokens)");
            }
            break;
        }

        let mut completed = false;
        for tc in &comp.tool_calls {
            let name = tc.function.name.clone();
            let args = tc.args_value();
            let result = tools.run(&name, &args, &env).await;
            let content = if let Some(img) = &result.image {
                image_content(&img.label, &img.data_url)
            } else {
                Content::text(result.content.clone())
            };
            output.tool_result(&name, result.success, &result.content);
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

/// A turn with no tool calls is normally a clean finish, but when the provider
/// reports a token-cap stop the response was cut off mid-thought and the run
/// should not be treated as complete. OpenAI-compatible APIs (incl. GLM) report
/// `"length"`; Anthropic passes `"max_tokens"` through unmapped.
fn is_truncated(finish_reason: Option<&str>) -> bool {
    matches!(finish_reason, Some("length") | Some("max_tokens"))
}

#[cfg(test)]
mod tests {
    use super::is_truncated;

    #[test]
    fn truncation_detected_across_providers() {
        // Both the OpenAI/GLM cap signal and the Anthropic cap signal count, so a
        // capped turn is reported instead of silently ending mid-task (#55).
        assert!(is_truncated(Some("length")));
        assert!(is_truncated(Some("max_tokens")));
        // Clean finishes and tool-call turns must NOT look like truncation, or
        // every normal turn-end would raise a spurious error.
        assert!(!is_truncated(Some("stop")));
        assert!(!is_truncated(Some("tool_calls")));
        assert!(!is_truncated(None));
    }
}
