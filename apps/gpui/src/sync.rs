//! How project sync reads in project settings: where a project's sync
//! stands, and what a sync just did.
use crate::{
    i18n::{fill, tr},
    present::Tone,
};
use wisp_server::desktop::{ProjectSyncInfo, ProjectSyncResult};

/// Seconds as "3 minutes ago"-style words, at `now`.
fn ago(then: i64, now: i64) -> String {
    let seconds = (now - then).max(0);
    match seconds {
        0..60 => tr("just now").to_owned(),
        60..3_600 => fill(tr("{} min ago"), &[&(seconds / 60)]),
        3_600..86_400 => fill(tr("{} h ago"), &[&(seconds / 3_600)]),
        _ => fill(tr("{} days ago"), &[&(seconds / 86_400)]),
    }
}

/// The project's sync in a line, and its colour.
pub fn sync_line(info: Option<&ProjectSyncInfo>, now: i64) -> (String, Tone) {
    let Some(info) = info else {
        return (
            tr("Not synchronized. Sync now publishes it to the relay or folder set in Settings, Remote access.")
                .to_owned(),
            Tone::Neutral,
        );
    };
    if info.kind == "workspace" {
        return match info.folder_status.as_deref() {
            Some("remote-newer") => (
                tr("Saved in its cloud-drive folder · another device saved a newer version")
                    .to_owned(),
                Tone::Attention,
            ),
            Some("unpublished") => (
                tr("Saved in its cloud-drive folder · changes here are not saved there yet")
                    .to_owned(),
                Tone::Active,
            ),
            _ => (
                tr("Saved in its cloud-drive folder").to_owned(),
                Tone::Success,
            ),
        };
    }
    if !info.synced {
        return (tr("Not synchronized yet").to_owned(), Tone::Neutral);
    }
    let place = if info.kind == "folder" {
        tr("through a shared folder")
    } else {
        tr("through the relay")
    };
    match info.last_synced_at {
        Some(at) => (
            fill(tr("Synchronized {}, {}"), &[&place, &ago(at, now)]),
            Tone::Success,
        ),
        None => (fill(tr("Synchronized {}"), &[&place]), Tone::Success),
    }
}

/// What a sync did, in a sentence.
pub fn result_note(result: &ProjectSyncResult) -> String {
    let said = match (result.status.as_str(), result.direction.as_str()) {
        (_, "pull") | ("pulled", _) => tr("Brought in the other device's changes."),
        ("up-to-date", _) | (_, "none") => tr("Already up to date."),
        ("recovered", _) => tr("The last sync had reached the relay; it is recorded now."),
        _ => tr("Published this device's changes."),
    }
    .to_owned();
    match result.skipped_paths.len() {
        0 => said,
        count => format!(
            "{said} {}",
            fill(
                tr("{} file(s) stay on this device: too large or not portable."),
                &[&count]
            )
        ),
    }
}

/// Whether a sync was refused because both devices changed the project.
pub fn is_conflict(error: &str) -> bool {
    error.contains("Sync conflict")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: &str, synced: bool, at: Option<i64>, folder: Option<&str>) -> ProjectSyncInfo {
        ProjectSyncInfo {
            kind: kind.into(),
            synced,
            last_synced_at: at,
            last_direction: None,
            folder_status: folder.map(str::to_owned),
        }
    }

    fn result(status: &str, direction: &str, skipped: usize) -> ProjectSyncResult {
        ProjectSyncResult {
            status: status.into(),
            direction: direction.into(),
            revision: None,
            uploaded_files: 0,
            downloaded_files: 0,
            skipped_paths: vec!["x".into(); skipped],
        }
    }

    #[test]
    fn the_line_says_how_and_when_a_project_was_synchronized() {
        assert!(sync_line(None, 0).0.starts_with("Not synchronized."));
        let now = 10_000;
        assert_eq!(
            sync_line(Some(&info("relay", true, Some(now - 120), None)), now).0,
            "Synchronized through the relay, 2 min ago"
        );
        assert_eq!(
            sync_line(Some(&info("folder", true, Some(now - 5), None)), now).0,
            "Synchronized through a shared folder, just now"
        );
        let (line, tone) = sync_line(
            Some(&info("workspace", true, None, Some("remote-newer"))),
            now,
        );
        assert!(line.contains("newer version"));
        assert!(matches!(tone, Tone::Attention));
    }

    #[test]
    fn a_sync_says_what_it_did_and_what_stayed_here() {
        assert_eq!(
            result_note(&result("synced", "push", 0)),
            "Published this device's changes."
        );
        assert_eq!(
            result_note(&result("synced", "pull", 2)),
            "Brought in the other device's changes. 2 file(s) stay on this device: too large or not portable."
        );
        assert_eq!(
            result_note(&result("up-to-date", "none", 0)),
            "Already up to date."
        );
        assert!(is_conflict(
            "Sync conflict: this device and another device both changed the project."
        ));
        assert!(!is_conflict(
            "Configure a sync relay URL in Settings first."
        ));
    }
}
