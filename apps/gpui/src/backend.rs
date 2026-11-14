//! Interactive UI never blocks on storage, tool execution, or model IO.
//! Final native application termination waits for the host's bounded cleanup.
use std::{future::Future, path::PathBuf, sync::Arc};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::{runtime::Runtime, sync::OnceCell, task::JoinHandle};
use wisp_dto::{
    SessionSearchInfo,
    native_conversations::{SendAccepted, SendRequest},
    native_files::{Directory, Preview},
    remote_access::RemoteRequest,
};
use wisp_server::{
    Server,
    config::Config,
    desktop::{
        ArtifactRow, BatchAction, BatchReport, ConnectorRow, ConversationList, ConversationOptions,
        ConversationStatus, CredentialRow, EnvironmentsView, Favorite, FileMatch, Folder,
        GlobalMemory, LibraryItem, LibraryItemSummary, McpServer, McpServerInput, McpServerRow,
        MemoryView, ModelInput, ModelSettings, ProjectGrants, ProjectMemo, ProjectMemoPatch,
        ReferenceOption, Remember, SkillRow, SshHostRow, StorageView, TurnDefaults, TurnRef,
        UsageView,
    },
};
use wisp_sync::DirectRemoteHost;

#[derive(Clone)]
pub struct Backend {
    runtime: Arc<Runtime>,
    host: Arc<OnceCell<Server>>,
}

pub type Reply<T> = JoinHandle<Result<T, String>>;

pub async fn receive<T>(reply: Reply<T>) -> Result<T, String> {
    reply.await.map_err(|e| e.to_string())?
}

