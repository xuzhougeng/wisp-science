//! How the research assistant's plan reads in the window: what each item's
//! status is called and how far the day has come.
use crate::i18n::{fill, tr};
use crate::present::Tone;
use wisp_dto::ResearchAssistantPlanItem;

/// Whether `project` is the research assistant's own, hidden project.
pub fn is_assistant_project(project: Option<&str>) -> bool {
    project == Some(wisp_dto::ASSISTANT_PROJECT_ID)
}

/// How a plan item stands, as its chip says.
pub fn status(status: &str) -> (&'static str, Tone) {
    match status {
        "done" => (tr("Done"), Tone::Success),
        "dropped" => (tr("Dropped"), Tone::Neutral),
        _ => (tr("Open"), Tone::Active),
    }
}

/// One line over the day's plan.
pub fn summary(items: &[ResearchAssistantPlanItem]) -> String {
    if items.is_empty() {
        return tr("Nothing planned for today. Tell the assistant what you mean to do.").to_owned();
    }
    let done = items.iter().filter(|item| item.status == "done").count();
    fill(tr("Today's plan: {} of {} done"), &[&done, &items.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: &str) -> ResearchAssistantPlanItem {
        ResearchAssistantPlanItem {
            id: id.into(),
            day: "2026-11-15".into(),
            title: format!("Item {id}"),
            project_id: None,
            project_name: None,
            session_id: None,
            status: status.into(),
        }
    }

    #[test]
    fn the_plan_reads_as_how_far_the_day_has_come() {
        assert!(is_assistant_project(Some(wisp_dto::ASSISTANT_PROJECT_ID)));
        assert!(!is_assistant_project(Some("p1")));
        assert!(!is_assistant_project(None));
        assert_eq!(
            summary(&[]),
            "Nothing planned for today. Tell the assistant what you mean to do."
        );
        let items = [item("a", "done"), item("b", "open"), item("c", "dropped")];
        assert_eq!(summary(&items), "Today's plan: 1 of 3 done");
        assert_eq!(status("open"), ("Open", Tone::Active));
        assert_eq!(status("done"), ("Done", Tone::Success));
        assert_eq!(status("dropped"), ("Dropped", Tone::Neutral));
    }
}
