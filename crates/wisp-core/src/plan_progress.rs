//! Track accepted execution plans through one loop pass, including compaction.
//! A review/Stop continuation restores the latest accepted snapshot from history.

use crate::ContextManager;
use wisp_llm::{Message, Role};
use wisp_tools::plan::render_plan;

#[derive(Debug)]
struct PlanSnapshot {
    rendered: String,
    unfinished: bool,
}

impl PlanSnapshot {
    fn from_args(args: &serde_json::Value) -> Option<Self> {
        let rendered = render_plan(args).ok()?;
        let unfinished = args["steps"].as_array()?.iter().any(|step| {
            matches!(
                step["status"].as_str().unwrap_or("pending"),
                "pending" | "in_progress"
            )
        });
        Some(Self {
            rendered,
            unfinished,
        })
    }
}

#[derive(Default)]
pub(crate) struct PlanProgress {
    latest: Option<PlanSnapshot>,
    reminded: bool,
    reminder_index: Option<usize>,
}

impl PlanProgress {
    pub(crate) fn from_history(messages: &[Message]) -> Self {
        let turn_start = messages
            .iter()
            .rposition(|message| message.role == Role::User)
            .unwrap_or(0);
        let turn = &messages[turn_start..];
        let latest = turn.iter().enumerate().rev().find_map(|(index, message)| {
            if message.role != Role::Tool || message.tool_name.as_deref() != Some("update_plan") {
                return None;
            }
            let id = message.tool_call_id.as_deref()?;
            let call = turn[..index]
                .iter()
                .rev()
                .flat_map(|message| &message.tool_calls)
                .find(|call| call.id == id && call.function.name == "update_plan")?;
            let plan = PlanSnapshot::from_args(&call.args_value())?;
            // Messages do not store ToolResult.success. Only the tool's validated
            // checklist (optionally followed by hook feedback) is an accepted
            // snapshot; rejected, skipped or interrupted calls never replace it.
            let output = message.content.as_text();
            let suffix = output.strip_prefix(&plan.rendered)?;
            (suffix.is_empty() || suffix.starts_with("\n\n[Hook feedback]\n")).then_some(plan)
        });
        Self {
            latest,
            reminded: false,
            reminder_index: None,
        }
    }

    pub(crate) fn accept(&mut self, args: &serde_json::Value) {
        self.latest = PlanSnapshot::from_args(args);
    }

    /// One reminder per loop pass, never an instruction to redo the task or
    /// manufacture completion. The caller enforces cancellation/iteration limits.
    pub(crate) fn remind(&mut self, ctx: &mut ContextManager) -> bool {
        if self.reminded {
            return false;
        }
        let Some(plan) = self.latest.as_ref().filter(|plan| plan.unfinished) else {
            return false;
        };
        let snapshot = if plan.rendered.len() > 8_000 {
            ContextManager::truncate_middle(
                &plan.rendered,
                4_000,
                4_000,
                "[... plan excerpt; consult the earlier update_plan for all steps ...]",
            )
        } else {
            plan.rendered.clone()
        };
        self.reminded = true;
        self.reminder_index = Some(ctx.runtime_injections.len());
        ctx.inject_user(format!(
            "Before ending this turn, reconcile the latest accepted execution plan with the work \
             actually done, including any automatic-review corrections or Stop-hook work. Call \
             update_plan with the full step list: mark only verified finished steps completed; \
             mark work explicitly removed by the user cancelled. Keep blocked or unfinished work \
             pending/in_progress and explain it in the final answer. Do not claim completion merely \
             because the turn is ending, repeat completed work, or start unrelated work. This is a \
             single plan-status reminder; if no status can change, explain what remains and finish.\n\n\
             Latest accepted plan:\n{snapshot}"
        ));
        true
    }

