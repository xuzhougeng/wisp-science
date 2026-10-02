//! Project-owned browser instances. Public tools accept only `workspace`;
//! instance lane names and connection tokens never come from tool arguments.
use super::*;
use std::process::Child;
use tokio::sync::watch;

pub(super) const MAX_WORKSPACES: usize = 3;

#[derive(Clone, Debug)]
pub(super) struct WorkspaceIdentity {
    pub project_id: String,
    pub lane: String,
    pub profile: PathBuf,
    pub extension: PathBuf,
    pub endpoint: String,
    pub connection_path: String,
}

impl WorkspaceIdentity {
    fn new(root: &Path, project_id: &str, port: u16) -> Self {
        // Hash opaque project ids; never treat them as directory components.
        let key = format!("{:x}", Sha256::digest(project_id.as_bytes()));
        let generation = Uuid::new_v4();
        let directory = root.join("browser-workspaces").join(&key);
        let connection_path = format!("/{generation}");
        Self {
            project_id: project_id.into(),
            lane: format!("workspace:{key}:{generation}"),
            profile: directory.join("profile"),
            extension: directory.join("extension"),
            endpoint: format!("ws://127.0.0.1:{port}{connection_path}"),
            connection_path,
        }
    }

    fn status(&self, connected: bool, running: bool) -> Value {
        json!({
            "session": "workspace",
            "project_id": self.project_id,
            "lane": self.lane,
            "connected": connected,
            "process_running": running,
            "profile_dir": self.profile,
            "extension_dir": self.extension,
            "endpoint": self.endpoint,
            "workspace_limit": MAX_WORKSPACES,
        })
    }
}

/// Owning the child lets us check for exit before signalling a PID, and reap it
/// on stop. Tests supply an in-memory process; they never launch a browser.
trait WorkspaceProcess: Send {
    fn running(&mut self) -> bool;
    fn stop(&mut self);
}

impl WorkspaceProcess for Child {
    fn running(&mut self) -> bool {
        matches!(self.try_wait(), Ok(None))
    }

    fn stop(&mut self) {
        if self.running() {
            #[cfg(windows)]
            workspace::terminate(self.id());
            let _ = self.kill();
            let _ = self.wait();
        }
    }
}

pub(super) struct ProjectWorkspace {
    pub identity: WorkspaceIdentity,
    process: Box<dyn WorkspaceProcess>,
    shutdown: watch::Sender<bool>,
}

impl Drop for ProjectWorkspace {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.process.stop();
    }
}

impl BrowserBridge {
    pub(super) async fn tool_session(
        &self,
        args: &Value,
        env: &dyn ToolEnv,
    ) -> Result<Option<String>, String> {
        let session = session_arg(args)?;
        if session.as_deref() != Some("workspace") {
            return Ok(session);
        }
        let project_id = require_project(env.project_id())?;
        self.state.lock().await.workspaces.get(project_id)
            .map(|workspace| Some(workspace.identity.lane.clone()))
            .ok_or_else(|| format!(
                "project '{project_id}' has no workspace browser; call browser_setup with action=start_workspace first"
            ))
    }

    pub(super) async fn workspace_status(&self, project_id: Option<&str>) -> Value {
        let mut state = self.state.lock().await;
        let Some(instance) = project_id.and_then(|id| state.workspaces.get_mut(id)) else {
            return json!({ "session": "workspace", "project_id": project_id,
                "connected": false, "process_running": false,
                "workspace_limit": MAX_WORKSPACES });
        };
        let identity = instance.identity.clone();
        let running = instance.process.running();
        let connected = state
            .sessions
            .get(&identity.lane)
            .is_some_and(|slot| slot.client.is_some());
        identity.status(connected, running)
    }

