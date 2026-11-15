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
        ProjectSettings, ReferenceOption, Remember, SkillRow, SshHostRow, StorageView,
        TurnDefaults, TurnRef, UsageView,
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
                let server = Server::open(config).await.map_err(|e| format!("{e:#}"))?;
                // Scheduled tasks fire while the app is open.
                server.start_scheduler();
                // The bridge to the browser extension, when it was left on.
                server.start_browser().await;
                // The tunnel for remote web access, when it was left on.
                server.start_remote_access().await;
                // The chat apps that were left on.
                server.start_channels().await;
                // The StickS3 desk display's listener, when it was left on;
                // a failure shows on its page and never stops the app.
                let _ = server.start_device_bridge().await;
                Ok::<_, String>(server)
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

    /// A picture in the project: its content type and its bytes.
    /// A document of the project as Markdown to read.
    pub fn document_preview(&self, project: String, path: String) -> Reply<Option<String>> {
        self.local(move |host| async move { host.document_preview(&project, &path).await })
    }

    pub fn project_image(&self, project: String, path: String) -> Reply<(String, Vec<u8>)> {
        self.local(move |host| async move { host.project_image(&project, &path).await })
    }

    /// Makes an empty folder or file in a project folder.
    pub fn create_project_entry(
        &self,
        project: String,
        parent: String,
        name: String,
        folder: bool,
    ) -> Reply<String> {
        self.local(move |host| async move {
            host.create_project_entry(&project, &parent, &name, folder)
                .await
        })
    }

    pub fn rename_project_entry(
        &self,
        project: String,
        path: String,
        name: String,
    ) -> Reply<String> {
        self.local(
            move |host| async move { host.rename_project_entry(&project, &path, &name).await },
        )
    }

    /// Moves a file or folder to the project's trash; the reply is where
    /// it went.
    pub fn trash_project_entry(&self, project: String, path: String) -> Reply<String> {
        self.local(move |host| async move { host.trash_project_entry(&project, &path).await })
    }

    pub fn project_file(&self, project: String, path: String) -> Reply<Preview> {
        self.local(move |host| async move { host.project_file(&project, &path).await })
    }

    /// Writes a project as one archive at `destination`.
    pub fn export_project(&self, project: String, destination: PathBuf) -> Reply<()> {
        self.local(move |host| async move { host.export_project(&project, &destination).await })
    }

    /// Reads an archive and puts its project in a new folder under `parent`.
    pub fn import_project(&self, archive: PathBuf, parent: PathBuf) -> Reply<String> {
        self.local(move |host| async move { host.import_project(&archive, &parent).await })
    }

    /// What Wisp can draw on in a project, counted.
    pub fn capabilities(&self, project: String) -> Reply<wisp_server::desktop::Capabilities> {
        self.local(move |host| async move { host.capabilities(&project).await })
    }

    /// A conversation as a trajectory: its turns, their steps and totals.
    pub fn trajectory(
        &self,
        project: String,
        session: String,
    ) -> Reply<wisp_dto::TrajectorySnapshotDto> {
        self.local(move |host| async move { host.trajectory(&project, &session).await })
    }

    /// A conversation, whole, as a Markdown document.
    pub fn export_conversation(&self, project: String, session: String) -> Reply<String> {
        self.local(move |host| async move { host.export_conversation(&project, &session).await })
    }

    /// What happened in a project between two moments, newest first.
    pub fn journey(
        &self,
        project: String,
        from: i64,
        until: i64,
    ) -> Reply<wisp_dto::ResearchJourney> {
        self.local(move |host| async move { host.journey(&project, from, until).await })
    }

    /// Every project's history between two moments, for the calendar.
    pub fn calendar(&self, from: i64, until: i64) -> Reply<Vec<wisp_dto::ResearchCalendarProject>> {
        self.local(move |host| async move { host.calendar(from, until).await })
    }

    /// Adds a record the researcher wrote to the project's history.
    pub fn add_journal_entry(
        &self,
        project: String,
        input: wisp_dto::ResearchJournalInput,
    ) -> Reply<String> {
        self.local(move |host| async move { host.add_journal_entry(&project, input).await })
    }

    pub fn project_settings(&self, project: String) -> Reply<ProjectSettings> {
        self.local(move |host| async move { host.project_settings(&project).await })
    }

    /// Saves a project's name, description and rules; the reply is what
    /// was kept.
    pub fn set_project_settings(
        &self,
        project: String,
        settings: ProjectSettings,
    ) -> Reply<ProjectSettings> {
        self.local(move |host| async move { host.set_project_settings(&project, settings).await })
    }

    pub fn star_project(&self, project: String, starred: bool) -> Reply<()> {
        self.local(move |host| async move { host.star_project(&project, starred).await })
    }

    /// What deleting a project together with its folder would delete.
    pub fn project_folder(&self, project: String) -> Reply<wisp_server::desktop::ProjectFolder> {
        self.local(move |host| async move { host.project_folder(&project).await })
    }

    /// Removes a project and deletes its folder. `typed` is the project's
    /// name as the user typed it; the host checks it again.
    pub fn delete_project_and_folder(&self, project: String, typed: String) -> Reply<()> {
        self.local(
            move |host| async move { host.delete_project_and_folder(&project, &typed).await },
        )
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

    /// Goes back to a message; with `undo_files`, the latest turn's files
    /// are put back as well and what was done to them is said.
    pub fn rewind_or_undo(
        &self,
        project: String,
        session: String,
        turn: TurnRef,
        undo_files: bool,
    ) -> Reply<(String, Option<wisp_server::desktop::TurnFiles>)> {
        self.local(move |host| async move {
            if undo_files {
                let (text, files) = host.undo_turn(&project, &session, &turn).await?;
                Ok((text, Some(files)))
            } else {
                let text = host.rewind_conversation(&project, &session, &turn).await?;
                Ok((text, None))
            }
        })
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

    /// Saves a piece of code with the language it is in.
    pub fn star_code(
        &self,
        project: String,
        session: String,
        language: String,
        code: String,
    ) -> Reply<LibraryItem> {
        self.local(move |host| async move {
            host.star_text(&project, &session, "code", Some(&language), &code)
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

    /// Turns a waiting request down with what should change.
    pub fn answer_with_feedback(
        &self,
        project: String,
        session: String,
        approval: String,
        feedback: String,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.answer_with_feedback(&project, &session, &approval, &feedback)
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

    /// What the model is sent for a conversation: its saved working set.
    pub fn model_view(
        &self,
        project: String,
        session: String,
    ) -> Reply<wisp_server::desktop::ModelView> {
        self.local(move |host| async move { host.model_view(&project, &session).await })
    }

    /// Has the conversation's model review the conversation.
    pub fn review_conversation(
        &self,
        project: String,
        session: String,
    ) -> Reply<wisp_server::desktop::Review> {
        self.local(move |host| async move { host.review_conversation(&project, &session).await })
    }

    /// Makes a sample project in a new folder under `parent`.
    pub fn create_sample_project(&self, parent: PathBuf) -> Reply<String> {
        self.local(move |host| async move { host.create_sample_project(&parent).await })
    }

    /// The research archive of a conversation, if it has been frozen.
    pub fn research_archive(
        &self,
        project: String,
        session: String,
    ) -> Reply<Option<wisp_server::desktop::ResearchArchive>> {
        self.local(move |host| async move { host.research_archive(&project, &session).await })
    }

    /// Has the model draft an archive of a conversation; nothing is saved.
    /// Conversations Codex and Claude Code keep on this computer.
    pub fn external_conversations(
        &self,
        project: String,
    ) -> Reply<Vec<wisp_server::desktop::ExternalConversation>> {
        self.local(move |host| async move { host.external_conversations(&project).await })
    }

    /// Brings one of them into the project.
    pub fn import_external_conversation(
        &self,
        project: String,
        path: String,
    ) -> Reply<(String, wisp_server::desktop::ImportOutcome)> {
        self.local(
            move |host| async move { host.import_external_conversation(&project, &path).await },
        )
    }

    /// Where a branch stands for being brought back into its source.
    pub fn branch_merge(
        &self,
        project: String,
        branch: String,
    ) -> Reply<wisp_server::desktop::BranchMerge> {
        self.local(move |host| async move { host.branch_merge(&project, &branch).await })
    }

    /// Has the model summarise what a branch did after it split off.
    pub fn draft_branch_summary(
        &self,
        project: String,
        branch: String,
        guard: String,
    ) -> Reply<String> {
        self.local(
            move |host| async move { host.draft_branch_summary(&project, &branch, &guard).await },
        )
    }

    /// Adds a branch's summary to its source; returns the source.
    pub fn merge_branch(
        &self,
        project: String,
        branch: String,
        guard: String,
        summary: String,
    ) -> Reply<String> {
        self.local(move |host| async move {
            host.merge_branch(&project, &branch, &guard, &summary).await
        })
    }

    pub fn draft_research_archive(
        &self,
        project: String,
        session: String,
    ) -> Reply<wisp_server::desktop::ArchiveDraft> {
        self.local(move |host| async move { host.draft_research_archive(&project, &session).await })
    }

    /// Freezes a reviewed draft as the conversation's research archive.
    pub fn freeze_research_archive(
        &self,
        project: String,
        session: String,
        draft: wisp_server::desktop::ArchiveDraft,
    ) -> Reply<wisp_server::desktop::ResearchArchive> {
        self.local(move |host| async move {
            host.freeze_research_archive(&project, &session, draft)
                .await
        })
    }

    /// One folder of a saved SSH host, to look at.
    pub fn remote_directory(
        &self,
        alias: String,
        path: String,
    ) -> Reply<wisp_server::desktop::RemoteDirectory> {
        self.local(move |host| async move { host.remote_directory(&alias, &path).await })
    }

    /// The tool call that last wrote a project file, if one is known.
    pub fn file_origin(
        &self,
        project: String,
        session: Option<String>,
        path: String,
    ) -> Reply<Option<wisp_server::desktop::FileOrigin>> {
        self.local(move |host| async move {
            host.file_origin(&project, session.as_deref(), &path).await
        })
    }

    /// What a run on this computer wrote, in full or its end.
    pub fn run_log(&self, project: String, run: String) -> Reply<wisp_server::desktop::RunLog> {
        self.local(move |host| async move { host.run_log(&project, &run).await })
    }

    /// Asks the release page whether a newer version has been published.
    pub fn check_for_update(&self) -> Reply<wisp_server::desktop::UpdateCheck> {
        self.local(|host| async move { host.check_for_update().await })
    }

    /// Compacts a conversation now.
    pub fn compact_conversation(
        &self,
        project: String,
        session: String,
    ) -> Reply<wisp_server::desktop::Compacted> {
        self.local(move |host| async move { host.compact_conversation(&project, &session).await })
    }

    /// The browser tools as they stand.
    pub fn browser_settings(&self) -> Reply<wisp_server::desktop::BrowserSettings> {
        self.local(|host| async move { host.browser_settings().await })
    }

    pub fn set_browser_enabled(
        &self,
        enabled: bool,
    ) -> Reply<wisp_server::desktop::BrowserSettings> {
        self.local(move |host| async move { host.set_browser_enabled(enabled).await })
    }

    pub fn save_browser_filters(
        &self,
        filters: wisp_server::desktop::BrowserFilters,
    ) -> Reply<wisp_server::desktop::BrowserSettings> {
        self.local(move |host| async move { host.save_browser_filters(filters).await })
    }

    /// The workflow templates.
    pub fn workflows(&self) -> Reply<Vec<wisp_server::desktop::Workflow>> {
        self.local(|host| async move { host.workflows().await })
    }

    pub fn save_workflow(
        &self,
        input: wisp_server::desktop::WorkflowInput,
    ) -> Reply<Vec<wisp_server::desktop::Workflow>> {
        self.local(move |host| async move { host.save_workflow(input).await })
    }

    pub fn delete_workflow(&self, id: String) -> Reply<Vec<wisp_server::desktop::Workflow>> {
        self.local(move |host| async move { host.delete_workflow(&id).await })
    }

    /// Starts a run in `project`; the conversation it runs in.
    pub fn run_workflow(&self, project: String, id: String) -> Reply<String> {
        self.local(move |host| async move { host.run_workflow(&project, &id).await })
    }

    /// Remote web access as it stands.
    pub fn remote_access(&self) -> Reply<wisp_server::desktop::RemoteAccess> {
        self.local(|host| async move { host.remote_access().await })
    }

    pub fn set_remote_access(
        &self,
        enabled: bool,
        relay_url: String,
        token: Option<String>,
    ) -> Reply<wisp_server::desktop::RemoteAccess> {
        self.local(
            move |host| async move { host.set_remote_access(enabled, &relay_url, token).await },
        )
    }

    pub fn reset_remote_access_code(&self) -> Reply<wisp_server::desktop::RemoteAccess> {
        self.local(|host| async move { host.reset_remote_access_code().await })
    }

    /// The installed plugins, and whether `project` has each switched on.
    pub fn plugins(&self, project: Option<String>) -> Reply<Vec<wisp_server::desktop::PluginRow>> {
        self.local(move |host| async move { host.plugins(project.as_deref()).await })
    }

    pub fn install_plugin(&self, folder: PathBuf) -> Reply<wisp_server::desktop::PluginRow> {
        self.local(move |host| async move { host.install_plugin(&folder).await })
    }

    pub fn set_plugin_enabled(
        &self,
        project: String,
        id: String,
        version: String,
        enabled: bool,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.set_plugin_enabled(&project, &id, &version, enabled)
                .await
        })
    }

    pub fn remove_plugin(&self, id: String, version: String) -> Reply<()> {
        self.local(move |host| async move { host.remove_plugin(&id, &version).await })
    }

    /// Every project's scheduled tasks.
    pub fn schedules(&self) -> Reply<Vec<wisp_server::desktop::ScheduleRow>> {
        self.local(|host| async move { host.schedules().await })
    }

    pub fn create_schedule(&self, input: wisp_server::desktop::ScheduleInput) -> Reply<()> {
        self.local(move |host| async move { host.create_schedule(input).await })
    }

    pub fn set_schedule_enabled(&self, id: String, enabled: bool) -> Reply<()> {
        self.local(move |host| async move { host.set_schedule_enabled(&id, enabled).await })
    }

    pub fn delete_schedule(&self, id: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_schedule(&id).await })
    }

    /// Removes one conversation's timers; how many there were.
    pub fn cancel_conversation_schedules(&self, project: String, session: String) -> Reply<usize> {
        self.local(move |host| async move {
            host.cancel_conversation_schedules(&project, &session).await
        })
    }

    /// Fires a task at once; the conversation it was sent into.
    pub fn run_schedule_now(&self, id: String) -> Reply<String> {
        self.local(move |host| async move { host.run_schedule_now(&id).await })
    }

    /// The papers the project is writing.
    pub fn papers(&self, project: String) -> Reply<Vec<wisp_server::desktop::PaperRow>> {
        self.local(move |host| async move { host.papers(&project).await })
    }

    /// One paper at a version, or at its newest.
    pub fn paper(
        &self,
        project: String,
        paper: String,
        version: Option<String>,
    ) -> Reply<wisp_server::desktop::Paper> {
        self.local(
            move |host| async move { host.paper(&project, &paper, version.as_deref()).await },
        )
    }

    pub fn create_paper(
        &self,
        project: String,
        input: wisp_server::desktop::PaperInput,
    ) -> Reply<wisp_server::desktop::Paper> {
        self.local(move |host| async move { host.create_paper(&project, input).await })
    }

    pub fn rename_paper(
        &self,
        project: String,
        paper: String,
        title: String,
        description: String,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.rename_paper(&project, &paper, &title, &description)
                .await
        })
    }

    pub fn delete_paper(&self, project: String, paper: String) -> Reply<()> {
        self.local(move |host| async move { host.delete_paper(&project, &paper).await })
    }

    pub fn save_paper_item(
        &self,
        project: String,
        version: String,
        input: wisp_server::desktop::PaperItemInput,
    ) -> Reply<()> {
        self.local(move |host| async move { host.save_paper_item(&project, &version, input).await })
    }

    pub fn delete_paper_item(&self, project: String, version: String, item: String) -> Reply<()> {
        self.local(
            move |host| async move { host.delete_paper_item(&project, &version, &item).await },
        )
    }

    /// Results or runs to choose evidence from, and whether more follow.
    pub fn paper_source_choices(
        &self,
        project: String,
        kind: wisp_server::desktop::PaperSourceKind,
        query: String,
        offset: u32,
    ) -> Reply<(Vec<wisp_server::desktop::PaperSourceChoice>, bool)> {
        self.local(move |host| async move {
            host.paper_source_choices(&project, kind, &query, offset)
                .await
        })
    }

    pub fn bind_paper_source(
        &self,
        project: String,
        version: String,
        item: String,
        kind: wisp_server::desktop::PaperSourceKind,
        source: String,
        purpose: String,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.bind_paper_source(&project, &version, &item, kind, &source, &purpose)
                .await
        })
    }

    pub fn unbind_paper_source(
        &self,
        project: String,
        version: String,
        binding: String,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.unbind_paper_source(&project, &version, &binding).await
        })
    }

    pub fn check_paper(
        &self,
        project: String,
        version: String,
        policy: wisp_server::desktop::PaperPolicy,
    ) -> Reply<wisp_server::desktop::PaperCheck> {
        self.local(move |host| async move { host.check_paper(&project, &version, policy).await })
    }

    pub fn finalize_paper(
        &self,
        project: String,
        version: String,
        policy: wisp_server::desktop::PaperPolicy,
    ) -> Reply<wisp_server::desktop::PaperCheck> {
        self.local(move |host| async move { host.finalize_paper(&project, &version, policy).await })
    }

    pub fn waive_paper_finding(
        &self,
        project: String,
        version: String,
        code: String,
        reason: String,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.waive_paper_finding(&project, &version, &code, &reason)
                .await
        })
    }

    pub fn new_paper_version(
        &self,
        project: String,
        paper: String,
        version: String,
        label: String,
    ) -> Reply<wisp_server::desktop::Paper> {
        self.local(move |host| async move {
            host.new_paper_version(&project, &paper, &version, &label)
                .await
        })
    }

    /// What is installed under the data folder, and how an install goes.
    pub fn toolchain(&self) -> Reply<wisp_dto::ToolchainStatus> {
        self.local(|host| async move { Ok(host.toolchain()) })
    }

    pub fn install_toolchain(&self, tools: Vec<String>) -> Reply<wisp_dto::ToolchainStatus> {
        self.local(move |host| async move { host.install_toolchain(tools).await })
    }

    pub fn cancel_toolchain_install(&self) -> Reply<bool> {
        self.local(|host| async move { Ok(host.cancel_toolchain_install()) })
    }

    pub fn acp_agents(&self) -> Reply<Vec<wisp_server::desktop::AcpAgentRow>> {
        self.local(|host| async move { Ok(host.acp_agents().await) })
    }

    pub fn save_acp_agent(&self, agent: wisp_server::desktop::AcpAgent) -> Reply<String> {
        self.local(move |host| async move { host.save_acp_agent(agent).await })
    }

    pub fn remove_acp_agent(&self, id: String) -> Reply<()> {
        self.local(move |host| async move { host.remove_acp_agent(&id).await })
    }

    pub fn set_acp_mode(
        &self,
        project: String,
        session: String,
        mode: String,
    ) -> Reply<wisp_server::desktop::AcpControls> {
        self.local(move |host| async move { host.set_acp_mode(&project, &session, &mode).await })
    }

    pub fn set_acp_option(
        &self,
        project: String,
        session: String,
        option: String,
        pick: wisp_server::desktop::AcpPick,
    ) -> Reply<wisp_server::desktop::AcpControls> {
        self.local(move |host| async move {
            host.set_acp_option(&project, &session, &option, pick).await
        })
    }

    pub fn sign_in_acp_agent(
        &self,
        id: String,
        method: String,
    ) -> Reply<wisp_server::desktop::AcpSignInOutcome> {
        self.local(move |host| async move { host.sign_in_acp_agent(&id, &method).await })
    }

    pub fn test_acp_agent(&self, id: String) -> Reply<wisp_server::desktop::AcpAgentReport> {
        self.local(move |host| async move { host.test_acp_agent(&id).await })
    }

    pub fn subscriptions(&self) -> Reply<Vec<wisp_server::desktop::SubscriptionRow>> {
        self.local(|host| async move { Ok(host.subscriptions()) })
    }

    pub fn start_subscription_sign_in(
        &self,
        subscription: wisp_server::desktop::Subscription,
        device: bool,
    ) -> Reply<wisp_server::desktop::SignInChallenge> {
        self.local(move |host| async move {
            host.start_subscription_sign_in(subscription, device).await
        })
    }

    pub fn subscription_sign_in(&self, id: String) -> Reply<wisp_server::desktop::SignInState> {
        self.local(move |host| async move { host.subscription_sign_in(&id) })
    }

    pub fn submit_subscription_redirect(
        &self,
        id: String,
        redirect: String,
    ) -> Reply<wisp_server::desktop::SignInState> {
        self.local(
            move |host| async move { host.submit_subscription_redirect(&id, &redirect).await },
        )
    }

    pub fn cancel_subscription_sign_in(&self, id: String) -> Reply<()> {
        self.local(move |host| async move {
            host.cancel_subscription_sign_in(&id);
            Ok(())
        })
    }

    /// The credentials never come here; the host keeps them in the keychain.
    pub fn save_subscription_sign_in(
        &self,
        id: String,
    ) -> Reply<wisp_server::desktop::ModelSettings> {
        self.local(move |host| async move { host.save_subscription_sign_in(&id).await })
    }

    pub fn sign_out_subscription(
        &self,
        subscription: wisp_server::desktop::Subscription,
    ) -> Reply<wisp_server::desktop::ModelSettings> {
        self.local(move |host| async move { host.sign_out_subscription(subscription).await })
    }

    /// The ChatGPT (Codex) accounts kept on this computer, one active.
    pub fn codex_accounts(&self) -> Reply<Vec<wisp_server::desktop::CodexAccount>> {
        self.local(|host| async move { host.codex_accounts() })
    }

    pub fn switch_codex_account(&self, id: String) -> Reply<wisp_server::desktop::ModelSettings> {
        self.local(move |host| async move { host.switch_codex_account(&id).await })
    }

    pub fn remove_codex_account(&self, id: String) -> Reply<wisp_server::desktop::ModelSettings> {
        self.local(move |host| async move { host.remove_codex_account(&id).await })
    }

    /// Asks ChatGPT how much of the account's allowance is used.
    pub fn codex_account_usage(&self, id: String) -> Reply<wisp_server::desktop::CodexUsage> {
        self.local(move |host| async move { host.codex_account_usage(&id).await })
    }

    /// Brings in the sign-ins of Codex CLI and CLIProxyAPI; how many
    /// accounts were found, and the models afterwards.
    pub fn import_codex_accounts(&self) -> Reply<(usize, wisp_server::desktop::ModelSettings)> {
        self.local(|host| async move {
            let found = host.import_codex_accounts().await?;
            Ok((found, host.model_settings().await?))
        })
    }

    pub fn weixin_status(&self) -> Reply<wisp_server::desktop::WeixinStatus> {
        self.local(|host| async move { Ok(host.weixin_status().await) })
    }

    /// Asks Tencent's iLink service for a QR code to bind WeChat with.
    pub fn start_weixin_bind(&self) -> Reply<wisp_server::desktop::WeixinQr> {
        self.local(|host| async move { host.start_weixin_bind().await })
    }

    pub fn poll_weixin_bind(&self, qrcode: String) -> Reply<wisp_server::desktop::WeixinScan> {
        self.local(move |host| async move { host.poll_weixin_bind(&qrcode).await })
    }

    pub fn set_weixin_enabled(&self, enabled: bool) -> Reply<wisp_server::desktop::WeixinStatus> {
        self.local(move |host| async move { host.set_weixin_enabled(enabled).await })
    }

    pub fn unbind_weixin(&self) -> Reply<wisp_server::desktop::WeixinStatus> {
        self.local(|host| async move { host.unbind_weixin().await })
    }

    pub fn feishu_status(&self) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(|host| async move { Ok(host.feishu_status().await) })
    }

    /// Keeps the Feishu app's ID, its service, a typed secret (none keeps
    /// the stored one), and turns the bot on or off.
    pub fn set_feishu(
        &self,
        enabled: bool,
        lark: bool,
        app_id: String,
        secret: Option<String>,
    ) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(move |host| async move { host.set_feishu(enabled, lark, &app_id, secret).await })
    }

    pub fn start_feishu_bind(&self, lark: bool) -> Reply<wisp_server::desktop::FeishuBind> {
        self.local(move |host| async move { host.start_feishu_bind(lark).await })
    }

    pub fn poll_feishu_bind(&self, flow: String) -> Reply<wisp_server::desktop::FeishuBindPoll> {
        self.local(move |host| async move { host.poll_feishu_bind(&flow).await })
    }

    pub fn cancel_feishu_bind(&self, flow: String) -> Reply<()> {
        self.local(move |host| async move {
            host.cancel_feishu_bind(&flow).await;
            Ok(())
        })
    }

    pub fn unbind_feishu(&self) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(|host| async move { host.unbind_feishu().await })
    }

    pub fn set_feishu_owner(&self, open_id: String) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(move |host| async move { host.set_feishu_owner(&open_id).await })
    }

    pub fn confirm_feishu_owner(&self) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(|host| async move { host.confirm_feishu_owner().await })
    }

    pub fn reject_feishu_owner(&self) -> Reply<wisp_server::desktop::FeishuStatus> {
        self.local(|host| async move { host.reject_feishu_owner().await })
    }

    /// The Wisp Lab this computer belongs to, as its relay sees it.
    pub fn lab_status(&self) -> Reply<wisp_server::desktop::LabStatus> {
        self.local(|host| async move { Ok(host.lab_status().await) })
    }

    /// Founds the relay's lab, or asks to join it with an invite code.
    pub fn join_lab(
        &self,
        relay: String,
        token: Option<String>,
        name: String,
        lab_name: String,
        invite: String,
    ) -> Reply<wisp_server::desktop::LabStatus> {
        self.local(move |host| async move {
            host.join_lab(&relay, token, &name, &lab_name, &invite)
                .await
        })
    }

    pub fn set_lab_relay_token(&self, token: String) -> Reply<()> {
        self.local(move |host| async move { host.set_lab_relay_token(&token).await })
    }

    pub fn leave_lab(&self, forget: bool) -> Reply<()> {
        self.local(move |host| async move { host.leave_lab(forget).await })
    }

    pub fn set_lab_inbox_project(&self, project: String) -> Reply<()> {
        self.local(move |host| async move { host.set_lab_inbox_project(&project).await })
    }

    pub fn create_lab_invite(&self) -> Reply<String> {
        self.local(|host| async move { host.create_lab_invite().await })
    }

    pub fn approve_lab_member(&self, member: String) -> Reply<()> {
        self.local(move |host| async move { host.approve_lab_member(&member).await })
    }

    pub fn remove_lab_member(&self, member: String) -> Reply<()> {
        self.local(move |host| async move { host.remove_lab_member(&member).await })
    }

    /// The StickS3 desk display's listener as it stands.
    pub fn device_bridge(&self) -> Reply<wisp_server::desktop::DeviceBridge> {
        self.local(|host| async move { Ok(host.device_bridge().await) })
    }

    pub fn set_device_bridge(
        &self,
        enabled: bool,
        address: String,
        port: u32,
    ) -> Reply<wisp_server::desktop::DeviceBridge> {
        self.local(move |host| async move { host.set_device_bridge(enabled, &address, port).await })
    }

    pub fn device_bridge_token(&self) -> Reply<String> {
        self.local(|host| async move { host.device_bridge_token() })
    }

    pub fn rotate_device_bridge_token(&self) -> Reply<String> {
        self.local(|host| async move { host.rotate_device_bridge_token() })
    }

    /// Waits until the desk display asks for a conversation.
    pub fn next_device_focus(&self) -> Reply<(String, String)> {
        self.local(|host| async move { Ok(host.next_device_focus().await) })
    }

    pub fn onboarding_done(&self) -> Reply<bool> {
        self.local(|host| async move { host.onboarding_done().await })
    }

    pub fn finish_onboarding(&self) -> Reply<()> {
        self.local(|host| async move { host.finish_onboarding().await })
    }

    /// Never logged or kept here: the key goes to the system keychain.
    pub fn set_up_deepseek(&self, key: String) -> Reply<wisp_server::desktop::ModelSettings> {
        self.local(move |host| async move { host.set_up_deepseek(&key).await })
    }

    /// The decision model as the form shows it; never its key.
    pub fn decision_model(&self) -> Reply<wisp_dto::DecisionModelSettings> {
        self.local(|host| async move { host.decision_model().await })
    }

    pub fn save_decision_model(
        &self,
        settings: wisp_dto::DecisionModelSettings,
        api_key: Option<String>,
    ) -> Reply<wisp_dto::DecisionModelSettings> {
        self.local(move |host| async move { host.save_decision_model(settings, api_key).await })
    }

    pub fn clear_decision_model_key(
        &self,
        preset: String,
    ) -> Reply<wisp_dto::DecisionModelSettings> {
        self.local(move |host| async move { host.clear_decision_model_key(&preset).await })
    }

    /// Asks the form's endpoint one question; saves nothing.
    pub fn test_decision_model(
        &self,
        settings: wisp_dto::DecisionModelSettings,
        api_key: Option<String>,
    ) -> Reply<wisp_dto::DecisionModelProbe> {
        self.local(move |host| async move { host.test_decision_model(settings, api_key).await })
    }

    /// The pet the user chose, with what to draw when it is on.
    pub fn pet(&self) -> Reply<wisp_server::desktop::Pet> {
        self.local(|host| async move { host.pet().await })
    }

    pub fn set_pet(&self, enabled: bool, directory: String) -> Reply<wisp_server::desktop::Pet> {
        self.local(move |host| async move { host.set_pet(enabled, &directory).await })
    }

    /// The requests kept as quick actions.
    pub fn quick_actions(&self) -> Reply<Vec<wisp_server::desktop::QuickAction>> {
        self.local(|host| async move { host.quick_actions().await })
    }

    pub fn save_quick_action(
        &self,
        input: wisp_server::desktop::QuickActionInput,
    ) -> Reply<Vec<wisp_server::desktop::QuickAction>> {
        self.local(move |host| async move { host.save_quick_action(input).await })
    }

    pub fn delete_quick_action(&self, id: String) -> Reply<Vec<wisp_server::desktop::QuickAction>> {
        self.local(move |host| async move { host.delete_quick_action(&id).await })
    }

    /// The experts a conversation can be started as.
    pub fn experts(&self) -> Reply<Vec<wisp_server::desktop::Expert>> {
        self.local(|host| async move { host.experts().await })
    }

    pub fn save_expert(
        &self,
        input: wisp_server::desktop::ExpertInput,
    ) -> Reply<Vec<wisp_server::desktop::Expert>> {
        self.local(move |host| async move { host.save_expert(input).await })
    }

    pub fn delete_expert(&self, id: String) -> Reply<Vec<wisp_server::desktop::Expert>> {
        self.local(move |host| async move { host.delete_expert(&id).await })
    }

    /// Has one turn reviewed: the conversation as it stood when it ended.
    pub fn review_turn(
        &self,
        project: String,
        session: String,
        turn: wisp_server::desktop::TurnRef,
    ) -> Reply<wisp_server::desktop::Review> {
        self.local(move |host| async move { host.review_turn(&project, &session, &turn).await })
    }

    /// Asks the conversation's model a question about the conversation,
    /// without adding anything to it.
    pub fn ask_aside(
        &self,
        project: String,
        session: String,
        earlier: Vec<wisp_server::desktop::AsideExchange>,
        question: String,
    ) -> Reply<String> {
        self.local(move |host| async move {
            host.ask_aside(&project, &session, &earlier, &question)
                .await
        })
    }

    /// Stops one of a conversation's interpreters, or takes a stopped one
    /// off the list.
    /// Puts a message into the running turn; false when it no longer reads
    /// any, and the message is to be sent as a turn of its own.
    pub fn guide(&self, project: String, session: String, text: String) -> Reply<bool> {
        self.local(move |host| async move { host.guide(&project, &session, &text).await })
    }

    pub fn stop_runtime(&self, project: String, session: String, id: String) -> Reply<()> {
        self.local(move |host| async move { host.stop_runtime(&project, &session, &id).await })
    }

    /// Returns once the host is no longer asking a model about the turn
    /// that just ended: what could be asked next, and a review of it.
    pub fn after_turn_settled(&self, project: String, session: String) -> Reply<()> {
        self.local(move |host| async move { host.after_turn_settled(&project, &session).await })
    }

    /// Puts away the review that followed the latest turn.
    pub fn dismiss_review(&self, project: String, session: String) -> Reply<()> {
        self.local(move |host| async move { host.dismiss_review(&project, &session).await })
    }

    /// The model profile reviews are made with; empty is the conversation's.
    pub fn review_model(&self) -> Reply<String> {
        self.local(|host| async move { host.review_model().await })
    }

    /// The model profile sub-agents work on; empty is the conversation's.
    pub fn subagent_model(&self) -> Reply<String> {
        self.local(|host| async move { host.subagent_model().await })
    }

    pub fn set_subagent_model(&self, profile: String) -> Reply<String> {
        self.local(move |host| async move { host.set_subagent_model(&profile).await })
    }

    pub fn set_review_model(&self, profile: String) -> Reply<String> {
        self.local(move |host| async move { host.set_review_model(&profile).await })
    }

    /// Puts away the questions suggested after the latest answer.
    pub fn dismiss_follow_ups(&self, project: String, session: String) -> Reply<()> {
        self.local(move |host| async move { host.dismiss_follow_ups(&project, &session).await })
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
