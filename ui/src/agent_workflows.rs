//! Dynamic Agent draft editor and persisted workflow activity surface.
//!
//! New v2 workflows are task/capability based. The legacy template editor is
//! retained only for revising already-persisted v1 drafts during migration.

use crate::bindings::invoke_checked;
use crate::dto::*;
use crate::i18n::{t, Locale};
use crate::text::{dom_value, event_target_checked, event_target_value, pretty_json};
use leptos::{ev, *};
use serde_wasm_bindgen::to_value;
use std::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DynamicTaskForm {
    key: u32,
    id: String,
    instruction: String,
    depends_on: Vec<String>,
    capabilities: Vec<String>,
    specialist_id: String,
    output_schema: String,
    isolated: bool,
    model_id: String,
    executor_key: String,
    max_tokens: String,
    max_tool_calls: String,
    max_cost_microunits: String,
}

impl DynamicTaskForm {
    fn blank(key: u32, id: String) -> Self {
        Self {
            key,
            id,
            instruction: String::new(),
            depends_on: vec![],
            capabilities: vec!["reasoning".into()],
            specialist_id: String::new(),
            output_schema: String::new(),
            isolated: false,
            model_id: String::new(),
            executor_key: String::new(),
            max_tokens: String::new(),
            max_tool_calls: String::new(),
            max_cost_microunits: String::new(),
        }
    }

    fn from_proposal(key: u32, task: DynamicAgentTaskProposal) -> Self {
        let budget = task.budget.unwrap_or_default();
        Self {
            key,
            id: task.id,
            instruction: task.instruction,
            depends_on: task.depends_on,
            capabilities: task.capabilities,
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
        }
    }

    fn proposal(&self) -> Result<DynamicAgentTaskProposal, String> {
        let output_schema = if self.output_schema.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&self.output_schema)
                    .map_err(|error| format!("Task {} output schema: {error}", self.id))?,
            )
        };
        let max_tokens = parse_optional_u32(&self.max_tokens, "token budget")?;
        let max_tool_calls = parse_optional_u32(&self.max_tool_calls, "tool-call budget")?;
        let max_cost_microunits = parse_optional_u64(&self.max_cost_microunits, "cost budget")?;
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
            capabilities: self.capabilities.clone(),
            specialist_id: nonempty(&self.specialist_id),
            output_schema,
            isolated: self.isolated,
            model_id: nonempty(&self.model_id),
            executor: parse_executor_key(&self.executor_key),
            budget,
        })
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
        if tasks.iter().any(|task| task.capabilities.is_empty()) {
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
                    && !task.capabilities.is_empty()
            })
    }

    fn add_task(&mut self) {
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
    }
}

#[derive(Clone, Copy)]
pub(super) struct AgentPanelState {
    pub(super) workflows: RwSignal<Vec<AgentWorkflowSnapshot>>,
    pub(super) templates: RwSignal<Vec<AgentTemplateSummary>>,
    pub(super) options: RwSignal<DynamicAgentEditorOptions>,
    pub(super) dynamic_form: RwSignal<DynamicWorkflowForm>,
    pub(super) dynamic_editing: RwSignal<Option<String>>,
    pub(super) legacy_goal: RwSignal<String>,
    pub(super) legacy_mode: RwSignal<String>,
    pub(super) legacy_selection: RwSignal<Vec<String>>,
    pub(super) legacy_editing: RwSignal<Option<String>>,
    pub(super) busy: RwSignal<bool>,
    pub(super) launching: RwSignal<Vec<String>>,
    pub(super) error: RwSignal<Option<String>>,
    pub(super) result: RwSignal<Option<AgentWorkflowResultDetail>>,
}

