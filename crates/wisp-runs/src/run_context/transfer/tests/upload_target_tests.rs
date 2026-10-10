use super::*;
use tokio::sync::{Notify, Semaphore};
use wisp_store::{RunRecord, RunStatus, Store};

struct ControlledUploadRunner {
    commands: StdMutex<Vec<RunCommand>>,
    started: Semaphore,
    finish: Semaphore,
}

impl ControlledUploadRunner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            commands: StdMutex::new(Vec::new()),
            started: Semaphore::new(0),
            finish: Semaphore::new(0),
        })
    }

    async fn wait_started(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.started.acquire())
            .await
            .expect("upload should start")
            .unwrap()
            .forget();
    }

    fn upload_count(&self) -> usize {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.script == "local upload")
            .count()
    }
}

#[async_trait::async_trait]
impl super::super::super::RunCommandRunner for ControlledUploadRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        let is_upload = command.script == "local upload";
        let stdout = if command.script == "check local upload destination" {
            "__WISP_RSYNC__:yes\n".into()
        } else {
            String::new()
        };
        self.commands.lock().unwrap().push(command);
        if is_upload {
            self.started.add_permits(1);
            self.finish.acquire().await.unwrap().forget();
        }
        Ok(RunCommandOutput {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        })
    }
}

async fn upload(
    manager: &RunManager,
    store: &Store,
    source: &Path,
    context_id: &str,
    destination: &str,
    transport: TransferTransport,
    resume: bool,
) -> Result<SubmitRunResponse, String> {
    let context = store
        .get_execution_context(context_id)
        .await
        .unwrap()
        .unwrap();
    manager
        .submit_local_upload_to_ssh(
            store.clone(),
            "p",
            Some("f"),
            source,
            &context,
            destination,
            transport,
            resume,
            Duration::from_secs(30),
        )
        .await
}

async fn wait_drained(manager: &RunManager, run_id: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while manager.upload_targets.lock().await.contains_key(run_id) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("upload lifecycle should drain");
}

async fn cancel_and_drain(manager: &RunManager, store: &Store, run_id: &str) {
    manager.cancel(store, run_id).await.unwrap();
    wait_drained(manager, run_id).await;
}

fn upload_record(id: &str, project_id: &str, source: &Path, path: &str) -> RunRecord {
    let mut run = RunRecord::new(id, project_id, "ssh:a", "Upload", "file_transfer");
    run.timeout_secs = Some(30);
    run.env_snapshot_json = serde_json::json!({
        "route": "local",
        "source_context_id": "local",
        "source_path": source,
        "destination_context_id": "ssh:a",
        "destination_path": path,
    })
    .to_string();
    run.remote_handle_json = Some(
        serde_json::to_string(&TransferHandle::LocalUpload {
            source_path: source.to_string_lossy().into_owned(),
            destination_context_id: "ssh:a".into(),
            destination_path: path.into(),
            transport: "rsync".into(),
            resume: true,
        })
        .unwrap(),
    );
    run
}

async fn seed_active(store: &Store, run: &RunRecord, owner: &str) {
    store.create_run(run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(&run.id, RunStatus::Submitted, owner, ACTIVE_LEASE_SECS)
        .await
        .unwrap());
}

#[test]
fn upload_target_overlap_uses_normalized_path_components_and_context() {
    let sample = UploadTarget::new("ssh:a", "/results//./sample/").unwrap();
    assert_eq!(sample.path, "/results/sample");
    for path in ["/results/sample", "/results/sample/R1", "/results"] {
        let other = UploadTarget::new("ssh:a", path).unwrap();
        assert!(sample.overlaps(&other), "{path}");
        assert!(other.overlaps(&sample), "{path}");
    }
    for path in ["/results/sample2", "/results/other", "~/results/sample"] {
        assert!(!sample.overlaps(&UploadTarget::new("ssh:a", path).unwrap()));
    }
    assert!(!sample.overlaps(&UploadTarget::new("ssh:b", "/results/sample").unwrap()));
    assert!(!UploadTarget::new("ssh:a", "/results/sample/R1")
        .unwrap()
        .overlaps(&UploadTarget::new("ssh:a", "/results/sample/R2").unwrap()));
    assert_eq!(
        UploadTarget::new("ssh:a", "~//results/./sample/")
            .unwrap()
            .path,
        "~/results/sample"
    );
    for path in ["/results/../sample", "~/results/../sample", "/./", "~/./"] {
        assert!(UploadTarget::new("ssh:a", path).is_err(), "{path}");
    }
}

