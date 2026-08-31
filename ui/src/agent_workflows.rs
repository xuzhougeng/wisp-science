//! Workflow Studio editor and persisted Agent workflow activity surface.

use crate::app_support::compose_icon;
use crate::bindings::invoke_checked;
use crate::dto::*;
use crate::i18n::{t, tf, Locale};
use crate::text::{dom_value, event_target_checked, event_target_value, md_to_html};
use crate::window_capture_escape;
use leptos::{ev, *};
use serde_json::Value;
use serde_wasm_bindgen::to_value;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::{JsCast, JsValue};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DynamicTaskForm {
    key: u32,
    id: String,
    instruction: String,
    depends_on: Vec<String>,
    task_kind: WorkflowTaskKind,
    run_activity_context_id: String,
    run_activity_input_task_id: String,
    run_activity_max_candidates: String,
    run_activity_max_wall_seconds: String,
    run_activity_max_evaluator_seconds: String,
    run_activity_max_cost_microunits: String,
    capabilities: Vec<String>,
    skill_ids: Vec<String>,
    specialist_id: String,
    output_schema: String,
    isolated: bool,
    model_id: String,
    executor_key: String,
    max_tokens: String,
    max_tool_calls: String,
    max_cost_microunits: String,
    timeout_secs: String,
}

impl DynamicTaskForm {
    fn blank(key: u32, id: String) -> Self {
        Self {
            key,
            id,
            instruction: String::new(),
            depends_on: vec![],
            task_kind: WorkflowTaskKind::Agent,
            run_activity_context_id: "local".into(),
            run_activity_input_task_id: String::new(),
            run_activity_max_candidates: "20".into(),
            run_activity_max_wall_seconds: "14400".into(),
            run_activity_max_evaluator_seconds: "120".into(),
            run_activity_max_cost_microunits: "5000000".into(),
            capabilities: vec!["reasoning".into()],
            skill_ids: vec![],
            specialist_id: String::new(),
            output_schema: String::new(),
            isolated: false,
            model_id: String::new(),
            executor_key: String::new(),
            max_tokens: String::new(),
            max_tool_calls: String::new(),
            max_cost_microunits: String::new(),
            timeout_secs: String::new(),
        }
    }

    fn from_proposal(key: u32, task: DynamicAgentTaskProposal) -> Self {
        let budget = task.budget.unwrap_or_default();
        let activity = task.run_activity.as_ref();
        Self {
            key,
            id: task.id,
            instruction: task.instruction,
            depends_on: task.depends_on,
            task_kind: task.task_kind,
            run_activity_context_id: activity
                .map(|value| value.context_id.clone())
                .unwrap_or_else(|| "local".into()),
            run_activity_input_task_id: activity
                .map(|value| value.input_task_id.clone())
                .unwrap_or_default(),
            run_activity_max_candidates: activity
                .map(|value| value.max_candidates.to_string())
                .unwrap_or_else(|| "20".into()),
            run_activity_max_wall_seconds: activity
                .map(|value| value.max_wall_seconds.to_string())
                .unwrap_or_else(|| "14400".into()),
            run_activity_max_evaluator_seconds: activity
                .map(|value| value.max_evaluator_seconds.to_string())
                .unwrap_or_else(|| "120".into()),
            run_activity_max_cost_microunits: activity
                .map(|value| value.max_cost_microunits.to_string())
                .unwrap_or_else(|| "5000000".into()),
            capabilities: task.capabilities,
            skill_ids: task.skill_ids,
            specialist_id: task.specialist_id.unwrap_or_default(),
            output_schema: task
                .output_schema
                .and_then(|schema| serde_json::to_string_pretty(&schema).ok())
                .unwrap_or_default(),
            isolated: task.isolated,
            model_id: task.model_id.unwrap_or_default(),
            executor_key: task.executor.as_ref().map(executor_key).unwrap_or_default(),
            max_tokens: budget
                .max_tokens
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_tool_calls: budget
                .max_tool_calls
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_cost_microunits: budget
                .max_cost_microunits
                .map(|value| value.to_string())
                .unwrap_or_default(),
            timeout_secs: task
                .timeout_secs
                .map(|value| value.to_string())
                .unwrap_or_default(),
        }
    }

    fn proposal(&self) -> Result<DynamicAgentTaskProposal, String> {
        if self.task_kind == WorkflowTaskKind::RunActivity {
            let max_candidates =
                parse_required_u32(&self.run_activity_max_candidates, "candidate budget")?;
            let max_wall_seconds =
                parse_required_u64(&self.run_activity_max_wall_seconds, "wall-time budget")?;
            let max_evaluator_seconds = parse_required_u64(
                &self.run_activity_max_evaluator_seconds,
                "evaluator-time budget",
            )?;
            let max_cost_microunits =
                parse_required_u64(&self.run_activity_max_cost_microunits, "cost budget")?;
            return Ok(DynamicAgentTaskProposal {
                id: self.id.trim().into(),
                instruction: self.instruction.trim().into(),
                depends_on: self.depends_on.clone(),
                task_kind: WorkflowTaskKind::RunActivity,
                run_activity: Some(RunActivityProposal {
                    activity: "method_search".into(),
                    context_id: self.run_activity_context_id.trim().into(),
                    input_task_id: self.run_activity_input_task_id.trim().into(),
                    spec_output_pointer: "method_search_spec_artifact_version_id".into(),
                    max_candidates,
                    max_wall_seconds,
                    max_evaluator_seconds,
                    max_cost_microunits,
                }),
                capabilities: vec![],
                skill_ids: vec![],
                specialist_id: None,
                output_schema: None,
                isolated: false,
                model_id: None,
                executor: None,
                budget: None,
                timeout_secs: None,
            });
        }
        let output_schema = if self.output_schema.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&self.output_schema)
                    .map_err(|error| format!("Task {} output schema: {error}", self.id))?,
            )
        };
        let max_tokens = parse_budget_u32(&self.max_tokens, "token budget")?;
        let max_tool_calls = parse_budget_u32(&self.max_tool_calls, "tool-call budget")?;
        let max_cost_microunits = parse_budget_u64(&self.max_cost_microunits, "cost budget")?;
        let timeout_secs = parse_budget_u64(&self.timeout_secs, "wall-time limit")?;
        let budget =
            (max_tokens.is_some() || max_tool_calls.is_some() || max_cost_microunits.is_some())
                .then_some(AgentBudgetProposal {
                    max_tokens,
                    max_tool_calls,
                    max_cost_microunits,
                });
        Ok(DynamicAgentTaskProposal {
            id: self.id.trim().into(),
            instruction: self.instruction.trim().into(),
            depends_on: self.depends_on.clone(),
            task_kind: WorkflowTaskKind::Agent,
            run_activity: None,
            capabilities: self.capabilities.clone(),
            skill_ids: self.skill_ids.clone(),
            specialist_id: nonempty(&self.specialist_id),
            output_schema,
            isolated: self.isolated,
            model_id: nonempty(&self.model_id),
            executor: parse_executor_key(&self.executor_key),
            budget,
            timeout_secs,
        })
    }
}

const MIN_ROUNDTABLE_PARTICIPANTS: usize = 2;
const MAX_ROUNDTABLE_PARTICIPANTS: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RoundtableAssignmentForm {
    specialist_id: String,
    model_id: String,
    executor_key: String,
}

impl RoundtableAssignmentForm {
    fn apply_to(&self, task: &mut DynamicTaskForm) {
        task.specialist_id.clone_from(&self.specialist_id);
        if self.specialist_id == "reviewer"
            && !task
                .capabilities
                .iter()
                .any(|capability| capability == "review")
        {
            task.capabilities.push("review".into());
        }
        task.executor_key.clone_from(&self.executor_key);
        task.model_id = if self.executor_key.is_empty() || self.executor_key == "native" {
            self.model_id.clone()
        } else {
            String::new()
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RoundtableTemplateForm {
    participant_count: usize,
    participants: Vec<RoundtableAssignmentForm>,
    chair: RoundtableAssignmentForm,
}

impl Default for RoundtableTemplateForm {
    fn default() -> Self {
        Self {
            participant_count: MIN_ROUNDTABLE_PARTICIPANTS,
            participants: vec![RoundtableAssignmentForm::default(); MAX_ROUNDTABLE_PARTICIPANTS],
            chair: RoundtableAssignmentForm::default(),
        }
    }
}

impl RoundtableTemplateForm {
    fn set_participant_count(&mut self, participant_count: usize) {
        self.participant_count =
            participant_count.clamp(MIN_ROUNDTABLE_PARTICIPANTS, MAX_ROUNDTABLE_PARTICIPANTS);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DynamicWorkflowForm {
    goal: String,
    context: String,
    approval_policy: AgentApprovalPolicy,
    tasks: Vec<DynamicTaskForm>,
    next_task_key: u32,
}

impl Default for DynamicWorkflowForm {
    fn default() -> Self {
        Self {
            goal: String::new(),
            context: String::new(),
            approval_policy: AgentApprovalPolicy::ReviewAll,
            tasks: vec![DynamicTaskForm::blank(1, "task_1".into())],
            next_task_key: 2,
        }
    }
}

impl DynamicWorkflowForm {
    fn from_proposal(proposal: DynamicAgentWorkflowProposal) -> Self {
        let mut next_task_key = 1;
        let tasks = proposal
            .tasks
            .into_iter()
            .map(|task| {
                let key = next_task_key;
                next_task_key += 1;
                DynamicTaskForm::from_proposal(key, task)
            })
            .collect();
        Self {
            goal: proposal.goal,
            context: proposal.context,
            approval_policy: proposal.approval_policy,
            tasks,
            next_task_key,
        }
    }

    fn proposal(&self) -> Result<DynamicAgentWorkflowProposal, String> {
        if self.goal.trim().is_empty() {
            return Err("A delegation goal is required.".into());
        }
        if self.tasks.is_empty() {
            return Err("Add at least one temporary task.".into());
        }
        let tasks = self
            .tasks
            .iter()
            .map(DynamicTaskForm::proposal)
            .collect::<Result<Vec<_>, _>>()?;
        if tasks
            .iter()
            .any(|task| task.id.is_empty() || task.instruction.is_empty())
        {
            return Err("Every task needs an id and instruction.".into());
        }
        if tasks
            .iter()
            .any(|task| task.task_kind == WorkflowTaskKind::Agent && task.capabilities.is_empty())
        {
            return Err("Every task needs at least one capability.".into());
        }
        Ok(DynamicAgentWorkflowProposal {
            goal: self.goal.trim().into(),
            context: self.context.trim().into(),
            approval_policy: self.approval_policy,
            tasks,
        })
    }

    fn ready(&self) -> bool {
        !self.goal.trim().is_empty()
            && !self.tasks.is_empty()
            && self.tasks.iter().all(|task| {
                !task.id.trim().is_empty()
                    && !task.instruction.trim().is_empty()
                    && match task.task_kind {
                        WorkflowTaskKind::Agent => !task.capabilities.is_empty(),
                        WorkflowTaskKind::RunActivity => {
                            !task.run_activity_context_id.trim().is_empty()
                                && !task.run_activity_input_task_id.trim().is_empty()
                                && task.depends_on.contains(&task.run_activity_input_task_id)
                                && parse_required_u32(
                                    &task.run_activity_max_candidates,
                                    "candidate budget",
                                )
                                .is_ok()
                                && parse_required_u64(
                                    &task.run_activity_max_wall_seconds,
                                    "wall-time budget",
                                )
                                .is_ok()
                                && parse_required_u64(
                                    &task.run_activity_max_evaluator_seconds,
                                    "evaluator-time budget",
                                )
                                .is_ok()
                                && parse_required_u64(
                                    &task.run_activity_max_cost_microunits,
                                    "cost budget",
                                )
                                .is_ok()
                        }
                    }
            })
    }

    fn add_task(&mut self) -> u32 {
        let key = self.next_task_key;
        self.next_task_key += 1;
        let mut number = self.tasks.len() + 1;
        let id = loop {
            let candidate = format!("task_{number}");
            if self.tasks.iter().all(|task| task.id != candidate) {
                break candidate;
            }
            number += 1;
        };
        self.tasks.push(DynamicTaskForm::blank(key, id));
        key
    }

    fn add_task_after(&mut self, source_key: u32) -> Option<u32> {
        let source_id = self
            .tasks
            .iter()
            .find(|task| task.key == source_key)
            .map(|task| task.id.trim().to_string())
            .filter(|id| !id.is_empty())?;
        let key = self.add_task();
        self.tasks
            .iter_mut()
            .find(|task| task.key == key)?
            .depends_on
            .push(source_id);
        Some(key)
    }

    fn remove_task(&mut self, key: u32) {
        self.tasks.retain(|task| task.key != key);
        let ids = self
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<HashSet<_>>();
        for task in &mut self.tasks {
            task.depends_on
                .retain(|dependency| ids.contains(dependency));
            if !task.depends_on.contains(&task.run_activity_input_task_id) {
                task.run_activity_input_task_id =
                    task.depends_on.first().cloned().unwrap_or_default();
            }
        }
    }

    fn add_dependency(&mut self, source_key: u32, target_key: u32) -> Result<bool, &'static str> {
        if source_key == target_key {
            return Err("same_task");
        }
        let Some(source_id) = self
            .tasks
            .iter()
            .find(|task| task.key == source_key)
            .map(|task| task.id.trim().to_string())
        else {
            return Err("missing_task");
        };
        let Some(target_id) = self
            .tasks
            .iter()
            .find(|task| task.key == target_key)
            .map(|task| task.id.trim().to_string())
        else {
            return Err("missing_task");
        };
        if source_id.is_empty() || target_id.is_empty() {
            return Err("empty_id");
        }
        if self.depends_transitively_on(&source_id, &target_id) {
            return Err("cycle");
        }
        let target = self
            .tasks
            .iter_mut()
            .find(|task| task.key == target_key)
            .ok_or("missing_task")?;
        if target.depends_on.contains(&source_id) {
            return Ok(false);
        }
        target.depends_on.push(source_id);
        if target.task_kind == WorkflowTaskKind::RunActivity
            && target.run_activity_input_task_id.is_empty()
        {
            target.run_activity_input_task_id =
                target.depends_on.last().cloned().unwrap_or_default();
        }
        Ok(true)
    }

    fn remove_dependency(&mut self, source_key: u32, target_key: u32) -> bool {
        let Some(source_id) = self
            .tasks
            .iter()
            .find(|task| task.key == source_key)
            .map(|task| task.id.clone())
        else {
            return false;
        };
        let Some(target) = self.tasks.iter_mut().find(|task| task.key == target_key) else {
            return false;
        };
        let before = target.depends_on.len();
        target
            .depends_on
            .retain(|dependency| dependency != &source_id);
        if target.run_activity_input_task_id == source_id {
            target.run_activity_input_task_id =
                target.depends_on.first().cloned().unwrap_or_default();
        }
        target.depends_on.len() != before
    }

    fn depends_transitively_on(&self, task_id: &str, dependency_id: &str) -> bool {
        let by_id = self
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect::<HashMap<_, _>>();
        let mut pending = vec![task_id];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(task) = by_id.get(id) else {
                continue;
            };
            for dependency in &task.depends_on {
                if dependency == dependency_id {
                    return true;
                }
                pending.push(dependency);
            }
        }
        false
    }

    fn apply_roundtable(&mut self, template: &RoundtableTemplateForm, locale: Locale) {
        let participant_count = template
            .participant_count
            .clamp(MIN_ROUNDTABLE_PARTICIPANTS, MAX_ROUNDTABLE_PARTICIPANTS);
        let opening_ids = (1..=participant_count)
            .map(|seat| format!("seat_{seat}_opening"))
            .collect::<Vec<_>>();
        let review_ids = (1..=participant_count)
            .map(|seat| format!("seat_{seat}_review"))
            .collect::<Vec<_>>();
        let mut tasks = Vec::with_capacity(participant_count * 2 + 1);
        let mut next_key = 1;
        let goal = self.goal.trim().to_string();
        let opening_instruction = tf(
            locale,
            "agents.roundtable.opening_instruction",
            &[("goal", &goal)],
        );
        let review_instruction = tf(
            locale,
            "agents.roundtable.review_instruction",
            &[("goal", &goal)],
        );
        let chair_instruction = tf(
            locale,
            "agents.roundtable.chair_instruction",
            &[("goal", &goal)],
        );

        for (index, id) in opening_ids.iter().enumerate() {
            let mut task = DynamicTaskForm::blank(next_key, id.clone());
            next_key += 1;
            task.instruction.clone_from(&opening_instruction);
            template.participants[index].apply_to(&mut task);
            tasks.push(task);
        }

        for (index, id) in review_ids.iter().enumerate() {
            let mut task = DynamicTaskForm::blank(next_key, id.clone());
            next_key += 1;
            task.instruction.clone_from(&review_instruction);
            task.depends_on.clone_from(&opening_ids);
            template.participants[index].apply_to(&mut task);
            tasks.push(task);
        }

        let mut chair = DynamicTaskForm::blank(next_key, "chair_synthesis".into());
        next_key += 1;
        chair.instruction = chair_instruction;
        chair.depends_on = review_ids;
        template.chair.apply_to(&mut chair);
        tasks.push(chair);

        self.tasks = tasks;
        self.next_task_key = next_key;
    }
}

const WORKFLOW_GRAPH_NODE_WIDTH: i32 = 208;
const WORKFLOW_GRAPH_NODE_HEIGHT: i32 = 112;
const WORKFLOW_GRAPH_COLUMN_GAP: i32 = 112;
const WORKFLOW_GRAPH_ROW_GAP: i32 = 30;
const WORKFLOW_GRAPH_PADDING_X: i32 = 28;
const WORKFLOW_GRAPH_PADDING_TOP: i32 = 58;
const WORKFLOW_GRAPH_PADDING_BOTTOM: i32 = 28;
const WORKFLOW_INSPECTOR_WIDTH_DEFAULT: i32 = 360;
const WORKFLOW_INSPECTOR_WIDTH_MIN: i32 = 280;
const WORKFLOW_INSPECTOR_WIDTH_MAX: i32 = 640;
const WORKFLOW_GRAPH_MIN_WIDTH: i32 = 320;
const WORKFLOW_GRAPH_RESIZER_WIDTH: i32 = 7;

fn clamp_workflow_inspector_width(width: i32, workspace_width: f64) -> i32 {
    let available =
        (workspace_width.floor() as i32 - WORKFLOW_GRAPH_MIN_WIDTH - WORKFLOW_GRAPH_RESIZER_WIDTH)
            .clamp(WORKFLOW_INSPECTOR_WIDTH_MIN, WORKFLOW_INSPECTOR_WIDTH_MAX);
    width.clamp(WORKFLOW_INSPECTOR_WIDTH_MIN, available)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowGraphNode {
    key: u32,
    id: String,
    instruction: String,
    task_kind: WorkflowTaskKind,
    capability_count: usize,
    specialist_id: String,
    executor_key: String,
    level: usize,
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowGraphEdge {
    source_key: u32,
    target_key: u32,
    source_id: String,
    target_id: String,
    path: String,
    mid_x: i32,
    mid_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowGraphStage {
    level: usize,
    x: i32,
    count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowGraphLayout {
    width: i32,
    height: i32,
    nodes: Vec<WorkflowGraphNode>,
    edges: Vec<WorkflowGraphEdge>,
    stages: Vec<WorkflowGraphStage>,
}

fn workflow_graph_edge_path(x1: i32, y1: i32, x2: i32, y2: i32) -> String {
    let bend = ((x2 - x1).abs() / 2).max(32);
    format!(
        "M {x1} {y1} C {} {y1}, {} {y2}, {x2} {y2}",
        x1 + bend,
        x2 - bend
    )
}

fn workflow_graph_port_out(node: &WorkflowGraphNode) -> (i32, i32) {
    (
        node.x + WORKFLOW_GRAPH_NODE_WIDTH,
        node.y + WORKFLOW_GRAPH_NODE_HEIGHT / 2,
    )
}

fn workflow_graph_port_in(node: &WorkflowGraphNode) -> (i32, i32) {
    (node.x, node.y + WORKFLOW_GRAPH_NODE_HEIGHT / 2)
}

fn workflow_graph_canvas_point(
    event: &web_sys::PointerEvent,
    canvas: &web_sys::Element,
    zoom: i32,
) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let scale = zoom as f64 / 100.0;
    (
        (event.client_x() as f64 - rect.left()) / scale,
        (event.client_y() as f64 - rect.top()) / scale,
    )
}

fn workflow_graph_event_element(event: &web_sys::Event) -> Option<web_sys::Element> {
    event.target()?.dyn_into().ok()
}

fn workflow_graph_target_is_graph_chrome(target: &web_sys::Element) -> bool {
    [
        "[data-testid=\"workflow-graph-node\"]",
        "[data-testid=\"workflow-graph-edge-hit\"]",
        "[data-testid=\"workflow-graph-edge-group\"]",
        "[data-testid=\"workflow-graph-edge-delete\"]",
    ]
    .iter()
    .any(|selector| target.closest(selector).ok().flatten().is_some())
}

fn workflow_graph_node_key_from_element(target: &web_sys::Element) -> Option<u32> {
    let node = target
        .closest("[data-testid=\"workflow-graph-node\"]")
        .ok()
        .flatten()?;
    node.get_attribute("data-node-key")?.parse().ok()
}

fn workflow_graph_node_key_from_event(event: &web_sys::PointerEvent) -> Option<u32> {
    workflow_graph_node_key_from_element(&workflow_graph_event_element(event.as_ref())?)
}

/// Hit-test under the cursor. Needed because canvas pointer capture retargets
/// `event.target()` away from the node the pointer is actually over.
fn workflow_graph_node_key_at_client(client_x: i32, client_y: i32) -> Option<u32> {
    let document = web_sys::window()?.document()?;
    let element = document.element_from_point(client_x as f32, client_y as f32)?;
    workflow_graph_node_key_from_element(&element)
}

fn workflow_graph_layout(tasks: &[DynamicTaskForm]) -> WorkflowGraphLayout {
    if tasks.is_empty() {
        return WorkflowGraphLayout {
            width: 520,
            height: 420,
            nodes: vec![],
            edges: vec![],
            stages: vec![],
        };
    }
    let mut by_id = HashMap::new();
    for (index, task) in tasks.iter().enumerate() {
        by_id.entry(task.id.as_str()).or_insert(index);
    }

    // Kahn-style level assignment. A node's level is one greater than its
    // deepest known dependency, so roots share a column and visually read as
    // parallel. Invalid/cyclic leftovers stay visible in a final review column.
    let mut levels = vec![0usize; tasks.len()];
    let mut processed = HashSet::new();
    while processed.len() < tasks.len() {
        let ready = tasks
            .iter()
            .enumerate()
            .filter(|(index, task)| {
                !processed.contains(index)
                    && task.depends_on.iter().all(|dependency| {
                        by_id
                            .get(dependency.as_str())
                            .is_none_or(|dependency_index| processed.contains(dependency_index))
                    })
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if ready.is_empty() {
            let fallback = levels.iter().copied().max().unwrap_or_default() + 1;
            for index in 0..tasks.len() {
                if !processed.contains(&index) {
                    levels[index] = fallback;
                    processed.insert(index);
                }
            }
            break;
        }
        for index in ready {
            levels[index] = tasks[index]
                .depends_on
                .iter()
                .filter_map(|dependency| by_id.get(dependency.as_str()))
                .map(|dependency_index| levels[*dependency_index] + 1)
                .max()
                .unwrap_or_default();
            processed.insert(index);
        }
    }

    let column_count = levels.iter().copied().max().unwrap_or_default() + 1;
    let mut columns = vec![Vec::new(); column_count];
    for (index, level) in levels.iter().copied().enumerate() {
        columns[level].push(index);
    }
    let max_rows = columns.iter().map(Vec::len).max().unwrap_or(1);
    let content_height = max_rows as i32 * WORKFLOW_GRAPH_NODE_HEIGHT
        + (max_rows.saturating_sub(1)) as i32 * WORKFLOW_GRAPH_ROW_GAP;
    let width = WORKFLOW_GRAPH_PADDING_X * 2
        + column_count as i32 * WORKFLOW_GRAPH_NODE_WIDTH
        + column_count.saturating_sub(1) as i32 * WORKFLOW_GRAPH_COLUMN_GAP;
    let natural_height =
        WORKFLOW_GRAPH_PADDING_TOP + content_height + WORKFLOW_GRAPH_PADDING_BOTTOM;
    let height = natural_height.max(420);
    let vertical_offset = (height - natural_height) / 2;
    let mut positions = vec![(0, 0); tasks.len()];
    let mut stages = Vec::with_capacity(column_count);
    for (level, column) in columns.iter().enumerate() {
        let x = WORKFLOW_GRAPH_PADDING_X
            + level as i32 * (WORKFLOW_GRAPH_NODE_WIDTH + WORKFLOW_GRAPH_COLUMN_GAP);
        let column_height = column.len() as i32 * WORKFLOW_GRAPH_NODE_HEIGHT
            + column.len().saturating_sub(1) as i32 * WORKFLOW_GRAPH_ROW_GAP;
        let start_y =
            WORKFLOW_GRAPH_PADDING_TOP + vertical_offset + (content_height - column_height) / 2;
        stages.push(WorkflowGraphStage {
            level,
            x,
            count: column.len(),
        });
        for (row, task_index) in column.iter().copied().enumerate() {
            positions[task_index] = (
                x,
                start_y + row as i32 * (WORKFLOW_GRAPH_NODE_HEIGHT + WORKFLOW_GRAPH_ROW_GAP),
            );
        }
    }
    let nodes = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| WorkflowGraphNode {
            key: task.key,
            id: task.id.clone(),
            instruction: task.instruction.clone(),
            task_kind: task.task_kind,
            capability_count: task.capabilities.len(),
            specialist_id: task.specialist_id.clone(),
            executor_key: task.executor_key.clone(),
            level: levels[index],
            x: positions[index].0,
            y: positions[index].1,
        })
        .collect::<Vec<_>>();
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut edges = Vec::new();
    for target in &nodes {
        let Some(task) = tasks.iter().find(|task| task.key == target.key) else {
            continue;
        };
        for dependency in &task.depends_on {
            let Some(source) = node_by_id.get(dependency.as_str()) else {
                continue;
            };
            let (x1, y1) = workflow_graph_port_out(source);
            let (x2, y2) = workflow_graph_port_in(target);
            edges.push(WorkflowGraphEdge {
                source_key: source.key,
                target_key: target.key,
                source_id: source.id.clone(),
                target_id: target.id.clone(),
                path: workflow_graph_edge_path(x1, y1, x2, y2),
                mid_x: (x1 + x2) / 2,
                mid_y: (y1 + y2) / 2,
            });
        }
    }
    WorkflowGraphLayout {
        width: width.max(520),
        height,
        nodes,
        edges,
        stages,
    }
}

#[derive(Clone, Copy)]
pub(super) struct AgentPanelState {
    pub(super) workflows: RwSignal<Vec<AgentWorkflowSnapshot>>,
    pub(super) session_id: RwSignal<Option<String>>,
    pub(super) options: RwSignal<DynamicAgentEditorOptions>,
    pub(super) dynamic_form: RwSignal<DynamicWorkflowForm>,
    roundtable_form: RwSignal<RoundtableTemplateForm>,
    pub(super) launching: RwSignal<Vec<String>>,
    retry_budgets: RwSignal<HashMap<(String, String), String>>,
    pub(super) error: RwSignal<Option<String>>,
    pub(super) result: RwSignal<Option<AgentWorkflowResultDetail>>,
}

impl AgentPanelState {
    pub(super) fn new(session_id: RwSignal<Option<String>>) -> Self {
        Self {
            workflows: create_rw_signal(vec![]),
            session_id,
            options: create_rw_signal(DynamicAgentEditorOptions::default()),
            dynamic_form: create_rw_signal(DynamicWorkflowForm::default()),
            roundtable_form: create_rw_signal(RoundtableTemplateForm::default()),
            launching: create_rw_signal(vec![]),
            retry_budgets: create_rw_signal(HashMap::new()),
            error: create_rw_signal(None),
            result: create_rw_signal(None),
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().into())
}

fn parse_optional_u32(value: &str, label: &str) -> Result<Option<u32>, String> {
    nonempty(value)
        .map(|value| {
            value
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("{label} must be a positive whole number"))
        })
        .transpose()
}

/// Budget fields accept 0 as an explicit "unlimited" (normalized downstream);
/// empty stays unset, which is also unlimited.
fn parse_budget_u32(value: &str, label: &str) -> Result<Option<u32>, String> {
    nonempty(value)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("{label} must be a whole number (0 = unlimited)"))
        })
        .transpose()
}

fn parse_budget_u64(value: &str, label: &str) -> Result<Option<u64>, String> {
    nonempty(value)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{label} must be a whole number (0 = unlimited)"))
        })
        .transpose()
}

fn parse_optional_u64(value: &str, label: &str) -> Result<Option<u64>, String> {
    nonempty(value)
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("{label} must be a positive whole number"))
        })
        .transpose()
}

