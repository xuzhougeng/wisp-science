//! Ephemeral interactive terminals backed by a local PTY/ConPTY.
//!
//! `ExecutionContext` selects what is launched (local shell, WSL, or OpenSSH),
//! while `Run` remains the durable abstraction for tracked computation. A
//! terminal panel is only a view: collapsing it keeps the session attached,
//! while closing a terminal tab terminates and unregisters its session. Each
//! `open_terminal` call creates an independent PTY so multiple terminals can
//! run concurrently, including multiple shells in the same context.

use crate::workspace_surface::WorkspaceSurface;
use base64::Engine;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State;

const DEFAULT_ROWS: u16 = 30;
const DEFAULT_COLS: u16 = 100;
const MAX_SCROLLBACK_BYTES: usize = 4 * 1024 * 1024;

struct TerminalKiller {
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
    #[cfg(not(windows))]
    inner: Box<dyn portable_pty::ChildKiller + Send + Sync>,
}

impl TerminalKiller {
    fn new(child: &dyn Child) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::BorrowedHandle;
            let raw = child.as_raw_handle().ok_or_else(|| {
                std::io::Error::other("terminal child has no Windows process handle")
            })?;
            // Duplicate while the child still owns the handle. The waiter may
            // exit concurrently later; never reopen by a potentially reused PID.
            let handle = unsafe { BorrowedHandle::borrow_raw(raw) }.try_clone_to_owned()?;
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                inner: child.clone_killer(),
            })
        }
    }

    fn kill(&mut self) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
            use windows::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
            let handle = HANDLE(self.handle.as_raw_handle());
            // portable-pty 0.9's cloned Windows killer inverts the BOOL result.
            // Use the typed API and only ignore failure if the process exited
            // concurrently (TerminateProcess then reports access denied).
            if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                return Ok(());
            }
            match unsafe { TerminateProcess(handle, 1) } {
                Ok(()) => Ok(()),
                Err(_) if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
        #[cfg(not(windows))]
        {
            self.inner.kill()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub display_cwd: String,
    pub envs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionSummary {
    pub id: String,
    pub project_id: String,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub display_cwd: String,
    pub process_id: Option<u32>,
    pub running: bool,
}

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum TerminalEvent {
    Output { base64: String },
    Exit { exit_code: u32 },
    Error { message: String },
}

struct TerminalOutputState {
    scrollback: Vec<u8>,
    subscribers: Vec<Channel<TerminalEvent>>,
    exit_code: Option<u32>,
}

impl TerminalOutputState {
    fn new() -> Self {
        Self {
            scrollback: Vec::new(),
            subscribers: Vec::new(),
            exit_code: None,
        }
    }
}

struct TerminalSession {
    id: String,
    project_id: String,
    scope_key: String,
    context_id: String,
    title: String,
    kind: String,
    display_cwd: String,
    process_id: Option<u32>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<TerminalKiller>,
    output: Mutex<TerminalOutputState>,
    /// One-shot OpenSSH askpass files. Must stay on disk until the child
    /// exits — authentication happens after spawn, not at spawn.
    auth_cleanup_envs: Vec<(String, String)>,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        crate::ssh_hosts::cleanup_password_auth_env(&self.auth_cleanup_envs);
    }
}

impl TerminalSession {
    fn running(&self) -> bool {
        lock(&self.output).exit_code.is_none()
    }

    fn summary(&self) -> TerminalSessionSummary {
        TerminalSessionSummary {
            id: self.id.clone(),
            project_id: self.project_id.clone(),
            context_id: self.context_id.clone(),
            title: self.title.clone(),
            kind: self.kind.clone(),
            display_cwd: self.display_cwd.clone(),
            process_id: self.process_id,
            running: self.running(),
        }
    }

    fn attach(&self, on_event: Channel<TerminalEvent>) -> Result<(), String> {
        let mut output = lock(&self.output);
        if !output.scrollback.is_empty() {
            on_event
                .send(output_event(&output.scrollback))
                .map_err(|error| error.to_string())?;
        }
        if let Some(exit_code) = output.exit_code {
            on_event
                .send(TerminalEvent::Exit { exit_code })
                .map_err(|error| error.to_string())?;
        } else {
            output.subscribers.push(on_event);
        }
        Ok(())
    }

    fn write(&self, data: &str) -> Result<(), String> {
        if !self.running() {
            return Err("Terminal session is no longer running".into());
        }
        let mut writer = lock(&self.writer);
        writer
            .write_all(data.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|error| format!("failed to write terminal input: {error}"))
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        if rows == 0 || cols == 0 {
            return Err("Terminal rows and columns must be greater than zero".into());
        }
        lock(&self.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("failed to resize terminal: {error}"))
    }

    fn terminate(&self) -> Result<(), String> {
        if !self.running() {
            return Ok(());
        }
        lock(&self.killer)
            .kill()
            .map_err(|error| format!("failed to terminate terminal: {error}"))
    }

    fn push_output(&self, bytes: &[u8]) {
        let mut output = lock(&self.output);
        append_scrollback(&mut output.scrollback, bytes, MAX_SCROLLBACK_BYTES);
        let event = output_event(bytes);
        output
            .subscribers
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }

    fn push_error(&self, message: String) {
        let mut output = lock(&self.output);
        let event = TerminalEvent::Error { message };
        output
            .subscribers
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }

    fn finish(&self, exit_code: u32) {
        let mut output = lock(&self.output);
        if output.exit_code.replace(exit_code).is_some() {
            return;
        }
        crate::ssh_hosts::cleanup_password_auth_env(&self.auth_cleanup_envs);
        let event = TerminalEvent::Exit { exit_code };
        output
            .subscribers
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
        output.subscribers.clear();
    }
}

#[derive(Default)]
struct TerminalManagerState {
    sessions: HashMap<String, Arc<TerminalSession>>,
}

#[derive(Clone, Default)]
pub struct TerminalManager {
    state: Arc<Mutex<TerminalManagerState>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn open(
        &self,
        project_id: &str,
        scope_key: &str,
        project_root: &Path,
        context: &wisp_store::ExecutionContext,
    ) -> Result<TerminalSessionSummary, String> {
        let spec = build_terminal_launch_spec(context, project_root)?;
        let cleanup_envs = spec.envs.clone();
        let label = if context.label.trim().is_empty() {
            context.id.clone()
        } else {
            context.label.clone()
        };
        self.open_spec_with_cleanup(
            project_id,
            scope_key,
            &context.id,
            format!("{label} — Terminal"),
            context.kind.as_str(),
            spec,
            cleanup_envs,
        )
    }

    pub(crate) fn open_spec(
        &self,
        project_id: &str,
        scope_key: &str,
        context_id: &str,
        title: String,
        kind: &str,
        spec: TerminalLaunchSpec,
    ) -> Result<TerminalSessionSummary, String> {
        self.open_spec_with_cleanup(
            project_id,
            scope_key,
            context_id,
            title,
            kind,
            spec,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open_spec_with_cleanup(
        &self,
        project_id: &str,
        scope_key: &str,
        context_id: &str,
        title: String,
        kind: &str,
        spec: TerminalLaunchSpec,
        cleanup_envs: Vec<(String, String)>,
    ) -> Result<TerminalSessionSummary, String> {
        let (session, reader, child) = spawn_session(
            project_id,
            scope_key,
            context_id,
            title,
            kind,
            spec,
            cleanup_envs,
        )?;
        let summary = session.summary();
        lock(&self.state)
            .sessions
            .insert(session.id.clone(), Arc::clone(&session));

        start_terminal_workers(Arc::clone(&session), reader, child);
        Ok(summary)
    }

    fn get(&self, id: &str) -> Result<Arc<TerminalSession>, String> {
        lock(&self.state)
            .sessions
            .get(id)
            .cloned()
            .ok_or_else(|| format!("Terminal session not found: {id}"))
    }

    fn close(&self, id: &str) -> Result<(), String> {
        let session = self.get(id)?;
        session.terminate()?;
        lock(&self.state).sessions.remove(id);
        Ok(())
    }

    pub fn shutdown_all(&self) {
        let sessions = lock(&self.state)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            let _ = session.terminate();
        }
    }

    pub(crate) fn has_running(&self, project_id: &str, scope_key: &str) -> bool {
        lock(&self.state).sessions.values().any(|session| {
            session.project_id == project_id && session.scope_key == scope_key && session.running()
        })
    }

    /// Terminate and forget every terminal owned by one state scope. Promotion
    /// uses this for losing exploration candidates before their workspaces are
    /// quarantined and removed.
    pub(crate) fn stop_scope(&self, project_id: &str, scope_key: &str) {
        let targets = lock(&self.state)
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.project_id == project_id && session.scope_key == scope_key
            })
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect::<Vec<_>>();
        for (_, session) in &targets {
            let _ = session.terminate();
        }
        let mut state = lock(&self.state);
        for (id, _) in targets {
            state.sessions.remove(&id);
        }
    }
}

