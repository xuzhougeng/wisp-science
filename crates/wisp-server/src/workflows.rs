//! Workflows: a goal broken into tasks that say which earlier tasks they
//! need, kept as templates and run in a project.
//!
//! A workflow runs in one of two ways. As one conversation, the model is
//! given the goal and the tasks, laid out by the host in the stages their
//! dependencies allow, and carries them out in that order. As agents, as
//! the WebView's Workflow Studio runs it, each task is its own agent with
//! the capabilities the template grants it: the tasks are drafted as a
//! delegated batch in a new conversation, which the researcher approves
//! (or, under auto-safe approval, starts at once when nothing in it asks
//! for approval), and its result is delivered to that conversation. The
//! templates are this desktop's own, kept under its own key; the
//! WebView's (`workflow_templates`) are neither read nor changed.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wisp_dto::remote_access::RemoteRequest;
use wisp_store::Store;

use crate::Server;

const KEY: &str = "desktop.workflows";
const NAME_LIMIT: usize = 100;
const GOAL_LIMIT: usize = 2_000;
const INSTRUCTION_LIMIT: usize = 8_000;
pub const TASK_LIMIT: usize = 20;
const LIMIT: usize = 40;

/// What a task run as an agent may be given, as the delegation registry
/// names it: thinking only, reading the project, changing it, running
/// code, or reviewing work.
pub const CAPABILITIES: [&str; 5] = [
    "reasoning",
    "project_read",
    "project_write",
    "code_run",
    "review",
];

/// What a task is given when the template names nothing.
const DEFAULT_CAPABILITY: &str = "project_read";

/// One task of a workflow.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTask {
    pub title: String,
    pub instructions: String,
    /// The tasks this one needs, by their place in the list from 0. Only
    /// earlier tasks can be named, so there is never a circle.
    #[serde(default)]
    pub after: Vec<usize>,
    /// What the task's agent may do when the workflow runs as agents,
    /// from [`CAPABILITIES`]; none is reading the project.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// How a workflow run as agents is approved, as the WebView's
/// `AgentApprovalPolicy`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowApproval {
    /// The researcher approves every run.
    #[default]
    ReviewAll,
    /// A run that changes nothing and runs no code starts at once.
    AutoSafe,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    /// What the whole workflow is to achieve.
    pub goal: String,
    pub tasks: Vec<WorkflowTask>,
    #[serde(default)]
    pub approval: WorkflowApproval,
}

/// A workflow drafted as agents: the conversation that holds it, its
/// batch, and whether the batch already started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftedWorkflow {
    pub session: String,
    pub batch: String,
    pub started: bool,
}

/// What the form sends: `id` empty adds a workflow.
pub type WorkflowInput = Workflow;

impl Workflow {
    /// The stage each task is in, from 0: one more than the latest stage
    /// of the tasks it needs. Tasks of one stage do not need each other.
    pub fn stages(&self) -> Vec<usize> {
        let mut stages: Vec<usize> = Vec::with_capacity(self.tasks.len());
        for task in &self.tasks {
            let stage = task
                .after
                .iter()
                .filter_map(|needed| stages.get(*needed))
                .max()
                .map_or(0, |latest| latest + 1);
            stages.push(stage);
        }
        stages
    }

    /// How many stages there are, and the most tasks in any one.
    pub fn shape(&self) -> (usize, usize) {
        let stages = self.stages();
        let count = stages.iter().max().map_or(0, |last| last + 1);
        let widest = (0..count)
            .map(|stage| stages.iter().filter(|at| **at == stage).count())
            .max()
            .unwrap_or(0);
        (count, widest)
    }