fn parse_required_u32(value: &str, label: &str) -> Result<u32, String> {
    parse_optional_u32(value, label)?.ok_or_else(|| format!("{label} is required"))
}

fn parse_required_u64(value: &str, label: &str) -> Result<u64, String> {
    parse_optional_u64(value, label)?.ok_or_else(|| format!("{label} is required"))
}

fn executor_key(executor: &AgentExecutorSelection) -> String {
    executor
        .profile_id
        .as_ref()
        .map(|profile_id| format!("{}:{profile_id}", executor.kind))
        .unwrap_or_else(|| executor.kind.clone())
}

fn parse_executor_key(value: &str) -> Option<AgentExecutorSelection> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (kind, profile_id) = value
        .split_once(':')
        .map_or((value, None), |(kind, profile)| (kind, nonempty(profile)));
    Some(AgentExecutorSelection {
        kind: kind.into(),
        profile_id,
    })
}

pub(super) fn refresh_agent_resources(
    state: AgentPanelState,
    specialists: RwSignal<Vec<Specialist>>,
) {
    spawn_local(async move {
        if let Ok(value) = invoke_checked("list_specialists", JsValue::UNDEFINED).await {
            if let Ok(items) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                specialists.set(items);
            }
        }
        match invoke_checked("get_dynamic_agent_options", JsValue::UNDEFINED).await {
            Ok(value) => {
                if let Ok(options) =
                    serde_wasm_bindgen::from_value::<DynamicAgentEditorOptions>(value)
                {
                    state.options.set(options);
                }
            }
            Err(error) => state.error.set(Some(js_error_text(error))),
        }
    });
}

pub(super) fn refresh_agent_workflows(state: AgentPanelState) {
    spawn_local(async move {
        let args = serde_json::json!({ "sessionId": state.session_id.get_untracked() });
        match invoke_checked("list_agent_workflows", to_value(&args).unwrap()).await {
            Ok(value) => {
                match serde_wasm_bindgen::from_value::<Vec<AgentWorkflowSnapshot>>(value) {
                    Ok(items) => {
                        state.workflows.set(items);
                        state.error.set(None);
                    }
                    Err(parse_error) => state.error.set(Some(parse_error.to_string())),
                }
            }
            Err(invoke_error) => state.error.set(Some(js_error_text(invoke_error))),
        }
    });
}

fn js_error_text(error: JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()
                .and_then(|value| value.as_string())
        })
        .unwrap_or_else(|| "Unknown Agent workflow error".into())
}

#[derive(Clone)]
struct AgentWorkflowGroup {
    frame_id: String,
    title: String,
    snapshots: Vec<AgentWorkflowSnapshot>,
}

