//! The browser tools as Settings words them, the two host lists, and what
//! the app asks or tells about the browser over a project: the tabs an
//! ended turn opened, the tabs whose page waits for a human, and the
//! extension that has to be updated, kept as the WebView keeps them.
use std::collections::BTreeSet;

use wisp_dto::{
    BrowserExtensionStatus, BrowserNeedsHumanTab, BrowserTabCleanupItem, BrowserTabCleanupPrompt,
};
use wisp_server::desktop::{BrowserAlerts, BrowserFilters, BrowserHostRule, BrowserSettings};

use crate::{
    i18n::{fill, tr},
    present::Tone,
};

/// Where the bridge to the browser stands, in a line.
pub fn status(settings: &BrowserSettings) -> (String, Tone) {
    let status = &settings.status;
    if !settings.enabled {
        return (tr("Off").to_owned(), Tone::Neutral);
    }
    if !settings.listening {
        let why = settings.problem.as_deref().unwrap_or_default();
        return (fill(tr("Not listening: {}"), &[&why]), Tone::Danger);
    }
    if !status.connected {
        return match &status.refused_origin {
            Some(origin) if !origin.is_empty() => (
                fill(
                    tr("Waiting for the extension. Something else tried to connect: {}"),
                    &[origin],
                ),
                Tone::Attention,
            ),
            _ => (
                tr("Waiting for the extension to connect.").to_owned(),
                Tone::Active,
            ),
        };
    }
    if status.update_required {
        return (
            fill(
                tr(
                    "Extension {} is connected but too old or too new for this app; reload it from the folder below.",
                ),
                &[&status.extension_version],
            ),
            Tone::Attention,
        );
    }
    if status.paused {
        return (
            tr("Connected, but browser control is paused in the extension's popup.").to_owned(),
            Tone::Attention,
        );
    }
    (
        fill(
            tr("Connected · extension {} · {} pages open"),
            &[&status.extension_version, &status.tabs],
        ),
        Tone::Success,
    )
}

/// Which of the two lists a change is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Block,
    Prefer,
}

fn list(filters: &mut BrowserFilters, which: List) -> &mut Vec<BrowserHostRule> {
    match which {
        List::Block => &mut filters.block,
        List::Prefer => &mut filters.prefer,
    }
}

/// The lists with a host added to one of them. The host checks it.
pub fn with_rule(
    filters: &BrowserFilters,
    which: List,
    host: &str,
    reason: &str,
) -> BrowserFilters {
    let mut next = filters.clone();
    list(&mut next, which).push(BrowserHostRule {
        host: host.trim().to_owned(),
        reason: reason.trim().to_owned(),
    });
    next
}

/// The lists without the rule at `index` of one of them.
pub fn without_rule(filters: &BrowserFilters, which: List, index: usize) -> BrowserFilters {
    let mut next = filters.clone();
    let rules = list(&mut next, which);
    if index < rules.len() {
        rules.remove(index);
    }
    next
}

/// What the app shows about the browser over the open project. The host
/// is asked every few seconds; what it says replaces what was shown,
/// except what the user is in the middle of.
#[derive(Debug, Default)]
pub struct Prompts {
    /// The ended turn whose tabs are asked about, the oldest first.
    pub cleanup: Option<BrowserTabCleanupPrompt>,
    /// Its tabs to close: all of them until the user unticks some.
    pub selected: BTreeSet<i64>,
    pub cleanup_busy: bool,
    pub cleanup_error: Option<String>,
    /// The tabs whose page waits for a human; empty while not shown.
    pub human: Vec<BrowserNeedsHumanTab>,
    /// The tabs the user put off with Later, shown again when they change.
    put_off: Option<Vec<(i64, String)>>,
    pub human_busy: bool,
    pub human_error: Option<String>,
    /// The extension, while it has to be updated.
    pub extension: Option<BrowserExtensionStatus>,
    /// The bundled version whose banner the user dismissed.
    dismissed: Option<String>,
    pub update_busy: bool,
    /// `manual_reload_required` once an update needs the user's click.
    pub update_outcome: Option<String>,
    pub update_error: Option<String>,
}

fn human_keys(tabs: &[BrowserNeedsHumanTab]) -> Vec<(i64, String)> {
    tabs.iter()
        .map(|tab| (tab.tab_id, tab.turn_id.clone()))
        .collect()
}

