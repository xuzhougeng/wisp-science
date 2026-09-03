use super::*;
use std::collections::VecDeque;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

#[tokio::test]
async fn run_tool_schemas_keep_waiting_updates_in_the_live_card() {
    use wisp_tools::Tool;

    let tmp = std::env::temp_dir().join(format!("wisp_quiet_run_schema_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    let run = RunInContextTool::new(store.clone(), RunManager::new(), "project".into(), None)
        .schema()
        .function
        .description;
    let monitor = MonitorRunTool::new(store, "project".into())
        .schema()
        .function
        .description;

    assert!(run.contains("call monitor_run directly without announcing"));
    assert!(run.contains("the Run card communicates that state"));
    assert!(monitor.contains("without a user-facing preamble"));
    assert!(monitor.contains("do not say that you are waiting or monitoring"));

    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn dirty_patch_secret_filter_rejects_high_signal_credentials() {
    assert!(contains_obvious_secret(
        "+-----BEGIN OPENSSH PRIVATE KEY-----\n+payload"
    ));
    assert!(contains_obvious_secret(
        "+Authorization: Bearer publication-token"
    ));
    assert!(!contains_obvious_secret(
        "+let api_key = std::env::var(\"API_KEY\")?;"
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn process_runner_keeps_only_bounded_output_tails() {
    let command = RunCommand {
        context_id: "local".into(),
        program: "sh".into(),
        args: vec![
            "-c".into(),
            "head -c 200000 /dev/zero | tr '\\0' x; printf OUT_END; head -c 200000 /dev/zero | tr '\\0' y >&2; printf ERR_END >&2".into(),
        ],
        script: String::new(),
        cwd: None,
        stdin: None,
        envs: Vec::new(),
    };

    let output = ProcessRunRunner
        .run(command, Duration::from_secs(10))
        .await
        .unwrap();

    assert_eq!(output.exit_code, 0);
    assert!(output.stdout.len() <= MAX_RUN_OUTPUT_BYTES);
    assert!(output.stderr.len() <= MAX_RUN_OUTPUT_BYTES);
    assert!(output.stdout.ends_with("OUT_END"));
    assert!(output.stderr.ends_with("ERR_END"));
}

#[cfg(unix)]
#[tokio::test]
async fn process_runner_timeout_cleans_up_inherited_pipes() {
    let auth_dir = std::env::temp_dir().join(format!("wisp_runner_auth_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&auth_dir).unwrap();
    let passfile = auth_dir.join("pass");
    let askpass = auth_dir.join("askpass.sh");
    std::fs::write(&passfile, "secret").unwrap();
    std::fs::write(&askpass, "#!/bin/sh\n").unwrap();
    let command = RunCommand {
        context_id: "local".into(),
        program: "sh".into(),
        args: vec!["-c".into(), "sleep 1 & wait".into()],
        script: String::new(),
        cwd: None,
        stdin: None,
        envs: vec![
            (
                "WISP_SSH_PASSFILE".into(),
                passfile.to_string_lossy().into_owned(),
            ),
            (
                "WISP_SSH_ASKPASS_SCRIPT".into(),
                askpass.to_string_lossy().into_owned(),
            ),
        ],
    };

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        ProcessRunRunner.run(command, Duration::from_millis(20)),
    )
    .await
    .expect("runner leaked a pipe reader after timeout")
    .unwrap_err();
    assert!(result.contains("timed out"));
    assert!(!auth_dir.exists(), "password auth directory leaked");
}

#[tokio::test]
async fn run_in_context_preview_keeps_long_commands_intact() {
    use wisp_tools::Tool;
    let tmp =
        std::env::temp_dir().join(format!("wisp_run_preview_{}.sqlite", uuid::Uuid::new_v4()));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    let tool = RunInContextTool::new(store, RunManager::new(), "p".into(), None);
    let command = format!(
        "grep -in snakemake {} {}",
        "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/README.md",
        "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/Snakefile"
    );
    assert!(
        command.len() > 140,
        "premise: command longer than old 140-char cap"
    );
    let preview = tool.preview(&serde_json::json!({
        "context_id": "ssh:CPU3",
        "command": command.clone(),
    }));
    assert_eq!(preview, format!("ssh:CPU3: {command}"));
    let _ = std::fs::remove_file(tmp);
}

struct RunToolTestEnv(PathBuf);

#[async_trait::async_trait]
impl wisp_tools::ToolEnv for RunToolTestEnv {
    fn project_root(&self) -> &std::path::Path {
        &self.0
    }

    async fn confirm(&self, _message: &str) -> bool {
        true
    }

    async fn emit(&self, _event: wisp_tools::ToolEvent) {}
}

struct DenyRunToolEnv(PathBuf);

#[async_trait::async_trait]
impl wisp_tools::ToolEnv for DenyRunToolEnv {
    fn project_root(&self) -> &std::path::Path {
        &self.0
    }

    async fn confirm(&self, _message: &str) -> bool {
        false
    }

    async fn emit(&self, _event: wisp_tools::ToolEvent) {}
}

struct GuidanceRunToolEnv {
    root: PathBuf,
    queue: Arc<wisp_core::GuidanceQueue>,
}

#[async_trait::async_trait]
impl wisp_tools::ToolEnv for GuidanceRunToolEnv {
    fn project_root(&self) -> &std::path::Path {
        &self.root
    }

    async fn confirm(&self, _message: &str) -> bool {
        true
    }

    async fn emit(&self, _event: wisp_tools::ToolEvent) {}

    fn guidance_pending(&self) -> bool {
        self.queue
            .lock()
            .map(|pending| !pending.is_empty())
            .unwrap_or(false)
    }
}

#[tokio::test]
async fn denied_dangerous_run_stops_the_model_batch() {
    use wisp_tools::{Tool, ToolControl};
    let tmp = std::env::temp_dir().join(format!("wisp_run_deny_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    let tool = RunInContextTool::new(store, RunManager::new(), "p".into(), None);

    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "rm -rf generated-output"
            }),
            &DenyRunToolEnv(tmp.clone()),
        )
        .await;

    assert!(!result.success);
    assert_eq!(result.control, ToolControl::StopBatch);
    drop(tool);
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn run_in_context_can_suspend_until_terminal_without_get_run_calls() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_run_wait_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
        ok_output(&poll_response("finished:0", "finished", "")),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner);
    let tool = RunInContextTool::new(store, manager, "p".into(), None);
    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "echo finished",
                "wait_for_completion": true
            }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(result.success, "{}", result.content);
    let run: wisp_store::RunRecord = serde_json::from_str(&result.content).unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(run.stdout_tail.as_deref(), Some("finished"));
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn run_in_context_wait_reports_a_failed_run_as_a_failed_tool_call() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_run_wait_fail_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
        ok_output(&poll_response(
            "finished:127",
            "",
            "python: command not found",
        )),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner);
    let tool = RunInContextTool::new(store, manager, "p".into(), None);
    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "python -c pass",
                "wait_for_completion": true
            }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(
        !result.success,
        "failed Run must not render as a green tool call"
    );
    let run: wisp_store::RunRecord = serde_json::from_str(&result.content).unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Failed);
    assert_eq!(run.exit_code, Some(127));
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn run_in_context_preflight_blocks_missing_packages_before_creating_a_run() {
    use wisp_tools::Tool;
    let tmp =
        std::env::temp_dir().join(format!("wisp_run_preflight_fail_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let manager = RunManager::with_runner(Arc::new(FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 9,
        stdout: "3.12.4\n".into(),
        stderr: "missing modules: decoupler".into(),
    }))));
    let tool = RunInContextTool::new(store.clone(), manager, "p".into(), None);

    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "python analysis.py",
                "preflight": {
                    "language": "python",
                    "packages": ["decoupler"]
                }
            }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(!result.success);
    assert!(result.content.contains("\"run_submitted\":false"));
    assert!(result.content.contains("missing modules: decoupler"));
    assert!(store.list_runs_by_project("p").await.unwrap().is_empty());
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn run_in_context_preflight_is_structured_and_persisted_with_the_run() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_run_preflight_ok_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("3.12.4\n"),
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
        ok_output(&poll_response("finished:0", "analysis complete", "")),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner.clone());
    let tool = RunInContextTool::new(store.clone(), manager, "p".into(), None);

    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "python analysis.py",
                "wait_for_completion": true,
                "preflight": {
                    "language": "python",
                    "packages": ["pandas"]
                }
            }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(result.success, "{}", result.content);
    let run: wisp_store::RunRecord = serde_json::from_str(&result.content).unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    let snapshot: serde_json::Value = serde_json::from_str(&run.env_snapshot_json).unwrap();
    assert_eq!(snapshot["preflight"]["status"], "passed");
    assert_eq!(snapshot["preflight"]["language"], "python");
    assert!(snapshot["preflight"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "packages" && check["status"] == "passed"));
    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands[0].script, "python interpreter/package preflight");
    assert!(commands[1].script.starts_with("prepare "));
    let prepare = commands[1].stdin.as_deref().unwrap();
    #[cfg(windows)]
    {
        // Windows prepare embeds the command as base64 into command.ps1.
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode("python analysis.py");
        assert!(prepare.contains(&encoded), "{prepare}");
    }
    #[cfg(not(windows))]
    {
        assert!(prepare.contains("python analysis.py"));
    }
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn run_in_context_rejects_nested_ssh_transfer_commands() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_run_ssh_guard_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let runner = Arc::new(FakeRunRunner::new(ok_output("should not run")));
    let tool = RunInContextTool::new(
        store.clone(),
        RunManager::with_runner(runner.clone()),
        "p".into(),
        None,
    );
    let result = tool
        .run(
            &serde_json::json!({
                "context_id": "local",
                "command": "rsync -a -e \"ssh -p 2222\" source/ host:/dest/"
            }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(!result.success);
    assert!(result.content.contains("transfer_between_contexts"));
    assert!(store.list_runs_by_project("p").await.unwrap().is_empty());
    assert!(
        runner.commands.lock().unwrap().is_empty(),
        "the guard must reject before anything reaches the runner"
    );
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn monitor_run_waits_once_for_an_existing_run() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_monitor_run_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let run = wisp_store::RunRecord::new("long-run", "p", "local", "Long run", "command");
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(
            "long-run",
            wisp_store::RunStatus::Submitted,
            "monitor-owner",
            60,
        )
        .await
        .unwrap());
    let snapshot = GetRunTool::new(store.clone(), "p".into())
        .run(
            &serde_json::json!({ "run_id": "long-run" }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;
    assert!(snapshot.success, "{}", snapshot.content);
    assert!(snapshot
        .content
        .contains("Call monitor_run with this run_id"));

    let finishing_store = store.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(finishing_store
            .transition_run_to_running_owned("long-run", "monitor-owner")
            .await
            .unwrap());
        assert!(finishing_store
            .finish_active_run_owned(
                "long-run",
                "monitor-owner",
                wisp_store::RunStatus::Succeeded,
                Some(0),
            )
            .await
            .unwrap());
    });

    let tool = MonitorRunTool::new(store, "p".into());
    let result = tool
        .run(
            &serde_json::json!({ "run_id": "long-run" }),
            &RunToolTestEnv(tmp.clone()),
        )
        .await;

    assert!(result.success, "{}", result.content);
    let run: wisp_store::RunRecord = serde_json::from_str(&result.content).unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn monitor_run_returns_early_when_mid_turn_guidance_is_pending() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!(
        "wisp_monitor_run_guidance_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new("long-run", "p", "ssh:gpu", "Long run", "ssh_direct");
    run.command = Some("sleep 3600".into());
    run.stdout_tail = Some("phase 2 of 9\n".into());
    run.last_polled_at = Some(chrono::Utc::now().timestamp());
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(
            "long-run",
            wisp_store::RunStatus::Running,
            "monitor-owner",
            60,
        )
        .await
        .unwrap());

    let queue = Arc::new(wisp_core::GuidanceQueue::default());
    let env = GuidanceRunToolEnv {
        root: tmp.clone(),
        queue: queue.clone(),
    };
    let tool = MonitorRunTool::new(store.clone(), "p".into());
    let waiting = tokio::spawn(async move {
        tool.run(&serde_json::json!({ "run_id": "long-run" }), &env)
            .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    queue.lock().unwrap().push((1, "你在进行哪一步？".into()));
    let result = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .expect("monitor_run should return once guidance is pending")
        .unwrap();

    assert!(result.success, "{}", result.content);
    let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["wait_interrupted"], true);
    assert_eq!(value["status"], "running");
    assert_eq!(value["stdout_tail"], "phase 2 of 9\n");
    assert!(
        value["next_action"]
            .as_str()
            .unwrap()
            .contains("call monitor_run again"),
        "{}",
        value["next_action"]
    );
    assert!(value["wait_detached"].is_null());
    let still = store.get_run("long-run").await.unwrap().unwrap();
    assert_eq!(still.status, wisp_store::RunStatus::Running);
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn builds_commands_for_local_ssh_and_wsl() {
    let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
    let ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
    let wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "Ubuntu").unwrap();

    let local_cmd = build_run_command(&local, "echo hi", Some(PathBuf::from("/tmp")));
    assert_eq!(local_cmd.script, "echo hi");
    assert_eq!(local_cmd.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    assert!(!local_cmd.program.is_empty());

    let ssh_cmd = build_run_command(&ssh, "echo hi", None);
    assert_eq!(ssh_cmd.program, "ssh");
    assert_eq!(ssh_cmd.args[0], "gpu-box");

    let wsl_cmd = build_run_command(&wsl, "echo hi", None);
    assert_eq!(wsl_cmd.program, "wsl.exe");
    assert!(wsl_cmd.args.contains(&"-d".to_string()));
    assert!(wsl_cmd.args.contains(&"Ubuntu-22.04".to_string()));
}

#[tokio::test]
async fn submit_run_records_success() {
    let tmp = std::env::temp_dir().join(format!("wisp_submit_run_{}.sqlite", uuid::Uuid::new_v4()));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: "hello\n".into(),
        stderr: String::new(),
    }));

    let res = submit_run_with_runner(
        &store,
        "p",
        None,
        SubmitRunRequest {
            context_id: "local".into(),
            command: "echo hello".into(),
            title: Some("Hello".into()),
            timeout_secs: Some(5),
            input_paths: None,
            output_specs: None,
        },
        &runner,
        None,
    )
    .await
    .unwrap();

    assert_eq!(res.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(res.exit_code, Some(0));
    assert_eq!(res.stdout_tail.as_deref(), Some("hello\n"));
    let run = store.get_run(&res.run_id).await.unwrap().unwrap();
    assert_eq!(run.context_id, "local");
    assert_eq!(run.command.as_deref(), Some("echo hello"));
    assert_eq!(run.title, "Hello");
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    // The success must come from executing the submitted command, not a
    // preloaded result short-circuiting the runner.
    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].context_id, "local");
    assert!(
        commands[0].script.contains("echo hello"),
        "unexpected script: {}",
        commands[0].script
    );
    drop(commands);

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn local_run_binds_inputs_before_execution_and_snapshots_environment() {
    let tmp = std::env::temp_dir().join(format!("wisp_local_run_inputs_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(tmp.join("data")).unwrap();
    std::fs::write(tmp.join("data/input.csv"), b"x\n1\n").unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: "ok".into(),
        stderr: String::new(),
    }));

    let result = submit_run_with_runner(
        &store,
        "p",
        Some("f"),
        SubmitRunRequest {
            context_id: "local".into(),
            command: "python analysis.py".into(),
            title: None,
            timeout_secs: Some(5),
            input_paths: Some(vec!["data/input.csv".into()]),
            output_specs: None,
        },
        &runner,
        Some(tmp.clone()),
    )
    .await
    .unwrap();

    let inputs = store.list_run_inputs(&result.run_id).await.unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].basis, wisp_store::LineageBasis::Declared);
    assert_eq!(inputs[0].confidence, wisp_store::LineageConfidence::Exact);
    let version = store
        .get_artifact_version(inputs[0].artifact_version_id.as_deref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        version.materialization,
        wisp_store::ArtifactMaterialization::Snapshot
    );
    let snapshot = tmp.join(&version.storage_path);
    std::fs::write(tmp.join("data/input.csv"), b"x\n2\n").unwrap();
    assert_eq!(std::fs::read(snapshot).unwrap(), b"x\n1\n");
    assert!(store
        .get_run_environment_snapshot(&result.run_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store.list_run_code_snapshots(&result.run_id).await.unwrap()[0].source_text,
        "python analysis.py"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn submit_run_records_failure() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_submit_run_fail_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let runner = FakeRunRunner::new(Err("timed out".into()));

    let res = submit_run_with_runner(
        &store,
        "p",
        None,
        SubmitRunRequest {
            context_id: "local".into(),
            command: "sleep 10".into(),
            title: None,
            timeout_secs: Some(1),
            input_paths: None,
            output_specs: None,
        },
        &runner,
        None,
    )
    .await
    .unwrap();

    assert_eq!(res.status, wisp_store::RunStatus::Failed);
    assert_eq!(res.exit_code, Some(-1));
    assert_eq!(res.stderr_tail.as_deref(), Some("timed out"));
    let run = store.get_run(&res.run_id).await.unwrap().unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Failed);
    assert_eq!(run.stderr_tail.as_deref(), Some("timed out"));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn submit_run_harvests_output_specs_on_success() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_submit_run_harvest_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(tmp.join("results")).unwrap();
    std::fs::write(tmp.join("results/out.tsv"), b"x\ty\n").unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: "done".into(),
        stderr: String::new(),
    }));

    let res = submit_run_with_runner(
        &store,
        "p",
        Some("f"),
        SubmitRunRequest {
            context_id: "local".into(),
            command: "make outputs".into(),
            title: None,
            timeout_secs: Some(5),
            input_paths: None,
            output_specs: Some(vec![crate::harvest::OutputSpec {
                glob: "results/*.tsv".into(),
                kind: "table".into(),
                residency: crate::harvest::OutputResidency::Auto,
                logical_key: None,
                max_file_mb: Some(1),
                max_total_mb: Some(1),
                bundle: false,
            }]),
        },
        &runner,
        Some(tmp.clone()),
    )
    .await
    .unwrap();

    let artifacts = store.list_artifacts("f").await.unwrap();
    assert_eq!(artifacts.len(), 1);
    let graph = store.research_graph("p").await.unwrap();
    assert!(graph.edges.iter().any(|edge| {
        edge.source_id == format!("run:{}", res.run_id)
            && edge.target_id == format!("artifact:{}", artifacts[0].0)
            && edge.relation == "produced"
    }));
    let run = store.get_run(&res.run_id).await.unwrap().unwrap();
    assert!(run.harvested_at.is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn background_run_can_be_cancelled_without_waiting_for_the_command() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_background_run_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let mut run = wisp_store::RunRecord::new("local-run", "p", "local", "Local", "local_detached");
    run.command = Some("long-running-analysis".into());
    run.timeout_secs = Some(60);
    run.remote_workdir = Some("~/.wisp-science/runs/local-run".into());
    run.remote_handle_json =
        Some(serde_json::to_string(&test_local_handle("local-run", true, None)).unwrap());
    run.status = wisp_store::RunStatus::Running;
    store.create_run(&run).await.unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(
        "__WISP_CANCEL__:cancelled\n",
    )]));
    let cancel_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *runner.rpc_gate.lock().unwrap() = Some(cancel_gate.clone());
    let manager = RunManager::with_runner(runner);

    manager.cancel(&store, "local-run").await.unwrap();
    assert_eq!(
        store.get_run("local-run").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Cancelling
    );
    assert!(manager.has_in_flight_project(&store, "p").await.unwrap());
    assert!(!manager
        .has_in_flight_project(&store, "other-project")
        .await
        .unwrap());
    cancel_gate.add_permits(1);
    assert_eq!(
        wait_for_terminal(&store, "local-run").await.status,
        wisp_store::RunStatus::Cancelled
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn second_cancel_force_finishes_a_wedged_cancelling_run() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_force_cancel_{}.sqlite", uuid::Uuid::new_v4()));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    let mut run = wisp_store::RunRecord::new("stuck-run", "p", "local", "Local", "local_detached");
    run.command = Some("Write-Host stuck".into());
    run.timeout_secs = Some(60);
    run.remote_workdir = Some("~\\.wisp-science\\runs\\stuck-run".into());
    run.remote_handle_json =
        Some(serde_json::to_string(&test_local_handle("stuck-run", true, None)).unwrap());
    run.status = wisp_store::RunStatus::Cancelling;
    run.last_poll_error = Some("SSH cancel response omitted status".into());
    store.create_run(&run).await.unwrap();
    // Cancel RPC stays wedged; the second cancel must not wait on it.
    let runner = Arc::new(ScriptedRunRunner::new(vec![]));
    let cancel_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *runner.rpc_gate.lock().unwrap() = Some(cancel_gate);
    let manager = RunManager::with_runner(runner);

    manager.cancel(&store, "stuck-run").await.unwrap();
    assert_eq!(
        store.get_run("stuck-run").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Cancelled
    );

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn remote_run_is_rejected_when_not_selected_for_its_session() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_remote_run_selection_{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let store = wisp_store::Store::open(&tmp).await.unwrap();
    store.create_project("p", "proj", "").await.unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
        .await
        .unwrap();
    let request = SubmitRunRequest {
        context_id: "ssh:gpu".into(),
        command: "echo remote".into(),
        title: None,
        timeout_secs: None,
        input_paths: None,
        output_specs: None,
    };
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
    }));

    let error = submit_run_with_runner(&store, "p", Some("f"), request.clone(), &runner, None)
        .await
        .unwrap_err();
    assert!(error.contains("not selected for this session"));

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn ssh_run_detaches_persists_handle_and_finishes_from_poller() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_lifecycle_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
    context.config_json = serde_json::json!({ "alias": "gpu" }).to_string();
    context.last_probe_status = Some("ok".into());
    store.upsert_execution_context(&context).await.unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
    ]));
    let manager = RunManager::with_runner(runner.clone());

    // The launch ACK contains a per-run token, so let the scripted runner
    // synthesize it from the prepare payload instead of hard-coding it.
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    runner.push(ok_output(&poll_response("finished:0", "complete", "")));
    // Hold the poller until the pre-completion status has been observed, so
    // the run cannot finish before the assertions below run under load.
    let poll_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *runner.rpc_gate.lock().unwrap() = Some(poll_gate.clone());
    let command = "printf '%s\\n' '$HOME' && printf '%s\\n' '$(date)'";
    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            Some("f".into()),
            SubmitRunRequest {
                context_id: "ssh:gpu".into(),
                command: command.into(),
                title: Some("Remote analysis".into()),
                timeout_secs: Some(3600),
                input_paths: None,
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();

    assert!(matches!(
        submitted.status,
        wisp_store::RunStatus::Submitted | wisp_store::RunStatus::Running
    ));
    assert!(submitted
        .remote_workdir
        .as_deref()
        .unwrap()
        .starts_with("~/.wisp-science/runs/"));
    poll_gate.add_permits(1);
    let finished = wait_for_terminal(&store, &submitted.run_id).await;
    assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(finished.exit_code, Some(0));
    assert_eq!(finished.stdout_tail.as_deref(), Some("complete"));
    assert!(finished
        .remote_handle_json
        .as_deref()
        .unwrap()
        .contains("ssh_direct"));

    let commands = runner.commands.lock().unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.program == "ssh")
            .count(),
        3
    );
    assert!(commands[0].stdin.as_deref().unwrap().contains(command));
    assert!(commands[0]
        .stdin
        .as_deref()
        .unwrap()
        .contains("setsid timeout -k 10"));
    assert!(!commands[0]
        .stdin
        .as_deref()
        .unwrap()
        .contains("else\n  bash -l"));
    assert!(!commands[1].stdin.as_deref().unwrap().contains(command));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_launch_failure_stops_after_the_first_attempt() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_stage_once_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("input.fasta"), b">seq\nACGT\n").unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
    context.config_json = serde_json::json!({ "alias": "gpu" }).to_string();
    context.last_probe_status = Some("ok".into());
    store.upsert_execution_context(&context).await.unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output(""),
        Err("temporary SSH disconnect".into()),
        // Post-failure reattach probe: nothing was submitted remotely, so the
        // original launch error must surface and the Run must fail.
        ok_output("__WISP_PREPARED__\n"),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner.clone());

    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            Some("f".into()),
            SubmitRunRequest {
                context_id: "ssh:gpu".into(),
                command: "wc -l input.fasta".into(),
                title: None,
                timeout_secs: Some(60),
                input_paths: Some(vec!["input.fasta".into()]),
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();

    let finished = wait_for_terminal(&store, &submitted.run_id).await;
    assert_eq!(finished.status, wisp_store::RunStatus::Failed);
    assert!(finished
        .last_poll_error
        .as_deref()
        .unwrap()
        .contains(SSH_RETRY_STOPPED_MARKER));
    let progress: wisp_store::RunProgress = serde_json::from_str(&finished.progress_json).unwrap();
    assert_eq!(progress.phase, "uploaded");
    assert_eq!(progress.completed_bytes, 10);
    assert_eq!(progress.total_bytes, 10);
    let commands = runner.commands.lock().unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.program == "scp")
            .count(),
        1
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.script == "launch SSH Run")
            .count(),
        1,
        "the reattach probe must never resend the launch"
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.script == "prepare SSH Run")
            .count(),
        2,
        "one prepare before launch, one reattach probe after the failure"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn local_launch_timeout_reattaches_when_supervisor_acknowledged() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_local_launch_reattach_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();

    let handle = test_local_handle("local-run", false, None);
    let RemoteRunHandle::LocalDetached { token, .. } = &handle else {
        unreachable!()
    };
    let token = token.clone();
    let mut run = wisp_store::RunRecord::new("local-run", "p", "local", "Local", "local_detached");
    run.command = Some("long-analysis".into());
    run.timeout_secs = Some(60);
    run.remote_handle_json = Some(serde_json::to_string(&handle).unwrap());
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle("local-run", wisp_store::RunStatus::Submitted, "owner", 360)
        .await
        .unwrap());

    let runner = ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        Err("run_in_context timed out after 20s".into()),
        ok_output(&format!("__WISP_HANDLE__:{token}:4242:999\n")),
    ]);
    let mut remote = RemoteRun {
        run_id: "local-run".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "long-analysis".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: Some(tmp.clone()),
        handle,
    };

    let confirmed = ensure_remote_started(&store, "owner", &runner, &mut remote)
        .await
        .unwrap();

    assert!(confirmed.is_confirmed());
    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 3);
    assert!(commands[2].script.starts_with("prepare local Run"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_launch_timeout_reattaches_when_supervisor_acknowledged() {
    let tmp =
        std::env::temp_dir().join(format!("wisp_ssh_launch_reattach_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();

    let handle = test_handle("ssh-run", false);
    let mut run = wisp_store::RunRecord::new("ssh-run", "p", "ssh:gpu", "Remote", "ssh_direct");
    run.command = Some("long-analysis".into());
    run.timeout_secs = Some(60);
    run.remote_handle_json = Some(serde_json::to_string(&handle).unwrap());
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle("ssh-run", wisp_store::RunStatus::Submitted, "owner", 360)
        .await
        .unwrap());

    // The remote supervisor wrote `_submitted`, but the launch RPC response
    // was lost to a transport timeout. The reattach probe must observe the
    // existing handle instead of failing the Run.
    let runner = ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        Err("launch SSH Run timed out after 20s".into()),
        ok_output("__WISP_HANDLE__:test-token:4242:999\n"),
    ]);
    let mut remote = RemoteRun {
        run_id: "ssh-run".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "long-analysis".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: Some(tmp.clone()),
        handle,
    };

    let confirmed = ensure_remote_started(&store, "owner", &runner, &mut remote)
        .await
        .unwrap();

    assert!(confirmed.is_confirmed());
    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[1].script, "launch SSH Run");
    assert_eq!(
        commands[2].script, "prepare SSH Run",
        "the probe re-reads the control directory and never relaunches"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn recovery_fails_unconfirmed_ssh_run_without_reconnecting() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_stale_start_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new("stale", "p", "ssh:gpu", "Stale", "ssh_direct");
    run.command = Some("echo stale".into());
    run.timeout_secs = Some(60);
    run.last_poll_error = Some("connection timed out".into());
    run.remote_workdir = Some("~/.wisp-science/runs/stale".into());
    run.remote_handle_json = Some(serde_json::to_string(&test_handle("stale", false)).unwrap());
    run.status = wisp_store::RunStatus::Submitted;
    store.create_run(&run).await.unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(Vec::new()));
    let manager = RunManager::with_runner(runner.clone());

    assert_eq!(manager.recover(&store).await.unwrap(), 0);
    let finished = wait_for_terminal(&store, "stale").await;
    assert_eq!(finished.status, wisp_store::RunStatus::Failed);
    assert!(finished
        .last_poll_error
        .as_deref()
        .unwrap()
        .contains(SSH_RETRY_STOPPED_MARKER));
    assert!(runner.commands.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn recovery_reattaches_ssh_after_transient_error_and_marks_local_lost() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_recover_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();

    let mut remote = wisp_store::RunRecord::new("remote", "p", "ssh:gpu", "Remote", "ssh_direct");
    remote.command = Some("long-analysis".into());
    remote.timeout_secs = Some(3600);
    remote.remote_workdir = Some("~/.wisp-science/runs/remote".into());
    remote.remote_handle_json = Some(serde_json::to_string(&test_handle("remote", true)).unwrap());
    remote.status = wisp_store::RunStatus::Running;
    store.create_run(&remote).await.unwrap();

    let mut local = wisp_store::RunRecord::new("local-run", "p", "local", "Local", "command");
    local.status = wisp_store::RunStatus::Running;
    store.create_run(&local).await.unwrap();

    let runner = Arc::new(ScriptedRunRunner::new(vec![
        Err("temporary SSH disconnect".into()),
        ok_output(&poll_response("finished:0", "reconnected", "")),
    ]));
    let manager = RunManager::with_runner(runner);
    assert_eq!(manager.recover(&store).await.unwrap(), 1);

    let finished = wait_for_terminal(&store, "remote").await;
    assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(finished.stdout_tail.as_deref(), Some("reconnected"));
    assert!(finished.last_poll_error.is_none());
    assert_eq!(
        store.get_run("local-run").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Lost
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn confirmed_ssh_run_stops_polling_after_authentication_failure() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_auth_stop_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();

    let mut run = wisp_store::RunRecord::new("remote", "p", "ssh:gpu", "Remote", "ssh_direct");
    run.command = Some("long-analysis".into());
    run.timeout_secs = Some(3600);
    run.remote_workdir = Some("~/.wisp-science/runs/remote".into());
    run.remote_handle_json = Some(serde_json::to_string(&test_handle("remote", true)).unwrap());
    run.status = wisp_store::RunStatus::Running;
    store.create_run(&run).await.unwrap();

    let runner = Arc::new(ScriptedRunRunner::new(vec![Err(
        "Permission denied (publickey).".into(),
    )]));
    let manager = RunManager::with_runner(runner.clone());

    assert_eq!(manager.recover(&store).await.unwrap(), 0);
    let finished = wait_for_terminal(&store, "remote").await;
    assert_eq!(finished.status, wisp_store::RunStatus::Lost);
    assert!(finished
        .last_poll_error
        .as_deref()
        .unwrap()
        .contains(SSH_RETRY_STOPPED_MARKER));
    assert_eq!(runner.commands.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_cancel_stays_cancelling_until_remote_group_confirms() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_cancel_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new("remote", "p", "ssh:gpu", "Remote", "ssh_direct");
    run.command = Some("long-analysis".into());
    run.timeout_secs = Some(3600);
    run.remote_workdir = Some("~/.wisp-science/runs/remote".into());
    run.remote_handle_json = Some(serde_json::to_string(&test_handle("remote", true)).unwrap());
    run.status = wisp_store::RunStatus::Running;
    store.create_run(&run).await.unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(
        "__WISP_CANCEL__:cancelled\n",
    )]));
    // Hold the remote cancel RPC so the group has not confirmed yet when the
    // pre-confirmation status is asserted, even under slow scheduling.
    let cancel_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *runner.rpc_gate.lock().unwrap() = Some(cancel_gate.clone());
    let manager = RunManager::with_runner(runner.clone());

    manager.cancel(&store, "remote").await.unwrap();
    assert_eq!(
        store.get_run("remote").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Cancelling
    );
    cancel_gate.add_permits(1);
    assert_eq!(
        wait_for_terminal(&store, "remote").await.status,
        wisp_store::RunStatus::Cancelled
    );
    let commands = runner.commands.lock().unwrap();
    let payload = commands[0].stdin.as_deref().unwrap();
    assert!(payload.contains("kill -TERM \"-4242\""));
    assert!(!payload.contains("kill -TERM --"));
    assert!(payload.contains("/proc/4242/stat"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn cancelling_ssh_input_staging_aborts_the_transfer() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_upload_cancel_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    let manager = RunManager::with_runner(Arc::new(ScriptedRunRunner::new(Vec::new())));
    let mut run = wisp_store::RunRecord::new("upload", "p", "ssh:gpu", "Upload", "ssh_direct");
    run.command = Some("analysis input.dat".into());
    run.timeout_secs = Some(3600);
    run.remote_workdir = Some("~/.wisp-science/runs/upload".into());
    run.remote_handle_json = Some(serde_json::to_string(&test_handle("upload", false)).unwrap());
    run.progress_json = serde_json::to_string(&wisp_store::RunProgress {
        phase: "uploading".into(),
        direction: "upload".into(),
        completed_bytes: 25,
        total_bytes: 100,
        files_completed: 0,
        files_total: 1,
        current_file: Some("input.dat".into()),
        bytes_per_second: Some(10),
        eta_seconds: Some(8),
        updated_at: chrono::Utc::now().timestamp(),
    })
    .unwrap();
    run.status = wisp_store::RunStatus::Submitted;
    store.create_run(&run).await.unwrap();
    assert!(store
        .claim_run_lifecycle("upload", &manager.owner_id, ACTIVE_LEASE_SECS)
        .await
        .unwrap());
    let task = tokio::spawn(std::future::pending::<()>());
    manager.active.lock().await.insert(
        "upload".into(),
        ActiveRun {
            abort: task.abort_handle(),
        },
    );

    manager.cancel(&store, "upload").await.unwrap();

    assert!(task.await.unwrap_err().is_cancelled());
    let run = store.get_run("upload").await.unwrap().unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Cancelled);
    let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
    assert_eq!(progress.phase, "cancelled");
    assert_eq!(progress.completed_bytes, 25);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn tail_preserves_utf8_boundaries() {
    let s = format!("{}{}", "a".repeat(3999), "科研");
    let out = tail(&s);
    assert!(out.starts_with('a') || out.starts_with('科'));
    assert!(out.ends_with("科研"));
}

#[cfg(unix)]
#[test]
fn remote_control_payloads_are_valid_posix_shell() {
    let remote = RemoteRun {
        run_id: "payload".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "printf '%s\\n' ok".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: None,
        handle: test_handle("payload", true),
    };
    let local = RemoteRun {
        run_id: "local-payload".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "printf '%s\\n' ok".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: None,
        handle: test_local_handle("local-payload", true, Some("/home/user/project")),
    };
    let wsl = RemoteRun {
        run_id: "wsl-payload".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "printf '%s\\n' ok".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: None,
        handle: test_wsl_handle("wsl-payload", true, Some(r"C:\Users\me\project")),
    };
    let scripts = [
        prepare_payload(&remote),
        launch_payload(&remote.handle),
        poll_payload(&remote.handle).unwrap(),
        cancel_payload(&remote.handle).unwrap(),
        prepare_payload(&local),
        launch_payload(&local.handle),
        poll_payload(&local.handle).unwrap(),
        cancel_payload(&local.handle).unwrap(),
        prepare_payload(&wsl),
        launch_payload(&wsl.handle),
        poll_payload(&wsl.handle).unwrap(),
        cancel_payload(&wsl.handle).unwrap(),
    ];
    for script in scripts {
        let mut child = std::process::Command::new("sh")
            .args(["-n", "-s"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success(), "invalid shell payload");
    }
    let local_prepare = prepare_payload(&local);
    assert!(local_prepare.contains("sleep 60"));
    assert!(!local_prepare.contains("setsid timeout"));
    // A relaunched supervisor must never rerun the command.
    assert!(local_prepare.contains("if [ -f _submitted ]; then"));
    // The local project root is entered directly; WSL goes through wslpath.
    assert!(local_prepare.contains("cd '/home/user/project' || exit 125"));
    assert!(!local_prepare.contains("wslpath"));
    let wsl_prepare = prepare_payload(&wsl);
    assert!(wsl_prepare.contains(r#"cd "$(wslpath 'C:\Users\me\project')" || exit 125"#));
    // Signals to the app's process group must not reach a detached supervisor.
    assert!(launch_payload(&local.handle).contains("nohup setsid sh"));
}

#[test]
fn remote_compute_skill_uses_the_real_wisp_run_contract() {
    let skill = include_str!("../../../../skills/remote-compute-ssh/SKILL.md");
    for tool in [
        "run_in_context",
        "get_run",
        "monitor_run",
        "cancel_run",
        "configure_ssh_trust",
        "transfer_between_contexts",
    ] {
        assert!(skill.contains(tool), "missing {tool}");
    }
    for stale in [
        "host.compute",
        "wait_for_notification",
        "compute_details",
        "submit_job",
        "attach_job",
        "repl tool",
    ] {
        assert!(!skill.contains(stale), "stale API remains: {stale}");
    }
    assert!(skill.contains("wait_interrupted"));
    assert!(skill.contains("call `monitor_run` again"));
    assert!(skill.contains("never call it repeatedly"));
    assert!(!skill.contains("ssh <alias>"));
    assert!(skill.contains("Scheduler lifecycle is not implemented yet"));
}

/// Single-response fake that records every command it receives, so tests can
/// assert what actually reached the runner (or that nothing did).
struct FakeRunRunner {
    output: Result<RunCommandOutput, String>,
    commands: StdMutex<Vec<RunCommand>>,
}

impl FakeRunRunner {
    fn new(output: Result<RunCommandOutput, String>) -> Self {
        Self {
            output,
            commands: StdMutex::new(Vec::new()),
        }
    }
}

/// Streaming fake with explicit synchronization instead of wall-clock sleeps:
/// it signals `first_chunk_sent` after emitting the stdout chunk, then blocks
/// until the test grants a `finish` permit, so the test can assert the
/// persisted mid-run state without racing the lifecycle task.
struct StreamingRunRunner {
    first_chunk_sent: Arc<tokio::sync::Notify>,
    finish: Arc<tokio::sync::Semaphore>,
}

#[async_trait::async_trait]
impl RunCommandRunner for StreamingRunRunner {
    async fn run(
        &self,
        _command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        unreachable!("streaming lifecycle must call run_streaming")
    }

    async fn run_streaming(
        &self,
        _command: RunCommand,
        _timeout: Duration,
        updates: tokio::sync::mpsc::UnboundedSender<RunOutputUpdate>,
    ) -> Result<RunCommandOutput, String> {
        updates
            .send(RunOutputUpdate {
                stream: RunOutputStream::Stdout,
                chunk: b"phase 1 complete\n".to_vec(),
            })
            .unwrap();
        self.first_chunk_sent.notify_one();
        let _permit = self.finish.acquire().await.unwrap();
        updates
            .send(RunOutputUpdate {
                stream: RunOutputStream::Stderr,
                chunk: b"warning: slow API\n".to_vec(),
            })
            .unwrap();
        Ok(RunCommandOutput {
            exit_code: 0,
            stdout: "phase 1 complete\n".into(),
            stderr: "warning: slow API\n".into(),
        })
    }
}

#[tokio::test]
async fn local_run_streams_bounded_output_and_heartbeat_before_completion() {
    let tmp = std::env::temp_dir().join(format!("wisp_run_stream_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let run = wisp_store::RunRecord::new("streaming", "p", "local", "Streaming", "command");
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(
            "streaming",
            wisp_store::RunStatus::Running,
            "stream-owner",
            60,
        )
        .await
        .unwrap());

    let first_chunk_sent = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Semaphore::new(0));
    let runner = StreamingRunRunner {
        first_chunk_sent: first_chunk_sent.clone(),
        finish: finish.clone(),
    };
    let task_store = store.clone();
    let task = tokio::spawn(async move {
        run_with_lifecycle_lease(
            &task_store,
            "streaming",
            "stream-owner",
            &runner,
            RunCommand {
                context_id: "local".into(),
                program: "unused".into(),
                args: Vec::new(),
                script: "stream test".into(),
                cwd: None,
                stdin: None,
                envs: Vec::new(),
            },
            Duration::from_secs(10),
        )
        .await
    });

    // The runner holds mid-run until the test releases it, so waiting for the
    // lifecycle task to flush the first chunk is bounded, not a race.
    tokio::time::timeout(Duration::from_secs(5), first_chunk_sent.notified())
        .await
        .expect("runner never emitted its first chunk");
    let live = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let run = store.get_run("streaming").await.unwrap().unwrap();
            if run.stdout_tail.is_some() && run.last_polled_at.is_some() {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first chunk was never flushed to the store");
    assert_eq!(live.status, wisp_store::RunStatus::Running);
    assert_eq!(live.stdout_tail.as_deref(), Some("phase 1 complete\n"));
    assert!(live.last_polled_at.is_some(), "heartbeat was not recorded");

    finish.add_permits(1);
    let output = task.await.unwrap().unwrap();
    assert_eq!(output.exit_code, 0);
    let _ = std::fs::remove_dir_all(tmp);
}

#[async_trait::async_trait]
impl RunCommandRunner for FakeRunRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        self.commands.lock().unwrap().push(command);
        self.output.clone()
    }
}

struct ScriptedRunRunner {
    outputs: StdMutex<VecDeque<Result<RunCommandOutput, String>>>,
    commands: StdMutex<Vec<RunCommand>>,
    synthesize_launch_ack: std::sync::atomic::AtomicBool,
    token: StdMutex<Option<String>>,
    // When set, poll/cancel SSH RPCs block until the test releases a permit,
    // so a test can observe pre-confirmation state without racing the
    // background lifecycle task.
    rpc_gate: StdMutex<Option<Arc<tokio::sync::Semaphore>>>,
}

impl ScriptedRunRunner {
    fn new(outputs: Vec<Result<RunCommandOutput, String>>) -> Self {
        Self {
            outputs: StdMutex::new(outputs.into()),
            commands: StdMutex::new(Vec::new()),
            synthesize_launch_ack: std::sync::atomic::AtomicBool::new(false),
            token: StdMutex::new(None),
            rpc_gate: StdMutex::new(None),
        }
    }

    fn push(&self, output: Result<RunCommandOutput, String>) {
        self.outputs.lock().unwrap().push_back(output);
    }
}

#[async_trait::async_trait]
impl RunCommandRunner for ScriptedRunRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        let is_poll_or_cancel =
            command.script.starts_with("poll ") || command.script.starts_with("cancel ");
        if is_poll_or_cancel {
            let gate = self.rpc_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                let _permit = gate.acquire().await.unwrap();
            }
        }
        if command.script.starts_with("prepare ") {
            if let Some(payload) = command.stdin.as_deref() {
                // Posix and Windows prepare payloads both write a token; parse
                // each form independently so a failed Posix prefix match does
                // not short-circuit the Windows branch via `?`.
                let token = payload.lines().find_map(|line| {
                    line.strip_prefix("  printf '%s\\n' '")
                        .and_then(|rest| rest.strip_suffix("' > \"$workdir/token.tmp\""))
                        .or_else(|| {
                            line.trim()
                                .strip_prefix(
                                    "Set-Content -LiteralPath ($tokenPath + '.tmp') -Value '",
                                )
                                .and_then(|rest| rest.strip_suffix("' -Encoding ascii"))
                        })
                        .map(str::to_string)
                });
                *self.token.lock().unwrap() = token;
            }
        }
        self.commands.lock().unwrap().push(command.clone());
        let output = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(format!("unexpected command: {}", command.script)))?;
        if command.script.starts_with("launch ")
            && self
                .synthesize_launch_ack
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            let token = self.token.lock().unwrap().clone().unwrap();
            return Ok(RunCommandOutput {
                exit_code: 0,
                stdout: format!("__WISP_HANDLE__:{token}:4242:999\n"),
                stderr: String::new(),
            });
        }
        Ok(output)
    }
}