impl Backend {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            host: Arc::default(),
        }
    }

    fn spawn<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
    ) -> Reply<T> {
        self.runtime.spawn(work)
    }

    pub fn initialize(&self) -> Reply<()> {
        let host = self.host.clone();
        self.spawn(async move {
            let base = dirs::data_local_dir().ok_or("Cannot locate the local data directory")?;
            let data = std::env::var_os("WISP_GPUI_DATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join("wisp-science-gpui"));
            let workspace = std::env::var_os("WISP_GPUI_WORKSPACE")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join("wisp-science-gpui-workspace"));
            host.get_or_try_init(|| async {
                let config = Config::local(data, workspace).map_err(|e| format!("{e:#}"))?;
                Server::open(config).await.map_err(|e| format!("{e:#}"))
            })
            .await?;
            Ok(())
        })
    }

    pub fn request<T: DeserializeOwned + Send + 'static>(
        &self,
        command: &str,
        project: Option<String>,
        args: Value,
    ) -> Reply<T> {
        let host = self.host.clone();
        let request = RemoteRequest {
            nonce: String::new(),
            seq: 0,
            id: uuid::Uuid::new_v4().to_string(),
            command: command.into(),
            project_id: project,
            args,
        };
        self.spawn(async move {
            let host = host.get().ok_or("Host is not ready")?;
            // Direct function call: no HTTP listener, pairing code or WebView.
            let value = host.dispatch(request).await?;
            serde_json::from_value(value).map_err(|e| e.to_string())
        })
    }

    /// Runs a desktop-only host operation. These are direct method calls and
    /// have no remote command name.
    fn local<T, F>(&self, work: impl FnOnce(Server) -> F + Send + 'static) -> Reply<T>
    where
        T: Send + 'static,
        F: Future<Output = anyhow::Result<T>> + Send,
    {
        let host = self.host.clone();
        self.spawn(async move {
            let host = host.get().ok_or("Host is not ready")?.clone();
            work(host).await.map_err(|e| format!("{e:#}"))
        })
    }

    pub fn register_project(&self, path: PathBuf) -> Reply<String> {
        self.local(move |host| async move { host.register_local_project(&path).await })
    }

    pub fn recent_conversations(&self, limit: usize) -> Reply<Vec<SessionSearchInfo>> {
        self.local(move |host| async move { host.recent_conversations(limit).await })
    }

    pub fn find_conversations(&self, query: String, limit: usize) -> Reply<Vec<SessionSearchInfo>> {
        self.local(move |host| async move { host.find_conversations(&query, limit).await })
    }

    /// Files by name across every project.
    /// Files by name in one project, or in every project.
    pub fn find_files(
        &self,
        project: Option<String>,
        query: String,
        limit: usize,
    ) -> Reply<Vec<FileMatch>> {
        self.local(
            move |host| async move { host.find_files(project.as_deref(), &query, limit).await },
        )
    }

    pub fn skill_options(
        &self,
        project: String,
        query: String,
        limit: usize,
    ) -> Reply<Vec<ReferenceOption>> {
        self.local(move |host| async move { host.skill_options(&project, &query, limit).await })
    }

    /// Copies files picked outside the project into it and returns the
    /// project-relative path of each.
    pub fn import_attachments(&self, project: String, files: Vec<PathBuf>) -> Reply<Vec<String>> {
        self.local(move |host| async move { host.import_attachments(&project, &files).await })
    }

    /// Sends a message with its files and references.
    pub fn send_message(&self, project: String, request: SendRequest) -> Reply<SendAccepted> {
        self.local(move |host| async move { host.send(&project, request).await })
    }

    pub fn project_directory(&self, project: String, path: String) -> Reply<Directory> {
        self.local(move |host| async move { host.project_directory(&project, &path).await })
    }

    pub fn project_file(&self, project: String, path: String) -> Reply<Preview> {
        self.local(move |host| async move { host.project_file(&project, &path).await })
    }

    pub fn star_project(&self, project: String, starred: bool) -> Reply<()> {
        self.local(move |host| async move { host.star_project(&project, starred).await })
    }

    pub fn remove_project(&self, project: String) -> Reply<()> {
        self.local(move |host| async move { host.remove_project(&project).await })
    }

    pub fn conversations(
        &self,
        project: String,
        query: String,
        archived: bool,
    ) -> Reply<ConversationList> {
        self.local(move |host| async move { host.conversations(&project, &query, archived).await })
    }

    pub fn rename_conversation(
        &self,
        project: String,
        session: String,
        title: String,
    ) -> Reply<()> {
        self.local(
            move |host| async move { host.rename_conversation(&project, &session, &title).await },
        )
    }

    pub fn pin_conversation(&self, project: String, session: String, pinned: bool) -> Reply<()> {
        self.local(
            move |host| async move { host.pin_conversation(&project, &session, pinned).await },
        )
    }

    pub fn archive_conversation(
        &self,
        project: String,
        session: String,
        archived: bool,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.archive_conversation(&project, &session, archived)
                .await
        })
    }

    pub fn delete_conversation(&self, project: String, session: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_conversation(&project, &session).await })
    }

    pub fn create_folder(&self, project: String, name: String) -> Reply<Folder> {
        self.local(move |host| async move { host.create_folder(&project, &name).await })
    }

    pub fn rename_folder(&self, project: String, folder: String, name: String) -> Reply<()> {
        self.local(move |host| async move { host.rename_folder(&project, &folder, &name).await })
    }

    pub fn delete_folder(&self, project: String, folder: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_folder(&project, &folder).await })
    }

    pub fn reorder_folders(&self, project: String, folders: Vec<String>) -> Reply<()> {
        self.local(move |host| async move { host.reorder_folders(&project, &folders).await })
    }

    pub fn group_conversation(
        &self,
        project: String,
        session: String,
        folder: Option<String>,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.group_conversation(&project, &session, folder.as_deref())
                .await
        })
    }

    pub fn batch(
        &self,
        project: String,
        sessions: Vec<String>,
        action: BatchAction,
    ) -> Reply<BatchReport> {
        self.local(move |host| async move { host.batch(&project, &sessions, action).await })
    }

    /// Copies (`keep`) or moves a conversation to another project and
    /// returns its id there.
    pub fn transfer_conversation(
        &self,
        project: String,
        session: String,
        target: String,
        keep: bool,
    ) -> Reply<String> {
        self.local(move |host| async move {
            host.transfer_conversation(&project, &session, &target, keep)
                .await
        })
    }

    /// Cuts the conversation back to just before `turn` and returns that
    /// message's text.
    pub fn rewind_conversation(
        &self,
        project: String,
        session: String,
        turn: TurnRef,
    ) -> Reply<String> {
        self.local(
            move |host| async move { host.rewind_conversation(&project, &session, &turn).await },
        )
    }

    /// Copies the conversation, or its part through `upto`'s reply, into a
    /// new one and returns its id.
    pub fn branch_conversation(
        &self,
        project: String,
        session: String,
        upto: Option<TurnRef>,
    ) -> Reply<String> {
        self.local(move |host| async move {
            host.branch_conversation(&project, &session, upto.as_ref())
                .await
        })
    }

    pub fn memos(&self, project: String) -> Reply<Vec<ProjectMemo>> {
        self.local(move |host| async move { host.memos(&project).await })
    }

    pub fn add_memo(&self, project: String, title: String, body: String) -> Reply<ProjectMemo> {
        self.local(move |host| async move { host.add_memo(&project, &title, &body).await })
    }

    pub fn update_memo(&self, project: String, patch: ProjectMemoPatch) -> Reply<ProjectMemo> {
        self.local(move |host| async move { host.update_memo(&project, patch).await })
    }

    pub fn delete_memo(&self, project: String, memo: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_memo(&project, &memo).await })
    }

    pub fn memory(&self, project: String) -> Reply<MemoryView> {
        self.local(move |host| async move { host.memory(&project).await })
    }

    pub fn set_memory_enabled(&self, enabled: bool) -> Reply<()> {
        self.local(move |host| async move { host.set_memory_enabled(enabled).await })
    }

    pub fn memory_note(&self, project: String, name: String) -> Reply<String> {
        self.local(move |host| async move { host.memory_note(&project, &name).await })
    }

    pub fn save_memory_note(&self, project: String, name: String, content: String) -> Reply<()> {
        self.local(
            move |host| async move { host.save_memory_note(&project, &name, &content).await },
        )
    }

    pub fn delete_memory_note(&self, project: String, name: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_memory_note(&project, &name).await })
    }

    /// Keeps `text` for the project, or with `everywhere` as a habit.
    pub fn remember(
        &self,
        project: String,
        session: Option<String>,
        text: String,
        everywhere: bool,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.remember(&project, session.as_deref(), &text, everywhere)
                .await
        })
    }

    pub fn add_habit(&self, content: String) -> Reply<GlobalMemory> {
        self.local(move |host| async move { host.add_habit(&content).await })
    }

    pub fn update_habit(&self, id: String, content: String) -> Reply<()> {
        self.local(move |host| async move { host.update_habit(&id, &content).await })
    }

    pub fn delete_habit(&self, id: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_habit(&id).await })
    }

    pub fn favorites(
        &self,
        query: String,
        kind: Option<&'static str>,
    ) -> Reply<Vec<LibraryItemSummary>> {
        self.local(move |host| async move { host.favorites(&query, kind).await })
    }

    pub fn favorite(&self, id: String) -> Reply<Favorite> {
        self.local(move |host| async move { host.favorite(&id).await })
    }

    pub fn conversation_favorites(
        &self,
        project: String,
        session: String,
    ) -> Reply<Vec<LibraryItem>> {
        self.local(move |host| async move { host.conversation_favorites(&project, &session).await })
    }

    /// Saves a message's text as it reads now.
    pub fn star_text(&self, project: String, session: String, text: String) -> Reply<LibraryItem> {
        self.local(move |host| async move {
            host.star_text(&project, &session, "text", None, &text)
                .await
        })
    }

    pub fn star_figure(
        &self,
        project: String,
        session: String,
        path: String,
    ) -> Reply<LibraryItem> {
        self.local(move |host| async move { host.star_figure(&project, &session, &path).await })
    }

    pub fn unstar(&self, id: String) -> Reply<()> {
        self.local(move |host| async move { host.unstar(&id).await })
    }

    pub fn preference(&self, name: &'static str) -> Reply<Option<String>> {
        self.local(move |host| async move { host.preference(name).await })
    }

    pub fn set_preference(&self, name: &'static str, value: String) -> Reply<()> {
        self.local(move |host| async move { host.set_preference(name, &value).await })
    }

    pub fn artifacts(
        &self,
        project: String,
        session: Option<String>,
        query: String,
        limit: usize,
    ) -> Reply<Vec<ArtifactRow>> {
        self.local(move |host| async move {
            host.artifacts(&project, session.as_deref(), &query, limit)
                .await
        })
    }

    pub fn cancel_run(&self, project: String, run: String) -> Reply<()> {
        self.local(move |host| async move { host.cancel_run(&project, &run).await })
    }

    pub fn conversation_status(
        &self,
        project: String,
        session: String,
    ) -> Reply<ConversationStatus> {
        self.local(move |host| async move { host.conversation_status(&project, &session).await })
    }

    pub fn set_conversation_options(
        &self,
        project: String,
        session: String,
        options: ConversationOptions,
    ) -> Reply<ConversationOptions> {
        self.local(move |host| async move {
            host.set_conversation_options(&project, &session, options)
                .await
        })
    }

    pub fn answer_approval(
        &self,
        project: String,
        session: String,
        approval: String,
        approved: bool,
        remember: Remember,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.answer_approval(&project, &session, &approval, approved, remember)
                .await
        })
    }

    /// Starts an empty conversation the way the desktop's defaults say.
    pub fn create_conversation(&self, project: String) -> Reply<String> {
        self.local(move |host| async move { host.create_conversation(&project).await })
    }

    pub fn network(&self) -> Reply<wisp_dto::NetworkSettings> {
        self.local(|host| async move { host.network().await })
    }

    /// Saves the routes and package sources; the reply is what was kept.
    pub fn set_network(
        &self,
        settings: wisp_dto::NetworkSettings,
    ) -> Reply<wisp_dto::NetworkSettings> {
        self.local(move |host| async move { host.set_network(settings).await })
    }

    pub fn environments(&self) -> Reply<EnvironmentsView> {
        self.local(|host| async move { host.environments().await })
    }

    /// Connects to the host now; the reply is what was found or why not.
    pub fn check_environment(
        &self,
        alias: String,
    ) -> Reply<wisp_server::desktop::EnvironmentCheck> {
        self.local(move |host| async move { host.check_environment(&alias).await })
    }

    pub fn save_ssh_host(&self, row: SshHostRow) -> Reply<()> {
        self.local(move |host| async move { host.save_ssh_host(row).await })
    }

    pub fn import_ssh_hosts(&self, aliases: Vec<String>) -> Reply<()> {
        self.local(move |host| async move { host.import_ssh_hosts(&aliases).await })
    }

    pub fn remove_ssh_host(&self, alias: String) -> Reply<()> {
        self.local(move |host| async move { host.remove_ssh_host(&alias).await })
    }

    /// Where new work runs: the host with this alias, or this computer.
    pub fn set_default_environment(&self, alias: Option<String>) -> Reply<()> {
        self.local(move |host| async move { host.set_default_environment(alias.as_deref()).await })
    }

    pub fn mcp_servers(&self) -> Reply<Vec<McpServerRow>> {
        self.local(|host| async move { host.mcp_servers().await })
    }

    pub fn save_mcp_server(&self, input: McpServerInput) -> Reply<McpServer> {
        self.local(move |host| async move { host.save_mcp_server(input).await })
    }

    pub fn set_mcp_server_enabled(&self, id: String, enabled: bool) -> Reply<()> {
        self.local(move |host| async move { host.set_mcp_server_enabled(&id, enabled).await })
    }

    pub fn delete_mcp_server(&self, id: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_mcp_server(&id).await })
    }

    /// Connects now and returns the names of the tools the server offers.
    pub fn test_mcp_server(&self, id: String) -> Reply<Vec<String>> {
        self.local(move |host| async move { host.test_mcp_server(&id).await })
    }

    pub fn credentials(&self) -> Reply<Vec<CredentialRow>> {
        self.local(|host| async move { host.credentials().await })
    }

    /// Saves a service key, or with an empty value forgets it.
    pub fn set_credential(&self, id: &'static str, value: String) -> Reply<()> {
        self.local(move |host| async move { host.set_credential(id, &value).await })
    }

    pub fn usage(&self) -> Reply<UsageView> {
        self.local(|host| async move { host.usage().await })
    }

    pub fn storage(&self) -> Reply<StorageView> {
        self.local(|host| async move { host.storage().await })
    }

    pub fn connectors(&self) -> Reply<Vec<ConnectorRow>> {
        self.local(|host| async move { host.connectors().await })
    }

    pub fn set_connector_enabled(&self, domain: String, enabled: bool) -> Reply<()> {
        self.local(move |host| async move { host.set_connector_enabled(&domain, enabled).await })
    }

    pub fn skills(&self, project: Option<String>) -> Reply<Vec<SkillRow>> {
        self.local(move |host| async move { host.skills(project.as_deref()).await })
    }

    pub fn set_skill_enabled(&self, name: String, enabled: bool) -> Reply<()> {
        self.local(move |host| async move { host.set_skill_enabled(&name, enabled).await })
    }

    pub fn turn_defaults(&self) -> Reply<TurnDefaults> {
        self.local(|host| async move { host.turn_defaults().await })
    }

    pub fn set_turn_defaults(&self, defaults: TurnDefaults) -> Reply<TurnDefaults> {
        self.local(move |host| async move { host.set_turn_defaults(defaults).await })
    }

    pub fn project_grants(&self) -> Reply<Vec<ProjectGrants>> {
        self.local(|host| async move { host.project_grants().await })
    }

    pub fn revoke_project_grant(&self, project: String, tool: String) -> Reply<()> {
        self.local(move |host| async move { host.revoke_project_grant(&project, &tool).await })
    }

    pub fn revoke_allowed_tools(&self, project: String, session: String) -> Reply<()> {
        self.local(move |host| async move { host.revoke_allowed_tools(&project, &session).await })
    }

    pub fn model_settings(&self) -> Reply<ModelSettings> {
        self.local(|host| async move { host.model_settings().await })
    }

    pub fn save_model_profile(&self, input: ModelInput) -> Reply<ModelSettings> {
        self.local(move |host| async move { host.save_model_profile(input).await })
    }

    pub fn set_default_model(&self, id: String) -> Reply<ModelSettings> {
        self.local(move |host| async move { host.set_default_model(&id).await })
    }

    pub fn delete_model_profile(&self, id: String) -> Reply<ModelSettings> {
        self.local(move |host| async move { host.delete_model_profile(&id).await })
    }

    /// Sends one request with the form as typed; nothing is saved.
    pub fn test_model(&self, input: ModelInput) -> Reply<String> {
        self.local(move |host| async move { host.test_model(input).await })
    }

    pub async fn shutdown(&self) {
        if let Some(host) = self.host.get() {
            host.shutdown().await;
        }
    }
}
