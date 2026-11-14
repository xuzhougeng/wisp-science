//! Workflows as Settings words them, and the bookkeeping of a task list
//! in which a task may only need tasks listed before it.
use wisp_server::desktop::Workflow;

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
        }
    }

    #[test]
    fn a_workflow_says_how_many_tasks_and_stages_it_has() {
        let mut workflow = Workflow {
            id: "w".into(),
            name: "Screen".into(),
            goal: "Find hits".into(),
            tasks: vec![task(&[]), task(&[0]), task(&[1])],
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