fn ok_output(stdout: &str) -> Result<RunCommandOutput, String> {
    Ok(RunCommandOutput {
        exit_code: 0,
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

#[tokio::test]
async fn ssh_download_uses_context_connection_options() {
    let tmp = std::env::temp_dir().join(format!("wisp-run-download-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store.create_project("p", "project", "").await.unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_TRANSFER_SIZE__:42\n"),
        ok_output(""),
    ]));
    let manager = RunManager::with_runner(runner.clone());
    let identity =
        std::env::temp_dir().join(format!("wisp-run-download-key-{}", uuid::Uuid::new_v4()));
    std::fs::write(&identity, b"test-key\n").unwrap();
    let mut context = wisp_store::ExecutionContext::new("ssh:CPU", "CPU").unwrap();
    context.config_json = serde_json::json!({
        "alias": "cpu.example",
        "user": "alice",
        "port": 2222,
        "identity_file": identity.to_string_lossy(),
    })
    .to_string();
    context.last_probe_status = Some("ok".into());
    let destination = tmp.join("results.tar.gz");

    manager
        .download_ssh_file(
            &store,
            "p",
            None,
            &context,
            "/data/results.tar.gz",
            &destination,
        )
        .await
        .unwrap();

    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].script, "measure SSH download");
    assert_eq!(commands[1].program, "scp");
    assert!(commands[1]
        .args
        .windows(2)
        .any(|args| args == ["-P", "2222"]));
    assert!(commands[1]
        .args
        .windows(2)
        .any(|args| { args[0] == "-i" && args[1] == identity.to_string_lossy() }));
    assert_eq!(
        &commands[1].args[commands[1].args.len() - 2..],
        [
            "alice@cpu.example:/data/results.tar.gz".to_string(),
            destination.to_string_lossy().into_owned()
        ]
    );
    drop(commands);
    let run = store
        .list_runs_by_project("p")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(run.kind, "file_transfer");
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
    assert_eq!(progress.phase, "downloaded");
    assert_eq!(progress.completed_bytes, 42);
    let _ = std::fs::remove_file(identity);
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn parses_remote_input_progress_without_confusing_missing_files() {
    let parsed = parse_input_progress(
        "noise\n__WISP_TRANSFER_FILE__:a.fastq.gz:1024\n__WISP_TRANSFER_FILE__:empty.txt:0\n",
    );
    assert_eq!(parsed.get("a.fastq.gz"), Some(&1024));
    assert_eq!(parsed.get("empty.txt"), Some(&0));
    assert!(!parsed.contains_key("missing.fastq.gz"));
}

