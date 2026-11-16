//! Cards for conversations in other projects that finished or failed while
//! this window shows something else, as the WebView desktop has them
//! (#1619, #1717): which conversation, which project, done or failed, and a
//! way to open it. They float in the window's corner over the page and
//! below dialogs, never take focus and never switch the project by
//! themselves.
//!
//! A card for a finished conversation leaves by itself after a linger that
//! starts over while the pointer is on it; a failure stays until it is
//! opened, closed, or viewed somewhere. The card for a conversation the
//! Needs-you list had and no longer has goes too: it was read elsewhere.
//! It holds no graphics state and is exercised without a window.
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use wisp_dto::SessionSearchInfo;

/// At most this many cards show; older ones drop off the bottom.
pub const MAX: usize = 3;

/// How long a finished conversation's card stays once the pointer is not
/// on it.
pub const LINGER: Duration = Duration::from_secs(15);

/// One card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub project_id: String,
    pub project_name: String,
    pub session_id: String,
    pub title: String,
    /// Its turn stopped on an error.
    pub failed: bool,
    /// Tells a repeated completion of the same conversation from the card
    /// it replaces, so the new card's linger starts over.
    pub arrival: u64,
    /// The Needs-you list has listed it since it came.
    listed: bool,
    /// When its linger started, or started over after the pointer left.
    since: Instant,
    hovered: bool,
}

impl Notice {
    /// Only a finished conversation's card leaves by itself.
    pub fn lingers(&self) -> bool {
        !self.failed
    }
}

/// The cards of one window, newest first.
#[derive(Debug, Default)]
pub struct Notices {
    cards: Vec<Notice>,
    next: u64,
}

impl Notices {
    pub fn cards(&self) -> &[Notice] {
        &self.cards
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// A card for `row`, newest first, one per conversation, at most
    /// [`MAX`].
    pub fn push(&mut self, row: &SessionSearchInfo, failed: bool, now: Instant) {
        self.cards.retain(|card| card.session_id != row.id);
        self.cards.insert(
            0,
            Notice {
                project_id: row.project_id.clone(),
                project_name: row.project_name.clone(),
                session_id: row.id.clone(),
                title: row.title.clone(),
                failed,
                arrival: self.next,
                listed: false,
                since: now,
                hovered: false,
            },
        );
        self.next = self.next.wrapping_add(1);
        self.cards.truncate(MAX);
    }

    /// Takes the card for `session` away; whether there was one.
    pub fn remove(&mut self, session: &str) -> bool {
        let before = self.cards.len();
        self.cards.retain(|card| card.session_id != session);
        self.cards.len() != before
    }

    /// Escape: the newest card goes.
    pub fn dismiss_newest(&mut self) -> bool {
        if self.cards.is_empty() {
            return false;
        }
        self.cards.remove(0);
        true
    }

    /// Matches the cards with a fresh Needs-you list. A card goes once the
    /// list had its conversation and no longer has it: someone read it. A
    /// list that never had it (it was read before the turn's end reached
    /// the list) leaves a fresh card alone. Returns whether anything
    /// changed.
    pub fn reconcile(&mut self, inbox: &HashSet<&str>) -> bool {
        let before = self.cards.clone();
        self.cards.retain_mut(|card| {
            let listed = inbox.contains(card.session_id.as_str());
            if listed {
                card.listed = true;
            }
            listed || !card.listed
        });
        self.cards != before
    }

    /// The pointer came onto a card or left it; leaving starts its linger
    /// over, so a card never goes from under the pointer.
    pub fn hover(&mut self, session: &str, hovered: bool, now: Instant) {
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.session_id == session)
        {
            if card.hovered && !hovered {
                card.since = now;
            }
            card.hovered = hovered;
        }
    }

    /// Takes away the finished conversations' cards whose linger is over,
    /// and says how long until the next one is due, if one lingers.
    pub fn expire(&mut self, now: Instant) -> Option<Duration> {
        self.cards.retain(|card| {
            card.hovered || !card.lingers() || now.duration_since(card.since) < LINGER
        });
        self.cards
            .iter()
            .filter(|card| card.lingers() && !card.hovered)
            .map(|card| LINGER.saturating_sub(now.duration_since(card.since)))
            .min()
    }
}

/// Which of the conversations that just stopped working get a card in this
/// window: one whose turn ended (`finished` has it, `true` when it failed;
/// a conversation that only waits for an approval is still working), in a
/// project other than the one this window shows (on the home page, any),
/// not the conversation on screen, and not in a project privacy mode hides.
pub fn arrivals<'a>(
    stopped: &[&'a SessionSearchInfo],
    finished: &HashMap<String, bool>,
    here: Option<&str>,
    open: Option<&str>,
    hidden: impl Fn(&str) -> bool,
) -> Vec<(&'a SessionSearchInfo, bool)> {
    stopped
        .iter()
        .filter(|row| here.is_none_or(|project| project != row.project_id))
        .filter(|row| open != Some(row.id.as_str()))
        .filter(|row| !hidden(&row.project_id))
        .filter_map(|row| finished.get(&row.id).map(|failed| (*row, *failed)))
        .collect()
}

/// Where "Open conversation" on a card opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// In this window, which shows the home page.
    Here,
    /// In the other window, at this place among them, that already shows
    /// the conversation's project.
    Window(usize),
    /// In a new window of its own, as this one keeps its project.
    NewWindow,
}

