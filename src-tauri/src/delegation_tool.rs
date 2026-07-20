//! Parent-facing tools for dynamic temporary-Agent delegation.

use crate::{
    delegation_runtime,
    dynamic_workflow::{
        self, AgentApprovalPolicy, DynamicAgentTaskProposal, DynamicAgentWorkflowProposal,
        MAX_CONTEXT_CHARS, MAX_GOAL_CHARS, MAX_INSTRUCTION_CHARS,
    },
    run_context::RunManager,
    specialists, ActiveProject,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use wisp_core::{
    AgentDelegator, AgentSpec, CapabilityRegistry, DelegationExecutionResult, DelegationHostPolicy,
    DelegationPlan, MAX_DELEGATION_TASKS,
};
use wisp_llm::ToolSchema;
use wisp_store::{AgentDelegationRootLimits, AgentWorkflowAttempt, Store};
use wisp_tools::{ConfirmDecision, Tool, ToolEnv, ToolResult};

const INLINE_DATA_BYTES: usize = 4_000;
const INLINE_TEXT_CHARS: usize = 1_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateTasksArgs {
    goal: String,
    #[serde(default)]
    context: String,
    tasks: Vec<DelegateTaskInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateTaskInput {
    id: String,
    instruction: String,
    #[serde(default)]
    depends_on: Vec<String>,
    capabilities: Vec<String>,
    #[serde(default)]
    specialist_id: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    isolated: bool,
}

#[derive(Debug, Clone)]
struct NestedDelegationContext {
    attempt: AgentWorkflowAttempt,
    parent_spec: AgentSpec,
    limits: AgentDelegationRootLimits,
    parent_display_id: String,
}

async fn nested_delegation_context(
    store: &Store,
    frame_id: &str,
) -> Result<Option<NestedDelegationContext>, String> {
    let Some(context) = nested_delegation_access_context(store, frame_id).await? else {
        return Ok(None);
    };
    if !store
        .agent_workflow_attempt_has_delegation_capacity(&context.attempt.id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    Ok(Some(context))
}

async fn nested_delegation_access_context(
    store: &Store,
    frame_id: &str,
) -> Result<Option<NestedDelegationContext>, String> {
    let Some(attempt) = store
        .running_agent_workflow_attempt_for_child_frame(frame_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if !attempt.allow_delegation {
        return Ok(None);
    }
    let step = store
        .get_agent_workflow_step(&attempt.step_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Delegating Agent step no longer exists.".to_string())?;
    let parent_spec: AgentSpec = serde_json::from_str(&step.spec_json)
        .map_err(|error| format!("Delegating Agent policy snapshot is invalid: {error}"))?;
    if !parent_spec.allow_delegation {
        return Ok(None);
    }
    let root = store
        .get_agent_workflow(&attempt.root_workflow_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Root Agent workflow no longer exists.".to_string())?;
    let limits: AgentDelegationRootLimits = serde_json::from_str(&root.root_limits_json)
        .map_err(|error| format!("Root Agent limits are invalid: {error}"))?;
    let parent_workflow = store
        .get_agent_workflow(&attempt.workflow_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Delegating Agent workflow no longer exists.".to_string())?;
    let parent_display_id = serde_json::from_str::<DelegationPlan>(&parent_workflow.plan_json)
        .ok()
        .and_then(|plan| {
            plan.steps
                .into_iter()
                .find(|step| step.id == attempt.step_id)
                .and_then(|step| {
                    step.input
                        .get("task_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        })
        .unwrap_or_else(|| attempt.step_id.clone());
    Ok(Some(NestedDelegationContext {
        attempt,
        parent_spec,
        limits,
        parent_display_id,
    }))
}

pub(crate) async fn nested_delegation_available(store: &Store, frame_id: &str) -> bool {
    nested_delegation_context(store, frame_id)
        .await
        .ok()
        .flatten()
        .is_some()
}

pub(crate) async fn nested_result_access_available(store: &Store, frame_id: &str) -> bool {
    nested_delegation_access_context(store, frame_id)
        .await
        .ok()
        .flatten()
        .is_some()
}

async fn delegation_policy_for_frame(
    store: &Store,
    project: &ActiveProject,
    frame_id: &str,
    app_data: &std::path::Path,
) -> Result<
    (
        delegation_runtime::ProjectDelegationPolicy,
        Option<NestedDelegationContext>,
    ),
    String,
> {
    let mut policy = delegation_runtime::dynamic_delegation_policy_for_project(
        store,
        project,
        Some(frame_id),
        app_data,
    )
    .await?;
    let nested = nested_delegation_context(store, frame_id).await?;
    if let Some(context) = &nested {
        (policy.registry, policy.host) = delegation_runtime::nested_delegation_policy(
            &policy.registry,
            &policy.host,
            &context.parent_spec,
            u8::try_from(context.attempt.depth)
                .map_err(|_| "Delegating Agent depth is invalid.".to_string())?,
            &context.limits,
        )?;
    }
    Ok((policy, nested))
}

pub(crate) async fn delegate_tasks_schema(
    store: &Store,
    project: &ActiveProject,
    frame_id: &str,
    app_data: &std::path::Path,
) -> Result<ToolSchema, String> {
    let (policy, nested) = delegation_policy_for_frame(store, project, frame_id, app_data).await?;
    let completion = if nested.is_some() {
        crate::delegation_completion::AgentCompletionSettings::default()
    } else {
        crate::delegation_completion::session_completion_settings(store, frame_id).await
    };
    let specialists = specialists::ensure(store).await;
    Ok(build_delegate_tasks_schema(
        &policy.registry,
        &policy.host,
        &specialists,
        completion,
    ))
}

fn build_delegate_tasks_schema(
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
    specialists: &[specialists::Specialist],
    completion: crate::delegation_completion::AgentCompletionSettings,
) -> ToolSchema {
    let capabilities = registry.available_ids(host);
    let capability_help = capabilities
        .iter()
        .filter_map(|id| {
            registry
                .get(id)
                .map(|definition| format!("{id}: {}", definition.description))
        })
        .collect::<Vec<_>>()
        .join("; ");
    let specialist_ids = specialists
        .iter()
        .map(|specialist| specialist.id.clone())
        .collect::<Vec<_>>();
    let specialist_help = specialists
        .iter()
        .map(|specialist| {
            let description = specialist.description.trim();
            if description.is_empty() {
                format!("{}: {}", specialist.id, specialist.name)
            } else {
                format!("{}: {} — {description}", specialist.id, specialist.name)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    let description = match completion.policy {
        crate::delegation_completion::AgentCompletionPolicy::Inline => "Run a bounded batch of temporary Wisp sub-Agents and return their results to this turn. Decompose the work yourself; independent tasks run in parallel, dependencies run after their prerequisites, and you must synthesize the returned evidence into your final answer. Use the smallest useful batch. Do not delegate trivial work. Nested delegation is available only when the advertised capability list explicitly includes delegation.",
        crate::delegation_completion::AgentCompletionPolicy::Background => "Start a bounded batch of temporary Wisp sub-Agents in the background and return its durable handle immediately. Independent tasks run in parallel and dependencies wait for prerequisites. Do not wait or poll: completion will be appended to this conversation, and the parent may auto-resume when enabled. Use the smallest useful batch. Do not delegate trivial work. Nested delegation is available only when the advertised capability list explicitly includes delegation.",
    };
    ToolSchema::new(
        "delegate_tasks",
        description,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "goal": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_GOAL_CHARS,
                    "description": "Overall delegated outcome that the parent Agent will synthesize"
                },
                "context": {
                    "type": "string",
                    "maxLength": MAX_CONTEXT_CHARS,
                    "description": "Bounded shared constraints and project state needed by every child; do not paste the full transcript"
                },
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_DELEGATION_TASKS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": {
                                "type": "string",
                                "pattern": "^[a-z][a-z0-9_-]{0,30}$",
                                "description": "Unique stable task id used by dependencies and results"
                            },
                            "instruction": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_INSTRUCTION_CHARS,
                                "description": "Self-contained assignment with concrete output and acceptance criteria"
                            },
                            "depends_on": {
                                "type": "array",
                                "uniqueItems": true,
                                "items": {
                                    "type": "string",
                                    "pattern": "^[a-z][a-z0-9_-]{0,30}$"
                                },
                                "description": "Task ids from this same batch that must succeed first"
                            },
                            "capabilities": {
                                "type": "array",
                                "minItems": 1,
                                "uniqueItems": true,
                                "items": {"type": "string", "enum": capabilities},
                                "description": capability_help
                            },
                            "specialist_id": {
                                "type": "string",
                                "enum": specialist_ids,
                                "description": format!("Optional reusable persona. Omit for a temporary generic Agent. {specialist_help}")
                            },
                            "output_schema": {
                                "type": "object",
                                "description": "Optional bounded JSON Schema for this task's data result"
                            },
                            "isolated": {
                                "type": "boolean",
                                "description": "Request a temporary Git worktree for this task. Successful changes are conflict-checked and cherry-picked; rejected or failed changes are preserved as patch Artifacts. Use this for independent parallel writers. It is unavailable for non-Git or dirty project checkouts."
                            }
                        },
                        "required": ["id", "instruction", "capabilities"]
                    }
                }
            },
            "required": ["goal", "tasks"]
        }),
    )
}

pub(crate) struct DelegateTasksTool {
    store: Store,
    project: ActiveProject,
    frame_id: String,
    run_manager: RunManager,
    runtime_manager: wisp_runtime::RuntimeManager,
    app_data: PathBuf,
    schema: ToolSchema,
    nested: Option<NestedDelegationContext>,
    policy_override: Option<(CapabilityRegistry, DelegationHostPolicy)>,
    delegator_override: Option<Arc<dyn AgentDelegator>>,
}

impl DelegateTasksTool {
    pub(crate) async fn new(
        store: Store,
        project: ActiveProject,
        frame_id: impl Into<String>,
        run_manager: RunManager,
        runtime_manager: wisp_runtime::RuntimeManager,
        app_data: PathBuf,
    ) -> Result<Self, String> {
        let frame_id = frame_id.into();
        let (policy, nested) =
            delegation_policy_for_frame(&store, &project, &frame_id, &app_data).await?;
        let completion = if nested.is_some() {
            crate::delegation_completion::AgentCompletionSettings::default()
        } else {
            crate::delegation_completion::session_completion_settings(&store, &frame_id).await
        };
        let specialists = specialists::ensure(&store).await;
        let schema =
            build_delegate_tasks_schema(&policy.registry, &policy.host, &specialists, completion);
        Ok(Self {
            store,
            project,
            frame_id,
            run_manager,
            runtime_manager,
            app_data,
            schema,
            nested,
            policy_override: None,
            delegator_override: None,
        })
    }

    #[cfg(test)]
    fn with_runtime(
        store: Store,
        project: ActiveProject,
        frame_id: impl Into<String>,
        policy: (CapabilityRegistry, DelegationHostPolicy),
        delegator: Arc<dyn AgentDelegator>,
    ) -> Self {
        let schema = build_delegate_tasks_schema(
            &policy.0,
            &policy.1,
            &[],
            crate::delegation_completion::AgentCompletionSettings::default(),
        );
        let app_data = project.root.clone();
        let runtime_manager = wisp_runtime::RuntimeManager::local(
            app_data.clone(),
            app_data.join("missing-python-worker.py"),
            None,
            vec![],
        );
        Self {
            store,
            project,
            frame_id: frame_id.into(),
            run_manager: RunManager::new(),
            runtime_manager,
            app_data,
            schema,
            nested: None,
            policy_override: Some(policy),
            delegator_override: Some(delegator),
        }
    }

    async fn policy(
        &self,
    ) -> Result<
        (
            CapabilityRegistry,
            DelegationHostPolicy,
            Option<crate::delegation_resources::ScientificResourceCatalog>,
        ),
        String,
    > {
        match &self.policy_override {
            Some(policy) => Ok((policy.0.clone(), policy.1.clone(), None)),
            None => {
                let (policy, nested) = delegation_policy_for_frame(
                    &self.store,
                    &self.project,
                    &self.frame_id,
                    &self.app_data,
                )
                .await?;
                if self.nested.is_some() != nested.is_some() {
                    return Err("Delegation authority changed before the tool call started.".into());
                }
                Ok((policy.registry, policy.host, Some(policy.resources)))
            }
        }
    }
}

#[async_trait]
impl Tool for DelegateTasksTool {
    fn name(&self) -> &str {
        "delegate_tasks"
    }

    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }

    fn preview(&self, args: &Value) -> String {
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let count = args
            .get("tasks")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        format!("{count} tasks · {goal}")
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        match self.run_batch(args, env).await {
            Ok(value) => ToolResult::ok(serde_json::to_string(&value).unwrap_or_else(|_| {
                r#"{"status":"failed","error":"result serialization failed"}"#.into()
            })),
            Err(error) => ToolResult::fail(format!("delegate_tasks error: {error}")),
        }
    }
}

impl DelegateTasksTool {
    async fn run_batch(&self, args: &Value, env: &dyn ToolEnv) -> Result<Value, String> {
        let nested = if self.nested.is_some() {
            Some(
                nested_delegation_context(&self.store, &self.frame_id)
                    .await?
                    .ok_or_else(|| {
                        "Nested delegation capacity or authority is no longer available."
                            .to_string()
                    })?,
            )
        } else {
            delegation_runtime::require_session_delegation(
                &self.store,
                &self.project.id,
                &self.frame_id,
            )
            .await?;
            None
        };
        let parsed: DelegateTasksArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid task batch: {error}"))?;
        let completion = if nested.is_some() {
            crate::delegation_completion::AgentCompletionSettings::default()
        } else {
            crate::delegation_completion::session_completion_settings(&self.store, &self.frame_id)
                .await
        };
        let workflow_id = uuid::Uuid::new_v4().to_string();
        let (registry, host, resources) = self.policy().await?;
        let proposal = DynamicAgentWorkflowProposal {
            goal: parsed.goal,
            context: parsed.context,
            approval_policy: AgentApprovalPolicy::AutoSafe,
            tasks: parsed
                .tasks
                .into_iter()
                .map(|task| DynamicAgentTaskProposal {
                    id: task.id,
                    instruction: task.instruction,
                    depends_on: task.depends_on,
                    capabilities: task.capabilities,
                    specialist_id: task.specialist_id,
                    output_schema: task.output_schema,
                    isolated: task.isolated,
                    model_id: None,
                    executor: None,
                    budget: None,
                })
                .collect(),
        };
        let mut plan = dynamic_workflow::resolve_proposal(
            &self.store,
            workflow_id.clone(),
            proposal,
            &registry,
            &host,
            resources.as_ref(),
        )
        .await?;
        if let Some(context) = &nested {
            namespace_nested_display_ids(&mut plan, &context.parent_display_id);
        }
        let display_ids = display_task_ids(&plan);
        let snapshot = match &nested {
            Some(context) => {
                delegation_runtime::persist_nested_dynamic_agent_workflow(
                    &self.store,
                    &self.project.id,
                    &self.project.root,
                    self.frame_id.clone(),
                    &plan,
                    &registry,
                    &host,
                    delegation_runtime::NestedWorkflowLineage {
                        root_workflow_id: context.attempt.root_workflow_id.clone(),
                        parent_attempt_id: context.attempt.id.clone(),
                        depth: context.attempt.depth,
                        root_limits_json: serde_json::to_string(&context.limits)
                            .map_err(|error| error.to_string())?,
                    },
                )
                .await?
            }
            None => {
                delegation_runtime::persist_dynamic_agent_workflow(
                    &self.store,
                    &self.project.id,
                    &self.project.root,
                    self.frame_id.clone(),
                    &plan,
                    &registry,
                    &host,
                )
                .await?
            }
        };

        if plan.requires_confirmation && nested.is_none() {
            match env
                .confirm_decision(&approval_prompt(&plan, &display_ids))
                .await
            {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return Ok(json!({
                        "workflow_id": workflow_id,
                        "status": "denied",
                        "feedback": feedback,
                        "results": [],
                        "message": "No delegated Agent was started. Revise the batch using the user's feedback before trying again."
                    }));
                }
            }
        }
        delegation_runtime::approve_created_automatic_workflow(&self.store, snapshot).await?;
        if env.is_cancelled() {
            self.store
                .transition_agent_workflow_status(
                    &workflow_id,
                    wisp_store::AgentWorkflowStatus::Approved,
                    wisp_store::AgentWorkflowStatus::Cancelled,
                )
                .await
                .map_err(|error| error.to_string())?;
            return Ok(json!({
                "workflow_id": workflow_id,
                "status": "cancelled",
                "results": [],
                "message": "The parent turn was cancelled before any delegated Agent started."
            }));
        }
        if completion.policy == crate::delegation_completion::AgentCompletionPolicy::Background {
            let delivery = self
                .store
                .create_agent_workflow_delivery(&workflow_id, completion.auto_resume)
                .await
                .map_err(|error| error.to_string())?;
            let store = self.store.clone();
            let project = self.project.clone();
            let run_manager = self.run_manager.clone();
            let runtime_manager = self.runtime_manager.clone();
            let app_data = self.app_data.clone();
            let workflow_id_for_task = workflow_id.clone();
            let display_ids_for_task = display_ids.clone();
            let delegator = self.delegator_override.clone();
            let dynamic_policy = (registry.clone(), host.clone());
            tokio::spawn(async move {
                let result = match delegator {
                    Some(delegator) => {
                        delegation_runtime::execute_agent_workflow_with_delegator(
                            &store,
                            &project.id,
                            &workflow_id_for_task,
                            delegator,
                            Some(dynamic_policy),
                            Some(delivery.generation),
                        )
                        .await
                    }
                    None => {
                        delegation_runtime::execute_inline_agent_workflow(
                            &store,
                            project,
                            run_manager,
                            runtime_manager,
                            app_data,
                            &workflow_id_for_task,
                            Some(delivery.generation),
                        )
                        .await
                    }
                };
                match result {
                    Ok(execution) => {
                        if let Err(error) = crate::delegation_completion::persist_execution_result(
                            &store,
                            &delivery,
                            &execution,
                            &display_ids_for_task,
                        )
                        .await
                        {
                            tracing::error!(target: "wisp", workflow_id = %workflow_id_for_task, %error, "failed to persist background Agent result");
                        }
                    }
                    Err(error) => {
                        if let Err(persist_error) =
                            crate::delegation_completion::persist_execution_failure(
                                &store, &delivery, &error,
                            )
                            .await
                        {
                            tracing::error!(target: "wisp", workflow_id = %workflow_id_for_task, %persist_error, "failed to persist background Agent failure");
                        }
                    }
                }
            });
            return Ok(background_execution_handle(
                &workflow_id,
                &plan,
                &display_ids,
                completion.auto_resume,
            ));
        }
        if let Some(context) = &nested {
            if !self
                .store
                .set_agent_workflow_attempt_delegation_slot_yielded(&context.attempt.id, true)
                .await
                .map_err(|error| error.to_string())?
            {
                let _ = self
                    .store
                    .transition_agent_workflow_status(
                        &workflow_id,
                        wisp_store::AgentWorkflowStatus::Approved,
                        wisp_store::AgentWorkflowStatus::Cancelled,
                    )
                    .await;
                return Err("Parent Agent could not yield its root concurrency slot.".into());
            }
        }
        let execution = match &self.delegator_override {
            Some(delegator) => {
                delegation_runtime::execute_agent_workflow_with_delegator(
                    &self.store,
                    &self.project.id,
                    &workflow_id,
                    delegator.clone(),
                    Some((registry, host)),
                    None,
                )
                .await
            }
            None => {
                delegation_runtime::execute_inline_agent_workflow(
                    &self.store,
                    self.project.clone(),
                    self.run_manager.clone(),
                    self.runtime_manager.clone(),
                    self.app_data.clone(),
                    &workflow_id,
                    None,
                )
                .await
            }
        };
        if let Some(context) = &nested {
            reacquire_parent_slot(&self.store, &context.attempt.id).await?;
        }
        let execution = execution?;
        Ok(compact_execution_result(&execution, &display_ids))
    }
}

fn namespace_nested_display_ids(plan: &mut DelegationPlan, parent_display_id: &str) {
    for step in &mut plan.steps {
        let Some(display_id) = step.input.get_mut("task_id") else {
            continue;
        };
        let Some(local_id) = display_id.as_str() else {
            continue;
        };
        *display_id = Value::String(format!("{parent_display_id}/{local_id}"));
    }
}

async fn reacquire_parent_slot(store: &Store, attempt_id: &str) -> Result<(), String> {
    loop {
        if store
            .set_agent_workflow_attempt_delegation_slot_yielded(attempt_id, false)
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let running = store
            .get_agent_workflow_attempt(attempt_id)
            .await
            .map_err(|error| error.to_string())?
            .is_some_and(|attempt| {
                attempt.status == wisp_store::AgentWorkflowAttemptStatus::Running
            });
        if !running {
            return Err("Parent Agent attempt ended before it could reacquire capacity.".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn background_execution_handle(
    workflow_id: &str,
    plan: &wisp_core::DelegationPlan,
    display_ids: &HashMap<String, String>,
    auto_resume: bool,
) -> Value {
    let tasks = plan
        .steps
        .iter()
        .map(|step| {
            json!({
                "id": display_ids.get(&step.id).unwrap_or(&step.id),
                "status": "queued"
            })
        })
        .collect::<Vec<_>>();
    json!({
        "workflow_id": workflow_id,
        "status": "running",
        "completion_policy": "background",
        "auto_resume": auto_resume,
        "tasks": tasks,
        "lookup": {
            "tool": "get_delegated_result",
            "mcp_tool": "wisp_get_delegated_result",
            "workflow_id": workflow_id
        },
        "message": "The delegated batch is running in the background. Do not poll or claim completion; one durable result will be delivered to this conversation."
    })
}

fn approval_prompt(
    plan: &wisp_core::DelegationPlan,
    display_ids: &HashMap<String, String>,
) -> String {
    let tasks = plan
        .steps
        .iter()
        .map(|step| {
            let id = display_ids
                .get(&step.id)
                .map(String::as_str)
                .unwrap_or(&step.id);
            let reasons = if step.spec.approval_reasons.is_empty() {
                "native read-only".into()
            } else {
                step.spec.approval_reasons.join("; ")
            };
            format!(
                "[ ] {id}: {}\n    capabilities={} · model={} · executor={} · {reasons}",
                step.spec.goal,
                step.spec.capabilities.join(","),
                step.spec.model.as_deref().unwrap_or("active"),
                step.spec
                    .executor
                    .as_ref()
                    .map(|executor| serde_json::to_string(executor).unwrap_or_default())
                    .unwrap_or_else(|| "native".into()),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}Delegated Agent batch: {}\n{}",
        wisp_tools::plan::PLAN_APPROVAL_PREFIX,
        plan.goal,
        tasks
    )
}

pub(crate) fn display_task_ids(plan: &wisp_core::DelegationPlan) -> HashMap<String, String> {
    plan.steps
        .iter()
        .filter_map(|step| {
            step.input
                .get("task_id")
                .and_then(Value::as_str)
                .map(|id| (step.id.clone(), id.to_string()))
        })
        .collect()
}

pub(crate) fn compact_execution_result(
    execution: &DelegationExecutionResult,
    display_ids: &HashMap<String, String>,
) -> Value {
    let results = execution
        .steps
        .iter()
        .map(|step| {
            let response = &step.response;
            let id = display_ids
                .get(&step.step_id)
                .cloned()
                .unwrap_or_else(|| step.step_id.clone());
            let summary = response
                .output
                .get("summary")
                .and_then(Value::as_str)
                .map(|value| bounded_chars(value, INLINE_TEXT_CHARS))
                .or_else(|| {
                    response
                        .error
                        .as_deref()
                        .map(|value| bounded_chars(value, INLINE_TEXT_CHARS))
                })
                .unwrap_or_default();
            let data = response
                .output
                .get("data")
                .unwrap_or(&response.output);
            json!({
                "id": id,
                "status": response.status,
                "summary": summary,
                "data": compact_value(data, INLINE_DATA_BYTES, &execution.workflow_id, &id),
                "artifacts": response.artifacts,
                "evidence": response.evidence,
                "tests": response.output.get("tests").cloned().unwrap_or_else(|| json!([])),
                "risks": response.output.get("risks").cloned().unwrap_or_else(|| json!([])),
                "usage": response.usage,
                "nested_results": response.nested_results,
                "child_frame_id": response.child_frame_id,
                "error": response.error.as_deref().map(|value| bounded_chars(value, INLINE_TEXT_CHARS)),
                "lookup": {
                    "tool": "get_delegated_result",
                    "mcp_tool": "wisp_get_delegated_result",
                    "workflow_id": execution.workflow_id,
                    "task_id": id,
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "workflow_id": execution.workflow_id,
        "status": execution.status,
        "results": results,
        "message": "Synthesize these ordered delegated results into the final answer. Independent failures are partial evidence; do not hide them."
    })
}

fn compact_value(value: &Value, limit: usize, workflow_id: &str, task_id: &str) -> Value {
    let raw = serde_json::to_string(value).unwrap_or_default();
    if raw.len() <= limit {
        return value.clone();
    }
    json!({
        "truncated": true,
        "preview": bounded_bytes(&raw, limit),
        "lookup": {
            "tool": "get_delegated_result",
            "mcp_tool": "wisp_get_delegated_result",
            "workflow_id": workflow_id,
            "task_id": task_id,
        }
    })
}

fn bounded_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn bounded_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let kept = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

pub(crate) fn get_delegated_result_schema() -> ToolSchema {
    ToolSchema::new(
        "get_delegated_result",
        "Read the latest persisted full result for one task from an earlier delegate_tasks batch. Use only when the compact inline result says it was truncated or lacks needed detail.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "workflow_id": {"type": "string"},
                "task_id": {"type": "string"}
            },
            "required": ["workflow_id", "task_id"]
        }),
    )
}

pub(crate) struct GetDelegatedResultTool {
    store: Store,
    project_id: String,
    frame_id: String,
}

impl GetDelegatedResultTool {
    pub(crate) fn new(
        store: Store,
        project_id: impl Into<String>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for GetDelegatedResultTool {
    fn name(&self) -> &str {
        "get_delegated_result"
    }

    fn schema(&self) -> ToolSchema {
        get_delegated_result_schema()
    }

    fn preview(&self, args: &Value) -> String {
        args.get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match self.read_result(args).await {
            Ok(value) => ToolResult::ok(serde_json::to_string(&value).unwrap_or_default()),
            Err(error) => ToolResult::fail(format!("get_delegated_result error: {error}")),
        }
    }
}

impl GetDelegatedResultTool {
    pub(crate) async fn read_result(&self, args: &Value) -> Result<Value, String> {
        if !nested_result_access_available(&self.store, &self.frame_id).await {
            delegation_runtime::require_session_delegation(
                &self.store,
                &self.project_id,
                &self.frame_id,
            )
            .await?;
        }
        let workflow_id = required_arg(args, "workflow_id")?;
        let task_id = required_arg(args, "task_id")?;
        let workflow = self
            .store
            .get_agent_workflow(workflow_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "workflow does not exist".to_string())?;
        if workflow.project_id != self.project_id
            || workflow.frame_id.as_deref() != Some(self.frame_id.as_str())
        {
            return Err("workflow does not belong to this conversation".into());
        }
        let steps = self
            .store
            .list_agent_workflow_steps(workflow_id)
            .await
            .map_err(|error| error.to_string())?;
        let planned_step_id = serde_json::from_str::<DelegationPlan>(&workflow.plan_json)
            .ok()
            .and_then(|plan| {
                plan.steps.into_iter().find_map(|step| {
                    (step.input.get("task_id").and_then(Value::as_str) == Some(task_id))
                        .then_some(step.id)
                })
            });
        let suffix = format!(":{task_id}");
        let step = steps
            .iter()
            .find(|step| {
                planned_step_id.as_deref() == Some(step.id.as_str())
                    || step.id == task_id
                    || step.id.ends_with(&suffix)
            })
            .ok_or_else(|| "task does not exist in this workflow".to_string())?;
        let attempt = self
            .store
            .list_agent_workflow_attempts(workflow_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|attempt| attempt.step_id == step.id)
            .max_by_key(|attempt| attempt.attempt)
            .ok_or_else(|| "task has no execution attempt".to_string())?;
        let response = attempt
            .response_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        Ok(json!({
            "workflow_id": workflow_id,
            "task_id": task_id,
            "stored_step_id": step.id,
            "attempt": attempt.attempt,
            "status": attempt.status,
            "response": response,
            "error": attempt.error,
        }))
    }
}

fn required_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("'{name}' is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{HashSet, VecDeque},
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Mutex,
        },
    };
    use wisp_core::{
        AgentBudget, AgentDelegationResponse, AgentExecutorRef, AgentOutputSchemaSource,
        AgentUsage, ContextPolicy, DelegationStatus, ExecutorFeature, ExecutorProfilePolicy,
        ModelProfilePolicy, NullOutput, PermissionSet, ValidatedAgentDelegationRequest,
    };
    use wisp_llm::{
        Completion, FunctionCall, LlmError, Message, Provider, Role, StreamSink, ToolCall, Usage,
    };

    struct NoEnv(PathBuf);

    #[async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    struct CancelledEnv(PathBuf);

    #[async_trait]
    impl ToolEnv for CancelledEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        fn is_cancelled(&self) -> bool {
            true
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    struct DecisionEnv {
        root: PathBuf,
        decision: ConfirmDecision,
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolEnv for DecisionEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            self.decision.approved()
        }

        async fn confirm_decision(&self, message: &str) -> ConfirmDecision {
            self.prompts.lock().unwrap().push(message.into());
            self.decision.clone()
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    #[derive(Debug, Clone)]
    struct DelegatorCall {
        task_id: String,
        input: Value,
    }

    struct FakeDelegator {
        active: AtomicUsize,
        max_active: AtomicUsize,
        calls: Mutex<Vec<DelegatorCall>>,
        failed_tasks: HashSet<String>,
        invalid_schema_tasks: HashSet<String>,
    }

    impl FakeDelegator {
        fn new(failed_tasks: &[&str], invalid_schema_tasks: &[&str]) -> Self {
            Self {
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                calls: Mutex::new(vec![]),
                failed_tasks: failed_tasks.iter().map(|value| (*value).into()).collect(),
                invalid_schema_tasks: invalid_schema_tasks
                    .iter()
                    .map(|value| (*value).into())
                    .collect(),
            }
        }

        fn calls(&self) -> Vec<DelegatorCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AgentDelegator for FakeDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            let request = request.into_request();
            let task_id = request
                .input
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or(&request.step_id)
                .to_string();
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.calls.lock().unwrap().push(DelegatorCall {
                task_id: task_id.clone(),
                input: request.input.clone(),
            });
            tokio::time::sleep(std::time::Duration::from_millis(35)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);

            if self.failed_tasks.contains(&task_id) {
                return Ok(AgentDelegationResponse {
                    request_id: request.request_id,
                    status: DelegationStatus::Failed,
                    output: json!({}),
                    artifact_ids: vec![],
                    artifacts: vec![],
                    evidence: vec![],
                    usage: AgentUsage::default(),
                    agent_session_id: None,
                    child_frame_id: Some(format!("child-{task_id}")),
                    error: Some(format!("{task_id} failed intentionally")),
                    nested_results: vec![],
                });
            }
            let output = if task_id == "resources" {
                json!({
                    "summary": "resource references complete",
                    "data": {
                        "data_asset": {"id": "data-1", "kind": "data_asset"},
                        "paper": {"id": "paper-1", "kind": "paper"}
                    },
                    "tests": [],
                    "risks": []
                })
            } else if request.spec.output_schema_source == AgentOutputSchemaSource::Task {
                if self.invalid_schema_tasks.contains(&task_id) {
                    json!({"score": "invalid"})
                } else {
                    json!({"score": 7})
                }
            } else {
                json!({
                    "summary": format!("{task_id} complete"),
                    "data": {"task": task_id},
                    "tests": [],
                    "risks": []
                })
            };
            let artifacts = if task_id == "resources" {
                vec![wisp_core::AgentArtifact {
                    id: "artifact-1".into(),
                    name: "table.tsv".into(),
                    kind: "table".into(),
                    path: Some("results/table.tsv".into()),
                }]
            } else {
                Vec::new()
            };
            Ok(AgentDelegationResponse {
                request_id: request.request_id,
                status: DelegationStatus::Succeeded,
                output,
                artifact_ids: artifacts
                    .iter()
                    .map(|artifact| artifact.id.clone())
                    .collect(),
                artifacts,
                evidence: vec![],
                usage: AgentUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    ..AgentUsage::default()
                },
                agent_session_id: None,
                child_frame_id: Some(format!("child-{task_id}")),
                error: None,
                nested_results: vec![],
            })
        }
    }

    struct CancellableDelegator {
        cancelled: AtomicBool,
    }

    #[async_trait]
    impl AgentDelegator for CancellableDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            let request = request.into_request();
            while !self.cancelled.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(AgentDelegationResponse {
                request_id: request.request_id,
                status: DelegationStatus::Cancelled,
                output: json!({}),
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: AgentUsage::default(),
                agent_session_id: None,
                child_frame_id: Some("cancelled-child".into()),
                error: Some("cancelled by workflow request".into()),
                nested_results: vec![],
            })
        }

        async fn cancel(&self, _request_id: &str) -> anyhow::Result<bool> {
            self.cancelled.store(true, Ordering::SeqCst);
            Ok(true)
        }
    }

    struct SequenceProvider {
        completions: Mutex<VecDeque<Completion>>,
        messages: Mutex<Vec<Vec<Message>>>,
        schemas: Mutex<Vec<Vec<String>>>,
    }

    impl SequenceProvider {
        fn new(completions: Vec<Completion>) -> Self {
            Self {
                completions: Mutex::new(completions.into()),
                messages: Mutex::new(vec![]),
                schemas: Mutex::new(vec![]),
            }
        }

        fn pop(&self, messages: &[Message], tools: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            self.messages.lock().unwrap().push(messages.to_vec());
            self.schemas.lock().unwrap().push(
                tools
                    .iter()
                    .map(|schema| schema.function.name.clone())
                    .collect(),
            );
            self.completions
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("fake parent ran out of completions".into()))
        }
    }

    #[async_trait]
    impl Provider for SequenceProvider {
        fn name(&self) -> &str {
            "sequence-parent"
        }

        fn model(&self) -> &str {
            "sequence-parent-model"
        }

        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            self.pop(messages, tools)
        }

        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.pop(messages, tools)
        }
    }

    fn completion(content: &str, tool_calls: Vec<ToolCall>) -> Completion {
        Completion {
            content: content.into(),
            reasoning: None,
            finish_reason: Some(if tool_calls.is_empty() {
                "stop".into()
            } else {
                "tool_calls".into()
            }),
            tool_calls,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        }
    }

    fn tool_call(args: Value) -> ToolCall {
        ToolCall {
            id: "delegate-call".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "delegate_tasks".into(),
                arguments: args.to_string(),
            },
        }
    }

    fn test_policy() -> (CapabilityRegistry, DelegationHostPolicy) {
        (
            CapabilityRegistry::builtins(),
            DelegationHostPolicy {
                revision: "delegate-tool-test-v1".into(),
                enabled_capabilities: vec![
                    "reasoning".into(),
                    "project_read".into(),
                    "project_write".into(),
                    "review".into(),
                ],
                models: vec![ModelProfilePolicy {
                    id: "local".into(),
                    features: vec![],
                    external: false,
                    enabled: true,
                }],
                executors: vec![ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Native,
                    features: vec![
                        ExecutorFeature::ProjectRead,
                        ExecutorFeature::ProjectWrite,
                        ExecutorFeature::CodeExecution,
                    ],
                    model_ids: vec!["local".into()],
                    enabled: true,
                }],
                default_model_id: Some("local".into()),
                permission_ceiling: PermissionSet {
                    tools: vec![
                        "read".into(),
                        "search".into(),
                        "grep".into(),
                        "write".into(),
                        "edit".into(),
                    ],
                    paths: vec!["project://**".into()],
                    network: false,
                    write: true,
                    execute: true,
                },
                context_ceiling: ContextPolicy {
                    include_history: false,
                    include_artifacts: true,
                    max_tokens: Some(32_000),
                },
                budget_ceiling: AgentBudget {
                    max_tokens: Some(32_000),
                    max_tool_calls: Some(64),
                    max_cost_microunits: Some(1_000_000),
                },
                default_timeout_secs: Some(5),
                timeout_ceiling_secs: Some(5),
                auto_safe: true,
                ..DelegationHostPolicy::default()
            },
        )
    }

    async fn fixture() -> (Store, ActiveProject, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("wisp_delegation_tool_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("store.sqlite");
        let store = Store::open(&database).await.unwrap();
        store
            .create_project("p", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("f", "p", "OPERON", "wisp")
            .await
            .unwrap();
        let project = ActiveProject {
            id: "p".into(),
            root: root.clone(),
            skills: std::sync::Arc::new(wisp_skills::SkillIndex::load(&[])),
            memory: std::sync::Arc::new(wisp_core::MemoryManager::new(&root)),
        };
        (store, project, root)
    }

    async fn enable_delegation(store: &Store) {
        delegation_runtime::save_session_delegation_enabled(store, "p", "f", true)
            .await
            .unwrap();
    }

    fn parse_tool_result(result: &ToolResult) -> Value {
        assert!(result.success, "{}", result.content);
        serde_json::from_str(&result.content).unwrap()
    }

    #[test]
    fn compact_preview_respects_utf8_byte_limit() {
        let compact = compact_value(&json!({"text": "界界界界界界"}), 10, "workflow", "task");
        let preview = compact["preview"].as_str().unwrap();
        assert!(preview.len() <= 13, "preview was {} bytes", preview.len());
        assert_eq!(compact["lookup"]["mcp_tool"], "wisp_get_delegated_result");
    }

    #[test]
    fn delegation_schema_lists_spawnable_specialists_without_private_prompts() {
        let (registry, host) = test_policy();
        let schema = build_delegate_tasks_schema(
            &registry,
            &host,
            &[specialists::Specialist {
                id: "paper-expert".into(),
                name: "Paper expert".into(),
                icon: String::new(),
                color: String::new(),
                description: "Finds primary literature".into(),
                instructions: "PRIVATE SPECIALIST RUBRIC".into(),
                model_id: "private-model-binding".into(),
                review_backend: None,
                skills: None,
                connectors: None,
                builtin: false,
            }],
            crate::delegation_completion::AgentCompletionSettings::default(),
        );
        let parameters = schema.function.parameters.to_string();
        assert!(parameters.contains("paper-expert"));
        assert!(parameters.contains("Finds primary literature"));
        assert!(!parameters.contains("PRIVATE SPECIALIST RUBRIC"));
        assert!(!parameters.contains("private-model-binding"));
    }

    #[tokio::test]
    async fn parent_agent_delegates_parallel_tasks_and_synthesizes_inline() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let args = json!({
            "goal": "Compare two independent inputs",
            "context": "Return concise evidence for the parent.",
            "tasks": [
                {
                    "id": "left",
                    "instruction": "Analyze the left input.",
                    "capabilities": ["reasoning"]
                },
                {
                    "id": "right",
                    "instruction": "Analyze the right input.",
                    "capabilities": ["reasoning"]
                }
            ]
        });
        let provider = SequenceProvider::new(vec![
            completion("", vec![tool_call(args)]),
            completion("Synthesized left and right evidence.", vec![]),
        ]);
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );
        let mut tools = wisp_tools::Registry::builtins().filtered(&[]);
        tools.add(Box::new(tool));
        let mut context = wisp_core::ContextManager::new(32_000);

        wisp_core::agent_loop(
            &mut context,
            &provider,
            None,
            &tools,
            &root,
            &NullOutput,
            "Compare these inputs and report one conclusion.",
            4,
            None,
        )
        .await
        .unwrap();

        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
        let provider_messages = provider.messages.lock().unwrap();
        assert_eq!(provider_messages.len(), 2);
        let tool_message = provider_messages[1]
            .iter()
            .find(|message| message.role == Role::Tool)
            .expect("delegation result should be returned to the parent model");
        let inline: Value = serde_json::from_str(&tool_message.content.as_text()).unwrap();
        assert_eq!(inline["status"], "succeeded");
        assert_eq!(inline["results"].as_array().unwrap().len(), 2);
        assert!(inline["message"].as_str().unwrap().contains("Synthesize"));
        assert_eq!(
            context.messages.last().unwrap().content.as_text(),
            "Synthesized left and right evidence."
        );
        assert!(provider
            .schemas
            .lock()
            .unwrap()
            .iter()
            .all(|schemas| schemas == &["delegate_tasks"]));

        drop(tools);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn background_batch_returns_handle_then_delivers_one_internal_result() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        crate::delegation_completion::save_session_completion_settings(
            &store,
            "p",
            "f",
            crate::delegation_completion::AgentCompletionSettings {
                policy: crate::delegation_completion::AgentCompletionPolicy::Background,
                auto_resume: false,
            },
        )
        .await
        .unwrap();
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool =
            DelegateTasksTool::with_runtime(store.clone(), project, "f", test_policy(), delegator);
        let result = tool
            .run(
                &json!({
                    "goal": "Analyze without blocking the parent",
                    "tasks": [{
                        "id": "background",
                        "instruction": "Return one result.",
                        "capabilities": ["reasoning"]
                    }]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let handle = parse_tool_result(&result);
        assert_eq!(handle["status"], "running");
        assert_eq!(handle["completion_policy"], "background");
        let workflow_id = handle["workflow_id"].as_str().unwrap();
        assert_eq!(
            store
                .list_agent_workflow_deliveries(workflow_id)
                .await
                .unwrap()
                .len(),
            1
        );

        let ready = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let deliveries = store
                    .list_agent_workflow_deliveries(workflow_id)
                    .await
                    .unwrap();
                if deliveries[0].result_json.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(ready.is_ok(), "background result was not persisted");
        assert_eq!(
            store
                .deliver_agent_workflow_completions("f")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .deliver_agent_workflow_completions("f")
            .await
            .unwrap()
            .is_empty());
        let messages = store.load_messages("f").await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].tool_name.as_deref(),
            Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL)
        );
        let envelope: Value = serde_json::from_str(&messages[0].content.as_text()).unwrap();
        assert_eq!(envelope["result"]["status"], "succeeded");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_before_background_start_is_explicit_and_spawns_nothing() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        crate::delegation_completion::save_session_completion_settings(
            &store,
            "p",
            "f",
            crate::delegation_completion::AgentCompletionSettings {
                policy: crate::delegation_completion::AgentCompletionPolicy::Background,
                auto_resume: true,
            },
        )
        .await
        .unwrap();
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );
        let result = tool
            .run(
                &json!({
                    "goal": "Cancelled batch",
                    "tasks": [{
                        "id": "never_started",
                        "instruction": "Must not run.",
                        "capabilities": ["reasoning"]
                    }]
                }),
                &CancelledEnv(root.clone()),
            )
            .await;
        let result = parse_tool_result(&result);
        assert_eq!(result["status"], "cancelled");
        assert!(delegator.calls().is_empty());
        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        assert_eq!(workflow.status, wisp_store::AgentWorkflowStatus::Cancelled);
        assert!(store
            .list_agent_workflow_attempts(&workflow.id)
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .list_agent_workflow_deliveries(&workflow.id)
            .await
            .unwrap()
            .is_empty());

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancellation_after_background_start_delivers_cancelled_terminal_result() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        crate::delegation_completion::save_session_completion_settings(
            &store,
            "p",
            "f",
            crate::delegation_completion::AgentCompletionSettings {
                policy: crate::delegation_completion::AgentCompletionPolicy::Background,
                auto_resume: false,
            },
        )
        .await
        .unwrap();
        let delegator = Arc::new(CancellableDelegator {
            cancelled: AtomicBool::new(false),
        });
        let tool =
            DelegateTasksTool::with_runtime(store.clone(), project, "f", test_policy(), delegator);
        let handle = parse_tool_result(
            &tool
                .run(
                    &json!({
                        "goal": "Cancel a running batch",
                        "tasks": [{
                            "id": "running",
                            "instruction": "Wait until cancelled.",
                            "capabilities": ["reasoning"]
                        }]
                    }),
                    &NoEnv(root.clone()),
                )
                .await,
        );
        let workflow_id = handle["workflow_id"].as_str().unwrap().to_string();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if store
                    .list_agent_workflow_attempts(&workflow_id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|attempt| {
                        attempt.status == wisp_store::AgentWorkflowAttemptStatus::Running
                    })
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            store
                .request_agent_workflow_cancel(&workflow_id)
                .await
                .unwrap(),
            1
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let deliveries = store
                    .list_agent_workflow_deliveries(&workflow_id)
                    .await
                    .unwrap();
                if deliveries[0].result_json.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            store
                .get_agent_workflow(&workflow_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            wisp_store::AgentWorkflowStatus::Cancelled
        );
        let delivery = store
            .list_agent_workflow_deliveries(&workflow_id)
            .await
            .unwrap()
            .remove(0);
        let envelope: Value = serde_json::from_str(&delivery.result_json.unwrap()).unwrap();
        assert_eq!(envelope["result"]["status"], "cancelled");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn inline_batch_runs_two_tasks_in_parallel_then_supplies_fan_in_results() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Analyze both sources and combine the evidence",
                    "tasks": [
                        {
                            "id": "source_a",
                            "instruction": "Analyze source A.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "source_b",
                            "instruction": "Analyze source B.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "combine",
                            "instruction": "Combine both source results.",
                            "depends_on": ["source_a", "source_b"],
                            "capabilities": ["reasoning"]
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);

        assert_eq!(value["status"], "succeeded");
        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
        let calls = delegator.calls();
        let combine = calls.iter().find(|call| call.task_id == "combine").unwrap();
        let dependencies = combine.input["dependency_results"].as_object().unwrap();
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies
            .values()
            .all(|output| output["summary"].as_str().is_some()));
        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        assert_eq!(workflow.max_parallel, 2);
        assert_eq!(workflow.status, wisp_store::AgentWorkflowStatus::Succeeded);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn durable_resource_references_survive_persistence_and_parent_delivery() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            Arc::new(FakeDelegator::new(&[], &[])),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Return durable scientific references",
                    "tasks": [{
                        "id": "resources",
                        "instruction": "Return Artifact, DataAsset, and Paper references.",
                        "capabilities": ["reasoning"]
                    }]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let inline = parse_tool_result(&result);
        assert_eq!(inline["results"][0]["data"]["data_asset"]["id"], "data-1");
        assert_eq!(inline["results"][0]["data"]["paper"]["id"], "paper-1");
        assert_eq!(inline["results"][0]["artifacts"][0]["id"], "artifact-1");

        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        let attempt = store
            .list_agent_workflow_attempts(&workflow.id)
            .await
            .unwrap()
            .remove(0);
        let persisted: Value =
            serde_json::from_str(attempt.response_json.as_deref().unwrap()).unwrap();
        assert_eq!(persisted["output"]["data"]["data_asset"]["id"], "data-1");
        assert_eq!(persisted["output"]["data"]["paper"]["id"], "paper-1");
        assert_eq!(persisted["artifacts"][0]["id"], "artifact-1");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn approval_denial_starts_no_children_and_returns_feedback_to_parent() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );
        let env = DecisionEnv {
            root: root.clone(),
            decision: ConfirmDecision::Denied {
                feedback: Some("keep this read-only".into()),
            },
            prompts: Mutex::new(vec![]),
        };

        let result = tool
            .run(
                &json!({
                    "goal": "Edit the report",
                    "tasks": [{
                        "id": "edit_report",
                        "instruction": "Edit report.md.",
                        "capabilities": ["project_write"]
                    }]
                }),
                &env,
            )
            .await;
        let value = parse_tool_result(&result);

        assert_eq!(value["status"], "denied");
        assert_eq!(value["feedback"], "keep this read-only");
        assert!(value["results"].as_array().unwrap().is_empty());
        assert!(delegator.calls().is_empty());
        {
            let prompts = env.prompts.lock().unwrap();
            assert_eq!(prompts.len(), 1);
            assert!(prompts[0].contains(wisp_tools::plan::PLAN_APPROVAL_PREFIX));
            assert!(prompts[0].contains("project_write"));
            assert!(prompts[0].contains("native"));
        }
        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        assert_eq!(workflow.status, wisp_store::AgentWorkflowStatus::Draft);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn independent_success_survives_failure_while_only_descendants_are_blocked() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&["bad"], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Collect as much independent evidence as possible",
                    "tasks": [
                        {
                            "id": "bad",
                            "instruction": "Run the failing analysis.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "independent",
                            "instruction": "Run an independent analysis.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "downstream",
                            "instruction": "Use the failing analysis.",
                            "depends_on": ["bad"],
                            "capabilities": ["reasoning"]
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);
        let results = value["results"].as_array().unwrap();
        let status = |id: &str| {
            results.iter().find(|item| item["id"] == id).unwrap()["status"]
                .as_str()
                .unwrap()
        };

        assert_eq!(value["status"], "failed");
        assert_eq!(status("bad"), "failed");
        assert_eq!(status("independent"), "succeeded");
        assert_eq!(status("downstream"), "blocked");
        let called = delegator
            .calls()
            .into_iter()
            .map(|call| call.task_id)
            .collect::<HashSet<_>>();
        assert_eq!(called, HashSet::from(["bad".into(), "independent".into()]));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn task_output_schema_is_enforced_and_full_result_can_be_loaded() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &["invalid"]));
        let tool =
            DelegateTasksTool::with_runtime(store.clone(), project, "f", test_policy(), delegator);
        let score_schema = json!({
            "type": "object",
            "properties": {"score": {"type": "integer", "minimum": 0}},
            "required": ["score"],
            "additionalProperties": false
        });

        let result = tool
            .run(
                &json!({
                    "goal": "Return validated scores",
                    "tasks": [
                        {
                            "id": "valid",
                            "instruction": "Return a valid score.",
                            "capabilities": ["reasoning"],
                            "output_schema": score_schema
                        },
                        {
                            "id": "invalid",
                            "instruction": "Return an invalid score for validation testing.",
                            "capabilities": ["reasoning"],
                            "output_schema": score_schema
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);
        let workflow_id = value["workflow_id"].as_str().unwrap();
        let results = value["results"].as_array().unwrap();
        assert_eq!(results[0]["status"], "succeeded");
        assert_eq!(results[0]["data"]["score"], 7);
        assert_eq!(results[1]["status"], "failed");
        assert!(results[1]["error"]
            .as_str()
            .unwrap()
            .contains("output_contract"));
        assert_eq!(results[1]["usage"]["input_tokens"], 1);

        let lookup = GetDelegatedResultTool::new(store.clone(), "p", "f")
            .run(
                &json!({"workflow_id": workflow_id, "task_id": "valid"}),
                &NoEnv(root.clone()),
            )
            .await;
        let full = parse_tool_result(&lookup);
        assert_eq!(full["response"]["output"]["data"]["score"], 7);
        assert_eq!(full["response"]["child_frame_id"], "child-valid");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn disabled_session_cannot_invoke_inline_delegation() {
        let (store, project, root) = fixture().await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Analyze one input",
                    "tasks": [{
                        "id": "analysis",
                        "instruction": "Analyze it.",
                        "capabilities": ["reasoning"]
                    }]
                }),
                &NoEnv(root.clone()),
            )
            .await;

        assert!(!result.success);
        assert!(result.content.contains("delegation is off"));
        assert!(delegator.calls().is_empty());
        assert!(store.list_agent_workflows("p").await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