#[test]
fn parse_remote_poll_accepts_windows_crlf_markers() {
    // PowerShell Write-Output uses CRLF; without normalization the host keeps
    // retrying poll forever even after the command has already finished.
    let raw = "__WISP_RUN_STATUS__:finished:0\r\n__WISP_STDOUT__\r\nCIM/launch fix test OK\r\n\r\n__WISP_STDERR__\r\n\r\n";
    let poll = remote::parse_remote_poll(raw).unwrap();
    assert_eq!(poll.state, remote::RemotePollState::Finished(0));
    assert_eq!(poll.stdout, "CIM/launch fix test OK");
    assert_eq!(poll.stderr, "");
}

#[test]
fn parse_remote_poll_accepts_empty_finished_exit_code() {
    // A Windows supervisor that read ExitCode before WaitForExit wrote `done:`.
    let raw = "__WISP_RUN_STATUS__:finished:\n__WISP_STDOUT__\nok\n__WISP_STDERR__\n\n";
    let poll = remote::parse_remote_poll(raw).unwrap();
    assert_eq!(poll.state, remote::RemotePollState::Finished(0));
}

#[test]
fn parse_remote_cancel_accepts_empty_finished_exit_code() {
    let cancel = remote::parse_remote_cancel("__WISP_CANCEL__:finished:\r\n").unwrap();
    assert_eq!(cancel, remote::RemoteCancel::Finished(0));
}

