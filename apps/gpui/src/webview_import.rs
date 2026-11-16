//! Bringing the WebView desktop's projects and settings into this one:
//! where its database is, which of its projects can come, what each group
//! of its settings is called, and what to say about how it went.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::i18n::{fill, tr};
use wisp_server::desktop::{WebViewModel, WebViewOutcome, WebViewProject};

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

/// The saved models ticked when the list is first shown: every one not
/// here yet.
pub fn first_model_picks(models: &[WebViewModel]) -> BTreeSet<String> {
    models
        .iter()
        .filter(|model| !model.here)
        .map(|model| model.id.clone())
        .collect()
}

/// The ticked saved models that can come, in the order they are listed.
pub fn chosen_models(models: &[WebViewModel], picks: &BTreeSet<String>) -> Vec<String> {
    models
        .iter()
        .filter(|model| !model.here && picks.contains(&model.id))
        .map(|model| model.id.clone())
        .collect()
}

/// The line under a saved model's name: its id at the provider, and
/// whether its key comes too.
pub fn model_detail(model: &WebViewModel) -> String {
    let key = if model.has_key {
        tr("its key comes too")
    } else {
        tr("no key saved; enter it here")
    };
    let mut parts = vec![model.model.as_str(), key];
    if model.active {
        parts.push(tr("the WebView's everyday model"));
    }
    parts.join(" · ")
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
        "subagent_model" => tr("The sub-agent model"),
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
    let models = match outcome.models.len() {
        0 => None,
        1 => Some(tr("1 saved model was brought over.").to_owned()),
        count => Some(fill(tr("{} saved models were brought over."), &[&count])),
    };
    let settings = (!outcome.settings.is_empty()).then(|| {
        let names: Vec<&str> = outcome
            .settings
            .iter()
            .map(|group| setting_label(group))
            .collect();
        fill(tr("Settings brought over: {}."), &[&names.join(", ")])
    });
    let lines: Vec<String> = [projects, models, settings].into_iter().flatten().collect();
    if lines.is_empty() {
        tr("Nothing was brought over.").into()
    } else {
        lines.join(" ")
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
                models: Vec::new(),
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
        let mut with_models = outcome(vec![done("a", None)], &["memory"]);
        with_models.models = vec!["m1".into(), "m2".into()];
        assert_eq!(
            summary(&with_models),
            "The project was brought over. 2 saved models were brought over. \
             Settings brought over: Memory."
        );
        with_models.models.pop();
        with_models.projects.clear();
        with_models.settings.clear();
        assert_eq!(summary(&with_models), "1 saved model was brought over.");
    }

    #[test]
    fn a_saved_model_says_whether_its_key_comes() {
        let mut model = WebViewModel {
            id: "m".into(),
            name: "Flash".into(),
            model: "flash-model".into(),
            provider: "openai".into(),
            has_key: true,
            active: true,
            here: false,
        };
        assert_eq!(
            model_detail(&model),
            "flash-model · its key comes too · the WebView's everyday model"
        );
        model.has_key = false;
        model.active = false;
        assert_eq!(
            model_detail(&model),
            "flash-model · no key saved; enter it here"
        );
        let mut here = model.clone();
        here.id = "h".into();
        here.here = true;
        let models = [model, here];
        assert_eq!(first_model_picks(&models).len(), 1);
        let ticked: BTreeSet<String> = ["h", "m"].map(String::from).into();
        assert_eq!(chosen_models(&models, &ticked), ["m"]);
    }

    #[test]
    fn every_group_of_settings_the_host_offers_has_a_name() {
        for (group, _) in wisp_server::desktop::WEBVIEW_SETTING_GROUPS {
            assert_ne!(setting_label(group), "Other settings", "{group}");
        }
    }
}