    pub(super) async fn start_workspace(
        self: &Arc<Self>,
        project_id: Option<&str>,
    ) -> Result<Value, String> {
        let project_id = require_project(project_id)?;
        // Serialize lifecycle transitions, not browser commands. Concurrent
        // starts cannot exceed the cap or launch twice into the same profile.
        let _lifecycle = self.workspace_lifecycle.lock().await;
        if let Some(status) = self.existing_workspace(project_id).await {
            return Ok(status);
        }
        self.check_workspace_capacity().await?;
        let extension_path = self.verified_extension_path().ok_or(
            "bundled browser-extension path is not available; cannot materialize workspace copy",
        )?;
        let browser = workspace::resolve_browser()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("cannot listen for project workspace: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let identity = WorkspaceIdentity::new(&workspace::app_data_root(), project_id, port);
        workspace::materialize_extension(
            Path::new(&extension_path),
            &identity.extension,
            &identity.lane,
            &identity.endpoint,
        )?;
        let (shutdown, receiver) = watch::channel(false);
        let bridge = self.clone();
        let listener_identity = identity.clone();
        tokio::spawn(async move {
            bridge
                .accept_workspace(listener, listener_identity, receiver)
                .await;
        });
        self.launch_workspace(
            identity,
            shutdown,
            &browser,
            &extension_path,
            WORKSPACE_CONNECT_WAIT,
            |identity| {
                workspace::launch_browser(&browser, &identity.profile, &identity.extension)
                    .map(|child| Box::new(child) as Box<dyn WorkspaceProcess>)
            },
        )
        .await
    }

    async fn existing_workspace(&self, project_id: &str) -> Option<Value> {
        let status = self.workspace_status(Some(project_id)).await;
        if status["process_running"] == true {
            return Some(status);
        }
        self.remove_workspace(project_id).await;
        None
    }

    async fn check_workspace_capacity(&self) -> Result<(), String> {
        if self.state.lock().await.workspaces.len() >= MAX_WORKSPACES {
            return Err(format!(
                "workspace browser limit ({MAX_WORKSPACES}) reached; use browser_setup action=stop_workspace in an idle project before starting another workspace"
            ));
        }
        Ok(())
    }

    async fn launch_workspace<L>(
        &self,
        identity: WorkspaceIdentity,
        shutdown: watch::Sender<bool>,
        browser: &workspace::WorkspaceBrowser,
        extension_path: &str,
        wait: Duration,
        launch: L,
    ) -> Result<Value, String>
    where
        L: FnOnce(&WorkspaceIdentity) -> Result<Box<dyn WorkspaceProcess>, String>,
    {
        let project_id = identity.project_id.clone();
        {
            // Reserve the state lock before invoking the synchronous runner.
            // Connections cannot install before the identity is registered,
            // and cancellation cannot drop an unowned child between awaits.
            let mut state = self.state.lock().await;
            let process = launch(&identity)?;
            state.workspaces.insert(
                project_id.clone(),
                ProjectWorkspace {
                    identity: identity.clone(),
                    process,
                    shutdown,
                },
            );
        }
        if self.wait_for_session(&identity.lane, wait).await {
            return Ok(identity.status(true, true));
        }
        self.remove_workspace(&project_id).await;
        Err(errors::structured(
            errors::WORKSPACE_EXTENSION_BLOCKED,
            &workspace::extension_blocked_message(
                browser,
                extension_path,
                &identity.endpoint,
                wait,
            ),
            false,
        ))
    }

    async fn accept_workspace(
        self: Arc<Self>,
        listener: TcpListener,
        identity: WorkspaceIdentity,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { break; };
                    let bridge = self.clone();
                    let identity = identity.clone();
                    let mut connection_shutdown = shutdown.clone();
                    tokio::spawn(async move {
                        if *connection_shutdown.borrow() { return; }
                        tokio::select! {
                            _ = connection_shutdown.changed() => {},
                            result = bridge.accept_connection_with_path(
                                stream, identity.lane, Some(identity.connection_path)
                            ) => {
                                if let Err(error) = result {
                                    tracing::warn!(target: "wisp", "project workspace connection rejected: {error}");
                                }
                            }
                        }
                    });
                }
            }
        }
    }

    pub(crate) async fn stop_project_workspace(&self, project_id: &str) -> Value {
        let _lifecycle = self.workspace_lifecycle.lock().await;
        self.remove_workspace(project_id).await;
        self.workspace_status(Some(project_id)).await
    }

    async fn remove_workspace(&self, project_id: &str) {
        let instance = self.state.lock().await.workspaces.remove(project_id);
        let Some(instance) = instance else {
            return;
        };
        let lane = instance.identity.lane.clone();
        // Shut down both the listener and its accepted sockets before forgetting
        // the instance. Generation-specific lanes make late replies harmless.
        drop(instance);
        if let Some(mut slot) = self.state.lock().await.sessions.remove(&lane) {
            fail_pending(&mut slot, "project workspace stopped");
        }
        self.occupancy
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&lane);
        self.turn_ledgers.lock().await.retain(|_, ledger| {
            ledger.tabs.retain(|tab| tab.session != lane);
            !ledger.tabs.is_empty()
        });
        self.pending_cleanups.lock().await.retain(|_, row| {
            row.prompt.tabs.retain(|tab| tab.session != lane);
            !row.prompt.tabs.is_empty()
        });
        self.needs_human
            .lock()
            .await
            .retain(|(session, _), _| session != &lane);
        self.persist_pending().await;
        self.persist_needs_human().await;
        self.emit_needs_human().await;
    }
}

