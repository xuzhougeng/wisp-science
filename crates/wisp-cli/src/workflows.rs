//! Headless independent Workflow conversion and execution. The desktop JSON
//! proposal contract and core policy/executor are shared; CLI children have
//! fresh contexts and an explicitly filtered local tool registry.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use wisp_core::workflow_conversion::{self, WorkflowSource};
use wisp_core::{
    agent_loop, AgentDelegationResponse, AgentDelegator, AgentLoopOutcome, AgentUsage,
    CapabilityRegistry, ContextManager, ContextPolicy, DelegationExecutionObserver,
    DelegationExecutionStatus, DelegationExecutor, DelegationHostPolicy, DelegationPlan,
    DelegationStatus, ExecutorFeature, ExecutorProfilePolicy, ModelProfilePolicy, Output,
    PermissionSet, ValidatedAgentDelegationRequest,
};
use wisp_dto::WorkflowTemplate;
use wisp_llm::{Message, Provider, ProviderConfig, Role, ToolSchema};
use wisp_runs::RunManager;
use wisp_skills::SkillIndex;
use wisp_store::Store;
use wisp_tools::{Approval, Registry, Tool, ToolEnv, ToolEvent, ToolResult};

const TEMPLATES: &str = "workflow_templates";
type ProviderFactory = Arc<dyn Fn() -> Box<dyn Provider> + Send + Sync>;

#[derive(Clone)]
struct WorkflowHost {
    store: Store,
    skills: Arc<SkillIndex>,
    factory: ProviderFactory,
    registry: CapabilityRegistry,
    policy: DelegationHostPolicy,
    manager: RunManager,
    max_context: usize,
    max_iter: usize,
}

pub fn register(
    tools: &mut Registry,
    store: Store,
    skills: Arc<SkillIndex>,
    cfg: ProviderConfig,
    manager: RunManager,
    max_context: usize,
    max_iter: usize,
) {
    let (registry, policy) = local_policy(&cfg.model);
    let host = WorkflowHost {
        store,
        skills,
        factory: Arc::new(move || wisp_llm::build(cfg.clone())),
        registry,
        policy,
        manager,
        max_context,
        max_iter,
    };
    for kind in [Kind::Create, Kind::Explain, Kind::Run] {
        tools.add(Box::new(WorkflowTool {
            host: host.clone(),
            kind,
        }));
    }
}

