//! Project-scoped interactive runtime ownership and execution serialization.

use crate::{find_rscript, KernelClient, KernelResp, PythonEnv};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot, watch};

pub const LOCAL_CONTEXT_ID: &str = "local";
pub const MAINLINE_RUNTIME_SCOPE: &str = "mainline";
/// Total budget for one stop request, however many runtimes it covers. Stopping
/// runs on agent tool calls, project switches, and app exit, so a worker that
/// refuses to die must cost a bounded wait rather than the whole turn.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLanguage {
    Python,
    R,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeKey {
    pub project_id: String,
    #[serde(default = "default_runtime_scope")]
    pub scope_key: String,
    /// Conversation (frame) that owns this interpreter. Parallel sessions in
    /// one project must not share in-memory Python/R state (#911), so agent
    /// tool calls key their runtime by session. Empty means scope-shared:
    /// user-started runtimes without an owning conversation.
    #[serde(default)]
    pub session_id: String,
    pub context_id: String,
    pub language: RuntimeLanguage,
}

impl RuntimeKey {
    pub fn python(project_id: impl Into<String>, context_id: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
            scope_key: MAINLINE_RUNTIME_SCOPE.into(),
            session_id: String::new(),
            context_id: context_id.into(),
            language: RuntimeLanguage::Python,
        }
    }

    pub fn python_in_scope(
        project_id: impl Into<String>,
        scope_key: impl Into<String>,
        context_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            scope_key: scope_key.into(),
            session_id: String::new(),
            context_id: context_id.into(),
            language: RuntimeLanguage::Python,
        }
    }

    pub fn local_python(project_id: impl Into<String>) -> Self {
        Self::python(project_id, LOCAL_CONTEXT_ID)
    }

    pub fn r(project_id: impl Into<String>, context_id: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
            scope_key: MAINLINE_RUNTIME_SCOPE.into(),
            session_id: String::new(),
            context_id: context_id.into(),
            language: RuntimeLanguage::R,
        }
    }

    pub fn r_in_scope(
        project_id: impl Into<String>,
        scope_key: impl Into<String>,
        context_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            scope_key: scope_key.into(),
            session_id: String::new(),
            context_id: context_id.into(),
            language: RuntimeLanguage::R,
        }
    }

    /// Bind this key to one conversation so its interpreter state is private.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }
}

fn default_runtime_scope() -> String {
    MAINLINE_RUNTIME_SCOPE.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Starting,
    Ready,
    Busy,
    Stopping,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub runtime_id: String,
    pub generation: u64,
    pub key: RuntimeKey,
    pub status: RuntimeStatus,
    pub interpreter: Option<String>,
    pub version: Option<String>,
    pub process_id: Option<u32>,
    pub started_at_ms: u64,
    pub last_activity_at_ms: u64,
    pub resident_memory_bytes: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObject {
    pub name: String,
    pub type_name: String,
    pub summary: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObjectList {
    pub objects: Vec<RuntimeObject>,
    pub total_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeMetadata {
    pub interpreter: Option<String>,
    pub version: Option<String>,
    pub process_id: Option<u32>,
}

pub struct LaunchedRuntime {
    kernel: Box<dyn RuntimeKernel>,
    metadata: RuntimeMetadata,
}

impl LaunchedRuntime {
    pub fn new(kernel: Box<dyn RuntimeKernel>, metadata: RuntimeMetadata) -> Self {
        Self { kernel, metadata }
    }
}

/// A launched, attached interpreter. The manager is its sole owner.
#[async_trait]
pub trait RuntimeKernel: Send {
    async fn execute(
        &mut self,
        id: &str,
        code: &str,
        source_name: Option<&str>,
        required_objects: &[String],
        output: &RuntimeOutput,
    ) -> Result<KernelResp>;

    async fn inspect(&mut self, id: &str) -> Result<RuntimeObjectList>;

    /// Best-effort non-blocking process/transport liveness check while idle.
    fn try_wait(&mut self) -> Result<Option<String>> {
        Ok(None)
    }

    async fn shutdown(&mut self) -> Result<()>;
}

/// Host boundary for starting a worker in the selected execution context.
#[async_trait]
pub trait RuntimeLauncher: Send + Sync {
    async fn launch(&self, key: &RuntimeKey, cwd: &Path) -> Result<LaunchedRuntime>;
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Stdout(String),
    Finished(std::result::Result<KernelResp, String>),
}

#[derive(Clone)]
pub struct RuntimeOutput {
    tx: mpsc::UnboundedSender<RuntimeEvent>,
}

impl RuntimeOutput {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RuntimeEvent>) -> Self {
        Self { tx }
    }

    pub fn stdout(&self, chunk: impl Into<String>) {
        let _ = self.tx.send(RuntimeEvent::Stdout(chunk.into()));
    }

    fn finish(&self, result: std::result::Result<KernelResp, String>) {
        let _ = self.tx.send(RuntimeEvent::Finished(result));
    }
}

#[derive(Debug)]
pub struct RuntimeExecution {
    rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    info: RuntimeInfo,
}

impl RuntimeExecution {
    pub async fn recv(&mut self) -> Option<RuntimeEvent> {
        self.rx.recv().await
    }

    /// The exact runtime generation that accepted this execution. This stays
    /// stable even if the runtime is restarted after the result is returned.
    pub fn info(&self) -> &RuntimeInfo {
        &self.info
    }
}

/// Preconditions and source metadata for one persistent runtime execution.
/// Empty/default options preserve the original lazy-starting cell behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeExecutionOptions {
    /// Project-relative source filename used in interpreter diagnostics.
    pub source_name: Option<String>,
    /// Bindings that must already exist before any source is evaluated.
    pub required_objects: Vec<String>,
    /// Optional optimistic guard against executing in a replacement runtime.
    pub expected_generation: Option<u64>,
}

#[derive(Clone)]
pub struct RuntimeManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    launcher: Arc<dyn RuntimeLauncher>,
    registry: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<RuntimeKey, Arc<RuntimeSession>>,
    generations: HashMap<RuntimeKey, u64>,
}

