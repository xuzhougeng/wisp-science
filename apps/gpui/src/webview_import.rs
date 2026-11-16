//! Bringing the WebView desktop's projects into this one: where its
//! database is, which of its projects can come, and what to say about each
//! and about how it went.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::i18n::{fill, tr};
use wisp_server::desktop::{WebViewImport, WebViewProject};

/// Where the WebView desktop keeps its database under `data`, the
/// system's application data folder, as Tauri names its folder there.
pub fn database_in(data: &Path) -> PathBuf {
    data.join("science.wisp-science")
        .join("wisp-science")
        .join("wisp.sqlite")
}

/// Where the WebView desktop keeps its database on this computer.
pub fn database() -> Option<PathBuf> {
    dirs::data_dir().map(|data| database_in(&data))
}

/// Why a project cannot be brought over, when it cannot.
pub fn hold(project: &WebViewProject) -> Option<&'static str> {
    if project.here {
        Some(tr("Already here"))
    } else if !project.folder_found {
        Some(tr("Its folder is not on this computer"))
    } else {
        None
    }
}

/// The projects ticked when the list is first shown: every one that can
/// come.
pub fn first_picks(found: &[WebViewProject]) -> BTreeSet<String> {
    found
        .iter()
        .filter(|project| hold(project).is_none())
        .map(|project| project.id.clone())
        .collect()
}

/// The ticked projects in the order they are listed.
pub fn chosen(found: &[WebViewProject], picks: &BTreeSet<String>) -> Vec<String> {
    found
        .iter()
        .filter(|project| picks.contains(&project.id) && hold(project).is_none())
        .map(|project| project.id.clone())
        .collect()
}

/// The line under a project's name: its conversations, or that it keeps
/// its own database and so is opened where it is.
pub fn detail(project: &WebViewProject) -> String {
    match project.conversations {
        _ if project.own_database => tr("Keeps its own database: opened where it is").into(),
        Some(count) => crate::organise::conversations(count),
        None => String::new(),
    }
}

/// What a finished import says, before the reasons for any left behind.
pub fn summary(done: &[WebViewImport]) -> String {
    let brought = done.iter().filter(|done| done.error.is_none()).count();
    match (brought, done.len()) {
        (0, _) => tr("Nothing was brought over.").into(),
        (1, 1) => tr("The project was brought over.").into(),
        (brought, all) if brought == all => {
            fill(tr("All {} projects were brought over."), &[&brought])
        }
        (brought, all) => fill(
            tr("{} of {} projects were brought over."),
            &[&brought, &all],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, here: bool, folder_found: bool) -> WebViewProject {
        WebViewProject {
            id: id.into(),
            name: id.into(),
            folder: format!("/research/{id}"),
            conversations: Some(2),
            own_database: false,
            starred: false,
            here,
            folder_found,
        }
    }

    fn done(id: &str, error: Option<&str>) -> WebViewImport {
        WebViewImport {
            id: id.into(),
            name: id.into(),
            error: error.map(String::from),
        }
    }

    #[test]
    fn the_webview_database_is_where_tauri_keeps_the_app_data() {
        assert_eq!(
            database_in(Path::new("/data")),
            Path::new("/data/science.wisp-science/wisp-science/wisp.sqlite")
        );
    }

    #[test]
    fn only_projects_not_here_with_their_folder_can_be_chosen() {
        let found = [
            project("new", false, true),
            project("here", true, true),
            project("moved", false, false),
            project("other", false, true),
        ];
        assert_eq!(hold(&found[0]), None);
        assert_eq!(hold(&found[1]), Some("Already here"));
        assert_eq!(hold(&found[2]), Some("Its folder is not on this computer"));
        let picks = first_picks(&found);
        assert_eq!(
            picks.iter().map(String::as_str).collect::<Vec<_>>(),
            ["new", "other"]
        );
        // Listed order, and a held one is never sent even if ticked.
        let ticked: BTreeSet<String> = ["other", "here", "new"].map(String::from).into();
        assert_eq!(chosen(&found, &ticked), ["new", "other"]);
    }

    #[test]
    fn a_project_says_what_it_brings() {
        let mut row = project("lab", false, true);
        assert_eq!(detail(&row), "2 conversations");
        row.own_database = true;
        row.conversations = None;
        assert_eq!(detail(&row), "Keeps its own database: opened where it is");
    }

    #[test]
    fn the_summary_counts_what_came() {
        assert_eq!(summary(&[done("a", None)]), "The project was brought over.");
        assert_eq!(
            summary(&[done("a", None), done("b", None)]),
            "All 2 projects were brought over."
        );
        assert_eq!(
            summary(&[done("a", None), done("b", Some("It is already here"))]),
            "1 of 2 projects were brought over."
        );
        assert_eq!(
            summary(&[done("a", Some("gone"))]),
            "Nothing was brought over."
        );
    }
}
