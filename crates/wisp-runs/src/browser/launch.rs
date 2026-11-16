//! Starting the user's own Chrome, Chromium, Edge or Brave: with its normal
//! profile on a new tab, so the unpacked Wisp extension can reconnect
//! (**Open browser automatically**), or on its extension-manager page (the
//! update banner's **Open extension page**). Never a temporary automation
//! profile, and never from a test: the bridges take a launcher they can
//! replace.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Start the user's existing Chrome/Chromium/Edge so the unpacked Wisp
/// extension can reconnect. Opens `url`, else the new-tab page.
pub fn spawn_user_browser(url: Option<&str>) -> Result<(), String> {
    let (program, args) = first_available_browser()
        .ok_or_else(|| "no Chrome, Chromium, or Edge browser was found".to_string())?;
    let mut args = shared_browser_launch_args(&program, args);
    if let Some(url) = url {
        *args
            .last_mut()
            .expect("shared launch always includes a URL") = url.into();
    }
    spawn_browser_with_url(&program, &args)
}

/// The arguments that open the browser's new-tab page in its normal profile.
pub fn shared_browser_launch_args(program: &Path, mut args: Vec<String>) -> Vec<String> {
    if program.ends_with("open") {
        args.push("--args".into());
    }
    args.push(format!(
        "{}/",
        extensions_page_url(program, &args).replace("extensions", "newtab")
    ));
    args
}

pub fn first_available_browser() -> Option<(PathBuf, Vec<String>)> {
    browser_launch_candidates()
        .into_iter()
        .find(|(program, args)| launch_plan_available(program, args))
}

/// Launch plan for the first real browser executable, pointed at its
/// extension manager page. The `cmd /C start` fallback is skipped: it cannot
/// carry the `*://extensions` URL.
pub fn browser_extension_page_plan() -> Option<(PathBuf, Vec<String>)> {
    browser_launch_candidates()
        .into_iter()
        .filter(|(program, _)| program.file_name().and_then(|name| name.to_str()) != Some("cmd"))
        .find(|(program, args)| launch_plan_available(program, args))
        .map(|(program, args)| {
            let args = extension_page_launch_args(&program, args);
            (program, args)
        })
}

/// Opens the first browser found on its extension-manager page. Whether one
/// was started.
pub fn open_extension_page() -> bool {
    browser_extension_page_plan().is_some_and(|(program, args)| {
        spawn_browser_with_url(&program, &args)
            .map_err(|error| {
                tracing::warn!(target: "wisp", "extension page launch failed: {error}");
            })
            .is_ok()
    })
}

/// `open -a App` needs `--args` before `chrome://…`; Chromium executables take
/// the URL as a trailing argument.
pub fn extension_page_launch_args(program: &Path, mut args: Vec<String>) -> Vec<String> {
    let url = extensions_page_url(program, &args);
    if program.ends_with("open") {
        args.push("--args".into());
    }
    args.push(url.to_string());
    args
}

/// Each Chromium fork only understands its own `*://extensions` scheme.
pub fn extensions_page_url(program: &Path, args: &[String]) -> &'static str {
    let name = if program.ends_with("open") {
        // macOS `open -a "<App Name>"`: the browser identity sits in the args.
        args.get(1).map(String::as_str).unwrap_or_default()
    } else {
        program
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
    }
    .to_lowercase();
    if name.contains("edge") {
        "edge://extensions"
    } else if name.contains("brave") {
        "brave://extensions"
    } else {
        "chrome://extensions"
    }
}

/// Start `program` detached, with caller-chosen args.
pub fn spawn_browser_with_url(program: &Path, args: &[String]) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x00000200);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to start {}: {error}", program.display()))
}

