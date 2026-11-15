mod acp_agents;
pub mod acp_bridge;
mod acp_turn;
mod aside;
mod assistant;
mod branch_merge;
mod browser;
mod browser_filters;
mod channels;
mod compact;
pub mod config;
mod debug_request;
mod decision_model;
mod delegation;
mod delegation_isolation;
mod demos;
pub mod desktop;
mod device_bridge;
mod excerpt;
mod experts;
mod exploration_workspace;
mod explorations;
mod export;
mod files;
mod follow_ups;
mod mandate_report;
mod mandates;
pub mod mcp;
mod memory_tidy;
mod method_search;
mod model_view;
mod network;
mod origin;
mod output;
mod paper_freeze;
mod papers;
mod pet;
mod plugins;
mod progress;
mod project_archive;
mod project_folder;
mod project_sync;
pub mod queue;
mod quick_actions;
mod recaps;
mod remote_access;
mod remote_dir;
mod research_archive;
mod review;
mod run_log;
mod runtime;
mod runtimes;
mod sample;
mod schedules;
mod session_import;
mod subagents;
mod subscriptions;
mod terminals;
#[cfg(test)]
mod tests;
mod toolchain;
mod tools_found;
mod trajectory;
mod turn_memory;
mod turn_undo;
mod updates;
mod workflows;

use anyhow::{bail, ensure, Context, Result};
use async_trait::async_trait;
use config::Config;
use output::Session;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{RwLock, Semaphore};
use wisp_dto::{native_conversations as dto, remote_access::RemoteRequest};
use wisp_store::Store;
use wisp_sync::DirectRemoteHost;

const PROJECT: &str = "server";

/// What a conversation is told while an exploration round froze its
/// project, as the WebView says it.
const EXPLORATION_FROZEN_NOTE: &str = "An active isolated exploration has frozen project state for possible promotion. This separate mainline conversation remains available for normal discussion and read-only inspection, but project-mutating tools are unavailable. Do not attempt to write files, run commands or jobs, change project records, or call mutating external tools until the exploration round finishes.";
const COMMANDS: &[&str] = &[
    "list_projects",
    "remote_sessions",
    "native_conversation_inbox",
    "native_conversation_create",
    "native_conversation_snapshot",
    "native_conversation_send",
    "native_conversation_stop",
    "native_conversation_approve",
    "native_conversation_seen",
    "native_conversation_panel_file_directory",
    "native_conversation_panel_file_read",
];

#[derive(Clone)]
pub struct Server(Arc<Inner>);

/// What a conversation is called after its first message: the message on
/// one line, its whitespace and line breaks collapsed, at most 80
/// characters. A title with a line break in it breaks every list it is in.
pub(crate) fn title_of(subject: &str) -> String {
    subject
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

struct Inner {
    config: Config,
    store: Store,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    runs: wisp_runs::RunManager,
    epoch: String,
    sequence: AtomicU64,
    turns: Arc<Semaphore>,
    shutting_down: AtomicBool,
    /// The bridge to the browser extension, when it is on.
    browser: browser::Slot,
    /// The tunnel to a relay for remote web access, when it is on.
    remote: remote_access::Tunnel,
    /// The loop that fires scheduled tasks was started.
    scheduling: AtomicBool,
    admission: RwLock<()>,
    _lock: File,
    models: ModelFactory,
    /// The model new turns use. A desktop can replace it while running.
    model: std::sync::RwLock<desktop::ModelState>,
    secrets: Arc<dyn desktop::SecretStore>,
    /// The favorites library: a database of its own, opened when a desktop
    /// first asks for it, so a headless host never creates one.
    library: tokio::sync::OnceCell<wisp_store::LibraryStore>,
    /// Connections to the MCP servers the user added.
    mcp: mcp::Connections,
    /// The install of Python, R and Node under the data directory.
    toolchain: toolchain::Slot,
    /// Wisp's tools for ACP agents, served once the first one starts.
    bridge: tokio::sync::OnceCell<Arc<acp_bridge::Bridge>>,
    /// Subscription sign-ins under way.
    sign_ins: subscriptions::Attempts,
    /// Chat apps that talk to Wisp, and the target they share.
    channels: channels::Channels,
    /// The StickS3 desk display's listener on the local network.
    device: device_bridge::Slot,
    /// Per project: what turns hold shared and exploration changes alone.
    project_locks: explorations::ProjectActivityLocks,
    /// A parent conversation's next turn resumes to handle a subagent's
    /// report: what it is told, whether dispatching is held, and the
    /// original request.
    subagent_resumes: std::sync::Mutex<HashMap<String, (String, bool, String)>>,
    /// Conversations a background batch's result is being delivered to.
    delivering: std::sync::Mutex<HashSet<String>>,
    /// Conversations whose running turn corrects what an automatic review
    /// found; the review after it is not corrected again.
    corrections: std::sync::Mutex<HashSet<String>>,
    /// Corrections to start and queues to move on, for the worker that does
    /// it, one after another: a turn's end cannot start the next turn
    /// itself.
    to_correct: tokio::sync::mpsc::UnboundedSender<AfterTurn>,
    sweeping_deliveries: AtomicBool,
    /// The daily recap run is going.
    recapping: AtomicBool,
    /// The memory tidy is running.
    tidying: AtomicBool,
    /// Serializes making the research assistant's project.
    assistant_lock: tokio::sync::Mutex<()>,
    /// What evaluates a method search's candidates in place of the local
    /// Python; set by tests only.
    method_search_evaluator:
        std::sync::RwLock<Option<Arc<dyn method_search::prepare::MethodSearchEvaluator>>>,
    /// Terminals drawn in the window.
    terminals: terminals::Terminals,
}

/// Where a turn ran and who started it, for what follows it.
#[derive(Clone, Copy)]
struct TurnPlace<'a> {
    project: &'a str,
    root: &'a str,
    origin: channels::TurnOrigin,
}

/// A correction an automatic review asked for: the conversation, what it
/// is told, and where and by whom its turn was started.
struct Correction {
    frame: String,
    prompt: String,
    project: String,
    root: String,
    origin: channels::TurnOrigin,
}

/// What the worker does after a turn: start the correction a review asked
/// for, or send the next message waiting in a conversation.
enum AfterTurn {
    Correct(Correction),
    Drain(String),
}

