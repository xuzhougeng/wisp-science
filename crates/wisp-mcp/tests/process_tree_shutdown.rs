use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wisp_mcp::McpClient;

const WRAPPER_ARG: &str = "--fake-mcp-wrapper";
const GRANDCHILD_ARG: &str = "--fake-mcp-grandchild";
const FAIL_INITIALIZE_ARG: &str = "--fail-initialize";

fn main() -> ExitCode {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some(WRAPPER_ARG) => fake_wrapper(
            args.get(2).map(PathBuf::from),
            args.get(3).is_some_and(|arg| arg == FAIL_INITIALIZE_ARG),
        ),
        Some(GRANDCHILD_ARG) => fake_grandchild(args.get(2).map(PathBuf::from)),
        _ => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            match runtime.block_on(run_regressions()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("process-tree shutdown regression failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

async fn run_regressions() -> Result<(), String> {
    initialize_failure_terminates_wrapper_and_grandchild().await?;
    drop_client_terminates_wrapper_and_grandchild().await?;
    explicit_shutdown_is_idempotent_and_terminates_process_tree().await?;
    cancelled_shutdown_terminates_process_tree().await?;
    cancelled_request_terminates_process_tree().await
}

fn fake_wrapper(root: Option<PathBuf>, fail_initialize: bool) -> ExitCode {
    let Some(root) = root else {
        return ExitCode::FAILURE;
    };
    if write_pid(&root.join("wrapper.pid")).is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(executable) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let Ok(_grandchild) = std::process::Command::new(executable)
        .arg(GRANDCHILD_ARG)
        .arg(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return ExitCode::FAILURE;
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return ExitCode::FAILURE;
        };
        let Some(id) = request.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let method = request.get("method").and_then(Value::as_str);
        if fail_initialize && method == Some("initialize") {
            // Return a protocol-level failure instead of relying on stdout EOF.
            // On Windows a long-lived grandchild can inherit the wrapper's pipe
            // handle, which would make this fixture wait for the unrelated 120s
            // request timeout instead of exercising initialization cleanup.
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": "forced initialize failure" }
            });
            if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
                return ExitCode::FAILURE;
            }
            return ExitCode::FAILURE;
        }
        let result = match method {
            Some("initialize") => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-wrapper", "version": "1" }
            }),
            Some("tools/list") => json!({
                "tools": [{
                    "name": "hang",
                    "description": "Never answer the request",
                    "inputSchema": { "type": "object" }
                }]
            }),
            Some("tools/call") => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            _ => return ExitCode::FAILURE,
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn fake_grandchild(root: Option<PathBuf>) -> ExitCode {
    let Some(root) = root else {
        return ExitCode::FAILURE;
    };
    #[cfg(unix)]
    unsafe {
        // Force the explicit shutdown test through its SIGKILL path after the
        // wrapper/group leader has exited on stdin EOF. This exercises the
        // invariant that the leader is not reaped before the final group signal.
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    if write_pid(&root.join("grandchild.pid")).is_err() {
        return ExitCode::FAILURE;
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn write_pid(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, std::process::id().to_string())
}

async fn drop_client_terminates_wrapper_and_grandchild() -> Result<(), String> {
    let fixture = launch_fixture().await?;

    drop(fixture.client);
    assert_fixture_stopped(fixture.root, fixture.wrapper, fixture.grandchild).await
}

async fn initialize_failure_terminates_wrapper_and_grandchild() -> Result<(), String> {
    let root = unique_temp_dir()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![
        WRAPPER_ARG.to_string(),
        root.to_string_lossy().into_owned(),
        FAIL_INITIALIZE_ARG.to_string(),
    ];
    if McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .is_ok()
    {
        return Err("fake MCP initialize failure unexpectedly succeeded".into());
    }
    let wrapper = wait_for_pid(&root.join("wrapper.pid")).await?;
    let grandchild = wait_for_pid(&root.join("grandchild.pid")).await?;
    assert_fixture_stopped(root, wrapper, grandchild).await
}

async fn explicit_shutdown_is_idempotent_and_terminates_process_tree() -> Result<(), String> {
    let fixture = launch_fixture().await?;
    let (first, concurrent) = tokio::join!(fixture.client.shutdown(), fixture.client.shutdown());
    first.map_err(|error| format!("first explicit shutdown failed: {error}"))?;
    concurrent.map_err(|error| format!("concurrent explicit shutdown failed: {error}"))?;
    fixture
        .client
        .shutdown()
        .await
        .map_err(|error| format!("second explicit shutdown failed: {error}"))?;
    drop(fixture.client);
    assert_fixture_stopped(fixture.root, fixture.wrapper, fixture.grandchild).await
}

async fn cancelled_shutdown_terminates_process_tree() -> Result<(), String> {
    let fixture = launch_fixture().await?;
    let shutdown = fixture.client.shutdown();
    if let Ok(result) = tokio::time::timeout(Duration::from_millis(50), shutdown).await {
        cleanup_fixture(&fixture.root, fixture.wrapper, fixture.grandchild);
        return Err(format!(
            "MCP shutdown unexpectedly completed before cancellation: {result:?}"
        ));
    }

    // Cancelling orderly shutdown during its EOF grace period must fall back
    // to synchronous tree termination while the client owner remains alive.
    let wrapper_stopped = wait_until_stopped(fixture.wrapper, Duration::from_secs(3)).await;
    let grandchild_stopped = wait_until_stopped(fixture.grandchild, Duration::from_secs(3)).await;
    if !wrapper_stopped || !grandchild_stopped {
        cleanup_fixture(&fixture.root, fixture.wrapper, fixture.grandchild);
        return Err(format!(
            "cancelled MCP shutdown left processes alive: wrapper_alive={}, grandchild_alive={}",
            !wrapper_stopped, !grandchild_stopped
        ));
    }

    let cleanup_result = fixture.client.shutdown().await;
    drop(fixture.client);
    cleanup_result.map_err(|error| format!("retry after cancelled shutdown failed: {error}"))?;
    assert_fixture_stopped(fixture.root, fixture.wrapper, fixture.grandchild).await
}

async fn cancelled_request_terminates_process_tree() -> Result<(), String> {
    let fixture = launch_fixture().await?;
    let arguments = json!({});
    let request = fixture.client.tool_call("hang", &arguments);
    if let Ok(result) = tokio::time::timeout(Duration::from_millis(150), request).await {
        cleanup_fixture(&fixture.root, fixture.wrapper, fixture.grandchild);
        return Err(format!(
            "fake hanging request unexpectedly completed: {result:?}"
        ));
    }

    // The outer timeout cancels the in-flight request future. Its cancellation
    // guard must synchronously terminate the whole tree and schedule direct-
    // child reaping; this must complete while the client owner remains alive,
    // before any explicit shutdown supplies a second cleanup path.
    let wrapper_stopped = wait_until_stopped(fixture.wrapper, Duration::from_secs(3)).await;
    let grandchild_stopped = wait_until_stopped(fixture.grandchild, Duration::from_secs(3)).await;
    if !wrapper_stopped || !grandchild_stopped {
        cleanup_fixture(&fixture.root, fixture.wrapper, fixture.grandchild);
        return Err(format!(
            "cancelled MCP request left processes alive before shutdown: wrapper_alive={}, grandchild_alive={}",
            !wrapper_stopped, !grandchild_stopped
        ));
    }

    // Explicit shutdown remains safe and idempotent after the cancellation
    // path has already killed and reaped the direct child.
    let cleanup_result = fixture.client.shutdown().await;
    drop(fixture.client);
    cleanup_result
        .map_err(|error| format!("shutdown after request cancellation failed: {error}"))?;
    assert_fixture_stopped(fixture.root, fixture.wrapper, fixture.grandchild).await
}

struct Fixture {
    root: PathBuf,
    client: McpClient,
    wrapper: u32,
    grandchild: u32,
}

async fn launch_fixture() -> Result<Fixture, String> {
    let root = unique_temp_dir()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = vec![WRAPPER_ARG.to_string(), root.to_string_lossy().into_owned()];
    let client = McpClient::launch(&executable.to_string_lossy(), &args)
        .await
        .map_err(|error| error.to_string())?;
    let wrapper = wait_for_pid(&root.join("wrapper.pid")).await?;
    let grandchild = wait_for_pid(&root.join("grandchild.pid")).await?;
    Ok(Fixture {
        root,
        client,
        wrapper,
        grandchild,
    })
}

async fn assert_fixture_stopped(
    root: PathBuf,
    wrapper: u32,
    grandchild: u32,
) -> Result<(), String> {
    let wrapper_stopped = wait_until_stopped(wrapper, Duration::from_secs(3)).await;
    let grandchild_stopped = wait_until_stopped(grandchild, Duration::from_secs(3)).await;
    if !wrapper_stopped {
        terminate_exact(wrapper);
    }
    if !grandchild_stopped {
        terminate_exact(grandchild);
    }
    let _ = std::fs::remove_dir_all(&root);

    if !wrapper_stopped || !grandchild_stopped {
        return Err(format!(
            "McpClient cleanup left processes alive: wrapper_alive={}, grandchild_alive={}",
            !wrapper_stopped, !grandchild_stopped
        ));
    }
    Ok(())
}

fn cleanup_fixture(root: &Path, wrapper: u32, grandchild: u32) {
    terminate_exact(wrapper);
    terminate_exact(grandchild);
    let _ = std::fs::remove_dir_all(root);
}

fn unique_temp_dir() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("wisp-mcp-process-tree-{nonce}"));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

async fn wait_for_pid(path: &Path) -> Result<u32, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(value) = std::fs::read_to_string(path) {
            if let Ok(pid) = value.trim().parse::<u32>() {
                return Ok(pid);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for {}", path.display()));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until_stopped(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !process_exists(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn terminate_exact(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    unsafe {
        let process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if process.is_null() {
            return false;
        }
        let status = WaitForSingleObject(process, 0);
        CloseHandle(process);
        status == WAIT_TIMEOUT
    }
}

#[cfg(windows)]
fn terminate_exact(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}
