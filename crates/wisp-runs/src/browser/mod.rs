//! What both desktops' real-browser bridges share: the two Browser
//! settings, the human-verification handoff, the extension's version check,
//! the managed copy of the extension (`extension`) and how the user's own
//! browser is started (`launch`).
//!
//! The bridges themselves differ (the Tauri shell's in `src-tauri/src/
//! browser_bridge`, the embedded host's in `wisp-server`), but they speak
//! one protocol to one extension, so the rules that decide what the user is
//! asked, what is refused and when the extension is out of date live here
//! and are called by both.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use wisp_store::Store;

pub mod extension;
pub mod launch;

/// The Wisp extension's stable id, from the key in its manifest.
pub const EXTENSION_ID: &str = "gnkjgagleagkgdlkkcianolobfdoocnp";
/// The only WebSocket origin a bridge accepts.
pub const EXTENSION_ORIGIN: &str = "chrome-extension://gnkjgagleagkgdlkkcianolobfdoocnp";
/// The bridge protocol this build speaks.
pub const REQUIRED_PROTOCOL: i64 = 2;

/// Settings key of **Open browser automatically** (on unless set off).
pub const AUTO_LAUNCH_KEY: &str = "browser_auto_launch";
/// Settings key of **Automatically close browser tabs** (off unless set on).
pub const AUTO_CLOSE_TABS_KEY: &str = "browser_auto_close_tabs";

pub fn parse_auto_launch(raw: Option<&str>) -> bool {
    match raw.map(str::trim) {
        None | Some("") => true,
        Some("false") | Some("0") | Some("off") => false,
        Some(_) => true,
    }
}

pub async fn auto_launch_enabled(store: &Store) -> bool {
    parse_auto_launch(
        store
            .get_setting(AUTO_LAUNCH_KEY)
            .await
            .ok()
            .flatten()
            .as_deref(),
    )
}

pub fn parse_auto_close_tabs(raw: Option<&str>) -> bool {
    match raw.map(str::trim) {
        Some("true") | Some("1") | Some("on") => true,
        Some(_) | None => false,
    }
}

pub async fn auto_close_tabs_enabled(store: &Store) -> bool {
    parse_auto_close_tabs(
        store
            .get_setting(AUTO_CLOSE_TABS_KEY)
            .await
            .ok()
            .flatten()
            .as_deref(),
    )
}

/// A page that asks its visitor to prove they are human: the handoff the
/// scan's result carries, or `None` for any other page. A page that only
/// mentions "Are you a robot?" is not one.
pub fn human_verification_handoff(page: &Value) -> Option<Value> {
    let title = page.get("title").and_then(Value::as_str).unwrap_or("");
    let text = page.get("text").and_then(Value::as_str).unwrap_or("");
    let content = format!("{title}\n{text}").to_ascii_lowercase();
    if !content.contains("are you a robot")
        || !(content.contains("confirm you are a human") || content.contains("captcha challenge"))
    {
        return None;
    }
    Some(json!({
        "required": true,
        "reason": "captcha_challenge",
        "instruction": "A verification prompt has been shown in the Wisp app. Stop browser automation. Do not call ask_user and do not click, solve, or bypass the challenge. End your turn. The user will confirm in the app after completing the challenge in the visible browser tab.",
        "resume": "After the user confirms, a follow-up message arrives. Call web_scan on the same tab and continue only when the challenge is no longer detected."
    }))
}

/// Why a script is refused on a tab that waits for a human.
pub fn needs_human_block_message() -> String {
    "This tab needs human verification. Do not automate the challenge. Wait for the user to complete it in the visible browser tab and confirm in the Wisp app.".into()
}

/// A script parsed as JSON, for the extension commands among scripts.
pub fn json_command(script: &str) -> Option<Value> {
    serde_json::from_str(script.trim()).ok()
}

/// Whether a script is the extension's `tabs create` command.
pub fn is_tabs_create(script: &str) -> bool {
    json_command(script).is_some_and(|value| {
        value.get("cmd").and_then(Value::as_str) == Some("tabs")
            && value.get("method").and_then(Value::as_str) == Some("create")
    })
}

/// The tabs a `tabs close` command names.
pub fn close_ids_from_script(script: &str) -> Option<Vec<i64>> {
    let value = json_command(script)?;
    if value.get("cmd").and_then(Value::as_str) != Some("tabs") {
        return None;
    }
    if value.get("method").and_then(Value::as_str) != Some("close") {
        return None;
    }
    value
        .get("tabIds")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
        .or_else(|| {
            value
                .get("tabId")
                .and_then(Value::as_i64)
                .map(|id| vec![id])
        })
        .filter(|ids| !ids.is_empty())
}

/// What may still be sent while a tab waits for a human: commands that only
/// look at or switch tabs, or pause the bridge.
pub fn script_allowed_during_needs_human(script: &str) -> bool {
    let Some(value) = json_command(script) else {
        return false;
    };
    match value.get("cmd").and_then(Value::as_str) {
        Some("control") => true,
        Some("tabs") => matches!(
            value.get("method").and_then(Value::as_str),
            None | Some("switch") | Some("query") | Some("list")
        ),
        _ => false,
    }
}

/// `rel` under the project `root`, refusing absolute paths and `..`.
pub fn project_relative_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Err("path must be a non-empty project-relative path".into());
    }
    let path = Path::new(trimmed);
    let inside_root = !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !inside_root {
        return Err(format!(
            "path must stay inside the project (no absolute paths or '..' segments): {trimmed}"
        ));
    }
    Ok(root.join(path))
}

/// A Chrome extension version (`major.minor.patch`), for ordering.
fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        // One digit, or digits that do not start with zero.
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        part.parse().ok()
    };
    let triple = (next()?, next()?, next()?);
    parts.next().is_none().then_some(triple)
}

