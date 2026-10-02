use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Points the workspace session at one browser executable, so a user who keeps
/// Chromium or Chrome for Testing outside the standard install paths can still
/// use workspace mode.
pub const BROWSER_ENV_OVERRIDE: &str = "WISP_WORKSPACE_BROWSER";

/// A Chrome-family build the workspace session can launch.
///
/// `loads_unpacked_extensions` records whether the build still honors
/// `--load-extension`. Official Google Chrome removed that flag in 137 and now
/// only logs `--load-extension is not allowed in Google Chrome, ignoring`, so a
/// workspace window opens with no Wisp extension and can never connect.
/// Chromium and Chrome for Testing keep the flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceBrowser {
    pub name: String,
    pub path: PathBuf,
    pub loads_unpacked_extensions: bool,
}

impl WorkspaceBrowser {
    fn new(name: &str, path: impl Into<PathBuf>, loads_unpacked_extensions: bool) -> Self {
        Self {
            name: name.to_string(),
            path: path.into(),
            loads_unpacked_extensions,
        }
    }
}

pub fn app_data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("science.wisp-science")
}

/// Every build the workspace session knows about, in install-path order. Paths
/// with a single component are looked up on `PATH`.
pub fn browser_candidates() -> Vec<WorkspaceBrowser> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os(BROWSER_ENV_OVERRIDE) {
        // The user named this build on purpose, so trust it to load the copy.
        let path = PathBuf::from(path);
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| BROWSER_ENV_OVERRIDE.to_string());
        candidates.push(WorkspaceBrowser::new(&name, path, true));
    }
    #[cfg(windows)]
    {
        for root in [
            std::env::var_os("LOCALAPPDATA"),
            std::env::var_os("PROGRAMFILES"),
            std::env::var_os("PROGRAMFILES(X86)"),
        ]
        .into_iter()
        .flatten()
        {
            let root = PathBuf::from(root);
            candidates.push(WorkspaceBrowser::new(
                "Chromium",
                root.join(r"Chromium\Application\chrome.exe"),
                true,
            ));
            candidates.push(WorkspaceBrowser::new(
                "Chrome for Testing",
                root.join(r"Google\Chrome for Testing\chrome.exe"),
                true,
            ));
            candidates.push(WorkspaceBrowser::new(
                "Google Chrome",
                root.join(r"Google\Chrome\Application\chrome.exe"),
                false,
            ));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(WorkspaceBrowser::new(
            "Chromium",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            true,
        ));
        candidates.push(WorkspaceBrowser::new(
            "Chrome for Testing",
            "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            true,
        ));
        candidates.push(WorkspaceBrowser::new(
            "Google Chrome",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            false,
        ));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        for name in ["chromium", "chromium-browser"] {
            candidates.push(WorkspaceBrowser::new("Chromium", name, true));
        }
        for name in ["google-chrome-stable", "google-chrome", "chrome"] {
            candidates.push(WorkspaceBrowser::new("Google Chrome", name, false));
        }
    }
    candidates
}

/// Pick the build most likely to load the workspace extension copy. A build
/// that still honors `--load-extension` always wins; branded Chrome is a last
/// resort because only releases older than 137 can work.
pub fn preferred_browser(installed: Vec<WorkspaceBrowser>) -> Option<WorkspaceBrowser> {
    installed
        .iter()
        .find(|browser| browser.loads_unpacked_extensions)
        .or_else(|| installed.first())
        .cloned()
}

pub fn resolve_browser() -> Result<WorkspaceBrowser, String> {
    let installed = browser_candidates()
        .into_iter()
        .filter_map(|browser| {
            locate(&browser.path).map(|path| WorkspaceBrowser { path, ..browser })
        })
        .collect();
    preferred_browser(installed).ok_or_else(|| {
        format!(
            "no Chrome-family browser found for the workspace session; install Chromium or Chrome for Testing, or set {BROWSER_ENV_OVERRIDE} to a browser executable"
        )
    })
}

fn locate(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_path_buf());
    }
    which::which(program).ok()
}

pub fn materialize_extension(
    source: &Path,
    dest: &Path,
    session: &str,
    endpoint: &str,
) -> Result<PathBuf, String> {
    if !source.join("manifest.json").is_file() {
        return Err("bundled browser-extension is missing manifest.json".into());
    }
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|error| format!("clear workspace extension: {error}"))?;
    }
    copy_dir(source, &dest)?;
    let config = format!(
        "// Generated for this project's workspace.\nvar WISP_BRIDGE_CONFIG = {};\n",
        json!({ "session": session, "endpoint": endpoint })
    );
    fs::write(dest.join("session_config.js"), config)
        .map_err(|error| format!("write workspace session_config.js: {error}"))?;
    Ok(dest.to_path_buf())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|error| format!("create {dest:?}: {error}"))?;
    for entry in fs::read_dir(src).map_err(|error| format!("read {src:?}: {error}"))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') || name.ends_with(".test.mjs") {
            continue;
        }
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|error| format!("copy {from:?}: {error}"))?;
        }
    }
    Ok(())
}

