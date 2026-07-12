use super::*;
use std::collections::VecDeque;
#[cfg(unix)]
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

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

    manager.cancel(&store, &submitted.run_id).await.unwrap();
    let run = store.get_run(&submitted.run_id).await.unwrap().unwrap();
    assert_eq!(run.status, wisp_store::RunStatus::Cancelled);

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
    store
        .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
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
    let command = "printf '%s\\n' '$HOME' && printf '%s\\n' '$(date)'";
    let submitted = manager
        .submit(
            store.clone(),
            "p".into(),
            None,
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
    for tool in ["run_in_context", "get_run", "cancel_run"] {
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
    assert!(skill.contains("Do not wait for completion"));
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
}

impl ScriptedRunRunner {
    fn new(outputs: Vec<Result<RunCommandOutput, String>>) -> Self {
        Self {
            outputs: StdMutex::new(outputs.into()),
            commands: StdMutex::new(Vec::new()),
            synthesize_launch_ack: std::sync::atomic::AtomicBool::new(false),
            token: StdMutex::new(None),
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
    let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output("")]));
    let manager = RunManager::with_runner(runner.clone());
    let mut context = wisp_store::ExecutionContext::new("ssh:CPU", "CPU").unwrap();
    context.config_json = serde_json::json!({
        "alias": "cpu.example",
        "user": "alice",
        "port": 2222,
        "identity_file": "/keys/cpu"
    })
    .to_string();
    let destination = std::env::temp_dir().join("results.tar.gz");

    manager
        .download_ssh_file(&context, "/data/results.tar.gz", &destination)
        .await
        .unwrap();

    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "scp");
    assert!(commands[0]
        .args
        .windows(2)
        .any(|args| args == ["-P", "2222"]));
    assert!(commands[0]
        .args
        .windows(2)
        .any(|args| args == ["-i", "/keys/cpu"]));
    assert_eq!(
        &commands[0].args[commands[0].args.len() - 2..],
        [
            "alice@cpu.example:/data/results.tar.gz".to_string(),
            destination.to_string_lossy().into_owned()
        ]
    );
}

fn poll_response(status: &str, stdout: &str, stderr: &str) -> String {
    format!("__WISP_RUN_STATUS__:{status}\n__WISP_STDOUT__\n{stdout}\n__WISP_STDERR__\n{stderr}\n")
}

fn test_handle(run_id: &str, confirmed: bool) -> RemoteRunHandle {
    RemoteRunHandle::SshDirect {
        connection: crate::ssh_hosts::SshConnection {
            alias: "gpu".into(),
            user: None,
            port: None,
            identity_file: None,
        },
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: "test-token".into(),
        pgid: confirmed.then_some(4242),
        start_time: confirmed.then_some(999),
    }
}

async fn wait_for_terminal(store: &wisp_store::Store, run_id: &str) -> wisp_store::RunRecord {
    tokio::time::timeout(Duration::from_secs(2), async {
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