struct RuntimeSession {
    cwd: PathBuf,
    requests: mpsc::UnboundedSender<RuntimeRequest>,
    stop: watch::Sender<bool>,
    info: watch::Receiver<RuntimeInfo>,
}

struct ExecuteRequest {
    id: String,
    code: String,
    source_name: Option<String>,
    required_objects: Vec<String>,
    output: RuntimeOutput,
}

struct InspectRequest {
    id: String,
    reply: oneshot::Sender<std::result::Result<RuntimeObjectList, String>>,
}

enum RuntimeRequest {
    Execute(ExecuteRequest),
    Inspect(InspectRequest),
}

impl RuntimeRequest {
    fn fail(self, message: &str) {
        match self {
            Self::Execute(request) => request.output.finish(Err(message.to_string())),
            Self::Inspect(request) => {
                let _ = request.reply.send(Err(message.to_string()));
            }
        }
    }
}

impl RuntimeSession {
    fn info(&self) -> RuntimeInfo {
        self.info.borrow().clone()
    }

    fn request_stop(&self) {
        let _ = self.stop.send(true);
    }

    async fn wait_started(&self) -> Result<RuntimeInfo> {
        let mut info = self.info.clone();
        loop {
            let current = info.borrow().clone();
            match current.status {
                RuntimeStatus::Ready | RuntimeStatus::Busy => return Ok(current),
                RuntimeStatus::Dead => {
                    return Err(anyhow!(
                        "{}; restart the runtime to create a new empty session",
                        current.last_error.as_deref().unwrap_or("runtime is dead")
                    ));
                }
                RuntimeStatus::Stopping => return Err(anyhow!("runtime is stopping")),
                RuntimeStatus::Starting => {}
            }
            info.changed()
                .await
                .map_err(|_| anyhow!("runtime startup task ended unexpectedly"))?;
        }
    }

    async fn wait_dead(&self) -> RuntimeInfo {
        let mut info = self.info.clone();
        loop {
            let current = info.borrow().clone();
            if current.status == RuntimeStatus::Dead {
                return current;
            }
            if info.changed().await.is_err() {
                return info.borrow().clone();
            }
        }
    }

    /// Wait for `Dead` until `deadline`, then report the last known state. The
    /// caller keeps going either way: a driver still stuck in worker shutdown
    /// must not hold up the conversation, the project switch, or app exit.
    async fn wait_dead_until(&self, deadline: tokio::time::Instant) -> RuntimeInfo {
        tokio::time::timeout_at(deadline, self.wait_dead())
            .await
            .unwrap_or_else(|_| self.info())
    }
}

impl RuntimeManager {
    pub fn new(launcher: Arc<dyn RuntimeLauncher>) -> Self {
        Self {
            inner: Arc::new(ManagerInner {
                launcher,
                registry: Mutex::new(Registry::default()),
            }),
        }
    }

    /// Build a local launcher for the two explicit runtime adapters.
    pub fn local(
        app_data: PathBuf,
        python_worker: PathBuf,
        r_worker: Option<PathBuf>,
        envs: Vec<(String, String)>,
    ) -> Self {
        Self::new(Arc::new(LocalRuntimeLauncher {
            app_data,
            python_worker,
            r_worker,
            envs,
        }))
    }

    /// Compatibility constructor for callers that only wire local Python.
    pub fn local_python(app_data: PathBuf, worker: PathBuf, envs: Vec<(String, String)>) -> Self {
        Self::local(app_data, worker, None, envs)
    }

    pub async fn start(&self, key: RuntimeKey, cwd: PathBuf) -> Result<RuntimeInfo> {
        self.session(key, cwd)?.wait_started().await
    }

    pub async fn execute(
        &self,
        key: &RuntimeKey,
        cwd: &Path,
        code: impl Into<String>,
    ) -> Result<RuntimeExecution> {
        self.execute_with_options(key, cwd, code, RuntimeExecutionOptions::default())
            .await
    }