pub fn launch_browser(
    browser: &WorkspaceBrowser,
    profile: &Path,
    extension: &Path,
) -> Result<Child, String> {
    fs::create_dir_all(profile).map_err(|error| format!("create workspace profile: {error}"))?;
    let mut command = Command::new(&browser.path);
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--load-extension={}", extension.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--new-window")
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|error| {
        format!(
            "failed to launch workspace {} at '{}': {error}",
            browser.name,
            browser.path.display()
        )
    })
}

#[cfg(windows)]
pub fn terminate(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

/// Why a launched workspace window never produced a bridge connection, and what
/// the user can do instead. Returned to the model so it stops rather than
/// browsing an empty `about:blank` window it cannot see.
pub fn extension_blocked_message(
    browser: &WorkspaceBrowser,
    extension_path: &str,
    endpoint: &str,
    waited: Duration,
) -> String {
    let cause = if browser.loads_unpacked_extensions {
        format!(
            "{} did load the workspace extension copy from '{extension_path}' on the command line, so check that this build is not blocking extensions by policy.",
            browser.name
        )
    } else {
        format!(
            "Official Google Chrome builds since version 137 ignore --load-extension and only log \"--load-extension is not allowed in Google Chrome, ignoring\", so {} cannot load the workspace copy at all and the window stays extension-less.",
            browser.name
        )
    };
    format!(
        "workspace {} at '{}' started but the Wisp extension never connected to {endpoint} within {}s, so Wisp closed that window instead of leaving it on about:blank. {cause} Use the shared session instead: in the browser you already use open chrome://extensions, enable Developer mode, Load unpacked from this exact path '{extension_path}', and wait until the popup shows Connected to Wisp. To keep using workspace mode, install Chromium or Chrome for Testing, or set {BROWSER_ENV_OVERRIDE} to such a build, then call start_workspace again. Do not retry start_workspace with the same browser, and never claim a workspace page was opened or read.",
        browser.name,
        browser.path.display(),
        waited.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_copies_keep_each_projects_connection_config() {
        let root = std::env::temp_dir().join(format!("workspace-copy-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("manifest.json"), "{}").unwrap();
        fs::write(source.join("background.js"), "// test extension").unwrap();
        let a = root.join("a");
        let b = root.join("b");
        materialize_extension(&source, &a, "workspace:a", "ws://127.0.0.1:12345/a").unwrap();
        let a_config = fs::read_to_string(a.join("session_config.js")).unwrap();
        materialize_extension(&source, &b, "workspace:b", "ws://127.0.0.1:12346/b").unwrap();
        assert_eq!(
            fs::read_to_string(a.join("session_config.js")).unwrap(),
            a_config
        );
        assert!(a_config.contains("workspace:a"));
        assert!(!a_config.contains("workspace:b"));
        assert!(fs::read_to_string(b.join("session_config.js"))
            .unwrap()
            .contains("12346/b"));
        assert!(!source.join("session_config.js").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidates_prefer_a_build_that_still_loads_unpacked_extensions() {
        let chrome = WorkspaceBrowser::new("Google Chrome", "/opt/chrome", false);
        let chromium = WorkspaceBrowser::new("Chromium", "/opt/chromium", true);

        assert_eq!(
            preferred_browser(vec![chrome.clone(), chromium.clone()]),
            Some(chromium)
        );
        // Branded Chrome is still tried when it is the only build installed:
        // releases older than 137 honor --load-extension.
        assert_eq!(preferred_browser(vec![chrome.clone()]), Some(chrome));
        assert_eq!(preferred_browser(Vec::new()), None);
    }

    #[test]
    fn branded_chrome_is_never_marked_as_loading_unpacked_extensions() {
        let candidates = browser_candidates();
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .any(|browser| browser.loads_unpacked_extensions));
        for browser in candidates {
            if browser.name == "Google Chrome" {
                assert!(
                    !browser.loads_unpacked_extensions,
                    "branded Chrome 137+ ignores --load-extension: {browser:?}"
                );
            }
        }
    }

    #[test]
    fn blocked_message_names_the_chrome_flag_removal_and_the_shared_fallback() {
        let message = extension_blocked_message(
            &WorkspaceBrowser::new("Google Chrome", "/opt/chrome", false),
            "/opt/wisp/browser-extension",
            "ws://127.0.0.1:12345/test-instance",
            Duration::from_secs(20),
        );

        assert!(message.contains("137"));
        assert!(message.contains("--load-extension"));
        assert!(message.contains("chrome://extensions"));
        assert!(message.contains("/opt/wisp/browser-extension"));
        assert!(message.contains("20s"));
        assert!(message.contains("Do not retry start_workspace"));

        let chromium = extension_blocked_message(
            &WorkspaceBrowser::new("Chromium", "/opt/chromium", true),
            "/opt/wisp/browser-extension",
            "ws://127.0.0.1:12345/test-instance",
            Duration::from_secs(20),
        );
        assert!(!chromium.contains("137"));
        assert!(chromium.contains("blocking extensions by policy"));
    }
}