fn poll_response(status: &str, stdout: &str, stderr: &str) -> String {
    format!("__WISP_RUN_STATUS__:{status}\n__WISP_STDOUT__\n{stdout}\n__WISP_STDERR__\n{stderr}\n")
}

fn test_handle(run_id: &str, confirmed: bool) -> RemoteRunHandle {
    RemoteRunHandle::SshDirect {
        connection: crate::ssh_hosts::SshConnection {
            alias: "gpu".into(),
            host_name: None,
            user: None,
            port: None,
            identity_file: None,
            auth_method: crate::ssh_hosts::SshAuthMethod::Key,
        },
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "test-token".into(),
        inputs_staged: false,
        pgid: confirmed.then_some(4242),
        start_time: confirmed.then_some(999),
    }
}

fn test_local_handle(run_id: &str, confirmed: bool, command_cwd: Option<&str>) -> RemoteRunHandle {
    // Match the host platform's real local transport so cancel/poll helpers
    // exercise the same payloads production uses.
    #[cfg(windows)]
    let transport = LocalTransport::Windows {
        context_id: "local".into(),
    };
    #[cfg(not(windows))]
    let transport = LocalTransport::Posix {
        context_id: "local".into(),
        program: "sh".into(),
        args: vec!["-s".into()],
    };
    RemoteRunHandle::LocalDetached {
        transport,
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "test-token".into(),
        inputs_staged: true,
        pgid: confirmed.then_some(4242),
        start_identity: confirmed.then(|| "999".into()),
        command_cwd: command_cwd.map(str::to_string),
    }
}

#[cfg(unix)]
fn test_wsl_handle(run_id: &str, confirmed: bool, command_cwd: Option<&str>) -> RemoteRunHandle {
    RemoteRunHandle::LocalDetached {
        transport: LocalTransport::Posix {
            context_id: "wsl:Ubuntu".into(),
            program: "wsl.exe".into(),
            args: vec![
                "-d".into(),
                "Ubuntu".into(),
                "--".into(),
                "sh".into(),
                "-s".into(),
            ],
        },
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "test-token".into(),
        inputs_staged: true,
        pgid: confirmed.then_some(4242),
        start_identity: confirmed.then(|| "999".into()),
        command_cwd: command_cwd.map(str::to_string),
    }
}

#[test]
fn permanent_remote_start_errors_require_user_intervention() {
    for error in [
        "SSH prepare failed with exit 255: Permission denied (publickey,password).",
        "Received disconnect: Too many authentication failures",
        "SSH input staging failed: Could not resolve hostname server",
        "Host key verification failed.",
        "kex_exchange_identification: read: Connection reset by peer",
        "kex_exchange_identification: Connection closed by remote host",
    ] {
        assert!(permanent_remote_start_error(error), "{error}");
    }
    assert!(permanent_remote_start_error(
        "SSH authentication gate blocked for `ssh:gpu` after a previous failure"
    ));
    for transient in [
        "SSH poll failed: Connection reset by peer",
        "SSH poll failed: Connection timed out",
        "connect timed out",
        "No route to host",
        "Network is unreachable",
        "Connection closed by remote host",
    ] {
        assert!(
            !permanent_remote_start_error(transient),
            "transient must not end a confirmed run: {transient}"
        );
    }
}

#[test]
fn remote_poll_transport_errors_back_off_without_exceeding_the_lease() {
    assert_eq!(remote_poll_delay_secs(0), 5);
    assert_eq!(remote_poll_delay_secs(1), 5);
    assert_eq!(remote_poll_delay_secs(2), 10);
    assert_eq!(remote_poll_delay_secs(3), 20);
    assert_eq!(remote_poll_delay_secs(100), 20);
    assert!(remote_poll_delay_secs(100) < ACTIVE_LEASE_SECS as u64);
}

#[test]
fn persisted_ssh_handles_without_staging_flag_remain_compatible() {
    let handle: RemoteRunHandle = serde_json::from_str(
        r#"{"kind":"ssh_direct","connection":{"alias":"gpu"},"workdir":".wisp-science/runs/old","token":"old-token","pgid":null,"start_time":null}"#,
    )
    .unwrap();
    assert!(!handle.inputs_staged());
}

#[test]
fn ssh_start_keeps_a_lease_longer_than_the_input_staging_timeout() {
    let pending = RemoteRun {
        run_id: "pending".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "echo pending".into(),
        timeout: Duration::from_secs(60),
        input_refs: vec!["input.fasta".into()],
        output_specs: Vec::new(),
        harvest_root: None,
        handle: test_handle("pending", false),
    };
    assert!(REMOTE_START_LEASE_SECS > 300);
    assert_eq!(
        remote_lifecycle_lease_secs(&pending),
        REMOTE_START_LEASE_SECS
    );

    let mut running = pending;
    running.handle = test_handle("running", true);
    assert_eq!(remote_lifecycle_lease_secs(&running), ACTIVE_LEASE_SECS);
}

#[cfg(windows)]
#[test]
fn scp_local_paths_strip_windows_extended_length_prefixes() {
    assert_eq!(
        scp_local_path(std::path::Path::new(r"\\?\E:\shui-jue\input.fasta")),
        r"E:\shui-jue\input.fasta"
    );
    assert_eq!(
        scp_local_path(std::path::Path::new(r"\\?\UNC\server\share\input.fasta")),
        r"\\server\share\input.fasta"
    );
}