#[test]
fn persisted_upload_target_falls_back_to_snapshot_and_fails_closed() {
    let mut run = upload_record("existing", "p", Path::new("/tmp/source"), "/results/sample");
    run.remote_handle_json = None;
    assert_eq!(
        UploadTarget::from_run(&run).unwrap().unwrap().path,
        "/results/sample"
    );
    run.remote_handle_json = Some(r#"{"kind":"local_upload"}"#.into());
    assert!(UploadTarget::from_run(&run).unwrap().is_some());
    for snapshot in [
        r#"{"source_context_id":"local","destination_context_id":"ssh:a"}"#,
        r#"{"source_context_id":"local","destination_context_id":"ssh:b","destination_path":"/results/sample"}"#,
        r#"{"source_context_id":"local","destination_context_id":"ssh:a","destination_path":"/results/../sample"}"#,
    ] {
        run.env_snapshot_json = snapshot.into();
        let error = UploadTarget::from_run(&run).unwrap_err();
        assert!(error.contains(&run.id), "{error}");
    }
    run.remote_handle_json = Some(
        serde_json::to_string(&TransferHandle::LocalDownload {
            source_context_id: "ssh:a".into(),
            source_path: "/results/sample".into(),
            destination_path: "/tmp/sample".into(),
            transport: "scp".into(),
        })
        .unwrap(),
    );
    assert!(UploadTarget::from_run(&run).unwrap().is_none());
    run.remote_handle_json = None;
    // Not marked as coming from `local`: another kind of transfer, not an
    // upload whose target could not be read.
    for snapshot in [
        "{}",
        "invalid json",
        r#"{"route":"local","source_context_id":"ssh:a","destination_context_id":"local"}"#,
        r#"{"route":"relay","source_context_id":"ssh:a","destination_context_id":"ssh:b"}"#,
        r#"{"route":"local","source_context_id":"","destination_context_id":"ssh:a","destination_path":"/results/sample"}"#,
    ] {
        run.env_snapshot_json = snapshot.into();
        assert!(UploadTarget::from_run(&run).unwrap().is_none());
    }
}

#[tokio::test]
async fn concurrent_manager_clones_admit_one_upload_even_with_resume_and_new_source() {
    let (root, store) = test_store().await;
    let first = root.join("R1");
    let second = root.join("R2");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let clone = manager.clone();
    let (first, second) = tokio::join!(
        upload(
            &manager,
            &store,
            &first,
            "ssh:a",
            "/results/sample",
            TransferTransport::Scp,
            false
        ),
        upload(
            &clone,
            &store,
            &second,
            "ssh:a",
            "/results//./sample/",
            TransferTransport::Rsync,
            true
        ),
    );
    let (accepted, rejected) = match (first, second) {
        (Ok(accepted), Err(rejected)) | (Err(rejected), Ok(accepted)) => (accepted, rejected),
        other => panic!("exactly one upload must be admitted: {other:?}"),
    };
    assert!(rejected.contains(&accepted.run_id), "{rejected}");
    assert_eq!(store.list_runs_by_project("p").await.unwrap().len(), 1);
    runner.wait_started().await;
    assert_eq!(runner.upload_count(), 1);
    cancel_and_drain(&manager, &store, &accepted.run_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persisted_active_upload_in_another_project_blocks_with_and_without_handle() {
    let (root, store) = test_store().await;
    store
        .create_project("other", "Other", &root.join("other").to_string_lossy())
        .await
        .unwrap();
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    for with_handle in [true, false] {
        let id = format!("persisted-{with_handle}");
        let mut run = upload_record(&id, "other", &source, "/results/sample");
        if !with_handle {
            run.remote_handle_json = None;
        }
        seed_active(&store, &run, "previous-owner").await;
        let error = upload(
            &manager,
            &store,
            &source,
            "ssh:a",
            "/results/sample/child",
            TransferTransport::Rsync,
            true,
        )
        .await
        .unwrap_err();
        assert!(error.contains(&id), "{error}");
        assert!(store.list_runs_by_project("p").await.unwrap().is_empty());
        assert!(runner.commands.lock().unwrap().is_empty());
        assert!(store
            .finish_active_run_owned(&id, "previous-owner", RunStatus::Failed, Some(-1))
            .await
            .unwrap());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn malformed_active_upload_metadata_prevents_launch() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let mut run = upload_record("malformed", "p", &source, "/results/sample");
    run.remote_handle_json = Some(r#"{"kind":"local_upload"}"#.into());
    run.env_snapshot_json =
        r#"{"source_context_id":"local","destination_context_id":"ssh:a"}"#.into();
    seed_active(&store, &run, "previous-owner").await;
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let error = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Scp,
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("malformed"), "{error}");
    assert_eq!(store.list_runs_by_project("p").await.unwrap().len(), 1);
    assert!(runner.commands.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn another_transfer_on_the_same_server_does_not_block_an_upload() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    // A Files-panel download: an active `file_transfer` Run on the SSH context
    // with no snapshot and no handle.
    let download = RunRecord::new("download", "p", "ssh:a", "Download", "file_transfer");
    seed_active(&store, &download, "previous-owner").await;
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let accepted = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Scp,
        false,
    )
    .await
    .unwrap();
    runner.wait_started().await;
    cancel_and_drain(&manager, &store, &accepted.run_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn unavailable_project_database_fails_admission_closed() {
    let root = std::env::temp_dir().join(format!("wisp_upload_admission_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open_application(&root.join("application.sqlite"))
        .await
        .unwrap();
    store
        .set_setting("decentralized_project_storage", "true")
        .await
        .unwrap();
    let workspace = root.join("offline-project");
    store
        .create_project("offline", "Offline", &workspace.to_string_lossy())
        .await
        .unwrap();
    std::fs::remove_file(workspace.join(wisp_store::PROJECT_DATABASE)).unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let mut destination = wisp_store::ExecutionContext::new("ssh:a", "a").unwrap();
    destination.config_json =
        serde_json::json!({"alias":"a","host_name":"a.example","user":"alice"}).to_string();
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let error = manager
        .submit_local_upload_to_ssh(
            store,
            "p",
            None,
            &source,
            &destination,
            "/results/sample",
            TransferTransport::Scp,
            false,
            Duration::from_secs(30),
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("Cannot check active upload destinations"),
        "{error}"
    );
    assert!(error.contains("offline"), "{error}");
    assert!(runner.commands.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn directory_upload_blocks_children_but_allows_siblings_and_other_contexts() {
    let (root, store) = test_store().await;
    let source = root.join("directory");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("R1"), b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let directory = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Rsync,
        false,
    )
    .await
    .unwrap();
    let error = upload(
        &manager,
        &store,
        &source.join("R1"),
        "ssh:a",
        "/results/sample/R1",
        TransferTransport::Scp,
        true,
    )
    .await
    .unwrap_err();
    assert!(error.contains(&directory.run_id), "{error}");
    let sibling = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample2",
        TransferTransport::Scp,
        false,
    )
    .await
    .unwrap();
    let other_context = upload(
        &manager,
        &store,
        &source,
        "ssh:b",
        "/results/sample",
        TransferTransport::Scp,
        false,
    )
    .await
    .unwrap();
    for _ in 0..3 {
        runner.wait_started().await;
    }
    assert_eq!(runner.upload_count(), 3);
    for response in [directory, sibling, other_context] {
        cancel_and_drain(&manager, &store, &response.run_id).await;
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn successful_upload_releases_target_for_a_terminal_retry() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let first = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Rsync,
        false,
    )
    .await
    .unwrap();
    runner.wait_started().await;
    runner.finish.add_permits(1);
    wait_drained(&manager, &first.run_id).await;
    assert_eq!(
        store.get_run(&first.run_id).await.unwrap().unwrap().status,
        RunStatus::Succeeded
    );
    let retry = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Rsync,
        true,
    )
    .await
    .unwrap();
    runner.wait_started().await;
    assert_eq!(runner.upload_count(), 2);
    cancel_and_drain(&manager, &store, &retry.run_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn recovery_fails_a_duplicate_without_launching_it_and_reclaims_one_writer() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let first = upload_record("recovered-first", "p", &source, "/results/sample");
    let second = upload_record("recovered-second", "p", &source, "/results/sample/R1");
    seed_active(&store, &first, &manager.owner_id).await;
    seed_active(&store, &second, &manager.owner_id).await;
    manager
        .reclaim_transfer(store.clone(), &first)
        .await
        .unwrap();
    let rejected = store.get_run(&first.id).await.unwrap().unwrap();
    assert_eq!(rejected.status, RunStatus::Failed);
    assert!(rejected.last_poll_error.unwrap().contains(&second.id));
    assert!(runner.commands.lock().unwrap().is_empty());
    manager
        .reclaim_transfer(store.clone(), &second)
        .await
        .unwrap();
    manager
        .reclaim_transfer(store.clone(), &second)
        .await
        .unwrap();
    runner.wait_started().await;
    assert_eq!(runner.upload_count(), 1);
    cancel_and_drain(&manager, &store, &second.id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn cancellation_keeps_target_reserved_until_lifecycle_cleanup_drains() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let first = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Rsync,
        false,
    )
    .await
    .unwrap();
    runner.wait_started().await;

    // Queue admission before aborting, while cleanup is deliberately held off.
    // Tokio's FIFO mutex lets that caller observe the cancelled Run's retained
    // reservation before the cleanup task is allowed to release it.
    let targets = manager.upload_targets.lock().await;
    let queued = Arc::new(Notify::new());
    let attempt = {
        let manager = manager.clone();
        let store = store.clone();
        let source = source.clone();
        let queued = queued.clone();
        tokio::spawn(async move {
            // Resolve the context before announcing the admission attempt.
            let context = store.get_execution_context("ssh:a").await.unwrap().unwrap();
            queued.notify_one();
            manager
                .submit_local_upload_to_ssh(
                    store,
                    "p",
                    Some("f"),
                    &source,
                    &context,
                    "/results/sample",
                    TransferTransport::Rsync,
                    true,
                    Duration::from_secs(30),
                )
                .await
        })
    };
    queued.notified().await;
    manager.cancel(&store, &first.run_id).await.unwrap();
    assert_eq!(
        store.get_run(&first.run_id).await.unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    assert!(targets.contains_key(&first.run_id));
    drop(targets);
    let error = attempt.await.unwrap().unwrap_err();
    assert!(error.contains(&first.run_id), "{error}");
    wait_drained(&manager, &first.run_id).await;
    let retry = upload(
        &manager,
        &store,
        &source,
        "ssh:a",
        "/results/sample",
        TransferTransport::Rsync,
        true,
    )
    .await
    .unwrap();
    runner.wait_started().await;
    assert_eq!(runner.upload_count(), 2);
    cancel_and_drain(&manager, &store, &retry.run_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn aborted_submission_before_registration_does_not_launch_or_leak_a_reservation() {
    let (root, store) = test_store().await;
    let source = root.join("R1");
    std::fs::write(&source, b"data").unwrap();
    let runner = ControlledUploadRunner::new();
    let manager = RunManager::with_runner(runner.clone());
    let active = manager.active.lock().await;
    let attempt = {
        let manager = manager.clone();
        let store = store.clone();
        let source = source.clone();
        tokio::spawn(async move {
            upload(
                &manager,
                &store,
                &source,
                "ssh:a",
                "/results/sample",
                TransferTransport::Rsync,
                false,
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if store
                .list_runs_by_project("p")
                .await
                .unwrap()
                .iter()
                .any(|run| run.remote_handle_json.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("submission should persist its recoverable handle");
    attempt.abort();
    assert!(attempt.await.unwrap_err().is_cancelled());
    assert!(manager.upload_targets.lock().await.is_empty());
    assert!(active.is_empty());
    assert!(runner.commands.lock().unwrap().is_empty());
    drop(active);

    // The durable active record still blocks retries and can be recovered.
    let run = store
        .list_runs_by_project("p")
        .await
        .unwrap()
        .pop()
        .unwrap();
    manager.reclaim_transfer(store.clone(), &run).await.unwrap();
    runner.wait_started().await;
    assert_eq!(runner.upload_count(), 1);
    cancel_and_drain(&manager, &store, &run.id).await;
    let _ = std::fs::remove_dir_all(root);
}
