//! `update_plan` — the agent's create-and-track task plan.
//!
//! Stateless, TodoWrite-style: the model sends the ENTIRE ordered step list
//! every call (first call creates the plan, later calls update statuses). The
//! rendered checklist is returned as the tool result, so it both stays in the
//! model's context (tracking) and shows up in the tool-call card (user sees it)
//! without any new event plumbing.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;

pub struct UpdatePlanTool;

/// Prefix on the blocking-confirm message that marks a plan-approval pause, so
/// the Tauri host (`parse_confirm_payload`) can route it to the dedicated plan
/// card instead of the generic "Run tool 'X'?" approval. The checklist follows.
pub const PLAN_APPROVAL_PREFIX: &str = "[plan-approval]\n";

/// A freshly proposed plan is every step still `pending` — that's when we pause
/// for the user to sign off. Once any step is in_progress/completed the call is
/// a progress update and runs without re-asking.
/// ponytail: stateless heuristic (no stored prior plan); good enough — the tool
/// can't diff against a plan it never kept.
fn is_fresh_proposal(args: &Value) -> bool {
    args.get("steps")
        .and_then(|v| v.as_array())
        .is_some_and(|steps| {
            !steps.is_empty()
                && steps.iter().all(|s| {
                    s.get("status").and_then(|v| v.as_str()).unwrap_or("pending") == "pending"
                })
        })
}

/// Validate the `steps` argument and render it as a checklist, or return a
/// message the model can act on. Pure so it carries the unit tests below.
fn render_plan(args: &Value) -> Result<String, String> {
    let steps = args
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or("update_plan error: 'steps' must be an array of {step, status}")?;
    if steps.is_empty() {
        return Err("update_plan error: 'steps' must not be empty".into());
    }
    let (mut done, mut running, mut pending) = (0usize, 0usize, 0usize);
    let mut lines = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        let text = s
            .get("step")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| format!("update_plan error: step {} is missing 'step' text", i + 1))?;
        let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
        let marker = match status {
            "completed" => {
                done += 1;
                "[x]"
            }
            "in_progress" => {
                running += 1;
                "[~]"
            }
            "pending" => {
                pending += 1;
                "[ ]"
            }
            other => {
                return Err(format!(
                    "update_plan error: step {} has invalid status '{}' (use pending|in_progress|completed)",
                    i + 1,
                    other
                ))
            }
        };
        lines.push(format!("{marker} {text}"));
    }
    if running > 1 {
        return Err(format!(
            "update_plan error: {running} steps are in_progress; keep at most one in_progress at a time"
        ));
    }
    let header = format!(
        "Plan ({} steps · {done} done · {running} in progress · {pending} pending):",
        steps.len()
    );
    Ok(format!("{header}\n{}", lines.join("\n")))
}

fn step_counts(args: &Value) -> (usize, usize) {
    let Some(steps) = args.get("steps").and_then(|v| v.as_array()) else {
        return (0, 0);
    };
    let done = steps
        .iter()
        .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed"))
        .count();
    (done, steps.len())
}

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "update_plan",
            "Create and track a step-by-step plan for a multi-stage task. Call it once to lay out the \
             steps, then again after each step to update statuses. Send the ENTIRE ordered step list \
             every call — it replaces the previous plan, it is not a delta. Reach for it only when the \
             work is genuinely multi-stage (several analyses to sequence, long compute, a pipeline worth \
             showing the user); skip it for lookups, a single computation, or reading one file. Keep at \
             most one step 'in_progress' at a time.",
            json!({
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "The full ordered list of plan steps, resent in its entirety every call.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "string", "description": "Short imperative description of the step." },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Defaults to 'pending' if omitted."
                                }
                            },
                            "required": ["step"]
                        }
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Optional one-line note on what changed (e.g. why you re-planned)."
                    }
                },
                "required": ["steps"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        let (done, total) = step_counts(args);
        format!("{done}/{total} steps done")
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let rendered = match render_plan(args) {
            Ok(r) => r,
            Err(e) => return ToolResult::fail(e),
        };
        // A newly proposed plan pauses for the user to approve before work
        // begins; progress updates (any step already in flight) run silently.
        if is_fresh_proposal(args)
            && !env.confirm(&format!("{PLAN_APPROVAL_PREFIX}{rendered}")).await
        {
            return ToolResult::fail(
                "Plan rejected by the user. Revise the plan or ask what they want changed before proceeding.",
            );
        }
        ToolResult::ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_fresh_proposal, render_plan};
    use serde_json::json;

    #[test]
    fn fresh_proposal_only_when_all_pending() {
        assert!(is_fresh_proposal(&json!({"steps": [{"step": "a"}, {"step": "b", "status": "pending"}]})));
        assert!(!is_fresh_proposal(&json!({"steps": [{"step": "a", "status": "in_progress"}]})));
        assert!(!is_fresh_proposal(&json!({"steps": [{"step": "a", "status": "completed"}]})));
        assert!(!is_fresh_proposal(&json!({"steps": []})), "empty is not a proposal");
    }

    #[test]
    fn renders_markers_counts_and_default_status() {
        let out = render_plan(&json!({"steps": [
            {"step": "Load counts", "status": "completed"},
            {"step": "Run DESeq2", "status": "in_progress"},
            {"step": "Write report"}
        ]}))
        .unwrap();
        assert!(out.contains("[x] Load counts"), "{out}");
        assert!(out.contains("[~] Run DESeq2"), "{out}");
        assert!(out.contains("[ ] Write report"), "{out}"); // omitted status -> pending
        assert!(out.contains("3 steps · 1 done · 1 in progress · 1 pending"), "{out}");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(render_plan(&json!({})).is_err(), "missing steps");
        assert!(render_plan(&json!({"steps": []})).is_err(), "empty steps");
        assert!(render_plan(&json!({"steps": [{"step": "x", "status": "bogus"}]})).is_err(), "bad status");
        assert!(render_plan(&json!({"steps": [{"status": "pending"}]})).is_err(), "missing text");
        assert!(
            render_plan(&json!({"steps": [
                {"step": "a", "status": "in_progress"},
                {"step": "b", "status": "in_progress"}
            ]}))
            .is_err(),
            "two in_progress"
        );
    }
}