fn group_workflows(
    snapshots: Vec<AgentWorkflowSnapshot>,
    sessions: &[SessionInfo],
    session_id: Option<&str>,
) -> Vec<AgentWorkflowGroup> {
    let titles = sessions
        .iter()
        .map(|session| (session.id.as_str(), session.title.as_str()))
        .collect::<HashMap<_, _>>();
    let root_frames = snapshots
        .iter()
        .filter(|snapshot| snapshot.workflow.depth == 0)
        .filter_map(|snapshot| {
            snapshot
                .workflow
                .frame_id
                .as_ref()
                .map(|frame_id| (snapshot.workflow.id.clone(), frame_id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut groups = Vec::<AgentWorkflowGroup>::new();
    for snapshot in snapshots {
        let root_id = if snapshot.workflow.root_workflow_id.is_empty() {
            snapshot.workflow.id.as_str()
        } else {
            snapshot.workflow.root_workflow_id.as_str()
        };
        let frame_id = root_frames
            .get(root_id)
            .cloned()
            .or_else(|| snapshot.workflow.frame_id.clone())
            .unwrap_or_else(|| "unbound".into());
        if let Some(group) = groups.iter_mut().find(|group| group.frame_id == frame_id) {
            group.snapshots.push(snapshot);
            continue;
        }
        let title = titles
            .get(frame_id.as_str())
            .map(|title| (*title).to_string())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| frame_id.clone());
        groups.push(AgentWorkflowGroup {
            frame_id,
            title,
            snapshots: vec![snapshot],
        });
    }
    for group in &mut groups {
        group.snapshots.sort_by(|left, right| {
            left.workflow
                .depth
                .cmp(&right.workflow.depth)
                .then_with(|| right.workflow.updated_at.cmp(&left.workflow.updated_at))
                .then_with(|| left.workflow.id.cmp(&right.workflow.id))
        });
    }
    groups.retain(|group| {
        let Some(session_id) = session_id else {
            return false;
        };
        group.frame_id == session_id
    });
    groups
}

fn status_label(locale: Locale, status: &str) -> String {
    let key = match status {
        "draft" => "agents.status.draft",
        "approved" => "agents.status.approved",
        "running" => "agents.status.running",
        "waiting_run" => "agents.status.waiting_run",
        "succeeded" => "agents.status.succeeded",
        "failed" => "agents.status.failed",
        "cancelled" => "agents.status.cancelled",
        "blocked" => "agents.status.blocked",
        "queued" => "agents.status.queued",
        _ => "agents.status.pending",
    };
    t(locale, key).into()
}

fn risk_label(locale: Locale, risk: &str) -> String {
    let key = match risk {
        "read_only" => "agents.risk.read_only",
        "write" => "agents.risk.write",
        "execute" => "agents.risk.execute",
        "network" => "agents.risk.network",
        _ => "agents.risk.external",
    };
    t(locale, key).into()
}

fn merge_policy_label(locale: Locale, policy: &str) -> String {
    let key = match policy {
        "automatic_cherry_pick" => "agents.task.merge.automatic_cherry_pick",
        "shared_serialized" => "agents.task.merge.shared_serialized",
        "not_applicable" => "agents.task.merge.not_applicable",
        _ => "agents.task.merge.unresolved",
    };
    t(locale, key)
}

fn update_task(
    form: RwSignal<DynamicWorkflowForm>,
    key: u32,
    update: impl FnOnce(&mut DynamicTaskForm),
) {
    form.update(|form| {
        if let Some(task) = form.tasks.iter_mut().find(|task| task.key == key) {
            update(task);
        }
    });
}

fn task_value<T: Default>(
    form: RwSignal<DynamicWorkflowForm>,
    key: u32,
    get: impl FnOnce(&DynamicTaskForm) -> T,
) -> T {
    form.with(|form| {
        form.tasks
            .iter()
            .find(|task| task.key == key)
            .map(get)
            .unwrap_or_default()
    })
}

#[derive(Clone, Copy)]
enum RoundtableAssignment {
    Participant(usize),
    Chair,
}

fn update_roundtable_assignment(
    form: RwSignal<RoundtableTemplateForm>,
    assignment: RoundtableAssignment,
    update: impl FnOnce(&mut RoundtableAssignmentForm),
) {
    form.update(|form| {
        let target = match assignment {
            RoundtableAssignment::Participant(index) => form.participants.get_mut(index),
            RoundtableAssignment::Chair => Some(&mut form.chair),
        };
        if let Some(target) = target {
            update(target);
        }
    });
}

fn roundtable_assignment_value(
    form: RwSignal<RoundtableTemplateForm>,
    assignment: RoundtableAssignment,
    get: impl FnOnce(&RoundtableAssignmentForm) -> String,
) -> String {
    form.with(|form| {
        let target = match assignment {
            RoundtableAssignment::Participant(index) => form.participants.get(index),
            RoundtableAssignment::Chair => Some(&form.chair),
        };
        target.map(get).unwrap_or_default()
    })
}

fn roundtable_assignment_editor(
    assignment: RoundtableAssignment,
    state: AgentPanelState,
    delegation_enabled: RwSignal<bool>,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    view! {
        <div class="roundtable-assignment" data-testid="roundtable-assignment">
            <strong>{move || match assignment {
                RoundtableAssignment::Participant(index) => tf(
                    locale.get(),
                    "agents.roundtable.seat",
                    &[("number", &(index + 1).to_string())],
                ),
                RoundtableAssignment::Chair => t(locale.get(), "agents.roundtable.chair"),
            }}</strong>
            <label>
                <span>{move || t(locale.get(), "agents.task.specialist")}</span>
                <select data-testid="roundtable-specialist"
                    disabled=move || !delegation_enabled.get()
                    on:change=move |event| update_roundtable_assignment(
                        state.roundtable_form,
                        assignment,
                        |target| target.specialist_id = dom_value(&event),
                    )>
                    <option value="" prop:selected=move || roundtable_assignment_value(
                        state.roundtable_form,
                        assignment,
                        |target| target.specialist_id.clone(),
                    ).is_empty()>
                        {move || t(locale.get(), "agents.task.temporary")}
                    </option>
                    <For each=move || specialists.get() key=|specialist| specialist.id.clone()
                        children=move |specialist| {
                            let id = specialist.id.clone();
                            let selected_id = id.clone();
                            view! {
                                <option value=id prop:selected=move || {
                                    roundtable_assignment_value(
                                        state.roundtable_form,
                                        assignment,
                                        |target| target.specialist_id.clone(),
                                    ) == selected_id
                                }>{specialist.name}</option>
                            }
                        }
                    />
                </select>
            </label>
            <label>
                <span>{move || t(locale.get(), "agents.task.executor")}</span>
                <select data-testid="roundtable-executor"
                    disabled=move || !delegation_enabled.get()
                    on:change=move |event| update_roundtable_assignment(
                        state.roundtable_form,
                        assignment,
                        |target| {
                            target.executor_key = dom_value(&event);
                            if !target.executor_key.is_empty() && target.executor_key != "native" {
                                target.model_id.clear();
                            }
                        },
                    )>
                    <option value="" prop:selected=move || roundtable_assignment_value(
                        state.roundtable_form,
                        assignment,
                        |target| target.executor_key.clone(),
                    ).is_empty()>
                        {move || t(locale.get(), "agents.task.auto")}
                    </option>
                    <For each=move || state.options.get().executors key=|executor| executor.id.clone()
                        children=move |executor| {
                            let key_value = executor.id.clone();
                            let selected_key = key_value.clone();
                            let label = if executor.kind == "native" {
                                executor.display_name.clone()
                            } else {
                                format!("{} · {}", executor.kind, executor.display_name)
                            };
                            let label = if executor.available {
                                label
                            } else {
                                format!(
                                    "{label} · {}",
                                    t(locale.get_untracked(), "runtime.unavailable"),
                                )
                            };
                            let supported_features = executor.supported_features.join(", ");
                            view! {
                                <option value=key_value title=supported_features
                                    disabled=!executor.available
                                    prop:selected=move || roundtable_assignment_value(
                                        state.roundtable_form,
                                        assignment,
                                        |target| target.executor_key.clone(),
                                    ) == selected_key>
                                    {label}
                                </option>
                            }
                        }
                    />
                </select>
            </label>
            <label>
                <span>{move || t(locale.get(), "agents.task.model")}</span>
                <select data-testid="roundtable-model"
                    disabled=move || {
                        if !delegation_enabled.get() {
                            return true;
                        }
                        let executor = roundtable_assignment_value(
                            state.roundtable_form,
                            assignment,
                            |target| target.executor_key.clone(),
                        );
                        !executor.is_empty() && executor != "native"
                    }
                    on:change=move |event| update_roundtable_assignment(
                        state.roundtable_form,
                        assignment,
                        |target| target.model_id = dom_value(&event),
                    )>
                    <option value="" prop:selected=move || roundtable_assignment_value(
                        state.roundtable_form,
                        assignment,
                        |target| target.model_id.clone(),
                    ).is_empty()>
                        {move || t(locale.get(), "agents.task.auto")}
                    </option>
                    <For each=move || state.options.get().models key=|model| model.id.clone()
                        children=move |model_option| {
                            let id = model_option.id.clone();
                            let selected_id = id.clone();
                            let label = models.get().into_iter().find(|model| model.id == id)
                                .map(|model| model.label).unwrap_or_else(|| id.clone());
                            view! {
                                <option value=id prop:selected=move || roundtable_assignment_value(
                                    state.roundtable_form,
                                    assignment,
                                    |target| target.model_id.clone(),
                                ) == selected_id>
                                    {if model_option.external {
                                        format!("{label} · external")
                                    } else {
                                        label
                                    }}
                                </option>
                            }
                        }
                    />
                </select>
            </label>
        </div>
    }
}

fn roundtable_template_editor(
    state: AgentPanelState,
    delegation_enabled: RwSignal<bool>,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    view! {
        <details class="dynamic-roundtable" data-testid="roundtable-template">
            <summary>{move || t(locale.get(), "agents.roundtable.title")}</summary>
            <p>{move || t(locale.get(), "agents.roundtable.help")}</p>
            <div class="roundtable-count-row">
                <label>
                    <span>{move || t(locale.get(), "agents.roundtable.participants")}</span>
                    <select data-testid="roundtable-participant-count"
                        disabled=move || !delegation_enabled.get()
                        on:change=move |event| state.roundtable_form.update(|form| {
                            let count = dom_value(&event).parse::<usize>().unwrap_or(
                                MIN_ROUNDTABLE_PARTICIPANTS,
                            );
                            form.set_participant_count(count);
                        })>
                        <option value="2" prop:selected=move || {
                            state.roundtable_form.get().participant_count == 2
                        }>{"2"}</option>
                        <option value="3" prop:selected=move || {
                            state.roundtable_form.get().participant_count == 3
                        }>{"3"}</option>
                    </select>
                </label>
                <span>{move || t(locale.get(), "agents.roundtable.profile_hint")}</span>
            </div>
            <div class="roundtable-assignment-list">
                <For each=move || 0..state.roundtable_form.get().participant_count
                    key=|index| *index
                    children=move |index| roundtable_assignment_editor(
                        RoundtableAssignment::Participant(index),
                        state,
                        delegation_enabled,
                        specialists,
                        models,
                        locale,
                    )
                />
                {roundtable_assignment_editor(
                    RoundtableAssignment::Chair,
                    state,
                    delegation_enabled,
                    specialists,
                    models,
                    locale,
                )}
            </div>
            <div class="roundtable-template-actions">
                <span>{move || t(locale.get(), "agents.roundtable.replace_hint")}</span>
                <button type="button" class="agents-secondary"
                    data-testid="roundtable-apply"
                    disabled=move || {
                        !delegation_enabled.get()
                            || state.dynamic_form.get().goal.trim().is_empty()
                    }
                    on:click=move |_| {
                        let template = state.roundtable_form.get_untracked();
                        let selected_locale = locale.get_untracked();
                        state.dynamic_form.update(|form| {
                            form.apply_roundtable(&template, selected_locale);
                        });
                        state.error.set(None);
                    }>
                    {move || t(locale.get(), "agents.roundtable.apply")}
                </button>
            </div>
        </details>
    }
}

fn dynamic_task_editor(
    task: DynamicTaskForm,
    state: AgentPanelState,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let key = task.key;
    let remove_key = key;
    let skill_query = create_rw_signal(String::new());
    let filtered_skills = create_memo(move |_| {
        let query = skill_query.get();
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return vec![];
        }
        state.options.with(|options| {
            options
                .skills
                .iter()
                .filter(|skill| {
                    [&skill.id, &skill.name, &skill.scope]
                        .into_iter()
                        .any(|value| value.to_lowercase().contains(&query))
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    view! {
        <fieldset class="dynamic-agent-task" data-testid="dynamic-agent-task" data-task-key=key>
            <div class="dynamic-agent-task-head">
                <legend>{move || {
                    let position = state.dynamic_form.with(|form| {
                        form.tasks.iter().position(|task| task.key == key).unwrap_or(0) + 1
                    });
                    format!("{} {position}", t(locale.get(), "agents.task"))
                }}</legend>
                <button type="button" class="agents-danger dynamic-task-remove"
                    aria-label=move || t(locale.get(), "agents.task.remove")
                    disabled=move || state.dynamic_form.with(|form| form.tasks.len() <= 1)
                    on:click=move |_| state.dynamic_form.update(|form| {
                        form.remove_task(remove_key);
                    })>{"×"}</button>
            </div>
            <div class="dynamic-agent-task-grid">
                <label>
                    <span>{move || t(locale.get(), "agents.task.id")}</span>
                    <input data-testid="dynamic-task-id" autocomplete="off"
                        prop:value=move || task_value(state.dynamic_form, key, |task| task.id.clone())
                        on:input=move |event| {
                            let next = event_target_value(&event);
                            let previous = task_value(state.dynamic_form, key, |task| task.id.clone());
                            state.dynamic_form.update(|form| {
                                if let Some(task) = form.tasks.iter_mut().find(|task| task.key == key) {
                                    task.id.clone_from(&next);
                                }
                                for task in &mut form.tasks {
                                    for dependency in &mut task.depends_on {
                                        if dependency == &previous {
                                            dependency.clone_from(&next);
                                        }
                                    }
                                    if task.run_activity_input_task_id == previous {
                                        task.run_activity_input_task_id.clone_from(&next);
                                    }
                                }
                            });
                        } />
                </label>
                <label class="dynamic-task-instruction">
                    <span>{move || t(locale.get(), "agents.task.instruction")}</span>
                    <textarea data-testid="dynamic-task-instruction"
                        prop:value=move || task_value(state.dynamic_form, key, |task| task.instruction.clone())
                        prop:placeholder=move || t(locale.get(), "agents.task.instruction_ph")
                        on:input=move |event| update_task(state.dynamic_form, key, |task| {
                            task.instruction = event_target_value(&event);
                        })></textarea>
                </label>
                <label>
                    <span>{move || t(locale.get(), "agents.task.type")}</span>
                    <select data-testid="dynamic-task-type"
                        on:change=move |event| {
                            let next = dom_value(&event);
                            update_task(state.dynamic_form, key, |task| {
                                task.task_kind = if next == "run_activity" {
                                    if task.run_activity_input_task_id.is_empty() {
                                        task.run_activity_input_task_id = task
                                            .depends_on
                                            .first()
                                            .cloned()
                                            .unwrap_or_default();
                                    }
                                    task.capabilities.clear();
                                    task.skill_ids.clear();
                                    task.specialist_id.clear();
                                    task.output_schema.clear();
                                    task.isolated = false;
                                    task.model_id.clear();
                                    task.executor_key.clear();
                                    task.max_tokens.clear();
                                    task.max_tool_calls.clear();
                                    task.max_cost_microunits.clear();
                                    WorkflowTaskKind::RunActivity
                                } else {
                                    if task.capabilities.is_empty() {
                                        task.capabilities.push("reasoning".into());
                                    }
                                    WorkflowTaskKind::Agent
                                };
                            });
                        }>
                        <option value="agent"
                            prop:selected=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.task_kind,
                            ) == WorkflowTaskKind::Agent>
                            {move || t(locale.get(), "agents.task.type_agent")}
                        </option>
                        <option value="run_activity"
                            prop:selected=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.task_kind,
                            ) == WorkflowTaskKind::RunActivity>
                            {move || t(locale.get(), "agents.task.type_run_activity")}
                        </option>
                    </select>
                </label>
            </div>
            <fieldset class="dynamic-agent-choice-group"
                prop:hidden=move || task_value(
                    state.dynamic_form,
                    key,
                    |task| task.task_kind,
                ) == WorkflowTaskKind::RunActivity>
                <legend>{move || t(locale.get(), "agents.task.capabilities")}</legend>
                <div class="dynamic-agent-checks" data-testid="dynamic-task-capabilities">
                    <For each=move || state.options.get().capabilities
                        key=|capability| capability.id.clone()
                        children=move |capability| {
                            let id = capability.id.clone();
                            let checked_id = id.clone();
                            let update_id = id.clone();
                            view! {
                                <label class="dynamic-agent-check" title=capability.description>
                                    <input type="checkbox"
                                        prop:checked=move || state.dynamic_form.with(|form| {
                                            form.tasks.iter().find(|task| task.key == key)
                                                .is_some_and(|task| task.capabilities.contains(&checked_id))
                                        })
                                        on:change=move |event| {
                                            let checked = event_target_checked(&event);
                                            update_task(state.dynamic_form, key, |task| {
                                                if checked {
                                                    if !task.capabilities.contains(&update_id) {
                                                        task.capabilities.push(update_id.clone());
                                                    }
                                                } else {
                                                    task.capabilities.retain(|id| id != &update_id);
                                                }
                                            });
                                        } />
                                    <span>{capability.display_name}</span>
                                    <small>{risk_label(locale.get(), &capability.risk)}</small>
                                </label>
                            }
                        }
                    />
                </div>
            </fieldset>
            <fieldset class="dynamic-agent-choice-group run-activity-config"
                data-testid="run-activity-config"
                prop:hidden=move || task_value(
                    state.dynamic_form,
                    key,
                    |task| task.task_kind,
                ) != WorkflowTaskKind::RunActivity>
                <legend>{move || t(locale.get(), "agents.run_activity")}</legend>
                <p>{move || t(locale.get(), "agents.run_activity_help")}</p>
                <div class="dynamic-agent-advanced-grid">
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_kind")}</span>
                        <input type="text" value="method_search" readonly />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_context")}</span>
                        <select data-testid="run-activity-context"
                            on:change=move |event| update_task(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_context_id = dom_value(&event),
                            )>
                            <option value="local" selected>{"Local"}</option>
                        </select>
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_input")}</span>
                        <select data-testid="run-activity-input-task"
                            on:change=move |event| update_task(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_input_task_id = dom_value(&event),
                            )>
                            <option value=""
                                prop:selected=move || task_value(
                                    state.dynamic_form,
                                    key,
                                    |task| task.run_activity_input_task_id.clone(),
                                ).is_empty()>
                                {move || t(locale.get(), "agents.run_activity_input_choose")}
                            </option>
                            {move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.depends_on.clone(),
                            ).into_iter().map(|dependency| {
                                let selected_dependency = dependency.clone();
                                view! {
                                    <option value=dependency.clone()
                                        prop:selected=move || task_value(
                                            state.dynamic_form,
                                            key,
                                            |task| task.run_activity_input_task_id.clone(),
                                        ) == selected_dependency>
                                        {dependency}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_pointer")}</span>
                        <input type="text"
                            value="method_search_spec_artifact_version_id" readonly />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_candidates")}</span>
                        <input type="number" min="1" max="50" inputmode="numeric"
                            data-testid="run-activity-max-candidates"
                            prop:value=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_max_candidates.clone(),
                            )
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.run_activity_max_candidates = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_wall")}</span>
                        <input type="number" min="1" max="604800" inputmode="numeric"
                            data-testid="run-activity-max-wall-seconds"
                            prop:value=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_max_wall_seconds.clone(),
                            )
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.run_activity_max_wall_seconds = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_evaluator")}</span>
                        <input type="number" min="1" max="300" inputmode="numeric"
                            data-testid="run-activity-max-evaluator-seconds"
                            prop:value=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_max_evaluator_seconds.clone(),
                            )
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.run_activity_max_evaluator_seconds = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.run_activity_cost")}</span>
                        <input type="number" min="1" inputmode="numeric"
                            data-testid="run-activity-max-cost"
                            prop:value=move || task_value(
                                state.dynamic_form,
                                key,
                                |task| task.run_activity_max_cost_microunits.clone(),
                            )
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.run_activity_max_cost_microunits = event_target_value(&event);
                            }) />
                    </label>
                </div>
            </fieldset>
            <fieldset class="dynamic-agent-choice-group">
                <legend>{move || t(locale.get(), "agents.task.dependencies")}</legend>
                <div class="dynamic-agent-checks dynamic-dependency-checks">
                    {move || {
                        let choices = state.dynamic_form.with(|form| {
                            form.tasks.iter().filter(|task| task.key != key)
                                .map(|task| (task.key, task.id.clone()))
                                .filter(|(_, id)| !id.trim().is_empty())
                                .collect::<Vec<_>>()
                        });
                        if choices.is_empty() {
                            view! { <span class="dynamic-agent-none">{t(locale.get(), "agents.task.no_dependencies")}</span> }.into_view()
                        } else {
                            choices.into_iter().map(|(dependency_key, dependency)| {
                                let checked_dependency = dependency.clone();
                                view! {
                                    <label class="dynamic-agent-check dependency">
                                        <input type="checkbox"
                                            prop:checked=move || state.dynamic_form.with(|form| {
                                                form.tasks.iter().find(|task| task.key == key)
                                                    .is_some_and(|task| task.depends_on.contains(&checked_dependency))
                                            })
                                            on:change=move |event| {
                                                let checked = event_target_checked(&event);
                                                let mut graph_error = None;
                                                state.dynamic_form.update(|form| {
                                                    if checked {
                                                        if let Err(error) = form.add_dependency(
                                                            dependency_key,
                                                            key,
                                                        ) {
                                                            graph_error = Some(error);
                                                        }
                                                    } else {
                                                        form.remove_dependency(dependency_key, key);
                                                    }
                                                });
                                                state.error.set(graph_error.map(|_| {
                                                    t(
                                                        locale.get_untracked(),
                                                        "workflow_studio.graph_cycle",
                                                    ).into()
                                                }));
                                            } />
                                        <span>{dependency}</span>
                                    </label>
                                }
                            }).collect_view()
                        }
                    }}
                </div>
            </fieldset>
            <fieldset class="dynamic-agent-choice-group dynamic-skill-picker"
                data-testid="dynamic-task-skills"
                prop:hidden=move || task_value(
                    state.dynamic_form,
                    key,
                    |task| task.task_kind,
                ) == WorkflowTaskKind::RunActivity>
                <legend>
                    <span>{move || t(locale.get(), "agents.task.skills")}</span>
                    <small>{move || tf(
                        locale.get(),
                        "agents.task.skills_selected",
                        &[(
                            "count",
                            &task_value(
                                state.dynamic_form,
                                key,
                                |task| task.skill_ids.len(),
                            ).to_string(),
                        )],
                    )}</small>
                </legend>
                <Show when=move || !task_value(
                    state.dynamic_form,
                    key,
                    |task| task.skill_ids.clone(),
                ).is_empty()>
                    <div class="dynamic-skill-selected" data-testid="dynamic-task-selected-skills">
                        <For each=move || {
                            let selected = task_value(
                                state.dynamic_form,
                                key,
                                |task| task.skill_ids.clone(),
                            );
                            state.options.with(|options| {
                                options.skills.iter()
                                    .filter(|skill| selected.contains(&skill.id))
                                    .cloned()
                                    .collect::<Vec<_>>()
                            })
                        }
                            key=|skill| skill.id.clone()
                            children=move |skill| {
                                let remove_id = skill.id.clone();
                                let remove_name = skill.name.clone();
                                view! {
                                    <button type="button" data-testid="dynamic-task-selected-skill"
                                        aria-label=move || tf(
                                            locale.get(),
                                            "agents.task.skill_remove",
                                            &[("skill", &remove_name)],
                                        )
                                        on:click=move |_| update_task(
                                            state.dynamic_form,
                                            key,
                                            |task| task.skill_ids.retain(|id| id != &remove_id),
                                        )>
                                        <span>{format!("{} · {}", skill.name, skill.scope)}</span>
                                        {compose_icon("close")}
                                    </button>
                                }
                            }
                        />
                    </div>
                </Show>
                <input type="search" class="dynamic-skill-search"
                    data-testid="dynamic-task-skill-search"
                    autocomplete="off"
                    prop:value=move || skill_query.get()
                    prop:placeholder=move || t(locale.get(), "agents.task.skills_search")
                    aria-label=move || t(locale.get(), "agents.task.skills_search")
                    on:input=move |event| skill_query.set(event_target_value(&event)) />
                <div class="dynamic-skill-results" data-testid="dynamic-task-skill-results">
                    {move || {
                        if skill_query.get().trim().is_empty() {
                            view! {
                                <span class="dynamic-skill-hint">{tf(
                                    locale.get(),
                                    "agents.task.skills_search_hint",
                                    &[("count", &state.options.get().skills.len().to_string())],
                                )}</span>
                            }.into_view()
                        } else if filtered_skills.get().is_empty() {
                            view! {
                                <span class="dynamic-skill-hint">
                                    {t(locale.get(), "agents.task.skills_no_results")}
                                </span>
                            }.into_view()
                        } else {
                            ().into_view()
                        }
                    }}
                    <For each=move || filtered_skills.get()
                        key=|skill| skill.id.clone()
                        children=move |skill| {
                            let id = skill.id.clone();
                            let checked_id = id.clone();
                            let update_id = id.clone();
                            view! {
                                <label class="dynamic-skill-option" title=skill.id
                                    data-testid="dynamic-task-skill-option">
                                    <input type="checkbox"
                                        prop:checked=move || state.dynamic_form.with(|form| {
                                            form.tasks.iter().find(|task| task.key == key)
                                                .is_some_and(|task| task.skill_ids.contains(&checked_id))
                                        })
                                        on:change=move |event| {
                                            let checked = event_target_checked(&event);
                                            update_task(state.dynamic_form, key, |task| {
                                                if checked {
                                                    if !task.skill_ids.contains(&update_id) {
                                                        task.skill_ids.push(update_id.clone());
                                                    }
                                                } else {
                                                    task.skill_ids.retain(|id| id != &update_id);
                                                }
                                            });
                                        } />
                                    <span>{skill.name}</span>
                                    <small>{skill.scope}</small>
                                </label>
                            }
                        }
                    />
                </div>
            </fieldset>
            <label prop:hidden=move || task_value(
                state.dynamic_form,
                key,
                |task| task.task_kind,
            ) == WorkflowTaskKind::RunActivity>
                <span>{move || t(locale.get(), "agents.task.specialist")}</span>
                <select data-testid="dynamic-task-specialist"
                    on:change=move |event| update_task(state.dynamic_form, key, |task| {
                        task.specialist_id = dom_value(&event);
                    })>
                    <option value="" prop:selected=move || task_value(state.dynamic_form, key, |task| task.specialist_id.clone()).is_empty()>
                        {move || t(locale.get(), "agents.task.temporary")}
                    </option>
                    <For each=move || specialists.get() key=|specialist| specialist.id.clone()
                        children=move |specialist| {
                            let id = specialist.id.clone();
                            let selected_id = id.clone();
                            view! {
                                <option value=id prop:selected=move || {
                                    task_value(state.dynamic_form, key, |task| task.specialist_id.clone()) == selected_id
                                }>{specialist.name}</option>
                            }
                        }
                    />
                </select>
            </label>
            <details class="dynamic-agent-advanced"
                prop:hidden=move || task_value(
                    state.dynamic_form,
                    key,
                    |task| task.task_kind,
                ) == WorkflowTaskKind::RunActivity>
                <summary>{move || t(locale.get(), "agents.task.advanced")}</summary>
                <p class="dynamic-agent-advanced-hint">{move || t(locale.get(), "agents.task.advanced_hint")}</p>
                <div class="dynamic-agent-advanced-grid">
                    <label>
                        <span>{move || t(locale.get(), "agents.task.model")}</span>
                        <select data-testid="dynamic-task-model"
                            disabled=move || {
                                let executor = task_value(state.dynamic_form, key, |task| task.executor_key.clone());
                                !executor.is_empty() && executor != "native"
                            }
                            on:change=move |event| update_task(state.dynamic_form, key, |task| {
                                task.model_id = dom_value(&event);
                            })>
                            <option value="" prop:selected=move || task_value(state.dynamic_form, key, |task| task.model_id.clone()).is_empty()>
                                {move || t(locale.get(), "agents.task.auto")}
                            </option>
                            <For each=move || state.options.get().models key=|model| model.id.clone()
                                children=move |model_option| {
                                    let id = model_option.id.clone();
                                    let selected_id = id.clone();
                                    let label = models.get().into_iter().find(|model| model.id == id)
                                        .map(|model| model.label).unwrap_or_else(|| id.clone());
                                    view! {
                                        <option value=id prop:selected=move || {
                                            task_value(state.dynamic_form, key, |task| task.model_id.clone()) == selected_id
                                        }>{if model_option.external { format!("{label} · external") } else { label }}</option>
                                    }
                                }
                            />
                        </select>
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.executor")}</span>
                        <select data-testid="dynamic-task-executor"
                            on:change=move |event| update_task(state.dynamic_form, key, |task| {
                                task.executor_key = dom_value(&event);
                                if !task.executor_key.is_empty() && task.executor_key != "native" {
                                    task.model_id.clear();
                                }
                            })>
                            <option value="" prop:selected=move || task_value(state.dynamic_form, key, |task| task.executor_key.clone()).is_empty()>
                                {move || t(locale.get(), "agents.task.auto")}
                            </option>
                            <For each=move || state.options.get().executors key=|executor| executor.id.clone()
                                children=move |executor| {
                                    let key_value = executor.id.clone();
                                    let selected_key = key_value.clone();
                                    let label = if executor.kind == "native" {
                                        executor.display_name.clone()
                                    } else {
                                        format!("{} · {}", executor.kind, executor.display_name)
                                    };
                                    let label = if executor.available {
                                        label
                                    } else {
                                        format!("{label} · {}", t(locale.get_untracked(), "runtime.unavailable"))
                                    };
                                    let supported_features = executor.supported_features.join(", ");
                                    view! {
                                        <option value=key_value title=supported_features disabled=!executor.available prop:selected=move || {
                                            task_value(state.dynamic_form, key, |task| task.executor_key.clone()) == selected_key
                                        }>{label}</option>
                                    }
                                }
                            />
                        </select>
                    </label>
                    <label class="dynamic-agent-inline-check">
                        <input type="checkbox"
                            prop:checked=move || state.dynamic_form.with(|form| {
                                form.tasks.iter().find(|task| task.key == key).is_some_and(|task| task.isolated)
                            })
                            on:change=move |event| update_task(state.dynamic_form, key, |task| {
                                task.isolated = event_target_checked(&event);
                            }) />
                        <span>{move || t(locale.get(), "agents.task.isolated")}</span>
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.max_tokens")}</span>
                        <input type="number" min="0" inputmode="numeric"
                            prop:placeholder=move || t(locale.get(), "agents.task.max_tokens_hint")
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_tokens.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_tokens = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.max_tools")}</span>
                        <input type="number" min="0" inputmode="numeric"
                            prop:placeholder=move || t(locale.get(), "agents.task.max_tools_hint")
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_tool_calls.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_tool_calls = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.max_cost")}</span>
                        <input type="number" min="0" inputmode="numeric"
                            prop:placeholder=move || t(locale.get(), "agents.task.max_cost_hint")
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_cost_microunits.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_cost_microunits = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.timeout")}</span>
                        <input type="number" min="0" inputmode="numeric"
                            data-testid="dynamic-task-timeout-secs"
                            prop:placeholder=move || t(locale.get(), "agents.task.timeout_hint")
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.timeout_secs.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.timeout_secs = event_target_value(&event);
                            }) />
                    </label>
                    <label class="dynamic-task-schema">
                        <span>{move || t(locale.get(), "agents.task.output_schema")}</span>
                        <textarea spellcheck="false"
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.output_schema.clone())
                            prop:placeholder=move || t(locale.get(), "agents.task.output_schema_hint")
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.output_schema = event_target_value(&event);
                            })></textarea>
                    </label>
                </div>
            </details>
        </fieldset>
    }
}