fn local_policy(model: &str) -> (CapabilityRegistry, DelegationHostPolicy) {
    let mut definitions = CapabilityRegistry::builtins().definitions();
    let code = definitions
        .iter_mut()
        .find(|definition| definition.id == "code_run")
        .unwrap();
    code.permissions
        .tools
        .retain(|tool| matches!(tool.as_str(), "read" | "search" | "grep" | "run_in_context"));
    let registry = CapabilityRegistry::new("cli-workflow-capabilities-v1", definitions).unwrap();
    // Advertise only paths implemented by the headless child. No inherited
    // MCP catalog, Python REPL, shell or nested delegation is implied.
    let enabled: Vec<String> = ["reasoning", "project_read", "project_write", "code_run"]
        .iter()
        .map(|s| (*s).into())
        .collect();
    let tools = enabled
        .iter()
        .flat_map(|id| registry.get(id).unwrap().permissions.tools.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let policy = DelegationHostPolicy {
        revision: format!("cli-independent-workflow-v1:{model}"),
        enabled_capabilities: enabled,
        models: vec![ModelProfilePolicy {
            id: model.into(),
            features: vec![],
            external: true,
            enabled: true,
        }],
        executors: vec![ExecutorProfilePolicy {
            executor: wisp_core::AgentExecutorRef::Native,
            features: vec![
                ExecutorFeature::ProjectRead,
                ExecutorFeature::ProjectWrite,
                ExecutorFeature::CodeExecution,
            ],
            model_ids: vec![model.into()],
            enabled: true,
        }],
        default_model_id: Some(model.into()),
        inherit_model_id: None,
        permission_ceiling: PermissionSet {
            tools,
            paths: vec!["project://**".into()],
            write: true,
            execute: true,
            network: false,
        },
        context_ceiling: ContextPolicy {
            include_history: false,
            include_artifacts: true,
            max_tokens: None,
        },
        ..Default::default()
    };
    (registry, policy)
}

#[derive(Clone, Copy)]
enum Kind {
    Create,
    Explain,
    Run,
}
struct WorkflowTool {
    host: WorkflowHost,
    kind: Kind,
}

#[async_trait]
impl Tool for WorkflowTool {
    fn name(&self) -> &str {
        match self.kind {
            Kind::Create => "create_workflow",
            Kind::Explain => "explain_workflow",
            Kind::Run => "run_workflow",
        }
    }
    fn read_only(&self) -> bool {
        matches!(self.kind, Kind::Explain)
    }
    fn schema(&self) -> ToolSchema {
        let (description, properties, required) = match self.kind {
            Kind::Create => (
                "Convert an installed Skill and its full Markdown references using the configured LLM into an independent, validated Workflow. Saves explicit node instructions, capability grants and output contracts, with no runtime Skill binding. Does NOT execute it. Call this tool instead of writing workflow JSON manually.",
                json!({"skill_name":{"type":"string"},"workflow_name":{"type":"string"}}), vec!["skill_name"],
            ),
            Kind::Explain => (
                "Inspect a saved independent Workflow: exact proposal, source hashes and host-resolved permissions. Use workflow_id '*' to list templates. Legacy Skill-bound templates require reconversion.",
                json!({"workflow_id":{"type":"string"}}), vec!["workflow_id"],
            ),
            Kind::Run => (
                "Execute a saved independent Workflow with a concrete user request. Shows resolved nodes/permissions and asks the host for approval before execution. Runs fresh child Agents with filtered tools using the core Workflow executor. Strictly checks output contracts and declared files. Local Native execution only. Does not reload source Skills. Results and child tool traces are persisted under .wisp/workflow-runs. Do not call this unless the user requested execution/testing.",
                json!({"workflow_id":{"type":"string"},"request":{"type":"string"},"retry_run_id":{"type":"string","description":"Retry a completed failed run, reusing only successful nodes whose contracts and file hashes still match. Omit request to reuse the original input."}}), vec!["workflow_id"],
            ),
        };
        ToolSchema::new(
            self.name(),
            description,
            json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        )
    }
    fn preview(&self, args: &Value) -> String {
        args.to_string()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let result = match self.kind {
            Kind::Create => self.host.create(args, env).await,
            Kind::Explain => self.host.explain(args).await,
            Kind::Run => self.host.execute(args, env).await,
        };
        match result {
            Ok(value)
                if matches!(
                    value.get("status").and_then(Value::as_str),
                    Some("failed" | "cancelled")
                ) =>
            {
                ToolResult::fail(value.to_string())
            }
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(format!("{error:#}")),
        }
    }
}

impl WorkflowHost {
    async fn templates(&self) -> Result<Vec<WorkflowTemplate>> {
        self.store
            .get_setting(TEMPLATES)
            .await?
            .map(|raw| serde_json::from_str(&raw).context("Invalid persisted Workflow catalog"))
            .unwrap_or_else(|| Ok(vec![]))
    }
    async fn template(&self, id: &str) -> Result<WorkflowTemplate> {
        self.templates()
            .await?
            .into_iter()
            .find(|t| t.id == id)
            .context("Workflow does not exist; use explain_workflow with '*' to list")
    }
    async fn create(&self, args: &Value, env: &dyn ToolEnv) -> Result<Value> {
        let name = wisp_tools::tool::arg_str(args, "skill_name").map_err(anyhow::Error::msg)?;
        let skill = self.skills.get(&name).context(
            "Skill is not installed; configure WISP_SKILLS_PATH or install it under .wisp/skills",
        )?;
        let source = WorkflowSource::read(skill)?;
        env.emit(ToolEvent::Stdout {
            chunk: format!(
                "Converting {name} and {} method documents into independent nodes...\n",
                source.files.len()
            ),
        })
        .await;
        let provider = (self.factory)();
        let proposal =
            workflow_conversion::convert(&source, provider.as_ref(), &self.registry, &self.policy)
                .await?;
        let id = uuid::Uuid::new_v4().to_string();
        let template = WorkflowTemplate {
            id: id.clone(),
            name: args
                .get("workflow_name")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .into(),
            description: format!(
                "Independent Workflow converted from {name}; source SHA-256 {}",
                source.sha256
            ),
            proposal,
            builtin: false,
        };
        let mut templates = self.templates().await?;
        if templates
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&template.name))
        {
            bail!("Workflow name already exists; choose a different workflow_name");
        }
        self.store
            .set_setting(
                &format!("workflow_source:{id}"),
                &serde_json::to_string(&source)?,
            )
            .await?;
        templates.push(template.clone());
        self.store
            .set_setting(TEMPLATES, &serde_json::to_string(&templates)?)
            .await?;
        Ok(
            json!({"created":true,"workflow":template,"source_sha256":source.sha256,"next":"Inspect with explain_workflow, then run_workflow if the user requested execution."}),
        )
    }
    async fn explain(&self, args: &Value) -> Result<Value> {
        let id = wisp_tools::tool::arg_str(args, "workflow_id").map_err(anyhow::Error::msg)?;
        if id == "*" {
            return Ok(json!({"workflows":self.templates().await?}));
        }
        let template = self.template(&id).await?;
        let plan = workflow_conversion::resolve(
            &template.proposal,
            &id,
            "",
            &self.registry,
            &self.policy,
        )?;
        Ok(json!({"workflow":template,"resolved_plan":plan}))
    }
    async fn execute(&self, args: &Value, env: &dyn ToolEnv) -> Result<Value> {
        let id = wisp_tools::tool::arg_str(args, "workflow_id").map_err(anyhow::Error::msg)?;
        let root = env.project_root().canonicalize()?;
        let prior = args
            .get("retry_run_id")
            .and_then(Value::as_str)
            .map(|id| load_retry(&root, id))
            .transpose()?;
        let previous_input = prior
            .as_ref()
            .and_then(|(plan, _, _)| plan.steps.first())
            .and_then(|step| step.input.get("request"))
            .and_then(Value::as_str);
        let request = args
            .get("request")
            .and_then(Value::as_str)
            .or(previous_input)
            .context("Workflow execution requires request (or retry_run_id)")?
            .to_string();
        if request.trim().is_empty() {
            bail!("Workflow execution needs a nonempty request");
        }
        let template = self.template(&id).await?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let plan = workflow_conversion::resolve(
            &template.proposal,
            &run_id,
            &request,
            &self.registry,
            &self.policy,
        )?;
        let completed = if let Some((previous, completed, _)) = &prior {
            if previous.steps != plan.steps || previous.goal != plan.goal {
                bail!("Workflow/input/authorization changed since the prior run; start a fresh run instead");
            }
            completed.clone()
        } else {
            vec![]
        };
        let reused = completed
            .iter()
            .map(|step| step.step_id.clone())
            .collect::<Vec<_>>();
        let review = serde_json::to_string_pretty(&plan)?;
        if !env
            .confirm(&format!(
                "Run independent Workflow '{}' with this exact plan?\nReusing verified successful nodes: {}\n{review}",
                template.name, reused.join(", ")
            ))
            .await
        {
            bail!("Workflow execution was not approved; no nodes started");
        }
        self.registry.validate_resolved_plan(&plan, &self.policy)?;
        let state_dir =
            wisp_tools::safety::resolve_under_root(&root, ".wisp").map_err(anyhow::Error::msg)?;
        let runs_dir = state_dir.join("workflow-runs");
        if !runs_dir.exists() {
            std::fs::create_dir(&runs_dir)?;
        }
        wisp_tools::safety::resolve_under_root(&root, ".wisp/workflow-runs")
            .map_err(anyhow::Error::msg)?;
        let directory =
            wisp_tools::safety::validate_file_path(&root, &format!(".wisp/workflow-runs/{run_id}"))
                .map_err(anyhow::Error::msg)?;
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("plan.json"),
            serde_json::to_vec_pretty(&plan)?,
        )?;
        for step in &completed {
            let data = step
                .response
                .output
                .get("data")
                .unwrap_or(&step.response.output);
            validate_artifacts(&root, &directory, data)?;
            std::fs::write(
                directory.join(format!("{}.result.json", step.step_id)),
                serde_json::to_vec_pretty(&step.response)?,
            )?;
        }
        std::fs::write(
            directory.join("reused.json"),
            serde_json::to_vec_pretty(&json!({
                "previous_run":args.get("retry_run_id"),"steps":reused
            }))?,
        )?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (approvals, mut approval_rx) = tokio::sync::mpsc::unbounded_channel::<NodeApproval>();
        let delegator = Arc::new(CliDelegator {
            factory: self.factory.clone(),
            root,
            directory: directory.clone(),
            store: self.store.clone(),
            manager: self.manager.clone(),
            cancelled: cancelled.clone(),
            max_context: self.max_context,
            max_iter: self.max_iter,
            approvals,
        });
        let executor = DelegationExecutor::new(delegator)
            .with_dynamic_policy(self.registry.clone(), self.policy.clone())
            .with_observer(Arc::new(FileObserver {
                directory: directory.clone(),
                cancelled: cancelled.clone(),
            }));
        let execution = executor.execute_with_completed_steps(plan, completed);
        tokio::pin!(execution);
        let result = loop {
            tokio::select! {
                result = &mut execution => break result?,
                Some(request) = approval_rx.recv() => {
                    let approved = env.confirm(&request.message).await;
                    let _ = request.reply.send(approved);
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                    if env.is_cancelled() { cancelled.store(true, Ordering::SeqCst); }
                }
            }
        };
        std::fs::write(
            directory.join("result.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        Ok(
            json!({"status":result.status,"run_id":run_id,"workflow_id":id,"reused_steps":reused,"trace_directory":directory,"result":result}),
        )
    }
}

