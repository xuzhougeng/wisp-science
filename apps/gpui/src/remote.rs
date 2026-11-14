//! Remote web access as Settings words it.
use wisp_server::desktop::{RemoteAccess, RemoteState};

use crate::{
    i18n::{fill, tr},
    present::Tone,
};

/// Where the tunnel stands, in a line, and how it should look.
pub fn status(access: &RemoteAccess) -> (String, Tone) {
    match access.state {
        RemoteState::Off => (tr("Off").to_owned(), Tone::Neutral),
        RemoteState::Connecting => (tr("Connecting to the relay…").to_owned(), Tone::Active),
        RemoteState::Connected => (
            fill(
                tr("Connected to the relay · browsers connected: {}"),
                &[&access.clients],
            ),
            Tone::Success,
        ),
        RemoteState::Failed => {
            let why = access.detail.lines().next().unwrap_or_default().trim();
            let line = if why.is_empty() {
                tr("Not connected; trying again.").to_owned()
            } else {
                fill(tr("Not connected: {} Trying again."), &[&why])
            };
            (line, Tone::Danger)
        }
    }
}

/// What the token field is for, given whether one is kept.
pub fn token_note(access: &RemoteAccess) -> &'static str {
    if access.has_token {
        tr("A token is saved. Type another to replace it.")
    } else {
        tr("The relay's WISP_RELAY_TOKEN. It is kept in the system keyring.")
    }
}

/// Whether saving would change anything: another address, or a token.
pub fn changed(access: &RemoteAccess, url: &str, token: &str) -> bool {
    url.trim() != access.relay_url || !token.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_says_where_it_stands() {
        let mut access = RemoteAccess::default();
        assert_eq!(status(&access), ("Off".to_owned(), Tone::Neutral));
        access.state = RemoteState::Connecting;
        assert_eq!(status(&access).1, Tone::Active);
        access.state = RemoteState::Connected;
        access.clients = 2;
        assert_eq!(
            status(&access),
            (
                "Connected to the relay · browsers connected: 2".to_owned(),
                Tone::Success
            )
        );
        access.state = RemoteState::Failed;
        assert_eq!(status(&access).0, "Not connected; trying again.");
        access.detail = "HTTP error: 401 Unauthorized\nmore".into();
        assert_eq!(
            status(&access),
            (
                "Not connected: HTTP error: 401 Unauthorized Trying again.".to_owned(),
                Tone::Danger
            )
        );
    }

    #[test]
    fn the_form_knows_when_there_is_something_to_save() {
        let mut access = RemoteAccess {
            relay_url: "https://relay.example".into(),
            ..RemoteAccess::default()
        };
        assert!(!changed(&access, " https://relay.example ", "  "));
        assert!(changed(&access, "https://other.example", ""));
        assert!(changed(&access, "https://relay.example", "made-up"));
        assert!(token_note(&access).starts_with("The relay's"));
        access.has_token = true;
        assert!(token_note(&access).starts_with("A token is saved"));
    }
}