/// Whether `version` is a valid extension version of three numbers.
pub fn valid_version(version: &str) -> bool {
    version_triple(version).is_some()
}

/// The version in a bundled extension's manifest.
pub fn bundled_manifest_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value.get("version")?.as_str().map(str::to_string)
}

/// Whether the connected extension is older than the one this build ships.
/// Versions that cannot be ordered count as outdated when they differ.
pub fn extension_version_outdated(current: &str, bundled: Option<&str>) -> bool {
    let Some(bundled) = bundled else {
        return false;
    };
    match (version_triple(current), version_triple(bundled)) {
        (Some(current), Some(bundled)) => current < bundled,
        _ => current != bundled,
    }
}

/// Whether a connected extension has to be updated: it speaks an older
/// protocol, reports no capabilities (a build before they were reported) or
/// is older than the bundled copy.
pub fn extension_requires_update(
    protocol: i64,
    capabilities: &[String],
    version: &str,
    bundled_version: Option<&str>,
) -> bool {
    protocol < REQUIRED_PROTOCOL
        || capabilities.is_empty()
        || extension_version_outdated(version, bundled_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn are_you_a_robot_page_requires_manual_handoff() {
        let handoff = human_verification_handoff(&json!({
            "title": "Are you a robot?",
            "text": "Please confirm you are a human by completing the captcha challenge below."
        }))
        .unwrap();

        assert_eq!(handoff["required"], true);
        assert_eq!(handoff["reason"], "captcha_challenge");
        assert!(handoff["instruction"]
            .as_str()
            .unwrap()
            .contains("verification prompt"));
        assert!(handoff["instruction"]
            .as_str()
            .unwrap()
            .contains("Do not call ask_user"));
        assert!(human_verification_handoff(&json!({
            "title": "Browser automation article",
            "text": "This article asks: Are you a robot?"
        }))
        .is_none());
        let sciencedirect = human_verification_handoff(&json!({
            "title": "Just a moment...",
            "url": "https://www.sciencedirect.com/science/article/pii/S000",
            "text": "Are you a robot?\nPlease confirm you are a human by completing the captcha challenge below."
        }))
        .unwrap();
        assert_eq!(sciencedirect["reason"], "captcha_challenge");
    }

    #[test]
    fn project_relative_path_rejects_traversal_and_absolute_paths() {
        let root = Path::new("/project");
        assert_eq!(
            project_relative_path(root, "browser-assets/shot.png").unwrap(),
            root.join("browser-assets/shot.png")
        );
        assert_eq!(
            project_relative_path(root, "./figures/a.png").unwrap(),
            root.join("./figures/a.png")
        );
        assert!(project_relative_path(root, "../outside.png").is_err());
        assert!(project_relative_path(root, "a/../../outside.png").is_err());
        assert!(project_relative_path(root, "/etc/passwd").is_err());
        assert!(project_relative_path(root, "").is_err());
        assert!(project_relative_path(root, "   ").is_err());
    }

    #[test]
    fn auto_launch_defaults_on_and_auto_close_defaults_off() {
        assert!(parse_auto_launch(None));
        assert!(parse_auto_launch(Some("")));
        assert!(!parse_auto_launch(Some("false")));
        assert!(!parse_auto_launch(Some("off")));
        assert!(!parse_auto_close_tabs(None));
        assert!(!parse_auto_close_tabs(Some("false")));
        assert!(parse_auto_close_tabs(Some("true")));
        assert!(parse_auto_close_tabs(Some("on")));
    }

    #[test]
    fn only_tab_lookups_pass_while_a_tab_needs_a_human() {
        assert!(script_allowed_during_needs_human(
            r#"{"cmd":"tabs","method":"switch","tabId":3}"#
        ));
        assert!(script_allowed_during_needs_human(
            r#"{"cmd":"control","method":"pause"}"#
        ));
        assert!(!script_allowed_during_needs_human(
            r#"{"cmd":"tabs","method":"close","tabIds":[3]}"#
        ));
        assert!(!script_allowed_during_needs_human("document.title"));
        assert_eq!(
            close_ids_from_script(r#" {"cmd":"tabs","method":"close","tabIds":[3,4]} "#),
            Some(vec![3, 4])
        );
        assert_eq!(
            close_ids_from_script(r#"{"cmd":"tabs","method":"close","tabId":5}"#),
            Some(vec![5])
        );
        assert!(is_tabs_create(
            r#"{"cmd":"tabs","method":"create","url":"https://a"}"#
        ));
        assert_eq!(
            close_ids_from_script(r#"{"cmd":"tabs","method":"switch"}"#),
            None
        );
    }

    #[test]
    fn an_older_or_capability_less_extension_needs_an_update() {
        let caps = vec!["tab_scope".to_string()];
        assert!(!extension_requires_update(2, &caps, "0.5.0", Some("0.5.0")));
        assert!(extension_requires_update(2, &caps, "0.4.9", Some("0.5.0")));
        assert!(extension_requires_update(2, &caps, "0.4.10", Some("0.5.0")));
        assert!(!extension_requires_update(
            2,
            &caps,
            "0.10.0",
            Some("0.5.0")
        ));
        assert!(extension_requires_update(1, &caps, "0.5.0", Some("0.5.0")));
        assert!(extension_requires_update(2, &[], "0.5.0", Some("0.5.0")));
        // Nothing bundled to compare with: only the protocol counts.
        assert!(!extension_requires_update(2, &caps, "0.1.0", None));
        // Versions that cannot be ordered count when they differ.
        assert!(extension_requires_update(2, &caps, "dev", Some("0.5.0")));
        assert!(!valid_version("01.2.3") && !valid_version("1.2") && valid_version("0.5.0"));
    }
}
