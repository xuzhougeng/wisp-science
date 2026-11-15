//! Workflows as Settings words them, and the bookkeeping of a task list
//! in which a task may only need tasks listed before it.
use wisp_server::desktop::{Workflow, WorkflowApproval};

use crate::i18n::{fill, tr};

/// How big a workflow is: its tasks, its stages, and how many tasks of
/// one stage do not depend on each other when that is more than one.
pub fn shape_line(workflow: &Workflow) -> String {
    let (stages, widest) = workflow.shape();
    let line = fill(
        tr("{} tasks · {} stages"),
        &[&workflow.tasks.len(), &stages],
    );
    if widest > 1 {
        format!("{line}{}", fill(tr(" · up to {} independent"), &[&widest]))
    } else {
        line
    }
}

/// What a capability lets a task's agent do, in a few words.
pub fn capability_label(name: &str) -> &'static str {
    match name {
        "reasoning" => tr("Think only"),
        "project_read" => tr("Read the project"),
        "project_write" => tr("Change the project"),
        "code_run" => tr("Run code"),
        "review" => tr("Review work"),
        "delegation" => tr("Delegate further"),
        _ => tr("Other"),
    }
}

/// What a task run as an agent may do; one given nothing reads the
/// project.
pub fn capability_line(capabilities: &[String]) -> String {
    if capabilities.is_empty() {
        return capability_label("project_read").to_owned();
    }
    capabilities
        .iter()
        .map(|name| capability_label(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `capabilities` with `capability` added, or taken out when it was
/// there, kept in the order the form lists them.
pub fn toggled_capability(capabilities: &[String], capability: &str) -> Vec<String> {
    let had = capabilities.iter().any(|kept| kept == capability);
    wisp_server::desktop::WORKFLOW_CAPABILITIES
        .iter()
        .filter(|name| {
            let kept = capabilities.iter().any(|kept| kept == *name);
            if **name == capability { !had } else { kept }
        })
        .map(|name| name.to_string())
        .collect()
}

/// How a run as agents is approved, in words.
pub fn approval_label(approval: WorkflowApproval) -> &'static str {
    match approval {
        WorkflowApproval::ReviewAll => tr("Ask before every run"),
        WorkflowApproval::AutoSafe => tr("Start at once when it only reads"),
    }
}

/// The dependencies of every task once the task at `removed` is taken
/// out: nobody needs it any more, and later tasks move up one place.
pub fn without(after: &[Vec<usize>], removed: usize) -> Vec<Vec<usize>> {
    after
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != removed)
        .map(|(_, needs)| {
            needs
                .iter()
                .filter(|needed| **needed != removed)
                .map(|needed| {
                    if *needed > removed {
                        needed - 1
                    } else {
                        *needed
                    }
                })
                .collect()
        })
        .collect()
}

/// `needs` with `task` added, or taken out when it was there; sorted.
pub fn toggled(needs: &[usize], task: usize) -> Vec<usize> {
    let mut next: Vec<usize> = needs
        .iter()
        .copied()
        .filter(|needed| *needed != task)
        .collect();
    if next.len() == needs.len() {
        next.push(task);
    }
    next.sort_unstable();
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::WorkflowTask;

    fn task(after: &[usize]) -> WorkflowTask {
        WorkflowTask {
            title: "t".into(),
            instructions: "i".into(),
            after: after.to_vec(),
            capabilities: Vec::new(),
        }
    }

    #[test]
    fn a_task_says_what_its_agent_may_do() {
        assert_eq!(capability_line(&[]), "Read the project");
        assert_eq!(
            capability_line(&["project_read".into(), "code_run".into()]),
            "Read the project, Run code"
        );
        // Every capability a template can grant has words of its own.
        for name in wisp_server::desktop::WORKFLOW_CAPABILITIES {
            assert_ne!(capability_label(name), "Other", "{name}");
        }
        assert_eq!(
            approval_label(WorkflowApproval::default()),
            "Ask before every run"
        );
        let on = toggled_capability(&["code_run".into()], "project_read");
        assert_eq!(on, ["project_read", "code_run"]);
        assert_eq!(toggled_capability(&on, "code_run"), ["project_read"]);
    }

    #[test]
    fn a_workflow_says_how_many_tasks_and_stages_it_has() {
        let mut workflow = Workflow {
            id: "w".into(),
            name: "Screen".into(),
            goal: "Find hits".into(),
            tasks: vec![task(&[]), task(&[0]), task(&[1])],
            approval: WorkflowApproval::ReviewAll,
        };
        assert_eq!(shape_line(&workflow), "3 tasks · 3 stages");
        workflow.tasks[2].after = vec![0];
        assert_eq!(
            shape_line(&workflow),
            "3 tasks · 2 stages · up to 2 independent"
        );
    }

    #[test]
    fn removing_a_task_keeps_the_other_dependencies_pointing_at_the_same_tasks() {
        // 0, 1 after 0, 2 after 0 and 1, 3 after 2.
        let after = vec![vec![], vec![0], vec![0, 1], vec![2]];
        assert_eq!(without(&after, 1), [vec![], vec![0], vec![1]]);
        assert_eq!(without(&after, 0), [vec![], vec![0], vec![1]]);
        assert_eq!(without(&after, 3), [vec![], vec![0], vec![0, 1]]);
        assert_eq!(toggled(&[0, 2], 1), [0, 1, 2]);
        assert_eq!(toggled(&[0, 1, 2], 1), [0, 2]);
        assert_eq!(toggled(&[], 0), [0]);
    }
}