fn graph_connection_message(locale: Locale, error: &str) -> String {
    let key = if error == "cycle" {
        "workflow_studio.graph_cycle"
    } else {
        "workflow_studio.graph_connect_invalid"
    };
    t(locale, key).into()
}

fn workflow_graph_editor(
    state: AgentPanelState,
    selected_task_key: RwSignal<Option<u32>>,
    connect_from_key: RwSignal<Option<u32>>,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let graph_zoom = create_rw_signal(100_i32);
    let connect_cursor = create_rw_signal::<Option<(f64, f64)>>(None);
    let connect_origin = create_rw_signal::<Option<(f64, f64)>>(None);
    let connect_dragging = create_rw_signal(false);
    let selected_edge = create_rw_signal::<Option<(u32, u32)>>(None);
    let entering_node_keys = create_rw_signal(HashSet::<u32>::new());
    let canvas_ref = create_node_ref::<leptos::html::Div>();
    let workspace_ref = create_node_ref::<leptos::html::Div>();
    let inspector_width = create_rw_signal(WORKFLOW_INSPECTOR_WIDTH_DEFAULT);
    let inspector_resizing = create_rw_signal(false);
    let layout = create_memo(move |_| {
        state
            .dynamic_form
            .with(|form| workflow_graph_layout(&form.tasks))
    });

    let cancel_connect = move || {
        connect_from_key.set(None);
        connect_cursor.set(None);
        connect_origin.set(None);
        connect_dragging.set(false);
    };

    // Studio-level Escape may clear `connect_from_key` without calling
    // `cancel_connect`; keep the rubber-band state in sync.
    create_effect(move |_| {
        if connect_from_key.get().is_none() {
            connect_cursor.set(None);
            connect_origin.set(None);
            connect_dragging.set(false);
        }
    });

    let mark_node_entering = move |key: u32| {
        entering_node_keys.update(|keys| {
            keys.insert(key);
        });
    };

    let finish_connect = move |source_key: u32, target_key: u32| {
        if source_key == target_key {
            cancel_connect();
            return;
        }
        let mut result = Ok(false);
        state.dynamic_form.update(|form| {
            result = form.add_dependency(source_key, target_key);
        });
        match result {
            Ok(_) => {
                selected_task_key.set(Some(target_key));
                selected_edge.set(None);
                cancel_connect();
                state.error.set(None);
            }
            Err(error) => {
                state.error.set(Some(graph_connection_message(
                    locale.get_untracked(),
                    error,
                )));
            }
        }
    };

    {
        let selected_edge_for_key = selected_edge;
        let remove_selected_edge = move || {
            let Some((source_key, target_key)) = selected_edge_for_key.get_untracked() else {
                return false;
            };
            state.dynamic_form.update(|form| {
                form.remove_dependency(source_key, target_key);
            });
            selected_edge_for_key.set(None);
            state.error.set(None);
            true
        };
        let listener = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::wrap(
            Box::new(move |event: web_sys::KeyboardEvent| {
                if event.default_prevented() || crate::text::ime_composing(&event) {
                    return;
                }
                if !matches!(event.key().as_str(), "Delete" | "Backspace") {
                    return;
                }
                if selected_edge_for_key.get_untracked().is_none() {
                    return;
                }
                if remove_selected_edge() {
                    event.prevent_default();
                    event.stop_propagation();
                }
            }),
        );
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref());
        }
        on_cleanup(move || {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "keydown",
                    listener.as_ref().unchecked_ref(),
                );
            }
        });
    }

    create_effect(move |_| {
        let keys = state
            .dynamic_form
            .with(|form| form.tasks.iter().map(|task| task.key).collect::<Vec<_>>());
        if selected_task_key
            .get_untracked()
            .is_none_or(|key| !keys.contains(&key))
        {
            selected_task_key.set(keys.first().copied());
        }
        if connect_from_key
            .get_untracked()
            .is_some_and(|key| !keys.contains(&key))
        {
            cancel_connect();
        }
        if selected_edge
            .get_untracked()
            .is_some_and(|(source, target)| !keys.contains(&source) || !keys.contains(&target))
        {
            selected_edge.set(None);
        }
        entering_node_keys.update(|entering| {
            entering.retain(|key| keys.contains(key));
        });
    });

    let create_parallel_node = move || {
        let mut new_key = None;
        state.dynamic_form.update(|form| {
            new_key = Some(form.add_task());
        });
        if let Some(key) = new_key {
            mark_node_entering(key);
            selected_task_key.set(Some(key));
            selected_edge.set(None);
            cancel_connect();
            state.error.set(None);
        }
    };

    let add_node = move |_| create_parallel_node();

    let add_after_selected = move |_| {
        let Some(source_key) = selected_task_key.get_untracked() else {
            return;
        };
        let new_key = {
            let mut created = None;
            state.dynamic_form.update(|form| {
                created = form.add_task_after(source_key);
            });
            created
        };
        if let Some(key) = new_key {
            mark_node_entering(key);
            selected_task_key.set(Some(key));
            selected_edge.set(None);
            cancel_connect();
            state.error.set(None);
        }
    };

    view! {
        <div class="workflow-graph-workspace" data-testid="workflow-graph-editor"
            node_ref=workspace_ref
            style=move || format!(
                "--workflow-inspector-width:{}px",
                inspector_width.get(),
            )>
            <div class="workflow-graph-main">
                <div class="workflow-graph-toolbar">
                    <div class="workflow-graph-legend">
                        <span class="workflow-graph-legend-item">
                            <i class="parallel"></i>
                            {move || t(locale.get(), "workflow_studio.graph_parallel")}
                        </span>
                        <span class="workflow-graph-legend-item">
                            <i class="serial"></i>
                            {move || t(locale.get(), "workflow_studio.graph_serial")}
                        </span>
                    </div>
                    <div class="workflow-graph-toolbar-actions">
                        <div class="workflow-graph-zoom" aria-label=move || {
                            t(locale.get(), "workflow_studio.graph_zoom_controls")
                        }>
                            <button type="button" data-testid="workflow-graph-zoom-out"
                                title=move || t(locale.get(), "workflow_studio.graph_zoom_out")
                                disabled=move || { graph_zoom.get() <= 60 }
                                on:click=move |_| {
                                    graph_zoom.update(|zoom| *zoom = (*zoom - 10).max(60));
                                }>
                                {"−"}
                            </button>
                            <button type="button" data-testid="workflow-graph-fit"
                                title=move || t(locale.get(), "workflow_studio.graph_fit")
                                on:click=move |_| graph_zoom.set(100)>
                                {move || format!("{}%", graph_zoom.get())}
                            </button>
                            <button type="button" data-testid="workflow-graph-zoom-in"
                                title=move || t(locale.get(), "workflow_studio.graph_zoom_in")
                                disabled=move || { graph_zoom.get() >= 140 }
                                on:click=move |_| {
                                    graph_zoom.update(|zoom| *zoom = (*zoom + 10).min(140));
                                }>
                                {"+"}
                            </button>
                        </div>
                        {move || connect_from_key.get().and_then(|key| {
                            state.dynamic_form.with(|form| {
                                form.tasks.iter().find(|task| task.key == key).map(|task| {
                                    let id = task.id.clone();
                                    view! {
                                        <span class="workflow-graph-connect-hint"
                                            data-testid="workflow-graph-connect-hint">
                                            {tf(
                                                locale.get(),
                                                "workflow_studio.graph_connect_hint",
                                                &[("node", &id)],
                                            )}
                                        </span>
                                        <button type="button" class="workflow-graph-tool-btn"
                                            data-testid="workflow-graph-connect-cancel"
                                            on:click=move |_| cancel_connect()>
                                            {move || t(locale.get(), "workflow_studio.graph_cancel_connect")}
                                        </button>
                                    }
                                })
                            })
                        })}
                        <button type="button" class="workflow-graph-tool-btn workflow-graph-add-after"
                            data-testid="workflow-graph-add-after"
                            disabled=move || selected_task_key.get().is_none()
                            title=move || t(locale.get(), "workflow_studio.graph_add_after")
                            on:click=add_after_selected>
                            {compose_icon("plus")}
                            <span>{move || t(locale.get(), "workflow_studio.graph_add_after")}</span>
                        </button>
                        <button type="button" class="workflow-graph-tool-btn workflow-graph-add-node"
                            data-testid="workflow-graph-add-node"
                            title=move || t(locale.get(), "workflow_studio.graph_add_parallel")
                            on:click=add_node>
                            {compose_icon("plus")}
                            <span>{move || t(locale.get(), "workflow_studio.graph_add_node")}</span>
                        </button>
                    </div>
                </div>
                <div class="workflow-graph-viewport" data-testid="workflow-graph-viewport">
                    <div class="workflow-graph-canvas-space"
                        style=move || {
                            let current = layout.get();
                            let zoom = graph_zoom.get();
                            format!(
                                "width:{}px;height:{}px",
                                current.width * zoom / 100,
                                current.height * zoom / 100,
                            )
                        }>
                        <div class="workflow-graph-canvas"
                            node_ref=canvas_ref
                            data-testid="workflow-graph-canvas"
                            class:connecting=move || connect_from_key.get().is_some()
                            style=move || format!(
                                "width:{}px;height:{}px;transform:scale({});",
                                layout.get().width,
                                layout.get().height,
                                graph_zoom.get() as f64 / 100.0,
                            )
                            aria-label=move || t(locale.get(), "workflow_studio.graph_dblclick_add")
                            on:dblclick=move |event: web_sys::MouseEvent| {
                                let Some(target) =
                                    workflow_graph_event_element(event.as_ref())
                                else {
                                    return;
                                };
                                if workflow_graph_target_is_graph_chrome(&target) {
                                    return;
                                }
                                event.prevent_default();
                                create_parallel_node();
                            }
                            on:pointermove=move |event: web_sys::PointerEvent| {
                                if connect_from_key.get_untracked().is_none() {
                                    return;
                                }
                                let Some(canvas) = canvas_ref.get() else {
                                    return;
                                };
                                let point = workflow_graph_canvas_point(
                                    &event,
                                    &canvas,
                                    graph_zoom.get_untracked(),
                                );
                                connect_cursor.set(Some(point));
                                if let Some(origin) = connect_origin.get_untracked() {
                                    let dx = point.0 - origin.0;
                                    let dy = point.1 - origin.1;
                                    if (dx * dx + dy * dy).sqrt() > 4.0 {
                                        connect_dragging.set(true);
                                    }
                                }
                            }
                            on:pointerup=move |event: web_sys::PointerEvent| {
                                let Some(source_key) = connect_from_key.get_untracked() else {
                                    return;
                                };
                                // Prefer hit-testing: with pointer capture on the canvas,
                                // event.target is the canvas even when released over a node.
                                let target_key = workflow_graph_node_key_at_client(
                                    event.client_x(),
                                    event.client_y(),
                                )
                                .or_else(|| workflow_graph_node_key_from_event(&event));
                                if let Some(target_key) = target_key {
                                    if target_key != source_key {
                                        finish_connect(source_key, target_key);
                                        return;
                                    }
                                }
                                if connect_dragging.get_untracked() {
                                    cancel_connect();
                                }
                            }
                            on:pointerdown=move |event: web_sys::PointerEvent| {
                                if workflow_graph_node_key_from_event(&event).is_some() {
                                    return;
                                }
                                if event
                                    .target()
                                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                    .is_some_and(|target| {
                                        target
                                            .closest("[data-testid=\"workflow-graph-edge-hit\"]")
                                            .ok()
                                            .flatten()
                                            .is_some()
                                    })
                                {
                                    return;
                                }
                                if connect_from_key.get_untracked().is_some() {
                                    cancel_connect();
                                }
                                selected_edge.set(None);
                            }>
                        <svg class="workflow-graph-edges"
                            width=move || layout.get().width
                            height=move || layout.get().height
                            viewBox=move || format!(
                                "0 0 {} {}",
                                layout.get().width,
                                layout.get().height,
                            )
                            aria-hidden="true">
                            <defs>
                                <marker id="workflow-graph-arrow" viewBox="0 0 10 10"
                                    refX="9" refY="5" markerWidth="7" markerHeight="7"
                                    orient="auto-start-reverse">
                                    <path d="M 0 0 L 10 5 L 0 10 z"></path>
                                </marker>
                            </defs>
                            {move || {
                                connect_from_key.get().and_then(|source_key| {
                                    let cursor = connect_cursor.get()?;
                                    let current = layout.get();
                                    let source_node = current
                                        .nodes
                                        .iter()
                                        .find(|node| node.key == source_key)?;
                                    let (x1, y1) = workflow_graph_port_out(source_node);
                                    Some(view! {
                                        <path class="workflow-graph-edge-preview"
                                            data-testid="workflow-graph-edge-preview"
                                            d=workflow_graph_edge_path(
                                                x1,
                                                y1,
                                                cursor.0.round() as i32,
                                                cursor.1.round() as i32,
                                            )></path>
                                    })
                                })
                            }}
                            <For each=move || layout.get().edges
                                key=|edge| (
                                    edge.source_key,
                                    edge.target_key,
                                    edge.source_id.clone(),
                                    edge.target_id.clone(),
                                    edge.path.clone(),
                                )
                                children=move |edge| {
                                    let source_key = edge.source_key;
                                    let target_key = edge.target_key;
                                    let source_id = edge.source_id.clone();
                                    let target_id = edge.target_id.clone();
                                    let path = edge.path.clone();
                                    let mid_x = edge.mid_x;
                                    let mid_y = edge.mid_y;
                                    view! {
                                        <g class="workflow-graph-edge-group"
                                            class:selected=move || {
                                                selected_edge.get()
                                                    == Some((source_key, target_key))
                                            }
                                            data-testid="workflow-graph-edge-group"
                                            data-source=source_id.clone()
                                            data-target=target_id.clone()>
                                            <path class="workflow-graph-edge-hit"
                                                data-testid="workflow-graph-edge-hit"
                                                d=path.clone()
                                                on:click=move |event: web_sys::MouseEvent| {
                                                    event.stop_propagation();
                                                    if selected_edge.get_untracked()
                                                        == Some((source_key, target_key))
                                                    {
                                                        state.dynamic_form.update(|form| {
                                                            form.remove_dependency(
                                                                source_key,
                                                                target_key,
                                                            );
                                                        });
                                                        selected_edge.set(None);
                                                        state.error.set(None);
                                                        return;
                                                    }
                                                    selected_edge.set(Some((source_key, target_key)));
                                                    selected_task_key.set(None);
                                                    cancel_connect();
                                                }></path>
                                            <path class="workflow-graph-edge"
                                                data-testid="workflow-graph-edge"
                                                data-source=source_id
                                                data-target=target_id
                                                d=path
                                                marker-end="url(#workflow-graph-arrow)"></path>
                                            <foreignObject
                                                class="workflow-graph-edge-delete-wrap"
                                                x=mid_x - 11
                                                y=mid_y - 11
                                                width="22"
                                                height="22">
                                                <button type="button"
                                                    xmlns="http://www.w3.org/1999/xhtml"
                                                    class="workflow-graph-edge-delete"
                                                    data-testid="workflow-graph-edge-delete"
                                                    title=move || t(
                                                        locale.get(),
                                                        "workflow_studio.graph_remove_edge",
                                                    )
                                                    on:click=move |event: web_sys::MouseEvent| {
                                                        event.stop_propagation();
                                                        state.dynamic_form.update(|form| {
                                                            form.remove_dependency(
                                                                source_key,
                                                                target_key,
                                                            );
                                                        });
                                                        selected_edge.set(None);
                                                        state.error.set(None);
                                                    }>
                                                    {"×"}
                                                </button>
                                            </foreignObject>
                                        </g>
                                    }
                                }
                            />
                        </svg>
                        <For each=move || layout.get().stages
                            key=|stage| (stage.level, stage.x, stage.count)
                            children=move |stage| {
                                let style = format!(
                                    "left:{}px;width:{}px",
                                    stage.x,
                                    WORKFLOW_GRAPH_NODE_WIDTH,
                                );
                                view! {
                                    <div class="workflow-graph-stage-label" style=style>
                                        <span>{tf(
                                            locale.get(),
                                            "workflow_studio.graph_stage",
                                            &[("number", &(stage.level + 1).to_string())],
                                        )}</span>
                                        {(stage.count > 1).then(|| view! {
                                            <small>{tf(
                                                locale.get(),
                                                "workflow_studio.graph_parallel_count",
                                                &[("count", &stage.count.to_string())],
                                            )}</small>
                                        })}
                                    </div>
                                }
                            }
                        />
                        <For each=move || layout.get().nodes
                            key=|node| format!(
                                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                                node.key,
                                node.id,
                                node.x,
                                node.y,
                                node.instruction,
                                node.capability_count,
                                node.specialist_id,
                                node.executor_key,
                                node.task_kind == WorkflowTaskKind::RunActivity,
                            )
                            children=move |node| {
                                let key = node.key;
                                let select_key = key;
                                let connect_key = key;
                                let delete_key = key;
                                let style = format!(
                                    "left:{}px;top:{}px;width:{}px;height:{}px",
                                    node.x,
                                    node.y,
                                    WORKFLOW_GRAPH_NODE_WIDTH,
                                    WORKFLOW_GRAPH_NODE_HEIGHT,
                                );
                                let node_id = node.id.clone();
                                let connect_node_title_id = node.id.clone();
                                let connect_node_aria_id = node.id.clone();
                                let is_run_activity =
                                    node.task_kind == WorkflowTaskKind::RunActivity;
                                let role = if is_run_activity {
                                    t(locale.get_untracked(), "agents.run_activity").into()
                                } else if node.specialist_id.is_empty() {
                                    t(locale.get_untracked(), "agents.task.temporary").into()
                                } else {
                                    node.specialist_id.clone()
                                };
                                let executor = if node.executor_key.is_empty() {
                                    t(locale.get_untracked(), "agents.task.auto").into()
                                } else {
                                    node.executor_key.clone()
                                };
                                view! {
                                    <div class="workflow-graph-node"
                                        class:run-activity=is_run_activity
                                        class:selected=move || {
                                            selected_task_key.get() == Some(key)
                                        }
                                        class:connecting=move || {
                                            connect_from_key.get() == Some(key)
                                        }
                                        class:connect-target=move || {
                                            connect_from_key.get().is_some_and(|source| source != key)
                                        }
                                        class:entering=move || {
                                            entering_node_keys.with(|keys| keys.contains(&key))
                                        }
                                        style=style
                                        data-testid="workflow-graph-node"
                                        data-node-id=node.id.clone()
                                        data-node-key=key.to_string()
                                        on:animationend=move |event: web_sys::AnimationEvent| {
                                            if event.animation_name()
                                                != "workflow-graph-node-enter"
                                            {
                                                return;
                                            }
                                            entering_node_keys.update(|keys| {
                                                keys.remove(&key);
                                            });
                                        }>
                                        <button type="button" class="workflow-graph-port input"
                                            class:connect-target=move || {
                                                connect_from_key.get().is_some_and(|source| source != key)
                                            }
                                            data-testid="workflow-graph-connect-target"
                                            title=move || t(
                                                locale.get(),
                                                "workflow_studio.graph_connect_target",
                                            )
                                            aria-label=move || t(
                                                locale.get(),
                                                "workflow_studio.graph_connect_target",
                                            )
                                            on:pointerup=move |event: web_sys::PointerEvent| {
                                                event.stop_propagation();
                                                if let Some(source_key) =
                                                    connect_from_key.get_untracked()
                                                {
                                                    if source_key != key {
                                                        finish_connect(source_key, key);
                                                    }
                                                }
                                            }
                                            on:click=move |event: web_sys::MouseEvent| {
                                                event.stop_propagation();
                                                if let Some(source_key) =
                                                    connect_from_key.get_untracked()
                                                {
                                                    if source_key != key {
                                                        finish_connect(source_key, key);
                                                    }
                                                }
                                            }></button>
                                        <button type="button" class="workflow-graph-node-main"
                                            data-testid="workflow-graph-node-select"
                                            on:click=move |_| {
                                                if let Some(source_key) =
                                                    connect_from_key.get_untracked()
                                                {
                                                    if source_key == select_key {
                                                        cancel_connect();
                                                        return;
                                                    }
                                                    finish_connect(source_key, select_key);
                                                } else {
                                                    selected_task_key.set(Some(select_key));
                                                    selected_edge.set(None);
                                                }
                                            }>
                                            <span class="workflow-graph-node-title">
                                                <strong>{node_id}</strong>
                                                <small>{format!("#{}", node.level + 1)}</small>
                                            </span>
                                            <span class="workflow-graph-node-instruction">
                                                {node.instruction}
                                            </span>
                                            <span class="workflow-graph-node-meta">
                                                <code>{role}</code>
                                                {(!is_run_activity).then(|| view! {
                                                    <code>{executor}</code>
                                                    <code>{tf(
                                                        locale.get(),
                                                        "workflow_studio.graph_capabilities",
                                                        &[("count", &node.capability_count.to_string())],
                                                    )}</code>
                                                })}
                                            </span>
                                        </button>
                                        <button type="button" class="workflow-graph-port output"
                                            class:active=move || {
                                                connect_from_key.get() == Some(connect_key)
                                            }
                                            data-testid="workflow-graph-connect"
                                            title=move || tf(
                                                locale.get(),
                                                "workflow_studio.graph_connect_from",
                                                &[("node", &connect_node_title_id)],
                                            )
                                            aria-label=move || tf(
                                                locale.get(),
                                                "workflow_studio.graph_connect_from",
                                                &[("node", &connect_node_aria_id)],
                                            )
                                            on:pointerdown=move |event: web_sys::PointerEvent| {
                                                event.stop_propagation();
                                                connect_from_key.set(Some(connect_key));
                                                selected_task_key.set(Some(connect_key));
                                                selected_edge.set(None);
                                                connect_dragging.set(false);
                                                state.error.set(None);
                                                if let Some(canvas) = canvas_ref.get() {
                                                    let point = workflow_graph_canvas_point(
                                                        &event,
                                                        &canvas,
                                                        graph_zoom.get_untracked(),
                                                    );
                                                    connect_origin.set(Some(point));
                                                    connect_cursor.set(Some(point));
                                                    let _ = canvas.set_pointer_capture(event.pointer_id());
                                                }
                                            }
                                            on:click=move |event: web_sys::MouseEvent| {
                                                event.stop_propagation();
                                            }></button>
                                        {move || (selected_task_key.get() == Some(key)).then(|| {
                                            let source_key = key;
                                            view! {
                                                <button type="button"
                                                    class="workflow-graph-add-next"
                                                    data-testid="workflow-graph-add-next"
                                                    title=move || t(
                                                        locale.get(),
                                                        "workflow_studio.graph_add_after",
                                                    )
                                                    on:click=move |event: web_sys::MouseEvent| {
                                                        event.stop_propagation();
                                                        let mut new_key = None;
                                                        state.dynamic_form.update(|form| {
                                                            new_key = form.add_task_after(source_key);
                                                        });
                                                        if let Some(key) = new_key {
                                                            mark_node_entering(key);
                                                            selected_task_key.set(Some(key));
                                                            selected_edge.set(None);
                                                            cancel_connect();
                                                            state.error.set(None);
                                                        }
                                                    }>
                                                    {compose_icon("plus")}
                                                </button>
                                            }
                                        })}
                                        <button type="button" class="workflow-graph-node-delete"
                                            data-testid="workflow-graph-delete-node"
                                            title=move || t(locale.get(), "agents.task.remove")
                                            aria-label=move || t(locale.get(), "agents.task.remove")
                                            disabled=move || state.dynamic_form.with(|form| {
                                                form.tasks.len() <= 1
                                            })
                                            on:click=move |_| {
                                                state.dynamic_form.update(|form| {
                                                    form.remove_task(delete_key);
                                                });
                                                if connect_from_key.get_untracked()
                                                    == Some(delete_key)
                                                {
                                                    cancel_connect();
                                                }
                                                state.error.set(None);
                                            }>
                                            {compose_icon("close")}
                                        </button>
                                    </div>
                                }
                            }
                        />
                        </div>
                    </div>
                </div>
                <svg class="workflow-graph-minimap"
                    data-testid="workflow-graph-minimap"
                    viewBox=move || format!(
                        "0 0 {} {}",
                        layout.get().width,
                        layout.get().height,
                    )
                    aria-label=move || t(locale.get(), "workflow_studio.graph_minimap")>
                    <For each=move || layout.get().nodes
                        key=|node| (node.key, node.x, node.y)
                        children=move |node| view! {
                            <rect
                                class:selected=move || {
                                    selected_task_key.get() == Some(node.key)
                                }
                                x=node.x
                                y=node.y
                                width=WORKFLOW_GRAPH_NODE_WIDTH
                                height=WORKFLOW_GRAPH_NODE_HEIGHT
                                rx="10"></rect>
                        }
                    />
                </svg>
            </div>
            <div class="workflow-graph-resizer"
                class:dragging=move || inspector_resizing.get()
                data-testid="workflow-graph-resizer"
                role="separator"
                tabindex="0"
                aria-orientation="vertical"
                aria-valuemin=WORKFLOW_INSPECTOR_WIDTH_MIN
                aria-valuemax=WORKFLOW_INSPECTOR_WIDTH_MAX
                aria-valuenow=move || inspector_width.get()
                aria-label=move || t(locale.get(), "workflow_studio.graph_resize_inspector")
                title=move || t(locale.get(), "workflow_studio.graph_resize_inspector")
                on:pointerdown=move |event: web_sys::PointerEvent| {
                    if event.button() != 0 {
                        return;
                    }
                    event.prevent_default();
                    if let Some(target) = event.target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                    {
                        let _ = target.set_pointer_capture(event.pointer_id());
                    }
                    inspector_resizing.set(true);
                }
                on:pointermove=move |event: web_sys::PointerEvent| {
                    if !inspector_resizing.get_untracked() {
                        return;
                    }
                    let Some(workspace) = workspace_ref.get() else {
                        return;
                    };
                    event.prevent_default();
                    let rect = workspace.get_bounding_client_rect();
                    let width = (rect.right() - event.client_x() as f64).round() as i32;
                    inspector_width.set(clamp_workflow_inspector_width(width, rect.width()));
                }
                on:pointerup=move |event: web_sys::PointerEvent| {
                    if let Some(target) = event.target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                    {
                        let _ = target.release_pointer_capture(event.pointer_id());
                    }
                    inspector_resizing.set(false);
                }
                on:pointercancel=move |_| inspector_resizing.set(false)
                on:keydown=move |event: web_sys::KeyboardEvent| {
                    let delta = match event.key().as_str() {
                        "ArrowLeft" => 24,
                        "ArrowRight" => -24,
                        _ => return,
                    };
                    event.prevent_default();
                    let workspace_width = workspace_ref.get()
                        .map(|workspace| workspace.get_bounding_client_rect().width())
                        .unwrap_or(960.0);
                    inspector_width.set(clamp_workflow_inspector_width(
                        inspector_width.get_untracked() + delta,
                        workspace_width,
                    ));
                }></div>
            <aside class="workflow-graph-inspector" data-testid="workflow-graph-inspector">
                {move || selected_task_key.get().and_then(|key| {
                    state.dynamic_form.with(|form| {
                        form.tasks.iter().find(|task| task.key == key).cloned()
                    }).map(|task| {
                        let target_key = task.key;
                        let task_id = task.id.clone();
                        let dependencies = state.dynamic_form.with(|form| {
                            task.depends_on.iter().filter_map(|dependency| {
                                form.tasks.iter()
                                    .find(|candidate| candidate.id == *dependency)
                                    .map(|source| (source.key, dependency.clone()))
                            }).collect::<Vec<_>>()
                        });
                        view! {
                            <div class="workflow-graph-inspector-head">
                                <div>
                                    <span>{move || t(locale.get(), "workflow_studio.graph_selected")}</span>
                                    <strong>{task_id}</strong>
                                </div>
                                <small>{move || t(locale.get(), "workflow_studio.graph_inspector_help")}</small>
                            </div>
                            <div class="workflow-graph-incoming">
                                <span>{move || t(locale.get(), "workflow_studio.graph_incoming")}</span>
                                <div>
                                    {if dependencies.is_empty() {
                                        view! {
                                            <small>{t(locale.get(), "workflow_studio.graph_root")}</small>
                                        }.into_view()
                                    } else {
                                        dependencies.into_iter().map(|(source_key, dependency)| {
                                            view! {
                                                <button type="button"
                                                    data-testid="workflow-graph-remove-edge"
                                                    title=move || t(
                                                        locale.get(),
                                                        "workflow_studio.graph_remove_edge",
                                                    )
                                                    on:click=move |_| {
                                                        state.dynamic_form.update(|form| {
                                                            form.remove_dependency(
                                                                source_key,
                                                                target_key,
                                                            );
                                                        });
                                                        state.error.set(None);
                                                    }>
                                                    <code>{dependency}</code>
                                                    <span aria-hidden="true">{"×"}</span>
                                                </button>
                                            }
                                        }).collect_view()
                                    }}
                                </div>
                            </div>
                            {dynamic_task_editor(task, state, specialists, models, locale)}
                        }
                    })
                })}
            </aside>
        </div>
    }
}

