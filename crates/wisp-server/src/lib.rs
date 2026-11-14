pub mod config;
pub mod desktop;
mod files;
pub mod mcp;
mod output;
mod runtime;
#[cfg(test)]
mod tests;

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

struct Inner {
    config: Config,
    store: Store,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    runs: wisp_runs::RunManager,
    epoch: String,
    sequence: AtomicU64,
    turns: Arc<Semaphore>,
    shutting_down: AtomicBool,
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
        store
            .create_project(PROJECT, "Server", &config.workspace.to_string_lossy())
            .await?;
        for (id, ..) in store.list_projects().await? {
            store.interrupt_server_turns(&id).await?;
        }
        let runs = wisp_runs::RunManager::new();
        runs.recover(&store).await.map_err(anyhow::Error::msg)?;
        let model = match config.model.take() {
            Some(model) => desktop::ModelState::environment(model),
            None => desktop::ModelState::saved(&store, secrets.as_ref()).await,
        };
        Ok(Self(Arc::new(Inner {
            config,
            store,
            sessions: Default::default(),
            runs,
            epoch: uuid::Uuid::new_v4().to_string(),
            sequence: AtomicU64::new(0),
            turns: Arc::new(Semaphore::new(4)),
            shutting_down: AtomicBool::new(false),
            admission: RwLock::new(()),
            _lock: lock,
            models,
            model: std::sync::RwLock::new(model),
            secrets,
            library: Default::default(),
            mcp: Default::default(),
        })))
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

    async fn start(
        &self,
        project: &str,
        root: &str,
        mut request: dto::SendRequest,
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
        ensure!(
            !request.message.trim().is_empty() && request.message.len() <= 128 * 1024,
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
        // Read once here: a change made while the turn runs applies to the next.
        let options = self.stored_options(&request.session_id).await?;
        let model = self.0.model.read().unwrap().pick(&options.model)?;
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
        let permit = self
            .0
            .turns
            .clone()
            .try_acquire_owned()
            .context("Server is busy; at most four turns can run concurrently")?;
        let full_access = options.full_access;
        let plan_mode = options.plan_first;
        let agent = runtime::build(
            &self.0.config,
            &model,
            &self.0.store,
            self.0.runs.clone(),
            project,
            root,
            &request.session_id,
            &session,
            &self.0.models,
            (!options.reasoning_effort.is_empty()).then_some(options.reasoning_effort),
            &request.references,
            options.plan_first,
            &self.service_keys().await,
            self.mcp_tools().await,
        )
        .await?;
        let page = self
            .0
            .store
            .load_session_transcript_page(&request.session_id, None, 20)
            .await?;
        self.0
            .store
            .accept_server_turn(&request.session_id, &request.request_id, &request_json)
            .await?;
        if !agent
            .ctx
            .messages
            .iter()
            .any(|msg| msg.role == wisp_llm::Role::User)
        {
            let title: String = subject.chars().take(80).collect();
            if let Err(error) = self
                .0
                .store
                .rename_session(&request.session_id, project, &title)
                .await
            {
                tracing::warn!(%error, "Could not name the new conversation");
            }
        }
        {
            let mut live = session.live.lock().unwrap();
            live.running = true;
            live.generation = live.generation.wrapping_add(1);
            live.error = None;
            live.usage = None;
            live.calls.clear();
            live.reasoning.clear();
            live.written.clear();
            live.draft.clear();
            live.messages = page.messages.into_iter().map(|(_, msg)| msg).collect();
            live.next_before_seq = page.next_before_seq;
            session.cancel.store(false, Ordering::Relaxed);
        }
        let server = self.clone();
        let session = session.clone();
        let project = project.to_string();
        let root = std::path::PathBuf::from(root);
        tokio::spawn(async move {
            let _permit = permit;
            let inner = server.clone();
            let running = session.clone();
            let input = request.clone();
            let task = tokio::spawn(async move {
                runtime::run(
                    inner.0.store.clone(),
                    agent,
                    running,
                    project,
                    input,
                    full_access,
                    plan_mode,
                    root,
                )
                .await
            });
            let mut error = match task.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("{e:#}")),
                Err(_) => Some("Agent task stopped unexpectedly".into()),
            };
            if let Err(e) = server
                .0
                .store
                .finish_server_turn(&request.session_id, &request.request_id, error.as_deref())
                .await
            {
                error = Some(format!("Could not save turn completion: {e}"));
            }
            let mut live = session.live.lock().unwrap();
            live.running = false;
            live.generation = live.generation.wrapping_add(1);
            live.approvals.clear();
            live.messages.clear();
            live.draft.clear();
            live.reasoning.clear();
            live.error = error;
        });
        Ok(reply)
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
        let profile = options.model;
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
        let live = session.live.lock().unwrap();
        // Completion clears the live transcript only after persisting it. If it
        // crossed these store reads, reload instead of returning an old page as
        // complete. A generation also detects a whole turn starting and ending.
        if live.generation != generation {
            return Ok(None);
        }
        let use_live = live.running && request.before_seq.is_none();
        let saved: Vec<_> = page.messages.into_iter().map(|(_, msg)| msg).collect();
        let mut items = runtime::items(if use_live { &live.messages } else { &saved });
        outcomes.extend(live.outcomes.clone());
        for item in &mut items {
            if let Some(outcome) = item.call_id.as_ref().and_then(|id| outcomes.get(id)) {
                item.ok = Some(outcome.ok);
                item.duration_ms = Some(outcome.duration_ms);
            }
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
            exploration: None,
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
            read_only: false,
            model_id: self.model_id(&profile),
            composer_references: Some(false),
            context_view: Some(false),
            file_browser: Some(true),
            file_transfers: Some(false),
            follow_ups: vec![],
            plan_mode: Some(options.plan_first),
            fast_mode: None,
            acp_agent_id: None,
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
