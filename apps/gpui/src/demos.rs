//! The example conversations that ship with Wisp, as their dialog words
//! them. The host reads the recordings; this says how a row reads and which
//! projects one can be copied into.
use crate::i18n::tr;
use wisp_dto::ProjectSummary;
use wisp_server::desktop::{DemoRole, DemoRow};

/// A folded row's line is cut to this many characters.
const LINE: usize = 160;

/// Who a row is from, as its label says.
pub fn label(row: &DemoRow) -> String {
    match row.role {
        DemoRole::You => tr("You").into(),
        DemoRole::Wisp => "Wisp".into(),
        DemoRole::Thinking => tr("Thinking").into(),
        DemoRole::Plan => tr("Plan").into(),
        DemoRole::Tool => match row.tool.as_deref().filter(|name| !name.trim().is_empty()) {
            Some(name) => name.to_owned(),
            None => tr("Tool").into(),
        },
    }
}

/// Whether a row shows one line until it is opened: what was thought, the
/// plan and what tools gave back, as in a conversation.
pub fn folded(row: &DemoRow) -> bool {
    matches!(
        row.role,
        DemoRole::Thinking | DemoRole::Plan | DemoRole::Tool
    )
}

/// A folded row's one line: its first line that says anything, cut short.
pub fn line(row: &DemoRow) -> String {
    let first = row
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if first.chars().count() <= LINE {
        return first.to_owned();
    }
    let cut: String = first.chars().take(LINE).collect();
    format!("{}…", cut.trim_end())
}

/// Whether opening a folded row shows more than its line.
pub fn opens(row: &DemoRow) -> bool {
    let text = row.text.trim();
    !text.is_empty() && line(row) != text
}

/// The line under the example's title: what it holds.
pub fn summary(rows: &[DemoRow]) -> String {
    let asked = rows.iter().filter(|row| row.role == DemoRole::You).count();
    let tools = rows.iter().filter(|row| row.role == DemoRole::Tool).count();
    crate::i18n::fill(
        tr(
            "{} requests and {} tool calls, as recorded. Read only: copy it into a project to go on with it.",
        ),
        &[&asked, &tools],
    )
}

/// The projects an example can be copied into: those with a folder on
/// this computer, not the research assistant's.
pub fn targets(projects: &[ProjectSummary]) -> Vec<&ProjectSummary> {
    projects
        .iter()
        .filter(|project| {
            !project.workspace_dir.trim().is_empty()
                && !crate::assistant::is_assistant_project(Some(&project.id))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: DemoRole, text: &str) -> DemoRow {
        DemoRow {
            role,
            text: text.into(),
            tool: None,
            ok: None,
            model: None,
        }
    }

    #[test]
    fn rows_are_labelled_and_folded_as_in_a_conversation() {
        let mut tool = row(DemoRole::Tool, "\n {\"status\": \"submitted\"}\nmore");
        assert_eq!(label(&tool), "Tool");
        tool.tool = Some("run_in_context".into());
        assert_eq!(label(&tool), "run_in_context");
        assert!(folded(&tool));
        assert_eq!(line(&tool), "{\"status\": \"submitted\"}");
        assert!(opens(&tool));

        let you = row(DemoRole::You, "Count the reads.");
        assert_eq!(label(&you), "You");
        assert!(!folded(&you));
        assert!(!opens(&you));
        assert_eq!(label(&row(DemoRole::Wisp, "x")), "Wisp");
        assert_eq!(label(&row(DemoRole::Thinking, "x")), "Thinking");
        assert!(folded(&row(DemoRole::Plan, "[ ] probe")));

        // A long line is cut, and opening shows the rest.
        let long = row(DemoRole::Thinking, &"word ".repeat(60));
        assert!(line(&long).ends_with('…'));
        assert!(line(&long).chars().count() <= LINE + 1);
        assert!(opens(&long));
        assert!(!opens(&row(DemoRole::Tool, "  ")));
    }

    #[test]
    fn the_summary_counts_requests_and_tool_calls() {
        let rows = [
            row(DemoRole::You, "a"),
            row(DemoRole::Tool, "b"),
            row(DemoRole::Tool, "c"),
            row(DemoRole::Wisp, "d"),
            row(DemoRole::You, "e"),
        ];
        assert!(summary(&rows).starts_with("2 requests and 2 tool calls"));
    }

    #[test]
    fn examples_go_into_projects_with_a_folder_only() {
        let project = |id: &str, folder: &str| -> ProjectSummary {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": id, "workspace_dir": folder
            }))
            .unwrap()
        };
        let projects = [
            project("lab", "/work/lab"),
            project("remote", ""),
            project(wisp_dto::ASSISTANT_PROJECT_ID, "/data/assistant"),
        ];
        let ids: Vec<&str> = targets(&projects).iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["lab"]);
    }
}