/// OS-specific launch plans. The first available program is used at runtime.
pub fn browser_launch_candidates() -> Vec<(PathBuf, Vec<String>)> {
    #[cfg(windows)]
    {
        let mut candidates = Vec::new();
        for root in [
            std::env::var_os("LOCALAPPDATA"),
            std::env::var_os("PROGRAMFILES"),
            std::env::var_os("PROGRAMFILES(X86)"),
        ]
        .into_iter()
        .flatten()
        {
            let root = PathBuf::from(root);
            candidates.push((
                root.join("Google/Chrome/Application/chrome.exe"),
                Vec::new(),
            ));
            candidates.push((
                root.join("Microsoft/Edge/Application/msedge.exe"),
                Vec::new(),
            ));
            candidates.push((
                root.join("BraveSoftware/Brave-Browser/Application/brave.exe"),
                Vec::new(),
            ));
        }
        candidates.push((
            PathBuf::from("cmd"),
            vec!["/C".into(), "start".into(), String::new(), "chrome".into()],
        ));
        candidates
    }
    #[cfg(target_os = "macos")]
    {
        [
            "Google Chrome",
            "Microsoft Edge",
            "Chromium",
            "Brave Browser",
        ]
        .into_iter()
        .map(|name| {
            (
                PathBuf::from("/usr/bin/open"),
                vec!["-a".into(), name.into()],
            )
        })
        .collect()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        [
            "google-chrome-stable",
            "google-chrome",
            "chromium-browser",
            "chromium",
            "microsoft-edge-stable",
            "microsoft-edge",
            "brave-browser",
        ]
        .into_iter()
        .map(|name| (PathBuf::from(name), Vec::new()))
        .collect()
    }
}

pub fn launch_plan_available(
    program: &Path,
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] args: &[String],
) -> bool {
    #[cfg(target_os = "macos")]
    {
        if program.ends_with("open") && args.first().map(String::as_str) == Some("-a") {
            return args.get(1).is_some_and(|name| {
                Path::new("/Applications")
                    .join(format!("{name}.app"))
                    .is_dir()
            });
        }
    }
    if program.components().count() > 1 {
        return program.is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
            || cfg!(windows) && dir.join(format!("{}.exe", program.display())).is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_page_url_matches_the_browser_scheme() {
        // Windows/Linux executables identify the browser by file stem.
        assert_eq!(
            extensions_page_url(Path::new("chrome"), &[]),
            "chrome://extensions"
        );
        assert_eq!(
            extensions_page_url(Path::new("chromium"), &[]),
            "chrome://extensions"
        );
        assert_eq!(
            extensions_page_url(Path::new("msedge"), &[]),
            "edge://extensions"
        );
        assert_eq!(
            extensions_page_url(Path::new("microsoft-edge-stable"), &[]),
            "edge://extensions"
        );
        assert_eq!(
            extensions_page_url(Path::new("brave"), &[]),
            "brave://extensions"
        );
        // macOS goes through `open -a "<App Name>"`, so the identity is an arg.
        let open = Path::new("/usr/bin/open");
        assert_eq!(
            extensions_page_url(open, &["-a".into(), "Google Chrome".into()]),
            "chrome://extensions"
        );
        assert_eq!(
            extensions_page_url(open, &["-a".into(), "Microsoft Edge".into()]),
            "edge://extensions"
        );
        assert_eq!(
            extensions_page_url(open, &["-a".into(), "Brave Browser".into()]),
            "brave://extensions"
        );
        assert_eq!(
            extension_page_launch_args(open, vec!["-a".into(), "Google Chrome".into()]),
            vec![
                "-a".to_string(),
                "Google Chrome".into(),
                "--args".into(),
                "chrome://extensions".into(),
            ]
        );
        assert_eq!(
            extension_page_launch_args(Path::new("msedge"), Vec::new()),
            vec!["edge://extensions".to_string()]
        );
    }

    #[test]
    fn shared_launch_uses_normal_profile_and_new_tab_on_each_platform() {
        for (program, args, expected) in [
            ("chrome.exe", vec![], vec!["chrome://newtab/"]),
            ("msedge.exe", vec![], vec!["edge://newtab/"]),
            (
                "/usr/bin/open",
                vec!["-a", "Google Chrome"],
                vec!["-a", "Google Chrome", "--args", "chrome://newtab/"],
            ),
        ] {
            let actual = shared_browser_launch_args(
                Path::new(program),
                args.into_iter().map(String::from).collect(),
            );
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn launch_candidates_name_a_real_chrome_family_browser() {
        let candidates = browser_launch_candidates();
        assert!(!candidates.is_empty());
        let rendered: Vec<String> = candidates
            .iter()
            .map(|(program, args)| format!("{} {}", program.display(), args.join(" ")))
            .collect();
        let blob = rendered.join("\n").to_lowercase();
        assert!(
            blob.contains("chrome") || blob.contains("chromium") || blob.contains("msedge"),
            "expected a Chrome-family launch plan, got {blob}"
        );
        #[cfg(windows)]
        assert!(blob.contains("chrome.exe") || blob.contains("start"));
        #[cfg(target_os = "macos")]
        assert!(blob.contains("open") && blob.contains("google chrome"));
    }
}
