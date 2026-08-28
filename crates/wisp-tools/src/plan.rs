//! `update_plan` — the agent's create-and-track task plan.
//!
//! Stateless, TodoWrite-style: the model sends the ENTIRE ordered step list
//! every call (first call creates the plan, later calls update statuses). The
//! rendered checklist is returned as the tool result, so it both stays in the
//! model's context (tracking) and shows up in the tool-call card (user sees it)
//! without any new event plumbing.
//!
//! `propose_plan` — plan mode's output channel, see [`ProposePlanTool`].

use crate::env::{ConfirmDecision, ToolEnv, ToolResult};
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
                    s.get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending")
                        == "pending"
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
    let (mut done, mut running, mut pending, mut cancelled) = (0usize, 0usize, 0usize, 0usize);
    let mut lines = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        let text = s
            .get("step")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| format!("update_plan error: step {} is missing 'step' text", i + 1))?;
        let status = s
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");
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
            "cancelled" => {
                cancelled += 1;
                "[-]"
            }
            other => {
                return Err(format!(
                    "update_plan error: step {} has invalid status '{}' (use pending|in_progress|completed|cancelled)",
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
        "Plan ({} steps · {done} done · {running} in progress · {pending} pending · {cancelled} cancelled):",
        steps.len()
    );
    Ok(format!("{header}\n{}", lines.join("\n")))
}

/// Structured payload used only by the blocking approval UI. The ordinary
/// tool result remains the human-readable checklist kept in model context.
fn approval_payload(args: &Value) -> String {
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|step| {
            let content = step.get("step")?.as_str()?.trim();
            if content.is_empty() {
                return None;
            }
            Some(json!({
                "content": content,
                "status": step.get("status").and_then(Value::as_str).unwrap_or("pending"),
            }))
        })
        .collect::<Vec<_>>();
    json!({ "v": 1, "steps": steps }).to_string()
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

fn plan_rejection_message(feedback: Option<String>) -> String {
    let mut msg =
        "Plan rejected by the user. Revise the plan or ask what they want changed before proceeding."
            .to_string();
    if let Some(feedback) = feedback.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        msg.push_str(" User feedback: ");
        msg.push_str(feedback);
    }
    msg
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
             most one step 'in_progress' at a time. A failed or cancelled tool call does not complete its \
             related step; keep a failed step in_progress/pending, and mark work the user explicitly removes \
             as 'cancelled'. Never restore a cancelled step unless the user asks for it again.",
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
                                    "enum": ["pending", "in_progress", "completed", "cancelled"],
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
        if is_fresh_proposal(args) {
            match env
                .confirm_decision(&format!("{PLAN_APPROVAL_PREFIX}{}", approval_payload(args)))
                .await
            {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return ToolResult::fail(plan_rejection_message(feedback)).stop_batch();
                }
            }
        }
        ToolResult::ok(rendered)
    }
}

/// Plan mode's output channel: the agent submits its finished plan here.
pub const PROPOSE_PLAN: &str = "propose_plan";

const PROPOSE_PLAN_NOTE: &str = "Plan submitted; it is now waiting for the user to approve it. \
     End your turn here — do not start carrying the plan out.";

/// Validate the `entries` argument into the canonical entry shape. Statuses and
/// priorities are normalised here so the persisted body never carries a value
/// the card would silently fall back on.
fn normalize_entries(args: &Value) -> Result<Vec<Value>, String> {
    let entries = args
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or("propose_plan error: 'entries' must be an array of {content, status, priority}")?;
    if entries.is_empty() {
        return Err("propose_plan error: 'entries' must not be empty".into());
    }
    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let content = entry
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    format!("propose_plan error: entry {} is missing 'content' text", i + 1)
                })?;
            let status = entry
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
            if !matches!(status, "pending" | "in_progress" | "completed") {
                return Err(format!(
                    "propose_plan error: entry {} has invalid status '{status}' (use pending|in_progress|completed)",
                    i + 1
                ));
            }
            let priority = entry
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("medium");
            if !matches!(priority, "low" | "medium" | "high") {
                return Err(format!(
                    "propose_plan error: entry {} has invalid priority '{priority}' (use low|medium|high)",
                    i + 1
                ));
            }
            Ok(json!({ "content": content, "status": status, "priority": priority }))
        })
        .collect()
}