async fn wait_for_terminal(store: &wisp_store::Store, run_id: &str) -> wisp_store::RunRecord {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn local_and_wsl_timeout_accepts_values_above_300s() {
    let tmp = std::env::temp_dir().join(format!("wisp_timeout_clamp_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
    wsl.config_json = serde_json::json!({ "distro": "Ubuntu" }).to_string();
    store.upsert_execution_context(&wsl).await.unwrap();
    store
        .set_session_execution_context_enabled("f", "wsl:Ubuntu", true)
        .await
        .unwrap();

    for (context_id, frame_id) in [("local", None), ("wsl:Ubuntu", Some("f"))] {
        let prepared = create_run_record(
            &store,
            "p",
            frame_id,
            SubmitRunRequest {
                context_id: context_id.into(),
                command: "sleep 1".into(),
                title: None,
                timeout_secs: Some(3600),
                input_paths: None,
                output_specs: None,
            },
            Some(tmp.clone()),
            wisp_store::RunStatus::Submitted,
            "owner",
            REMOTE_START_LEASE_SECS,
            None,
        )
        .await
        .unwrap();
        assert_eq!(prepared.timeout, Duration::from_secs(3600));
        let run = store.get_run(&prepared.run_id).await.unwrap().unwrap();
        assert_eq!(run.timeout_secs, Some(3600));
        assert_eq!(run.kind, "local_detached");
        assert!(run
            .remote_handle_json
            .as_deref()
            .unwrap()
            .contains("local_detached"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn local_detached_run_finishes_from_poller() {
    let tmp = std::env::temp_dir().join(format!("wisp_local_lifecycle_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    runner.push(ok_output(&poll_response("finished:0", "local-done", "")));
    let poll_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *runner.rpc_gate.lock().unwrap() = Some(poll_gate.clone());
    let manager = RunManager::with_runner(runner.clone());
    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            None,
            SubmitRunRequest {
                context_id: "local".into(),
                command: "printf done".into(),
                title: Some("Local analysis".into()),
                timeout_secs: Some(7200),
                input_paths: None,
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();
    let workdir = submitted.remote_workdir.as_deref().unwrap();
    #[cfg(windows)]
    assert!(workdir.starts_with("~\\.wisp-science\\runs\\"), "{workdir}");
    #[cfg(not(windows))]
    assert!(workdir.starts_with("~/.wisp-science/runs/"), "{workdir}");
    poll_gate.add_permits(1);
    let finished = wait_for_terminal(&store, &submitted.run_id).await;
    assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(finished.stdout_tail.as_deref(), Some("local-done"));
    assert_eq!(finished.timeout_secs, Some(7200));
    let commands = runner.commands.lock().unwrap();
    let prepare = commands[0].stdin.as_deref().unwrap();
    #[cfg(windows)]
    {
        let shell = local_detached::windows_powershell_program();
        assert!(
            commands.iter().any(|command| command.program == shell),
            "expected host shell {shell}"
        );
        // Timeout lives inside the base64-encoded supervisor.ps1 body.
        use base64::Engine as _;
        let supervisor = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(
                    prepare
                        .lines()
                        .find(|line| {
                            line.starts_with("[System.IO.File]::WriteAllText($supervisorPath")
                        })
                        .and_then(|line| line.split("FromBase64String('").nth(1))
                        .and_then(|rest| rest.split('\'').next())
                        .expect("supervisor base64 in prepare payload"),
                )
                .expect("valid supervisor base64"),
        )
        .expect("utf8 supervisor script");
        assert!(supervisor.contains("AddSeconds(7200)"), "{supervisor}");
    }
    #[cfg(not(windows))]
    {
        assert!(commands.iter().any(|command| command.program == "sh"));
        assert!(prepare.contains("sleep 7200"), "{prepare}");
        assert!(!prepare.contains("setsid timeout"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn wsl_detached_run_uses_wsl_transport_and_finishes() {
    let tmp = std::env::temp_dir().join(format!("wisp_wsl_lifecycle_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
    wsl.config_json = serde_json::json!({ "distro": "Ubuntu" }).to_string();
    store.upsert_execution_context(&wsl).await.unwrap();
    store
        .set_session_execution_context_enabled("f", "wsl:Ubuntu", true)
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output("__WISP_HANDLE__:token-will-be-replaced"),
        ok_output(&poll_response("finished:0", "wsl-done", "")),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner.clone());
    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            Some("f".into()),
            SubmitRunRequest {
                context_id: "wsl:Ubuntu".into(),
                command: "sleep 1 && echo done".into(),
                title: None,
                timeout_secs: Some(600),
                input_paths: None,
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();
    let finished = wait_for_terminal(&store, &submitted.run_id).await;
    assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
    assert_eq!(finished.timeout_secs, Some(600));
    let commands = runner.commands.lock().unwrap();
    assert!(commands.iter().all(|command| command.program == "wsl.exe"));
    assert!(commands[0].args.contains(&"-d".to_string()));
    assert!(commands[0].args.contains(&"Ubuntu".to_string()));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn windows_control_payloads_contain_process_identity_and_timeout() {
    let remote = RemoteRun {
        run_id: "win".into(),
        project_id: "p".into(),
        frame_id: None,
        command: "Write-Output ok".into(),
        timeout: Duration::from_secs(120),
        input_refs: Vec::new(),
        output_specs: Vec::new(),
        harvest_root: None,
        handle: RemoteRunHandle::LocalDetached {
            transport: LocalTransport::Windows {
                context_id: "local".into(),
            },
            workdir: ".wisp-science/runs/win".into(),
            token: "test-token".into(),
            inputs_staged: true,
            pgid: Some(4242),
            start_identity: Some("639000105000000000".into()),
            command_cwd: Some(r"C:\project".into()),
        },
    };
    use base64::Engine as _;
    let prepare = prepare_payload(&remote);
    assert!(prepare.contains("FromBase64String"));
    assert!(prepare.contains("__WISP_PREPARED__"));
    let supervisor = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(
                prepare
                    .lines()
                    .find(|line| line.starts_with("[System.IO.File]::WriteAllText($supervisorPath"))
                    .and_then(|line| line.split("FromBase64String('").nth(1))
                    .and_then(|rest| rest.split('\'').next())
                    .expect("supervisor base64 in prepare payload"),
            )
            .expect("valid supervisor base64"),
    )
    .expect("utf8 supervisor script");
    // The supervisor must be idempotent and use a culture-stable identity from
    // System.Diagnostics.Process; CIM can be unavailable or miss fast exits.
    assert!(supervisor.contains("if (Test-Path -LiteralPath (Join-Path $workdir '_submitted'))"));
    assert!(supervisor.contains("$proc.StartTime.ToUniversalTime().Ticks"));
    assert!(!supervisor.contains("Get-CimInstance"));
    // Start-Process -PassThru + RedirectStandard* returns a null ExitCode on
    // Windows PowerShell 5.1; the .NET Process API works on both 5.1 and 7.
    assert!(supervisor.contains("New-Object System.Diagnostics.ProcessStartInfo"));
    assert!(supervisor.contains("CopyToAsync"));
    assert!(supervisor.contains("-ExecutionPolicy Bypass"));
    assert!(!supervisor.contains("Start-Process @startParams"));
    let launch = launch_payload(&remote.handle);
    assert!(launch.contains("Start-Process"));
    assert!(launch.contains("'-ExecutionPolicy','Bypass','-File'"));
    // Supervisor must follow the host engine (pwsh when present, else powershell).
    assert!(launch.contains("GetCurrentProcess().MainModule.FileName"));
    // Only the launcher that created the lock may start the supervisor, and a
    // live lock owner must not be raced.
    assert!(launch.contains("if ($acquired)"));
    assert!(launch.contains("Get-Process -Id $ownerId"));
    assert!(launch.contains("supervisor.stderr.log"));
    assert!(launch.contains("local supervisor did not acknowledge launch: "));
    let poll = poll_payload(&remote.handle).unwrap();
    assert!(poll.contains("Get-Process -Id 4242"));
    assert!(poll.contains("__WISP_RUN_STATUS__"));
    assert!(poll.contains("StartTime.ToUniversalTime().Ticks"));
    // Log tails must share the writer's handle and stay bounded.
    assert!(poll.contains("[System.IO.FileShare]::ReadWrite"));
    assert!(!poll.contains("ReadAllBytes"));
    let cancel = cancel_payload(&remote.handle).unwrap();
    assert!(cancel.contains("taskkill.exe"));
    assert!(cancel.contains("__WISP_CANCEL__"));
    assert!(cancel.contains("StartTime.ToUniversalTime().Ticks"));
}

#[test]
fn windows_transport_executes_stdin_as_one_script() {
    let handle = RemoteRunHandle::LocalDetached {
        transport: LocalTransport::Windows {
            context_id: "local".into(),
        },
        workdir: ".wisp-science/runs/win".into(),
        token: "test-token".into(),
        inputs_staged: true,
        pgid: None,
        start_identity: None,
        command_cwd: None,
    };
    let command =
        local_detached::transport_script_command(&handle, "prepare local Run", "exit 0".into())
            .unwrap();
    assert_eq!(
        command.program,
        local_detached::windows_powershell_program()
    );
    // `-Command -` parses stdin line-by-line like an interactive session on
    // Windows PowerShell 5.1; the same form works under pwsh.
    assert!(!command.args.contains(&"-".to_string()));
    assert!(command
        .args
        .contains(&"[Console]::In.ReadToEnd() | Invoke-Expression".to_string()));
    assert_eq!(command.stdin.as_deref(), Some("exit 0"));
}

#[cfg(windows)]
#[test]
fn windows_shell_prefers_pwsh_when_present_on_path() {
    let program = local_detached::windows_powershell_program();
    let has_pwsh = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .any(|dir| dir.join("pwsh.exe").is_file() || dir.join("pwsh").is_file())
        })
        .unwrap_or(false);
    if has_pwsh {
        assert_eq!(program, "pwsh");
    } else {
        assert_eq!(program, "powershell");
    }
}

#[test]
fn handle_ack_preserves_identities_containing_colons_and_spaces() {
    let handle = test_local_handle("mac", false, None);
    let confirmed = remote::handle_from_ack(
        &handle,
        "__WISP_HANDLE__:test-token:4242:Mon Aug  3 10:55:00 2026\n",
    )
    .unwrap();
    let RemoteRunHandle::LocalDetached {
        pgid,
        start_identity,
        ..
    } = confirmed
    else {
        panic!("expected LocalDetached");
    };
    assert_eq!(pgid, Some(4242));
    assert_eq!(start_identity.as_deref(), Some("Mon Aug  3 10:55:00 2026"));
}

#[cfg(unix)]
#[tokio::test]
async fn local_detached_real_shell_lifecycle() {
    let tmp = std::env::temp_dir().join(format!("wisp_local_real_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
        .await
        .unwrap();
    let manager = RunManager::new();
    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            None,
            SubmitRunRequest {
                context_id: "local".into(),
                command: "printf 'real-shell-ok\\n'".into(),
                title: Some("Real local".into()),
                timeout_secs: Some(60),
                input_paths: None,
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();
    let finished = wait_for_terminal(&store, &submitted.run_id).await;
    assert_eq!(
        finished.status,
        wisp_store::RunStatus::Succeeded,
        "stderr={:?} poll_error={:?} handle={:?}",
        finished.stderr_tail,
        finished.last_poll_error,
        finished.remote_handle_json
    );
    assert_eq!(finished.exit_code, Some(0));
    assert!(finished
        .stdout_tail
        .as_deref()
        .unwrap_or_default()
        .contains("real-shell-ok"));
    if let Some(workdir) = finished.remote_workdir.as_deref() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let path = workdir.replacen("~", &home, 1);
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

// --- SSH harvest v2 -------------------------------------------------------

fn sha256_hex_of(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn harvest_test_context() -> wisp_store::ExecutionContext {
    let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
    context.config_json = serde_json::json!({ "alias": "gpu" }).to_string();
    context.last_probe_status = Some("ok".into());
    context
}

fn harvest_test_remote(
    run_id: &str,
    tmp: &Path,
    specs: Vec<crate::harvest::OutputSpec>,
) -> RemoteRun {
    let connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&harvest_test_context()).unwrap();
    RemoteRun {
        run_id: run_id.into(),
        project_id: "p".into(),
        frame_id: Some("f".into()),
        command: "make outputs".into(),
        timeout: Duration::from_secs(60),
        input_refs: Vec::new(),
        output_specs: specs,
        harvest_root: Some(tmp.to_path_buf()),
        handle: RemoteRunHandle::SshDirect {
            connection,
            workdir: format!(".wisp-science/runs/{run_id}"),
            token: "harvest-token".into(),
            inputs_staged: true,
            pgid: Some(4242),
            start_time: Some(99),
        },
    }
}

async fn seed_harvest_run(tmp: &Path, run_id: &str) -> wisp_store::Store {
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new(run_id, "p", "ssh:gpu", "Remote", "ssh_direct");
    run.frame_id = Some("f".into());
    run.status = wisp_store::RunStatus::Succeeded;
    store.create_run(&run).await.unwrap();
    store
}

/// Answers the collect RPC with a fixed manifest and materializes the scp
/// download by writing the prepared files under the destination directory.
struct HarvestFakeRunner {
    manifest: String,
    files: Vec<(String, Vec<u8>)>,
    commands: StdMutex<Vec<RunCommand>>,
    collect_hold: Option<Duration>,
}

#[async_trait::async_trait]
impl RunCommandRunner for HarvestFakeRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        let program = command.program.clone();
        let destination = command.args.last().cloned().unwrap_or_default();
        self.commands.lock().unwrap().push(command);
        match program.as_str() {
            "ssh" => {
                if let Some(hold) = self.collect_hold {
                    tokio::time::sleep(hold).await;
                }
                Ok(RunCommandOutput {
                    exit_code: 0,
                    stdout: self.manifest.clone(),
                    stderr: String::new(),
                })
            }
            "scp" => {
                let root = PathBuf::from(destination).join("harvest");
                for (relative, bytes) in &self.files {
                    let path = root.join(relative);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, bytes).unwrap();
                }
                Ok(RunCommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
            other => Err(format!("unexpected program {other}")),
        }
    }
}

fn harvest_specs() -> Vec<crate::harvest::OutputSpec> {
    vec![
        crate::harvest::OutputSpec {
            glob: "results/*.tsv".into(),
            kind: "table".into(),
            residency: crate::harvest::OutputResidency::Auto,
            logical_key: None,
            max_file_mb: Some(1),
            max_total_mb: None,
            bundle: false,
        },
        crate::harvest::OutputSpec {
            glob: "parts/*".into(),
            kind: "archive".into(),
            residency: crate::harvest::OutputResidency::Auto,
            logical_key: None,
            max_file_mb: None,
            max_total_mb: None,
            bundle: true,
        },
        crate::harvest::OutputSpec {
            glob: "big/*.bam".into(),
            kind: "data".into(),
            residency: crate::harvest::OutputResidency::Remote,
            logical_key: None,
            max_file_mb: None,
            max_total_mb: None,
            bundle: false,
        },
    ]
}

#[tokio::test]
async fn ssh_harvest_downloads_verifies_and_registers_selected_outputs() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-h").await;
    let table = b"a\tb\n1\t2\n".to_vec();
    let archive = b"fake-targz-bytes".to_vec();
    let remote_path = "/home/alice/.wisp-science/artifacts/run-h/big/x.bam";
    let manifest = format!(
        "__WISP_HARVEST__:file:0:{}:{}:results/out.tsv\n\
         __WISP_HARVEST__:bundle:1:{}:{}:132481:987654:bundle_1.tar.gz\n\
         __WISP_HARVEST__:remote:2:12345:{}:{}\n\
         __WISP_HARVEST_DONE__\n",
        table.len(),
        sha256_hex_of(&table),
        archive.len(),
        sha256_hex_of(&archive),
        "ab".repeat(32),
        remote_path,
    );
    let runner = HarvestFakeRunner {
        manifest,
        files: vec![
            ("files/results/out.tsv".into(), table.clone()),
            ("bundles/bundle_1.tar.gz".into(), archive.clone()),
        ],
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote("run-h", &tmp, harvest_specs());

    let harvested = harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, false)
        .await
        .unwrap();

    assert_eq!(harvested.len(), 3);
    let landing = tmp.join("remote/gpu/run-h");
    assert_eq!(
        std::fs::read(landing.join("results/out.tsv")).unwrap(),
        table
    );
    assert_eq!(
        std::fs::read(landing.join("bundles/bundle_1.tar.gz")).unwrap(),
        archive
    );
    let run = store.get_run("run-h").await.unwrap().unwrap();
    assert!(run.harvested_at.is_some());
    let outputs = store.list_run_outputs("run-h").await.unwrap();
    let mut keys: Vec<_> = outputs
        .iter()
        .map(|output| output.logical_output_key.clone())
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "bundle:parts/*".to_string(),
            "path:big/x.bam".to_string(),
            "path:results/out.tsv".to_string(),
        ]
    );
    let remote_output = outputs
        .iter()
        .find(|output| output.logical_output_key == "path:big/x.bam")
        .unwrap();
    let version = store
        .get_artifact_version(&remote_output.artifact_version_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(version.storage_path, format!("ssh://gpu{remote_path}"));
    assert_eq!(version.checksum.as_deref(), Some("ab".repeat(32).as_str()));
    assert_eq!(
        version.materialization,
        wisp_store::ArtifactMaterialization::External
    );
    let staged = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].source, "harvest_persist");
    assert_eq!(staged[0].remote_path, remote_path);
    assert_eq!(staged[0].run_id.as_deref(), Some("run-h"));
    let listed = crate::run_context::remote_files::list_remote_files(&store, "p", "ssh:gpu")
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].state,
        crate::run_context::remote_files::RemoteFileState::Active
    );
    let transfers: Vec<_> = store
        .list_runs_by_project("p")
        .await
        .unwrap()
        .into_iter()
        .filter(|run| run.kind == "file_transfer")
        .collect();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].status, wisp_store::RunStatus::Succeeded);
    assert_eq!(transfers[0].command.as_deref(), Some("harvest run-h"));
    // The collect script only ran once and validated the workdir token.
    {
        let commands = runner.commands.lock().unwrap();
        let collect = commands.iter().find(|c| c.program == "ssh").unwrap();
        let payload = collect.stdin.as_deref().unwrap();
        assert!(payload.contains("harvest-token"));
        assert!(payload.contains("tar -czf"));
        // Default remote data root derives from the project name ("proj").
        assert!(payload.contains("persist=\"$HOME/wisp/proj/data/artifacts/run-h\""));
        assert_eq!(
            collect.args.last().map(String::as_str),
            Some("sh -s --"),
            "collect must use a dedicated SSH session"
        );
        assert!(
            crate::ssh_master::eligible_payload(
                &collect.program,
                &collect.args,
                collect.stdin.as_deref(),
            )
            .is_none(),
            "collect must not occupy the shared SSH master slot"
        );
    }

    // A retried harvest re-registers nothing: same versions, same lineage rows.
    let again = harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, false)
        .await
        .unwrap();
    assert_eq!(again.len(), 3);
    assert_eq!(store.list_run_outputs("run-h").await.unwrap().len(), 3);
    assert_eq!(
        store
            .list_remote_staging("p", "ssh:gpu", false)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        again
            .iter()
            .map(|artifact| artifact.artifact_version_id.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        harvested
            .iter()
            .map(|artifact| artifact.artifact_version_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_harvest_checksum_mismatch_registers_nothing() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_bad_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-bad").await;
    let table = b"a\tb\n1\t2\n".to_vec();
    let manifest = format!(
        "__WISP_HARVEST__:file:0:{}:{}:results/out.tsv\n__WISP_HARVEST_DONE__\n",
        table.len(),
        "cd".repeat(32),
    );
    let runner = HarvestFakeRunner {
        manifest,
        files: vec![("files/results/out.tsv".into(), table)],
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote(
        "run-bad",
        &tmp,
        vec![crate::harvest::OutputSpec {
            glob: "results/*.tsv".into(),
            kind: "table".into(),
            residency: crate::harvest::OutputResidency::Auto,
            logical_key: None,
            max_file_mb: Some(1),
            max_total_mb: None,
            bundle: false,
        }],
    );

    let error = harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, false)
        .await
        .unwrap_err();

    assert!(error.contains("checksum mismatch"), "{error}");
    let run = store.get_run("run-bad").await.unwrap().unwrap();
    assert!(run.harvested_at.is_none());
    assert!(store.list_run_outputs("run-bad").await.unwrap().is_empty());
    assert!(!tmp.join("remote/gpu/run-bad/results/out.tsv").exists());
    let transfers: Vec<_> = store
        .list_runs_by_project("p")
        .await
        .unwrap()
        .into_iter()
        .filter(|run| run.kind == "file_transfer")
        .collect();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].status, wisp_store::RunStatus::Failed);
    // The partial download directory does not leak.
    let landing = tmp.join("remote/gpu/run-bad");
    if landing.exists() {
        assert!(std::fs::read_dir(&landing).unwrap().next().is_none());
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_harvest_collect_error_reports_bundle_guidance() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_cap_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-cap").await;
    let runner = HarvestFakeRunner {
        manifest: "__WISP_HARVEST_ERROR__:output glob matched more than 500 files; set bundle:true or narrow the glob to final products\n".into(),
        files: Vec::new(),
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote(
        "run-cap",
        &tmp,
        vec![crate::harvest::OutputSpec {
            glob: "read_partitions/*".into(),
            kind: "data".into(),
            residency: crate::harvest::OutputResidency::Auto,
            logical_key: None,
            max_file_mb: None,
            max_total_mb: None,
            bundle: false,
        }],
    );

    let error = harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, false)
        .await
        .unwrap_err();

    assert!(error.contains("bundle:true"), "{error}");
    assert!(store
        .get_run("run-cap")
        .await
        .unwrap()
        .unwrap()
        .harvested_at
        .is_none());

    let _ = std::fs::remove_dir_all(&tmp);
}

fn remote_only_harvest_spec() -> Vec<crate::harvest::OutputSpec> {
    vec![crate::harvest::OutputSpec {
        glob: "big/*.bam".into(),
        kind: "data".into(),
        residency: crate::harvest::OutputResidency::Remote,
        logical_key: None,
        max_file_mb: None,
        max_total_mb: None,
        bundle: false,
    }]
}

fn remote_only_manifest(run_id: &str) -> String {
    format!(
        "__WISP_HARVEST__:remote:0:12345:{}:/home/alice/.wisp-science/artifacts/{run_id}/big/x.bam\n\
         __WISP_HARVEST_DONE__\n",
        "ab".repeat(32),
    )
}

async fn seed_active_harvest_run(
    tmp: &Path,
    run_id: &str,
    owner: &str,
    lease_secs: i64,
) -> wisp_store::Store {
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new(run_id, "p", "ssh:gpu", "Remote", "ssh_direct");
    run.frame_id = Some("f".into());
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(run_id, wisp_store::RunStatus::Running, owner, lease_secs)
        .await
        .unwrap());
    store
}

/// Models the shared SSH master slot: non-dedicated RPCs serialize, dedicated
/// collect does not take the lock so poll/cancel can proceed concurrently.
struct SlotAwareHarvestRunner {
    manifest: String,
    slot: tokio::sync::Mutex<()>,
    collect_started: Arc<tokio::sync::Semaphore>,
    collect_release: Arc<tokio::sync::Semaphore>,
    commands: StdMutex<Vec<RunCommand>>,
}

#[async_trait::async_trait]
impl RunCommandRunner for SlotAwareHarvestRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        let uses_master = crate::ssh_master::eligible_payload(
            &command.program,
            &command.args,
            command.stdin.as_deref(),
        )
        .is_some();
        let script = command.script.clone();
        self.commands.lock().unwrap().push(command);
        if script.contains("collect SSH") {
            assert!(
                !uses_master,
                "collect must bypass the shared SSH master slot"
            );
            self.collect_started.add_permits(1);
            let _permit = self.collect_release.acquire().await.unwrap();
            return Ok(RunCommandOutput {
                exit_code: 0,
                stdout: self.manifest.clone(),
                stderr: String::new(),
            });
        }
        let _slot = self
            .slot
            .try_lock()
            .map_err(|_| "shared SSH master slot is occupied".to_string())?;
        if script.contains("poll") {
            return ok_output(&poll_response("running", "still going", ""));
        }
        if script.contains("cancel") {
            return ok_output("__WISP_CANCEL__:cancelled\n");
        }
        Err(format!("unexpected command: {script}"))
    }
}

struct RefuseCollectRunner {
    commands: StdMutex<Vec<RunCommand>>,
}

#[async_trait::async_trait]
impl RunCommandRunner for RefuseCollectRunner {
    async fn run(
        &self,
        command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        self.commands.lock().unwrap().push(command);
        Err("should not collect".into())
    }
}

#[tokio::test]
async fn ssh_harvest_collect_renews_parent_lease_past_expiry() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_lease_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    // `lifecycle_lease_until` is a unix-second timestamp. A 1s lease can
    // expire in 1ms if activation lands at the end of a second, so harvest
    // startup on a loaded CI runner loses it before the renewer runs.
    // Two seconds always covers startup; collect is held longer so the
    // original lease would expire without renewal.
    let store = seed_active_harvest_run(&tmp, "run-lease", "test-owner", 2).await;
    let runner = HarvestFakeRunner {
        manifest: remote_only_manifest("run-lease"),
        files: Vec::new(),
        commands: StdMutex::new(Vec::new()),
        collect_hold: Some(Duration::from_millis(2500)),
    };
    let remote = harvest_test_remote("run-lease", &tmp, remote_only_harvest_spec());

    harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, true)
        .await
        .unwrap();

    assert!(store
        .finish_active_run_owned(
            "run-lease",
            "test-owner",
            wisp_store::RunStatus::Succeeded,
            Some(0),
        )
        .await
        .unwrap());
    let collect = runner
        .commands
        .lock()
        .unwrap()
        .iter()
        .find(|command| command.program == "ssh")
        .cloned()
        .unwrap();
    assert_eq!(collect.args.last().map(String::as_str), Some("sh -s --"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_harvest_collect_renew_failure_aborts() {
    let tmp = std::env::temp_dir().join(format!(
        "wisp_ssh_harvest_renew_fail_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_active_harvest_run(&tmp, "run-steal", "test-owner", 30).await;
    let runner = HarvestFakeRunner {
        manifest: remote_only_manifest("run-steal"),
        files: Vec::new(),
        commands: StdMutex::new(Vec::new()),
        collect_hold: Some(Duration::from_millis(80)),
    };
    let remote = harvest_test_remote("run-steal", &tmp, remote_only_harvest_spec());

    let error = harvest_remote::harvest_ssh_run(&store, &runner, "other-owner", &remote, true)
        .await
        .unwrap_err();

    assert!(error.contains("lease was lost"), "{error}");
    assert!(store
        .get_run("run-steal")
        .await
        .unwrap()
        .unwrap()
        .harvested_at
        .is_none());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_harvest_collect_does_not_block_same_host_poll_or_cancel() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_slot_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-slot").await;
    let collect_started = Arc::new(tokio::sync::Semaphore::new(0));
    let collect_release = Arc::new(tokio::sync::Semaphore::new(0));
    let runner = Arc::new(SlotAwareHarvestRunner {
        manifest: remote_only_manifest("run-slot"),
        slot: tokio::sync::Mutex::new(()),
        collect_started: collect_started.clone(),
        collect_release: collect_release.clone(),
        commands: StdMutex::new(Vec::new()),
    });
    let remote = harvest_test_remote("run-slot", &tmp, remote_only_harvest_spec());
    let harvest_remote_run = remote.clone();
    let harvest = tokio::spawn({
        let store = store.clone();
        let runner = runner.clone();
        async move {
            harvest_remote::harvest_ssh_run(
                &store,
                runner.as_ref(),
                "test-owner",
                &harvest_remote_run,
                false,
            )
            .await
        }
    });

    let _started = tokio::time::timeout(Duration::from_secs(2), collect_started.acquire())
        .await
        .expect("collect never started")
        .expect("collect start permit");
    let poll = tokio::time::timeout(
        Duration::from_secs(1),
        poll_remote(runner.as_ref(), &remote.handle),
    )
    .await
    .expect("poll blocked by harvest collect")
    .unwrap();
    assert_eq!(poll.state, RemotePollState::Running);
    let cancel = tokio::time::timeout(
        Duration::from_secs(1),
        cancel_remote(runner.as_ref(), &remote.handle),
    )
    .await
    .expect("cancel blocked by harvest collect")
    .unwrap();
    assert_eq!(cancel, RemoteCancel::Cancelled);

    collect_release.add_permits(1);
    harvest.await.unwrap().unwrap();

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn finish_remote_run_errors_when_lease_is_lost() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_finish_lease_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_active_harvest_run(&tmp, "run-fin", "owner-a", 30).await;
    let runner = RefuseCollectRunner {
        commands: StdMutex::new(Vec::new()),
    };
    let remote = harvest_test_remote("run-fin", &tmp, Vec::new());

    let error = finish_remote_run(
        &store,
        &runner,
        "owner-b",
        &remote,
        wisp_store::RunStatus::Succeeded,
        Some(0),
    )
    .await
    .unwrap_err();

    assert!(error.contains("lease was lost"), "{error}");
    assert_eq!(
        store.get_run("run-fin").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Running
    );
    assert!(runner.commands.lock().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn auto_harvest_skips_collect_when_already_harvested() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_harvest_skip_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_active_harvest_run(&tmp, "run-skip", "test-owner", 30).await;
    let runner = HarvestFakeRunner {
        manifest: remote_only_manifest("run-skip"),
        files: Vec::new(),
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote("run-skip", &tmp, remote_only_harvest_spec());

    harvest_finished_remote(&store, &runner, "test-owner", &remote, "f")
        .await
        .unwrap();
    assert_eq!(
        runner
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.program == "ssh")
            .count(),
        1
    );

    harvest_finished_remote(&store, &runner, "test-owner", &remote, "f")
        .await
        .unwrap();
    assert_eq!(
        runner
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.program == "ssh")
            .count(),
        1,
        "second auto harvest must not collect again"
    );

    let refuse = RefuseCollectRunner {
        commands: StdMutex::new(Vec::new()),
    };
    finish_remote_run(
        &store,
        &refuse,
        "test-owner",
        &remote,
        wisp_store::RunStatus::Succeeded,
        Some(0),
    )
    .await
    .unwrap();
    assert!(
        refuse.commands.lock().unwrap().is_empty(),
        "finish after harvested_at must skip collect"
    );
    assert_eq!(
        store.get_run("run-skip").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Succeeded
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_submit_rejects_shell_unsafe_output_globs() {
    let tmp = std::env::temp_dir().join(format!("wisp_ssh_glob_guard_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
    }));

    let error = submit_run_with_runner(
        &store,
        "p",
        Some("f"),
        SubmitRunRequest {
            context_id: "ssh:gpu".into(),
            command: "make outputs".into(),
            title: None,
            timeout_secs: Some(60),
            input_paths: None,
            output_specs: Some(vec![crate::harvest::OutputSpec {
                glob: "results/$(rm -rf ~)".into(),
                kind: "table".into(),
                residency: crate::harvest::OutputResidency::Auto,
                logical_key: None,
                max_file_mb: None,
                max_total_mb: None,
                bundle: false,
            }]),
        },
        &runner,
        Some(tmp.clone()),
    )
    .await
    .unwrap_err();

    assert!(error.contains("unsupported character"), "{error}");

    let _ = std::fs::remove_dir_all(&tmp);
}

// --- storage preferences ---------------------------------------------------

#[tokio::test]
async fn ssh_run_workdir_honors_stored_workdir_root_pref() {
    let tmp = std::env::temp_dir().join(format!("wisp_workdir_pref_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    store
        .upsert_context_storage_prefs(&wisp_store::ContextStoragePrefs {
            project_id: "p".into(),
            context_id: "ssh:gpu".into(),
            remote_data_root: "~/wisp/proj/data".into(),
            remote_workdir_root: "scratch/wisp-runs".into(),
            local_results_dir: "remote/gpu".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
    let runner = FakeRunRunner::new(Ok(RunCommandOutput {
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
    }));

    let result = submit_run_with_runner(
        &store,
        "p",
        Some("f"),
        SubmitRunRequest {
            context_id: "ssh:gpu".into(),
            command: "echo remote".into(),
            title: None,
            timeout_secs: Some(60),
            input_paths: None,
            output_specs: None,
        },
        &runner,
        Some(tmp.clone()),
    )
    .await
    .unwrap();

    let run = store.get_run(&result.run_id).await.unwrap().unwrap();
    assert_eq!(
        run.remote_workdir.as_deref(),
        Some(format!("~/scratch/wisp-runs/{}", result.run_id).as_str())
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn ssh_harvest_uses_stored_local_results_dir_and_data_root() {
    let tmp = std::env::temp_dir().join(format!("wisp_harvest_prefs_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-p").await;
    store
        .upsert_context_storage_prefs(&wisp_store::ContextStoragePrefs {
            project_id: "p".into(),
            context_id: "ssh:gpu".into(),
            remote_data_root: "/scratch/proj".into(),
            remote_workdir_root: ".wisp-science/runs".into(),
            local_results_dir: "results/from-gpu".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
    let table = b"a\tb\n1\t2\n".to_vec();
    let manifest = format!(
        "__WISP_HARVEST__:file:0:{}:{}:results/out.tsv\n__WISP_HARVEST_DONE__\n",
        table.len(),
        sha256_hex_of(&table),
    );
    let runner = HarvestFakeRunner {
        manifest,
        files: vec![("files/results/out.tsv".into(), table.clone())],
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote(
        "run-p",
        &tmp,
        vec![crate::harvest::OutputSpec {
            glob: "results/*.tsv".into(),
            kind: "table".into(),
            residency: crate::harvest::OutputResidency::Auto,
            logical_key: None,
            max_file_mb: Some(1),
            max_total_mb: None,
            bundle: false,
        }],
    );

    harvest_remote::harvest_ssh_run(&store, &runner, "test-owner", &remote, false)
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(tmp.join("results/from-gpu/run-p/results/out.tsv")).unwrap(),
        table
    );
    let commands = runner.commands.lock().unwrap();
    let collect = commands.iter().find(|c| c.program == "ssh").unwrap();
    assert!(collect
        .stdin
        .as_deref()
        .unwrap()
        .contains("persist=\"/scratch/proj/artifacts/run-p\""));

    let _ = std::fs::remove_dir_all(&tmp);
}

// --- run workspace cleanup ---------------------------------------------------

/// The log-pull RPC that precedes every first workspace deletion: the workdir
/// is already gone, so there is nothing to save.
fn log_pull_absent() -> Result<RunCommandOutput, String> {
    ok_output("__WISP_LOGPULL__:absent\n__WISP_LOGPULL__:done\n")
}

async fn seed_cleanup_run(
    store: &wisp_store::Store,
    run_id: &str,
    status: wisp_store::RunStatus,
    specs_json: &str,
    harvested: bool,
) {
    let mut run = wisp_store::RunRecord::new(run_id, "p", "ssh:gpu", "Remote", "ssh_direct");
    run.frame_id = Some("f".into());
    run.status = status;
    run.command = Some("make outputs".into());
    run.output_specs_json = specs_json.into();
    let connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&harvest_test_context()).unwrap();
    let handle = RemoteRunHandle::SshDirect {
        connection,
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "cleanup-token".into(),
        inputs_staged: true,
        pgid: Some(4242),
        start_time: Some(99),
    };
    run.remote_workdir = Some(handle.display_workdir());
    run.remote_handle_json = Some(serde_json::to_string(&handle).unwrap());
    store.create_run(&run).await.unwrap();
    if harvested {
        store.mark_run_harvested(run_id).await.unwrap();
    }
}

#[tokio::test]
async fn cleanup_requires_terminal_state_and_harvested_outputs() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_guard_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let specs = serde_json::json!([{
        "glob": "results/*.tsv", "kind": "table", "residency": "auto"
    }])
    .to_string();
    seed_cleanup_run(
        &store,
        "run-active",
        wisp_store::RunStatus::Running,
        "[]",
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-unharvested",
        wisp_store::RunStatus::Succeeded,
        &specs,
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-failed",
        wisp_store::RunStatus::Failed,
        &specs,
        false,
    )
    .await;
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());

    let error = manager
        .cleanup_run_workspace(&store, "run-active", false)
        .await
        .unwrap_err();
    assert!(error.contains("still running"), "{error}");

    let error = manager
        .cleanup_run_workspace(&store, "run-unharvested", false)
        .await
        .unwrap_err();
    assert!(error.contains("harvest_run"), "{error}");
    assert_eq!(runner.commands.lock().unwrap().len(), 0);

    // Failed runs have nothing to harvest: cleanup proceeds without force.
    let cleaned = manager
        .cleanup_run_workspace(&store, "run-failed", false)
        .await
        .unwrap();
    assert!(cleaned.cleaned_at.is_some());

    // Explicit user confirmation (force) accepts the data loss.
    let cleaned = manager
        .cleanup_run_workspace(&store, "run-unharvested", true)
        .await
        .unwrap();
    assert!(cleaned.cleaned_at.is_some());
    {
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 4);
        // Logs are pulled before the workdir is deleted.
        let logs_payload = commands[0].stdin.as_deref().unwrap();
        assert!(logs_payload.contains("stdout"));
        assert!(!logs_payload.contains("rm -rf"));
        let payload = commands[1].stdin.as_deref().unwrap();
        assert!(payload.contains("workdir=\"$HOME/.wisp-science/runs/run-failed\""));
        assert!(payload.contains("rm -rf \"$workdir\""));
        assert!(payload.contains("cleanup-token"));
    }

    // Idempotent: a second cleanup issues no further remote commands.
    let again = manager
        .cleanup_run_workspace(&store, "run-failed", false)
        .await
        .unwrap();
    assert!(again.cleaned_at.is_some());
    assert_eq!(runner.commands.lock().unwrap().len(), 4);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn cleanup_rejects_foreign_workdirs_and_records_failures() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_paths_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();

    // Malicious/foreign workdir strings are refused before any command runs.
    for (run_id, workdir) in [
        ("run-root", "/"),
        ("run-home", "runs/../run-home"),
        ("run-other", ".wisp-science/runs/another-run"),
    ] {
        let mut run = wisp_store::RunRecord::new(run_id, "p", "ssh:gpu", "Remote", "ssh_direct");
        run.frame_id = Some("f".into());
        run.status = wisp_store::RunStatus::Failed;
        let connection =
            crate::ssh_hosts::SshConnection::from_execution_context(&harvest_test_context())
                .unwrap();
        run.remote_handle_json = Some(
            serde_json::to_string(&RemoteRunHandle::SshDirect {
                connection,
                workdir: workdir.into(),
                token: "tok".into(),
                inputs_staged: true,
                pgid: Some(1),
                start_time: Some(1),
            })
            .unwrap(),
        );
        store.create_run(&run).await.unwrap();
        let runner = Arc::new(ScriptedRunRunner::new(vec![]));
        let manager = RunManager::with_runner(runner.clone());
        let error = manager
            .cleanup_run_workspace(&store, run_id, false)
            .await
            .unwrap_err();
        assert!(
            error.contains("workdir"),
            "workdir {workdir} should be rejected: {error}"
        );
        assert!(runner.commands.lock().unwrap().is_empty());
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert!(run.cleaned_at.is_none());
        assert!(run
            .cleanup_error
            .as_deref()
            .unwrap_or_default()
            .contains("workdir"));
    }

    // A failed remote deletion records the error and stays retryable.
    seed_cleanup_run(
        &store,
        "run-flaky",
        wisp_store::RunStatus::Failed,
        "[]",
        false,
    )
    .await;
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        Err("ssh: connect to host gpu port 22: Connection refused".into()),
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());
    let error = manager
        .cleanup_run_workspace(&store, "run-flaky", false)
        .await
        .unwrap_err();
    assert!(error.contains("Connection refused"), "{error}");
    let run = store.get_run("run-flaky").await.unwrap().unwrap();
    assert!(run.cleaned_at.is_none());
    assert!(run
        .cleanup_error
        .as_deref()
        .unwrap()
        .contains("Connection refused"));
    let cleaned = manager
        .cleanup_run_workspace(&store, "run-flaky", false)
        .await
        .unwrap();
    assert!(cleaned.cleaned_at.is_some());
    assert!(cleaned.cleanup_error.is_none());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn cleanup_refuses_while_an_external_reference_points_into_the_workdir() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_extref_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    seed_cleanup_run(
        &store,
        "run-ref",
        wisp_store::RunStatus::Succeeded,
        "[]",
        true,
    )
    .await;
    // Simulate a (buggy or legacy) External reference into the workdir itself.
    let version_id = store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: wisp_store::logical_artifact_id("p", "path:stuck.bam"),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "stuck.bam".into(),
            content_type: "data".into(),
            storage_path: "ssh://gpu/home/alice/.wisp-science/runs/run-ref/inputs/stuck.bam".into(),
            logical_key: Some("path:stuck.bam".into()),
            size_bytes: None,
            checksum: None,
            producing_run_id: Some("run-ref".into()),
            env_snapshot_hash: None,
            materialization: wisp_store::ArtifactMaterialization::External,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    store
        .save_run_output(&wisp_store::RunOutput {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: "run-ref".into(),
            artifact_version_id: version_id,
            role: "data".into(),
            logical_output_key: "path:stuck.bam".into(),
            source_path: "stuck.bam".into(),
            created_at: chrono::Utc::now().timestamp(),
        })
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![]));
    let manager = RunManager::with_runner(runner.clone());
    let error = manager
        .cleanup_run_workspace(&store, "run-ref", false)
        .await
        .unwrap_err();
    assert!(error.contains("still points into"), "{error}");
    assert!(runner.commands.lock().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

// --- remote staging ledger ---------------------------------------------------

#[tokio::test]
async fn ssh_input_staging_ledgers_uploaded_files() {
    let tmp = std::env::temp_dir().join(format!("wisp_staging_ledger_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("input.fasta"), b">seq\nACGT\n").unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_PREPARED__\n"),
        ok_output(""),
        Err("temporary SSH disconnect".into()),
    ]));
    runner
        .synthesize_launch_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let manager = RunManager::with_runner(runner.clone());

    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            Some("f".into()),
            SubmitRunRequest {
                context_id: "ssh:gpu".into(),
                command: "wc -l input.fasta".into(),
                title: None,
                timeout_secs: Some(60),
                input_paths: Some(vec!["input.fasta".into()]),
                output_specs: None,
            },
            Some(tmp.clone()),
        )
        .await
        .unwrap();
    wait_for_terminal(&store, &submitted.run_id).await;

    let entries = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "run_input");
    assert_eq!(
        entries[0].run_id.as_deref(),
        Some(submitted.run_id.as_str())
    );
    assert_eq!(
        entries[0].remote_path,
        format!(
            "~/.wisp-science/runs/{}/inputs/input.fasta",
            submitted.run_id
        )
    );
    assert_eq!(entries[0].size_bytes, Some(10));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn remote_files_classify_and_remove_only_ledgered_paths() {
    let tmp = std::env::temp_dir().join(format!("wisp_remote_files_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let context = harvest_test_context();
    store.upsert_execution_context(&context).await.unwrap();
    // Active: staged input of a succeeded-but-not-cleaned run.
    seed_cleanup_run(
        &store,
        "run-live",
        wisp_store::RunStatus::Succeeded,
        "[]",
        true,
    )
    .await;
    let mut active = wisp_store::RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        Some("run-live".into()),
        "~/.wisp-science/runs/run-live/inputs/input.fasta",
        "run_input",
    );
    active.created_at = 100;
    store.record_remote_staging(&active).await.unwrap();
    // Replaced: an older upload to the same path as a newer one.
    let mut old_upload = wisp_store::RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        None,
        "~/wisp/proj/data/matrix.tsv",
        "transfer",
    );
    old_upload.created_at = 200;
    store.record_remote_staging(&old_upload).await.unwrap();
    let mut new_upload = wisp_store::RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        None,
        "~/wisp/proj/data/matrix.tsv",
        "transfer",
    );
    new_upload.created_at = 300;
    store.record_remote_staging(&new_upload).await.unwrap();

    let files = remote_files::list_remote_files(&store, "p", "ssh:gpu")
        .await
        .unwrap();
    let state_of = |id: &str| {
        files
            .iter()
            .find(|file| file.id == id)
            .map(|file| file.state)
            .unwrap()
    };
    assert_eq!(state_of(&active.id), remote_files::RemoteFileState::Active);
    assert_eq!(
        state_of(&old_upload.id),
        remote_files::RemoteFileState::Replaced
    );
    assert_eq!(
        state_of(&new_upload.id),
        remote_files::RemoteFileState::Active
    );

    // Active entries require force; unledgered ids are refused outright.
    let runner = Arc::new(ScriptedRunRunner::new(vec![]));
    let manager = RunManager::with_runner(runner.clone());
    let error = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &[active.id.clone()],
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("still referenced"), "{error}");
    let error = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &["not-ledgered".into()],
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("not ledgered"), "{error}");
    assert!(runner.commands.lock().unwrap().is_empty());

    // Replaced is ledger-only: deleting it must not rm the current file.
    let runner = Arc::new(ScriptedRunRunner::new(vec![]));
    let manager = RunManager::with_runner(runner.clone());
    let removed = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &[old_upload.id.clone()],
        false,
    )
    .await
    .unwrap();
    assert_eq!(removed, 1);
    assert!(runner.commands.lock().unwrap().is_empty());
    let remaining = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|e| e.id == new_upload.id));
    assert!(remaining.iter().any(|e| e.id == active.id));

    // Current upload is Active and requires force. After force, one rm.
    let error = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &[new_upload.id.clone()],
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("still referenced"), "{error}");
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(&format!(
        "__WISP_RM__:{}\n",
        new_upload.id
    ))]));
    let manager = RunManager::with_runner(runner.clone());
    let removed = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &[new_upload.id.clone()],
        true,
    )
    .await
    .unwrap();
    assert_eq!(removed, 1);
    {
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        let payload = commands[0].stdin.as_deref().unwrap();
        assert_eq!(payload.matches("rm -rf \"$path\"").count(), 1);
        assert!(payload.contains("'wisp/proj/data/matrix.tsv'"));
    }
    let remaining = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, active.id);

    // Force removes the active entry with explicit user confirmation.
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(&format!(
        "__WISP_RM__:{}\n",
        active.id
    ))]));
    let manager = RunManager::with_runner(runner.clone());
    let removed = remote_files::remove_remote_files(
        &store,
        manager.runner_ref(),
        "p",
        &context,
        &[active.id.clone()],
        true,
    )
    .await
    .unwrap();
    assert_eq!(removed, 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn discarding_a_server_marks_external_artifacts_and_blocks_fetch() {
    let tmp = std::env::temp_dir().join(format!("wisp_discard_src_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let uri = "ssh://gpu/scratch/proj/artifacts/r1/big.bam";
    store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: wisp_store::logical_artifact_id("p", "path:big.bam"),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "big.bam".into(),
            content_type: "data".into(),
            storage_path: uri.into(),
            logical_key: Some("path:big.bam".into()),
            size_bytes: Some(12),
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: wisp_store::ArtifactMaterialization::External,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();
    let mut persist = wisp_store::RemoteStagingEntry::new(
        "p",
        "ssh:gpu",
        Some("r1".into()),
        "/scratch/proj/artifacts/r1/big.bam",
        "harvest_persist",
    );
    persist.created_at = 10;
    store.record_remote_staging(&persist).await.unwrap();

    let files = remote_files::list_remote_files(&store, "p", "ssh:gpu")
        .await
        .unwrap();
    assert_eq!(files[0].state, remote_files::RemoteFileState::Active);
    remote_files::refuse_if_source_discarded(&store, uri)
        .await
        .unwrap();

    assert_eq!(
        remote_files::abandon_context_sources(&store, "gpu")
            .await
            .unwrap(),
        1
    );
    assert!(store.ssh_uri_source_discarded(uri).await.unwrap());
    assert!(store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap()
        .is_empty());
    let error = remote_files::refuse_if_source_discarded(&store, uri)
        .await
        .unwrap_err();
    assert!(error.starts_with("source_discarded:"), "{error}");
    let error = remote_files::refuse_if_context_path_discarded(
        &store,
        "ssh:gpu",
        "/scratch/proj/artifacts/r1/big.bam",
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("source_discarded:"), "{error}");

    let found = store
        .search_artifacts(Some("p"), "big", 8, None)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].source_discarded);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn orphan_file_sweep_reclaims_only_expired_unreferenced_entries() {
    let tmp = std::env::temp_dir().join(format!("wisp_orphan_gc_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp();
    let seed_entry = |run_id: Option<&str>, path: &str, source: &str, age_days: i64| {
        let mut entry = wisp_store::RemoteStagingEntry::new(
            "p",
            "ssh:gpu",
            run_id.map(Into::into),
            path,
            source,
        );
        entry.created_at = now - age_days * 86_400;
        entry
    };
    // Failed transfer (partial): orphan, reclaimed. Successful current upload:
    // Active, kept. Replaced: ledger-only. Persist with no live artifact: orphan.
    seed_cleanup_run(
        &store,
        "run-fail",
        wisp_store::RunStatus::Failed,
        "[]",
        false,
    )
    .await;
    let orphan_failed = seed_entry(
        Some("run-fail"),
        "~/wisp/proj/data/stale.bam",
        "transfer",
        40,
    );
    let persist_orphan = seed_entry(
        None,
        "/scratch/proj/artifacts/r1/gone.bam",
        "harvest_persist",
        40,
    );
    let kept_upload = seed_entry(None, "~/wisp/proj/data/fresh.bam", "transfer", 40);
    let replaced_old = seed_entry(None, "~/wisp/proj/data/matrix.tsv", "transfer", 45);
    let mut replacement = seed_entry(None, "~/wisp/proj/data/matrix.tsv", "transfer", 41);
    replacement.created_at += 1; // strictly newer than replaced_old
    seed_cleanup_run(
        &store,
        "run-live",
        wisp_store::RunStatus::Running,
        "[]",
        false,
    )
    .await;
    let active_old = seed_entry(
        Some("run-live"),
        "~/.wisp-science/runs/run-live/inputs/input.fasta",
        "run_input",
        40,
    );
    for entry in [
        &orphan_failed,
        &persist_orphan,
        &kept_upload,
        &replaced_old,
        &replacement,
        &active_old,
    ] {
        store.record_remote_staging(entry).await.unwrap();
    }

    // Sweep is opt-in: without the window nothing is inspected.
    let manager = RunManager::with_runner(Arc::new(ScriptedRunRunner::new(vec![])));
    assert_eq!(manager.orphan_file_sweep(&store).await.unwrap(), 0);

    store
        .set_project_run_retention("p", None, None, Some(30))
        .await
        .unwrap();
    // Failed partial + unreferenced persist are deleted. Replaced is closed
    // in-ledger only. Current successful upload and live run input stay.
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(&format!(
        "__WISP_RM__:{}\n__WISP_RM__:{}\n",
        orphan_failed.id, persist_orphan.id
    ))]));
    let manager = RunManager::with_runner(runner.clone());
    let removed = manager.orphan_file_sweep(&store).await.unwrap();
    assert_eq!(removed, 3);
    {
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        let payload = commands[0].stdin.as_deref().unwrap();
        assert!(payload.contains("'wisp/proj/data/stale.bam'"));
        assert!(payload.contains("gone.bam"));
        assert!(!payload.contains("fresh.bam"));
        assert!(!payload.contains("input.fasta"));
        assert!(!payload.contains("matrix.tsv"));
    }
    let mut remaining: Vec<String> = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    remaining.sort();
    let mut expected = vec![
        active_old.id.clone(),
        kept_upload.id.clone(),
        replacement.id.clone(),
    ];
    expected.sort();
    assert_eq!(remaining, expected);

    // Idempotent: nothing left that is due.
    let manager = RunManager::with_runner(Arc::new(ScriptedRunRunner::new(vec![])));
    assert_eq!(manager.orphan_file_sweep(&store).await.unwrap(), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn transfer_without_handle_fails_instead_of_lost_on_reconcile() {
    let tmp = std::env::temp_dir().join(format!("wisp_xfer_reclaim_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    let mut run = wisp_store::RunRecord::new("xfer-1", "p", "ssh:gpu", "Upload", "file_transfer");
    run.frame_id = Some("f".into());
    run.status = wisp_store::RunStatus::Running;
    store.create_run(&run).await.unwrap();

    let manager = RunManager::with_runner(Arc::new(ScriptedRunRunner::new(vec![])));
    let lost = manager.recover(&store).await.unwrap();
    assert_eq!(lost, 0);
    let run = store.get_run("xfer-1").await.unwrap().unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Failed);
    assert!(
        run.last_poll_error
            .as_deref()
            .is_some_and(|e| e.contains("no recoverable handle")),
        "{:?}",
        run.last_poll_error
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn disposal_report_counts_every_project_on_the_host() {
    let tmp = std::env::temp_dir().join(format!("wisp_disposal_all_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p1", "one", &tmp.to_string_lossy())
        .await
        .unwrap();
    store
        .create_project("p2", "two", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
    store.create_frame("f2", "p2", "OPERON", "m").await.unwrap();
    let context = harvest_test_context();
    store.upsert_execution_context(&context).await.unwrap();
    store
        .record_remote_staging(&wisp_store::RemoteStagingEntry::new(
            "p1",
            "ssh:gpu",
            None,
            "~/wisp/p1/data/a.bam",
            "transfer",
        ))
        .await
        .unwrap();
    store
        .record_remote_staging(&wisp_store::RemoteStagingEntry::new(
            "p2",
            "ssh:gpu",
            None,
            "~/wisp/p2/data/b.bam",
            "transfer",
        ))
        .await
        .unwrap();
    let uri = "ssh://gpu/scratch/p2/out.bam";
    store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: wisp_store::logical_artifact_id("p2", "path:out.bam"),
            project_id: "p2".into(),
            root_frame_id: "f2".into(),
            filename: "out.bam".into(),
            content_type: "data".into(),
            storage_path: uri.into(),
            logical_key: Some("path:out.bam".into()),
            size_bytes: Some(12),
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: wisp_store::ArtifactMaterialization::External,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .unwrap();

    let report = remote_files::context_disposal_report(&store, "p1", &context)
        .await
        .unwrap();
    assert_eq!(report.staged_files, 2);
    assert_eq!(report.external_references, 1);
    assert_eq!(report.sole_remote_copies, 1);
    assert_eq!(report.active_runs, 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn workspace_cleanup_marks_staged_inputs_removed() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_ledger_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    seed_cleanup_run(
        &store,
        "run-led",
        wisp_store::RunStatus::Failed,
        "[]",
        false,
    )
    .await;
    store
        .record_remote_staging(&wisp_store::RemoteStagingEntry::new(
            "p",
            "ssh:gpu",
            Some("run-led".into()),
            "~/.wisp-science/runs/run-led/inputs/input.fasta",
            "run_input",
        ))
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner);

    manager
        .cleanup_run_workspace(&store, "run-led", false)
        .await
        .unwrap();

    assert!(store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap()
        .is_empty());
    let all = store
        .list_remote_staging("p", "ssh:gpu", true)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].removed_at.is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn cleanup_pulls_full_logs_into_the_project_first() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_logs_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    seed_cleanup_run(
        &store,
        "run-logs",
        wisp_store::RunStatus::Failed,
        "[]",
        false,
    )
    .await;
    use base64::Engine as _;
    let stdout_b64 = base64::engine::general_purpose::STANDARD.encode(b"full stdout\n");
    // stderr reports a larger on-server size than the pulled bytes: truncated.
    let stderr_b64 = base64::engine::general_purpose::STANDARD.encode(b"tail of stderr\n");
    let log_pull = format!(
        "__WISP_LOGPULL__:stdout:12\n{stdout_b64}\n__WISP_LOGPULL__:end\n\
         __WISP_LOGPULL__:stderr:9999\n{stderr_b64}\n__WISP_LOGPULL__:end\n\
         __WISP_LOGPULL__:done\n"
    );
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output(&log_pull),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());

    let cleaned = manager
        .cleanup_run_workspace(&store, "run-logs", false)
        .await
        .unwrap();
    assert!(cleaned.cleaned_at.is_some());
    assert_eq!(cleaned.logs_path.as_deref(), Some("runs/run-logs"));
    assert_eq!(
        std::fs::read(tmp.join("runs/run-logs/stdout.log")).unwrap(),
        b"full stdout\n"
    );
    let stderr =
        String::from_utf8(std::fs::read(tmp.join("runs/run-logs/stderr.log")).unwrap()).unwrap();
    assert!(stderr.starts_with("[wisp] log truncated: showing last 15 of 9999 bytes\n"));
    assert!(stderr.ends_with("tail of stderr\n"));
    // The log pull happened before the deletion RPC.
    {
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert!(!commands[0].stdin.as_deref().unwrap().contains("rm -rf"));
        assert!(commands[1].stdin.as_deref().unwrap().contains("rm -rf"));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn cleanup_aborts_when_log_pull_fails_and_falls_back_without_encoder() {
    let tmp = std::env::temp_dir().join(format!("wisp_cleanup_logfail_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let mut run = wisp_store::RunRecord::new("run-nolog", "p", "ssh:gpu", "Remote", "ssh_direct");
    run.frame_id = Some("f".into());
    run.status = wisp_store::RunStatus::Failed;
    run.command = Some("make outputs".into());
    run.stdout_tail = Some("tail out".into());
    let connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&harvest_test_context()).unwrap();
    let handle = RemoteRunHandle::SshDirect {
        connection,
        workdir: ".wisp-science/runs/run-nolog".into(),
        token: "cleanup-token".into(),
        inputs_staged: true,
        pgid: Some(4242),
        start_time: Some(99),
    };
    run.remote_handle_json = Some(serde_json::to_string(&handle).unwrap());
    store.create_run(&run).await.unwrap();

    // A log pull that cannot confirm completion aborts cleanup: the workdir
    // (and its logs) must survive for a retry.
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("garbled"),
        // Retry: no base64 on the server → persisted tails become the copy.
        ok_output("__WISP_LOGPULL__:noencoder\n__WISP_LOGPULL__:done\n"),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());
    let error = manager
        .cleanup_run_workspace(&store, "run-nolog", false)
        .await
        .unwrap_err();
    assert!(error.contains("saving run logs"), "{error}");
    let run = store.get_run("run-nolog").await.unwrap().unwrap();
    assert!(run.cleaned_at.is_none());
    assert!(run.cleanup_error.as_deref().unwrap().contains("logs"));

    let cleaned = manager
        .cleanup_run_workspace(&store, "run-nolog", false)
        .await
        .unwrap();
    assert!(cleaned.cleaned_at.is_some());
    assert_eq!(cleaned.logs_path.as_deref(), Some("runs/run-nolog"));
    assert_eq!(
        std::fs::read(tmp.join("runs/run-nolog/stdout.log")).unwrap(),
        b"tail out"
    );
    assert!(!tmp.join("runs/run-nolog/stderr.log").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn transfer_upload_ledgers_its_destination() {
    let tmp = std::env::temp_dir().join(format!("wisp_transfer_ledger_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    store
        .set_session_execution_context_enabled("f", "ssh:gpu", true)
        .await
        .unwrap();
    let source = tmp.join("matrix.tsv");
    std::fs::write(&source, b"a\tb\n").unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output(""), // destination pre-check
        ok_output(""), // scp upload
    ]));
    let manager = RunManager::with_runner(runner);
    let context = store
        .get_execution_context("ssh:gpu")
        .await
        .unwrap()
        .unwrap();

    let response = manager
        .submit_local_upload_to_ssh(
            store.clone(),
            "p",
            Some("f"),
            &source,
            &context,
            "~/wisp/proj/data/matrix.tsv",
            transfer::TransferTransport::Scp,
            false,
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    wait_for_terminal(&store, &response.run_id).await;

    let entries = store
        .list_remote_staging("p", "ssh:gpu", false)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "transfer");
    assert_eq!(entries[0].remote_path, "~/wisp/proj/data/matrix.tsv");
    assert_eq!(entries[0].run_id.as_deref(), Some(response.run_id.as_str()));

    let _ = std::fs::remove_dir_all(&tmp);
}

// --- run review: selective download / browse / delete -----------------------

#[tokio::test]
async fn download_run_files_registers_one_row_per_selection() {
    let tmp = std::env::temp_dir().join(format!("wisp_review_dl_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = seed_harvest_run(&tmp, "run-sel").await;
    // Give the seeded run a confirmed handle for terminal_ssh_remote.
    let table = b"a\tb\n".to_vec();
    let archive = b"dir-targz".to_vec();
    let manifest = format!(
        "__WISP_HARVEST__:file:0:{}:{}:results/out.tsv\n\
         __WISP_HARVEST__:bundle:1:{}:{}:132481:987654:bundle_1.tar.gz\n\
         __WISP_HARVEST_DONE__\n",
        table.len(),
        sha256_hex_of(&table),
        archive.len(),
        sha256_hex_of(&archive),
    );
    let runner = HarvestFakeRunner {
        manifest,
        files: vec![
            ("files/results/out.tsv".into(), table.clone()),
            ("bundles/bundle_1.tar.gz".into(), archive.clone()),
        ],
        commands: StdMutex::new(Vec::new()),
        collect_hold: None,
    };
    let remote = harvest_test_remote("run-sel", &tmp, Vec::new());

    let harvested = harvest_remote::download_run_files(
        &store,
        &runner,
        "test-owner",
        &remote,
        &["results/out.tsv".into()],
        &["read_partitions".into()],
    )
    .await
    .unwrap();

    assert_eq!(harvested.len(), 2);
    let outputs = store.list_run_outputs("run-sel").await.unwrap();
    assert_eq!(outputs.len(), 2);
    let mut keys: Vec<_> = outputs
        .iter()
        .map(|output| output.logical_output_key.clone())
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "bundle:read_partitions".to_string(),
            "path:results/out.tsv".to_string(),
        ]
    );
    // Selection does not mark the run harvested — that stays spec-driven.
    assert!(store
        .get_run("run-sel")
        .await
        .unwrap()
        .unwrap()
        .harvested_at
        .is_none());
    // The collect script bundles the directory selection via find.
    {
        let commands = runner.commands.lock().unwrap();
        let collect = commands.iter().find(|c| c.program == "ssh").unwrap();
        let payload = collect.stdin.as_deref().unwrap();
        assert!(payload.contains("find read_partitions -type f"));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn run_review_browse_and_delete_require_a_terminal_ssh_run() {
    let tmp = std::env::temp_dir().join(format!("wisp_review_guard_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    seed_cleanup_run(
        &store,
        "run-open",
        wisp_store::RunStatus::Running,
        "[]",
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-done",
        wisp_store::RunStatus::Succeeded,
        "[]",
        false,
    )
    .await;
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output(
            "__WISP_LS__:file:1024::out.tsv\n__WISP_LS__:dir:2048:12:parts\n__WISP_LS_DONE__\n",
        ),
        ok_output("__WISP_RM_DONE__\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());

    let error = manager
        .list_run_workspace_files(&store, "run-open", "", "", 0, 100)
        .await
        .unwrap_err();
    assert!(error.contains("still running"), "{error}");

    let listing = manager
        .list_run_workspace_files(&store, "run-done", "", "", 0, 100)
        .await
        .unwrap();
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[1].kind, "dir");

    // Path traversal is rejected before any command runs.
    let error = manager
        .delete_run_files(&store, "run-done", &["../escape".into()])
        .await
        .unwrap_err();
    assert!(error.contains("workdir-relative"), "{error}");

    manager
        .delete_run_files(&store, "run-done", &["parts".into()])
        .await
        .unwrap();
    {
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        let payload = commands[1].stdin.as_deref().unwrap();
        assert!(payload.contains("rm -rf -- 'parts'"));
        assert!(payload.contains("cleanup-token"));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn review_prompt_reserved_for_unresolved_product_decisions() {
    let tmp = std::env::temp_dir().join(format!("wisp_review_prompt_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();

    let specs = r#"[{"glob":"results/*.tsv","kind":"table"}]"#;
    seed_cleanup_run(
        &store,
        "run-unharvested",
        wisp_store::RunStatus::Succeeded,
        specs,
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-harvested",
        wisp_store::RunStatus::Succeeded,
        specs,
        true,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-active",
        wisp_store::RunStatus::Running,
        "[]",
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-files",
        wisp_store::RunStatus::Succeeded,
        "[]",
        false,
    )
    .await;
    seed_cleanup_run(
        &store,
        "run-empty",
        wisp_store::RunStatus::Succeeded,
        "[]",
        false,
    )
    .await;

    let runner = Arc::new(ScriptedRunRunner::new(vec![
        ok_output("__WISP_LS__:file:1024::out.tsv\n__WISP_LS_DONE__\n"),
        ok_output("__WISP_LS_DONE__\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());

    // Declared outputs never harvested: results only exist on the server, so
    // the prompt is warranted — decided from the record alone, no SSH call.
    assert!(manager
        .should_prompt_run_review(&store, "run-unharvested")
        .await
        .unwrap());
    // Harvested products are already local; the leftover cleanup decision is
    // not worth an interruption.
    assert!(!manager
        .should_prompt_run_review(&store, "run-harvested")
        .await
        .unwrap());
    // Non-terminal runs never prompt.
    assert!(!manager
        .should_prompt_run_review(&store, "run-active")
        .await
        .unwrap());
    assert_eq!(runner.commands.lock().unwrap().len(), 0);

    // No declared outputs: the workspace listing decides. Files present →
    // prompt; empty workspace → stay silent.
    assert!(manager
        .should_prompt_run_review(&store, "run-files")
        .await
        .unwrap());
    assert!(!manager
        .should_prompt_run_review(&store, "run-empty")
        .await
        .unwrap());
    assert_eq!(runner.commands.lock().unwrap().len(), 2);

    // A dismissed prompt stays dismissed, and dismissal is idempotent.
    assert!(store
        .mark_run_review_dismissed("run-unharvested")
        .await
        .unwrap());
    assert!(!store
        .mark_run_review_dismissed("run-unharvested")
        .await
        .unwrap());
    assert!(store.run_review_dismissed("run-unharvested").await.unwrap());
    assert!(!manager
        .should_prompt_run_review(&store, "run-unharvested")
        .await
        .unwrap());

    // Exploratory local command runs are out of scope regardless of state.
    let mut local = wisp_store::RunRecord::new("run-local", "p", "local", "Local", "command");
    local.frame_id = Some("f".into());
    local.status = wisp_store::RunStatus::Succeeded;
    store.create_run(&local).await.unwrap();
    assert!(!manager
        .should_prompt_run_review(&store, "run-local")
        .await
        .unwrap());

    let _ = std::fs::remove_dir_all(&tmp);
}

// --- retention sweep ---------------------------------------------------------

async fn seed_retention_run(
    store: &wisp_store::Store,
    run_id: &str,
    status: wisp_store::RunStatus,
    ended_days_ago: i64,
    specs_json: &str,
    harvested: bool,
) {
    let mut run = wisp_store::RunRecord::new(run_id, "p", "ssh:gpu", "Remote", "ssh_direct");
    run.frame_id = Some("f".into());
    run.status = status;
    run.command = Some("make outputs".into());
    run.output_specs_json = specs_json.into();
    run.ended_at = Some(chrono::Utc::now().timestamp() - ended_days_ago * 86_400);
    let connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&harvest_test_context()).unwrap();
    let handle = RemoteRunHandle::SshDirect {
        connection,
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "retention-token".into(),
        inputs_staged: true,
        pgid: Some(4242),
        start_time: Some(99),
    };
    run.remote_workdir = Some(handle.display_workdir());
    run.remote_handle_json = Some(serde_json::to_string(&handle).unwrap());
    store.create_run(&run).await.unwrap();
    if harvested {
        store.mark_run_harvested(run_id).await.unwrap();
    }
}

#[tokio::test]
async fn retention_sweep_cleans_only_expired_eligible_runs() {
    let tmp = std::env::temp_dir().join(format!("wisp_retention_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "proj", &tmp.to_string_lossy())
        .await
        .unwrap();
    store.create_frame("f", "p", "OPERON", "m").await.unwrap();
    store
        .upsert_execution_context(&harvest_test_context())
        .await
        .unwrap();
    let specs = serde_json::json!([{
        "glob": "results/*.tsv", "kind": "table", "residency": "auto"
    }])
    .to_string();

    // Retention disabled: nothing is due even when old.
    seed_retention_run(
        &store,
        "run-old",
        wisp_store::RunStatus::Succeeded,
        30,
        "[]",
        false,
    )
    .await;
    assert!(store
        .list_runs_due_for_retention(chrono::Utc::now().timestamp())
        .await
        .unwrap()
        .is_empty());

    store
        .set_project_run_retention("p", Some(7), Some(14), None)
        .await
        .unwrap();
    assert!(store
        .set_project_run_retention("p", Some(0), None, None)
        .await
        .is_err());

    // Eligible: succeeded+harvested past 7d; succeeded with no declared specs;
    // failed past 14d. Not eligible: unharvested specs, recent, still running.
    seed_retention_run(
        &store,
        "run-done",
        wisp_store::RunStatus::Succeeded,
        8,
        &specs,
        true,
    )
    .await;
    seed_retention_run(
        &store,
        "run-unharvested",
        wisp_store::RunStatus::Succeeded,
        8,
        &specs,
        false,
    )
    .await;
    seed_retention_run(
        &store,
        "run-recent",
        wisp_store::RunStatus::Succeeded,
        2,
        "[]",
        false,
    )
    .await;
    seed_retention_run(
        &store,
        "run-failed-old",
        wisp_store::RunStatus::Failed,
        15,
        &specs,
        false,
    )
    .await;
    seed_retention_run(
        &store,
        "run-failed-new",
        wisp_store::RunStatus::Failed,
        8,
        &specs,
        false,
    )
    .await;
    seed_retention_run(
        &store,
        "run-active",
        wisp_store::RunStatus::Running,
        15,
        "[]",
        false,
    )
    .await;

    let due: Vec<String> = store
        .list_runs_due_for_retention(chrono::Utc::now().timestamp())
        .await
        .unwrap()
        .into_iter()
        .map(|run| run.id)
        .collect();
    assert_eq!(due, vec!["run-old", "run-failed-old", "run-done"]);

    // The first cleanup fails; the sweep continues and cleans the rest.
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        Err("connection refused".into()),
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner.clone());
    let cleaned = manager.run_retention_sweep(&store).await.unwrap();
    assert_eq!(cleaned, 2);
    assert!(store
        .get_run("run-old")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_none());
    assert!(store
        .get_run("run-done")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_some());
    assert!(store
        .get_run("run-failed-old")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_some());
    assert!(store
        .get_run("run-unharvested")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_none());

    // The failed run retries on the next sweep and drops out once cleaned.
    let runner = Arc::new(ScriptedRunRunner::new(vec![
        log_pull_absent(),
        ok_output("__WISP_CLEANUP__:done\n"),
    ]));
    let manager = RunManager::with_runner(runner);
    assert_eq!(manager.run_retention_sweep(&store).await.unwrap(), 1);
    assert!(store
        .get_run("run-old")
        .await
        .unwrap()
        .unwrap()
        .cleaned_at
        .is_some());
    assert_eq!(manager.run_retention_sweep(&store).await.unwrap(), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}