/// What runs a turn: Wisp's own agent (with the model its side requests
/// use), or the ACP agent the conversation was given.
enum Engine {
    Wisp(Box<wisp_core::Agent>, wisp_llm::ProviderConfig),
    Acp(desktop::AcpAgent, acp_turn::Offered),
}

type ModelFactory =
    Arc<dyn Fn(wisp_llm::ProviderConfig) -> Box<dyn wisp_llm::Provider> + Send + Sync>;

impl Server {
    pub async fn open(config: Config) -> Result<Self> {
        Self::open_with(
            config,
            Arc::new(wisp_llm::build),
            Arc::new(desktop::Keyring),
        )
        .await
    }

    /// Explicit local action; intentionally absent from the remote allowlist.
    /// The desktop's OS folder picker is the authority to register this root.
    pub async fn register_local_project(&self, root: &std::path::Path) -> Result<String> {
        let _admission = self.0.admission.write().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let root = tokio::fs::canonicalize(root).await?;
        ensure!(root.is_dir(), "Select a project directory");
        ensure!(
            !self.0.config.data.starts_with(&root) && !root.starts_with(&self.0.config.data),
            "Keep project files separate from the host data directory"
        );
        let path = root.to_str().context("Project path must be UTF-8")?;
        for (id, _, existing, ..) in self.0.store.list_projects().await? {
            if std::path::Path::new(&existing) == root {
                return Ok(id);
            }
        }
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Workspace");
        let id = uuid::Uuid::new_v4().to_string();
        self.0.store.create_project(&id, name, path).await?;
        Ok(id)
    }

    /// Makes a new project, as the WebView's New project does: a folder
    /// named after it in `parent` (made when it is not there), registered
    /// under `name` with its description, and, when asked for, the standard
    /// research layout (data, analysis, results, figures and more) with
    /// its manifest. A folder that is already a project is refused.
    pub async fn create_local_project(
        &self,
        parent: &std::path::Path,
        name: &str,
        description: &str,
        standard_layout: bool,
    ) -> Result<String> {
        let name = name.trim();
        let description = description.trim();
        ensure!(!name.is_empty(), "Give the project a name");
        ensure!(
            name.chars().count() <= desktop::PROJECT_NAME_LIMIT
                && !name.chars().any(char::is_control),
            "A project name is one line of at most {} characters",
            desktop::PROJECT_NAME_LIMIT
        );
        ensure!(
            description.chars().count() <= desktop::PROJECT_DESCRIPTION_LIMIT,
            "Keep the description under {} characters",
            desktop::PROJECT_DESCRIPTION_LIMIT
        );
        let folder = desktop::project_folder_name(name);
        ensure!(
            !folder.is_empty(),
            "Give the project a name a folder can have"
        );
        let parent = tokio::fs::canonicalize(parent).await?;
        let root = parent.join(&folder);
        for (_, _, existing, ..) in self.0.store.list_projects().await? {
            ensure!(
                std::path::Path::new(&existing) != root,
                "This folder is already registered as a project."
            );
        }
        tokio::fs::create_dir_all(&root)
            .await
            .context("Could not make the project's folder")?;
        let id = self.register_local_project(&root).await?;
        if standard_layout {
            let (folder, layout_id, layout_name) = (root.clone(), id.clone(), name.to_owned());
            tokio::task::spawn_blocking(move || {
                project_archive::workspace_manifest::init_workspace_layout(
                    &folder,
                    &layout_id,
                    &layout_name,
                )
            })
            .await?
            .map_err(anyhow::Error::msg)?;
        }
        self.0.store.update_project(&id, name, description).await?;
        Ok(id)
    }

    #[cfg(test)]
    async fn open_with_models(config: Config, models: ModelFactory) -> Result<Self> {
        Self::open_with(config, models, Arc::new(desktop::MemorySecrets::default())).await
    }

    async fn open_with(
        mut config: Config,
        models: ModelFactory,
        secrets: Arc<dyn desktop::SecretStore>,
    ) -> Result<Self> {
        tokio::fs::create_dir_all(&config.data).await?;
        config.data = tokio::fs::canonicalize(&config.data).await?;
        tokio::fs::create_dir_all(&config.workspace).await?;
        config.workspace = tokio::fs::canonicalize(&config.workspace).await?;
        ensure!(
            !config.data.starts_with(&config.workspace),
            "Keep WISP_SERVER_DATA outside the public project workspace"
        );
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.data.join("server.lock"))?;
        lock.try_lock()
            .context("Another wisp-server already owns this data directory")?;
        let store = Store::open(&config.data.join("wisp.sqlite")).await?;
        // The built-in project follows the configured workspace; its name is
        // the user's once they have changed it.
        let name = store
            .get_project(PROJECT)
            .await?
            .map(|(name, _)| name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Server".into());
        store
            .create_project(PROJECT, &name, &config.workspace.to_string_lossy())
            .await?;
        for (id, ..) in store.list_projects().await? {
            store.interrupt_server_turns(&id).await?;
        }
        // A promotion cut off mid-way is rolled back or finished before any
        // tool can touch a workspace, and disposed workspaces are swept.
        explorations::promotion::recover_incomplete_promotions(&store, &config.data).await;
        if let Err(error) =
            exploration_workspace::PersistentExplorationWorkspace::new(config.data.clone())
                .sweep_disposal_quarantine()
        {
            tracing::warn!(%error, "Could not sweep quarantined exploration workspaces");
        }
        // A delegated batch cut off by the app closing ends as failed; its
        // tasks are not started again.
        if let Err(error) = store.recover_interrupted_agent_workflows().await {
            tracing::warn!(%error, "Could not settle interrupted delegated batches");
        }
        // A method search the app closed on waits paused, before the Runs
        // are reconciled, which would otherwise mark it lost.
        if let Err(error) = store.recover_interrupted_method_search_runs().await {
            tracing::warn!(%error, "Could not pause interrupted method searches");
        }
        let runs = wisp_runs::RunManager::new();
        runs.recover(&store).await.map_err(anyhow::Error::msg)?;
        let routes = network::load(&store).await;
        // Programs the tools start take their route from here; the route is
        // one per process, as in the WebView desktop.
        wisp_tools::network::set_command_proxy(&routes.command_proxy_url);
        // Tools installed under the data directory lead PATH for the
        // commands and runs this host starts, as in the WebView desktop.
        wisp_tools::local_env::set_managed_path(toolchain::Layout::new(&config.data).path_dirs());
        let mcp = mcp::Connections::default();
        mcp.set_proxy(&routes.mcp_proxy_url);
        let model = match config.model.take() {
            Some(model) => desktop::ModelState::environment(model),
            None => desktop::ModelState::saved(&store, secrets.as_ref()).await,
        };
        let (to_correct, corrections_asked) = tokio::sync::mpsc::unbounded_channel();
        let server = Self(Arc::new(Inner {
            config,
            store,
            sessions: Default::default(),
            runs,
            epoch: uuid::Uuid::new_v4().to_string(),
            sequence: AtomicU64::new(0),
            turns: Arc::new(Semaphore::new(4)),
            shutting_down: AtomicBool::new(false),
            browser: Default::default(),
            remote: Default::default(),
            scheduling: AtomicBool::new(false),
            admission: RwLock::new(()),
            _lock: lock,
            models,
            model: std::sync::RwLock::new(model),
            secrets,
            library: Default::default(),
            mcp,
            toolchain: Default::default(),
            bridge: Default::default(),
            sign_ins: Default::default(),
            channels: Default::default(),
            device: Default::default(),
            project_locks: Default::default(),
            subagent_resumes: Default::default(),
            delivering: Default::default(),
            corrections: Default::default(),
            to_correct,
            sweeping_deliveries: AtomicBool::new(false),
            recapping: AtomicBool::new(false),
            tidying: AtomicBool::new(false),
            assistant_lock: Default::default(),
            method_search_evaluator: Default::default(),
            terminals: Default::default(),
        }));
        server.start_corrections(corrections_asked);
        Ok(server)
    }

