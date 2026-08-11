use super::*;
use std::collections::VecDeque;
#[cfg(unix)]
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

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

/// Records structured progress payloads a tool emits while running.
struct RecordingRunToolEnv {
    root: PathBuf,
    progress: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl RecordingRunToolEnv {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            progress: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl wisp_tools::ToolEnv for RecordingRunToolEnv {
    fn project_root(&self) -> &std::path::Path {
        &self.root
    }

    async fn confirm(&self, _message: &str) -> bool {
        true
    }

    async fn emit(&self, event: wisp_tools::ToolEvent) {
        if let wisp_tools::ToolEvent::Progress { details } = event {
            self.progress.lock().unwrap().push(details);
        }
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
    // The model sees a human summary; the structured record rides `details`.
    assert!(result.content.contains("finished with status succeeded"));
    assert!(result.content.contains("stdout tail"));
    let run: wisp_store::RunRecord =
        serde_json::from_value(result.details.clone().expect("run details")).unwrap();
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
    let run: wisp_store::RunRecord =
        serde_json::from_value(result.details.clone().expect("run details")).unwrap();
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
    let manager = RunManager::with_runner(Arc::new(FakeRunRunner {
        output: Ok(RunCommandOutput {
            exit_code: 9,
            stdout: "3.12.4\n".into(),
            stderr: "missing modules: decoupler".into(),
        }),
    }));
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
    let run: wisp_store::RunRecord =
        serde_json::from_value(result.details.clone().expect("run details")).unwrap();
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
    let tool = RunInContextTool::new(
        store.clone(),
        RunManager::with_runner(Arc::new(FakeRunRunner {
            output: ok_output("should not run"),
        })),
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
    assert!(snapshot.content.contains("Call monitor_run exactly once"));

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
    let run: wisp_store::RunRecord =
        serde_json::from_value(result.details.clone().expect("run details")).unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn monitor_run_streams_progress_while_waiting() {
    use wisp_tools::Tool;
    let tmp = std::env::temp_dir().join(format!("wisp_monitor_progress_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .create_project("p", "project", &tmp.to_string_lossy())
        .await
        .unwrap();
    let run = wisp_store::RunRecord::new("live-run", "p", "local", "Live run", "command");
    store.create_run(&run).await.unwrap();
    assert!(store
        .activate_run_lifecycle(
            "live-run",
            wisp_store::RunStatus::Submitted,
            "monitor-owner",
            60,
        )
        .await
        .unwrap());
    let finishing_store = store.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(finishing_store
            .finish_active_run_owned(
                "live-run",
                "monitor-owner",
                wisp_store::RunStatus::Succeeded,
                Some(0),
            )
            .await
            .unwrap());
    });

    let env = RecordingRunToolEnv::new(tmp.clone());
    let result = MonitorRunTool::new(store, "p".into())
        .run(&serde_json::json!({ "run_id": "live-run" }), &env)
        .await;

    assert!(result.success, "{}", result.content);
    // A non-terminal snapshot streamed as structured progress while waiting…
    let progress: Vec<serde_json::Value> = env
        .progress
        .lock()
        .unwrap()
        .iter()
        .filter(|details| details["id"] == "live-run")
        .cloned()
        .collect();
    assert!(
        !progress.is_empty(),
        "the wait must stream at least one live Run snapshot"
    );
    // …and the terminal record lands once, on the result's details.
    let final_run = result.details.clone().expect("final run details");
    assert_eq!(final_run["status"], serde_json::json!("succeeded"));
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
    let runner = FakeRunRunner {
        output: Ok(RunCommandOutput {
            exit_code: 0,
            stdout: "hello\n".into(),
            stderr: String::new(),
        }),
    };

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
    let runner = FakeRunRunner {
        output: Ok(RunCommandOutput {
            exit_code: 0,
            stdout: "ok".into(),
            stderr: String::new(),
        }),
    };

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
    let runner = FakeRunRunner {
        output: Err("timed out".into()),
    };

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
    let runner = FakeRunRunner {
        output: Ok(RunCommandOutput {
            exit_code: 0,
            stdout: "done".into(),
            stderr: String::new(),
        }),
    };

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
    let runner = FakeRunRunner {
        output: Ok(RunCommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }),
    };

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
        1
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
    let skill = include_str!("../../../skills/remote-compute-ssh/SKILL.md");
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
    assert!(skill.contains("call `monitor_run` exactly once"));
    assert!(skill.contains("never call it repeatedly"));
    assert!(!skill.contains("ssh <alias>"));
    assert!(skill.contains("Scheduler lifecycle is not implemented yet"));
}

struct FakeRunRunner {
    output: Result<RunCommandOutput, String>,
}

struct StreamingRunRunner;

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
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        updates
            .send(RunOutputUpdate {
                stream: RunOutputStream::Stderr,
                chunk: b"warning: slow API\n".to_vec(),
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
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

    let task_store = store.clone();
    let task = tokio::spawn(async move {
        run_with_lifecycle_lease(
            &task_store,
            "streaming",
            "stream-owner",
            &StreamingRunRunner,
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

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let live = store.get_run("streaming").await.unwrap().unwrap();
    assert_eq!(live.status, wisp_store::RunStatus::Running);
    assert_eq!(live.stdout_tail.as_deref(), Some("phase 1 complete\n"));
    assert!(live.last_polled_at.is_some(), "heartbeat was not recorded");

    let output = task.await.unwrap().unwrap();
    assert_eq!(output.exit_code, 0);
    let _ = std::fs::remove_dir_all(tmp);
}

#[async_trait::async_trait]
impl RunCommandRunner for FakeRunRunner {
    async fn run(
        &self,
        _command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
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
    let destination = std::env::temp_dir().join("results.tar.gz");

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
        "SSH launch failed: connection timed out"
    ));
    assert!(permanent_remote_start_error(
        "SSH authentication gate blocked for `ssh:gpu` after a previous failure"
    ));
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