    pub(crate) fn clear_reminder(&mut self, ctx: &mut ContextManager) {
        if let Some(index) = self.reminder_index.take() {
            // Keep host/review injections and any later auto-continue notices.
            ctx.runtime_injections.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_llm::{FunctionCall, ToolCall};

    fn args(status: &str) -> serde_json::Value {
        json!({"steps": [{"step": "Check results", "status": status}]})
    }

    fn append_plan(ctx: &mut ContextManager, id: &str, args: serde_json::Value, output: &str) {
        ctx.append_assistant(
            String::new(),
            vec![ToolCall {
                id: id.into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "update_plan".into(),
                    arguments: args.to_string(),
                },
            }],
            None,
        );
        ctx.messages.push(Message::tool(id, "update_plan", output));
    }

    #[test]
    fn restores_only_accepted_current_turn_plans() {
        let mut ctx = ContextManager::new(100_000);
        ctx.append_user("Analyze");
        let running = args("in_progress");
        append_plan(
            &mut ctx,
            "initial",
            running.clone(),
            &format!(
                "{}\n\n[Hook feedback]\nKeep tracking",
                render_plan(&running).unwrap()
            ),
        );
        // A denied update with otherwise valid arguments must not claim completion.
        append_plan(&mut ctx, "denied", args("completed"), "denied by the user");
        append_plan(
            &mut ctx,
            "skipped",
            args("completed"),
            "Skipped after ask_user",
        );
        ctx.inject_user("An automatic reviewer requested corrections");
        let mut progress = PlanProgress::from_history(&ctx.messages);
        assert!(progress.remind(&mut ctx));
        let prompt = ctx.runtime_injections.last().unwrap().content.as_text();
        assert!(prompt.contains("[~] Check results"));
        assert!(prompt.contains("only verified finished steps completed"));
        progress.clear_reminder(&mut ctx);
        assert_eq!(ctx.runtime_injections.len(), 1);

        // A real new user turn must not revive this turn's stale plan.
        ctx.append_user("Unrelated question");
        assert!(!PlanProgress::from_history(&ctx.messages).remind(&mut ctx));
    }

    #[test]
    fn latest_terminal_snapshot_supersedes_unfinished_history() {
        for status in ["completed", "cancelled"] {
            let mut ctx = ContextManager::new(100_000);
            ctx.append_user("Analyze");
            let initial = args("pending");
            append_plan(
                &mut ctx,
                "initial",
                initial.clone(),
                &render_plan(&initial).unwrap(),
            );
            let finished = args(status);
            append_plan(
                &mut ctx,
                "finished",
                finished.clone(),
                &render_plan(&finished).unwrap(),
            );
            assert!(
                !PlanProgress::from_history(&ctx.messages).remind(&mut ctx),
                "{status}"
            );
        }
    }

    #[test]
    fn unsuccessful_and_unpaired_plans_never_create_a_reminder() {
        for output in [
            "Plan rejected by the user.",
            "interrupted (unpaired on load)",
            "",
        ] {
            let mut ctx = ContextManager::new(100_000);
            ctx.append_user("Analyze");
            append_plan(&mut ctx, "failed", args("in_progress"), output);
            assert!(!PlanProgress::from_history(&ctx.messages).remind(&mut ctx));
        }
    }

    #[test]
    fn snapshot_survives_compaction_and_reminder_is_bounded_and_scoped() {
        let mut ctx = ContextManager::new(100_000);
        ctx.inject_user("Host context");
        let mut progress = PlanProgress::default();
        progress.accept(&json!({"steps": [{"step": "Analyze"}]}));
        // Compaction can replace the transcript after the plan was accepted.
        ctx.messages = vec![Message::user("Compaction checkpoint")];
        assert!(progress.remind(&mut ctx));
        assert!(!progress.remind(&mut ctx));
        // Even revising/reopening the plan cannot start an endless reminder loop.
        progress.accept(&args("in_progress"));
        assert!(!progress.remind(&mut ctx));
        ctx.inject_user("Later runtime note");
        progress.clear_reminder(&mut ctx);
        assert!(!progress.remind(&mut ctx));
        assert_eq!(
            ctx.runtime_injections
                .iter()
                .map(|message| message.content.as_text())
                .collect::<Vec<_>>(),
            ["Host context", "Later runtime note"]
        );
    }

    #[test]
    fn long_plan_reminder_keeps_a_bounded_utf8_excerpt() {
        let mut ctx = ContextManager::new(100_000);
        let mut progress = PlanProgress::default();
        progress.accept(&json!({"steps": [
            {"step": format!("检查{}", "输入".repeat(5_000)), "status": "in_progress"},
            {"step": "最后复核", "status": "pending"}
        ]}));
        assert!(progress.remind(&mut ctx));
        let prompt = ctx.runtime_injections.last().unwrap().content.as_text();
        assert!(prompt.len() < 10_000);
        assert!(prompt.contains("[... plan excerpt"));
        assert!(prompt.ends_with("[ ] 最后复核"));
    }
}