pub(super) fn workflow_studio(
    state: AgentPanelState,
    templates: RwSignal<Vec<WorkflowTemplate>>,
    selected_template_id: RwSignal<Option<String>>,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
    on_back: Callback<()>,
) -> impl IntoView {
    let template_name = create_rw_signal(String::new());
    let template_description = create_rw_signal(String::new());
    let creating = create_rw_signal(false);
    let loaded_id = create_rw_signal::<Option<String>>(None);
    let saving = create_rw_signal(false);
    let selected_task_key = create_rw_signal::<Option<u32>>(None);
    let connect_from_key = create_rw_signal::<Option<u32>>(None);
    let portfolio_open = create_rw_signal(false);
    let portfolio_request = create_rw_signal(String::new());
    let portfolio_model_id = create_rw_signal(String::new());
    let portfolio_draft = create_rw_signal::<Option<SkillPortfolioDraft>>(None);
    let portfolio_loading = create_rw_signal(false);

    // Escape stack for the studio surface (registered while Workflows is open):
    // cancel in-progress connect → close portfolio planner → leave studio.
    window_capture_escape(move || {
        if connect_from_key.get_untracked().is_some() {
            connect_from_key.set(None);
            return true;
        }
        if portfolio_open.get_untracked() {
            portfolio_open.set(false);
            return true;
        }
        on_back.call(());
        true
    });

    create_effect(move |_| {
        let available = state.options.get().models;
        let profiles = models.get();
        let current = portfolio_model_id.get_untracked();
        if available.iter().any(|model| model.id == current) {
            return;
        }
        let selected = profiles
            .iter()
            .find(|profile| profile.active && available.iter().any(|model| model.id == profile.id))
            .map(|profile| profile.id.clone())
            .or_else(|| available.first().map(|model| model.id.clone()))
            .unwrap_or_default();
        portfolio_model_id.set(selected);
    });

    let generate_portfolio = move |_| {
        let request_text = portfolio_request.get_untracked().trim().to_string();
        let model_id = portfolio_model_id.get_untracked();
        if request_text.is_empty() || model_id.is_empty() {
            state.error.set(Some(
                t(
                    locale.get_untracked(),
                    "workflow_studio.portfolio.validation",
                )
                .into(),
            ));
            return;
        }
        portfolio_loading.set(true);
        let args = serde_json::json!({
            "request": {
                "request": request_text,
                "model_id": model_id,
            }
        });
        spawn_local(async move {
            match invoke_checked("plan_skill_portfolio", to_value(&args).unwrap()).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<SkillPortfolioDraft>(value) {
                    Ok(draft) => {
                        portfolio_draft.set(Some(draft));
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error.to_string())),
                },
                Err(error) => state.error.set(Some(js_error_text(error))),
            }
            portfolio_loading.set(false);
        });
    };

    create_effect(move |_| {
        let items = templates.get();
        let selected = selected_template_id.get();
        if selected.is_none() && !creating.get_untracked() {
            if let Some(template) = items.first() {
                selected_template_id.set(Some(template.id.clone()));
            }
            return;
        }
        if creating.get_untracked() || loaded_id.get_untracked() == selected {
            return;
        }
        let Some(id) = selected else {
            return;
        };
        let Some(template) = items.into_iter().find(|template| template.id == id) else {
            return;
        };
        template_name.set(template.name);
        template_description.set(template.description);
        let form = DynamicWorkflowForm::from_proposal(template.proposal);
        selected_task_key.set(form.tasks.first().map(|task| task.key));
        connect_from_key.set(None);
        state.dynamic_form.set(form);
        loaded_id.set(Some(template.id));
        state.error.set(None);
    });

    let start_new = move |_| {
        creating.set(true);
        loaded_id.set(None);
        selected_template_id.set(None);
        template_name.set(String::new());
        template_description.set(String::new());
        let form = DynamicWorkflowForm::default();
        selected_task_key.set(form.tasks.first().map(|task| task.key));
        connect_from_key.set(None);
        state.dynamic_form.set(form);
        state.roundtable_form.set(RoundtableTemplateForm::default());
        state.error.set(None);
    };

    let submit = move |event: ev::SubmitEvent| {
        event.prevent_default();
        if saving.get_untracked() {
            return;
        }
        let name = template_name.get_untracked().trim().to_string();
        if name.is_empty() {
            state.error.set(Some(
                t(locale.get_untracked(), "workflow_studio.name_required").into(),
            ));
            return;
        }
        let proposal = match state.dynamic_form.get_untracked().proposal() {
            Ok(proposal) => proposal,
            Err(error) => {
                state.error.set(Some(error));
                return;
            }
        };
        let selected = selected_template_id.get_untracked();
        let selected_is_builtin = selected.as_ref().is_some_and(|id| {
            templates.with_untracked(|items| {
                items
                    .iter()
                    .any(|template| &template.id == id && template.builtin)
            })
        });
        let template = WorkflowTemplate {
            id: if creating.get_untracked() || selected_is_builtin {
                String::new()
            } else {
                selected.unwrap_or_default()
            },
            name,
            description: template_description.get_untracked().trim().into(),
            proposal,
            builtin: false,
        };
        saving.set(true);
        spawn_local(async move {
            let args = serde_json::json!({ "template": template });
            match invoke_checked("save_workflow_template", to_value(&args).unwrap()).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<WorkflowTemplate>(value) {
                    Ok(saved) => {
                        let saved_id = saved.id.clone();
                        templates.update(|items| {
                            items.retain(|template| template.id != saved_id);
                            items.push(saved);
                            items.sort_by(|left, right| {
                                right
                                    .builtin
                                    .cmp(&left.builtin)
                                    .then_with(|| left.name.cmp(&right.name))
                            });
                        });
                        creating.set(false);
                        loaded_id.set(None);
                        selected_template_id.set(Some(saved_id));
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error.to_string())),
                },
                Err(error) => state.error.set(Some(js_error_text(error))),
            }
            saving.set(false);
        });
    };

    let remove_selected = move |_| {
        let Some(template_id) = selected_template_id.get_untracked() else {
            return;
        };
        if templates.with_untracked(|items| {
            items
                .iter()
                .any(|template| template.id == template_id && template.builtin)
        }) {
            return;
        }
        saving.set(true);
        spawn_local(async move {
            let args = serde_json::json!({ "templateId": template_id });
            match invoke_checked("remove_workflow_template", to_value(&args).unwrap()).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<WorkflowTemplate>>(value) {
                    Ok(items) => {
                        let next = items.first().map(|template| template.id.clone());
                        templates.set(items);
                        creating.set(false);
                        loaded_id.set(None);
                        selected_template_id.set(next);
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error.to_string())),
                },
                Err(error) => state.error.set(Some(js_error_text(error))),
            }
            saving.set(false);
        });
    };

    let editor_enabled = create_rw_signal(true);
    view! {
        <div class="workflow-studio" data-testid="workflow-studio">
            <aside class="workflow-studio-library">
                <button type="button" class="workflow-studio-back"
                    data-testid="workflow-studio-back"
                    on:click=move |_| on_back.call(())>
                    {compose_icon("chevron-left")}
                    <span>{move || t(locale.get(), "workflow_studio.back_to_settings")}</span>
                </button>
                <div class="workflow-studio-library-head">
                    <strong>{move || t(locale.get(), "workflow_studio.library")}</strong>
                    <span>{move || tf(
                        locale.get(),
                        "workflow_studio.count",
                        &[("count", &templates.get().len().to_string())],
                    )}</span>
                </div>
                <div class="workflow-studio-library-actions">
                    <button type="button" class="settings-add-btn" data-testid="workflow-new"
                        on:click=start_new>
                        {move || format!("+ {}", t(locale.get(), "workflow_studio.new"))}
                    </button>
                    <button type="button" class="settings-add-btn" data-testid="portfolio-planner-open"
                        on:click=move |_| {
                            portfolio_draft.set(None);
                            portfolio_open.set(true);
                        }>
                        {move || t(locale.get(), "workflow_studio.plan_from_skills")}
                    </button>
                </div>
                <div class="workflow-studio-template-list">
                    <For each=move || templates.get() key=|template| template.id.clone()
                        children=move |template| {
                            let id = template.id.clone();
                            let selected_id = id.clone();
                            let task_count = template.proposal.tasks.len();
                            view! {
                                <button type="button" class="workflow-template-card"
                                    class:active=move || {
                                        selected_template_id.get().as_deref()
                                            == Some(selected_id.as_str())
                                    }
                                    data-testid="workflow-template-card"
                                    data-workflow-id=template.id
                                    on:click=move |_| {
                                        creating.set(false);
                                        loaded_id.set(None);
                                        selected_template_id.set(Some(id.clone()));
                                    }>
                                    <span class="workflow-template-card-title">
                                        <strong>{template.name}</strong>
                                        {template.builtin.then(|| view! {
                                            <small>{move || t(locale.get(), "workflow_studio.builtin")}</small>
                                        })}
                                    </span>
                                    <span>{template.description}</span>
                                    <code>{move || tf(
                                        locale.get(),
                                        "workflow_studio.tasks",
                                        &[("count", &task_count.to_string())],
                                    )}</code>
                                </button>
                            }
                        }
                    />
                </div>
            </aside>
            <form class="workflow-studio-editor agents-create" data-testid="workflow-studio-editor"
                on:submit=submit>
                <div class="workflow-studio-editor-head">
                    <div class="workflow-studio-editor-identity">
                        <span>{move || if creating.get() {
                            t(locale.get(), "workflow_studio.new")
                        } else {
                            t(locale.get(), "workflow_studio.edit")
                        }}</span>
                        <strong>{move || {
                            let name = template_name.get();
                            if name.trim().is_empty() {
                                t(locale.get(), "workflow_studio.untitled").into()
                            } else {
                                name
                            }
                        }}</strong>
                        <small>{move || {
                            let description = template_description.get();
                            if description.trim().is_empty() {
                                t(locale.get(), "workflow_studio.help").into()
                            } else {
                                description
                            }
                        }}</small>
                    </div>
                    <div class="workflow-studio-editor-status">
                        <span class="workflow-studio-policy-badge">
                            {move || if state.dynamic_form.get().approval_policy
                                == AgentApprovalPolicy::AutoSafe
                            {
                                t(locale.get(), "agents.approval.auto_safe")
                            } else {
                                t(locale.get(), "agents.approval.review_all")
                            }}
                        </span>
                        <span>{move || tf(
                            locale.get(),
                            "workflow_studio.tasks",
                            &[("count", &state.dynamic_form.get().tasks.len().to_string())],
                        )}</span>
                    </div>
                    <div class="workflow-studio-editor-actions">
                        {move || selected_template_id.get().and_then(|id| {
                            templates.get().into_iter()
                                .find(|template| template.id == id && !template.builtin)
                                .map(|_| view! {
                                    <button type="button" class="agents-danger"
                                        data-testid="workflow-delete"
                                        disabled=move || saving.get()
                                        on:click=remove_selected>
                                        {move || t(locale.get(), "workflow_studio.delete")}
                                    </button>
                                })
                        })}
                        <button type="button" class="agents-secondary"
                            on:click=start_new>
                            {move || t(locale.get(), "workflow_studio.reset")}
                        </button>
                        <button type="submit" class="agents-primary" data-testid="workflow-save"
                            disabled=move || {
                                saving.get()
                                    || template_name.get().trim().is_empty()
                                    || !state.dynamic_form.get().ready()
                            }>
                            {move || {
                                if saving.get() {
                                    t(locale.get(), "agents.saving")
                                } else if selected_template_id.get().is_some_and(|id| {
                                    templates.get().into_iter().any(|template| {
                                        template.id == id && template.builtin
                                    })
                                }) {
                                    t(locale.get(), "workflow_studio.save_copy")
                                } else {
                                    t(locale.get(), "workflow_studio.save")
                                }
                            }}
                        </button>
                    </div>
                </div>
                <details class="workflow-studio-config" data-testid="workflow-studio-config">
                    <summary>
                        <span>{compose_icon("settings")}</span>
                        <span>
                            <strong>{move || t(locale.get(), "workflow_studio.configuration")}</strong>
                            <small>{move || t(locale.get(), "workflow_studio.configuration_help")}</small>
                        </span>
                        <span class="workflow-studio-config-chevron">{compose_icon("chevron-right")}</span>
                    </summary>
                    <div class="workflow-studio-config-body">
                {move || selected_template_id.get().and_then(|id| {
                    templates.get().into_iter()
                        .find(|template| template.id == id && template.builtin)
                        .map(|_| view! {
                            <div class="workflow-studio-builtin-note">
                                {move || t(locale.get(), "workflow_studio.builtin_help")}
                            </div>
                        })
                })}
                <div class="workflow-studio-meta">
                    <label>
                        <span>{move || t(locale.get(), "workflow_studio.name")}</span>
                        <input type="text" data-testid="workflow-name"
                            prop:value=move || template_name.get()
                            on:input=move |event| template_name.set(event_target_value(&event)) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "workflow_studio.description")}</span>
                        <input type="text" data-testid="workflow-description"
                            prop:value=move || template_description.get()
                            on:input=move |event| {
                                template_description.set(event_target_value(&event));
                            } />
                    </label>
                </div>
                <label>
                    <span>{move || t(locale.get(), "agents.goal")}</span>
                    <textarea data-testid="workflow-goal"
                        prop:value=move || state.dynamic_form.get().goal
                        prop:placeholder=move || t(locale.get(), "agents.goal_ph")
                        on:input=move |event| state.dynamic_form.update(|form| {
                            form.goal = event_target_value(&event);
                        })></textarea>
                </label>
                {roundtable_template_editor(
                    state,
                    editor_enabled,
                    specialists,
                    models,
                    locale,
                )}
                <div class="dynamic-agent-policy-row">
                    <label>
                        <span>{move || t(locale.get(), "agents.approval_policy")}</span>
                        <select data-testid="workflow-approval-policy"
                            on:change=move |event| state.dynamic_form.update(|form| {
                                form.approval_policy = if dom_value(&event) == "auto_safe" {
                                    AgentApprovalPolicy::AutoSafe
                                } else {
                                    AgentApprovalPolicy::ReviewAll
                                };
                            })>
                            <option value="review_all"
                                prop:selected=move || state.dynamic_form.get().approval_policy
                                    == AgentApprovalPolicy::ReviewAll>
                                {move || t(locale.get(), "agents.approval.review_all")}
                            </option>
                            <option value="auto_safe"
                                prop:selected=move || state.dynamic_form.get().approval_policy
                                    == AgentApprovalPolicy::AutoSafe>
                                {move || t(locale.get(), "agents.approval.auto_safe")}
                            </option>
                        </select>
                    </label>
                    <span>{move || if state.dynamic_form.get().approval_policy
                        == AgentApprovalPolicy::AutoSafe
                    {
                        t(locale.get(), "agents.approval.auto_safe_help")
                    } else {
                        t(locale.get(), "agents.approval.review_all_help")
                    }}</span>
                </div>
                <details class="dynamic-agent-context">
                    <summary>{move || t(locale.get(), "agents.shared_context")}</summary>
                    <textarea data-testid="workflow-context"
                        prop:value=move || state.dynamic_form.get().context
                        prop:placeholder=move || t(locale.get(), "agents.shared_context_ph")
                        on:input=move |event| state.dynamic_form.update(|form| {
                            form.context = event_target_value(&event);
                        })></textarea>
                </details>
                    </div>
                </details>
                <section class="workflow-studio-graph">
                    <div class="workflow-studio-section-head">
                        <div>
                            <strong>{move || t(locale.get(), "workflow_studio.graph")}</strong>
                            <span>{move || t(locale.get(), "workflow_studio.graph_help")}</span>
                        </div>
                    </div>
                    {workflow_graph_editor(
                        state,
                        selected_task_key,
                        connect_from_key,
                        specialists,
                        models,
                        locale,
                    )}
                </section>
                {move || state.error.get().map(|error| view! {
                    <div class="agents-error" data-testid="workflow-studio-error">{error}</div>
                })}
            </form>
            {move || portfolio_open.get().then(|| view! {
                <div class="overlay" role="presentation" data-testid="portfolio-planner-overlay"
                    on:click=move |_| portfolio_open.set(false)>
                    <div class="modal portfolio-planner-modal" role="dialog" aria-modal="true"
                        aria-labelledby="portfolio-planner-title"
                        on:click=move |event| event.stop_propagation()>
                        <div class="ps-head">
                            <h2 id="portfolio-planner-title">
                                {move || t(locale.get(), "workflow_studio.portfolio.title")}
                            </h2>
                            <button type="button" class="ps-close"
                                title=move || t(locale.get(), "workflow_studio.portfolio.close")
                                aria-label=move || t(locale.get(), "workflow_studio.portfolio.close")
                                on:click=move |_| portfolio_open.set(false)>
                                {compose_icon("close")}
                            </button>
                        </div>
                        <p class="hint">
                            {move || t(locale.get(), "workflow_studio.portfolio.subtitle")}
                        </p>
                        <label>
                            {move || t(locale.get(), "workflow_studio.portfolio.request")}
                            <textarea data-testid="portfolio-request"
                                prop:value=move || portfolio_request.get()
                                on:input=move |event| portfolio_request.set(event_target_value(&event))></textarea>
                        </label>
                        <div class="portfolio-planner-fields">
                            <label>
                                {move || t(locale.get(), "workflow_studio.portfolio.model")}
                                <select data-testid="portfolio-model"
                                    disabled=move || portfolio_loading.get()
                                    on:change=move |event| portfolio_model_id.set(dom_value(&event))>
                                    <For each=move || state.options.get().models key=|model| model.id.clone()
                                        children=move |model_option| {
                                            let id = model_option.id.clone();
                                            let selected_id = id.clone();
                                            let label = models.get().into_iter()
                                                .find(|model| model.id == id)
                                                .map(|model| model.label)
                                                .unwrap_or_else(|| id.clone());
                                            let display_label = if model_option.external {
                                                tf(
                                                    locale.get_untracked(),
                                                    "workflow_studio.portfolio.model_external",
                                                    &[("model", &label)],
                                                )
                                            } else {
                                                label
                                            };
                                            view! {
                                                <option value=id prop:selected=move || portfolio_model_id.get() == selected_id>
                                                    {display_label}
                                                </option>
                                            }
                                        }
                                    />
                                    {move || state.options.get().models.is_empty().then(|| view! {
                                        <option value="">
                                            {move || t(locale.get(), "workflow_studio.portfolio.no_models")}
                                        </option>
                                    })}
                                </select>
                            </label>
                        </div>
                        {move || portfolio_draft.get().map(|draft| {
                            let plan = draft.plan.clone();
                            let proposal = draft.proposal.clone();
                            let loc = locale.get();
                            let skill_count = plan.tasks.iter()
                                .flat_map(|task| task.skill_ids.iter())
                                .collect::<HashSet<_>>()
                                .len();
                            let planner_label = plan.planner_model_label.clone();
                            let summary = tf(
                                loc,
                                "workflow_studio.portfolio.summary",
                                &[
                                    ("tasks", &plan.tasks.len().to_string()),
                                    ("skills", &skill_count.to_string()),
                                    ("model", &planner_label),
                                ],
                            );
                            let description_label = planner_label.clone();
                            view! {
                                <section class="portfolio-plan-card" data-testid="portfolio-plan-card">
                                    <strong>{summary}</strong>
                                    <p>{plan.rationale}</p>
                                    <ul>{plan.tasks.into_iter().map(|task| {
                                        let skills = task.skill_ids.join(", ");
                                        let dependencies = task.depends_on.join(", ");
                                        let skill_text = (!skills.is_empty()).then(|| tf(
                                            loc,
                                            "workflow_studio.portfolio.task_skills",
                                            &[("skills", &skills)],
                                        ));
                                        let dependency_text = (!dependencies.is_empty()).then(|| tf(
                                            loc,
                                            "workflow_studio.portfolio.task_after",
                                            &[("tasks", &dependencies)],
                                        ));
                                        view! {
                                            <li><code>{task.id}</code>
                                                {format!(" · {}", task.rationale)}
                                                {skill_text.map(|text| view! {
                                                    <span>{format!(" · {text}")}</span>
                                                })}
                                                {dependency_text.map(|text| view! {
                                                    <span>{format!(" · {text}")}</span>
                                                })}
                                            </li>
                                        }
                                    }).collect_view()}</ul>
                                    <p>{move || t(locale.get(), "workflow_studio.portfolio.validated_unbudgeted")}</p>
                                    <div class="row">
                                        <button type="button" class="primary" data-testid="portfolio-edit-studio"
                                            on:click=move |_| {
                                                let form = DynamicWorkflowForm::from_proposal(proposal.clone());
                                                selected_task_key.set(form.tasks.first().map(|task| task.key));
                                                state.dynamic_form.set(form);
                                                template_name.set(
                                                    t(locale.get_untracked(), "workflow_studio.portfolio.template_name").into(),
                                                );
                                                template_description.set(
                                                    tf(
                                                        locale.get_untracked(),
                                                        "workflow_studio.portfolio.template_description",
                                                        &[("model", &description_label)],
                                                    ),
                                                );
                                                creating.set(true);
                                                loaded_id.set(None);
                                                selected_template_id.set(None);
                                                portfolio_open.set(false);
                                            }>
                                            {move || t(locale.get(), "workflow_studio.portfolio.edit_studio")}
                                        </button>
                                    </div>
                                </section>
                            }
                        })}
                        <div class="row">
                            <button type="button"
                                on:click=move |_| portfolio_open.set(false)>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            <button type="button" class="primary" data-testid="portfolio-generate"
                                disabled=move || portfolio_loading.get() || portfolio_model_id.get().is_empty()
                                on:click=generate_portfolio>
                                {move || if portfolio_loading.get() {
                                    t(locale.get(), "workflow_studio.portfolio.planning")
                                } else {
                                    t(locale.get(), "workflow_studio.portfolio.generate")
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            })}
        </div>
    }
}

