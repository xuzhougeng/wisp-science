//! Cross-project completion cards (#1619). A conversation in another project
//! finished while this window shows a different one: the backend sends
//! `session-notice` only to such windows. Cards never take focus, never switch
//! the project, and leave the composer alone; the user opens one explicitly.
//!
//! A card for a finished conversation leaves by itself after a short linger
//! (#1717); the bell's other-projects dot and inbox still list it. A failure
//! stays until it is opened, dismissed, or viewed somewhere.

use super::*;

/// At most this many cards stay on screen; older ones drop off the bottom.
const MAX_SESSION_NOTICES: usize = 3;

/// The CSS animation that fades a finished card out; its `animationend`
/// removes the card. Named in `chat.css` (`.session-notice-autoclose`).
const SESSION_NOTICE_LEAVE_ANIMATION: &str = "session-notice-leave";

thread_local! {
    static NEXT_SESSION_NOTICE_ARRIVAL: Cell<u64> = const { Cell::new(0) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionNoticeCard {
    pub(crate) notice: SessionNotice,
    /// Set once the Needs-you inbox has listed this session. A later refresh
    /// that no longer lists it means someone viewed the reply, so the card
    /// goes away. A refresh that never listed it (e.g. it raced the turn end)
    /// must not drop a fresh card.
    pub(crate) listed: bool,
    /// Distinguishes a repeated completion of the same conversation from the
    /// card it replaces, so the replacement is a new element whose linger
    /// starts over instead of inheriting the old card's remaining time.
    pub(crate) arrival: u64,
}

/// Only a successful completion leaves on its own; a failure needs attention.
pub(crate) fn session_notice_autocloses(notice: &SessionNotice) -> bool {
    notice.kind != "error"
}

/// Newest first, one card per session, bounded.
pub(crate) fn push_session_notice(cards: &mut Vec<SessionNoticeCard>, notice: SessionNotice) {
    cards.retain(|card| card.notice.session_id != notice.session_id);
    let arrival = NEXT_SESSION_NOTICE_ARRIVAL.with(|next| {
        let arrival = next.get();
        next.set(arrival.wrapping_add(1));
        arrival
    });
    cards.insert(
        0,
        SessionNoticeCard {
            notice,
            listed: false,
            arrival,
        },
    );
    cards.truncate(MAX_SESSION_NOTICES);
}

/// Reconcile cards with a fresh Needs-you inbox snapshot. Returns whether
/// anything changed so callers can skip a redundant signal write.
pub(crate) fn reconcile_session_notices(
    cards: &mut Vec<SessionNoticeCard>,
    inbox_session_ids: &HashSet<&str>,
) -> bool {
    let before = cards.clone();
    cards.retain_mut(|card| {
        let listed = inbox_session_ids.contains(card.notice.session_id.as_str());
        if listed {
            card.listed = true;
        }
        listed || !card.listed
    });
    *cards != before
}

pub(crate) fn session_notice_title(locale: Locale, notice: &SessionNotice) -> String {
    let title = user_message_presentation(&notice.session_title).body;
    if title.trim().is_empty() {
        t(locale, "sidebar.untitled")
    } else {
        title
    }
}

/// Desktop body: the project comes first because the window showing it
/// belongs to a different project.
pub(crate) fn session_notice_body(locale: Locale, notice: &SessionNotice) -> String {
    let title = session_notice_title(locale, notice);
    if notice.project_name.trim().is_empty() {
        title
    } else {
        format!("{} · {title}", notice.project_name)
    }
}

#[component]
pub(crate) fn SessionNoticeStack(
    cards: RwSignal<Vec<SessionNoticeCard>>,
    locale: ReadSignal<Locale>,
    on_open: Callback<(String, String)>,
) -> impl IntoView {
    view! {
        <Show when=move || cards.with(|cards| !cards.is_empty())>
            <div class="session-notice-stack" role="status" aria-live="polite"
                aria-label=move || t(locale.get(), "session_notice.region")
                data-testid="session-notice-stack">
                <For each=move || cards.get()
                    key=|card| (card.notice.session_id.clone(), card.arrival)
                    children=move |card| {
                        let arrival = card.arrival;
                        let notice = card.notice;
                        let failed = notice.kind == "error";
                        let autoclose = session_notice_autocloses(&notice);
                        let autoclose_id = notice.session_id.clone();
                        let status_key = if failed { "notify.error" } else { "notify.done" };
                        let open_target = (notice.project_id.clone(), notice.session_id.clone());
                        let dismiss_id = notice.session_id.clone();
                        let project = notice.project_name.clone();
                        let notice_for_title = notice.clone();
                        view! {
                            <div class="session-notice" class:session-notice-error=failed
                                class:session-notice-autoclose=autoclose
                                data-testid="session-notice" data-session=notice.session_id.clone()
                                on:animationend=move |ev: web_sys::AnimationEvent| {
                                    // The entrance animation ends on this element
                                    // too; only the fade-out removes the card.
                                    if !autoclose || ev.animation_name() != SESSION_NOTICE_LEAVE_ANIMATION {
                                        return;
                                    }
                                    cards.update(|cards| cards.retain(|card| {
                                        card.notice.session_id != autoclose_id || card.arrival != arrival
                                    }));
                                }>
                                <span class="session-notice-icon">
                                    {compose_icon(if failed { "circle-alert" } else { "circle-check" })}
                                </span>
                                <div class="session-notice-text">
                                    <span class="session-notice-project">{project}</span>
                                    <span class="session-notice-title">
                                        {move || session_notice_title(locale.get(), &notice_for_title)}
                                    </span>
                                    <span class="session-notice-status">{move || t(locale.get(), status_key)}</span>
                                </div>
                                <button type="button" class="session-notice-open"
                                    data-testid="session-notice-open"
                                    on:click=move |_| {
                                        let (project_id, session_id) = open_target.clone();
                                        cards.update(|cards| cards.retain(|card| card.notice.session_id != session_id));
                                        on_open.call((project_id, session_id));
                                    }>
                                    {move || t(locale.get(), "session_notice.open")}
                                </button>
                                <button type="button" class="session-notice-dismiss"
                                    data-testid="session-notice-dismiss"
                                    title=move || t(locale.get(), "session_notice.dismiss")
                                    aria-label=move || t(locale.get(), "session_notice.dismiss")
                                    on:click=move |_| {
                                        let id = dismiss_id.clone();
                                        cards.update(|cards| cards.retain(|card| card.notice.session_id != id));
                                    }>
                                    {compose_icon("close")}
                                </button>
                            </div>
                        }
                    }
                />
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(session: &str) -> SessionNotice {
        SessionNotice {
            project_id: "project-b".into(),
            project_name: "Project B".into(),
            session_id: session.into(),
            session_title: format!("Title {session}"),
            kind: "done".into(),
        }
    }

    fn ids(cards: &[SessionNoticeCard]) -> Vec<&str> {
        cards
            .iter()
            .map(|card| card.notice.session_id.as_str())
            .collect()
    }

    #[test]
    fn notices_are_newest_first_deduplicated_and_bounded() {
        let mut cards = Vec::new();
        for session in ["s1", "s2", "s3"] {
            push_session_notice(&mut cards, notice(session));
        }
        assert_eq!(ids(&cards), ["s3", "s2", "s1"]);
        // A second completion of s1 moves it to the top instead of duplicating,
        // as a new arrival so its linger starts over.
        let first_arrival = cards[2].arrival;
        push_session_notice(&mut cards, notice("s1"));
        assert_eq!(ids(&cards), ["s1", "s3", "s2"]);
        assert_ne!(cards[0].arrival, first_arrival);
        push_session_notice(&mut cards, notice("s4"));
        assert_eq!(ids(&cards), ["s4", "s1", "s3"]);
    }

    #[test]
    fn a_card_clears_only_after_the_inbox_listed_and_then_dropped_it() {
        let mut cards = Vec::new();
        push_session_notice(&mut cards, notice("s1"));
        // The inbox refresh raced the turn end and does not list s1 yet.
        assert!(!reconcile_session_notices(&mut cards, &HashSet::new()));
        assert_eq!(ids(&cards), ["s1"]);
        assert!(reconcile_session_notices(
            &mut cards,
            &HashSet::from(["s1"])
        ));
        assert!(cards[0].listed);
        // The reply was viewed somewhere: the inbox no longer lists it.
        assert!(reconcile_session_notices(&mut cards, &HashSet::new()));
        assert!(cards.is_empty());
    }

    #[test]
    fn only_finished_conversations_leave_on_their_own() {
        assert!(session_notice_autocloses(&notice("s1")));
        let mut failed = notice("s2");
        failed.kind = "error".into();
        assert!(!session_notice_autocloses(&failed));
    }

    #[test]
    fn desktop_body_names_the_project_first() {
        assert_eq!(
            session_notice_body(Locale::En, &notice("s1")),
            "Project B · Title s1"
        );
        let mut untitled = notice("s2");
        untitled.project_name.clear();
        untitled.session_title.clear();
        assert_eq!(
            session_notice_body(Locale::En, &untitled),
            t(Locale::En, "sidebar.untitled")
        );
    }
}