/// As the WebView decides it (#1726): a window on the home page takes the
/// conversation itself; a window that shows another project leaves it
/// alone and uses another window that already shows the conversation's
/// project, else opens one for it. `others` are the projects the other
/// windows show.
pub fn opens(here: Option<&str>, others: &[Option<String>], project: &str) -> Opens {
    if here.is_none() {
        return Opens::Here;
    }
    others
        .iter()
        .position(|other| other.as_deref() == Some(project))
        .map_or(Opens::NewWindow, Opens::Window)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, project: &str) -> SessionSearchInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "project_id": project, "project_name": format!("Lab {project}"),
            "title": format!("About {id}"), "status": "complete"
        }))
        .unwrap()
    }

    fn ids(notices: &Notices) -> Vec<&str> {
        notices
            .cards()
            .iter()
            .map(|card| card.session_id.as_str())
            .collect()
    }

    #[test]
    fn cards_are_newest_first_one_per_conversation_and_at_most_three() {
        let now = Instant::now();
        let mut notices = Notices::default();
        for id in ["s1", "s2", "s3"] {
            notices.push(&row(id, "b"), false, now);
        }
        assert_eq!(ids(&notices), ["s3", "s2", "s1"]);
        // Finishing again moves it to the top as a new arrival.
        let first = notices.cards()[2].arrival;
        notices.push(&row("s1", "b"), true, now);
        assert_eq!(ids(&notices), ["s1", "s3", "s2"]);
        assert_ne!(notices.cards()[0].arrival, first);
        assert!(notices.cards()[0].failed);
        notices.push(&row("s4", "b"), false, now);
        assert_eq!(ids(&notices), ["s4", "s1", "s3"]);
        let card = &notices.cards()[0];
        assert_eq!(
            (card.project_name.as_str(), card.title.as_str()),
            ("Lab b", "About s4")
        );
        // Escape takes the newest; opening or closing one takes that one.
        assert!(notices.dismiss_newest());
        assert!(notices.remove("s3"));
        assert!(!notices.remove("s3"));
        assert_eq!(ids(&notices), ["s1"]);
    }

    #[test]
    fn only_a_finished_card_leaves_by_itself_and_never_from_under_the_pointer() {
        let start = Instant::now();
        let mut notices = Notices::default();
        notices.push(&row("done", "b"), false, start);
        notices.push(&row("failed", "b"), true, start);
        assert_eq!(notices.expire(start), Some(LINGER));
        let later = start + LINGER - Duration::from_secs(5);
        assert_eq!(notices.expire(later), Some(Duration::from_secs(5)));
        // The pointer rests on it past its time: it stays.
        notices.hover("done", true, later);
        assert_eq!(notices.expire(start + LINGER * 2), None);
        assert_eq!(ids(&notices), ["failed", "done"]);
        // Leaving starts the wait over.
        let left = start + LINGER * 2;
        notices.hover("done", false, left);
        assert_eq!(notices.expire(left + LINGER / 2), Some(LINGER / 2));
        assert_eq!(notices.expire(left + LINGER), None);
        // The failure is still there, whatever the time.
        assert_eq!(ids(&notices), ["failed"]);
        assert_eq!(notices.expire(left + LINGER * 10), None);
        assert_eq!(ids(&notices), ["failed"]);
    }

    #[test]
    fn a_card_goes_once_the_needs_you_list_had_it_and_then_dropped_it() {
        let mut notices = Notices::default();
        notices.push(&row("s1", "b"), false, Instant::now());
        // The list was read before the turn's end reached it.
        assert!(!notices.reconcile(&HashSet::new()));
        assert_eq!(ids(&notices), ["s1"]);
        assert!(notices.reconcile(&HashSet::from(["s1"])));
        assert!(!notices.reconcile(&HashSet::from(["s1"])));
        // Read somewhere: the list no longer has it.
        assert!(notices.reconcile(&HashSet::new()));
        assert!(notices.is_empty());
    }

    #[test]
    fn a_card_is_for_a_turn_that_ended_in_another_project_and_not_on_screen() {
        let (a, b, c, d, e) = (
            row("a", "lab"),
            row("b", "other"),
            row("c", "other"),
            row("d", "other"),
            row("e", "secret"),
        );
        let stopped = [&a, &b, &c, &d, &e];
        // c only waits for an approval: its turn has not ended.
        let finished = HashMap::from([
            ("a".to_string(), false),
            ("b".to_string(), true),
            ("d".to_string(), false),
            ("e".to_string(), false),
        ]);
        let hidden = |project: &str| project == "secret";
        let shown = |here, open| {
            arrivals(&stopped, &finished, here, open, hidden)
                .into_iter()
                .map(|(row, failed)| (row.id.as_str(), failed))
                .collect::<Vec<_>>()
        };
        // The window's own project is in its sidebar already.
        assert_eq!(shown(Some("lab"), None), [("b", true), ("d", false)]);
        // The conversation on screen never gets a card.
        assert_eq!(shown(Some("lab"), Some("d")), [("b", true)]);
        // On the home page every project is another one.
        assert_eq!(shown(None, None), [("a", false), ("b", true), ("d", false)]);
    }

    #[test]
    fn opening_a_card_uses_the_window_that_already_shows_its_project() {
        let others = [Some("x".to_string()), None, Some("lab".to_string())];
        assert_eq!(opens(Some("mine"), &others, "lab"), Opens::Window(2));
        assert_eq!(opens(Some("mine"), &others, "y"), Opens::NewWindow);
        assert_eq!(opens(Some("mine"), &[], "lab"), Opens::NewWindow);
        // The home page takes it itself.
        assert_eq!(opens(None, &others, "lab"), Opens::Here);
    }
}
