use super::*;
use std::collections::VecDeque;
#[cfg(unix)]
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

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
    let command = RunCommand {
        context_id: "local".into(),
        program: "sh".into(),
        args: vec!["-c".into(), "sleep 1 & wait".into()],
        script: String::new(),
        cwd: None,
        stdin: None,
        envs: Vec::new(),
    };

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        ProcessRunRunner.run(command, Duration::from_millis(20)),
    )
    .await
    .expect("runner leaked a pipe reader after timeout")
    .unwrap_err();
    assert!(result.contains("timed out"));
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
    let manager = RunManager::with_runner(Arc::new(FakeRunRunner {
        output: ok_output("finished\n"),
    }));
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
    assert_eq!(run.stdout_tail.as_deref(), Some("finished\n"));
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
    store
        .update_run_status("long-run", wisp_store::RunStatus::Submitted)
        .await
        .unwrap();
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
        finishing_store
            .update_run_status("long-run", wisp_store::RunStatus::Running)
            .await
            .unwrap();
        finishing_store
            .update_run_status("long-run", wisp_store::RunStatus::Succeeded)
            .await
            .unwrap();
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
                max_file_mb: Some(1),
                max_total_mb: Some(1),
            }]),
        },
        &runner,
        Some(tmp.clone()),
    )
    .await
    .unwrap();

    let links = store.list_run_artifacts(&res.run_id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].1, "table");
    assert_eq!(store.list_artifacts("f").await.unwrap().len(), 1);

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
    let manager = RunManager::with_runner(Arc::new(PendingRunRunner));

    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            None,
            SubmitRunRequest {
                context_id: "local".into(),
                command: "long-running-analysis".into(),
                title: None,
                timeout_secs: Some(60),
                input_paths: None,
                output_specs: None,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(submitted.status, wisp_store::RunStatus::Submitted);
    assert!(manager.has_in_flight_project(&store, "p").await.unwrap());
    assert!(!manager
        .has_in_flight_project(&store, "other-project")
        .await
        .unwrap());

    manager.cancel(&store, &submitted.run_id).await.unwrap();
    let run = store.get_run(&submitted.run_id).await.unwrap().unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Cancelled);

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
    *runner.poll_gate.lock().unwrap() = Some(poll_gate.clone());
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
    store.create_run(&run).await.unwrap();
    store
        .update_run_status("stale", wisp_store::RunStatus::Submitted)
        .await
        .unwrap();
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
    store.create_run(&remote).await.unwrap();
    store
        .update_run_status("remote", wisp_store::RunStatus::Running)
        .await
        .unwrap();

    let local = wisp_store::RunRecord::new("local-run", "p", "local", "Local", "command");
    store.create_run(&local).await.unwrap();
    store
        .update_run_status("local-run", wisp_store::RunStatus::Running)
        .await
        .unwrap();

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
    store.create_run(&run).await.unwrap();
    store
        .update_run_status("remote", wisp_store::RunStatus::Running)
        .await
        .unwrap();

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
    store.create_run(&run).await.unwrap();
    store
        .update_run_status("remote", wisp_store::RunStatus::Running)
        .await
        .unwrap();
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(
        "__WISP_CANCEL__:cancelled\n",
    )]));
    let manager = RunManager::with_runner(runner.clone());

    manager.cancel(&store, "remote").await.unwrap();
    assert_eq!(
        store.get_run("remote").await.unwrap().unwrap().status,
        wisp_store::RunStatus::Cancelling
    );
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
    store.create_run(&run).await.unwrap();
    store
        .update_run_status("upload", wisp_store::RunStatus::Submitted)
        .await
        .unwrap();
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
    let scripts = [
        prepare_payload(&remote),
        launch_payload(&remote.handle),
        poll_payload(&remote.handle).unwrap(),
        cancel_payload(&remote.handle).unwrap(),
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
}

#[test]
fn remote_compute_skill_uses_the_real_wisp_run_contract() {
    let skill = include_str!("../../../skills/remote-compute-ssh/SKILL.md");
    for tool in ["run_in_context", "get_run", "monitor_run", "cancel_run"] {
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
    assert!(skill.contains("Scheduler lifecycle is not implemented yet"));
}

struct FakeRunRunner {
    output: Result<RunCommandOutput, String>,
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

struct PendingRunRunner;

#[async_trait::async_trait]
impl RunCommandRunner for PendingRunRunner {
    async fn run(
        &self,
        _command: RunCommand,
        _timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        std::future::pending().await
    }
}

struct ScriptedRunRunner {
    outputs: StdMutex<VecDeque<Result<RunCommandOutput, String>>>,
    commands: StdMutex<Vec<RunCommand>>,
    synthesize_launch_ack: std::sync::atomic::AtomicBool,
    token: StdMutex<Option<String>>,
    // When set, "poll SSH Run" commands block until the test releases a permit,
    // so a test can observe pre-completion state without racing the poller.
    poll_gate: StdMutex<Option<Arc<tokio::sync::Semaphore>>>,
}

impl ScriptedRunRunner {
    fn new(outputs: Vec<Result<RunCommandOutput, String>>) -> Self {
        Self {
            outputs: StdMutex::new(outputs.into()),
            commands: StdMutex::new(Vec::new()),
            synthesize_launch_ack: std::sync::atomic::AtomicBool::new(false),
            token: StdMutex::new(None),
            poll_gate: StdMutex::new(None),
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
        if command.script == "poll SSH Run" {
            let gate = self.poll_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                let _permit = gate.acquire().await.unwrap();
            }
        }
        if command.script == "prepare SSH Run" {
            if let Some(payload) = command.stdin.as_deref() {
                let token = payload
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("  printf '%s\\n' '")?
                            .strip_suffix("' > \"$workdir/token.tmp\"")
                    })
                    .map(str::to_string);
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
        if command.script == "launch SSH Run"
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
        "SSH connectivity gate blocked for `ssh:gpu` after a previous failure"
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