    /// What a run sends: the goal, then the tasks stage by stage, each
    /// with the tasks it needs named by number.
    pub fn message(&self) -> String {
        let stages = self.stages();
        let mut text = format!("[Workflow: {}]\n\nGoal: {}\n\n", self.name, self.goal);
        text.push_str(
            "Carry out the tasks below in the order of their stages. A task that names \
             others after it needs their results; tasks of one stage do not depend on each \
             other. Finish each task and state its result under its own heading before \
             going on, and end with a short summary of what was achieved against the goal \
             and what was not.\n",
        );
        let count = stages.iter().max().map_or(0, |last| last + 1);
        for stage in 0..count {
            text.push_str(&format!("\nStage {}\n", stage + 1));
            for (index, task) in self.tasks.iter().enumerate() {
                if stages[index] != stage {
                    continue;
                }
                let needs = match task.after.as_slice() {
                    [] => String::new(),
                    after => format!(
                        " (after {})",
                        after
                            .iter()
                            .map(|needed| format!("task {}", needed + 1))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                text.push_str(&format!(
                    "- Task {} — {}{needs}: {}\n",
                    index + 1,
                    task.title,
                    task.instructions
                ));
            }
        }
        text
    }

    /// The tasks as a delegated batch, one agent each, as `delegate_tasks`
    /// takes it: ids made from the titles, each task's instruction its
    /// title and instructions, and the capabilities it was given.
    pub(crate) fn batch(&self) -> Value {
        let ids = task_ids(&self.tasks);
        let tasks: Vec<Value> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| {
                let both = format!("{}: {}", task.title, task.instructions);
                // The title goes when both together are too long.
                let instruction = if both.chars().count() <= INSTRUCTION_LIMIT {
                    both
                } else {
                    task.instructions.clone()
                };
                let capabilities = if task.capabilities.is_empty() {
                    vec![DEFAULT_CAPABILITY.to_owned()]
                } else {
                    task.capabilities.clone()
                };
                json!({
                    "id": ids[index],
                    "instruction": instruction,
                    "depends_on": task.after.iter().map(|needed| &ids[*needed]).collect::<Vec<_>>(),
                    "capabilities": capabilities,
                })
            })
            .collect();
        json!({
            "goal": self.goal,
            "context": format!("The researcher's workflow \"{}\".", self.name),
            "background": true,
            "tasks": tasks,
        })
    }
}

/// A task id for each task, from its title: lower case, words joined by
/// `-`, at most 31 bytes, starting with a letter, and each one once.
fn task_ids(tasks: &[WorkflowTask]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        let mut slug = String::new();
        for ch in task.title.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let mut base = slug.trim_end_matches('-').to_owned();
        if !base.starts_with(|ch: char| ch.is_ascii_lowercase()) {
            base = format!("task-{}", index + 1);
        }
        base.truncate(26);
        let base = base.trim_end_matches('-').to_owned();
        let mut id = base.clone();
        let mut copy = 2;
        while ids.contains(&id) {
            id = format!("{base}-{copy}");
            copy += 1;
        }
        ids.push(id);
    }
    ids
}

fn starters() -> Vec<Workflow> {
    let task = |title: &str, instructions: &str, after: &[usize]| WorkflowTask {
        title: title.into(),
        instructions: instructions.into(),
        after: after.to_vec(),
        capabilities: Vec::new(),
    };
    vec![
        Workflow {
            id: "starter-evidence".into(),
            name: "Literature evidence review".into(),
            goal: "Check what the literature says about the claim named in the first message \
                   after this one, and report how well it is supported."
                .into(),
            tasks: vec![
                task(
                    "Find the papers",
                    "Search the literature for work that tests the claim directly. List each \
                     paper with its source and one line on what it measured.",
                    &[],
                ),
                task(
                    "Find the counter-evidence",
                    "Search for work that contradicts or fails to replicate the claim. List \
                     each with its source and what it found instead.",
                    &[],
                ),
                task(
                    "Weigh the evidence",
                    "Compare the two lists: sample sizes, methods, and who replicated whom. \
                     Say which results the claim rests on and which it cannot explain.",
                    &[0, 1],
                ),
                task(
                    "Write the report",
                    "Write a short report with citations: what is established, what is \
                     contested, what is unknown, and which claims you could not verify.",
                    &[2],
                ),
            ],
            approval: WorkflowApproval::ReviewAll,
        },
        Workflow {
            id: "starter-design".into(),
            name: "Data-driven research design".into(),
            goal: "Turn the question named in the first message after this one into an \
                   analysis plan grounded in the data this project holds."
                .into(),
            tasks: vec![
                task(
                    "Survey the data",
                    "List the data files in this project with what each holds: samples, \
                     variables, units and anything missing. Change nothing.",
                    &[],
                ),
                task(
                    "State the hypotheses",
                    "Write the question as hypotheses that the data could confirm or refute, \
                     each with the result that would refute it.",
                    &[],
                ),
                task(
                    "Design the analysis",
                    "For each hypothesis name the comparison, the controls and confounders, \
                     and the test, using only what the survey found. Do not run anything.",
                    &[0, 1],
                ),
            ],
            approval: WorkflowApproval::ReviewAll,
        },
    ]
}

pub(crate) async fn list(store: &Store) -> Vec<Workflow> {
    match store.get_setting(KEY).await.ok().flatten() {
        // Saved, even as an empty list: the user removed them all.
        Some(saved) => serde_json::from_str(&saved).unwrap_or_default(),
        None => starters(),
    }
}

