//! Interactive UI never blocks on storage, tool execution, or model IO.
//! Final native application termination waits for the host's bounded cleanup.
use std::{future::Future, path::PathBuf, sync::Arc};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::{runtime::Runtime, sync::OnceCell, task::JoinHandle};
use wisp_dto::remote_access::RemoteRequest;
use wisp_server::{Server, config::Config};
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

    pub fn register_project(&self, path: PathBuf) -> Reply<String> {
        let host = self.host.clone();
        self.spawn(async move {
            host.get()
                .ok_or("Host is not ready")?
                .register_local_project(&path)
                .await
                .map_err(|e| format!("{e:#}"))
        })
    }

    pub async fn shutdown(&self) {
        if let Some(host) = self.host.get() {
            host.shutdown().await;
        }
    }
}
