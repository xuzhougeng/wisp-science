//! How the chat apps that talk to Wisp read on their Settings page: where
//! a bot's connection stands and how far a QR scan has got.
use crate::{
    i18n::{fill, host, tr},
    present::Tone,
};
use wisp_server::desktop::{ChannelState, WeixinScan, WeixinStatus};

/// The line under WeChat's name, and its colour.
pub fn weixin_line(status: &WeixinStatus) -> (String, Tone) {
    if !status.bound {
        return (tr("Not bound").to_owned(), Tone::Neutral);
    }
    if !status.enabled {
        return (tr("Bound · off").to_owned(), Tone::Neutral);
    }
    let detail = host(status.detail.lines().next().unwrap_or_default().trim());
    match status.state {
        ChannelState::Running => (
            tr("Connected; waiting for messages").to_owned(),
            Tone::Success,
        ),
        ChannelState::Connecting | ChannelState::Stopped => {
            (tr("Connecting…").to_owned(), Tone::Active)
        }
        ChannelState::Error if detail.is_empty() => (tr("Not connected").to_owned(), Tone::Danger),
        ChannelState::Error => (fill(tr("Not connected: {}"), &[&detail]), Tone::Danger),
    }
}

/// What the QR code's panel says while the owner scans.
pub fn scan_note(scan: Option<WeixinScan>) -> &'static str {
    match scan {
        None | Some(WeixinScan::Waiting) => {
            tr("Scan this code with WeChat on your phone, then confirm there.")
        }
        Some(WeixinScan::Scanned) => tr("Scanned; confirm on your phone."),
        Some(WeixinScan::Confirmed) => tr("Bound."),
        Some(WeixinScan::Expired) => tr("The code expired; get a new one."),
    }
}

/// Whether the scan is still worth asking about.
pub fn scanning(scan: Option<WeixinScan>) -> bool {
    matches!(scan, None | Some(WeixinScan::Waiting | WeixinScan::Scanned))
}

/// The commands a chat can send, as the page lists them.
pub const COMMANDS: [(&str, &str); 8] = [
    ("/project", "list projects, or switch the one chats go to"),
    ("/session", "list that project's conversations, or switch"),
    ("/new", "start a new conversation with the next message"),
    ("/status", "say where chats go"),
    ("/approval", "list approvals waiting"),
    ("/approve", "approve one, by its code"),
    ("/reject", "refuse one, by its code, with a reason"),
    ("/stop", "stop the running turn"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn status(bound: bool, enabled: bool, state: ChannelState, detail: &str) -> WeixinStatus {
        WeixinStatus {
            enabled,
            bound,
            state,
            detail: detail.into(),
        }
    }

    #[test]
    fn the_line_says_whether_wechat_is_bound_on_and_connected() {
        assert_eq!(
            weixin_line(&status(false, false, ChannelState::Stopped, "")).0,
            "Not bound"
        );
        assert_eq!(
            weixin_line(&status(true, false, ChannelState::Stopped, "")).0,
            "Bound · off"
        );
        let (line, tone) = weixin_line(&status(true, true, ChannelState::Running, "Connected"));
        assert_eq!(line, "Connected; waiting for messages");
        assert!(matches!(tone, Tone::Success));
        let (line, tone) = weixin_line(&status(
            true,
            true,
            ChannelState::Error,
            "The WeChat sign-in expired; bind again with a new QR code\nmore",
        ));
        assert_eq!(
            line,
            "Not connected: The WeChat sign-in expired; bind again with a new QR code"
        );
        assert!(matches!(tone, Tone::Danger));
    }

    #[test]
    fn a_scan_is_followed_until_it_is_confirmed_or_expires() {
        assert!(scanning(None));
        assert!(scanning(Some(WeixinScan::Scanned)));
        assert!(!scanning(Some(WeixinScan::Confirmed)));
        assert!(!scanning(Some(WeixinScan::Expired)));
        assert!(scan_note(Some(WeixinScan::Expired)).contains("expired"));
    }
}
