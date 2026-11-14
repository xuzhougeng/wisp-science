//! Interactive UI never blocks on storage, tool execution, or model IO.
//! Final native application termination waits for the host's bounded cleanup.
use std::{future::Future, path::PathBuf, sync::Arc};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::{runtime::Runtime, sync::OnceCell, task::JoinHandle};
use wisp_dto::{
    SessionSearchInfo,
    native_files::{Directory, Preview},
    remote_access::RemoteRequest,
};
use wisp_server::{
    Server,
    config::Config,
    desktop::{
        ConversationList, ConversationOptions, ConversationStatus, FileMatch, ModelInput,
        ModelSettings,
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
    pub fn find_files(&self, query: String, limit: usize) -> Reply<Vec<FileMatch>> {
        self.local(move |host| async move { host.find_files(None, &query, limit).await })
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
        for_conversation: bool,
    ) -> Reply<()> {
        self.local(move |host| async move {
            host.answer_approval(&project, &session, &approval, approved, for_conversation)
                .await
        })
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