pub(super) fn require_project(project_id: Option<&str>) -> Result<&str, String> {
    project_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "workspace browser requires a current project".into())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct FakeProcess(Arc<AtomicUsize>);
    impl WorkspaceProcess for FakeProcess {
        fn running(&mut self) -> bool {
            self.0.load(Ordering::SeqCst) == 0
        }
        fn stop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(in crate::browser_bridge) async fn register_fake(
        bridge: &BrowserBridge,
        project: &str,
    ) -> (WorkspaceIdentity, Arc<AtomicUsize>) {
        let identity = WorkspaceIdentity::new(Path::new("unused-test-root"), project, 12345);
        let stops = Arc::new(AtomicUsize::new(0));
        let (shutdown, _) = watch::channel(false);
        bridge.state.lock().await.workspaces.insert(
            project.into(),
            ProjectWorkspace {
                identity: identity.clone(),
                process: Box::new(FakeProcess(stops.clone())),
                shutdown,
            },
        );
        (identity, stops)
    }

    struct Env(&'static str, &'static str);
    #[async_trait]
    impl ToolEnv for Env {
        fn project_root(&self) -> &Path {
            Path::new(".")
        }
        fn project_id(&self) -> Option<&str> {
            Some(self.0)
        }
        fn turn_id(&self) -> Option<&str> {
            Some(self.1)
        }
        async fn confirm(&self, _: &str) -> bool {
            true
        }
        async fn emit(&self, _: wisp_tools::ToolEvent) {}
    }

    #[test]
    fn profiles_are_project_stable_but_connections_change_every_start() {
        let root = Path::new("test-root");
        let a = WorkspaceIdentity::new(root, "../../project/a", 12345);
        let again = WorkspaceIdentity::new(root, "../../project/a", 12345);
        let b = WorkspaceIdentity::new(root, "../../project/b", 12345);
        assert_eq!(a.profile, again.profile);
        assert_ne!(a.profile, b.profile);
        assert_ne!(a.extension, b.extension);
        assert_ne!(a.lane, again.lane);
        assert_ne!(a.endpoint, again.endpoint);
        assert!(workspace_path_matches(
            Some(&a.connection_path),
            &a.connection_path
        ));
        assert!(!workspace_path_matches(
            Some(&a.connection_path),
            &again.connection_path
        ));
        assert!(!workspace_path_matches(Some(&a.connection_path), "/"));
        assert!(a.profile.starts_with(root.join("browser-workspaces")));
        assert!(!a
            .profile
            .components()
            .any(|c| c == std::path::Component::ParentDir));
    }

    #[tokio::test]
    async fn tools_route_identical_tab_ids_to_their_own_project() {
        let bridge = Arc::new(BrowserBridge::new(PathBuf::from("extension")));
        let a = register_fake(&bridge, "a").await.0;
        let b = register_fake(&bridge, "b").await.0;
        let (a_tx, _a_rx) = mpsc::unbounded_channel();
        let (b_tx, _b_rx) = mpsc::unbounded_channel();
        bridge.install_client_on(1, a_tx, &a.lane).await;
        bridge.install_client_on(2, b_tx, &b.lane).await;
        for (id, identity, url) in [(1, &a, "https://a.example"), (2, &b, "https://b.example")] {
            bridge
                .handle_text_on(
                    id,
                    &identity.lane,
                    &json!({
                        "type": "ext_ready", "protocol_version": 2,
                        "tabs": [{ "id": 7, "url": url, "title": url, "active": true }]
                    })
                    .to_string(),
                )
                .await;
        }
        let tool = WebScanTool::new(bridge.clone());
        let args = json!({ "session": "workspace", "tabs_only": true });
        let (ra, rb) = tokio::join!(
            tool.run(&args, &Env("a", "turn-a")),
            tool.run(&args, &Env("b", "turn-b"))
        );
        assert!(ra.success, "{}", ra.content);
        assert!(rb.success, "{}", rb.content);
        assert!(ra.content.contains("https://a.example"));
        assert!(!ra.content.contains("https://b.example"));
        assert!(rb.content.contains("https://b.example"));
        assert!(!rb.content.contains("https://a.example"));
        assert!(bridge
            .tool_session(&json!({"session": a.lane}), &Env("b", "turn-b"))
            .await
            .is_err());
        assert!(bridge
            .tool_session(&args, &Env("", "turn-c"))
            .await
            .is_err());
        assert!(bridge
            .tool_session(&args, &Env("missing", "turn-c"))
            .await
            .is_err());
        bridge.complete_turn("turn-a").await;
        assert!(!bridge.occupancy.lock().unwrap().contains_key(&a.lane));
        assert!(bridge.occupancy.lock().unwrap().contains_key(&b.lane));
        // A global shared lease remains exclusive while workspace tools run.
        bridge.occupy_turn("shared", "a", "shared-a").unwrap();
        assert!(bridge.occupy_turn("shared", "b", "shared-b").is_err());
    }

    #[tokio::test]
    async fn stop_removes_only_its_process_requests_and_tab_state() {
        let bridge = BrowserBridge::new(PathBuf::from("extension"));
        let (a, a_stops) = register_fake(&bridge, "a").await;
        let (b, b_stops) = register_fake(&bridge, "b").await;
        let (a_tx, _a_rx) = mpsc::unbounded_channel();
        let (b_tx, _b_rx) = mpsc::unbounded_channel();
        bridge.install_client_on(1, a_tx, &a.lane).await;
        bridge.install_client_on(2, b_tx, &b.lane).await;
        let (pending_tx, pending_rx) = oneshot::channel();
        bridge
            .state
            .lock()
            .await
            .sessions
            .get_mut(&a.lane)
            .unwrap()
            .pending
            .insert("pending".into(), pending_tx);
        for identity in [&a, &b] {
            bridge.record_opened_tab(&identity.project_id, &identity.project_id, &identity.lane,
                &json!({ "id": 7, "url": "https://example.org", "title": "tab", "active": true })).await;
            bridge.needs_human.lock().await.insert(
                (identity.lane.clone(), 7),
                BrowserNeedsHumanTab {
                    session: identity.lane.clone(),
                    tab_id: 7,
                    ..Default::default()
                },
            );
            bridge
                .stash_pending(
                    BrowserTabCleanupPrompt {
                        turn_id: identity.project_id.clone(),
                        frame_id: identity.project_id.clone(),
                        tabs: vec![BrowserTabCleanupItem {
                            session: identity.lane.clone(),
                            tab_id: 7,
                            ..Default::default()
                        }],
                    },
                    false,
                )
                .await;
        }
        bridge.stop_project_workspace("a").await;
        assert_eq!(a_stops.load(Ordering::SeqCst), 1);
        assert_eq!(b_stops.load(Ordering::SeqCst), 0);
        assert!(pending_rx.await.unwrap().unwrap_err().contains("stopped"));
        assert!(!bridge.state.lock().await.sessions.contains_key(&a.lane));
        assert!(bridge.state.lock().await.sessions.contains_key(&b.lane));
        assert!(!bridge.turn_ledgers.lock().await.contains_key("a"));
        assert!(bridge.turn_ledgers.lock().await.contains_key("b"));
        assert_eq!(bridge.list_pending_cleanups().await.len(), 1);
        assert_eq!(bridge.snapshot_needs_human().await[0].session, b.lane);
        // A socket finishing its handshake after stop cannot resurrect the lane.
        let (late_tx, _late_rx) = mpsc::unbounded_channel();
        bridge.install_client_on(99, late_tx, &a.lane).await;
        assert!(!bridge.state.lock().await.sessions.contains_key(&a.lane));
        bridge.stop_project_workspace("a").await;
        assert_eq!(a_stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn starts_are_idempotent_and_capacity_is_reclaimed_on_stop() {
        let bridge = BrowserBridge::new(PathBuf::from("extension"));
        for project in ["a", "b", "c"] {
            register_fake(&bridge, project).await;
        }
        assert!(bridge
            .check_workspace_capacity()
            .await
            .unwrap_err()
            .contains("stop_workspace"));
        let original = bridge.workspace_status(Some("a")).await;
        assert_eq!(
            bridge.existing_workspace("a").await.unwrap()["lane"],
            original["lane"]
        );
        bridge.stop_project_workspace("b").await;
        assert!(bridge.check_workspace_capacity().await.is_ok());
        assert_eq!(
            bridge.workspace_status(Some("a")).await["lane"],
            original["lane"]
        );
    }

    #[tokio::test]
    async fn failed_launch_and_connection_timeout_leave_no_instance() {
        let bridge = BrowserBridge::new(PathBuf::from("extension"));
        let browser = workspace::WorkspaceBrowser {
            name: "Chrome".into(),
            path: "unused".into(),
            loads_unpacked_extensions: false,
        };
        let identity = WorkspaceIdentity::new(Path::new("unused"), "a", 12345);
        let (shutdown, mut receiver) = watch::channel(false);
        assert!(bridge
            .launch_workspace(
                identity.clone(),
                shutdown,
                &browser,
                "unused",
                Duration::ZERO,
                |_| Err("fake launch failure".into())
            )
            .await
            .unwrap_err()
            .contains("fake launch"));
        assert!(receiver.changed().await.is_err());
        let (shutdown, _) = watch::channel(false);
        let stops = Arc::new(AtomicUsize::new(0));
        let recorder = stops.clone();
        let error = bridge
            .launch_workspace(
                identity,
                shutdown,
                &browser,
                "extension",
                Duration::ZERO,
                |_| Ok(Box::new(FakeProcess(recorder))),
            )
            .await
            .unwrap_err();
        assert!(error.contains(errors::WORKSPACE_EXTENSION_BLOCKED));
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(bridge.state.lock().await.workspaces.is_empty());
    }

    #[tokio::test]
    async fn successful_start_waits_for_its_own_connection() {
        let bridge = Arc::new(BrowserBridge::new(PathBuf::from("extension")));
        let identity = WorkspaceIdentity::new(Path::new("unused"), "a", 12345);
        let browser = workspace::WorkspaceBrowser {
            name: "Chromium".into(),
            path: "unused".into(),
            loads_unpacked_extensions: true,
        };
        let (shutdown, _) = watch::channel(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        let stops = Arc::new(AtomicUsize::new(0));
        let recorder = stops.clone();
        let connector = bridge.clone();
        let status = bridge
            .launch_workspace(
                identity.clone(),
                shutdown,
                &browser,
                "extension",
                Duration::from_secs(1),
                move |identity| {
                    let lane = identity.lane.clone();
                    tokio::spawn(async move {
                        connector.install_client_on(1, tx, &lane).await;
                    });
                    Ok(Box::new(FakeProcess(recorder)))
                },
            )
            .await
            .unwrap();
        assert_eq!(status["connected"], true);
        assert_eq!(status["project_id"], "a");
        assert_eq!(status["lane"], identity.lane);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn requests_and_replies_cannot_cross_project_connections() {
        let bridge = Arc::new(BrowserBridge::new(PathBuf::from("extension")));
        let a = register_fake(&bridge, "a").await.0;
        let b = register_fake(&bridge, "b").await.0;
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        bridge.install_client_on(1, a_tx, &a.lane).await;
        bridge.install_client_on(2, b_tx, &b.lane).await;
        let request = |lane: String| {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .send_command_on(Some(&lane), "test".into(), Duration::from_secs(2))
                    .await
            })
        };
        let ra = request(a.lane.clone());
        let rb = request(b.lane.clone());
        let a_message = tokio::time::timeout(Duration::from_secs(1), a_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let b_message = tokio::time::timeout(Duration::from_secs(1), b_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let a_request: Value = serde_json::from_str(a_message.to_text().unwrap()).unwrap();
        let b_request: Value = serde_json::from_str(b_message.to_text().unwrap()).unwrap();
        let a_reply = json!({ "type": "result", "id": a_request["id"], "result": {"project":"a"} })
            .to_string();
        let b_reply = json!({ "type": "result", "id": b_request["id"], "result": {"project":"b"} })
            .to_string();
        bridge.handle_text_on(2, &b.lane, &a_reply).await;
        bridge.handle_text_on(1, &a.lane, &b_reply).await;
        assert_eq!(bridge.state.lock().await.sessions[&a.lane].pending.len(), 1);
        assert_eq!(bridge.state.lock().await.sessions[&b.lane].pending.len(), 1);
        bridge.handle_text_on(1, &a.lane, &a_reply).await;
        bridge.handle_text_on(2, &b.lane, &b_reply).await;
        assert_eq!(ra.await.unwrap().unwrap().value["project"], "a");
        assert_eq!(rb.await.unwrap().unwrap().value["project"], "b");
    }

    #[tokio::test]
    async fn restart_discards_old_workspace_cleanup_and_human_prompts() {
        let path =
            std::env::temp_dir().join(format!("workspace-restart-{}.sqlite", Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        let bridge = BrowserBridge::new_with_store("extension".into(), store.clone());
        let a = register_fake(&bridge, "a").await.0;
        for session in [&a.lane, "workspace", "shared"] {
            bridge
                .stash_pending(
                    BrowserTabCleanupPrompt {
                        turn_id: session.into(),
                        frame_id: "frame".into(),
                        tabs: vec![BrowserTabCleanupItem {
                            session: session.into(),
                            tab_id: 7,
                            ..Default::default()
                        }],
                    },
                    false,
                )
                .await;
            bridge.needs_human.lock().await.insert(
                (session.into(), 7),
                BrowserNeedsHumanTab {
                    session: session.into(),
                    tab_id: 7,
                    ..Default::default()
                },
            );
        }
        bridge.persist_needs_human().await;
        let restarted = BrowserBridge::new_with_store("extension".into(), store.clone());
        restarted.load_pending_cleanups().await;
        restarted.load_pending_needs_human().await;
        assert_eq!(restarted.list_pending_cleanups().await.len(), 1);
        assert_eq!(
            restarted.list_pending_cleanups().await[0].tabs[0].session,
            "shared"
        );
        assert_eq!(restarted.snapshot_needs_human().await.len(), 1);
        assert_eq!(restarted.snapshot_needs_human().await[0].session, "shared");
        drop(bridge);
        drop(restarted);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