impl Prompts {
    /// Takes what the host says now.
    pub fn accept(&mut self, alerts: BrowserAlerts) {
        if !self.cleanup_busy {
            let shown = self.cleanup.as_ref().and_then(|shown| {
                alerts
                    .cleanups
                    .iter()
                    .find(|now| now.turn_id == shown.turn_id)
            });
            match shown {
                // The same question: what the user unticked stays unticked.
                Some(now) => {
                    self.selected
                        .retain(|tab| now.tabs.iter().any(|item| item.tab_id == *tab));
                    self.cleanup = Some(now.clone());
                }
                None => {
                    self.cleanup = alerts.cleanups.into_iter().next();
                    self.selected = self
                        .cleanup
                        .iter()
                        .flat_map(|prompt| prompt.tabs.iter().map(|tab| tab.tab_id))
                        .collect();
                    self.cleanup_error = None;
                }
            }
        }
        if !self.human_busy {
            let now = human_keys(&alerts.needs_human);
            if alerts.needs_human.is_empty() {
                self.human.clear();
                self.put_off = None;
                self.human_error = None;
            } else if self.put_off.as_ref() == Some(&now) {
                self.human.clear();
            } else {
                self.put_off = None;
                if human_keys(&self.human) != now {
                    self.human_error = None;
                }
                self.human = alerts.needs_human;
            }
        }
        if !self.update_busy {
            match alerts.extension {
                Some(status) => {
                    let target = status.bundled_version.clone().unwrap_or_default();
                    if self.dismissed.as_deref() != Some(target.as_str()) {
                        self.extension = Some(status);
                    }
                }
                None => {
                    self.extension = None;
                    self.update_outcome = None;
                    self.update_error = None;
                }
            }
        }
    }

    /// Ticks or unticks one tab of the question.
    pub fn toggle(&mut self, tab: i64) {
        if !self.selected.remove(&tab) {
            self.selected.insert(tab);
        }
    }

    /// The ticked tabs, to close.
    pub fn chosen(&self) -> Vec<BrowserTabCleanupItem> {
        self.cleanup
            .iter()
            .flat_map(|prompt| prompt.tabs.iter())
            .filter(|tab| self.selected.contains(&tab.tab_id))
            .cloned()
            .collect()
    }

    /// The question is answered: the next one is shown when the host says.
    pub fn cleanup_answered(&mut self) {
        self.cleanup = None;
        self.selected.clear();
        self.cleanup_busy = false;
        self.cleanup_error = None;
    }

    /// Later: the question goes until the tabs that wait change.
    pub fn put_off(&mut self) {
        self.put_off = Some(human_keys(&self.human));
        self.human.clear();
        self.human_error = None;
    }

    /// Dismiss: no banner for this bundled version again.
    pub fn dismiss_banner(&mut self) {
        if let Some(status) = self.extension.take() {
            self.dismissed = Some(status.bundled_version.unwrap_or_default());
        }
        self.update_outcome = None;
        self.update_error = None;
    }

    /// The update banner's words, while there is one.
    pub fn banner(&self) -> Option<Banner> {
        let status = self.extension.as_ref()?;
        let manual = self.update_outcome.as_deref() == Some("manual_reload_required");
        let unknown = || tr("unknown").to_owned();
        let current = status.current_version.clone().unwrap_or_else(unknown);
        let bundled = status.bundled_version.clone().unwrap_or_else(unknown);
        let (title, body) = if manual {
            (
                tr("Updated files are ready — reload the extension"),
                tr(
                    "Chrome cannot reload version {} automatically. On the extension page, reload \
                    Wisp Real Browser Bridge {}; if it still shows the old version, load the \
                    unpacked folder below.",
                ),
            )
        } else if !status.connected {
            (
                tr("Waiting for the browser extension to reconnect"),
                tr(
                    "Last detected version {}; this build requires {}. Keep Wisp running while \
                    the extension reconnects, or open the extension page below.",
                ),
            )
        } else {
            (
                tr("Wisp Real Browser Bridge is out of date"),
                tr(
                    "Connected version {}; this build includes {}. Update before using newer \
                    browser tools.",
                ),
            )
        };
        Some(Banner {
            title,
            body: fill(body, &[&current, &bundled]),
            path: status
                .extension_path
                .clone()
                .filter(|path| !path.is_empty()),
            manual,
            error: self.update_error.clone().or_else(|| status.error.clone()),
        })
    }
}

/// What the update banner says.
#[derive(Debug, PartialEq, Eq)]
pub struct Banner {
    pub title: &'static str,
    pub body: String,
    /// The folder to load, when it is verified.
    pub path: Option<String>,
    /// The update needs the user's click on Reload: the steps are shown and
    /// Update extension is not offered again.
    pub manual: bool,
    pub error: Option<String>,
}