    /// Starts the corrections automatic reviews ask for and sends queued
    /// messages, one after another, for as long as the host is there.
    fn start_corrections(&self, mut asked: tokio::sync::mpsc::UnboundedReceiver<AfterTurn>) {
        let host = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            while let Some(next) = asked.recv().await {
                let Some(inner) = host.upgrade() else {
                    break;
                };
                match next {
                    AfterTurn::Correct(correction) => Self(inner).correct(correction).await,
                    AfterTurn::Drain(session) => Self(inner).drain(&session).await,
                }
            }
        });
    }

    /// Resumes a conversation to correct what its automatic review found.
    /// The findings are told to that turn only, as the WebView injects
    /// them; nothing is added to the conversation as said by the user.
    async fn correct(&self, correction: Correction) {
        let frame = correction.frame;
        self.0
            .subagent_resumes
            .lock()
            .unwrap()
            .insert(frame.clone(), (correction.prompt, false, String::new()));
        self.0.corrections.lock().unwrap().insert(frame.clone());
        let request = dto::SendRequest {
            session_id: frame.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message: String::new(),
            resume: true,
            resume_after: None,
            attachments: Vec::new(),
            references: Vec::new(),
            acp_agent_id: None,
        };
        if let Err(error) = self
            .start_turn(
                &correction.project,
                &correction.root,
                request,
                correction.origin,
            )
            .await
        {
            self.0.subagent_resumes.lock().unwrap().remove(&frame);
            self.0.corrections.lock().unwrap().remove(&frame);
            tracing::warn!(%error, "Could not correct what the review found");
        }
    }

    /// Model a conversation that chose `profile` runs on (an empty id is the
    /// default); empty until one is configured.
    fn model_id(&self, profile: &str) -> String {
        self.0.model.read().unwrap().model_id(profile)
    }

    fn session(&self, id: &str) -> Arc<Session> {
        self.0
            .sessions
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .clone()
    }

    fn activity(&self) -> (HashSet<String>, HashSet<String>) {
        let mut running = HashSet::new();
        let mut awaiting = HashSet::new();
        for (id, session) in self.0.sessions.lock().unwrap().iter() {
            let live = session.live.lock().unwrap();
            if live.running {
                running.insert(id.clone());
            }
            if !live.approvals.is_empty() {
                awaiting.insert(id.clone());
            }
        }
        (running, awaiting)
    }

    async fn sessions(&self, project: Option<&str>) -> Result<Value> {
        Ok(serde_json::to_value(
            self.session_rows(project, "", false).await?,
        )?)
    }

    /// Recent conversations, or keyword matches when `query` is not empty.
    /// Only a keyword search may include archived conversations.
    async fn session_rows(
        &self,
        project: Option<&str>,
        query: &str,
        include_archived: bool,
    ) -> Result<Vec<wisp_dto::SessionSearchInfo>> {
        let (running, awaiting) = self.activity();
        let mut rows = Vec::new();
        for row in self
            .0
            .store
            .search_sessions_with_options(
                project,
                query,
                100,
                None,
                project,
                &[],
                wisp_store::SessionSearchOptions {
                    include_shelved: include_archived,
                },
            )
            .await?
        {
            let status = if awaiting.contains(&row.id) {
                "needs_you"
            } else if running.contains(&row.id) {
                "running"
            } else if row.unseen && row.last_role.as_deref() == Some("assistant") {
                "needs_you"
            } else {
                "complete"
            };
            rows.push(wisp_dto::SessionSearchInfo {
                id: row.id,
                project_id: row.project_id,
                project_name: row.project_name,
                title: row.title,
                ts: row.created_at,
                activity_at: row.activity_at,
                status: status.into(),
                shelved: row.shelved,
            });
        }
        Ok(rows)
    }

    async fn execute(&self, request: RemoteRequest) -> Result<Value> {
        ensure!(
            COMMANDS.contains(&request.command.as_str()) && request.args.is_object(),
            "Unsupported command"
        );
        if request.command == "list_projects" {
            let (running, awaiting) = self.activity();
            let projects =
                wisp_app::projects::list_projects(&self.0.store, &running, &awaiting).await?;
            return Ok(serde_json::to_value(
                wisp_app::projects::available_remote_projects(&self.0.store, projects).await?,
            )?);
        }
        if request.command == "native_conversation_inbox" {
            let mut rows = self.sessions(None).await?;
            rows.as_array_mut()
                .unwrap()
                .retain(|row| row["status"] == "needs_you");
            return Ok(rows);
        }
        let project = request
            .project_id
            .as_deref()
            .context("A project is required")?;
        let (_, root) = self
            .0
            .store
            .get_project(project)
            .await?
            .context("Project not found")?;
        if request.command == "remote_sessions" {
            return self.sessions(Some(project)).await;
        }
        if request.command == "native_conversation_create" {
            let args: dto::CreateRequest = serde_json::from_value(request.args)?;
            ensure!(
                args.acp_agent_id.is_none(),
                "ACP sessions are not supported by this server"
            );
            let id = uuid::Uuid::new_v4().to_string();
            self.0
                .store
                .create_frame(&id, project, "main", &self.model_id(""))
                .await?;
            self.0
                .store
                .rename_session(&id, project, "New conversation")
                .await?;
            return Ok(id.into());
        }
        let id = request
            .args
            .get("session_id")
            .and_then(Value::as_str)
            .context("A conversation is required")?;
        ensure!(
            self.0.store.live_frame_project_id(id).await?.as_deref() == Some(project),
            "Conversation does not belong to the selected project"
        );
        match request.command.as_str() {
            "native_conversation_snapshot" => {
                let args: dto::SessionRequest = serde_json::from_value(request.args)?;
                self.snapshot(project, &args).await
            }
            "native_conversation_send" => {
                let args: dto::SendRequest = serde_json::from_value(request.args)?;
                // Files and references are a desktop-only way to send.
                ensure!(
                    args.attachments.is_empty() && args.references.is_empty(),
                    "This server currently accepts text messages"
                );
                self.start(project, &root, args).await
            }
            "native_conversation_stop" => {
                let args: dto::SessionRequest = serde_json::from_value(request.args)?;
                self.session(&args.session_id).stop();
                Ok(Value::Null)
            }
            "native_conversation_approve" => {
                let args: dto::ApprovalRequest = serde_json::from_value(request.args)?;
                ensure!(
                    args.scope == dto::ApprovalScope::Once,
                    "Remote approvals apply once only"
                );
                let session = self.session(&args.session_id);
                let (_, sender) = session
                    .live
                    .lock()
                    .unwrap()
                    .approvals
                    .remove(&args.approval_id)
                    .context("Approval expired or already answered")?;
                sender
                    .send(if args.approved {
                        wisp_tools::ConfirmDecision::Approved
                    } else {
                        wisp_tools::ConfirmDecision::Denied {
                            feedback: args.feedback,
                        }
                    })
                    .map_err(|_| anyhow::anyhow!("Approval expired"))?;
                Ok(Value::Null)
            }
            "native_conversation_seen" => {
                self.0.store.mark_frame_seen(id).await?;
                Ok(Value::Null)
            }
            "native_conversation_panel_file_directory" | "native_conversation_panel_file_read" => {
                let args: wisp_dto::native_files::Request = serde_json::from_value(request.args)?;
                files::read(project, &root, &request.command, args).await
            }
            _ => bail!("Unsupported command"),
        }
    }

    async fn start(&self, project: &str, root: &str, request: dto::SendRequest) -> Result<Value> {
        self.start_turn(project, root, request, channels::TurnOrigin::Desktop)
            .await
    }

    /// Starts a turn. One a chat app or a lab member's mail started asks
    /// before every tool that changes something, whatever the conversation
    /// allows.
    async fn start_turn(
        &self,
        project: &str,
        root: &str,
        mut request: dto::SendRequest,
        origin: channels::TurnOrigin,
    ) -> Result<Value> {
        ensure!(
            request.attachments.len() <= 20 && request.references.len() <= 20,
            "Attach at most 20 files and 20 references to one message"
        );
        for path in &request.attachments {
            files::attachable(std::path::Path::new(root), path).await?;
        }
        // What the turn is named after, before the file list is folded in.
        let subject = match request.message.trim() {
            "" => request.attachments.first().cloned().unwrap_or_default(),
            text => text.to_owned(),
        };
        // The same block the WebView saves, so a reloaded transcript shows
        // the files and the model sees their project-relative paths.
        request.message = dto::message_with_attachments(&request.message, &request.attachments);
        // A resumed turn carries no new message.
        ensure!(
            (request.resume || !request.message.trim().is_empty())
                && request.message.len() <= 128 * 1024,
            "Message must contain 1–131072 bytes"
        );
        ensure!(
            !request.request_id.trim().is_empty() && request.request_id.len() <= 128,
            "Invalid request ID"
        );
        let session = self.session(&request.session_id);
        let _accept = session.accept.lock().await;
        let _admission = self.0.admission.read().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Server is shutting down"
        );
        // A conversation whose research was archived is a record: it is
        // read, and work goes on in a new one.
        ensure!(
            !self.archived(&request.session_id).await?,
            "This conversation's research is archived and it is read-only; continue in a new conversation"
        );
        // An exploration's turns work in its own copy of the project. The
        // source of an exploration round is frozen; other conversations in
        // the project keep to tools that change nothing until it ends.
        let (working, scope) = explorations::service::working_project_in(
            &self.0.store,
            &self.0.config.data,
            &request.session_id,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let write_locked = explorations::service::conversation_project_write_locked(
            &self.0.store,
            &scope,
            Some(&request.session_id),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let working_root = working.root.to_string_lossy().into_owned();
        let in_exploration = matches!(scope, wisp_store::StateScope::Exploration { .. });
        let root: &str = if in_exploration { &working_root } else { root };
        let mut notes = Vec::new();
        if let Some(note) =
            explorations::service::exploration_runtime_injection(std::path::Path::new(root), &scope)
                .map_err(anyhow::Error::msg)?
        {
            notes.push(note);
        }
        if write_locked {
            notes.push(EXPLORATION_FROZEN_NOTE.to_owned());
        }
        // The research assistant's conversation never ends, so each turn
        // is told the date and time.
        if assistant::is_assistant(&request.session_id) {
            notes.push(assistant::now_note());
        }
        let isolation = wisp_runs::exploration_isolation::boundary_for_scope(&self.0.store, &scope)
            .await
            .map_err(anyhow::Error::msg)?;
        // Read once here: a change made while the turn runs applies to the next.
        let options = self.stored_options(&request.session_id).await?;
        // A conversation an ACP agent runs needs no model of Wisp's.
        let acp = acp_turn::agent_of(&self.0.store, &request.session_id).await?;
        let model = match &acp {
            Some(_) => None,
            None => {
                // An expert with a model of its own runs on it.
                let profile = self.turn_profile(&options).await;
                let mut model = self.0.model.read().unwrap().pick(&profile)?;
                // Priority processing, where the model's service offers it.
                if options.fast && desktop::fast_available(&model.provider) {
                    model.provider.service_tier = Some(desktop::FAST_TIER.into());
                }
                // A subscription's token is refreshed before the turn.
                self.fresh_subscription(&mut model, &self.provider_of(&profile))
                    .await?;
                Some(model)
            }
        };
        let request_json = serde_json::to_string(&request)?;
        let reply = serde_json::to_value(dto::SendAccepted {
            request_id: request.request_id.clone(),
            session_id: request.session_id.clone(),
            epoch: self.0.epoch.clone(),
        })?;
        if let Some(saved) = self
            .0
            .store
            .server_turn_request(&request.session_id, &request.request_id)
            .await?
        {
            ensure!(
                saved == request_json,
                "Request ID was already used with different input"
            );
            return Ok(reply);
        }
        ensure!(
            !session.live.lock().unwrap().running,
            "A turn is already running in this conversation"
        );
        // Held while the turn runs, so an exploration is not started or
        // ended under it.
        let activity = self
            .0
            .project_locks
            .project(project)
            .try_read_owned()
            .map_err(|_| {
                anyhow::anyhow!(
                    "This project is busy. Try again when the current project operation finishes."
                )
            })?;
        let permit = self
            .0
            .turns
            .clone()
            .try_acquire_owned()
            .context("Server is busy; at most four turns can run concurrently")?;
        // Only the desktop's own turns (and its subagents') may run with the
        // conversation's full access.
        // A mandate's conversation is given its brief and tools on every
        // turn; a message the researcher wrote answers its open request.
        let mandate = mandates::turn_context(
            &self.0.store,
            project,
            &request.session_id,
            (!request.resume).then_some(request.message.as_str()),
        )
        .await;
        let mandate_reviews = mandate
            .as_ref()
            .is_some_and(|mandate| mandate.review_mutations);
        let full_access = options.full_access
            && !mandate_reviews
            && matches!(
                origin,
                channels::TurnOrigin::Desktop
                    | channels::TurnOrigin::Subagent { unattended: false }
            );
        let lab_mail = origin == channels::TurnOrigin::LabMail;
        // A subagent's conversation takes instructions only from its parent;
        // a parent resumed for a subagent's report is told about it.
        let dispatched_from = self.subagent_parent(&request.session_id).await;
        ensure!(
            dispatched_from.is_none()
                || request.resume
                || matches!(origin, channels::TurnOrigin::Subagent { .. }),
            "This subagent conversation only takes instructions from the conversation that started it."
        );
        let resumed = if request.resume {
            self.0
                .subagent_resumes
                .lock()
                .unwrap()
                .remove(&request.session_id)
        } else {
            None
        };
        let mut notes = notes;
        let subagents = match (&dispatched_from, resumed) {
            (Some(_), _) => None,
            (None, Some((prompt, held, authorization))) => {
                notes.push(prompt);
                Some(subagents::SubagentSetup {
                    authorization,
                    held,
                    unattended: origin != channels::TurnOrigin::Desktop,
                })
            }
            (None, None) => Some(subagents::SubagentSetup {
                authorization: request.message.clone(),
                held: false,
                unattended: origin != channels::TurnOrigin::Desktop,
            }),
        };
        let auto_review = options.auto_review;
        let plan_mode = options.plan_first;
        let engine = match (acp, model) {
            (Some(agent), _) => Engine::Acp(
                agent,
                self.bridge_offer(project, root, &request.session_id)
                    .await?,
            ),
            (None, Some(model)) => {
                let mut notes = notes;
                if let Some(mandate) = &mandate {
                    notes.push(mandate.brief.clone());
                }
                let mut agent = self
                    .wisp_agent(
                        project, root, &request, &session, &options, &model, lab_mail, notes,
                        subagents,
                    )
                    .await?;
                if let Some(mandate) = &mandate {
                    for tool in self.mandate_tools(&mandate.mandate_id) {
                        agent.add_tool(tool);
                    }
                }
                Engine::Wisp(Box::new(agent), model.provider.clone())
            }
            (None, None) => unreachable!("a model is picked when no ACP agent runs"),
        };
        let page = self
            .0
            .store
            .load_session_transcript_page(&request.session_id, None, 20)
            .await?;
        self.0
            .store
            .accept_server_turn(&request.session_id, &request.request_id, &request_json)
            .await?;
        // The project's latest conversation, for the chat apps' target.
        if let Err(error) = self.record_last_message_session(&request.session_id).await {
            tracing::warn!(%error, "Could not record the latest conversation");
        }
        // A conversation a lab member's mail or a parent's dispatch opened
        // keeps the name it was given, and so does the research assistant's.
        let keep_name = lab_mail
            || matches!(origin, channels::TurnOrigin::Subagent { .. })
            || assistant::is_assistant(&request.session_id);
        if !keep_name && self.0.store.max_message_seq(&request.session_id).await? == 0
            || !keep_name
                && matches!(&engine, Engine::Wisp(agent, _) if !agent
                .ctx
                .messages
                .iter()
                .any(|msg| msg.role == wisp_llm::Role::User))
        {
            let title = title_of(&subject);
            if let Err(error) = self
                .0
                .store
                .rename_session(&request.session_id, project, &title)
                .await
            {
                tracing::warn!(%error, "Could not name the new conversation");
            }
        }
        self.begin_turn(&session, page);
        let server = self.clone();
        let (project_of_turn, root_of_turn) = (project.to_owned(), root.to_owned());
        let session = session.clone();
        let project = project.to_string();
        let root = std::path::PathBuf::from(root);
        tokio::spawn(async move {
            let inner = server.clone();
            let running = session.clone();
            let input = request.clone();
            let (task, side_model) = match engine {
                Engine::Wisp(agent, side_model) => (
                    tokio::spawn(async move {
                        runtime::run(
                            inner.0.store.clone(),
                            *agent,
                            running,
                            project,
                            input,
                            full_access,
                            plan_mode,
                            root,
                            runtime::TurnScope {
                                lab_mail,
                                write_locked,
                                isolation,
                                state: scope,
                            },
                        )
                        .await
                    }),
                    Some(side_model),
                ),
                Engine::Acp(agent, offered) => (
                    tokio::spawn(async move {
                        acp_turn::run(
                            inner.0.store.clone(),
                            running,
                            project,
                            input,
                            full_access,
                            root,
                            agent,
                            offered,
                        )
                        .await
                    }),
                    None,
                ),
            };
            server
                .end_turn(
                    &request,
                    &session,
                    task,
                    permit,
                    activity,
                    side_model,
                    auto_review,
                    TurnPlace {
                        project: &project_of_turn,
                        root: &root_of_turn,
                        origin,
                    },
                )
                .await;
        });
        Ok(reply)
    }

    /// The bridge, started the first time an ACP agent needs it.
    async fn bridge(&self) -> Result<Arc<acp_bridge::Bridge>> {
        self.0
            .bridge
            .get_or_try_init(acp_bridge::Bridge::start)
            .await
            .cloned()
    }

    /// Wisp's science tools for the ACP agent of a conversation, as of
    /// this turn: what the bridge is to offer it.
    pub(crate) async fn bridge_offer(
        &self,
        project: &str,
        root: &str,
        frame: &str,
    ) -> Result<acp_turn::Offered> {
        let (tools, hidden) = runtime::bridge_tools(
            &self.0.config,
            &self.0.store,
            self.0.runs.clone(),
            project,
            std::path::Path::new(root),
            frame,
            &self.service_keys().await,
            self.mcp_tools().await,
        )
        .await;
        Ok(acp_turn::Offered {
            bridge: self.bridge().await?,
            tools,
            hidden,
        })
    }

    /// The Wisp agent that runs a turn, with its model, tools and context.
    async fn wisp_agent(
        &self,
        project: &str,
        root: &str,
        request: &dto::SendRequest,
        session: &Arc<Session>,
        options: &desktop::ConversationOptions,
        model: &config::ModelConfig,
        lab_mail: bool,
        notes: Vec<String>,
        subagents: Option<subagents::SubagentSetup>,
    ) -> Result<wisp_core::Agent> {
        let subagent = self.subagent_model_for(&request.session_id, model).await;
        // A lab member's agent can list the lab and write to its members.
        let member = self.is_lab_member().await;
        // A turn another member's mail started reads only its own project,
        // so it is not pointed at the knowledge base.
        let knowledge_base = match member && !lab_mail {
            true => self.lab_knowledge_base_root().await,
            false => None,
        };
        let lab = runtime::LabTurn {
            context: member.then(|| channels::lab_mail::turn_context(knowledge_base.as_deref())),
            mail: lab_mail,
            notes,
        };
        let mut agent = runtime::build(
            &self.0.config,
            model,
            &self.0.store,
            self.0.runs.clone(),
            project,
            root,
            &request.session_id,
            &session,
            &self.0.models,
            (!options.reasoning_effort.is_empty()).then(|| options.reasoning_effort.clone()),
            &request.references,
            options.plan_first,
            &self.service_keys().await,
            self.mcp_tools().await,
            &subagent,
            self.decision_model_in_use().await,
            self.0.browser.bridge(),
            lab,
        )
        .await?;
        // A model that cannot see is told what attached pictures show by
        // one that can, as the WebView's image model does.
        if !model.supports_vision {
            if let Some(reader) = self.picture_reader().await {
                let mut cfg = reader.provider.with_session_id(&request.session_id);
                let network = network::load(&self.0.store).await;
                cfg.proxy = network::route(&network.model_proxy_url);
                agent.vision_provider = Some((self.0.models)(cfg));
            }
        }
        // A top-level conversation can run subagents; a subagent cannot,
        // and asks its parent rather than the researcher.
        match subagents {
            Some(setup) => {
                for tool in self.subagent_tools(project, &request.session_id, setup) {
                    agent.add_tool(tool);
                }
                // And hand bounded work to temporary agents.
                for tool in self
                    .delegation_tools(project, root, &request.session_id)
                    .await
                {
                    agent.add_tool(tool);
                }
            }
            None => {
                let keep: Vec<String> = agent
                    .tools
                    .names()
                    .into_iter()
                    .filter(|name| !subagents::WITHHELD.contains(name))
                    .map(str::to_owned)
                    .collect();
                let tools = std::mem::replace(&mut agent.tools, wisp_tools::Registry::builtins());
                agent.tools = tools.filtered(&keep);
            }
        }
        // The research assistant keeps to its own tools and prompt.
        if assistant::is_assistant(&request.session_id) {
            self.shape_assistant(&mut agent).await?;
            return Ok(agent);
        }
        if member {
            agent.add_tool(Box::new(channels::lab_mail::LabMembersTool {
                server: self.clone(),
            }));
            agent.add_tool(Box::new(channels::lab_mail::LabSendTool {
                server: self.clone(),
                frame: request.session_id.clone(),
            }));
        }
        Ok(agent)
    }

    /// Marks the conversation's turn as running, with what it had so far.
    fn begin_turn(&self, session: &Session, page: wisp_store::SessionTranscriptPage) {
        let mut live = session.live.lock().unwrap();
        live.running = true;
        live.guidable = true;
        session.guidance.lock().unwrap().clear();
        live.generation = live.generation.wrapping_add(1);
        live.error = None;
        live.usage = None;
        // They were about the answer before this turn.
        live.follow_ups.clear();
        live.suggesting = false;
        live.review = None;
        live.reviewing = false;
        live.memory_proposal = None;
        live.proposing = false;
        live.stop_reason = None;
        session.suggested.notify_waiters();
        live.calls.clear();
        live.provenance.clear();
        live.reasoning.clear();
        live.written.clear();
        live.draft.clear();
        live.messages = page.messages.into_iter().map(|(_, msg)| msg).collect();
        live.next_before_seq = page.next_before_seq;
        session.cancel.store(false, Ordering::Relaxed);
    }

    /// Records how a turn ended and frees its place; after a Wisp turn
    /// that answered, asks what could come next and reviews it.
    async fn end_turn(
        &self,
        request: &dto::SendRequest,
        session: &Arc<Session>,
        task: tokio::task::JoinHandle<Result<()>>,
        permit: tokio::sync::OwnedSemaphorePermit,
        activity: tokio::sync::OwnedRwLockReadGuard<()>,
        side_model: Option<wisp_llm::ProviderConfig>,
        auto_review: bool,
        place: TurnPlace<'_>,
    ) {
        let mut error = match task.await {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(network::explain(&self.0.store, &format!("{e:#}")).await),
            Err(_) => Some("Agent task stopped unexpectedly".into()),
        };
        if let Err(e) = self
            .0
            .store
            .finish_server_turn(&request.session_id, &request.request_id, error.as_deref())
            .await
        {
            error = Some(format!("Could not save turn completion: {e}"));
        }
        // The turn's work is done: the project is free before anyone sees
        // the turn over, so an exploration can start from its answer at
        // once, while follow-ups and the review are still being asked for.
        drop(activity);
        let answered = error.is_none() && side_model.is_some();
        let ended = {
            let mut live = session.live.lock().unwrap();
            live.running = false;
            live.guidable = false;
            live.generation = live.generation.wrapping_add(1);
            live.approvals.clear();
            live.messages.clear();
            live.draft.clear();
            live.reasoning.clear();
            live.error = error;
            // Set with the turn's end, so whoever sees the turn over
            // can wait for the questions without missing them.
            live.suggesting = answered;
            live.reviewing = answered && auto_review;
            live.proposing = answered;
            live.generation
        };
        // The turn is over: what follows is not one of the four.
        drop(permit);
        let frame = &request.session_id;
        match (answered, side_model) {
            (true, Some(side_model)) => {
                tokio::join!(
                    self.suggest_follow_ups(frame, session, side_model.clone(), ended),
                    self.propose_memory_after_turn(frame, session, side_model.clone(), ended),
                    async {
                        // A correction the review asks for goes before
                        // what was queued, as the WebView's review runs
                        // before its queue moves.
                        self.review_after_turn(
                            frame,
                            session,
                            side_model,
                            ended,
                            auto_review,
                            place,
                        )
                        .await;
                        self.drain_later(frame);
                    },
                );
            }
            _ => self.drain_later(frame),
        }
    }

    /// Has the turn that just ended reviewed, when its conversation asks
    /// for that and the turn has something to check, and keeps the report
    /// for the conversation's status unless a newer turn began meanwhile.
    async fn review_after_turn(
        &self,
        frame: &str,
        session: &Session,
        model: wisp_llm::ProviderConfig,
        ended: u64,
        wanted: bool,
        place: TurnPlace<'_>,
    ) {
        // The turn that ended corrected an earlier review: this one only
        // checks the correction.
        let follow_up = self.0.corrections.lock().unwrap().remove(frame);
        if !wanted {
            return;
        }
        let found = async {
            anyhow::ensure!(
                !self.0.shutting_down.load(Ordering::Relaxed),
                "The app is closing"
            );
            let numbered = self.numbered_transcript(frame).await?;
            let through = numbered.last().map_or(0, |(seq, _)| *seq);
            let messages: Vec<_> = numbered.into_iter().map(|(_, message)| message).collect();
            if !review::worth_reviewing(&messages) {
                return Ok(None);
            }
            let reviewer = self.reviewer(model).await;
            let review = review::ask(reviewer.as_ref(), &messages).await?;
            self.keep_review(frame, through, &review).await;
            Ok(Some(review))
        }
        .await;
        let found = match found {
            Ok(found) => found.map(Ok),
            Err(error) => {
                let said = network::explain(&self.0.store, &format!("{error:#}")).await;
                Some(Err(said))
            }
        };
        // What was found is corrected once, as the WebView does: the turn
        // is resumed with the findings, and what it then says is reviewed.
        let to_correct = match &found {
            Some(Ok(review)) if !review.findings.is_empty() && !follow_up => Some(review.clone()),
            _ => None,
        };
        let current = {
            let mut live = session.live.lock().unwrap();
            let current = live.generation == ended;
            if current {
                live.review = found;
                live.reviewing = false;
            }
            current
        };
        session.suggested.notify_waiters();
        if let (true, Some(review)) = (current, to_correct) {
            if !self.0.shutting_down.load(Ordering::Relaxed) {
                let _ = self.0.to_correct.send(AfterTurn::Correct(Correction {
                    frame: frame.to_owned(),
                    prompt: review.correction(),
                    project: place.project.to_owned(),
                    root: place.root.to_owned(),
                    origin: place.origin,
                }));
            }
        }
    }

    /// Whether a conversation's research has been frozen as an archive.
    pub(crate) async fn archived(&self, session: &str) -> Result<bool> {
        Ok(self
            .0
            .store
            .research_archive(session)
            .await?
            .is_some_and(|archive| archive.frozen_at.is_some()))
    }

    /// Asks the conversation's model what could be asked next and keeps
    /// the answer for the snapshot, unless a newer turn began meanwhile.
    /// A failure costs the user nothing but the suggestions.
    async fn suggest_follow_ups(
        &self,
        frame: &str,
        session: &Session,
        mut model: wisp_llm::ProviderConfig,
        ended: u64,
    ) {
        let store = &self.0.store;
        let found = async {
            if self.0.shutting_down.load(Ordering::Relaxed) || !follow_ups::enabled(store).await {
                return Ok(vec![]);
            }
            let messages = store
                .load_recent_turn_preview_messages(frame, follow_ups::TURNS)
                .await?;
            // The call goes the way the turn went, and is kept short.
            let routes = network::load(store).await;
            if let Some(route) = network::route(&routes.model_proxy_url) {
                model.proxy = Some(route);
            }
            model.max_tokens = follow_ups::MAX_TOKENS;
            let provider = (self.0.models)(model);
            follow_ups::suggest(provider.as_ref(), &messages).await
        }
        .await;
        let questions = found.unwrap_or_else(|error| {
            tracing::warn!(%frame, "No follow-up questions: {error:#}");
            vec![]
        });
        {
            // A newer turn has its own questions to wait for.
            let mut live = session.live.lock().unwrap();
            if live.generation == ended {
                live.follow_ups = questions;
                live.suggesting = false;
            }
        }
        session.suggested.notify_waiters();
    }

    async fn snapshot(&self, project: &str, request: &dto::SessionRequest) -> Result<Value> {
        let session = self.session(&request.session_id);
        loop {
            let generation = session.live.lock().unwrap().generation;
            if let Some(snapshot) = self
                .snapshot_at_generation(project, request, &session, generation)
                .await?
            {
                return Ok(snapshot);
            }
        }
    }

    async fn snapshot_at_generation(
        &self,
        project: &str,
        request: &dto::SessionRequest,
        session: &Session,
        generation: u64,
    ) -> Result<Option<Value>> {
        let options = self.stored_options(&request.session_id).await?;
        let archived = self.archived(&request.session_id).await?;
        let profile = self.turn_profile(&options).await;
        let page = self
            .0
            .store
            .load_session_transcript_page(&request.session_id, request.before_seq, 20)
            .await?;
        #[cfg(test)]
        {
            let pause = session.snapshot_after_read.lock().unwrap().take();
            if let Some((loaded, resume)) = pause {
                let _ = loaded.send(());
                let _ = resume.await;
            }
        }
        let turn = self.0.store.latest_server_turn(&request.session_id).await?;
        let runs = self.0.store.list_runs_by_project(project).await?;
        let mut outcomes: HashMap<_, _> = self
            .0
            .store
            .load_session_ui_events(&request.session_id)
            .await?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<output::OutcomeEvent>(&raw).ok())
            .filter(|event| event.kind == "ServerToolResult")
            .map(|event| (event.outcome.call_id.clone(), event.outcome))
            .collect();
        // The agent that runs the conversation, when one does.
        let model_id = match options.acp_agent.as_str() {
            "" => self.model_id(&profile),
            id => acp_agents::list(&self.0.store)
                .await
                .into_iter()
                .find(|agent| agent.id == id)
                .map(|agent| agent.label)
                .unwrap_or_else(|| "ACP agent".into()),
        };
        let exploration = self.exploration_state(project, &request.session_id).await;
        let dispatched = self.subagent_parent(&request.session_id).await.is_some();
        let live = session.live.lock().unwrap();
        // Completion clears the live transcript only after persisting it. If it
        // crossed these store reads, reload instead of returning an old page as
        // complete. A generation also detects a whole turn starting and ending.
        if live.generation != generation {
            return Ok(None);
        }
        let use_live = live.running && request.before_seq.is_none();
        let seqs: Vec<i64> = page.messages.iter().map(|(seq, _)| *seq).collect();
        let saved: Vec<_> = page.messages.into_iter().map(|(_, msg)| msg).collect();
        let mut items = runtime::items(if use_live { &live.messages } else { &saved });
        outcomes.extend(live.outcomes.clone());
        for item in &mut items {
            if let Some(outcome) = item.call_id.as_ref().and_then(|id| outcomes.get(id)) {
                item.ok = Some(outcome.ok);
                item.duration_ms = Some(outcome.duration_ms);
            }
        }
        let states: Vec<runtime::RunState> = runs.iter().map(Into::into).collect();
        runtime::annotate_runs(&mut items, &states, &request.session_id);
        runtime::annotate_plans(&mut items);
        // Reviews kept with the conversation, after what each one read.
        if !use_live {
            runtime::place_reviews(&mut items, &seqs, &saved, &page.reviews);
        }
        if use_live && !live.draft.is_empty() {
            items.push(runtime::item("assistant", &live.draft));
        }
        let approvals: Vec<_> = live
            .approvals
            .values()
            .map(|(value, _)| value.clone())
            .collect();
        let snapshot = dto::Snapshot {
            magpie: None,
            schema: dto::SCHEMA.into(),
            epoch: self.0.epoch.clone(),
            sequence: self.0.sequence.fetch_add(1, Ordering::Relaxed),
            project_id: project.into(),
            session_id: request.session_id.clone(),
            exploration,
            branches: vec![],
            items,
            next_before_seq: if use_live {
                live.next_before_seq
            } else {
                page.next_before_seq
            },
            user_offset: 0,
            running: live.running,
            stopping: live.running && session.cancel.load(Ordering::Relaxed),
            activity_status: None,
            // A subagent's conversation is watched, not written to.
            read_only: archived || dispatched,
            model_id,
            composer_references: Some(false),
            context_view: Some(false),
            file_browser: Some(true),
            file_transfers: Some(false),
            follow_ups: live.follow_ups.clone(),
            plan_mode: Some(options.plan_first),
            fast_mode: None,
            acp_agent_id: (!options.acp_agent.is_empty()).then(|| options.acp_agent.clone()),
            acp: None,
            acp_state: None,
            history_state: None,
            queue: None,
            request_id: turn.as_ref().map(|turn| turn.request_id.clone()),
            error: live
                .error
                .clone()
                .or_else(|| turn.and_then(|turn| turn.error)),
            approval_scopes: approvals
                .iter()
                .map(|a| (a.approval_id.clone(), vec![dto::ApprovalScope::Once]))
                .collect(),
            approvals,
            run_cards: runs
                .into_iter()
                .filter(|run| run.frame_id.as_deref() == Some(&request.session_id))
                .take(100)
                .map(|run| serde_json::from_value(serde_json::to_value(run)?))
                .collect::<serde_json::Result<_>>()?,
            run_review_supported: Some(false),
        };
        Ok(Some(serde_json::to_value(snapshot)?))
    }

    pub async fn shutdown(&self) {
        self.0.remote.stop();
        self.0.channels.stop_all();
        self.0.device.stop().await;
        self.0.browser.stop();
        {
            let _admission = self.0.admission.write().await;
            self.0.shutting_down.store(true, Ordering::Relaxed);
            for session in self.0.sessions.lock().unwrap().values() {
                session.stop();
            }
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            self.0.turns.acquire_many(4),
        )
        .await;
        let runtimes: Vec<_> = self
            .0
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter_map(|s| s.runtime.lock().unwrap().clone())
            .collect();
        self.0.mcp.close_all().await;
        for runtime in runtimes {
            runtime.shutdown_all().await;
        }
        // A method search under way resumes from its checkpoint later.
        if let Err(error) = self.0.store.pause_method_searches_for_shutdown().await {
            tracing::warn!(%error, "Could not pause method searches for shutdown");
        }
        self.0.terminals.stop_all();
    }
}

#[async_trait]
impl DirectRemoteHost for Server {
    fn name(&self) -> &str {
        "Wisp Server"
    }
    fn commands(&self) -> &'static [&'static str] {
        COMMANDS
    }
    async fn dispatch(&self, request: RemoteRequest) -> Result<Value, String> {
        self.execute(request).await.map_err(|e| format!("{e:#}"))
    }
}