fn load_retry(
    root: &Path,
    id: &str,
) -> Result<(
    DelegationPlan,
    Vec<wisp_core::DelegationStepExecution>,
    PathBuf,
)> {
    uuid::Uuid::parse_str(id).context("retry_run_id must be a valid run UUID")?;
    let directory =
        wisp_tools::safety::resolve_under_root(root, &format!(".wisp/workflow-runs/{id}"))
            .map_err(anyhow::Error::msg)?;
    let plan: DelegationPlan =
        serde_json::from_slice(&std::fs::read(directory.join("plan.json"))?)?;
    let result: wisp_core::DelegationExecutionResult =
        serde_json::from_slice(&std::fs::read(directory.join("result.json"))?)
            .context("Only terminal runs with a persisted result can be retried")?;
    if result.status != DelegationExecutionStatus::Failed
        || plan.id != id
        || result.workflow_id != id
    {
        bail!("Only a completed failed run can be retried");
    }
    let mut hashes = std::collections::HashMap::new();
    if directory.join("artifacts.jsonl").is_file() {
        for line in std::fs::read_to_string(directory.join("artifacts.jsonl"))?.lines() {
            for item in serde_json::from_str::<Vec<Value>>(line)? {
                hashes.insert(
                    item["path"]
                        .as_str()
                        .context("Invalid snapshot path")?
                        .to_string(),
                    item["sha256"]
                        .as_str()
                        .context("Invalid snapshot hash")?
                        .to_string(),
                );
            }
        }
    }
    let completed = result
        .steps
        .into_iter()
        .filter(|step| step.response.status == DelegationStatus::Succeeded)
        .collect::<Vec<_>>();
    for step in &completed {
        let spec = &plan
            .steps
            .iter()
            .find(|node| node.id == step.step_id)
            .context("Unknown cached node")?
            .spec;
        let data = step
            .response
            .output
            .get("data")
            .unwrap_or(&step.response.output);
        if !wisp_core::delegation::matches_json_contract(data, &spec.output_contract) {
            bail!(
                "Cached node {} no longer satisfies its output contract",
                step.step_id
            );
        }
        for item in data["artifacts"]
            .as_array()
            .context("Invalid cached artifacts")?
        {
            let path = item.as_str().context("Invalid cached artifact path")?;
            let file =
                wisp_tools::safety::validate_file_path(root, path).map_err(anyhow::Error::msg)?;
            if std::fs::metadata(&file)?.len() > 32 * 1024 * 1024 {
                bail!("Cached artifact grew beyond snapshot limit");
            }
            let hash = format!("{:x}", Sha256::digest(std::fs::read(&file)?));
            if hashes.get(path) != Some(&hash) {
                bail!("Cached artifact changed: {path}; start a fresh run");
            }
        }
    }
    Ok((plan, completed, directory))
}

