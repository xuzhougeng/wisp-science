//! Stable logical connection, independent of any one tool future or view.
//! Its owner supplies a factory that rehydrates current credentials on each launch.
use crate::McpClient;
use anyhow::{anyhow, Result};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
use tracing::Instrument;

/// After a failed connect, callers get the cached error until this passes, so an
/// offline endpoint does not add its connect timeout to every agent turn.
// ponytail: fixed window, exponential backoff if one retry a minute still costs too much.
const RETRY_COOLDOWN: Duration = Duration::from_secs(60);

pub type ClientFactory =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<McpClient>> + Send>> + Send + Sync>;

pub struct ManagedConnection {
    identity: (String, String, String),
    factory: ClientFactory,
    current: RwLock<Option<Arc<McpClient>>>,
    connect: Mutex<()>,
    failure: RwLock<Option<(Instant, String)>>,
    generation: AtomicU64,
    registered_generation: AtomicU64,
    catalog_changed: AtomicBool,
    closed: AtomicBool,
}
impl ManagedConnection {
    pub fn new(factory: ClientFactory) -> Self {
        Self {
            identity: Default::default(),
            factory,
            current: RwLock::new(None),
            connect: Mutex::new(()),
            failure: RwLock::new(None),
            generation: AtomicU64::new(0),
            registered_generation: AtomicU64::new(0),
            catalog_changed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }
    pub fn with_identity(mut self, project: &str, frame: &str, connector: &str) -> Self {
        self.identity = (project.into(), frame.into(), connector.into());
        self
    }
    pub fn span(&self) -> tracing::Span {
        tracing::info_span!(target: "wisp", "mcp", project=%self.identity.0, frame=%self.identity.1, connector=%self.identity.2, generation=self.generation())
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    pub fn needs_catalog_refresh(&self) -> bool {
        self.catalog_changed.load(Ordering::SeqCst)
            || (self.recent_failure().is_none()
                && (!self.is_connected()
                    || self.generation() != self.registered_generation.load(Ordering::SeqCst)))
    }
    fn recent_failure(&self) -> Option<String> {
        self.failure
            .read()
            .unwrap()
            .as_ref()
            .filter(|(at, _)| at.elapsed() < RETRY_COOLDOWN)
            .map(|(_, message)| message.clone())
    }
    pub fn mark_catalog_current(&self) {
        self.registered_generation
            .store(self.generation(), Ordering::SeqCst);
        self.catalog_changed.store(false, Ordering::SeqCst);
    }
    pub fn catalog_changed(&self) {
        self.catalog_changed.store(true, Ordering::SeqCst);
    }
    pub fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
            && self
                .current
                .read()
                .unwrap()
                .as_ref()
                .is_some_and(|c| c.is_connected())
    }
    pub async fn ready(&self) -> Result<Arc<McpClient>> {
        let _connect = self.connect.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(anyhow!(
                "MCP connector disabled or replaced; request not sent"
            ));
        }
        let previous = self.current.read().unwrap().clone();
        if let Some(client) = &previous {
            if client.is_connected() {
                return Ok(client.clone());
            }
        }
        // Also answers callers that queued behind the attempt that just failed.
        if let Some(message) = self.recent_failure() {
            return Err(anyhow!(message));
        }
        if let Some(client) = previous {
            let _ = Box::pin(client.shutdown()).await;
        }
        self.current.write().unwrap().take();
        self.generation.fetch_add(1, Ordering::SeqCst);
        tracing::info!(target: "wisp", generation=self.generation(), "mcp.connection.connecting");
        let result = tokio::select! {
            result = async {
                let client = (self.factory)().await?;
                Box::pin(client.tools_list()).await?;
                Ok::<_,anyhow::Error>(client)
            }.instrument(self.span()) => result,
            _ = async { loop {
                if self.closed.load(Ordering::SeqCst) { break; }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            } } => Err(anyhow!("MCP connector closed during initialization")),
        };
        match result {
            Ok(client) => {
                if self.closed.load(Ordering::SeqCst) {
                    let _ = Box::pin(client.shutdown()).await;
                    return Err(anyhow!("MCP connector closed during initialization"));
                }
                let client = Arc::new(client);
                *self.current.write().unwrap() = Some(client.clone());
                self.failure.write().unwrap().take();
                let generation = self.generation();
                tracing::info!(target: "wisp", generation, "mcp.connection.ready");
                Ok(client)
            }
            Err(error) => {
                *self.failure.write().unwrap() = Some((Instant::now(), error.to_string()));
                tracing::warn!(target: "wisp", error = %format!("{error:#}"), cooldown_s = RETRY_COOLDOWN.as_secs(), "mcp.connection.failed");
                Err(error)
            }
        }
    }
    pub async fn shutdown(&self) -> Result<()> {
        // Reject new requests before waiting for a concurrent bounded initialization.
        self.closed.store(true, Ordering::SeqCst);
        let _connect = self.connect.lock().await;
        let client = self.current.write().unwrap().take();
        tracing::info!(target: "wisp", generation=self.generation(), "mcp.connection.shutdown");
        if let Some(client) = client {
            Box::pin(client.shutdown()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test(start_paused = true)]
    async fn failed_connect_is_not_retried_until_cooldown_passes() {
        let launches = Arc::new(AtomicUsize::new(0));
        let counted = launches.clone();
        let factory: ClientFactory = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(anyhow!("connection refused")) })
        });
        let connection = ManagedConnection::new(factory);
        assert!(connection.needs_catalog_refresh());
        assert!(connection.ready().await.is_err());
        // Later turns neither rebuild the catalog nor wait on another connect.
        assert!(!connection.needs_catalog_refresh());
        let Err(error) = connection.ready().await else {
            panic!("cooldown should return the cached error");
        };
        assert_eq!(error.to_string(), "connection refused");
        assert_eq!(launches.load(Ordering::SeqCst), 1);

        tokio::time::advance(RETRY_COOLDOWN).await;
        assert!(connection.needs_catalog_refresh());
        assert!(connection.ready().await.is_err());
        assert_eq!(launches.load(Ordering::SeqCst), 2);
    }
}
