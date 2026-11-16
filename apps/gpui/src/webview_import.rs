//! Bringing the WebView desktop's projects and settings into this one:
//! where its database is, which of its projects can come, what each group
//! of its settings is called, and what to say about how it went.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::i18n::{fill, tr};
use wisp_server::desktop::{WebViewOutcome, WebViewProject};

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

/// What a group of the WebView's settings is called here.
pub fn setting_label(group: &str) -> &'static str {
    match group {
        "network" => tr("Network routes and mirrors"),
        "skills" => tr("Skills switched off, and skill tags"),
        "connectors" => tr("Connectors switched off"),
        "experts" => tr("Experts"),
        "acp_agents" => tr("ACP agents"),
        "hooks" => tr("Command hooks"),
        "credentials" => tr("Your credentials (their values stay in the keychain)"),
        "browser" => tr("Browser address rules"),
        "review" => tr("Reviewing turns"),
        "failure_analysis" => tr("Analysing failures"),
        "compaction" => tr("Automatic compaction"),
        "continuing" => tr("Continuing cut-off answers"),
        "memory" => tr("Memory"),
        "follow_ups" => tr("Suggesting what to ask next"),
        "update_check" => tr("Checking for updates"),
        "pet" => tr("The pet"),
        _ => tr("Other settings"),
    }
}

/// What a finished import says, before the reasons for any project left
/// behind.
pub fn summary(outcome: &WebViewOutcome) -> String {
    let done = &outcome.projects;
    let brought = done.iter().filter(|done| done.error.is_none()).count();
    let projects = match (brought, done.len()) {
        (_, 0) => None,
        (0, _) => Some(tr("No project was brought over.").to_owned()),
        (1, 1) => Some(tr("The project was brought over.").to_owned()),
        (brought, all) if brought == all => {
            Some(fill(tr("All {} projects were brought over."), &[&brought]))
        }
        (brought, all) => Some(fill(
            tr("{} of {} projects were brought over."),
            &[&brought, &all],
        )),
    };
    let settings = (!outcome.settings.is_empty()).then(|| {
        let names: Vec<&str> = outcome
            .settings
            .iter()
            .map(|group| setting_label(group))
            .collect();
        fill(tr("Settings brought over: {}."), &[&names.join(", ")])
    });
    match (projects, settings) {
        (None, None) => tr("Nothing was brought over.").into(),
        (Some(line), None) | (None, Some(line)) => line,
        (Some(projects), Some(settings)) => format!("{projects} {settings}"),
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

    fn done(id: &str, error: Option<&str>) -> wisp_server::desktop::WebViewImport {
        wisp_server::desktop::WebViewImport {
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
        let outcome = |projects: Vec<wisp_server::desktop::WebViewImport>, settings: &[&str]| {
            WebViewOutcome {
                projects,
                settings: settings.iter().map(|group| (*group).to_owned()).collect(),
            }
        };
        assert_eq!(
            summary(&outcome(vec![done("a", None)], &[])),
            "The project was brought over."
        );
        assert_eq!(
            summary(&outcome(vec![done("a", None), done("b", None)], &[])),
            "All 2 projects were brought over."
        );
        assert_eq!(
            summary(&outcome(
                vec![done("a", None), done("b", Some("It is already here"))],
                &[]
            )),
            "1 of 2 projects were brought over."
        );
        assert_eq!(
            summary(&outcome(vec![done("a", Some("gone"))], &[])),
            "No project was brought over."
        );
        assert_eq!(
            summary(&outcome(vec![done("a", None)], &["memory", "experts"])),
            "The project was brought over. Settings brought over: Memory, Experts."
        );
        assert_eq!(
            summary(&outcome(Vec::new(), &["network"])),
            "Settings brought over: Network routes and mirrors."
        );
        assert_eq!(
            summary(&outcome(Vec::new(), &[])),
            "Nothing was brought over."
        );
    }

    #[test]
    fn every_group_of_settings_the_host_offers_has_a_name() {
        for (group, _) in wisp_server::desktop::WEBVIEW_SETTING_GROUPS {
            assert_ne!(setting_label(group), "Other settings", "{group}");
        }
    }
}
