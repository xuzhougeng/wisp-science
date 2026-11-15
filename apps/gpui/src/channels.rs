//! How the chat apps that talk to Wisp read on their Settings page: where
//! a bot's connection stands and how far a QR scan has got.
use crate::{
    i18n::{fill, host, tr},
    present::Tone,
};
use wisp_server::desktop::{ChannelState, FeishuStatus, WeixinScan, WeixinStatus};

/// The line under WeChat's name, and its colour.
pub fn weixin_line(status: &WeixinStatus) -> (String, Tone) {
    line(status.bound, status.enabled, status.state, &status.detail)
}

/// The line under Feishu's name, and its colour.
pub fn feishu_line(status: &FeishuStatus) -> (String, Tone) {
    line(status.bound, status.enabled, status.state, &status.detail)
}

/// Whether the Feishu form differs from what is kept: another App ID, a
/// typed secret, or the other service.
pub fn feishu_changed(status: &FeishuStatus, app_id: &str, secret: &str, lark: bool) -> bool {
    app_id.trim() != status.app_id || !secret.trim().is_empty() || lark != status.international
}

fn line(bound: bool, enabled: bool, state: ChannelState, detail: &str) -> (String, Tone) {
    if !bound {
        return (tr("Not bound").to_owned(), Tone::Neutral);
    }
    if !enabled {
        return (tr("Bound · off").to_owned(), Tone::Neutral);
    }
    let detail = host(detail.lines().next().unwrap_or_default().trim());
    match state {
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
    fn the_feishu_form_is_saved_only_when_it_changed() {
        let status = FeishuStatus {
            enabled: true,
            bound: true,
            international: false,
            app_id: "cli_1".into(),
            has_secret: true,
            state: ChannelState::Connecting,
            detail: String::new(),
            owner: "ou_1".into(),
            pending_owner: String::new(),
        };
        assert_eq!(feishu_line(&status).0, "Connecting…");
        assert!(!feishu_changed(&status, " cli_1 ", "", false));
        assert!(feishu_changed(&status, "cli_2", "", false));
        assert!(feishu_changed(&status, "cli_1", "secret", false));
        assert!(feishu_changed(&status, "cli_1", "", true));
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