fn invoke_workflow_action(command: &'static str, args: serde_json::Value, state: AgentPanelState) {
    spawn_local(async move {
        match invoke_checked(command, to_value(&args).unwrap()).await {
            Ok(_) => refresh_agent_workflows(state),
            Err(error) => state.error.set(Some(js_error_text(error))),
        }
    });
}

fn retry_workflow(snapshot: AgentWorkflowSnapshot, state: AgentPanelState) {
    let workflow_id = snapshot.workflow.id;
    let overrides = match state.retry_budgets.with_untracked(|values| {
        snapshot
            .dynamic
            .tasks
            .iter()
            .filter_map(|task| {
                let raw = values.get(&(workflow_id.clone(), task.id.clone()))?;
                Some((task, raw))
            })
            .try_fold(HashMap::new(), |mut overrides, (task, raw)| {
                let max_tokens = raw.trim().parse::<u32>().map_err(|_| {
                    "Retry token budget must be a whole number (0 = unlimited)".to_string()
                })?;
                if task.budget.max_tokens != Some(max_tokens) {
                    overrides.insert(
                        task.id.clone(),
                        AgentBudgetProposal {
                            max_tokens: Some(max_tokens),
                            max_tool_calls: None,
                            max_cost_microunits: None,
                        },
                    );
                }
                Ok(overrides)
            })
    }) {
        Ok(overrides) => overrides,
        Err(error) => {
            state.error.set(Some(error));
            return;
        }
    };
    let args = serde_json::json!({
        "workflowId": workflow_id.clone(),
        "budgetOverrides": (!overrides.is_empty()).then_some(overrides),
    });
    spawn_local(async move {
        match invoke_checked("retry_agent_workflow", to_value(&args).unwrap()).await {
            Ok(_) => {
                state
                    .retry_budgets
                    .update(|values| values.retain(|(id, _), _| id != &workflow_id));
                refresh_agent_workflows(state);
            }
            Err(error) => state.error.set(Some(js_error_text(error))),
        }
    });
}

fn launch_workflow(workflow_id: String, state: AgentPanelState) {
    if state
        .launching
        .with_untracked(|ids| ids.contains(&workflow_id))
    {
        return;
    }
    state.launching.update(|ids| ids.push(workflow_id.clone()));
    spawn_local(async move {
        let args = serde_json::json!({ "workflowId": workflow_id.clone() });
        match invoke_checked("run_agent_workflow", to_value(&args).unwrap()).await {
            Ok(_) => refresh_agent_workflows(state),
            Err(error) => state.error.set(Some(js_error_text(error))),
        }
        state
            .launching
            .update(|ids| ids.retain(|id| id != &workflow_id));
    });
}

fn open_workflow_result(workflow_id: String, step_id: String, state: AgentPanelState) {
    spawn_local(async move {
        let args = serde_json::json!({
            "workflowId": workflow_id,
            "stepId": step_id,
        });
        match invoke_checked("get_agent_workflow_result", to_value(&args).unwrap()).await {
            Ok(value) => match serde_wasm_bindgen::from_value::<AgentWorkflowResultDetail>(value) {
                Ok(result) => {
                    state.result.set(Some(result));
                    request_animation_frame(|| {
                        let _ = web_sys::window()
                            .and_then(|window| window.document())
                            .and_then(|document| document.get_element_by_id("agent-result-close"))
                            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                            .and_then(|element| element.focus().ok());
                    });
                }
                Err(error) => state.error.set(Some(error.to_string())),
            },
            Err(error) => state.error.set(Some(js_error_text(error))),
        }
    });
}

fn workflow_actions(
    snapshot: &AgentWorkflowSnapshot,
    state: AgentPanelState,
    locale: RwSignal<Locale>,
) -> View {
    if snapshot.workflow.depth > 0 {
        return view! {}.into_view();
    }
    let workflow = snapshot.workflow.clone();
    let workflow_id = workflow.id.clone();
    let approve_id = workflow_id.clone();
    let discard_id = workflow_id.clone();
    let run_id = workflow_id.clone();
    let run_busy_id = workflow_id.clone();
    let cancel_id = workflow_id.clone();
    let retry_snapshot = snapshot.clone();
    let delegation_enabled = snapshot.delegation_enabled;
    let automatic = snapshot.approval_policy == AgentApprovalPolicy::AutoSafe;
    view! {
        <div class="agent-workflow-actions">
            {(workflow.status == "draft").then(|| {
                view! {
                    <button type="button" class="agents-primary" data-testid="agent-approve"
                        disabled=!delegation_enabled
                        on:click=move |_| invoke_workflow_action(
                            "approve_agent_workflow",
                            serde_json::json!({
                                "workflowId": approve_id,
                                "expectedVersion": workflow.version,
                            }),
                            state,
                        )>{if automatic {
                            t(locale.get(), "agents.approve_run")
                        } else {
                            t(locale.get(), "agents.approve")
                        }}</button>
                    <button type="button" class="agents-danger" data-testid="agent-discard"
                        on:click=move |_| invoke_workflow_action(
                            "discard_agent_workflow",
                            serde_json::json!({ "workflowId": discard_id }),
                            state,
                        )>{t(locale.get(), "agents.discard")}</button>
                }
            })}
            {(workflow.status == "approved").then(|| view! {
                <button type="button" class="agents-primary" data-testid="agent-run"
                    disabled=move || !delegation_enabled || state.launching.with(|ids| ids.contains(&run_busy_id))
                    on:click=move |_| launch_workflow(run_id.clone(), state)>
                    {t(locale.get(), "agents.run")}
                </button>
            })}
            {(workflow.status == "running").then(|| view! {
                <button type="button" class="agents-danger" data-testid="agent-cancel"
                    on:click=move |_| invoke_workflow_action(
                        "cancel_agent_workflow",
                        serde_json::json!({ "workflowId": cancel_id }),
                        state,
                    )>{t(locale.get(), "agents.cancel")}</button>
            })}
            {matches!(workflow.status.as_str(), "failed" | "cancelled").then(|| view! {
                <button type="button" class="agents-primary" data-testid="agent-retry"
                    disabled=!delegation_enabled
                    on:click=move |_| retry_workflow(retry_snapshot.clone(), state)>
                    {t(locale.get(), "agents.retry")}
                </button>
            })}
        </div>
    }
    .into_view()
}