impl AgentPanelState {
    pub(super) fn new() -> Self {
        Self {
            workflows: create_rw_signal(vec![]),
            templates: create_rw_signal(vec![]),
            options: create_rw_signal(DynamicAgentEditorOptions::default()),
            dynamic_form: create_rw_signal(DynamicWorkflowForm::default()),
            dynamic_editing: create_rw_signal(None),
            legacy_goal: create_rw_signal(String::new()),
            legacy_mode: create_rw_signal("assisted".into()),
            legacy_selection: create_rw_signal(vec![]),
            legacy_editing: create_rw_signal(None),
            busy: create_rw_signal(false),
            launching: create_rw_signal(vec![]),
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
        if let Ok(value) = invoke_checked("list_agent_templates", JsValue::UNDEFINED).await {
            if let Ok(items) = serde_wasm_bindgen::from_value::<Vec<AgentTemplateSummary>>(value) {
                state.templates.set(items);
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

pub(super) fn refresh_agent_workflows(
    workflows: RwSignal<Vec<AgentWorkflowSnapshot>>,
    error: RwSignal<Option<String>>,
) {
    spawn_local(async move {
        match invoke_checked("list_agent_workflows", JsValue::UNDEFINED).await {
            Ok(value) => {
                match serde_wasm_bindgen::from_value::<Vec<AgentWorkflowSnapshot>>(value) {
                    Ok(items) => {
                        workflows.set(items);
                        error.set(None);
                    }
                    Err(parse_error) => error.set(Some(parse_error.to_string())),
                }
            }
            Err(invoke_error) => error.set(Some(js_error_text(invoke_error))),
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

fn dynamic_command_error(error: JsValue) -> (String, bool) {
    serde_wasm_bindgen::from_value::<DynamicWorkflowCommandError>(error.clone())
        .map(|error| (error.message, error.code == "version_conflict"))
        .unwrap_or_else(|_| (js_error_text(error), false))
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
) -> Vec<AgentWorkflowGroup> {
    let titles = sessions
        .iter()
        .map(|session| (session.id.as_str(), session.title.as_str()))
        .collect::<HashMap<_, _>>();
    let mut groups = Vec::<AgentWorkflowGroup>::new();
    for snapshot in snapshots {
        let frame_id = snapshot
            .workflow
            .frame_id
            .clone()
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
    groups
}

fn status_label(locale: Locale, status: &str) -> String {
    let key = match status {
        "draft" => "agents.status.draft",
        "approved" => "agents.status.approved",
        "running" => "agents.status.running",
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

fn task_value(
    form: RwSignal<DynamicWorkflowForm>,
    key: u32,
    get: impl FnOnce(&DynamicTaskForm) -> String,
) -> String {
    form.with(|form| {
        form.tasks
            .iter()
            .find(|task| task.key == key)
            .map(get)
            .unwrap_or_default()
    })
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
                        form.tasks.retain(|task| task.key != remove_key);
                        let ids = form.tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
                        for task in &mut form.tasks {
                            task.depends_on.retain(|dependency| ids.contains(dependency));
                        }
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
            </div>
            <fieldset class="dynamic-agent-choice-group">
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
            <fieldset class="dynamic-agent-choice-group">
                <legend>{move || t(locale.get(), "agents.task.dependencies")}</legend>
                <div class="dynamic-agent-checks dynamic-dependency-checks">
                    {move || {
                        let choices = state.dynamic_form.with(|form| {
                            form.tasks.iter().filter(|task| task.key != key)
                                .map(|task| task.id.clone()).filter(|id| !id.trim().is_empty())
                                .collect::<Vec<_>>()
                        });
                        if choices.is_empty() {
                            view! { <span class="dynamic-agent-none">{t(locale.get(), "agents.task.no_dependencies")}</span> }.into_view()
                        } else {
                            choices.into_iter().map(|dependency| {
                                let checked_dependency = dependency.clone();
                                let update_dependency = dependency.clone();
                                view! {
                                    <label class="dynamic-agent-check dependency">
                                        <input type="checkbox"
                                            prop:checked=move || state.dynamic_form.with(|form| {
                                                form.tasks.iter().find(|task| task.key == key)
                                                    .is_some_and(|task| task.depends_on.contains(&checked_dependency))
                                            })
                                            on:change=move |event| {
                                                let checked = event_target_checked(&event);
                                                update_task(state.dynamic_form, key, |task| {
                                                    if checked {
                                                        if !task.depends_on.contains(&update_dependency) {
                                                            task.depends_on.push(update_dependency.clone());
                                                        }
                                                    } else {
                                                        task.depends_on.retain(|id| id != &update_dependency);
                                                    }
                                                });
                                            } />
                                        <span>{dependency}</span>
                                    </label>
                                }
                            }).collect_view()
                        }
                    }}
                </div>
            </fieldset>
            <label>
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
            <details class="dynamic-agent-advanced">
                <summary>{move || t(locale.get(), "agents.task.advanced")}</summary>
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
                        <input type="number" min="1" inputmode="numeric"
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_tokens.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_tokens = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.max_tools")}</span>
                        <input type="number" min="1" inputmode="numeric"
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_tool_calls.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_tool_calls = event_target_value(&event);
                            }) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "agents.task.max_cost")}</span>
                        <input type="number" min="1" inputmode="numeric"
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.max_cost_microunits.clone())
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.max_cost_microunits = event_target_value(&event);
                            }) />
                    </label>
                    <label class="dynamic-task-schema">
                        <span>{move || t(locale.get(), "agents.task.output_schema")}</span>
                        <textarea spellcheck="false"
                            prop:value=move || task_value(state.dynamic_form, key, |task| task.output_schema.clone())
                            prop:placeholder="{\"type\":\"object\"}"
                            on:input=move |event| update_task(state.dynamic_form, key, |task| {
                                task.output_schema = event_target_value(&event);
                            })></textarea>
                    </label>
                </div>
            </details>
        </fieldset>
    }
}

fn dynamic_editor(
    state: AgentPanelState,
    delegation_enabled: RwSignal<bool>,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let submit = move |event: ev::SubmitEvent| {
        event.prevent_default();
        if !delegation_enabled.get_untracked() || state.busy.get_untracked() {
            return;
        }
        let proposal = match state.dynamic_form.get_untracked().proposal() {
            Ok(proposal) => proposal,
            Err(error) => {
                state.error.set(Some(error));
                return;
            }
        };
        let editing = state.dynamic_editing.get_untracked();
        let expected_version = editing.as_ref().and_then(|workflow_id| {
            state.workflows.with_untracked(|workflows| {
                workflows
                    .iter()
                    .find(|snapshot| &snapshot.workflow.id == workflow_id)
                    .map(|snapshot| snapshot.workflow.version)
            })
        });
        if editing.is_some() && expected_version.is_none() {
            state.error.set(Some(
                "The draft is no longer available; refresh and try again.".into(),
            ));
            return;
        }
        state.busy.set(true);
        spawn_local(async move {
            let (command, args) = match (editing, expected_version) {
                (Some(workflow_id), Some(expected_version)) => (
                    "revise_dynamic_agent_workflow",
                    serde_json::json!({
                        "workflowId": workflow_id,
                        "proposal": proposal,
                        "expectedVersion": expected_version,
                    }),
                ),
                _ => (
                    "create_dynamic_agent_workflow",
                    serde_json::json!({ "proposal": proposal }),
                ),
            };
            match invoke_checked(command, to_value(&args).unwrap()).await {
                Ok(_) => {
                    state.dynamic_form.set(DynamicWorkflowForm::default());
                    state.dynamic_editing.set(None);
                    state.error.set(None);
                    refresh_agent_workflows(state.workflows, state.error);
                }
                Err(error) => {
                    let (message, conflict) = dynamic_command_error(error);
                    if conflict {
                        if let Ok(value) =
                            invoke_checked("list_agent_workflows", JsValue::UNDEFINED).await
                        {
                            if let Ok(items) =
                                serde_wasm_bindgen::from_value::<Vec<AgentWorkflowSnapshot>>(value)
                            {
                                state.workflows.set(items);
                            }
                        }
                    }
                    state.error.set(Some(message));
                }
            }
            state.busy.set(false);
        });
    };

    view! {
        <form class="agents-create dynamic-agent-editor" data-testid="dynamic-agent-editor" on:submit=submit>
            <div class="dynamic-agent-editor-head">
                <div>
                    <strong>{move || if state.dynamic_editing.get().is_some() {
                        t(locale.get(), "agents.editor.edit")
                    } else {
                        t(locale.get(), "agents.editor.new")
                    }}</strong>
                    <p>{move || t(locale.get(), "agents.editor.help")}</p>
                </div>
                <button type="button" class="icon-btn" title=move || t(locale.get(), "agents.refresh")
                    aria-label=move || t(locale.get(), "agents.refresh")
                    on:click=move |_| {
                        refresh_agent_resources(state, specialists);
                        refresh_agent_workflows(state.workflows, state.error);
                    }>{"↻"}</button>
            </div>
            <label for="dynamic-agent-goal">{move || t(locale.get(), "agents.goal")}</label>
            <textarea id="dynamic-agent-goal" data-testid="agent-goal"
                prop:value=move || state.dynamic_form.get().goal
                prop:placeholder=move || t(locale.get(), "agents.goal_ph")
                disabled=move || !delegation_enabled.get()
                on:input=move |event| state.dynamic_form.update(|form| {
                    form.goal = event_target_value(&event);
                })></textarea>
            <div class="dynamic-agent-policy-row">
                <label>
                    <span>{move || t(locale.get(), "agents.approval_policy")}</span>
                    <select data-testid="agent-approval-policy"
                        disabled=move || !delegation_enabled.get()
                        on:change=move |event| state.dynamic_form.update(|form| {
                            form.approval_policy = if dom_value(&event) == "auto_safe" {
                                AgentApprovalPolicy::AutoSafe
                            } else {
                                AgentApprovalPolicy::ReviewAll
                            };
                        })>
                        <option value="review_all" prop:selected=move || state.dynamic_form.get().approval_policy == AgentApprovalPolicy::ReviewAll>
                            {move || t(locale.get(), "agents.approval.review_all")}
                        </option>
                        <option value="auto_safe" prop:selected=move || state.dynamic_form.get().approval_policy == AgentApprovalPolicy::AutoSafe>
                            {move || t(locale.get(), "agents.approval.auto_safe")}
                        </option>
                    </select>
                </label>
                <span>{move || if state.dynamic_form.get().approval_policy == AgentApprovalPolicy::AutoSafe {
                    t(locale.get(), "agents.approval.auto_safe_help")
                } else {
                    t(locale.get(), "agents.approval.review_all_help")
                }}</span>
            </div>
            <details class="dynamic-agent-context">
                <summary>{move || t(locale.get(), "agents.shared_context")}</summary>
                <textarea prop:value=move || state.dynamic_form.get().context
                    prop:placeholder=move || t(locale.get(), "agents.shared_context_ph")
                    on:input=move |event| state.dynamic_form.update(|form| {
                        form.context = event_target_value(&event);
                    })></textarea>
            </details>
            <div class="dynamic-agent-task-list">
                <For each=move || state.dynamic_form.get().tasks
                    key=|task| task.key
                    children=move |task| dynamic_task_editor(task, state, specialists, models, locale)
                />
            </div>
            <button type="button" class="agents-secondary dynamic-task-add" data-testid="dynamic-add-task"
                disabled=move || !delegation_enabled.get()
                on:click=move |_| state.dynamic_form.update(DynamicWorkflowForm::add_task)>
                {move || format!("+ {}", t(locale.get(), "agents.task.add"))}
            </button>
            <div class="agents-create-actions">
                <button type="button" class="agents-secondary"
                    on:click=move |_| {
                        state.dynamic_form.set(DynamicWorkflowForm::default());
                        state.dynamic_editing.set(None);
                        state.error.set(None);
                    }>{move || if state.dynamic_editing.get().is_some() {
                        t(locale.get(), "agents.edit.cancel")
                    } else {
                        t(locale.get(), "agents.editor.reset")
                    }}</button>
                <button type="submit" class="agents-primary" data-testid="agent-create"
                    disabled=move || !delegation_enabled.get() || state.busy.get() || !state.dynamic_form.get().ready()>
                    {move || if state.busy.get() {
                        t(locale.get(), "agents.saving")
                    } else if state.dynamic_editing.get().is_some() {
                        t(locale.get(), "agents.save_changes")
                    } else {
                        t(locale.get(), "agents.create_dynamic")
                    }}
                </button>
            </div>
        </form>
    }
}

fn legacy_editor(
    state: AgentPanelState,
    delegation_enabled: RwSignal<bool>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let submit = move |event: ev::SubmitEvent| {
        event.prevent_default();
        if !delegation_enabled.get_untracked()
            || state.busy.get_untracked()
            || state.legacy_goal.get_untracked().trim().is_empty()
            || (state.legacy_mode.get_untracked() == "manual"
                && state.legacy_selection.get_untracked().is_empty())
        {
            return;
        }
        let Some(workflow_id) = state.legacy_editing.get_untracked() else {
            return;
        };
        let Some(expected_version) = state.workflows.with_untracked(|workflows| {
            workflows
                .iter()
                .find(|snapshot| snapshot.workflow.id == workflow_id)
                .map(|snapshot| snapshot.workflow.version)
        }) else {
            state
                .error
                .set(Some("The legacy draft is no longer available.".into()));
            return;
        };
        let goal = state.legacy_goal.get_untracked().trim().to_string();
        let mode = state.legacy_mode.get_untracked();
        let template_ids = if mode == "manual" {
            state.legacy_selection.get_untracked()
        } else {
            vec![]
        };
        state.busy.set(true);
        spawn_local(async move {
            let args = serde_json::json!({
                "workflowId": workflow_id,
                "goal": goal,
                "mode": mode,
                "templateIds": template_ids,
                "expectedVersion": expected_version,
            });
            match invoke_checked("revise_agent_workflow", to_value(&args).unwrap()).await {
                Ok(_) => {
                    state.legacy_editing.set(None);
                    state.legacy_goal.set(String::new());
                    state.legacy_selection.set(vec![]);
                    refresh_agent_workflows(state.workflows, state.error);
                }
                Err(error) => state.error.set(Some(js_error_text(error))),
            }
            state.busy.set(false);
        });
    };

    view! {
        <form class="agents-create legacy-agent-editor" data-testid="legacy-agent-editor" on:submit=submit>
            <div class="dynamic-agent-editor-head">
                <div>
                    <strong>{move || t(locale.get(), "agents.legacy.edit")}</strong>
                    <p>{move || t(locale.get(), "agents.legacy.edit_help")}</p>
                </div>
                <span class="agent-kind-badge legacy">{move || t(locale.get(), "agents.legacy")}</span>
            </div>
            <label for="legacy-agent-goal">{move || t(locale.get(), "agents.goal")}</label>
            <textarea id="legacy-agent-goal" data-testid="legacy-agent-goal"
                prop:value=move || state.legacy_goal.get()
                on:input=move |event| state.legacy_goal.set(event_target_value(&event))></textarea>
            <div class="agents-mode-row">
                <select data-testid="legacy-agent-mode"
                    on:change=move |event| state.legacy_mode.set(dom_value(&event))>
                    <option value="manual" prop:selected=move || state.legacy_mode.get() == "manual">{move || t(locale.get(), "agents.mode.manual")}</option>
                    <option value="assisted" prop:selected=move || state.legacy_mode.get() == "assisted">{move || t(locale.get(), "agents.mode.assisted")}</option>
                    <option value="automatic" prop:selected=move || state.legacy_mode.get() == "automatic">{move || t(locale.get(), "agents.mode.automatic")}</option>
                </select>
            </div>
            {move || (state.legacy_mode.get() == "manual").then(|| {
                let templates = state.templates.get();
                view! {
                    <div class="agents-manual-builder">
                        <div class="agents-manual-title">{t(locale.get(), "agents.manual.title")}</div>
                        {templates.into_iter().map(|template| {
                            let id = template.id.clone();
                            let checked_id = id.clone();
                            view! {
                                <label class="dynamic-agent-check">
                                    <input type="checkbox"
                                        prop:checked=move || state.legacy_selection.get().contains(&checked_id)
                                        on:change=move |event| {
                                            let checked = event_target_checked(&event);
                                            state.legacy_selection.update(|ids| {
                                                if checked && !ids.contains(&id) {
                                                    ids.push(id.clone());
                                                } else if !checked {
                                                    ids.retain(|value| value != &id);
                                                }
                                            });
                                        } />
                                    <span>{template.display_name}</span>
                                </label>
                            }
                        }).collect_view()}
                    </div>
                }
            })}
            <div class="agents-create-actions">
                <button type="button" class="agents-secondary" on:click=move |_| {
                    state.legacy_editing.set(None);
                    state.legacy_goal.set(String::new());
                    state.legacy_selection.set(vec![]);
                }>{move || t(locale.get(), "agents.edit.cancel")}</button>
                <button type="submit" class="agents-primary"
                    disabled=move || state.busy.get() || state.legacy_goal.get().trim().is_empty()
                        || (state.legacy_mode.get() == "manual" && state.legacy_selection.get().is_empty())>
                    {move || if state.busy.get() { t(locale.get(), "agents.saving") } else { t(locale.get(), "agents.save_changes") }}
                </button>
            </div>
        </form>
    }
}

fn invoke_workflow_action(command: &'static str, args: serde_json::Value, state: AgentPanelState) {
    spawn_local(async move {
        match invoke_checked(command, to_value(&args).unwrap()).await {
            Ok(_) => refresh_agent_workflows(state.workflows, state.error),
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
            Ok(_) => refresh_agent_workflows(state.workflows, state.error),
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
    let workflow = snapshot.workflow.clone();
    let workflow_id = workflow.id.clone();
    let approve_id = workflow_id.clone();
    let discard_id = workflow_id.clone();
    let run_id = workflow_id.clone();
    let run_busy_id = workflow_id.clone();
    let cancel_id = workflow_id.clone();
    let retry_id = workflow_id.clone();
    let edit_id = workflow_id.clone();
    let delegation_enabled = snapshot.delegation_enabled;
    let dynamic_proposal = snapshot
        .dynamic
        .as_ref()
        .map(|dynamic| dynamic.editable_proposal.clone());
    let legacy_templates = snapshot
        .steps
        .iter()
        .map(|step| step.template_id.clone())
        .collect::<Vec<_>>();
    let edit_goal = workflow.goal.clone();
    let edit_mode = workflow.mode.clone();
    let automatic = snapshot.approval_policy == AgentApprovalPolicy::AutoSafe;
    view! {
        <div class="agent-workflow-actions">
            {(workflow.status == "draft").then(|| {
                let proposal = dynamic_proposal.clone();
                let templates = legacy_templates.clone();
                view! {
                    <button type="button" class="agents-secondary" data-testid="agent-edit"
                        disabled=!delegation_enabled on:click=move |_| {
                            if let Some(proposal) = proposal.clone() {
                                state.dynamic_form.set(DynamicWorkflowForm::from_proposal(proposal));
                                state.dynamic_editing.set(Some(edit_id.clone()));
                                state.legacy_editing.set(None);
                            } else {
                                state.legacy_goal.set(edit_goal.clone());
                                state.legacy_mode.set(edit_mode.clone());
                                state.legacy_selection.set(templates.clone());
                                state.legacy_editing.set(Some(edit_id.clone()));
                                state.dynamic_editing.set(None);
                            }
                        }>{t(locale.get(), "agents.edit")}</button>
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
                    on:click=move |_| invoke_workflow_action(
                        "retry_agent_workflow",
                        serde_json::json!({ "workflowId": retry_id }),
                        state,
                    )>{t(locale.get(), "agents.retry")}</button>
            })}
        </div>
    }
    .into_view()
}

fn dynamic_workflow_card(
    snapshot: AgentWorkflowSnapshot,
    state: AgentPanelState,
    locale: RwSignal<Locale>,
    load_session: Callback<String>,
    refresh_sessions: Callback<()>,
) -> View {
    let workflow = snapshot.workflow.clone();
    let workflow_id = workflow.id.clone();
    let status = workflow.status.clone();
    let status_class = format!("agent-workflow-status {status}");
    let dynamic = snapshot.dynamic.clone().expect("v2 summary");
    let policy_label = match dynamic.approval_policy {
        AgentApprovalPolicy::ReviewAll => t(locale.get(), "agents.approval.review_all"),
        AgentApprovalPolicy::AutoSafe => t(locale.get(), "agents.approval.auto_safe"),
    };
    let actions = workflow_actions(&snapshot, state, locale);
    let workflow_delegation_enabled = snapshot.delegation_enabled;
    view! {
        <article class="agent-workflow-card dynamic" data-workflow-id=workflow_id.clone() data-schema-version="2">
            <div class="agent-workflow-head">
                <div>
                    <div class="agent-workflow-name">{workflow.name.clone()}</div>
                    <div class="agent-workflow-meta">
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
            {(!workflow_delegation_enabled).then(|| view! {
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
                    let task_status = result.as_ref().map(|result| result.status.as_str())
                        .unwrap_or("pending")
                        .to_string();
                    let attempt_class = format!("agent-attempt-status {task_status}");
                    let specialist = task.specialist_name.clone()
                        .unwrap_or_else(|| t(locale.get(), "agents.task.temporary").into());
                    let executor = task.executor.profile_id.as_ref()
                        .map(|profile| format!("{} · {profile}", task.executor.kind))
                        .unwrap_or_else(|| task.executor.kind.clone());
                    let model = task.executor.model_id.clone().unwrap_or_else(|| "—".into());
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
                    let child_frame = result.as_ref().and_then(|result| result.child_frame_id.clone());
                    let task_approval_reasons = task.approval_reasons.clone();
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
                                {full_result.then(|| view! {
                                    <button type="button" class="agents-secondary" data-testid="agent-inspect-result"
                                        on:click=move |_| open_workflow_result(
                                            result_workflow_id.clone(), result_step_id.clone(), state,
                                        )>{t(locale.get(), "agents.inspect_result")}</button>
                                })}
                                {child_frame.map(|frame_id| view! {
                                    <button type="button" class="agents-secondary agent-takeover"
                                        on:click=move |_| {
                                            load_session.call(frame_id.clone());
                                            refresh_sessions.call(());
                                        }>{t(locale.get(), "agents.takeover")}</button>
                                })}
                            </div>
                        </section>
                    }
                }).collect_view()}
            </div>
        </article>
    }.into_view()
}

fn legacy_attempt_summary(attempt: &AgentWorkflowAttempt) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&attempt.output_json)
        .ok()
        .and_then(|value| value.get("summary")?.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty())
}

fn legacy_step_limits(step: &AgentWorkflowStep) -> String {
    let permissions =
        serde_json::from_str::<serde_json::Value>(&step.permissions_json).unwrap_or_default();
    let budget = serde_json::from_str::<serde_json::Value>(&step.budget_json).unwrap_or_default();
    let tools = permissions
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "no tools".into());
    let token_limit = budget
        .get("max_tokens")
        .and_then(serde_json::Value::as_u64)
        .map(|value| format!("{value} tokens"))
        .unwrap_or_else(|| "token budget unavailable".into());
    let timeout = step
        .timeout_secs
        .map(|value| format!("{value}s"))
        .unwrap_or_else(|| "no timeout".into());
    format!("{tools} · {token_limit} · {timeout}")
}

fn legacy_workflow_card(
    snapshot: AgentWorkflowSnapshot,
    state: AgentPanelState,
    locale: RwSignal<Locale>,
    load_session: Callback<String>,
    refresh_sessions: Callback<()>,
) -> View {
    let workflow = snapshot.workflow.clone();
    let status = workflow.status.clone();
    let status_class = format!("agent-workflow-status {status}");
    let latest_attempts = snapshot.attempts.clone();
    let actions = workflow_actions(&snapshot, state, locale);
    let workflow_delegation_enabled = snapshot.delegation_enabled;
    view! {
        <article class="agent-workflow-card legacy" data-workflow-id=workflow.id.clone() data-schema-version="1">
            <div class="agent-workflow-head">
                <div>
                    <div class="agent-workflow-name">{workflow.name.clone()}</div>
                    <div class="agent-workflow-meta">
                        <span class="agent-kind-badge legacy">{t(locale.get(), "agents.legacy")}</span>
                        {format!(" · {} · max {}", workflow.mode, workflow.max_parallel)}
                    </div>
                </div>
                <span class=status_class>{status_label(locale.get(), &status)}</span>
            </div>
            <p class="agent-workflow-goal">{workflow.goal}</p>
            {workflow.requires_confirmation.then(|| view! {
                <div class="agent-confirm-hint">{t(locale.get(), "agents.confirm_hint")}</div>
            })}
            {(!workflow_delegation_enabled).then(|| view! {
                <div class="agent-delegation-off">{t(locale.get(), "agents.workflow_disabled")}</div>
            })}
            {actions}
            <div class="agent-step-list legacy">
                {snapshot.steps.into_iter().map(|step| {
                    let attempt = latest_attempts.iter().rev()
                        .find(|attempt| attempt.step_id == step.id).cloned();
                    let attempt_status = attempt.as_ref().map(|attempt| attempt.status.clone())
                        .unwrap_or_else(|| "pending".into());
                    let attempt_class = format!("agent-attempt-status {attempt_status}");
                    let summary = attempt.as_ref().and_then(legacy_attempt_summary);
                    let error = attempt.as_ref().and_then(|attempt| attempt.error.clone());
                    let child_frame = attempt.as_ref().and_then(|attempt| attempt.child_frame_id.clone());
                    let usage = attempt.as_ref().map(|attempt| format!(
                        "{} tokens · {} tools · {:.4}",
                        attempt.input_tokens.saturating_add(attempt.output_tokens),
                        attempt.tool_calls,
                        attempt.cost_microunits as f64 / 1_000_000.0,
                    ));
                    view! {
                        <section class="agent-step legacy" data-step-id=step.id.clone()>
                            <div class="agent-step-head">
                                <span class="agent-step-name">{step.display_name()}</span>
                                <span class=attempt_class>{status_label(locale.get(), &attempt_status)}</span>
                            </div>
                            <div class="agent-step-meta">{format!(
                                "{} · {}{}",
                                step.role,
                                step.backend,
                                step.model.as_ref().map(|model| format!(" · {model}")).unwrap_or_default(),
                            )}</div>
                            <div class="agent-step-limits">{legacy_step_limits(&step)}</div>
                            {summary.map(|summary| view! { <p class="agent-attempt-summary">{summary}</p> })}
                            {error.map(|error| view! { <div class="agents-error">{error}</div> })}
                            {usage.map(|usage| view! { <div class="agent-usage">{usage}</div> })}
                            {child_frame.map(|frame_id| view! {
                                <button type="button" class="agents-secondary agent-takeover"
                                    on:click=move |_| {
                                        load_session.call(frame_id.clone());
                                        refresh_sessions.call(());
                                    }>{t(locale.get(), "agents.takeover")}</button>
                            })}
                        </section>
                    }
                }).collect_view()}
            </div>
        </article>
    }.into_view()
}

fn workflow_result_dialog(state: AgentPanelState, locale: RwSignal<Locale>) -> View {
    view! {
        {move || state.result.get().map(|result| {
            let title = format!(
                "{} · {} · #{}",
                result.step_id,
                status_label(locale.get(), &result.status),
                result.attempt,
            );
            let response = serde_json::to_string_pretty(&result.response)
                .unwrap_or_else(|_| pretty_json(&result.response.to_string()));
            view! {
                <div class="overlay agent-result-overlay" role="presentation"
                    on:click=move |_| state.result.set(None)>
                    <div class="modal agent-result-modal" role="dialog" aria-modal="true"
                        aria-labelledby="agent-result-title"
                        tabindex="-1"
                        on:keydown:undelegated=move |event| {
                            if event.key() == "Escape" {
                                event.prevent_default();
                                event.stop_propagation();
                                state.result.set(None);
                            }
                        }
                        on:click=|event| event.stop_propagation()>
                        <div class="agent-result-head">
                            <div>
                                <h2 id="agent-result-title">{t(locale.get(), "agents.result.title")}</h2>
                                <span>{title}</span>
                            </div>
                            <button type="button" class="agents-secondary"
                                id="agent-result-close"
                                autofocus=true
                                aria-label=t(locale.get(), "agents.result.close")
                                on:click=move |_| state.result.set(None)>{"×"}</button>
                        </div>
                        <pre data-testid="agent-result-json">{response}</pre>
                    </div>
                </div>
            }
        })}
    }
    .into_view()
}

pub(super) fn agent_workflows_panel(
    state: AgentPanelState,
    specialists: RwSignal<Vec<Specialist>>,
    models: RwSignal<Vec<ModelProfile>>,
    sessions: RwSignal<Vec<SessionInfo>>,
    delegation_enabled: RwSignal<bool>,
    locale: RwSignal<Locale>,
    load_session: Callback<String>,
    refresh_sessions: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="agents-pane dynamic-agents-panel" data-testid="agent-workflows" data-panel-version="2">
            <div class="agents-inline-notice">
                <strong>{move || t(locale.get(), "agents.inline_notice_title")}</strong>
                <span>{move || t(locale.get(), "agents.inline_notice")}</span>
            </div>
            {move || (!delegation_enabled.get()).then(|| view! {
                <div class="agents-disabled">{t(locale.get(), "agents.disabled")}</div>
            })}
            {move || if state.legacy_editing.get().is_some() {
                legacy_editor(state, delegation_enabled, locale).into_view()
            } else {
                dynamic_editor(state, delegation_enabled, specialists, models, locale).into_view()
            }}
            {move || state.error.get().map(|message| view! {
                <div class="agents-error" role="alert">{message}</div>
            })}
            <div class="agent-workflow-groups" aria-live="polite">
                {move || {
                    let groups = group_workflows(state.workflows.get(), &sessions.get());
                    if groups.is_empty() {
                        view! {
                            <div class="rp-empty"><p>{t(locale.get(), "agents.empty")}</p></div>
                        }.into_view()
                    } else {
                        groups.into_iter().map(|group| {
                            let frame_id = group.frame_id.clone();
                            let conversation_id = frame_id.clone();
                            view! {
                                <section class="agent-workflow-group" data-frame-id=frame_id>
                                    <button type="button" class="agent-workflow-group-head"
                                        on:click=move |_| load_session.call(conversation_id.clone())>
                                        <span>{t(locale.get(), "agents.conversation")}</span>
                                        <strong>{group.title}</strong>
                                        <small>{format!(
                                            "{} {}",
                                            group.snapshots.len(),
                                            t(locale.get(), "agents.workflow_count"),
                                        )}</small>
                                    </button>
                                    <div class="agent-workflow-group-list">
                                        {group.snapshots.into_iter().map(|snapshot| {
                                            if snapshot.plan_schema_version >= 2 && snapshot.dynamic.is_some() {
                                                dynamic_workflow_card(
                                                    snapshot,
                                                    state,
                                                    locale,
                                                    load_session,
                                                    refresh_sessions,
                                                )
                                            } else {
                                                legacy_workflow_card(
                                                    snapshot,
                                                    state,
                                                    locale,
                                                    load_session,
                                                    refresh_sessions,
                                                )
                                            }
                                        }).collect_view()}
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
    fn arbitrary_tasks_round_trip_without_a_template() {
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
        form.tasks[1].output_schema = r#"{"type":"object"}"#.into();

        let proposal = form.proposal().expect("valid arbitrary workflow");
        assert_eq!(proposal.tasks.len(), 2);
        assert_eq!(proposal.tasks[1].depends_on, ["task_1"]);
        assert_eq!(proposal.tasks[1].executor.as_ref().unwrap().kind, "native");
        assert_eq!(
            proposal.tasks[1].budget.as_ref().unwrap().max_tokens,
            Some(2048)
        );
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
    fn executor_selection_round_trips_an_optional_profile() {
        let executor = AgentExecutorSelection {
            kind: "acp".into(),
            profile_id: Some("remote-coder".into()),
        };
        assert_eq!(
            parse_executor_key(&executor_key(&executor)),
            Some(executor)
        );
        assert_eq!(parse_executor_key(""), None);
    }

    #[test]
    fn explicit_budgets_must_be_positive() {
        assert!(parse_optional_u32("0", "token budget").is_err());
        assert!(parse_optional_u64("0", "cost budget").is_err());
        assert_eq!(parse_optional_u32("42", "token budget").unwrap(), Some(42));
    }
}