fn spawn_session(
    project_id: &str,
    scope_key: &str,
    context_id: &str,
    title: String,
    kind: &str,
    spec: TerminalLaunchSpec,
    cleanup_envs: Vec<(String, String)>,
) -> Result<
    (
        Arc<TerminalSession>,
        Box<dyn Read + Send>,
        Box<dyn Child + Send + Sync>,
    ),
    String,
> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to create terminal PTY: {error}"))?;

    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for (key, value) in &spec.envs {
        command.env(key, value);
    }
    if let Some(cwd) = &spec.cwd {
        command.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(command).map_err(|error| {
        crate::ssh_hosts::cleanup_password_auth_env(&cleanup_envs);
        format!("failed to start {context_id} terminal: {error}")
    })?;
    drop(pair.slave);

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            crate::ssh_hosts::cleanup_password_auth_env(&cleanup_envs);
            return Err(format!("failed to read terminal PTY: {error}"));
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            crate::ssh_hosts::cleanup_password_auth_env(&cleanup_envs);
            return Err(format!("failed to write terminal PTY: {error}"));
        }
    };
    let process_id = child.process_id();
    let killer = TerminalKiller::new(child.as_ref()).map_err(|error| {
        let _ = child.kill();
        crate::ssh_hosts::cleanup_password_auth_env(&cleanup_envs);
        format!("failed to duplicate terminal process handle: {error}")
    })?;
    let session = Arc::new(TerminalSession {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.into(),
        scope_key: scope_key.into(),
        context_id: context_id.into(),
        title,
        kind: kind.into(),
        display_cwd: spec.display_cwd,
        process_id,
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        output: Mutex::new(TerminalOutputState::new()),
        auth_cleanup_envs: cleanup_envs,
    });
    Ok((session, reader, child))
}