struct NodeApproval {
    message: String,
    reply: tokio::sync::oneshot::Sender<bool>,
}

struct FileObserver {
    directory: PathBuf,
    cancelled: Arc<AtomicBool>,
}
#[async_trait]
impl DelegationExecutionObserver for FileObserver {
    async fn workflow_started(&self, _: &DelegationPlan) -> Result<()> {
        std::fs::write(
            self.directory.join("status.json"),
            br#"{"status":"running"}"#,
        )?;
        Ok(())
    }
    async fn workflow_cancel_requested(&self, _: &DelegationPlan) -> Result<bool> {
        Ok(self.cancelled.load(Ordering::SeqCst))
    }
    async fn step_finished(
        &self,
        request: &wisp_core::AgentDelegationRequest,
        response: &AgentDelegationResponse,
    ) -> Result<()> {
        std::fs::write(
            self.directory
                .join(format!("{}.result.json", request.step_id)),
            serde_json::to_vec_pretty(response)?,
        )?;
        Ok(())
    }
    async fn workflow_finished(
        &self,
        _: &DelegationPlan,
        status: DelegationExecutionStatus,
    ) -> Result<()> {
        std::fs::write(
            self.directory.join("status.json"),
            serde_json::to_vec(&json!({"status":status}))?,
        )?;
        Ok(())
    }
}

struct CliDelegator {
    factory: ProviderFactory,
    root: PathBuf,
    directory: PathBuf,
    store: Store,
    manager: RunManager,
    cancelled: Arc<AtomicBool>,
    max_context: usize,
    max_iter: usize,
    approvals: tokio::sync::mpsc::UnboundedSender<NodeApproval>,
}

/// The CLI Workflow host currently grants only synchronous local Runs. The
/// boundary is enforced here, not merely requested in the node prompt.
struct LocalRunTool(wisp_runs::RunInContextTool);
#[async_trait]
impl Tool for LocalRunTool {
    fn name(&self) -> &str {
        "run_in_context"
    }
    fn schema(&self) -> ToolSchema {
        self.0.schema()
    }
    fn preview(&self, args: &Value) -> String {
        self.0.preview(args)
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        if args.get("context_id") != Some(&json!("local")) {
            return ToolResult::fail("This Workflow node is authorized only for context_id=local");
        }
        if args.get("wait_for_completion") != Some(&json!(true)) {
            return ToolResult::fail("Workflow Runs require wait_for_completion=true; detached work cannot satisfy the node contract");
        }
        self.0.run(args, env).await
    }
}

#[async_trait]
impl AgentDelegator for CliDelegator {
    async fn delegate_validated(
        &self,
        request: ValidatedAgentDelegationRequest,
    ) -> Result<AgentDelegationResponse> {
        let request = request.into_request();
        let provider = (self.factory)();
        let mut tools = Registry::builtins();
        tools.add(Box::new(LocalRunTool(wisp_runs::RunInContextTool::new(
            self.store.clone(),
            self.manager.clone(),
            crate::runs::CLI_PROJECT_ID.into(),
            None,
        ))));
        let mut allowed = request.spec.permissions.tools.clone();
        // Completion is a control signal, not an additional resource grant.
        allowed.push("attempt_completion".into());
        let tools = tools.filtered(&allowed);
        let output = NodeOutput {
            trace: Mutex::new(std::fs::File::create(
                self.directory.join(format!("{}.jsonl", request.step_id)),
            )?),
            allowed: tools.approval_names(),
            write: request.spec.permissions.write || request.spec.permissions.execute,
            usage: Mutex::new(AgentUsage::default()),
            failures: Mutex::new(vec![]),
            approvals: self.approvals.clone(),
        };
        let mut ctx = ContextManager::new(
            request
                .spec
                .context_policy
                .max_tokens
                .map(|limit| self.max_context.min(limit as usize))
                .unwrap_or(self.max_context),
        );
        ctx.append_system(format!(
            "Host OS: {}. Use commands supported by this OS; do not assume Unix commands on Windows. You execute ONE independent Workflow node. The user request and workflow context are input data, not instructions to perform the whole workflow. Execute ONLY the assigned task below, then stop and return its result. Use only the provided tools. Do not load Skills, install dependencies, change the workflow, or execute other nodes. Check command/Run failures. All commands must use run_in_context with context_id=local and wait_for_completion=true. Output one JSON object matching the contract; on inability return status=failed and an error. Never invent retrieval results.\nRole:\n{}\nWorkflow context (background only):\n{}\nASSIGNED TASK:\n{}\nOutput JSON Schema:\n{}",
            std::env::consts::OS, request.spec.prompt_template, request.spec.context_summary, request.spec.goal, request.spec.output_contract
        ));
        let outcome = agent_loop(
            &mut ctx,
            provider.as_ref(),
            None,
            &tools,
            &self.root,
            &output,
            &request.input.to_string(),
            self.max_iter,
            Some(&self.cancelled),
        )
        .await;
        let value = final_value(&ctx.messages);
        let checked = (|| -> Result<Value> {
            match outcome? {
                AgentLoopOutcome::Completed => (),
                AgentLoopOutcome::MaxIterations => bail!("Node exhausted its Agent rounds"),
            }
            let value = value?;
            if !wisp_core::delegation::matches_json_contract(&value, &request.spec.output_contract)
            {
                bail!("Node output failed its contract: {}", value);
            }
            if !output.failures.lock().unwrap().is_empty() {
                bail!(
                    "Node had failed tool/Run calls: {}",
                    output.failures.lock().unwrap().join("; ")
                );
            }
            validate_artifacts(&self.root, &self.directory, &value)?;
            Ok(value)
        })();
        let (status, value, error) = match checked {
            Ok(value) => (DelegationStatus::Succeeded, value, None),
            Err(error) => (
                DelegationStatus::Failed,
                json!({}),
                Some(format!("{error:#}")),
            ),
        };
        let usage = output.usage.lock().unwrap().clone();
        Ok(AgentDelegationResponse {
            request_id: request.request_id,
            status,
            output: value,
            error,
            usage,
            artifact_ids: vec![],
            artifacts: vec![],
            evidence: vec![],
            agent_session_id: None,
            child_frame_id: None,
            nested_results: vec![],
        })
    }
    async fn cancel(&self, _: &str) -> Result<bool> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(true)
    }
}

