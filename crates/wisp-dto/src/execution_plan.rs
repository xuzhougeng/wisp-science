//! Shared interpretation of accepted update_plan results and approval previews.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlanStep {
    pub status: PlanStatus,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    Running,
    Done,
    Cancelled,
}

impl PlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
    fn from_protocol(status: &str) -> Self {
        match status {
            "completed" | "done" => Self::Done,
            "in_progress" | "running" => Self::Running,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// Structured approval data preserves nested checklists as content. Legacy
/// tool results retain their line continuations, exactly as the WebView does.
pub fn parse_plan_steps(preview: &str) -> Vec<PlanStep> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(preview) {
        if value.get("v").and_then(serde_json::Value::as_u64) == Some(1) {
            if let Some(steps) = value.get("steps").and_then(serde_json::Value::as_array) {
                return steps
                    .iter()
                    .filter_map(|step| {
                        let content = step.get("content")?.as_str()?.trim();
                        (!content.is_empty()).then(|| PlanStep {
                            status: PlanStatus::from_protocol(
                                step.get("status")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("pending"),
                            ),
                            content: content.into(),
                        })
                    })
                    .collect();
            }
        }
    }
    let mut steps: Vec<PlanStep> = vec![];
    for line in preview.lines() {
        let row = [
            ("[x] ", PlanStatus::Done),
            ("[~] ", PlanStatus::Running),
            ("[ ] ", PlanStatus::Pending),
            ("[-] ", PlanStatus::Cancelled),
        ]
        .into_iter()
        .find_map(|(prefix, status)| {
            line.strip_prefix(prefix).map(|content| PlanStep {
                status,
                content: content.into(),
            })
        });
        if let Some(row) = row {
            steps.push(row);
        } else if let Some(step) = steps.last_mut() {
            if !step.content.is_empty() {
                step.content.push('\n');
            }
            step.content.push_str(line);
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_nested_checklists_and_legacy_continuations_agree() {
        let legacy =
            parse_plan_steps("Plan:\n[x] Inspect\n[~] Analyze\n  details\n[ ] Report\n[-] Extra");
        assert_eq!(
            legacy.iter().map(|s| s.status.as_str()).collect::<Vec<_>>(),
            ["done", "running", "pending", "cancelled"]
        );
        assert_eq!(legacy[1].content, "Analyze\n  details");
        let structured = parse_plan_steps(
            r#"{"v":1,"steps":[{"content":"Code\n[x] nested","status":"in_progress"},{"content":"Future","status":"unknown"}]}"#,
        );
        assert_eq!(structured.len(), 2);
        assert_eq!(structured[0].content, "Code\n[x] nested");
        assert_eq!(structured[1].status, PlanStatus::Pending);
        assert!(parse_plan_steps("update_plan error: rejected").is_empty());
    }
}