fn start_terminal_workers(
    session: Arc<TerminalSession>,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
) {
    let output_session = Arc::clone(&session);
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => output_session.push_output(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    output_session.push_error(format!("terminal output stopped: {error}"));
                    break;
                }
            }
        }
    });

    std::thread::spawn(move || match child.wait() {
        Ok(status) => session.finish(status.exit_code()),
        Err(error) => {
            session.push_error(format!("failed to wait for terminal process: {error}"));
            session.finish(1);
        }
    });
}

pub fn build_terminal_launch_spec(
    context: &wisp_store::ExecutionContext,
    project_root: &Path,
) -> Result<TerminalLaunchSpec, String> {
    let config: serde_json::Value = serde_json::from_str(&context.config_json).unwrap_or_default();
    match context.kind {
        wisp_store::ExecutionContextKind::Local => Ok(local_launch_spec(project_root)),
        wisp_store::ExecutionContextKind::Wsl => {
            let distro = config
                .get("distro")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| context.id.strip_prefix("wsl:").unwrap_or(&context.id));
            let project = project_root.to_string_lossy().into_owned();
            Ok(TerminalLaunchSpec {
                program: "wsl.exe".into(),
                args: vec!["-d".into(), distro.into(), "--cd".into(), project.clone()],
                cwd: None,
                display_cwd: project,
                envs: Vec::new(),
            })
        }
        wisp_store::ExecutionContextKind::Ssh => {
            let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
            Ok(TerminalLaunchSpec {
                program: "ssh".into(),
                args: connection.interactive_ssh_args()?,
                cwd: None,
                display_cwd: "~".into(),
                envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
            })
        }
    }
}

