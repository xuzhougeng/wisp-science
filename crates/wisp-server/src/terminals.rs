//! Terminals drawn in the app, a port of the WebView's
//! `terminal_sessions.rs`: each is a PTY (ConPTY on Windows) running a
//! shell on this computer in the project's folder, or `ssh` to a saved host
//! or a WSL distribution, as the environment's launch says. The window reads
//! what it wrote by offset (the shared `TerminalOutput` of the native
//! clients), sends keys, resizes and closes it. Closing one stops it; all
//! are stopped when the host shuts down. Output is kept up to 4 MB, so a
//! window that comes back to a terminal redraws it from there.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, ensure, Context as _, Result};
use base64::Engine;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use wisp_dto::native_conversations::{TerminalInfo, TerminalOutput};

use crate::Server;

const DEFAULT_ROWS: u16 = 30;
const DEFAULT_COLS: u16 = 100;
const MAX_SCROLLBACK_BYTES: usize = 4 * 1024 * 1024;
/// The most terminals open at once, across projects.
const MAX_TERMINALS: usize = 16;

/// What a terminal runs, where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) display_cwd: String,
    pub(crate) envs: Vec<(String, String)>,
}

struct Output {
    scrollback: Vec<u8>,
    total: u64,
    exit: Option<u32>,
}

struct Session {
    id: String,
    project: String,
    context: String,
    title: String,
    display_cwd: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<Output>,
    changed: tokio::sync::Notify,
    /// One-shot askpass files for a stored SSH password, removed when the
    /// terminal ends.
    cleanup: Vec<(String, String)>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Session {
    fn running(&self) -> bool {
        lock(&self.output).exit.is_none()
    }

    fn info(&self) -> TerminalInfo {
        TerminalInfo {
            id: self.id.clone(),
            project_id: self.project.clone(),
            context_id: self.context.clone(),
            title: self.title.clone(),
            kind: "terminal".into(),
            display_cwd: self.display_cwd.clone(),
            running: self.running(),
        }
    }

    fn push(&self, bytes: &[u8]) {
        let mut output = lock(&self.output);
        output.total += bytes.len() as u64;
        append_scrollback(&mut output.scrollback, bytes, MAX_SCROLLBACK_BYTES);
        drop(output);
        self.changed.notify_waiters();
    }

    fn finish(&self, code: u32) {
        let mut output = lock(&self.output);
        if output.exit.replace(code).is_none() {
            wisp_runs::ssh_hosts::cleanup_password_auth_env(&self.cleanup);
        }
        drop(output);
        self.changed.notify_waiters();
    }

    fn stop(&self) {
        if self.running() {
            // Best effort: a terminal that already ended has nothing to stop.
            let _ = lock(&self.killer).kill();
        }
    }

    /// What it wrote from `cursor` on; from the oldest kept byte when the
    /// cursor is none, too old or past the end.
    fn read(&self, cursor: Option<u64>) -> TerminalOutput {
        let output = lock(&self.output);
        let oldest = output.total - output.scrollback.len() as u64;
        let reset = cursor.is_none_or(|cursor| cursor < oldest || cursor > output.total);
        let start = if reset {
            oldest
        } else {
            cursor.unwrap_or(oldest)
        };
        let bytes = &output.scrollback[(start - oldest) as usize..];
        TerminalOutput {
            terminal_id: self.id.clone(),
            start,
            end: output.total,
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            reset,
            exit_code: output.exit,
        }
    }
}

/// The host's open terminals.
#[derive(Default)]
pub(crate) struct Terminals {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Terminals {
    fn get(&self, project: &str, id: &str) -> Result<Arc<Session>> {
        let session = lock(&self.sessions)
            .get(id)
            .cloned()
            .context("That terminal is closed")?;
        ensure!(
            session.project == project,
            "That terminal belongs to another project"
        );
        Ok(session)
    }

