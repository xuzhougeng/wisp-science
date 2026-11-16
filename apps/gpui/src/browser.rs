//! The browser tools as Settings words them, and the two host lists.
use wisp_server::desktop::{BrowserFilters, BrowserHostRule, BrowserSettings};

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
}