#[cfg(target_os = "windows")]
fn local_launch_spec(project_root: &Path) -> TerminalLaunchSpec {
    TerminalLaunchSpec {
        program: "powershell.exe".into(),
        args: vec!["-NoLogo".into()],
        cwd: Some(project_root.to_path_buf()),
        display_cwd: project_root.to_string_lossy().into_owned(),
        envs: Vec::new(),
    }
}

#[cfg(not(target_os = "windows"))]
fn local_launch_spec(project_root: &Path) -> TerminalLaunchSpec {
    TerminalLaunchSpec {
        program: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        args: vec!["-l".into()],
        cwd: Some(project_root.to_path_buf()),
        display_cwd: project_root.to_string_lossy().into_owned(),
        envs: Vec::new(),
    }
}

fn append_scrollback(scrollback: &mut Vec<u8>, bytes: &[u8], max_bytes: usize) {
    if max_bytes == 0 {
        scrollback.clear();
        return;
    }
    if bytes.len() >= max_bytes {
        scrollback.clear();
        scrollback.extend_from_slice(&bytes[bytes.len() - max_bytes..]);
        return;
    }
    let overflow = scrollback
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(max_bytes);
    if overflow > 0 {
        scrollback.drain(..overflow);
    }
    scrollback.extend_from_slice(bytes);
}

fn output_event(bytes: &[u8]) -> TerminalEvent {
    TerminalEvent::Output {
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
pub async fn open_terminal(
    app_state: State<'_, crate::AppState>,
    terminals: State<'_, TerminalManager>,
    window: WorkspaceSurface,
    context_id: String,
) -> Result<TerminalSessionSummary, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&app_state, window.label())
            .await?;
    let _activity = app_state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&app_state.store, &scope).await?;
    let context = app_state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    app_state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    terminals.open(&project.id, scope.scope_key(), &project.root, &context)
}

#[tauri::command]
pub fn attach_terminal(
    terminals: State<'_, TerminalManager>,
    session_id: String,
    on_event: Channel<TerminalEvent>,
) -> Result<TerminalSessionSummary, String> {
    let session = terminals.get(&session_id)?;
    session.attach(on_event)?;
    Ok(session.summary())
}

#[tauri::command]
pub fn write_terminal(
    terminals: State<'_, TerminalManager>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    terminals.get(&session_id)?.write(&data)
}

#[tauri::command]
pub fn resize_terminal(
    terminals: State<'_, TerminalManager>,
    session_id: String,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    terminals.get(&session_id)?.resize(rows, cols)
}