    pub async fn execute_with_options(
        &self,
        key: &RuntimeKey,
        cwd: &Path,
        code: impl Into<String>,
        options: RuntimeExecutionOptions,
    ) -> Result<RuntimeExecution> {
        let requires_existing =
            options.expected_generation.is_some() || !options.required_objects.is_empty();
        let session = if requires_existing {
            let session = self
                .registry()
                .sessions
                .get(key)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "runtime is not started; load the required objects before executing this source"
                    )
                })?;
            // The registry lookup bypasses `session`, so this branch is the only
            // place that still has to reject a runtime started elsewhere.
            ensure_session_cwd(key, &session, cwd)?;
            session
        } else {
            self.session(key.clone(), cwd.to_path_buf())?
        };
        let info = session.wait_started().await?;
        if let Some(expected) = options.expected_generation {
            if info.generation != expected {
                return Err(anyhow!(
                    "runtime generation changed: expected {expected}, current {}; reload or rebind the required objects explicitly",
                    info.generation
                ));
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let request = ExecuteRequest {
            id: uuid::Uuid::new_v4().to_string(),
            code: code.into(),
            source_name: options.source_name,
            required_objects: options.required_objects,
            output: RuntimeOutput::new(tx),
        };
        session
            .requests
            .send(RuntimeRequest::Execute(request))
            .map_err(|_| anyhow!("runtime request queue is closed"))?;
        Ok(RuntimeExecution { rx, info })
    }

    pub async fn inspect(&self, key: &RuntimeKey) -> Result<RuntimeObjectList> {
        let session = self
            .registry()
            .sessions
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow!("runtime is not started"))?;
        session.wait_started().await?;
        let (reply, result) = oneshot::channel();
        session
            .requests
            .send(RuntimeRequest::Inspect(InspectRequest {
                id: uuid::Uuid::new_v4().to_string(),
                reply,
            }))
            .map_err(|_| anyhow!("runtime request queue is closed"))?;
        result
            .await
            .map_err(|_| anyhow!("runtime inspection ended without a result"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn list(&self) -> Vec<RuntimeInfo> {
        let registry = self.registry();
        let mut infos = registry
            .sessions
            .values()
            .map(|session| session.info())
            .collect::<Vec<_>>();
        infos.sort_by(|a, b| {
            a.key
                .project_id
                .cmp(&b.key.project_id)
                .then_with(|| a.key.scope_key.cmp(&b.key.scope_key))
                .then_with(|| a.key.session_id.cmp(&b.key.session_id))
                .then_with(|| a.key.context_id.cmp(&b.key.context_id))
                .then_with(|| a.key.language.cmp(&b.key.language))
        });
        infos
    }

    /// Stop one runtime but keep its dead record, so the next call reports the
    /// stop instead of silently starting a replacement.
    pub async fn stop(&self, key: &RuntimeKey) -> Option<RuntimeInfo> {
        let session = { self.registry().sessions.get(key).cloned()? };
        self.stop_sessions(&[(key.clone(), session)]).await.pop()
    }

    pub async fn restart(&self, key: RuntimeKey, cwd: PathBuf) -> Result<RuntimeInfo> {
        let current = { self.registry().sessions.get(&key).cloned() };
        if let Some(session) = current {
            let sessions = [(key.clone(), session)];
            self.stop_sessions(&sessions).await;
            self.forget_sessions(&sessions);
        }
        self.start(key, cwd).await
    }

    pub async fn stop_project(&self, project_id: &str) {
        self.stop_and_forget(|key| key.project_id == project_id)
            .await;
    }

    /// Stop every runtime owned by one conversation, e.g. when the session is
    /// deleted or a delegated subagent completes. Scope-shared runtimes
    /// (empty session) are never matched.
    pub async fn stop_session(&self, project_id: &str, session_id: &str) {
        if session_id.is_empty() {
            return;
        }
        self.stop_and_forget(|key| key.project_id == project_id && key.session_id == session_id)
            .await;
    }

    pub async fn stop_scope(&self, project_id: &str, scope_key: &str) {
        self.stop_and_forget(|key| key.project_id == project_id && key.scope_key == scope_key)
            .await;
    }

    pub async fn shutdown_all(&self) {
        self.stop_and_forget(|_| true).await;
    }

    async fn stop_and_forget(&self, matches: impl Fn(&RuntimeKey) -> bool) {
        let sessions = {
            self.registry()
                .sessions
                .iter()
                .filter(|(key, _)| matches(key))
                .map(|(key, session)| (key.clone(), session.clone()))
                .collect::<Vec<_>>()
        };
        self.stop_sessions(&sessions).await;
        self.forget_sessions(&sessions);
    }

    /// Ask every session to stop and wait out one shared budget. Returning on
    /// the deadline is what keeps a worker that refuses to exit from blocking
    /// the agent turn, the project switch, or app exit.
    async fn stop_sessions(
        &self,
        sessions: &[(RuntimeKey, Arc<RuntimeSession>)],
    ) -> Vec<RuntimeInfo> {
        for (_, session) in sessions {
            session.request_stop();
        }
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        let mut stopped = Vec::with_capacity(sessions.len());
        for (_, session) in sessions {
            stopped.push(session.wait_dead_until(deadline).await);
        }
        stopped
    }

    fn forget_sessions(&self, sessions: &[(RuntimeKey, Arc<RuntimeSession>)]) {
        let mut registry = self.registry();
        for (key, session) in sessions {
            if registry
                .sessions
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, session))
            {
                registry.sessions.remove(key);
            }
        }
    }

    fn session(&self, key: RuntimeKey, cwd: PathBuf) -> Result<Arc<RuntimeSession>> {
        let mut registry = self.registry();
        if let Some(session) = registry.sessions.get(&key) {
            ensure_session_cwd(&key, session, &cwd)?;
            return Ok(session.clone());
        }

        let generation = registry.generations.entry(key.clone()).or_default();
        *generation += 1;
        let now = now_ms();
        let initial = RuntimeInfo {
            runtime_id: uuid::Uuid::new_v4().to_string(),
            generation: *generation,
            key: key.clone(),
            status: RuntimeStatus::Starting,
            interpreter: None,
            version: None,
            process_id: None,
            started_at_ms: now,
            last_activity_at_ms: now,
            resident_memory_bytes: None,
            last_error: None,
        };
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = watch::channel(false);
        let (info_tx, info_rx) = watch::channel(initial);
        let session = Arc::new(RuntimeSession {
            cwd: cwd.clone(),
            requests: request_tx,
            stop: stop_tx,
            info: info_rx,
        });
        registry.sessions.insert(key.clone(), session.clone());
        drop(registry);

        tokio::spawn(runtime_driver(
            self.inner.launcher.clone(),
            key,
            cwd,
            request_rx,
            stop_rx,
            info_tx,
        ));
        Ok(session)
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct LocalRuntimeLauncher {
    app_data: PathBuf,
    python_worker: PathBuf,
    r_worker: Option<PathBuf>,
    envs: Vec<(String, String)>,
}

#[async_trait]
impl RuntimeLauncher for LocalRuntimeLauncher {
    async fn launch(&self, key: &RuntimeKey, cwd: &Path) -> Result<LaunchedRuntime> {
        if key.context_id != LOCAL_CONTEXT_ID {
            return Err(anyhow!("local launcher only supports the local context"));
        }

        let (interpreter, args, envs, language) = match key.language {
            RuntimeLanguage::Python => {
                let python = PythonEnv::managed(&self.app_data).python();
                if !python.is_file() {
                    return Err(anyhow!(
                        "managed Python interpreter not found at {}; wait for Python bootstrap",
                        python.display()
                    ));
                }
                if !self.python_worker.is_file() {
                    return Err(anyhow!(
                        "Python runtime worker not found at {}",
                        self.python_worker.display()
                    ));
                }
                (
                    python,
                    vec![self.python_worker.as_os_str().to_os_string()],
                    self.envs.clone(),
                    "python",
                )
            }
            RuntimeLanguage::R => {
                let rscript = find_rscript()
                    .map(|path| crate::direct_rscript(&path))
                    .ok_or_else(|| {
                        anyhow!("Rscript not found on PATH; install R and add Rscript to PATH")
                    })?;
                let worker = self
                    .r_worker
                    .as_ref()
                    .ok_or_else(|| anyhow!("R runtime worker is not configured for this host"))?;
                if !worker.is_file() {
                    return Err(anyhow!(
                        "R runtime worker not found at {}",
                        worker.display()
                    ));
                }
                let envs = crate::conda_prefix_envs(&rscript);
                (
                    rscript,
                    vec![
                        OsString::from("--vanilla"),
                        worker.as_os_str().to_os_string(),
                    ],
                    envs,
                    "r",
                )
            }
        };
        let client =
            KernelClient::spawn_command(&interpreter, &args, &envs, Some(cwd), language).await?;
        let ready = client.ready().clone();
        Ok(LaunchedRuntime::new(
            Box::new(client),
            RuntimeMetadata {
                interpreter: Some(interpreter.to_string_lossy().into_owned()),
                version: Some(ready.version),
                process_id: Some(ready.pid),
            },
        ))
    }
}

enum ExecuteOutcome {
    Completed(Result<KernelResp>),
    Stop,
}

enum InspectOutcome {
    Completed(Result<RuntimeObjectList>),
    Stop,
}

async fn runtime_driver(
    launcher: Arc<dyn RuntimeLauncher>,
    key: RuntimeKey,
    cwd: PathBuf,
    mut requests: mpsc::UnboundedReceiver<RuntimeRequest>,
    mut stop: watch::Receiver<bool>,
    info_tx: watch::Sender<RuntimeInfo>,
) {
    let mut info = info_tx.borrow().clone();
    let launched = match launcher.launch(&key, &cwd).await {
        Ok(launched) => launched,
        Err(error) => {
            let message = error.to_string();
            info.status = RuntimeStatus::Dead;
            info.last_activity_at_ms = now_ms();
            info.last_error = Some(message.clone());
            info_tx.send_replace(info);
            fail_pending(&mut requests, &message);
            return;
        }
    };
    info.status = RuntimeStatus::Ready;
    info.interpreter = launched.metadata.interpreter;
    info.version = launched.metadata.version;
    info.process_id = launched.metadata.process_id;
    info.last_activity_at_ms = now_ms();
    info_tx.send_replace(info.clone());

    let mut kernel = launched.kernel;
    let mut stop_message = None;
    let mut status_poll = tokio::time::interval(std::time::Duration::from_millis(250));
    status_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let request = tokio::select! {
            biased;
            changed = stop.changed() => {
                let _ = changed;
                break;
            }
            request = requests.recv() => match request {
                Some(request) => request,
                None => break,
            },
            _ = status_poll.tick() => {
                match kernel.try_wait() {
                    Ok(Some(status)) => {
                        let message = format!("runtime process exited unexpectedly ({status})");
                        info.status = RuntimeStatus::Dead;
                        info.last_activity_at_ms = now_ms();
                        info.last_error = Some(message.clone());
                        info_tx.send_replace(info);
                        fail_pending(&mut requests, &message);
                        return;
                    }
                    Err(error) => {
                        let message = format!("runtime process status failed: {error}");
                        info.status = RuntimeStatus::Dead;
                        info.last_activity_at_ms = now_ms();
                        info.last_error = Some(message.clone());
                        info_tx.send_replace(info);
                        fail_pending(&mut requests, &message);
                        return;
                    }
                    Ok(None) => continue,
                }
            }
        };

        info.status = RuntimeStatus::Busy;
        info.last_activity_at_ms = now_ms();
        info_tx.send_replace(info.clone());
        match request {
            RuntimeRequest::Execute(request) => {
                let outcome = {
                    let execution = kernel.execute(
                        &request.id,
                        &request.code,
                        request.source_name.as_deref(),
                        &request.required_objects,
                        &request.output,
                    );
                    tokio::pin!(execution);
                    tokio::select! {
                        biased;
                        changed = stop.changed() => {
                            let _ = changed;
                            ExecuteOutcome::Stop
                        }
                        result = &mut execution => ExecuteOutcome::Completed(result),
                    }
                };

                match outcome {
                    ExecuteOutcome::Completed(Ok(response)) => {
                        if response.rss_kb > 0 {
                            info.resident_memory_bytes = Some(response.rss_kb.saturating_mul(1024));
                        }
                        info.status = RuntimeStatus::Ready;
                        info.last_activity_at_ms = now_ms();
                        info_tx.send_replace(info.clone());
                        request.output.finish(Ok(response));
                    }
                    ExecuteOutcome::Completed(Err(error)) => {
                        let message = error.to_string();
                        info.status = RuntimeStatus::Dead;
                        info.last_activity_at_ms = now_ms();
                        info.last_error = Some(message.clone());
                        info_tx.send_replace(info.clone());
                        request.output.finish(Err(message.clone()));
                        fail_pending(&mut requests, &message);
                        let _ = kernel.shutdown().await;
                        return;
                    }
                    ExecuteOutcome::Stop => {
                        let message = "runtime stopped while executing".to_string();
                        request.output.finish(Err(message.clone()));
                        stop_message = Some(message);
                        break;
                    }
                }
            }
            RuntimeRequest::Inspect(request) => {
                let outcome = {
                    let inspection = kernel.inspect(&request.id);
                    tokio::pin!(inspection);
                    tokio::select! {
                        biased;
                        changed = stop.changed() => {
                            let _ = changed;
                            InspectOutcome::Stop
                        }
                        result = &mut inspection => InspectOutcome::Completed(result),
                    }
                };

                match outcome {
                    InspectOutcome::Completed(Ok(objects)) => {
                        info.status = RuntimeStatus::Ready;
                        info.last_activity_at_ms = now_ms();
                        info_tx.send_replace(info.clone());
                        let _ = request.reply.send(Ok(objects));
                    }
                    InspectOutcome::Completed(Err(error)) => {
                        let message = error.to_string();
                        info.status = RuntimeStatus::Dead;
                        info.last_activity_at_ms = now_ms();
                        info.last_error = Some(message.clone());
                        info_tx.send_replace(info.clone());
                        let _ = request.reply.send(Err(message.clone()));
                        fail_pending(&mut requests, &message);
                        let _ = kernel.shutdown().await;
                        return;
                    }
                    InspectOutcome::Stop => {
                        let message = "runtime stopped while inspecting".to_string();
                        let _ = request.reply.send(Err(message.clone()));
                        stop_message = Some(message);
                        break;
                    }
                }
            }
        }
    }

    info.status = RuntimeStatus::Stopping;
    info.last_activity_at_ms = now_ms();
    info_tx.send_replace(info.clone());
    let shutdown_error = kernel.shutdown().await.err().map(|error| error.to_string());
    let pending_error = stop_message.unwrap_or_else(|| "runtime stopped".into());
    fail_pending(&mut requests, &pending_error);
    info.status = RuntimeStatus::Dead;
    info.last_activity_at_ms = now_ms();
    if let Some(error) = shutdown_error {
        info.last_error = Some(error);
    }
    info_tx.send_replace(info);
}

/// A live runtime is bound to the directory it was launched in; reusing it from
/// another root would silently resolve relative paths against the wrong project.
fn ensure_session_cwd(key: &RuntimeKey, session: &RuntimeSession, cwd: &Path) -> Result<()> {
    if session.cwd == cwd {
        return Ok(());
    }
    Err(anyhow!(
        "runtime for project '{}' was started in '{}', not '{}'",
        key.project_id,
        session.cwd.display(),
        cwd.display()
    ))
}

fn fail_pending(requests: &mut mpsc::UnboundedReceiver<RuntimeRequest>, message: &str) {
    while let Ok(request) = requests.try_recv() {
        request.fail(message);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    #[derive(Clone, Default)]
    struct FakeLauncher {
        launches: Arc<AtomicUsize>,
        shutdowns: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        exited: Arc<AtomicBool>,
        stuck_shutdown: Arc<AtomicBool>,
        launch_cwds: Arc<Mutex<Vec<PathBuf>>>,
        launch_delay: Duration,
    }

    #[async_trait]
    impl RuntimeLauncher for FakeLauncher {
        async fn launch(&self, _key: &RuntimeKey, cwd: &Path) -> Result<LaunchedRuntime> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            self.launch_cwds.lock().unwrap().push(cwd.to_path_buf());
            sleep(self.launch_delay).await;
            Ok(LaunchedRuntime::new(
                Box::new(FakeKernel {
                    value: 0,
                    shutdowns: self.shutdowns.clone(),
                    active: self.active.clone(),
                    max_active: self.max_active.clone(),
                    exited: self.exited.clone(),
                    stuck_shutdown: self.stuck_shutdown.clone(),
                }),
                RuntimeMetadata {
                    interpreter: Some("fake-python".into()),
                    version: Some("test".into()),
                    process_id: None,
                },
            ))
        }
    }

    struct FakeKernel {
        value: i64,
        shutdowns: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        exited: Arc<AtomicBool>,
        stuck_shutdown: Arc<AtomicBool>,
    }

    struct ActiveCall(Arc<AtomicUsize>);

    impl Drop for ActiveCall {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl RuntimeKernel for FakeKernel {
        async fn execute(
            &mut self,
            _id: &str,
            code: &str,
            _source_name: Option<&str>,
            required_objects: &[String],
            output: &RuntimeOutput,
        ) -> Result<KernelResp> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let _active = ActiveCall(self.active.clone());

            let missing = required_objects
                .iter()
                .filter(|name| name.as_str() != "value")
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Ok(KernelResp {
                    error: Some(format!(
                        "required runtime objects are missing: {}",
                        missing.join(", ")
                    )),
                    ..KernelResp::default()
                });
            }

            if let Some(value) = code.strip_prefix("set_after_delay:") {
                sleep(Duration::from_millis(40)).await;
                self.value = value.parse()?;
            } else if let Some(value) = code.strip_prefix("set:") {
                self.value = value.parse()?;
            } else if code == "increment_after_delay" {
                sleep(Duration::from_millis(40)).await;
                self.value += 1;
            } else if code == "delay" {
                sleep(Duration::from_millis(40)).await;
            } else if code == "fail_after_delay" {
                sleep(Duration::from_millis(40)).await;
                bail!("fake protocol failure");
            } else if code == "stream" {
                output.stdout("chunk");
            }

            Ok(KernelResp {
                stdout: if code == "get" {
                    self.value.to_string()
                } else {
                    String::new()
                },
                ..KernelResp::default()
            })
        }

        async fn inspect(&mut self, _id: &str) -> Result<RuntimeObjectList> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let _active = ActiveCall(self.active.clone());
            Ok(RuntimeObjectList {
                objects: vec![RuntimeObject {
                    name: "value".into(),
                    type_name: "integer".into(),
                    summary: self.value.to_string(),
                    size_bytes: Some(std::mem::size_of_val(&self.value) as u64),
                }],
                total_count: 1,
            })
        }

        async fn shutdown(&mut self) -> Result<()> {
            if self.stuck_shutdown.load(Ordering::SeqCst) {
                // A worker whose descendants keep the inherited stderr pipe
                // open never reports that it finished shutting down (#941).
                std::future::pending::<()>().await;
            }
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn try_wait(&mut self) -> Result<Option<String>> {
            Ok(self
                .exited
                .load(Ordering::SeqCst)
                .then(|| "fake exit".to_string()))
        }
    }

    async fn finished(mut execution: RuntimeExecution) -> std::result::Result<KernelResp, String> {
        loop {
            match execution.recv().await {
                Some(RuntimeEvent::Stdout(_)) => {}
                Some(RuntimeEvent::Finished(result)) => return result,
                None => return Err("execution channel closed".into()),
            }
        }
    }

    fn manager(launcher: &FakeLauncher) -> RuntimeManager {
        RuntimeManager::new(Arc::new(launcher.clone()))
    }

    #[tokio::test]
    async fn concurrent_start_for_one_key_launches_once() {
        let launcher = FakeLauncher {
            launch_delay: Duration::from_millis(30),
            ..FakeLauncher::default()
        };
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let (first, second) = tokio::join!(
            manager.start(key.clone(), cwd.clone()),
            manager.start(key, cwd.clone())
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.runtime_id, second.runtime_id);
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
        assert_eq!(launcher.launch_cwds.lock().unwrap().as_slice(), &[cwd]);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn one_runtime_is_ordered_while_different_keys_run_independently() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key_a = RuntimeKey::local_python("project-a");
        let key_b = RuntimeKey::local_python("project-b");
        let cwd_a = PathBuf::from("project-a");
        let cwd_b = PathBuf::from("project-b");

        let first = manager
            .execute(&key_a, &cwd_a, "set_after_delay:7")
            .await
            .unwrap();
        let second = manager.execute(&key_a, &cwd_a, "get").await.unwrap();
        let (_, second) = tokio::join!(finished(first), finished(second));
        assert_eq!(second.unwrap().stdout, "7");
        assert_eq!(launcher.max_active.load(Ordering::SeqCst), 1);

        launcher.max_active.store(0, Ordering::SeqCst);
        let a = manager.execute(&key_a, &cwd_a, "delay").await.unwrap();
        let b = manager.execute(&key_b, &cwd_b, "delay").await.unwrap();
        let _ = tokio::join!(finished(a), finished(b));
        assert_eq!(launcher.max_active.load(Ordering::SeqCst), 2);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn mainline_and_exploration_runtime_state_is_independent() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let mainline = RuntimeKey::local_python("project-a");
        let exploration =
            RuntimeKey::python_in_scope("project-a", "exploration-a", LOCAL_CONTEXT_ID);
        let cwd = PathBuf::from("project-a");

        finished(manager.execute(&mainline, &cwd, "set:3").await.unwrap())
            .await
            .unwrap();
        finished(manager.execute(&exploration, &cwd, "set:7").await.unwrap())
            .await
            .unwrap();

        let mainline_value = finished(manager.execute(&mainline, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        let exploration_value = finished(manager.execute(&exploration, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        assert_eq!(mainline_value.stdout, "3");
        assert_eq!(exploration_value.stdout, "7");
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn parallel_sessions_in_one_scope_keep_independent_runtime_state() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let session_a = RuntimeKey::local_python("project-a").with_session("frame-a");
        let session_b = RuntimeKey::local_python("project-a").with_session("frame-b");
        let cwd = PathBuf::from("project-a");

        finished(manager.execute(&session_a, &cwd, "set:3").await.unwrap())
            .await
            .unwrap();
        finished(manager.execute(&session_b, &cwd, "set:7").await.unwrap())
            .await
            .unwrap();

        let value_a = finished(manager.execute(&session_a, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        let value_b = finished(manager.execute(&session_b, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        assert_eq!(value_a.stdout, "3");
        assert_eq!(value_b.stdout, "7");
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn stopping_a_session_keeps_sibling_and_scope_shared_runtimes() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let shared = RuntimeKey::local_python("project-a");
        let session_a = RuntimeKey::local_python("project-a").with_session("frame-a");
        let session_b = RuntimeKey::local_python("project-a").with_session("frame-b");
        let cwd = PathBuf::from("project-a");
        for key in [&shared, &session_a, &session_b] {
            manager.start(key.clone(), cwd.clone()).await.unwrap();
        }

        manager.stop_session("project-a", "frame-a").await;
        let remaining = manager
            .list()
            .into_iter()
            .map(|runtime| runtime.key)
            .collect::<Vec<_>>();
        assert!(remaining.contains(&shared));
        assert!(remaining.contains(&session_b));
        assert!(!remaining.contains(&session_a));

        // An empty session id must never match the scope-shared runtime.
        manager.stop_session("project-a", "").await;
        assert_eq!(manager.list().len(), 2);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn stopping_a_scope_stops_its_session_runtimes_too() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let mainline_session = RuntimeKey::local_python("project-a").with_session("frame-a");
        let exploration_session =
            RuntimeKey::python_in_scope("project-a", "exploration-a", LOCAL_CONTEXT_ID)
                .with_session("frame-b");
        let cwd = PathBuf::from("project-a");
        for key in [&mainline_session, &exploration_session] {
            manager.start(key.clone(), cwd.clone()).await.unwrap();
        }

        manager
            .stop_scope("project-a", MAINLINE_RUNTIME_SCOPE)
            .await;
        let remaining = manager
            .list()
            .into_iter()
            .map(|runtime| runtime.key)
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![exploration_session]);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn fake_python_and_r_workers_keep_independent_persistent_state() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let python = RuntimeKey::local_python("project-a");
        let r = RuntimeKey::r("project-a", LOCAL_CONTEXT_ID);
        let cwd = PathBuf::from("project-a");

        finished(manager.execute(&python, &cwd, "set:3").await.unwrap())
            .await
            .unwrap();
        finished(manager.execute(&r, &cwd, "set:7").await.unwrap())
            .await
            .unwrap();

        let python_value = finished(manager.execute(&python, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        let r_value = finished(manager.execute(&r, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        assert_eq!(python_value.stdout, "3");
        assert_eq!(r_value.stdout, "7");
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn required_objects_never_lazy_start_an_empty_runtime() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");

        let error = manager
            .execute_with_options(
                &key,
                &cwd,
                "get",
                RuntimeExecutionOptions {
                    required_objects: vec!["value".into()],
                    ..RuntimeExecutionOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("runtime is not started"));
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
    }

    /// The guarded path looks the session up straight out of the registry, which
    /// skips the directory check that `session` applies on the lazy path.
    #[tokio::test]
    async fn a_guarded_execution_still_rejects_a_runtime_from_another_root() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        manager
            .start(key.clone(), PathBuf::from("project-a"))
            .await
            .unwrap();

        let error = manager
            .execute_with_options(
                &key,
                Path::new("project-b"),
                "get",
                RuntimeExecutionOptions {
                    required_objects: vec!["value".into()],
                    ..RuntimeExecutionOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("was started in"),
            "{error}, expected the started-elsewhere rejection"
        );
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn required_objects_are_checked_before_source_execution() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        manager.start(key.clone(), cwd.clone()).await.unwrap();

        let execution = manager
            .execute_with_options(
                &key,
                &cwd,
                "set:9",
                RuntimeExecutionOptions {
                    required_objects: vec!["sce".into()],
                    ..RuntimeExecutionOptions::default()
                },
            )
            .await
            .unwrap();
        let response = finished(execution).await.unwrap();
        assert_eq!(
            response.error.as_deref(),
            Some("required runtime objects are missing: sce")
        );

        let value = finished(manager.execute(&key, &cwd, "get").await.unwrap())
            .await
            .unwrap();
        assert_eq!(value.stdout, "0", "guarded source must not run");
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn expected_generation_rejects_a_replacement_runtime() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let first = manager.start(key.clone(), cwd.clone()).await.unwrap();
        let second = manager.restart(key.clone(), cwd.clone()).await.unwrap();
        assert!(second.generation > first.generation);

        let error = manager
            .execute_with_options(
                &key,
                &cwd,
                "get",
                RuntimeExecutionOptions {
                    expected_generation: Some(first.generation),
                    ..RuntimeExecutionOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("runtime generation changed"));

        let execution = manager
            .execute_with_options(
                &key,
                &cwd,
                "get",
                RuntimeExecutionOptions {
                    expected_generation: Some(second.generation),
                    ..RuntimeExecutionOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(execution.info().generation, second.generation);
        finished(execution).await.unwrap();
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn inspection_uses_the_same_serialized_persistent_state() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");

        let execution = manager
            .execute(&key, &cwd, "set_after_delay:7")
            .await
            .unwrap();
        let (execution, objects) = tokio::join!(finished(execution), manager.inspect(&key));
        execution.unwrap();
        let objects = objects.unwrap();
        assert_eq!(objects.total_count, 1);
        assert_eq!(objects.objects[0].name, "value");
        assert_eq!(objects.objects[0].summary, "7");
        assert_eq!(launcher.max_active.load(Ordering::SeqCst), 1);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn inspecting_a_missing_runtime_does_not_start_one() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let error = manager
            .inspect(&RuntimeKey::local_python("project-a"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not started"));
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropping_a_caller_does_not_cancel_or_desynchronize_the_runtime() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let abandoned = manager
            .execute(&key, &cwd, "increment_after_delay")
            .await
            .unwrap();
        drop(abandoned);

        let next = manager.execute(&key, &cwd, "get").await.unwrap();
        assert_eq!(finished(next).await.unwrap().stdout, "1");
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn stop_and_restart_replace_the_generation_and_clear_state() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let set = manager.execute(&key, &cwd, "set:9").await.unwrap();
        finished(set).await.unwrap();
        let first = manager.list().pop().unwrap();

        let running = manager.execute(&key, &cwd, "delay").await.unwrap();
        let stopped = manager.stop(&key).await.unwrap();
        assert_eq!(stopped.status, RuntimeStatus::Dead);
        assert!(finished(running)
            .await
            .unwrap_err()
            .contains("runtime stopped"));
        assert!(manager.execute(&key, &cwd, "get").await.is_err());
        let restarted = manager.restart(key.clone(), cwd.clone()).await.unwrap();
        assert_eq!(restarted.generation, first.generation + 1);
        assert_ne!(restarted.runtime_id, first.runtime_id);
        let get = manager.execute(&key, &cwd, "get").await.unwrap();
        assert_eq!(finished(get).await.unwrap().stdout, "0");
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        manager.shutdown_all().await;
    }

    /// Stopping runs inside agent tool calls, project switches, and app exit,
    /// so it must complete on a budget even when the worker never does (#941).
    #[tokio::test(start_paused = true)]
    async fn stopping_a_stuck_worker_gives_up_on_the_budget() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        manager.start(key.clone(), cwd.clone()).await.unwrap();
        launcher.stuck_shutdown.store(true, Ordering::SeqCst);

        let started = tokio::time::Instant::now();
        let stopped = manager.stop(&key).await.unwrap();
        assert!(started.elapsed() >= STOP_TIMEOUT);
        assert_eq!(stopped.status, RuntimeStatus::Stopping);
        assert_eq!(launcher.shutdowns.load(Ordering::SeqCst), 0);

        // An explicit stop keeps its record, so the next call reports the stop
        // instead of quietly starting a second interpreter.
        assert!(manager.execute(&key, &cwd, "get").await.is_err());
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn restarting_past_a_stuck_worker_still_yields_a_fresh_runtime() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let first = manager.start(key.clone(), cwd.clone()).await.unwrap();
        launcher.stuck_shutdown.store(true, Ordering::SeqCst);

        let restarted = manager.restart(key.clone(), cwd.clone()).await.unwrap();
        assert_eq!(restarted.generation, first.generation + 1);
        assert_eq!(restarted.status, RuntimeStatus::Ready);
        assert_eq!(manager.list().len(), 1);
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
    }

    /// One stuck worker per project must not add up: app exit spends a single
    /// budget for every runtime it stops.
    #[tokio::test(start_paused = true)]
    async fn app_exit_shares_one_budget_across_stuck_runtimes() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        for project in ["project-a", "project-b", "project-c"] {
            manager
                .start(RuntimeKey::local_python(project), PathBuf::from(project))
                .await
                .unwrap();
        }
        launcher.stuck_shutdown.store(true, Ordering::SeqCst);

        let started = tokio::time::Instant::now();
        manager.shutdown_all().await;
        assert!(started.elapsed() >= STOP_TIMEOUT);
        assert!(started.elapsed() < STOP_TIMEOUT * 2);
        assert!(manager.list().is_empty());
    }

    #[tokio::test]
    async fn kernel_failure_marks_dead_and_fails_queued_calls() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        let cwd = PathBuf::from("project-a");
        let failing = manager
            .execute(&key, &cwd, "fail_after_delay")
            .await
            .unwrap();
        let queued = manager.execute(&key, &cwd, "get").await.unwrap();
        assert!(finished(failing)
            .await
            .unwrap_err()
            .contains("fake protocol failure"));
        assert!(finished(queued)
            .await
            .unwrap_err()
            .contains("fake protocol failure"));
        let info = manager.list().pop().unwrap();
        assert_eq!(info.status, RuntimeStatus::Dead);
        assert!(info
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("fake protocol failure")));
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn idle_worker_exit_transitions_the_runtime_to_dead() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key = RuntimeKey::local_python("project-a");
        manager
            .start(key, PathBuf::from("project-a"))
            .await
            .unwrap();
        launcher.exited.store(true, Ordering::SeqCst);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager.list()[0].status == RuntimeStatus::Dead {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(manager.list()[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("fake exit")));
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn stopping_a_project_removes_only_its_runtimes() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let key_a = RuntimeKey::local_python("project-a");
        let key_b = RuntimeKey::local_python("project-b");
        manager
            .start(key_a, PathBuf::from("project-a"))
            .await
            .unwrap();
        manager
            .start(key_b.clone(), PathBuf::from("project-b"))
            .await
            .unwrap();

        manager.stop_project("project-a").await;
        let remaining = manager.list();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key, key_b);
        manager.shutdown_all().await;
        assert_eq!(launcher.shutdowns.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stopping_a_scope_keeps_sibling_runtime_state() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        let mainline = RuntimeKey::local_python("project-a");
        let selected = RuntimeKey::python_in_scope("project-a", "exploration-a", LOCAL_CONTEXT_ID);
        let sibling = RuntimeKey::python_in_scope("project-a", "exploration-b", LOCAL_CONTEXT_ID);
        for key in [&mainline, &selected, &sibling] {
            manager
                .start(key.clone(), PathBuf::from(&key.scope_key))
                .await
                .unwrap();
        }

        manager.stop_scope("project-a", "exploration-a").await;
        let remaining = manager
            .list()
            .into_iter()
            .map(|runtime| runtime.key)
            .collect::<Vec<_>>();
        assert!(remaining.contains(&mainline));
        assert!(remaining.contains(&sibling));
        assert!(!remaining.contains(&selected));
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn dropping_the_manager_shuts_down_attached_workers() {
        let launcher = FakeLauncher::default();
        let manager = manager(&launcher);
        manager
            .start(
                RuntimeKey::local_python("project-a"),
                PathBuf::from("project-a"),
            )
            .await
            .unwrap();
        drop(manager);

        tokio::time::timeout(Duration::from_secs(1), async {
            while launcher.shutdowns.load(Ordering::SeqCst) == 0 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
