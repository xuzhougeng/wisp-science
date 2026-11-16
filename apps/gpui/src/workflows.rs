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

/// The typed fields of a task's agent settings: a token budget and a time
/// limit in seconds (empty is none), and the shape its result must take,
/// as JSON (empty is none).
pub fn agent_fields(
    tokens: &str,
    seconds: &str,
    shape: &str,
) -> Result<(Option<u32>, Option<u64>, Option<serde_json::Value>), &'static str> {
    let tokens = match tokens.trim() {
        "" => None,
        text => Some(
            text.parse::<u32>()
                .ok()
                .filter(|tokens| *tokens > 0)
                .ok_or(tr("A token budget is a whole number above zero."))?,
        ),
    };
    let seconds = match seconds.trim() {
        "" => None,
        text => Some(
            text.parse::<u64>()
                .map_err(|_| tr("A time limit is a whole number of seconds."))?,
        ),
    };
    let shape = match shape.trim() {
        "" => None,
        text => Some(
            serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .filter(serde_json::Value::is_object)
                .ok_or(tr(
                    "A result shape is a JSON object, such as a JSON Schema.",
                ))?,
        ),
    };
    Ok((tokens, seconds, shape))
}

/// The model a task's agent works on, in words: the delegation default,
/// the sub-agent model, or the conversation's own.
pub fn model_label(model: Option<wisp_server::desktop::ModelTier>) -> &'static str {
    use wisp_server::desktop::ModelTier;
    match model {
        None => tr("Default model"),
        Some(ModelTier::Fast) => tr("Sub-agent model"),
        Some(ModelTier::Inherit) => tr("The conversation's model"),
    }
}

/// How a run as agents is approved, in words.
pub fn approval_label(approval: WorkflowApproval) -> &'static str {
    match approval {
        WorkflowApproval::ReviewAll => tr("Ask before every run"),
        WorkflowApproval::AutoSafe => tr("Start at once when it only reads"),
    }
}

/// Where one task sits on the workflow's map: its stage is its column,
/// and its row is its place among the tasks of that stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapNode {
    pub task: usize,
    pub column: usize,
    pub row: usize,
}

/// The workflow as a map, as the WebView's graph editor lays it out: each
/// task in the column of its stage (one after the latest stage of the
/// tasks it needs), and an edge from each needed task to the task that
/// needs it.
pub fn map(after: &[Vec<usize>]) -> (Vec<MapNode>, Vec<(usize, usize)>) {
    let mut stages: Vec<usize> = Vec::with_capacity(after.len());
    for needs in after {
        let stage = needs
            .iter()
            .filter_map(|needed| stages.get(*needed))
            .max()
            .map_or(0, |latest| latest + 1);
        stages.push(stage);
    }
    let mut filled: Vec<usize> = Vec::new();
    let nodes = stages
        .iter()
        .enumerate()
        .map(|(task, &column)| {
            if filled.len() <= column {
                filled.resize(column + 1, 0);
            }
            let row = filled[column];
            filled[column] += 1;
            MapNode { task, column, row }
        })
        .collect();
    let edges = after
        .iter()
        .enumerate()
        .flat_map(|(task, needs)| needs.iter().map(move |needed| (*needed, task)))
        .filter(|(needed, task)| needed < task)
        .collect();
    (nodes, edges)
}

/// A click on the map's task `clicked` with `picked` picked before: the
/// pick it leaves, and the dependency to add or take away, as the earlier
/// task and the later one that needs it. The first click picks; a click on
/// the picked task lets it go; a click on another joins the two, the one
/// listed later needing the other.
pub fn map_click(picked: Option<usize>, clicked: usize) -> (Option<usize>, Option<(usize, usize)>) {
    match picked {
        None => (Some(clicked), None),
        Some(picked) if picked == clicked => (None, None),
        Some(picked) => (None, Some((picked.min(clicked), picked.max(clicked)))),
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
            agent: Default::default(),
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
    fn the_map_puts_tasks_in_stage_columns_and_clicks_join_them() {
        // 0; 1 after 0; 2 alone; 3 after 1 and 2.
        let after = vec![vec![], vec![0], vec![], vec![1, 2]];
        let (nodes, edges) = map(&after);
        let at: Vec<(usize, usize)> = nodes.iter().map(|node| (node.column, node.row)).collect();
        assert_eq!(at, [(0, 0), (1, 0), (0, 1), (2, 0)]);
        assert_eq!(edges, [(0, 1), (1, 3), (2, 3)]);
        assert_eq!(map(&[]), (vec![], vec![]));

        // Pick, let go, and join either way round: the later task needs
        // the earlier one.
        assert_eq!(map_click(None, 2), (Some(2), None));
        assert_eq!(map_click(Some(2), 2), (None, None));
        assert_eq!(map_click(Some(3), 1), (None, Some((1, 3))));
        assert_eq!(map_click(Some(0), 2), (None, Some((0, 2))));
    }

    #[test]
    fn a_tasks_agent_settings_are_read_from_their_fields() {
        assert_eq!(agent_fields(" ", "", ""), Ok((None, None, None)));
        let (tokens, seconds, shape) =
            agent_fields("9000", " 600 ", r#"{"type": "object"}"#).unwrap();
        assert_eq!((tokens, seconds), (Some(9000), Some(600)));
        assert_eq!(shape.unwrap()["type"], "object");
        assert!(agent_fields("0", "", "").is_err());
        assert!(agent_fields("many", "", "").is_err());
        assert!(agent_fields("", "-1", "").is_err());
        assert!(agent_fields("", "", "[1]").is_err());
        assert!(agent_fields("", "", "{not json").is_err());
        assert_eq!(model_label(None), "Default model");
    }

    #[test]
    fn a_workflow_says_how_many_tasks_and_stages_it_has() {
        let mut workflow = Workflow {
            id: "w".into(),
            name: "Screen".into(),
            goal: "Find hits".into(),
            tasks: vec![task(&[]), task(&[0]), task(&[1])],
            approval: WorkflowApproval::ReviewAll,
            context: String::new(),
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