/// The steps of loading the folder in the browser, as the WebView lists them
/// under a banner that waits for the user.
pub fn setup_steps(mac: bool) -> [&'static str; 4] {
    [
        tr(
            "On the extension page that just opened (chrome://extensions), turn on Developer \
            mode in the top-right corner.",
        ),
        tr("Click \"Load unpacked\". A folder picker opens."),
        if mac {
            tr(
                "In the picker press ⌘⇧G to open \"Go to Folder\", paste the path above with ⌘V \
                (it is already copied), then press Return. The Library folder is hidden, so \
                clicking through folders will not find it.",
            )
        } else {
            tr(
                "Paste the path above into the picker's address bar with Ctrl+V (Linux: press \
                Ctrl+L first), then press Enter.",
            )
        },
        if mac {
            tr(
                "Once manifest.json is listed, do not click any file: press \"Select\" in the \
                bottom-right corner.",
            )
        } else {
            tr("Once manifest.json is listed, press \"Select Folder\".")
        },
    ]
}

/// The tab as a row of a question: its title, else its address.
pub fn tab_name(title: &str, url: &str) -> String {
    if title.trim().is_empty() {
        url.to_owned()
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::BrowserStatus;

    #[test]
    fn the_bridge_says_where_it_stands() {
        let mut settings = BrowserSettings::default();
        assert_eq!(status(&settings), ("Off".to_owned(), Tone::Neutral));
        settings.enabled = true;
        settings.problem = Some("cannot listen on 127.0.0.1:18765".into());
        assert_eq!(
            status(&settings),
            (
                "Not listening: cannot listen on 127.0.0.1:18765".to_owned(),
                Tone::Danger
            )
        );
        settings.listening = true;
        settings.problem = None;
        assert_eq!(status(&settings).1, Tone::Active);
        settings.status.refused_origin = Some("https://example.com".into());
        assert!(status(&settings).0.ends_with("https://example.com"));
        settings.status = BrowserStatus {
            connected: true,
            extension_version: "0.5.0".into(),
            update_required: true,
            paused: true,
            tabs: 3,
            refused_origin: None,
        };
        assert_eq!(status(&settings).1, Tone::Attention);
        assert!(status(&settings).0.contains("reload it"));
        settings.status.update_required = false;
        assert!(status(&settings).0.contains("paused"));
        settings.status.paused = false;
        assert_eq!(
            status(&settings),
            (
                "Connected · extension 0.5.0 · 3 pages open".to_owned(),
                Tone::Success
            )
        );
    }

    #[test]
    fn a_host_is_added_to_one_list_and_taken_out_of_it() {
        let empty = BrowserFilters::default();
        let blocked = with_rule(&empty, List::Block, " example.com ", " taken over ");
        assert_eq!(blocked.block.len(), 1);
        assert_eq!(blocked.block[0].host, "example.com");
        assert_eq!(blocked.block[0].reason, "taken over");
        assert!(blocked.prefer.is_empty());
        let both = with_rule(&blocked, List::Prefer, "nih.gov", "");
        assert_eq!((both.block.len(), both.prefer.len()), (1, 1));
        assert_eq!(without_rule(&both, List::Block, 0).block.len(), 0);
        assert_eq!(without_rule(&both, List::Block, 0).prefer.len(), 1);
        // An index that is not there changes nothing.
        assert_eq!(without_rule(&both, List::Prefer, 5), both);
    }

    fn cleanup(turn: &str, tabs: &[i64]) -> BrowserTabCleanupPrompt {
        BrowserTabCleanupPrompt {
            turn_id: turn.into(),
            frame_id: "c".into(),
            tabs: tabs
                .iter()
                .map(|tab| BrowserTabCleanupItem {
                    session: "shared".into(),
                    tab_id: *tab,
                    url: format!("https://example.org/{tab}"),
                    ..Default::default()
                })
                .collect(),
        }
    }

    fn waiting(tab: i64, turn: &str) -> BrowserNeedsHumanTab {
        BrowserNeedsHumanTab {
            tab_id: tab,
            turn_id: turn.into(),
            frame_id: "c".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_turns_tabs_are_asked_about_one_turn_at_a_time_all_ticked() {
        let mut prompts = Prompts::default();
        prompts.accept(BrowserAlerts {
            cleanups: vec![cleanup("t1", &[1, 2]), cleanup("t2", &[3])],
            ..Default::default()
        });
        assert_eq!(prompts.cleanup.as_ref().unwrap().turn_id, "t1");
        assert_eq!(prompts.selected, BTreeSet::from([1, 2]));
        // What the user unticked stays unticked when the host is asked again,
        // and a tab that closed meanwhile goes.
        prompts.toggle(2);
        prompts.accept(BrowserAlerts {
            cleanups: vec![cleanup("t1", &[1, 2]), cleanup("t2", &[3])],
            ..Default::default()
        });
        assert_eq!(prompts.chosen().len(), 1);
        prompts.accept(BrowserAlerts {
            cleanups: vec![cleanup("t1", &[2]), cleanup("t2", &[3])],
            ..Default::default()
        });
        assert!(prompts.chosen().is_empty());
        // While its answer is on the way, nothing changes under the user.
        prompts.cleanup_busy = true;
        prompts.accept(BrowserAlerts::default());
        assert_eq!(prompts.cleanup.as_ref().unwrap().turn_id, "t1");
        // Answered, the next turn's question comes, all ticked.
        prompts.cleanup_answered();
        prompts.accept(BrowserAlerts {
            cleanups: vec![cleanup("t2", &[3])],
            ..Default::default()
        });
        assert_eq!(prompts.cleanup.as_ref().unwrap().turn_id, "t2");
        assert_eq!(prompts.selected, BTreeSet::from([3]));
        prompts.accept(BrowserAlerts::default());
        assert!(prompts.cleanup.is_none());
    }

    #[test]
    fn later_puts_the_verification_question_off_until_the_tabs_change() {
        let mut prompts = Prompts::default();
        let one = BrowserAlerts {
            needs_human: vec![waiting(5, "t1")],
            ..Default::default()
        };
        prompts.accept(one.clone());
        assert_eq!(prompts.human.len(), 1);
        prompts.human_error = Some("still".into());
        // Asked again with nothing new, the error stays.
        prompts.accept(one.clone());
        assert_eq!(prompts.human_error.as_deref(), Some("still"));
        prompts.put_off();
        prompts.accept(one.clone());
        assert!(prompts.human.is_empty());
        // Another tab, or the same tab in a later turn, asks again.
        prompts.accept(BrowserAlerts {
            needs_human: vec![waiting(5, "t2")],
            ..Default::default()
        });
        assert_eq!(prompts.human.len(), 1);
        assert!(prompts.human_error.is_none());
        // Nothing waits any more: the question goes.
        prompts.accept(BrowserAlerts::default());
        assert!(prompts.human.is_empty());
        prompts.accept(one);
        assert_eq!(prompts.human.len(), 1);
    }

    #[test]
    fn the_update_banner_says_what_waits_and_stays_dismissed_for_its_version() {
        let mut prompts = Prompts::default();
        assert!(prompts.banner().is_none());
        let status = BrowserExtensionStatus {
            connected: true,
            current_version: Some("0.4.0".into()),
            bundled_version: Some("0.5.0".into()),
            update_required: true,
            extension_path: Some("/data/browser-extension".into()),
            ..Default::default()
        };
        let alerts = BrowserAlerts {
            extension: Some(status.clone()),
            ..Default::default()
        };
        prompts.accept(alerts.clone());
        let banner = prompts.banner().unwrap();
        assert_eq!(banner.title, "Wisp Real Browser Bridge is out of date");
        assert!(banner.body.contains("0.4.0") && banner.body.contains("0.5.0"));
        assert_eq!(banner.path.as_deref(), Some("/data/browser-extension"));
        assert!(!banner.manual);
        prompts.update_outcome = Some("manual_reload_required".into());
        let banner = prompts.banner().unwrap();
        assert!(banner.manual);
        assert!(banner.title.starts_with("Updated files are ready"));
        // Away, it says it waits for the extension.
        prompts.update_outcome = None;
        prompts.accept(BrowserAlerts {
            extension: Some(BrowserExtensionStatus {
                connected: false,
                current_version: None,
                ..status.clone()
            }),
            ..Default::default()
        });
        let banner = prompts.banner().unwrap();
        assert!(banner.title.starts_with("Waiting"));
        assert!(banner.body.contains("unknown"));
        // Dismissed, not for this version again; up to date, gone.
        prompts.dismiss_banner();
        prompts.accept(alerts.clone());
        assert!(prompts.banner().is_none());
        prompts.accept(BrowserAlerts {
            extension: Some(BrowserExtensionStatus {
                bundled_version: Some("0.6.0".into()),
                ..status
            }),
            ..Default::default()
        });
        assert!(prompts.banner().is_some());
        prompts.update_outcome = Some("manual_reload_required".into());
        prompts.accept(BrowserAlerts::default());
        assert!(prompts.banner().is_none() && prompts.update_outcome.is_none());
        assert_eq!(tab_name(" ", "https://a"), "https://a");
        assert_ne!(setup_steps(true)[2], setup_steps(false)[2]);
    }
}