#[tauri::command]
pub fn close_terminal(
    terminals: State<'_, TerminalManager>,
    session_id: String,
) -> Result<(), String> {
    terminals.close(&session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn open_test_terminal(manager: &TerminalManager) -> Arc<TerminalSession> {
        let summary = manager
            .open_spec(
                "test-project",
                "main",
                "local",
                "Close regression".into(),
                "local",
                TerminalLaunchSpec {
                    program: "powershell.exe".into(),
                    args: vec![
                        "-NoLogo".into(),
                        "-NoProfile".into(),
                        "-Command".into(),
                        "[Console]::WriteLine('terminal-ready'); Start-Sleep -Seconds 30".into(),
                    ],
                    cwd: None,
                    display_cwd: String::new(),
                    envs: Vec::new(),
                },
            )
            .unwrap();
        let session = manager.get(&summary.id).unwrap();
        // ConPTY asks xterm for the cursor position before launching the shell.
        // This headless test supplies the same terminal response.
        session.write("\u{1b}[1;1R").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !String::from_utf8_lossy(&lock(&session.output).scrollback).contains("terminal-ready")
        {
            assert!(
                std::time::Instant::now() < deadline,
                "terminal did not start: {:?}",
                String::from_utf8_lossy(&lock(&session.output).scrollback)
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        session
    }

    #[test]
    #[cfg(windows)]
    fn windows_terminal_closes_on_first_attempt_and_exited_kill_is_idempotent() {
        let manager = TerminalManager::new();
        let session = open_test_terminal(&manager);
        assert!(session.running());

        manager
            .close(&session.id)
            .expect("first close must succeed");
        assert!(manager.get(&session.id).is_err());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.running() {
            assert!(std::time::Instant::now() < deadline, "child did not exit");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Exercise the handle directly, including the exit race that can
        // precede the waiter's update of TerminalOutputState.
        lock(&session.killer).kill().unwrap();
        session.terminate().unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn windows_terminal_retains_session_on_real_termination_failure() {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

        let manager = TerminalManager::new();
        let session = open_test_terminal(&manager);
        // A valid handle without PROCESS_TERMINATE must report access denied,
        // not silently unregister a still-running shell.
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, session.process_id.unwrap()) }
            .unwrap();
        let restricted = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let original = std::mem::replace(&mut lock(&session.killer).handle, restricted);
        let result = manager.close(&session.id);
        let retained = manager.get(&session.id).is_ok();
        lock(&session.killer).handle = original;
        manager.close(&session.id).unwrap();

        assert!(result.unwrap_err().contains("failed to terminate terminal"));
        assert!(retained);
    }

    #[test]
    fn builds_wsl_terminal_for_selected_distro_and_project() {
        let mut context = wisp_store::ExecutionContext::new("wsl:Ubuntu-24.04", "Ubuntu").unwrap();
        context.config_json = serde_json::json!({"distro": "Ubuntu-24.04"}).to_string();
        let root = Path::new(r"C:\Users\scientist\project");

        let spec = build_terminal_launch_spec(&context, root).unwrap();

        assert_eq!(spec.program, "wsl.exe");
        assert_eq!(
            spec.args,
            ["-d", "Ubuntu-24.04", "--cd", r"C:\Users\scientist\project"]
        );
        assert_eq!(spec.cwd, None);
    }

    #[test]
    fn builds_interactive_ssh_terminal_without_batch_mode() {
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        context.config_json = serde_json::json!({
            "alias": "gpu",
            "user": "alice",
            "port": 2222,
            "identity_file": "/keys/lab key"
        })
        .to_string();

        let spec = build_terminal_launch_spec(&context, Path::new("/local/project")).unwrap();

        assert_eq!(spec.program, "ssh");
        assert_eq!(
            spec.args,
            [
                "-tt",
                "-p",
                "2222",
                "-o",
                "IdentitiesOnly=yes",
                "-i",
                "/keys/lab key",
                "alice@gpu"
            ]
        );
        assert!(!spec.args.iter().any(|arg| arg.contains("BatchMode")));
        assert!(spec.envs.is_empty());
        assert_eq!(spec.display_cwd, "~");
    }

    #[test]
    fn password_ssh_terminal_requires_stored_password_before_spawn() {
        let mut context = wisp_store::ExecutionContext::new("ssh:lab", "Lab").unwrap();
        context.config_json = serde_json::json!({
            "alias": "lab",
            "user": "alice",
            "auth_method": "password"
        })
        .to_string();

        let error = build_terminal_launch_spec(&context, Path::new("/local/project")).unwrap_err();
        assert!(
            error.contains("password"),
            "expected a stored-password error, got {error}"
        );
    }

    #[test]
    fn scrollback_keeps_only_the_newest_bytes() {
        let mut scrollback = b"1234".to_vec();
        append_scrollback(&mut scrollback, b"56789", 6);
        assert_eq!(scrollback, b"456789");

        append_scrollback(&mut scrollback, b"abcdefgh", 4);
        assert_eq!(scrollback, b"efgh");
    }

    #[test]
    fn terminal_events_use_the_javascript_channel_shape() {
        assert_eq!(
            serde_json::to_value(TerminalEvent::Exit { exit_code: 7 }).unwrap(),
            serde_json::json!({"event": "exit", "data": {"exitCode": 7}})
        );
    }
}