fn dynamic_workflow_card(
    snapshot: AgentWorkflowSnapshot,
    state: AgentPanelState,
    locale: RwSignal<Locale>,
) -> View {
    let workflow = snapshot.workflow.clone();
    let workflow_id = workflow.id.clone();
    let status = workflow.status.clone();
    let status_class = format!("agent-workflow-status {status}");
    let dynamic = snapshot.dynamic.clone();
    let policy_label = match dynamic.approval_policy {
        AgentApprovalPolicy::ReviewAll => t(locale.get(), "agents.approval.review_all"),
        AgentApprovalPolicy::AutoSafe => t(locale.get(), "agents.approval.auto_safe"),
    };
    let actions = workflow_actions(&snapshot, state, locale);
    let workflow_delegation_enabled = snapshot.delegation_enabled;
    let nested = workflow.depth > 0;
    let card_class = if nested {
        "agent-workflow-card dynamic nested"
    } else {
        "agent-workflow-card dynamic"
    };
    let root_workflow_id = workflow.root_workflow_id.clone();
    let parent_attempt_id = workflow.parent_attempt_id.clone().unwrap_or_default();
    let depth = workflow.depth;
    view! {
        <article class=card_class data-workflow-id=workflow_id.clone()
            data-root-workflow-id=root_workflow_id data-parent-attempt-id=parent_attempt_id
            data-depth=depth data-schema-version="2">
            <div class="agent-workflow-head">
                <div>
                    <div class="agent-workflow-name">{workflow.name.clone()}</div>
                    <div class="agent-workflow-meta">
                        {nested.then(|| view! {
                            <span class="agent-kind-badge nested">{t(locale.get(), "agents.nested")}</span>
                            <span>{format!(" · {} {} · ", t(locale.get(), "agents.depth"), depth + 1)}</span>
                        })}
                        <span class="agent-kind-badge dynamic">{t(locale.get(), "agents.dynamic")}</span>
                        {format!(" · {policy_label} · max {}", workflow.max_parallel)}
                    </div>
                </div>
                <span class=status_class>{status_label(locale.get(), &status)}</span>
            </div>
            <p class="agent-workflow-goal">{workflow.goal.clone()}</p>
            {workflow.requires_confirmation.then(|| view! {
                <div class="agent-confirm-hint">{t(locale.get(), "agents.confirm_hint")}</div>
            })}
            {(!nested && !workflow_delegation_enabled).then(|| view! {
                <div class="agent-delegation-off">{t(locale.get(), "agents.workflow_disabled")}</div>
            })}
            {(!dynamic.approval_reasons.is_empty()).then(|| view! {
                <section class="agent-approval-reasons" aria-label=t(locale.get(), "agents.approval_reasons")>
                    <strong>{t(locale.get(), "agents.approval_reasons")}</strong>
                    <ul>
                        {dynamic.approval_reasons.clone().into_iter().map(|reason| view! {
                            <li><span class="agent-reason-task">{reason.task_id}</span>{reason.message}</li>
                        }).collect_view()}
                    </ul>
                </section>
            })}
            {actions}
            <div class="agent-step-list dynamic" role="list">
                {dynamic.tasks.into_iter().map(|task| {
                    let result = task.result.clone();
                    let is_run_activity = task.task_kind == WorkflowTaskKind::RunActivity;
                    let run_activity = task.run_activity.clone();
                    let task_status = result.as_ref().map(|result| result.status.as_str())
                        .unwrap_or("pending")
                        .to_string();
                    let attempt_class = format!("agent-attempt-status {task_status}");
                    let specialist = if is_run_activity {
                        t(locale.get(), "agents.run_activity").into()
                    } else {
                        task.specialist_name.clone()
                            .unwrap_or_else(|| t(locale.get(), "agents.task.temporary").into())
                    };
                    let executor = if let Some(activity) = run_activity.as_ref() {
                        format!("{} · {}", activity.activity, activity.context_id)
                    } else {
                        task.executor.profile_id.as_ref()
                            .map(|profile| format!("{} · {profile}", task.executor.kind))
                            .unwrap_or_else(|| task.executor.kind.clone())
                    };
                    let model = run_activity.as_ref()
                        .and_then(|activity| activity.model_profile_id.clone())
                        .or_else(|| task.executor.model_id.clone())
                        .unwrap_or_else(|| "—".into());
                    let summary = result.as_ref().and_then(|result| result.summary.clone());
                    let result_error = result.as_ref().and_then(|result| result.error.clone());
                    let usage = result.as_ref().map(|result| format!(
                        "{} tokens · {} tools · {:.4}",
                        result.input_tokens.saturating_add(result.output_tokens),
                        result.tool_calls,
                        result.cost_microunits as f64 / 1_000_000.0,
                    ));
                    let duration = result.as_ref().and_then(|result| result.duration_secs)
                        .map(|seconds| format!("{seconds}s"));
                    let full_result = result.as_ref().is_some_and(|result| result.full_result_available);
                    let linked_run_id = result.as_ref().and_then(|result| result.run_id.clone());
                    let task_approval_reasons = task.approval_reasons.clone();
                    let task_budget = task.budget.clone();
                    let retry_budget_key = (workflow_id.clone(), task.id.clone());
                    let retry_budget_value = task_budget.max_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_default();
                    let show_retry_budget = !is_run_activity && task_status == "failed";
                    let result_workflow_id = workflow_id.clone();
                    let result_step_id = task.stored_step_id.clone();
                    view! {
                        <section class="agent-step dynamic" role="listitem" data-step-id=task.stored_step_id.clone()>
                            <div class="agent-step-head">
                                <div>
                                    <span class="agent-step-name">{task.id.clone()}</span>
                                    <div class="agent-step-meta">{format!("{specialist} · {executor} · {model}")}</div>
                                </div>
                                <span class=attempt_class>{status_label(locale.get(), &task_status)}</span>
                            </div>
                            <p class="agent-task-instruction">{task.instruction}</p>
                            {(!is_run_activity).then(|| view! {
                                <div class="agent-step-limits">{format!(
                                "{} tokens · {} tools",
                                task_budget.max_tokens.map_or_else(|| "—".into(), |value| value.to_string()),
                                task_budget.max_tool_calls.map_or_else(|| "—".into(), |value| value.to_string()),
                                )}</div>
                            })}
                            {show_retry_budget.then(|| {
                                let key = retry_budget_key.clone();
                                view! {
                                    <label class="agent-retry-budget">
                                        <span>{t(locale.get(), "agents.retry.max_tokens")}</span>
                                        <input type="number" min="1" step="1"
                                            data-testid="agent-retry-max-tokens"
                                            prop:value=retry_budget_value
                                            on:input=move |event| state.retry_budgets.update(|values| {
                                                values.insert(key.clone(), event_target_value(&event));
                                            }) />
                                    </label>
                                }
                            })}
                            {run_activity.map(|activity| view! {
                                <div class="agent-chip-row" data-testid="agent-run-activity">
                                    <span class="agent-chip capability">{activity.activity}</span>
                                    <span class="agent-chip dependency">{activity.context_id}</span>
                                    <span class="agent-chip muted">{format!("{} candidates", activity.max_candidates)}</span>
                                </div>
                            })}
                            <div class="agent-chip-row" aria-label=t(locale.get(), "agents.task.dependencies")>
                                <span class="agent-chip-label">{t(locale.get(), "agents.task.dependencies")}</span>
                                {if task.depends_on.is_empty() {
                                    view! { <span class="agent-chip muted">{t(locale.get(), "agents.task.none")}</span> }.into_view()
                                } else {
                                    task.depends_on.into_iter().map(|dependency| view! {
                                        <span class="agent-chip dependency">{dependency}</span>
                                    }).collect_view()
                                }}
                            </div>
                            <div class="agent-chip-row" aria-label=t(locale.get(), "agents.task.capabilities")>
                                <span class="agent-chip-label">{t(locale.get(), "agents.task.capabilities")}</span>
                                {task.capabilities.into_iter().map(|capability| view! {
                                    <span class="agent-chip capability">{capability}</span>
                                }).collect_view()}
                            </div>
                            {(!task.skill_bindings.is_empty()).then(|| view! {
                                <div class="agent-chip-row" aria-label="Skills">
                                    <span class="agent-chip-label">{"Skills"}</span>
                                    {task.skill_bindings.into_iter().map(|binding| view! {
                                        <span class="agent-chip skill" title=format!("{} · {}", binding.path, binding.skill_md_sha256)>
                                            {format!("{} · {}", binding.name, binding.scope)}
                                        </span>
                                    }).collect_view()}
                                </div>
                            })}
                            <div class="agent-resolved-authority">
                                <div><span>{t(locale.get(), "agents.task.workspace")}</span><strong>{task.workspace_policy}</strong></div>
                                <div><span>{t(locale.get(), "agents.task.merge")}</span><strong>{merge_policy_label(locale.get(), &task.merge_policy)}</strong></div>
                                <div><span>{t(locale.get(), "agents.task.tools")}</span><strong>{if task.tools.is_empty() { "—".into() } else { task.tools.join(", ") }}</strong></div>
                                <div class="agent-authority-flags">
                                    {task.can_write.then(|| view! { <span>{t(locale.get(), "agents.task.write")}</span> })}
                                    {task.can_execute.then(|| view! { <span>{t(locale.get(), "agents.task.execute")}</span> })}
                                    {task.can_access_network.then(|| view! { <span>{t(locale.get(), "agents.task.network")}</span> })}
                                    {(!task.can_write && !task.can_execute && !task.can_access_network).then(|| view! {
                                        <span>{t(locale.get(), "agents.task.read_only")}</span>
                                    })}
                                </div>
                            </div>
                            {(!task_approval_reasons.is_empty()).then(|| view! {
                                <ul class="agent-task-approval-reasons">
                                    {task_approval_reasons.into_iter().map(|reason| view! {
                                        <li>{reason}</li>
                                    }).collect_view()}
                                </ul>
                            })}
                            <div class="agent-current-activity">
                                <span>{t(locale.get(), "agents.task.activity")}</span>
                                <strong>{status_label(locale.get(), &task_status)}</strong>
                                {duration.map(|duration| view! { <small>{duration}</small> })}
                            </div>
                            {summary.map(|summary| view! { <p class="agent-attempt-summary">{summary}</p> })}
                            {result_error.map(|error| view! { <div class="agents-error">{error}</div> })}
                            {usage.map(|usage| view! { <div class="agent-usage">{usage}</div> })}
                            <div class="agent-result-actions">
                                {linked_run_id.map(|run_id| view! {
                                    <span class="agent-chip dependency" data-run-id=run_id.clone()>
                                        {format!("{} · {}", t(locale.get(), "agents.run_id"), run_id)}
                                    </span>
                                })}
                                {full_result.then(|| view! {
                                    <button type="button" class="agents-secondary" data-testid="agent-inspect-result"
                                        on:click=move |_| open_workflow_result(
                                            result_workflow_id.clone(), result_step_id.clone(), state,
                                        )>{t(locale.get(), "agents.inspect_result")}</button>
                                })}
                            </div>
                        </section>
                    }
                }).collect_view()}
            </div>
        </article>
    }.into_view()
}

#[derive(Clone, Debug, PartialEq)]
struct AgentResultPresentation {
    summary: Option<String>,
    diff_summary: Option<String>,
    files_changed: Vec<Value>,
    artifacts: Vec<Value>,
    evidence: Vec<Value>,
    tests: Vec<Value>,
    risks: Vec<Value>,
    details: Vec<(String, Value)>,
    error: Option<String>,
}

impl AgentResultPresentation {
    fn from_response(response: &Value) -> Self {
        let is_envelope = response.get("output").is_some();
        let output = response.get("output").unwrap_or(response);
        let summary = result_string(output.get("summary"));
        let diff_summary = result_string(output.get("diff_summary"));
        let files_changed = result_items(output.get("files_changed"));
        let mut artifacts = result_items(output.get("artifacts"));
        if is_envelope {
            merge_result_artifacts(&mut artifacts, result_items(response.get("artifacts")));
        }
        let persisted_evidence = is_envelope
            .then(|| result_items(response.get("evidence")))
            .unwrap_or_default();
        let evidence = if persisted_evidence.is_empty() {
            result_items(output.get("evidence"))
        } else {
            persisted_evidence
        };
        let tests = result_items(output.get("tests"));
        let risks = result_items(output.get("risks"));
        let details = match output {
            Value::Object(fields) => fields
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "task_id"
                            | "summary"
                            | "diff_summary"
                            | "files_changed"
                            | "artifacts"
                            | "evidence"
                            | "tests"
                            | "risks"
                            | "error"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            Value::Null => vec![],
            value => vec![("result".into(), value.clone())],
        };
        let error =
            result_string(response.get("error")).or_else(|| result_string(output.get("error")));
        Self {
            summary,
            diff_summary,
            files_changed,
            artifacts,
            evidence,
            tests,
            risks,
            details,
            error,
        }
    }

    fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.diff_summary.is_none()
            && self.files_changed.is_empty()
            && self.artifacts.is_empty()
            && self.evidence.is_empty()
            && self.tests.is_empty()
            && self.risks.is_empty()
            && self.details.is_empty()
            && self.error.is_none()
    }
}

fn result_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).and_then(nonempty)
}

fn result_items(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(values)) => values.clone(),
        Some(Value::Null) | None => vec![],
        Some(value) => vec![value.clone()],
    }
}

fn merge_result_artifacts(artifacts: &mut Vec<Value>, persisted: Vec<Value>) {
    let mut identities = artifacts
        .iter()
        .filter_map(result_artifact_identity)
        .collect::<HashSet<_>>();
    for artifact in persisted {
        let Some(identity) = result_artifact_identity(&artifact) else {
            artifacts.push(artifact);
            continue;
        };
        if identities.insert(identity) {
            artifacts.push(artifact);
        }
    }
}

fn result_artifact_identity(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["name", "path", "id"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str).and_then(nonempty))
}

fn result_field_label(key: &str) -> String {
    key.replace('_', " ").replace('-', " ")
}

fn result_value_view(value: Value) -> View {
    match value {
        Value::Null => view! { <span class="agent-result-empty-value">{"—"}</span> }.into_view(),
        Value::Bool(value) => view! { <span>{value.to_string()}</span> }.into_view(),
        Value::Number(value) => view! { <span>{value.to_string()}</span> }.into_view(),
        Value::String(value) => view! {
            <div class="agent-result-markdown md" inner_html=md_to_html(&value)></div>
        }
        .into_view(),
        Value::Array(values) => view! {
            <ul class="agent-result-value-list">
                {values.into_iter().map(|value| view! {
                    <li>{result_value_view(value)}</li>
                }).collect_view()}
            </ul>
        }
        .into_view(),
        Value::Object(fields) => view! {
            <dl class="agent-result-fields">
                {fields.into_iter().map(|(key, value)| view! {
                    <div>
                        <dt>{result_field_label(&key)}</dt>
                        <dd>{result_value_view(value)}</dd>
                    </div>
                }).collect_view()}
            </dl>
        }
        .into_view(),
    }
}

fn result_artifact_view(value: Value, locale: Locale) -> View {
    let Value::Object(mut fields) = value else {
        return result_value_view(value);
    };
    let name = fields
        .remove("name")
        .and_then(|value| value.as_str().map(str::to_string));
    let kind = fields
        .remove("kind")
        .and_then(|value| value.as_str().map(str::to_string))
        .and_then(|value| nonempty(&value));
    let path = fields
        .remove("path")
        .and_then(|value| value.as_str().map(str::to_string))
        .and_then(|value| nonempty(&value));
    fields.remove("id");
    let content = fields
        .remove("content")
        .or_else(|| fields.remove("summary"));
    let title = name
        .and_then(|value| nonempty(&value))
        .or_else(|| path.clone())
        .unwrap_or_else(|| t(locale, "agents.result.output").into());
    let details = (!fields.is_empty()).then_some(Value::Object(fields));
    view! {
        <article class="agent-result-artifact">
            <div class="agent-result-item-head">
                <strong>{title}</strong>
                {kind.map(|kind| view! { <span>{kind}</span> })}
            </div>
            {path.map(|path| view! { <code class="agent-result-reference">{path}</code> })}
            {content.map(result_value_view)}
            {details.map(result_value_view)}
        </article>
    }
    .into_view()
}

fn result_evidence_view(value: Value) -> View {
    let Value::Object(mut fields) = value else {
        return result_value_view(value);
    };
    let kind = fields
        .remove("kind")
        .and_then(|value| value.as_str().map(str::to_string))
        .and_then(|value| nonempty(&value));
    let reference = fields
        .remove("reference")
        .and_then(|value| value.as_str().map(str::to_string))
        .and_then(|value| nonempty(&value));
    let summary = fields
        .remove("summary")
        .or_else(|| fields.remove("evidence"));
    let details = (!fields.is_empty()).then_some(Value::Object(fields));
    view! {
        <article class="agent-result-evidence">
            {kind.map(|kind| view! { <span class="agent-result-kind">{result_field_label(&kind)}</span> })}
            {summary.map(result_value_view)}
            {reference.map(|reference| view! { <code class="agent-result-reference">{reference}</code> })}
            {details.map(result_value_view)}
        </article>
    }
    .into_view()
}

fn workflow_result_dialog(state: AgentPanelState, locale: RwSignal<Locale>) -> View {
    window_capture_escape(move || {
        if state.result.get_untracked().is_none() {
            return false;
        }
        state.result.set(None);
        true
    });
    view! {
        {move || state.result.get().map(|result| {
            let current_locale = locale.get();
            let step = result.step_id.rsplit(':').next().unwrap_or(&result.step_id);
            let attempt = result.attempt.to_string();
            let title = format!(
                "{} · {} · {}",
                step,
                status_label(current_locale, &result.status),
                tf(current_locale, "agents.result.attempt", &[("number", &attempt)]),
            );
            let presentation = AgentResultPresentation::from_response(&result.response);
            let empty = presentation.is_empty();
            let has_changes = presentation.diff_summary.is_some()
                || !presentation.files_changed.is_empty();
            view! {
                <div class="overlay agent-result-overlay" role="presentation"
                    on:click=move |_| state.result.set(None)>
                    <div class="modal artifact-modal agent-result-modal" role="dialog" aria-modal="true"
                        aria-labelledby="agent-result-title"
                        tabindex="-1"
                        on:click=|event| event.stop_propagation()>
                        <div class="agent-result-head">
                            <div>
                                <h2 id="agent-result-title">{t(locale.get(), "agents.result.title")}</h2>
                                <span>{title}</span>
                            </div>
                            <button type="button" class="ps-close"
                                id="agent-result-close"
                                title=t(locale.get(), "agents.result.close")
                                aria-label=t(locale.get(), "agents.result.close")
                                on:click=move |_| state.result.set(None)>
                                {compose_icon("close")}
                            </button>
                        </div>
                        <div class="agent-result-body" data-testid="agent-result-content">
                            {presentation.error.map(|error| view! {
                                <div class="agents-error agent-result-error" role="alert">{error}</div>
                            })}
                            {presentation.summary.map(|summary| view! {
                                <section class="agent-result-section agent-result-summary"
                                    data-testid="agent-result-summary">
                                    <h3>{t(locale.get(), "agents.result.summary")}</h3>
                                    {result_value_view(Value::String(summary))}
                                </section>
                            })}
                            {has_changes.then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-changes">
                                    <h3>{t(locale.get(), "agents.result.changes")}</h3>
                                    {presentation.diff_summary.map(|summary| result_value_view(Value::String(summary)))}
                                    {(!presentation.files_changed.is_empty()).then(|| view! {
                                        <ul class="agent-result-files">
                                            {presentation.files_changed.into_iter().map(|file| view! {
                                                <li>{result_value_view(file)}</li>
                                            }).collect_view()}
                                        </ul>
                                    })}
                                </section>
                            })}
                            {(!presentation.artifacts.is_empty()).then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-artifacts">
                                    <h3>{t(locale.get(), "agents.result.artifacts")}</h3>
                                    <div class="agent-result-card-list">
                                        {presentation.artifacts.into_iter().map(|artifact| {
                                            result_artifact_view(artifact, locale.get())
                                        }).collect_view()}
                                    </div>
                                </section>
                            })}
                            {(!presentation.evidence.is_empty()).then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-evidence">
                                    <h3>{t(locale.get(), "agents.result.evidence")}</h3>
                                    <div class="agent-result-card-list">
                                        {presentation.evidence.into_iter().map(result_evidence_view).collect_view()}
                                    </div>
                                </section>
                            })}
                            {(!presentation.tests.is_empty()).then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-tests">
                                    <h3>{t(locale.get(), "agents.result.tests")}</h3>
                                    <ul class="agent-result-value-list">
                                        {presentation.tests.into_iter().map(|test| view! {
                                            <li>{result_value_view(test)}</li>
                                        }).collect_view()}
                                    </ul>
                                </section>
                            })}
                            {(!presentation.risks.is_empty()).then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-risks">
                                    <h3>{t(locale.get(), "agents.result.risks")}</h3>
                                    <ul class="agent-result-value-list agent-result-risk-list">
                                        {presentation.risks.into_iter().map(|risk| view! {
                                            <li>{result_value_view(risk)}</li>
                                        }).collect_view()}
                                    </ul>
                                </section>
                            })}
                            {(!presentation.details.is_empty()).then(|| view! {
                                <section class="agent-result-section" data-testid="agent-result-details">
                                    <h3>{t(locale.get(), "agents.result.details")}</h3>
                                    <dl class="agent-result-fields">
                                        {presentation.details.into_iter().map(|(key, value)| view! {
                                            <div>
                                                <dt>{result_field_label(&key)}</dt>
                                                <dd>{result_value_view(value)}</dd>
                                            </div>
                                        }).collect_view()}
                                    </dl>
                                </section>
                            })}
                            {empty.then(|| view! {
                                <div class="agent-result-empty">{t(locale.get(), "agents.result.empty")}</div>
                            })}
                        </div>
                    </div>
                </div>
            }
        })}
    }
    .into_view()
}