/// The plan card's body, in the shape the ACP path already persists — the UI
/// parses live tool results and reloaded tool messages with the one function.
fn plan_body(entries: Vec<Value>) -> Value {
    json!({ "v": 1, "source": "native", "entries": entries, "note": PROPOSE_PLAN_NOTE })
}

/// The built-in counterpart to an ACP agent's plan update.
///
/// ponytail: no store handle and no event of its own. The tool result IS the
/// plan card — the registry already streams it to the UI, and the agent loop
/// already persists it as a `Message::tool(_, "propose_plan", body)` that pairs
/// with its own call. A separate `wisp:plan` row would need turn-end plumbing
/// and would reload as an orphan tool message the provider rejects.
/// Known ceiling: calling twice in one turn replaces the live card but keeps
/// both rows, so a reloaded turn shows the revision. Collapsing them needs
/// turn boundaries the windowed transcript loader does not have, and the
/// prompt asks for one call followed by the end of the turn.
pub struct ProposePlanTool;

#[async_trait]
impl Tool for ProposePlanTool {
    fn name(&self) -> &str {
        PROPOSE_PLAN
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            PROPOSE_PLAN,
            "Submit the plan you researched for this conversation's plan mode. This is the only way \
             a plan reaches the user: send the complete ordered list of steps in one call. Calling it \
             again in the same turn replaces the plan you just submitted. The plan is not executed — \
             the user reviews it and approves it, so end your turn right after this call.",
            json!({
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "description": "The full ordered plan, as concrete steps naming what changes and how it is verified.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string", "description": "One plan step." },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Defaults to 'pending'; a fresh plan is all pending."
                                },
                                "priority": {
                                    "type": "string",
                                    "enum": ["low", "medium", "high"],
                                    "description": "Defaults to 'medium'."
                                }
                            },
                            "required": ["content"]
                        }
                    }
                },
                "required": ["entries"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        let count = args
            .get("entries")
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        format!("{count} steps")
    }
    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match normalize_entries(args) {
            Ok(entries) => ToolResult::ok(plan_body(entries).to_string()).stop_turn(),
            Err(error) => ToolResult::fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_fresh_proposal, render_plan, ProposePlanTool, UpdatePlanTool, PROPOSE_PLAN};
    use crate::env::{ConfirmDecision, ToolControl, ToolEnv, ToolEvent};
    use crate::tool::Tool;
    use serde_json::json;
    use std::path::{Path, PathBuf};

    struct PlanDecisionEnv {
        root: PathBuf,
        decision: ConfirmDecision,
    }

    #[async_trait::async_trait]
    impl ToolEnv for PlanDecisionEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            self.decision.approved()
        }

        async fn confirm_decision(&self, _message: &str) -> ConfirmDecision {
            self.decision.clone()
        }

        async fn emit(&self, _event: ToolEvent) {}
    }

    #[test]
    fn fresh_proposal_only_when_all_pending() {
        assert!(is_fresh_proposal(
            &json!({"steps": [{"step": "a"}, {"step": "b", "status": "pending"}]})
        ));
        assert!(!is_fresh_proposal(
            &json!({"steps": [{"step": "a", "status": "in_progress"}]})
        ));
        assert!(!is_fresh_proposal(
            &json!({"steps": [{"step": "a", "status": "completed"}]})
        ));
        assert!(!is_fresh_proposal(
            &json!({"steps": [{"step": "a", "status": "cancelled"}]})
        ));
        assert!(
            !is_fresh_proposal(&json!({"steps": []})),
            "empty is not a proposal"
        );
    }

    #[test]
    fn renders_markers_counts_and_default_status() {
        let out = render_plan(&json!({"steps": [
            {"step": "Load counts", "status": "completed"},
            {"step": "Run DESeq2", "status": "in_progress"},
            {"step": "Download every MSigDB collection", "status": "cancelled"},
            {"step": "Write report"}
        ]}))
        .unwrap();
        assert!(out.contains("[x] Load counts"), "{out}");
        assert!(out.contains("[~] Run DESeq2"), "{out}");
        assert!(
            out.contains("[-] Download every MSigDB collection"),
            "{out}"
        );
        assert!(out.contains("[ ] Write report"), "{out}"); // omitted status -> pending
        assert!(
            out.contains("4 steps · 1 done · 1 in progress · 1 pending · 1 cancelled"),
            "{out}"
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(render_plan(&json!({})).is_err(), "missing steps");
        assert!(render_plan(&json!({"steps": []})).is_err(), "empty steps");
        assert!(
            render_plan(&json!({"steps": [{"step": "x", "status": "bogus"}]})).is_err(),
            "bad status"
        );
        assert!(
            render_plan(&json!({"steps": [{"status": "pending"}]})).is_err(),
            "missing text"
        );
        assert!(
            render_plan(&json!({"steps": [
                {"step": "a", "status": "in_progress"},
                {"step": "b", "status": "in_progress"}
            ]}))
            .is_err(),
            "two in_progress"
        );
    }

    #[tokio::test]
    async fn propose_plan_writes_the_persisted_card_shape() {
        let env = PlanDecisionEnv {
            root: PathBuf::from("."),
            decision: ConfirmDecision::Approved,
        };
        let result = ProposePlanTool
            .run(
                &json!({ "entries": [
                    { "content": " Read the loader ", "priority": "high" },
                    { "content": "Wire the card", "status": "in_progress" }
                ]}),
                &env,
            )
            .await;

        assert!(result.success, "{}", result.content);
        let body: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(body["v"], 1);
        assert_eq!(body["source"], "native");
        assert_eq!(
            body["entries"],
            json!([
                { "content": "Read the loader", "status": "pending", "priority": "high" },
                { "content": "Wire the card", "status": "in_progress", "priority": "medium" },
            ]),
            "statuses and priorities are normalised for the card"
        );
        assert!(
            body["note"].as_str().unwrap().contains("End your turn"),
            "the result has to stop the agent, not invite it to execute"
        );
        assert_eq!(ProposePlanTool.name(), PROPOSE_PLAN);
        assert_eq!(result.control, ToolControl::StopTurn);
    }

    #[tokio::test]
    async fn propose_plan_rejects_junk() {
        let env = PlanDecisionEnv {
            root: PathBuf::from("."),
            decision: ConfirmDecision::Approved,
        };
        for args in [
            json!({}),
            json!({ "entries": [] }),
            json!({ "entries": [{ "content": "  " }] }),
            json!({ "entries": [{ "content": "x", "status": "blocked" }] }),
            json!({ "entries": [{ "content": "x", "priority": "urgent" }] }),
        ] {
            let result = ProposePlanTool.run(&args, &env).await;
            assert!(!result.success, "{args} should be refused");
        }
    }

    #[tokio::test]
    async fn rejected_plan_feedback_is_returned_to_model() {
        let env = PlanDecisionEnv {
            root: PathBuf::from("."),
            decision: ConfirmDecision::Denied {
                feedback: Some("Split protocol changes from UI work".to_string()),
            },
        };
        let result = UpdatePlanTool
            .run(
                &json!({
                    "steps": [
                        { "step": "Change confirmation protocol", "status": "pending" },
                        { "step": "Add plan card feedback UI", "status": "pending" }
                    ]
                }),
                &env,
            )
            .await;

        assert!(!result.success);
        assert_eq!(result.control, ToolControl::StopBatch);
        assert!(
            result
                .content
                .contains("User feedback: Split protocol changes from UI work"),
            "{}",
            result.content
        );
    }
}