async fn save(store: &Store, workflows: &[Workflow]) -> Result<()> {
    store
        .set_setting(KEY, &serde_json::to_string(workflows)?)
        .await
}

/// Checks a form and gives the workflow it describes, trimmed.
fn checked(input: &WorkflowInput) -> Result<Workflow> {
    let name = input.name.trim();
    ensure!(!name.is_empty(), "Give the workflow a name");
    ensure!(
        name.chars().count() <= NAME_LIMIT,
        "A name is at most {NAME_LIMIT} characters"
    );
    let goal = input.goal.trim();
    ensure!(!goal.is_empty(), "Say what the workflow is to achieve");
    ensure!(
        goal.chars().count() <= GOAL_LIMIT,
        "A goal is at most {GOAL_LIMIT} characters"
    );
    ensure!(
        !input.tasks.is_empty(),
        "A workflow needs at least one task"
    );
    ensure!(
        input.tasks.len() <= TASK_LIMIT,
        "A workflow has at most {TASK_LIMIT} tasks"
    );
    let mut tasks = Vec::with_capacity(input.tasks.len());
    for (index, task) in input.tasks.iter().enumerate() {
        let number = index + 1;
        let title = task.title.trim();
        let instructions = task.instructions.trim();
        ensure!(!title.is_empty(), "Task {number} needs a title");
        ensure!(
            title.chars().count() <= NAME_LIMIT,
            "The title of task {number} is longer than {NAME_LIMIT} characters"
        );
        ensure!(!instructions.is_empty(), "Task {number} needs instructions");
        ensure!(
            instructions.chars().count() <= INSTRUCTION_LIMIT,
            "The instructions of task {number} are longer than {INSTRUCTION_LIMIT} characters"
        );
        let mut after = task.after.clone();
        after.sort_unstable();
        after.dedup();
        ensure!(
            after.iter().all(|needed| *needed < index),
            "Task {number} can only come after tasks listed before it"
        );
        let mut capabilities: Vec<String> = Vec::new();
        for capability in &task.capabilities {
            let capability = capability.trim();
            ensure!(
                CAPABILITIES.contains(&capability),
                "Task {number} names an unknown capability \"{capability}\""
            );
            if !capabilities.iter().any(|kept| kept == capability) {
                capabilities.push(capability.into());
            }
        }
        tasks.push(WorkflowTask {
            title: title.into(),
            instructions: instructions.into(),
            after,
            capabilities,
        });
    }
    Ok(Workflow {
        id: input.id.clone(),
        name: name.into(),
        goal: goal.into(),
        tasks,
        approval: input.approval,
    })
}

/// Adds a workflow or changes one.
pub(crate) async fn upsert(store: &Store, input: WorkflowInput) -> Result<Vec<Workflow>> {
    let mut workflow = checked(&input)?;
    let mut workflows = list(store).await;
    let taken = workflows.iter().any(|other| {
        other.id != workflow.id && other.name.to_lowercase() == workflow.name.to_lowercase()
    });
    ensure!(
        !taken,
        "Another workflow is already called \"{}\"",
        workflow.name
    );
    if workflow.id.is_empty() {
        ensure!(
            workflows.len() < LIMIT,
            "There are already {LIMIT} workflows; remove one first"
        );
        workflow.id = uuid::Uuid::new_v4().to_string();
        workflows.push(workflow);
    } else {
        let Some(kept) = workflows.iter_mut().find(|kept| kept.id == workflow.id) else {
            bail!("That workflow is no longer there");
        };
        *kept = workflow;
    }
    save(store, &workflows).await?;
    Ok(workflows)
}

pub(crate) async fn remove(store: &Store, id: &str) -> Result<Vec<Workflow>> {
    let mut workflows = list(store).await;
    let before = workflows.len();
    workflows.retain(|workflow| workflow.id != id);
    ensure!(workflows.len() < before, "That workflow is no longer there");
    save(store, &workflows).await?;
    Ok(workflows)
}

impl Server {
    /// The workflow templates, in the order they were added.
    pub async fn workflows(&self) -> Result<Vec<Workflow>> {
        Ok(list(&self.0.store).await)
    }

