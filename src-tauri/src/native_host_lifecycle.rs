//! macOS native-shell ownership and transition into the regular workspace.
//! Both clients share a singleton runtime. A helper must load the real frontend
//! before it can be shown, and must outlive its owner once used as a desktop app.
use crate::workspace_surface::WorkspaceManager;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

pub(crate) struct NativeHostLifecycle {
    headless: AtomicBool,
}

impl NativeHostLifecycle {
    pub(crate) fn new(headless: bool) -> Self {
        Self {
            headless: AtomicBool::new(headless),
        }
    }

    fn is_headless(&self) -> bool {
        self.headless.load(Ordering::SeqCst)
    }

    fn workspace_opened(&self) {
        self.headless.store(false, Ordering::SeqCst);
    }
}

fn workspace_url(dev_url: Option<&tauri::Url>) -> tauri::Url {
    // Match WebviewUrl::App("index.html"): the configured dev URL or the app
    // root. The current document can still be about:blank during startup.
    dev_url
        .cloned()
        .unwrap_or_else(|| "tauri://localhost/".parse().unwrap())
}

/// Called on the main thread, including single-instance notifications. Do not
/// rebuild the runtime or its active conversations when the regular app opens.
pub(crate) fn prepare_workspace(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<NativeHostLifecycle>();
    if !state.is_headless() {
        return Ok(());
    }
    let main = app
        .workspace_surface("main")
        .ok_or("Main workspace is unavailable")?;
    let url = workspace_url(if tauri::is_dev() {
        app.config().build.dev_url.as_ref()
    } else {
        None
    });
    main.webview().navigate(url).map_err(|e| e.to_string())?;
    app.set_activation_policy(tauri::ActivationPolicy::Regular)
        .map_err(|e| e.to_string())?;
    state.workspace_opened();
    tauri::async_runtime::spawn(crate::ui_health::run_watchdog(app.clone()));
    Ok(())
}

fn owner_pipe_requested(args: impl IntoIterator<Item = String>) -> bool {
    let args: Vec<_> = args.into_iter().collect();
    crate::native_settings::requested(args.clone())
        && args
            .iter()
            .any(|arg| arg == "--native-settings-owner-stdin")
}

fn wait_for_owner_end(mut input: impl std::io::Read) -> std::io::Result<()> {
    std::io::copy(&mut input, &mut std::io::sink()).map(|_| ())
}

pub(crate) fn watch_owner(app: &tauri::AppHandle) {
    if !owner_pipe_requested(std::env::args()) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // The SwiftUI process holds the write end until it exits (also on a
        // crash). No PID lookup, global kill, or borrowed-host termination.
        if let Err(error) = wait_for_owner_end(std::io::stdin().lock()) {
            tracing::warn!(%error, "Native host owner pipe closed with an error");
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            // Serialize this decision with workspace activation. A helper that
            // now serves the regular app is no longer owned by the native UI.
            if handle.state::<NativeHostLifecycle>().is_headless() {
                tracing::info!("Native shell exited; shutting down its desktop host");
                handle.exit(0);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicitly_owned_helper_watches_stdin() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(owner_pipe_requested(args(&[
            "wisp",
            "--native-settings-host",
            "--native-settings-owner-stdin"
        ])));
        assert!(!owner_pipe_requested(args(&[
            "wisp",
            "--native-settings-host"
        ])));
        assert!(!owner_pipe_requested(args(&[
            "wisp",
            "--native-settings-owner-stdin"
        ])));
    }

    #[test]
    fn owner_exit_only_shuts_down_a_headless_host() {
        let helper = NativeHostLifecycle::new(true);
        assert!(helper.is_headless());
        helper.workspace_opened();
        assert!(
            !helper.is_headless(),
            "Opening the regular app transfers lifetime ownership"
        );
        helper.workspace_opened();
        assert!(
            !helper.is_headless(),
            "Repeated activation must not restore native ownership"
        );
        assert!(
            !NativeHostLifecycle::new(false).is_headless(),
            "A borrowed regular app must keep running"
        );
    }

    #[test]
    fn activation_loads_the_frontend_on_the_same_origin() {
        assert_eq!(workspace_url(None).as_str(), "tauri://localhost/");
        for dev in ["http://localhost:1421/", "http://localhost:1421/base/"] {
            assert_eq!(workspace_url(Some(&dev.parse().unwrap())).as_str(), dev);
        }
    }

    #[test]
    fn owner_pipe_waits_until_eof_and_propagates_read_failure() {
        assert!(wait_for_owner_end(std::io::Cursor::new(b"keepalive")).is_ok());
        struct FailedPipe;
        impl std::io::Read for FailedPipe {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "owner gone",
                ))
            }
        }
        assert_eq!(
            wait_for_owner_end(FailedPipe).unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}
