//! How the chat apps that talk to Wisp read on their Settings page: where
//! a bot's connection stands and how far a QR scan has got.
use crate::{
    i18n::{fill, host, tr},
    present::Tone,
};
use wisp_server::desktop::{
    ChannelState, FeishuStatus, LabMemberInfo, LabStatus, WeixinDestination, WeixinQr, WeixinScan,
    WeixinStatus,
};

/// One WeChat bot's card: where it stands, the QR code being scanned and
/// how far, which bind attempt is current, and how the last action went.
#[derive(Default)]
pub struct WeixinCard {
    pub status: Option<WeixinStatus>,
    pub qr: Option<WeixinQr>,
    pub scan: Option<WeixinScan>,
    pub bind: u64,
    pub note: Option<Result<String, String>>,
}

/// The card's heading.
pub fn weixin_title(destination: WeixinDestination) -> &'static str {
    match destination {
        WeixinDestination::Projects => tr("WeChat"),
        WeixinDestination::Assistant => tr("Research assistant on WeChat"),
    }
}

/// What the card's bot is for.
pub fn weixin_about(destination: WeixinDestination) -> &'static str {
    match destination {
        WeixinDestination::Projects => tr(
            "A WeChat bot through Tencent's iLink service. Only the person who scanned \
             the code is answered, one to one. The bot's token is kept in the system \
             keychain and shared with the WebView desktop.",
        ),
        WeixinDestination::Assistant => tr(
            "A second WeChat bot that talks only to the research assistant: every message \
             goes to its conversation, yes, no and full answer what it asks, and its \
             mandate notices arrive with your next message. Bind a different bot from \
             the one above.",
        ),
    }
}

/// What is said once a scan binds the bot.
pub fn weixin_bound_note(destination: WeixinDestination) -> &'static str {
    match destination {
        WeixinDestination::Projects => tr("WeChat is bound. Turn it on to answer messages."),
        WeixinDestination::Assistant => {
            tr("The assistant's WeChat is bound. Turn it on to answer messages.")
        }
    }
}

/// The question before the bot is unbound.
pub fn weixin_unbind_question(destination: WeixinDestination) -> &'static str {
    match destination {
        WeixinDestination::Projects => {
            tr("Unbind WeChat? The bot stops and its token is forgotten on this computer.")
        }
        WeixinDestination::Assistant => tr(
            "Unbind the assistant's WeChat? Its bot stops and its token is forgotten on this computer.",
        ),
    }
}

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

/// The line under the lab's name, and its colour.
pub fn lab_line(status: &LabStatus) -> (String, Tone) {
    match status.state.as_str() {
        "active" if status.leader => (
            fill(tr("{} · you lead it"), &[&status.lab_name]),
            Tone::Success,
        ),
        "active" => (fill(tr("{} · member"), &[&status.lab_name]), Tone::Success),
        "pending" => (
            fill(
                tr("{} · waiting for the leader's approval"),
                &[&status.lab_name],
            ),
            Tone::Attention,
        ),
        "removed" => (
            tr("The leader removed this computer from the lab").to_owned(),
            Tone::Danger,
        ),
        "error" if status.detail.trim().is_empty() => {
            (tr("The lab could not be reached").to_owned(), Tone::Danger)
        }
        "error" => (
            fill(
                tr("The lab could not be reached: {}"),
                &[&host(status.detail.trim())],
            ),
            Tone::Danger,
        ),
        _ => (tr("Not in a lab").to_owned(), Tone::Neutral),
    }
}

/// A member as the list names them.
pub fn member_label(member: &LabMemberInfo, me: &str) -> String {
    let mut label = member.name.clone();
    if member.leader {
        label.push_str(&format!(" · {}", tr("leader")));
    }
    if member.id == me {
        label.push_str(&format!(" · {}", tr("this computer")));
    }
    label
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
    fn each_wechat_bot_says_what_it_is_for() {
        let both = [WeixinDestination::Projects, WeixinDestination::Assistant];
        for destination in both {
            assert!(!weixin_title(destination).is_empty());
            assert!(!weixin_bound_note(destination).is_empty());
        }
        assert_ne!(
            weixin_title(WeixinDestination::Projects),
            weixin_title(WeixinDestination::Assistant)
        );
        assert!(weixin_about(WeixinDestination::Assistant).contains("research assistant"));
        assert!(weixin_unbind_question(WeixinDestination::Assistant).contains("assistant"));
        let card = WeixinCard::default();
        assert!(card.status.is_none() && card.qr.is_none() && card.bind == 0);
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
    fn the_lab_line_says_where_this_computer_stands() {
        let mut status = LabStatus {
            state: "none".into(),
            lab_name: "Genomics".into(),
            ..Default::default()
        };
        assert_eq!(lab_line(&status).0, "Not in a lab");
        status.state = "pending".into();
        assert_eq!(
            lab_line(&status).0,
            "Genomics · waiting for the leader's approval"
        );
        status.state = "active".into();
        status.leader = true;
        assert_eq!(lab_line(&status).0, "Genomics · you lead it");
        status.state = "error".into();
        status.detail = "connection refused".into();
        assert_eq!(
            lab_line(&status).0,
            "The lab could not be reached: connection refused"
        );
        let member = LabMemberInfo {
            id: "m1".into(),
            name: "Ada".into(),
            leader: true,
            pending: false,
        };
        assert_eq!(member_label(&member, "m1"), "Ada · leader · this computer");
        assert_eq!(member_label(&member, "m2"), "Ada · leader");
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