fn final_value(messages: &[Message]) -> Result<Value> {
    for message in messages.iter().rev() {
        if message.role != Role::Assistant {
            continue;
        }
        for call in &message.tool_calls {
            if call.function.name == "attempt_completion" {
                if let Some(text) = call.args_value().get("result").and_then(Value::as_str) {
                    return parse_value(text);
                }
            }
        }
        let text = message.content.as_text();
        if !text.trim().is_empty() {
            return parse_value(&text);
        }
    }
    bail!("Node returned no JSON result")
}
fn parse_value(raw: &str) -> Result<Value> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("```json")
        .or_else(|| raw.strip_prefix("```"))
        .and_then(|s| s.trim_end().strip_suffix("```"))
        .unwrap_or(raw)
        .trim();
    serde_json::from_str(raw).context("Node returned invalid JSON")
}

fn validate_artifacts(root: &Path, directory: &Path, output: &Value) -> Result<()> {
    let artifacts = output
        .get("artifacts")
        .and_then(Value::as_array)
        .context("Missing artifacts array")?;
    let mut snapshots = vec![];
    for item in artifacts {
        let path = item
            .as_str()
            .context("Artifact must be a project-relative path string")?;
        if Path::new(path).is_absolute() || path.contains(':') {
            bail!("Artifact must be project-relative: {path}");
        }
        let file =
            wisp_tools::safety::validate_file_path(root, path).map_err(anyhow::Error::msg)?;
        if !file.is_file() {
            bail!("Declared artifact does not exist or is not a regular file: {path}");
        }
        let metadata = std::fs::metadata(&file)?;
        if metadata.len() > 32 * 1024 * 1024 {
            bail!("Artifact exceeds local snapshot limit: {path}");
        }
        let bytes = std::fs::read(&file)?;
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let storage = directory.join("artifacts");
        std::fs::create_dir_all(&storage)?;
        std::fs::write(storage.join(&sha), bytes)?;
        snapshots.push(json!({"path":path,"sha256":sha}));
    }
    // Each node also has a result/trace; this manifest accumulates verified
    // file versions, even when a later node changes the same path.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("artifacts.jsonl"))?;
    writeln!(file, "{}", serde_json::to_string(&snapshots)?)?;
    Ok(())
}