    pub async fn save_workflow(&self, input: WorkflowInput) -> Result<Vec<Workflow>> {
        upsert(&self.0.store, input).await
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<Vec<Workflow>> {
        remove(&self.0.store, id).await
    }

    /// Runs a workflow in `project`: a new conversation named after it is
    /// sent the goal and the staged tasks. Returns the conversation.
    pub async fn run_workflow(&self, project: &str, id: &str) -> Result<String> {
        let workflow = list(&self.0.store)
            .await
            .into_iter()
            .find(|workflow| workflow.id == id)
            .context("That workflow is no longer there")?;
        let session = self.create_conversation(project).await?;
        self.execute(RemoteRequest {
            nonce: "workflow".into(),
            seq: 0,
            id: uuid::Uuid::new_v4().to_string(),
            command: "native_conversation_send".into(),
            project_id: Some(project.to_owned()),
            args: json!({
                "session_id": session,
                "request_id": uuid::Uuid::new_v4().to_string(),
                "message": workflow.message(),
            }),
        })
        .await?;
        // Named after the workflow; the send names a new conversation after
        // its message, so this comes after it.
        self.0
            .store
            .rename_session(&session, project, &workflow.name)
            .await?;
        Ok(session)
    }

    /// Drafts a workflow as agents in `project`: a new conversation named
    /// after it holds the batch of its tasks, one agent each, waiting in
    /// its Agents tab for the researcher to approve or discard it. Under
    /// auto-safe approval a batch that changes nothing and runs no code
    /// starts at once. Either way its result is delivered to that
    /// conversation.
    pub async fn draft_workflow(&self, project: &str, id: &str) -> Result<DraftedWorkflow> {
        let workflow = list(&self.0.store)
            .await
            .into_iter()
            .find(|workflow| workflow.id == id)
            .context("That workflow is no longer there")?;
        ensure!(
            workflow.tasks.len() <= wisp_core::MAX_DELEGATION_TASKS,
            "Run as agents takes at most {} tasks; this workflow has {}",
            wisp_core::MAX_DELEGATION_TASKS,
            workflow.tasks.len()
        );
        let session = self.create_conversation(project).await?;
        self.0
            .store
            .rename_session(&session, project, &workflow.name)
            .await?;
        let drafted = self
            .draft_batch(
                project,
                &session,
                workflow.batch(),
                workflow.approval == WorkflowApproval::ReviewAll,
            )
            .await;
        match drafted {
            Ok((batch, started)) => Ok(DraftedWorkflow {
                session,
                batch,
                started,
            }),
            Err(error) => {
                // Nothing was drafted, so the empty conversation goes too.
                let _ = self.delete_conversation(project, &session).await;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, after: &[usize]) -> WorkflowTask {
        WorkflowTask {
            title: title.into(),
            instructions: format!("Do {title}."),
            after: after.to_vec(),
            capabilities: Vec::new(),
        }
    }

    fn workflow(tasks: Vec<WorkflowTask>) -> Workflow {
        Workflow {
            id: String::new(),
            name: " Screen ".into(),
            goal: " Find the hits. ".into(),
            tasks,
            approval: WorkflowApproval::ReviewAll,
        }
    }

    #[test]
    fn a_workflow_runs_as_agents_with_ids_from_its_titles() {
        let mut first = task("Load the data!", &[]);
        first.capabilities = vec!["code_run".into()];
        let made = checked(&workflow(vec![
            first,
            task("Load the data", &[0]),
            task("2nd look", &[0, 1]),
        ]))
        .unwrap();
        let batch = made.batch();
        assert_eq!(batch["goal"], "Find the hits.");
        assert_eq!(batch["background"], true);
        let tasks = batch["tasks"].as_array().unwrap();
        let ids: Vec<&str> = tasks
            .iter()
            .map(|task| task["id"].as_str().unwrap())
            .collect();
        // Each id once, and one that would start with a digit is named
        // by its place.
        assert_eq!(ids, ["load-the-data", "load-the-data-2", "task-3"]);
        assert_eq!(tasks[0]["capabilities"], json!(["code_run"]));
        // A task given nothing reads the project.
        assert_eq!(tasks[1]["capabilities"], json!(["project_read"]));
        assert_eq!(
            tasks[2]["depends_on"],
            json!(["load-the-data", "load-the-data-2"])
        );
        assert_eq!(tasks[1]["instruction"], "Load the data: Do Load the data.");
        let long = task_ids(&[WorkflowTask {
            title: "A very long title that goes on and on and on".into(),
            ..Default::default()
        }]);
        assert!(long[0].len() <= 31 && !long[0].ends_with('-'));
        // An unknown capability is refused; a repeated one is kept once.
        let mut odd = task("odd", &[]);
        odd.capabilities = vec!["fly".into()];
        assert!(checked(&workflow(vec![odd])).is_err());
        let mut twice = task("twice", &[]);
        twice.capabilities = vec!["review".into(), " review ".into()];
        assert_eq!(
            checked(&workflow(vec![twice])).unwrap().tasks[0].capabilities,
            ["review"]
        );
    }

    #[test]
    fn tasks_are_staged_by_what_they_need() {
        let diamond = workflow(vec![
            task("load", &[]),
            task("left", &[0]),
            task("right", &[0]),
            task("merge", &[1, 2]),
            task("note", &[]),
        ]);
        assert_eq!(diamond.stages(), [0, 1, 1, 2, 0]);
        assert_eq!(diamond.shape(), (3, 2));
        assert_eq!(workflow(vec![]).shape(), (0, 0));

        let message = checked(&diamond).unwrap().message();
        assert!(message.starts_with("[Workflow: Screen]\n\nGoal: Find the hits.\n\n"));
        let stage_one = message.find("\nStage 1\n").unwrap();
        let stage_two = message.find("\nStage 2\n").unwrap();
        let stage_three = message.find("\nStage 3\n").unwrap();
        assert!(stage_one < stage_two && stage_two < stage_three);
        let at = |line: &str| {
            message
                .find(line)
                .unwrap_or_else(|| panic!("{line}\n{message}"))
        };
        // An independent task is in the first stage wherever it is listed.
        assert!(at("- Task 5 — note: Do note.") < stage_two);
        assert!(at("- Task 2 — left (after task 1): Do left.") > stage_two);
        assert!(at("- Task 4 — merge (after task 2, task 3): Do merge.") > stage_three);
    }

    #[test]
    fn a_form_is_checked_and_a_task_only_follows_earlier_ones() {
        let made = checked(&workflow(vec![task(" load ", &[]), task("fit", &[0, 0])])).unwrap();
        assert_eq!(
            (made.name.as_str(), made.goal.as_str()),
            ("Screen", "Find the hits.")
        );
        assert_eq!(made.tasks[0].title, "load");
        assert_eq!(made.tasks[1].after, [0]);

        let refused = |tasks: Vec<WorkflowTask>| checked(&workflow(tasks)).is_err();
        assert!(refused(vec![]));
        assert!(refused(vec![task("  ", &[])]));
        assert!(refused(vec![task("self", &[0])]));
        assert!(refused(vec![task("early", &[1]), task("late", &[])]));
        assert!(refused(
            (0..=TASK_LIMIT)
                .map(|n| task(&format!("t{n}"), &[]))
                .collect()
        ));
        let mut blank = task("blank", &[]);
        blank.instructions = " ".into();
        assert!(refused(vec![blank]));
        for (name, goal) in [("", "goal"), ("name", " ")] {
            let form = Workflow {
                name: name.into(),
                goal: goal.into(),
                ..workflow(vec![task("load", &[])])
            };
            assert!(checked(&form).is_err());
        }
    }

    #[tokio::test]
    async fn workflows_start_with_a_few_and_are_the_users_to_change_and_remove() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let start = list(&store).await;
        assert_eq!(start.len(), 2);
        assert!(start
            .iter()
            .all(|workflow| workflow.id.starts_with("starter-")));
        assert!(start.iter().all(|workflow| checked(workflow).is_ok()));

        let added = upsert(&store, workflow(vec![task("load", &[])]))
            .await
            .unwrap();
        assert_eq!(added.len(), 3);
        let id = added[2].id.clone();
        assert!(!id.is_empty());
        // A name is used once, whatever its case.
        let twin = Workflow {
            name: "SCREEN".into(),
            ..workflow(vec![task("load", &[])])
        };
        assert!(upsert(&store, twin).await.is_err());

        let changed = Workflow {
            id: id.clone(),
            ..workflow(vec![task("load", &[]), task("fit", &[0])])
        };
        let kept = upsert(&store, changed).await.unwrap();
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[2].tasks.len(), 2);
        let gone = Workflow {
            id: "absent".into(),
            name: "Other".into(),
            ..workflow(vec![task("load", &[])])
        };
        assert!(upsert(&store, gone).await.is_err());

        // A starter is removed like any other and stays removed.
        for id in ["starter-evidence", "starter-design", id.as_str()] {
            remove(&store, id).await.unwrap();
        }
        assert!(remove(&store, &id).await.is_err());
        assert!(list(&store).await.is_empty());
    }
}