pub(super) fn agent_workflows_panel(
    state: AgentPanelState,
    sessions: RwSignal<Vec<SessionInfo>>,
    delegation_enabled: RwSignal<bool>,
    locale: RwSignal<Locale>,
    open_workflows: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="agents-pane dynamic-agents-panel" data-testid="agent-workflows" data-panel-version="2">
            <div class="agents-inline-notice">
                <strong>{move || t(locale.get(), "agents.inline_notice_title")}</strong>
                <span>{move || t(locale.get(), "agents.inline_notice")}</span>
                <button type="button" class="agents-secondary agents-manage-workflows"
                    data-testid="agent-open-workflows"
                    on:click=move |_| open_workflows.call(())>
                    {compose_icon("branch")}
                    <span>{move || t(locale.get(), "agents.manage_workflows")}</span>
                </button>
            </div>
            {move || (!delegation_enabled.get()).then(|| view! {
                <div class="agents-disabled">{t(locale.get(), "agents.disabled")}</div>
            })}
            {move || state.error.get().map(|message| view! {
                <div class="agents-error" role="alert">{message}</div>
            })}
            <div class="agent-workflow-groups" aria-live="polite">
                {move || {
                    let session_id = state.session_id.get();
                    let groups = group_workflows(
                        state.workflows.get(),
                        &sessions.get(),
                        session_id.as_deref(),
                    );
                    if groups.is_empty() {
                        view! {
                            <div class="rp-empty"><p>{t(locale.get(), "agents.empty")}</p></div>
                        }.into_view()
                    } else {
                        groups.into_iter().map(|group| {
                            let frame_id = group.frame_id.clone();
                            view! {
                                <section class="agent-workflow-group" data-frame-id=frame_id>
                                    <div class="agent-workflow-group-head">
                                        <span>{t(locale.get(), "agents.conversation")}</span>
                                        <strong>{group.title}</strong>
                                        <small>{format!(
                                            "{} {}",
                                            group.snapshots.len(),
                                            t(locale.get(), "agents.workflow_count"),
                                        )}</small>
                                    </div>
                                    <div class="agent-workflow-group-list">
                                        {group.snapshots.into_iter().map(|snapshot| dynamic_workflow_card(
                                            snapshot,
                                            state,
                                            locale,
                                        )).collect_view()}
                                    </div>
                                </section>
                            }
                        }).collect_view()
                    }
                }}
            </div>
            {workflow_result_dialog(state, locale)}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_width_keeps_both_panes_usable() {
        assert_eq!(clamp_workflow_inspector_width(120, 1200.0), 280);
        assert_eq!(clamp_workflow_inspector_width(900, 1200.0), 640);
        assert_eq!(clamp_workflow_inspector_width(500, 727.0), 400);
    }

    #[test]
    fn arbitrary_tasks_round_trip() {
        let mut form = DynamicWorkflowForm::default();
        form.goal = "Compare two analyses".into();
        form.tasks[0].instruction = "Interpret the input".into();
        form.tasks[0].capabilities = vec!["reasoning".into(), "project_read".into()];
        form.add_task();
        form.tasks[1].id = "compare".into();
        form.tasks[1].instruction = "Compare the interpretations".into();
        form.tasks[1].depends_on = vec!["task_1".into()];
        form.tasks[1].executor_key = "native".into();
        form.tasks[1].max_tokens = "2048".into();
        form.tasks[1].timeout_secs = "0".into();
        form.tasks[1].output_schema = r#"{"type":"object"}"#.into();

        let proposal = form.proposal().expect("valid arbitrary workflow");
        assert_eq!(proposal.tasks.len(), 2);
        assert_eq!(proposal.tasks[1].depends_on, ["task_1"]);
        assert_eq!(proposal.tasks[1].executor.as_ref().unwrap().kind, "native");
        assert_eq!(
            proposal.tasks[1].budget.as_ref().unwrap().max_tokens,
            Some(2048)
        );
        assert_eq!(proposal.tasks[1].timeout_secs, Some(0));
        assert_eq!(
            proposal.tasks[1].output_schema.as_ref().unwrap()["type"],
            "object"
        );
        assert!(proposal
            .tasks
            .iter()
            .all(|task| task.specialist_id.is_none()));
    }

    #[test]
    fn run_activity_round_trips_without_agent_only_fields() {
        let proposal = DynamicAgentWorkflowProposal {
            goal: "Develop a method".into(),
            context: String::new(),
            approval_policy: AgentApprovalPolicy::ReviewAll,
            tasks: vec![
                DynamicAgentTaskProposal {
                    id: "prepare".into(),
                    instruction: "Freeze the evaluator".into(),
                    depends_on: vec![],
                    task_kind: WorkflowTaskKind::Agent,
                    run_activity: None,
                    capabilities: vec!["code_run".into()],
                    skill_ids: vec![],
                    specialist_id: None,
                    output_schema: Some(serde_json::json!({
                        "type": "object",
                        "required": ["method_search_spec_artifact_version_id"],
                        "properties": {
                            "method_search_spec_artifact_version_id": { "type": "string" }
                        }
                    })),
                    isolated: false,
                    model_id: None,
                    executor: None,
                    budget: None,
                    timeout_secs: None,
                },
                DynamicAgentTaskProposal {
                    id: "search".into(),
                    instruction: "Run the method search".into(),
                    depends_on: vec!["prepare".into()],
                    task_kind: WorkflowTaskKind::RunActivity,
                    run_activity: Some(RunActivityProposal {
                        activity: "method_search".into(),
                        context_id: "local".into(),
                        input_task_id: "prepare".into(),
                        spec_output_pointer: "method_search_spec_artifact_version_id".into(),
                        max_candidates: 20,
                        max_wall_seconds: 14_400,
                        max_evaluator_seconds: 120,
                        max_cost_microunits: 5_000_000,
                    }),
                    capabilities: vec![],
                    skill_ids: vec![],
                    specialist_id: None,
                    output_schema: None,
                    isolated: false,
                    model_id: None,
                    executor: None,
                    budget: None,
                    timeout_secs: None,
                },
            ],
        };
        let round_tripped = DynamicWorkflowForm::from_proposal(proposal.clone())
            .proposal()
            .unwrap();
        assert_eq!(round_tripped, proposal);
        let activity = &round_tripped.tasks[1];
        assert!(activity.capabilities.is_empty());
        assert!(activity.skill_ids.is_empty());
        assert!(activity.budget.is_none());
        assert!(activity.output_schema.is_none());
    }

    #[test]
    fn add_task_after_places_node_in_next_stage() {
        let mut form = DynamicWorkflowForm::default();
        form.tasks[0].id = "fetch".into();
        form.tasks[0].instruction = "Fetch data".into();
        let source_key = form.tasks[0].key;
        let next_key = form
            .add_task_after(source_key)
            .expect("creates dependent task");
        form.tasks
            .iter()
            .find(|task| task.key == next_key)
            .map(|task| {
                assert_eq!(task.depends_on, ["fetch"]);
            })
            .expect("created task exists");

        let layout = workflow_graph_layout(&form.tasks);
        let fetch = layout.nodes.iter().find(|node| node.id == "fetch").unwrap();
        let next = layout
            .nodes
            .iter()
            .find(|node| node.key == next_key)
            .unwrap();
        assert_eq!(fetch.level, 0);
        assert_eq!(next.level, 1);
        assert!(next.x > fetch.x);
    }

    #[test]
    fn graph_layout_places_parallel_roots_before_their_fan_in() {
        let mut form = DynamicWorkflowForm::default();
        form.tasks[0].id = "supporting".into();
        form.tasks[0].instruction = "Find supporting evidence".into();
        let challenging_key = form.add_task();
        form.tasks[1].id = "challenging".into();
        form.tasks[1].instruction = "Find challenging evidence".into();
        let synthesis_key = form.add_task();
        form.tasks[2].id = "synthesis".into();
        form.tasks[2].instruction = "Synthesize both branches".into();
        form.tasks[2].depends_on = vec!["supporting".into(), "challenging".into()];

        let layout = workflow_graph_layout(&form.tasks);
        let supporting = layout
            .nodes
            .iter()
            .find(|node| node.id == "supporting")
            .unwrap();
        let challenging = layout
            .nodes
            .iter()
            .find(|node| node.key == challenging_key)
            .unwrap();
        let synthesis = layout
            .nodes
            .iter()
            .find(|node| node.key == synthesis_key)
            .unwrap();
        assert_eq!(supporting.level, 0);
        assert_eq!(challenging.level, 0);
        assert_ne!(supporting.y, challenging.y);
        assert_eq!(synthesis.level, 1);
        assert!(synthesis.x > supporting.x);
        assert_eq!(layout.edges.len(), 2);
    }

    #[test]
    fn graph_connections_update_dependencies_and_reject_cycles() {
        let mut form = DynamicWorkflowForm::default();
        form.tasks[0].id = "root".into();
        let root_key = form.tasks[0].key;
        let middle_key = form.add_task();
        form.tasks[1].id = "middle".into();
        let final_key = form.add_task();
        form.tasks[2].id = "final".into();

        assert_eq!(form.add_dependency(root_key, middle_key), Ok(true));
        assert_eq!(form.add_dependency(middle_key, final_key), Ok(true));
        assert_eq!(form.add_dependency(final_key, root_key), Err("cycle"));
        assert_eq!(form.tasks[0].depends_on, Vec::<String>::new());
        assert_eq!(form.tasks[1].depends_on, ["root"]);
        assert_eq!(form.tasks[2].depends_on, ["middle"]);

        assert!(form.remove_dependency(middle_key, final_key));
        assert!(form.tasks[2].depends_on.is_empty());
        form.remove_task(middle_key);
        assert_eq!(form.tasks.len(), 2);
    }

    #[test]
    fn roundtable_template_builds_parallel_openings_cross_review_and_chair() {
        let mut template = RoundtableTemplateForm::default();
        template.participants[0] = RoundtableAssignmentForm {
            specialist_id: "reader".into(),
            model_id: "native-model-that-must-be-cleared".into(),
            executor_key: "acp:codex".into(),
        };
        template.participants[1] = RoundtableAssignmentForm {
            specialist_id: "reviewer".into(),
            model_id: "opus".into(),
            executor_key: "native".into(),
        };
        template.chair = RoundtableAssignmentForm {
            specialist_id: String::new(),
            model_id: String::new(),
            executor_key: "acp:kimi".into(),
        };

        let mut form = DynamicWorkflowForm::default();
        form.goal = "Choose a website architecture".into();
        form.context = "Mobile and desktop must share the same information model.".into();
        form.apply_roundtable(&template, Locale::En);
        let proposal = form.proposal().expect("roundtable proposal is valid");

        assert_eq!(proposal.tasks.len(), 5);
        assert_eq!(
            proposal
                .tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            [
                "seat_1_opening",
                "seat_2_opening",
                "seat_1_review",
                "seat_2_review",
                "chair_synthesis",
            ]
        );
        assert!(proposal.tasks[..2]
            .iter()
            .all(|task| task.depends_on.is_empty()));
        assert!(proposal
            .tasks
            .iter()
            .all(|task| task.instruction.contains("Choose a website architecture")));
        assert!(proposal.tasks[2..4]
            .iter()
            .all(|task| { task.depends_on == ["seat_1_opening", "seat_2_opening"] }));
        assert_eq!(
            proposal.tasks[4].depends_on,
            ["seat_1_review", "seat_2_review"]
        );
        // Budgets are an advanced override: the template leaves them unset,
        // which resolves to unlimited at planning time.
        assert!(proposal.tasks.iter().all(|task| task.budget.is_none()));

        for task in [&proposal.tasks[0], &proposal.tasks[2]] {
            assert_eq!(task.specialist_id.as_deref(), Some("reader"));
            assert_eq!(task.executor.as_ref().unwrap().kind, "acp");
            assert_eq!(
                task.executor.as_ref().unwrap().profile_id.as_deref(),
                Some("codex")
            );
            assert_eq!(task.model_id, None);
        }
        for task in [&proposal.tasks[1], &proposal.tasks[3]] {
            assert_eq!(task.specialist_id.as_deref(), Some("reviewer"));
            assert!(task
                .capabilities
                .iter()
                .any(|capability| capability == "review"));
            assert_eq!(task.executor.as_ref().unwrap().kind, "native");
            assert_eq!(task.model_id.as_deref(), Some("opus"));
        }
        assert_eq!(
            proposal.tasks[4]
                .executor
                .as_ref()
                .unwrap()
                .profile_id
                .as_deref(),
            Some("kimi")
        );
    }

    #[test]
    fn roundtable_template_caps_at_three_participants_and_seven_tasks() {
        let mut template = RoundtableTemplateForm::default();
        template.set_participant_count(99);
        assert_eq!(template.participant_count, MAX_ROUNDTABLE_PARTICIPANTS);

        let mut form = DynamicWorkflowForm::default();
        form.goal = "Evaluate three independent positions".into();
        form.apply_roundtable(&template, Locale::En);
        let proposal = form.proposal().expect("three-seat roundtable is valid");

        assert_eq!(proposal.tasks.len(), 7);
        assert_eq!(
            proposal.tasks.last().unwrap().depends_on,
            ["seat_1_review", "seat_2_review", "seat_3_review"]
        );
    }

    #[test]
    fn exported_json_round_trips_through_import() {
        let mut form = DynamicWorkflowForm::default();
        form.goal = "Compare two analyses".into();
        form.tasks[0].instruction = "Interpret the input".into();
        form.tasks[0].max_tokens = "2048".into();
        let proposal = form.proposal().expect("valid workflow");
        let exported = serde_json::to_string_pretty(&proposal).expect("serializes");
        let imported: DynamicAgentWorkflowProposal =
            serde_json::from_str(&exported).expect("exported JSON parses back");
        let round_tripped = DynamicWorkflowForm::from_proposal(imported)
            .proposal()
            .expect("imported form stays valid");
        assert_eq!(round_tripped, proposal);
        assert!(serde_json::from_str::<DynamicAgentWorkflowProposal>("{\"goal\":1}").is_err());
    }

    #[test]
    fn workflow_groups_stay_scoped_to_the_parent_conversation() {
        let snapshot: AgentWorkflowSnapshot = serde_json::from_value(serde_json::json!({
            "workflow": {
                "id": "wf-1",
                "frame_id": "parent-frame",
                "root_workflow_id": "",
                "depth": 0,
                "name": "Workflow",
                "goal": "g",
                "mode": "manual",
                "status": "running",
                "max_parallel": 1,
                "requires_confirmation": true,
                "version": 1,
                "updated_at": 0
            },
            "delegation_enabled": true,
            "dynamic": {
                "schema_version": 1,
                "approval_policy": "review_all",
                "editable_proposal": {
                    "goal": "g",
                    "context": "",
                    "approval_policy": "review_all",
                    "tasks": []
                },
                "approval_reasons": [],
                "tasks": [{
                    "id": "task_1",
                    "stored_step_id": "step-1",
                    "instruction": "do",
                    "depends_on": [],
                    "capabilities": [],
                    "specialist_id": null,
                    "specialist_name": null,
                    "executor": {"kind": "native", "profile_id": null, "model_id": null},
                    "workspace_policy": "shared",
                    "merge_policy": "manual",
                    "tools": [],
                    "can_write": false,
                    "can_execute": false,
                    "can_access_network": false,
                    "budget": {
                        "max_tokens": null,
                        "max_tool_calls": null,
                        "max_cost_microunits": null
                    },
                    "timeout_secs": null,
                    "approval_reasons": [],
                    "output_schema": null,
                    "result": {
                        "status": "running",
                        "summary": null,
                        "error": null,
                        "child_frame_id": "agent-child",
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "tool_calls": 0,
                        "cost_microunits": 0,
                        "duration_secs": null,
                        "full_result_available": false
                    }
                }]
            }
        }))
        .expect("snapshot fixture deserializes");

        let parent = group_workflows(vec![snapshot.clone()], &[], Some("parent-frame"));
        assert_eq!(parent.len(), 1);
        assert!(group_workflows(vec![snapshot.clone()], &[], Some("agent-child")).is_empty());
        assert!(group_workflows(vec![snapshot], &[], Some("unrelated")).is_empty());
    }

    #[test]
    fn result_presentation_extracts_readable_content_from_the_runtime_envelope() {
        let response = serde_json::json!({
            "request_id": "request-1",
            "status": "succeeded",
            "output": {
                "task_id": "seat_1_opening",
                "summary": "Completed the opening position.",
                "files_changed": ["results/opening.md"],
                "diff_summary": "Created the report.",
                "artifacts": [{
                    "name": "opening.md",
                    "kind": "markdown",
                    "content": "# Opening position"
                }],
                "evidence": ["declared evidence"],
                "tests": ["Word limit checked"],
                "risks": ["Evidence remains uncertain"],
                "confidence": "medium"
            },
            "artifacts": [{
                "id": "declared:opening.md",
                "name": "opening.md",
                "kind": "markdown",
                "path": null
            }],
            "evidence": [{"kind": "agent", "summary": "persisted evidence"}],
            "child_frame_id": "internal-child-frame",
            "error": null
        });

        let result = AgentResultPresentation::from_response(&response);

        assert_eq!(
            result.summary.as_deref(),
            Some("Completed the opening position.")
        );
        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.evidence[0]["summary"], "persisted evidence");
        assert_eq!(result.tests, [serde_json::json!("Word limit checked")]);
        assert_eq!(
            result.details,
            [("confidence".into(), serde_json::json!("medium"))]
        );
        assert!(!result
            .details
            .iter()
            .any(|(key, _)| key == "task_id" || key == "child_frame_id"));
    }

    #[test]
    fn executor_selection_round_trips_an_optional_profile() {
        let executor = AgentExecutorSelection {
            kind: "acp".into(),
            profile_id: Some("remote-coder".into()),
        };
        assert_eq!(parse_executor_key(&executor_key(&executor)), Some(executor));
        assert_eq!(parse_executor_key(""), None);
    }

    #[test]
    fn explicit_budgets_must_be_positive() {
        assert!(parse_optional_u32("0", "token budget").is_err());
        assert!(parse_optional_u64("0", "cost budget").is_err());
        assert_eq!(parse_optional_u32("42", "token budget").unwrap(), Some(42));
    }

    #[test]
    fn task_budget_fields_accept_zero_as_unlimited() {
        assert_eq!(parse_budget_u32("0", "token budget").unwrap(), Some(0));
        assert_eq!(parse_budget_u64("0", "cost budget").unwrap(), Some(0));
        assert_eq!(parse_budget_u32("", "token budget").unwrap(), None);
        assert_eq!(parse_budget_u32("42", "token budget").unwrap(), Some(42));
        assert!(parse_budget_u32("nope", "token budget").is_err());
    }
}