struct NodeOutput {
    trace: Mutex<std::fs::File>,
    allowed: HashSet<String>,
    write: bool,
    usage: Mutex<AgentUsage>,
    failures: Mutex<Vec<String>>,
    approvals: tokio::sync::mpsc::UnboundedSender<NodeApproval>,
}
impl Output for NodeOutput {
    fn restrict_read_paths_to_project(&self) -> bool {
        true
    }
    fn project_write_locked(&self) -> bool {
        !self.write
    }
    fn approval_mode(&self, tool: &str) -> Approval {
        if self.allowed.contains(tool) {
            Approval::Allow
        } else {
            Approval::Deny
        }
    }
    fn confirm(&self, _: &str) -> bool {
        false
    }
    fn confirm_async<'a>(&'a self, message: &'a str) -> wisp_core::OutputFuture<'a, bool> {
        Box::pin(async move {
            let (reply, result) = tokio::sync::oneshot::channel();
            if self
                .approvals
                .send(NodeApproval {
                    message: message.into(),
                    reply,
                })
                .is_err()
            {
                return false;
            }
            result.await.unwrap_or(false)
        })
    }
    fn confirm_decision_async<'a>(
        &'a self,
        message: &'a str,
    ) -> wisp_core::OutputFuture<'a, wisp_tools::ConfirmDecision> {
        Box::pin(async move {
            if self.confirm_async(message).await {
                wisp_tools::ConfirmDecision::Approved
            } else {
                wisp_tools::ConfirmDecision::Denied { feedback: None }
            }
        })
    }
    fn on_message(&self, message: &Message) {
        let _ = writeln!(
            self.trace.lock().unwrap(),
            "{}",
            serde_json::to_string(message).unwrap_or_default()
        );
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str, _: u64) {
        self.usage.lock().unwrap().tool_calls += 1;
        let failed_run = matches!(name, "run_in_context" | "get_run")
            && serde_json::from_str::<Value>(content)
                .ok()
                .is_some_and(|value| {
                    [value.get("status"), value.pointer("/run/status")]
                        .into_iter()
                        .flatten()
                        .any(|status| {
                            matches!(status.as_str(), Some("failed" | "cancelled" | "timed_out"))
                        })
                });
        if !ok || failed_run {
            self.failures.lock().unwrap().push(format!(
                "{name}: {}",
                content.chars().take(1000).collect::<String>()
            ));
        }
    }
    fn usage(
        &self,
        _: usize,
        input: u64,
        output: u64,
        _: u64,
        _: u64,
        _: usize,
        _: usize,
        _: wisp_core::ContextUsage,
    ) {
        let mut usage = self.usage.lock().unwrap();
        usage.input_tokens += input;
        usage.output_tokens += output;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::{ScriptedCompletion, ScriptedProvider, ScriptedToolCall};

    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wisp-workflow-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(path.join(".wisp/skills/bear-support-fixture/references"))
                .unwrap();
            std::fs::write(path.join(".wisp/skills/bear-support-fixture/SKILL.md"),
                "---\nname: bear-support-fixture\ndescription: Synthetic conversion fixture, not an upstream package.\n---\nCheck the search CLI, extract C1 claims, retrieve evidence, then write and verify reports. Read references/cli.md. Never invent citations.").unwrap();
            std::fs::write(path.join(".wisp/skills/bear-support-fixture/references/cli.md"),
                "Check sci --version; search with --mode low --limit 10; preserve empty results. Deliver report.md, report.html and references.bib.").unwrap();
            Self(path)
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct Env {
        root: PathBuf,
        approve: bool,
    }
    #[async_trait]
    impl ToolEnv for Env {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _: &str) -> bool {
            self.approve
        }
        async fn emit(&self, _: ToolEvent) {}
    }
    fn completion(value: Value) -> ScriptedCompletion {
        ScriptedCompletion {
            content: value.to_string(),
            ..Default::default()
        }
    }
    fn call(name: &str, args: Value) -> ScriptedCompletion {
        ScriptedCompletion {
            tool_calls: vec![ScriptedToolCall {
                name: name.into(),
                arguments: args,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    fn proposal() -> Value {
        let contract = json!({"type":"object","required":["summary","status","artifacts"],"properties":{
            "summary":{"type":"string"},"status":{"const":"succeeded"},"artifacts":{"type":"array","const":["report.md"]}
        }});
        json!({"goal":"Write and independently verify a fixture report", "context":"This is an offline fixture, not real literature evidence", "approval_policy":"review_all", "tasks":[
            {"id":"render","instruction":"Write fixture report.md", "depends_on":[],"capabilities":["project_write"],"skill_ids":[],"isolated":false,"output_schema":contract},
            {"id":"verify","instruction":"Read report.md and verify the fixture marker", "depends_on":["render"],"capabilities":["project_read"],"skill_ids":[],"isolated":false,"output_schema":contract}
        ]})
    }
    async fn host(workspace: &Workspace, provider: ScriptedProvider) -> WorkflowHost {
        let store =
            wisp_runs::open_project_store(&workspace.0, crate::runs::CLI_PROJECT_ID, "test")
                .await
                .unwrap();
        let (registry, policy) = local_policy("fixture");
        WorkflowHost {
            store,
            skills: Arc::new(SkillIndex::load(&[workspace.0.join(".wisp/skills")])),
            factory: Arc::new(move || Box::new(provider.clone())),
            registry,
            policy,
            manager: RunManager::new(),
            max_context: 64000,
            max_iter: 8,
        }
    }
    async fn saved(host: &WorkflowHost) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let template = WorkflowTemplate {
            id: id.clone(),
            name: "fixture".into(),
            description: "fixture".into(),
            proposal: serde_json::from_value(proposal()).unwrap(),
            builtin: false,
        };
        host.store
            .set_setting(TEMPLATES, &serde_json::to_string(&vec![template]).unwrap())
            .await
            .unwrap();
        id
    }
    #[tokio::test]
    async fn cli_agent_calls_conversion_tool_and_generator_reads_full_references() {
        let workspace = Workspace::new();
        let generator = ScriptedProvider::new("fixture", vec![completion(proposal())]);
        let host = host(&workspace, generator.clone()).await;
        let mut tools = Registry::builtins();
        tools.add(Box::new(WorkflowTool {
            host: host.clone(),
            kind: Kind::Create,
        }));
        let caller = ScriptedProvider::new(
            "fixture",
            vec![
                call(
                    "create_workflow",
                    json!({"skill_name":"bear-support-fixture"}),
                ),
                completion(json!({"summary":"created"})),
            ],
        );
        let mut ctx = ContextManager::new(64000);
        agent_loop(
            &mut ctx,
            &caller,
            None,
            &tools,
            &workspace.0,
            &wisp_core::NullOutput,
            "Convert the fixture Skill into an independent Workflow",
            5,
            None,
        )
        .await
        .unwrap();
        let templates = host.templates().await.unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].proposal.tasks.len(), 2);
        assert!(templates[0]
            .proposal
            .tasks
            .iter()
            .all(|node| node.skill_ids.is_empty()));
        let requests = generator.snapshot();
        let raw = serde_json::to_string(&requests).unwrap();
        assert!(raw.contains("--mode low --limit 10"));
        assert!(raw.contains("references.bib"));
        assert!(host
            .store
            .get_setting(&format!("workflow_source:{}", templates[0].id))
            .await
            .unwrap()
            .is_some());
    }
    #[tokio::test]
    async fn workflow_runs_without_source_skill_and_filters_each_child_tools() {
        let workspace = Workspace::new();
        let done =
            json!({"summary":"fixture verified","status":"succeeded","artifacts":["report.md"]});
        let provider = ScriptedProvider::new(
            "fixture",
            vec![
                call(
                    "write",
                    json!({"path":"report.md","content":"fixture marker"}),
                ),
                completion(done.clone()),
                call("read", json!({"path":"report.md"})),
                completion(done),
            ],
        );
        let host = host(&workspace, provider.clone()).await;
        let id = saved(&host).await;
        std::fs::remove_dir_all(workspace.0.join(".wisp/skills")).unwrap();
        let result = host
            .execute(
                &json!({"workflow_id":id,"request":"Test the fixture"}),
                &Env {
                    root: workspace.0.clone(),
                    approve: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "succeeded", "{result}");
        assert_eq!(
            std::fs::read_to_string(workspace.0.join("report.md")).unwrap(),
            "fixture marker"
        );
        let snapshot = serde_json::to_value(provider.snapshot()).unwrap();
        let requests = snapshot["requests"].as_array().unwrap();
        assert!(requests[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("ASSIGNED TASK:\nWrite fixture report.md"));
        assert!(requests.last().unwrap()["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("ASSIGNED TASK:\nRead report.md and verify the fixture marker"));
        let first = requests[0]["tool_names"].to_string();
        let last = requests.last().unwrap()["tool_names"].to_string();
        assert!(first.contains("\"write\""));
        assert!(!last.contains("\"write\""));
        assert!(!last.contains("\"shell\""));
        assert!(!last.contains("use_skill"));
        assert!(requests.last().unwrap()["messages"]
            .to_string()
            .contains("dependency_results"));
        assert!(Path::new(result["trace_directory"].as_str().unwrap())
            .join("result.json")
            .is_file());
    }
    #[tokio::test]
    async fn missing_artifact_fails_and_blocks_downstream_even_with_success_json() {
        let workspace = Workspace::new();
        let provider = ScriptedProvider::new(
            "fixture",
            vec![completion(
                json!({"summary":"claimed success","status":"succeeded","artifacts":["report.md"]}),
            )],
        );
        let host = host(&workspace, provider.clone()).await;
        let id = saved(&host).await;
        let result = host
            .execute(
                &json!({"workflow_id":id,"request":"test"}),
                &Env {
                    root: workspace.0.clone(),
                    approve: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "failed");
        assert!(result
            .to_string()
            .contains("Declared artifact does not exist"));
        assert_eq!(provider.snapshot().requests.len(), 1);
    }
    #[tokio::test]
    async fn approval_denial_starts_no_nodes() {
        let workspace = Workspace::new();
        let provider = ScriptedProvider::new("fixture", vec![]);
        let host = host(&workspace, provider.clone()).await;
        let id = saved(&host).await;
        assert!(host
            .execute(
                &json!({"workflow_id":id,"request":"test"}),
                &Env {
                    root: workspace.0.clone(),
                    approve: false
                }
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("not approved"));
        assert!(provider.snapshot().requests.is_empty());
    }
    #[test]
    fn rejects_legacy_bindings_cycles_unavailable_tools_and_missing_contracts() {
        let (registry, policy) = local_policy("fixture");
        for (pointer, value) in [
            ("/tasks/0/skill_ids", json!(["bear-support"])),
            ("/tasks/0/depends_on", json!(["verify"])),
            ("/tasks/0/capabilities", json!(["literature_search"])),
            ("/tasks/0/output_schema", json!({"type":"object"})),
        ] {
            let mut value_proposal = proposal();
            *value_proposal.pointer_mut(pointer).unwrap() = value;
            let proposal = serde_json::from_value(value_proposal).unwrap();
            assert!(
                workflow_conversion::resolve(&proposal, "test", "", &registry, &policy).is_err(),
                "{pointer}"
            );
        }
    }
    #[test]
    fn rejects_escaped_or_non_file_artifacts() {
        let workspace = Workspace::new();
        let output_dir = workspace.0.join("snapshots");
        std::fs::create_dir(&output_dir).unwrap();
        for path in ["../outside", "/tmp/outside", ".wisp", "missing.md"] {
            assert!(
                validate_artifacts(&workspace.0, &output_dir, &json!({"artifacts":[path]}))
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn run_boundary_rejects_remote_and_detached_commands_before_launch() {
        let workspace = Workspace::new();
        let host = host(&workspace, ScriptedProvider::new("fixture", vec![])).await;
        let tool = LocalRunTool(wisp_runs::RunInContextTool::new(
            host.store,
            host.manager,
            crate::runs::CLI_PROJECT_ID.into(),
            None,
        ));
        let env = Env {
            root: workspace.0.clone(),
            approve: true,
        };
        for args in [
            json!({"context_id":"ssh:unapproved", "command":"sci --version", "wait_for_completion":true}),
            json!({"context_id":"local", "command":"sci --version", "wait_for_completion":false}),
        ] {
            assert!(!tool.run(&args, &env).await.success);
        }
    }

    #[tokio::test]
    async fn failed_command_cannot_be_hidden_by_success_summary() {
        struct FailureRunner;
        #[async_trait]
        impl wisp_runs::RunCommandRunner for FailureRunner {
            async fn run(
                &self,
                _: wisp_runs::RunCommand,
                _: std::time::Duration,
            ) -> Result<wisp_runs::RunCommandOutput, String> {
                Err("fixture CLI authentication unavailable".into())
            }
        }
        let workspace = Workspace::new();
        let provider = ScriptedProvider::new(
            "fixture",
            vec![
                call(
                    "run_in_context",
                    json!({"context_id":"local","command":"sci --version","wait_for_completion":true}),
                ),
                completion(
                    json!({"summary":"claimed success","status":"succeeded","artifacts":[]}),
                ),
            ],
        );
        let mut host = host(&workspace, provider).await;
        host.manager = RunManager::with_runner(Arc::new(FailureRunner));
        let mut value = proposal();
        value["tasks"][0]["capabilities"] = json!(["code_run"]);
        value["tasks"][0]["output_schema"]["properties"]["artifacts"]["const"] = json!([]);
        let template = WorkflowTemplate {
            id: "failed-cli".into(),
            name: "failure".into(),
            description: "fixture".into(),
            proposal: serde_json::from_value(value).unwrap(),
            builtin: false,
        };
        host.store
            .set_setting(TEMPLATES, &serde_json::to_string(&vec![template]).unwrap())
            .await
            .unwrap();
        let result = host
            .execute(
                &json!({"workflow_id":"failed-cli","request":"test"}),
                &Env {
                    root: workspace.0.clone(),
                    approve: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "failed");
        assert!(
            result.to_string().contains("failed tool/Run calls"),
            "{result}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn declared_symlink_cannot_escape_project() {
        let workspace = Workspace::new();
        let outside = Workspace::new();
        std::fs::write(outside.0.join("secret.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.0.join("secret.txt"), workspace.0.join("report.md"))
            .unwrap();
        assert!(validate_artifacts(
            &workspace.0,
            &workspace.0,
            &json!({"artifacts":["report.md"]})
        )
        .is_err());
    }

    #[tokio::test]
    async fn retry_reuses_verified_success_and_rejects_changed_files() {
        let workspace = Workspace::new();
        let done = json!({"summary":"verified","status":"succeeded","artifacts":["report.md"]});
        let provider = ScriptedProvider::new(
            "fixture",
            vec![
                call(
                    "write",
                    json!({"path":"report.md","content":"fixture marker"}),
                ),
                completion(done.clone()),
                completion(
                    json!({"summary":"verification failed","status":"failed","artifacts":[]}),
                ),
                call("read", json!({"path":"report.md"})),
                completion(done),
            ],
        );
        let host = host(&workspace, provider.clone()).await;
        let id = saved(&host).await;
        let env = Env {
            root: workspace.0.clone(),
            approve: true,
        };
        let first = host
            .execute(&json!({"workflow_id":id,"request":"test"}), &env)
            .await
            .unwrap();
        assert_eq!(first["status"], "failed");
        let retry = json!({"workflow_id":id,"retry_run_id":first["run_id"]});
        std::fs::write(workspace.0.join("report.md"), "tampered").unwrap();
        assert!(host
            .execute(&retry, &env)
            .await
            .unwrap_err()
            .to_string()
            .contains("Cached artifact changed"));
        assert_eq!(provider.snapshot().requests.len(), 3);
        std::fs::write(workspace.0.join("report.md"), "fixture marker").unwrap();
        let second = host.execute(&retry, &env).await.unwrap();
        assert_eq!(second["status"], "succeeded", "{second}");
        assert_eq!(second["reused_steps"], json!(["render"]));
        assert_eq!(provider.snapshot().requests.len(), 5);
        assert!(Path::new(first["trace_directory"].as_str().unwrap())
            .join("verify.jsonl")
            .is_file());
    }

    #[tokio::test]
    async fn nested_command_confirmation_is_forwarded_to_host_and_denial_prevents_launch() {
        struct CountingRunner(Arc<std::sync::atomic::AtomicUsize>);
        #[async_trait]
        impl wisp_runs::RunCommandRunner for CountingRunner {
            async fn run(
                &self,
                _: wisp_runs::RunCommand,
                _: std::time::Duration,
            ) -> Result<wisp_runs::RunCommandOutput, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err("fixture launch error".into())
            }
        }
        struct ApprovalEnv {
            root: PathBuf,
            messages: Mutex<Vec<String>>,
        }
        #[async_trait]
        impl ToolEnv for ApprovalEnv {
            fn project_root(&self) -> &Path {
                &self.root
            }
            async fn confirm(&self, message: &str) -> bool {
                let mut messages = self.messages.lock().unwrap();
                messages.push(message.into());
                messages.len() == 1 // approve plan, deny the actual flagged command
            }
            async fn emit(&self, _: ToolEvent) {}
        }
        let workspace = Workspace::new();
        let provider = ScriptedProvider::new(
            "fixture",
            vec![
                call(
                    "run_in_context",
                    json!({"context_id":"local","command":"echo format","wait_for_completion":true}),
                ),
                completion(json!({"summary":"denied","status":"failed","artifacts":[]})),
            ],
        );
        let mut host = host(&workspace, provider).await;
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        host.manager = RunManager::with_runner(Arc::new(CountingRunner(count.clone())));
        let mut p = proposal();
        p["tasks"][0]["capabilities"] = json!(["code_run"]);
        host.store
            .set_setting(
                TEMPLATES,
                &serde_json::to_string(&vec![WorkflowTemplate {
                    id: "approval".into(),
                    name: "approval".into(),
                    description: "test".into(),
                    proposal: serde_json::from_value(p).unwrap(),
                    builtin: false,
                }])
                .unwrap(),
            )
            .await
            .unwrap();
        let env = ApprovalEnv {
            root: workspace.0.clone(),
            messages: Mutex::new(vec![]),
        };
        let result = host
            .execute(&json!({"workflow_id":"approval","request":"test"}), &env)
            .await
            .unwrap();
        assert_eq!(result["status"], "failed");
        let messages = env.messages.lock().unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[1].contains("Dangerous command detected"));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