    /// Stops every terminal; the host is shutting down.
    pub(crate) fn stop_all(&self) {
        let sessions: Vec<_> = lock(&self.sessions).drain().map(|(_, s)| s).collect();
        for session in sessions {
            session.stop();
            wisp_runs::ssh_hosts::cleanup_password_auth_env(&session.cleanup);
        }
    }
}

/// Keeps at most `max_bytes` of the newest output.
fn append_scrollback(scrollback: &mut Vec<u8>, bytes: &[u8], max_bytes: usize) {
    if bytes.len() >= max_bytes {
        scrollback.clear();
        scrollback.extend_from_slice(&bytes[bytes.len() - max_bytes..]);
        return;
    }
    let overflow = (scrollback.len() + bytes.len()).saturating_sub(max_bytes);
    if overflow > 0 {
        scrollback.drain(..overflow);
    }
    scrollback.extend_from_slice(bytes);
}

/// This computer's shell, in `cwd`.
pub(crate) fn local_launch(cwd: &Path) -> Launch {
    let (program, args) = if cfg!(windows) {
        ("powershell.exe".to_owned(), vec!["-NoLogo".to_owned()])
    } else {
        (
            std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".into()),
            vec!["-l".to_owned()],
        )
    };
    Launch {
        program,
        args,
        cwd: Some(cwd.to_path_buf()),
        display_cwd: cwd.to_string_lossy().into_owned(),
        envs: Vec::new(),
    }
}

fn spawn(
    project: &str,
    context: &str,
    title: String,
    launch: Launch,
) -> Result<(
    Arc<Session>,
    Box<dyn Read + Send>,
    Box<dyn portable_pty::Child + Send + Sync>,
)> {
    let cleanup = launch.envs.clone();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| anyhow::anyhow!("Could not make a terminal: {error}"))?;
    let mut command = CommandBuilder::new(&launch.program);
    command.args(&launch.args);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for (key, value) in &launch.envs {
        command.env(key, value);
    }
    if let Some(cwd) = &launch.cwd {
        command.cwd(cwd);
    }
    let started = pair.slave.spawn_command(command);
    drop(pair.slave);
    let child = match started {
        Ok(child) => child,
        Err(error) => {
            wisp_runs::ssh_hosts::cleanup_password_auth_env(&cleanup);
            bail!("Could not start {}: {error}", launch.program);
        }
    };
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow::anyhow!("Could not read the terminal: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| anyhow::anyhow!("Could not write to the terminal: {error}"))?;
    let killer = child.clone_killer();
    let session = Arc::new(Session {
        id: uuid::Uuid::new_v4().to_string(),
        project: project.into(),
        context: context.into(),
        title,
        display_cwd: launch.display_cwd,
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        output: Mutex::new(Output {
            scrollback: Vec::new(),
            total: 0,
            exit: None,
        }),
        changed: tokio::sync::Notify::new(),
        cleanup,
    });
    Ok((session, reader, child))
}

/// Copies what the terminal writes into its scrollback, and records how it
/// ended.
fn watch(
    session: Arc<Session>,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
) {
    let output = Arc::clone(&session);
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => output.push(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    std::thread::spawn(move || {
        let code = child.wait().map_or(1, |status| status.exit_code());
        session.finish(code);
    });
}

impl Server {
    /// Opens a terminal for `project`: this computer's shell in the
    /// project's folder (or `folder` inside it), or, for a saved SSH host or
    /// WSL distribution, what its environment launches (in `folder` there).
    pub async fn open_terminal(
        &self,
        project: &str,
        context: Option<&str>,
        folder: Option<&str>,
    ) -> Result<TerminalInfo> {
        let open = lock(&self.0.terminals.sessions).len();
        ensure!(
            open < MAX_TERMINALS,
            "There are already {MAX_TERMINALS} terminals open; close one first"
        );
        let root = PathBuf::from(self.project_root(project).await?);
        let context = context
            .map(str::trim)
            .filter(|context| !context.is_empty() && *context != wisp_runtime::LOCAL_CONTEXT_ID);
        let (context_id, title, launch) = match context {
            None => {
                let cwd = match folder.map(str::trim).filter(|folder| !folder.is_empty()) {
                    Some(folder) => {
                        let inside = Path::new(folder)
                            .components()
                            .all(|part| matches!(part, std::path::Component::Normal(_)));
                        ensure!(inside, "A terminal opens inside the project's folder");
                        root.join(folder)
                    }
                    None => root.clone(),
                };
                ensure!(cwd.is_dir(), "The folder {} is not there", cwd.display());
                (
                    wisp_runtime::LOCAL_CONTEXT_ID.to_owned(),
                    "This computer".to_owned(),
                    local_launch(&cwd),
                )
            }
            Some(context) => {
                let saved = self
                    .0
                    .store
                    .get_execution_context(context)
                    .await?
                    .context("That environment is no longer saved")?;
                let started = self.terminal_launch(project, context, folder).await?;
                let mut envs = Vec::new();
                if saved.kind == wisp_store::ExecutionContextKind::Ssh {
                    wisp_runs::openssh::require_local_openssh().map_err(anyhow::Error::msg)?;
                    let connection =
                        wisp_runs::ssh_hosts::SshConnection::from_execution_context(&saved)
                            .map_err(anyhow::Error::msg)?;
                    // A stored password answers the prompt; otherwise the
                    // terminal asks, as a terminal does.
                    if connection.uses_password() {
                        envs = wisp_runs::ssh_hosts::auth_envs_for_connection(&connection)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                let label = if saved.label.trim().is_empty() {
                    saved.id.clone()
                } else {
                    saved.label.clone()
                };
                (
                    saved.id.clone(),
                    label,
                    Launch {
                        program: started.program,
                        args: started.args,
                        cwd: None,
                        display_cwd: folder.unwrap_or("~").to_owned(),
                        envs,
                    },
                )
            }
        };
        self.start_terminal(project, &context_id, title, launch)
    }

    /// Starts `launch` as a terminal of `project`.
    pub(crate) fn start_terminal(
        &self,
        project: &str,
        context: &str,
        title: String,
        launch: Launch,
    ) -> Result<TerminalInfo> {
        let (session, reader, child) = spawn(project, context, title, launch)?;
        let info = session.info();
        lock(&self.0.terminals.sessions).insert(session.id.clone(), Arc::clone(&session));
        watch(session, reader, child);
        Ok(info)
    }

    /// The project's open terminals, oldest first.
    pub fn terminals(&self, project: &str) -> Vec<TerminalInfo> {
        let sessions = lock(&self.0.terminals.sessions);
        let mut open: Vec<_> = sessions
            .values()
            .filter(|session| session.project == project)
            .map(|session| session.info())
            .collect();
        drop(sessions);
        open.sort_by(|a, b| a.id.cmp(&b.id));
        open
    }

    /// What a terminal wrote from `cursor` on. With nothing new yet and the
    /// terminal still running, waits up to `wait` for more.
    pub async fn read_terminal(
        &self,
        project: &str,
        id: &str,
        cursor: Option<u64>,
        wait: Duration,
    ) -> Result<TerminalOutput> {
        let session = self.0.terminals.get(project, id)?;
        let changed = session.changed.notified();
        tokio::pin!(changed);
        // Registered before looking, so nothing written in between is missed.
        changed.as_mut().enable();
        let now = session.read(cursor);
        if now.end != now.start || now.reset || now.exit_code.is_some() || wait.is_zero() {
            return Ok(now);
        }
        let _ = tokio::time::timeout(wait, changed).await;
        Ok(session.read(cursor))
    }

    /// Types `bytes` into a terminal.
    pub fn write_terminal(&self, project: &str, id: &str, bytes: &[u8]) -> Result<()> {
        let session = self.0.terminals.get(project, id)?;
        ensure!(session.running(), "That terminal has ended");
        let mut writer = lock(&session.writer);
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .context("Could not type into the terminal")
    }

    /// Gives a terminal `rows` by `cols` characters.
    pub fn resize_terminal(&self, project: &str, id: &str, rows: u16, cols: u16) -> Result<()> {
        ensure!(
            (2..=500).contains(&rows) && (2..=1000).contains(&cols),
            "A terminal is 2 to 500 rows and 2 to 1000 columns"
        );
        let session = self.0.terminals.get(project, id)?;
        let resized = lock(&session.master).resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        resized.map_err(|error| anyhow::anyhow!("Could not resize the terminal: {error}"))
    }

    /// Stops a terminal and forgets it.
    pub fn close_terminal(&self, project: &str, id: &str) -> Result<()> {
        let session = self.0.terminals.get(project, id)?;
        session.stop();
        lock(&self.0.terminals.sessions).remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_keeps_the_newest_bytes() {
        let mut kept = Vec::new();
        append_scrollback(&mut kept, b"abcdef", 4);
        assert_eq!(kept, b"cdef");
        append_scrollback(&mut kept, b"gh", 4);
        assert_eq!(kept, b"efgh");
        append_scrollback(&mut kept, b"i", 8);
        assert_eq!(kept, b"efghi");
    }

    #[test]
    fn this_computers_shell_opens_where_it_is_asked() {
        let launch = local_launch(Path::new("/tmp/project"));
        assert_eq!(launch.cwd.as_deref(), Some(Path::new("/tmp/project")));
        assert!(!launch.program.is_empty());
        if !cfg!(windows) {
            // A login shell, so the researcher's own setup applies.
            assert_eq!(launch.args, ["-l"]);
        }
    }
}
