//! The Browser page of Settings: the bridge to the Wisp extension in the
//! user's own browser, and the hosts to refuse or prefer.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt};
use wisp_server::desktop::{BrowserHostRule, BrowserSettings};

use super::{
    Workspace,
    settings_sections::explained_setting,
    surfaces::{self, switch},
    tone_color,
};
use crate::{
    backend::{Reply, receive},
    browser::{self, List},
};

impl Workspace {
    pub(super) fn load_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.browser_settings();
        self.receive_browser(reply, None, window, cx);
    }

    /// Takes what the host keeps. `added` names the list a host was just
    /// added to, whose fields are then emptied.
    fn receive_browser(
        &mut self,
        reply: Reply<BrowserSettings>,
        added: Option<List>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(settings) => {
                        this.browser = Some(settings);
                        if let Some(list) = added {
                            let at = if list == List::Block { 0 } else { 2 };
                            for field in &this.browser_fields[at..at + 2] {
                                field.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                        }
                    }
                    // A refused change keeps what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn add_browser_host(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = &self.browser else {
            return;
        };
        let at = if list == List::Block { 0 } else { 2 };
        let host = self.browser_fields[at].read(cx).value().to_string();
        let reason = self.browser_fields[at + 1].read(cx).value().to_string();
        let filters = browser::with_rule(&settings.filters, list, &host, &reason);
        let reply = self.backend.save_browser_filters(filters);
        self.receive_browser(reply, Some(list), window, cx);
    }

    /// The hosts of one list as record rows, each removed by an icon
    /// button in the danger colour; the fields to add one under them.
    fn host_list(&self, list: List, rules: &[BrowserHostRule], cx: &mut Context<Self>) -> Div {
        let (name, at, icon) = match list {
            List::Block => ("block", 0, "ban"),
            List::Prefer => ("prefer", 2, "star"),
        };
        let mut section = div().v_flex().gap_2();
        if rules.is_empty() {
            section = section.child(surfaces::settings_note(
                match list {
                    List::Block => tr("No blocked hosts."),
                    List::Prefer => tr("No preferred hosts."),
                },
                cx,
            ));
        }
        let rows: Vec<_> = rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let mut row = surfaces::record_row(
                    SharedString::from(format!("browser-{name}-{index}")),
                    rule.host.clone(),
                )
                .icon(icon)
                .mono();
                if !rule.reason.is_empty() {
                    row = row.suffix(rule.reason.clone());
                }
                row.action(
                    surfaces::danger_action(
                        SharedString::from(format!("browser-{name}-remove-{index}")),
                        "trash",
                        tr("Remove"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(settings) = &this.browser else {
                            return;
                        };
                        let filters = browser::without_rule(&settings.filters, list, index);
                        let reply = this.backend.save_browser_filters(filters);
                        this.receive_browser(reply, None, window, cx);
                    })),
                )
            })
            .collect();
        if !rows.is_empty() {
            section = section.child(surfaces::record_group(rows, cx));
        }
        section.child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
                .child(
                    div()
                        .w(px(200.))
                        .child(surfaces::input(&self.browser_fields[at], cx).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(surfaces::input(&self.browser_fields[at + 1], cx).small()),
                )
                .child(
                    surfaces::outlined_action(
                        SharedString::from(format!("browser-{name}-add")),
                        "plus",
                        tr("Add"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.add_browser_host(list, window, cx)
                    })),
                ),
        )
    }

    pub(super) fn render_browser_settings(&self, cx: &mut Context<Self>) -> Div {
        let page = self.settings_page(cx);
        let Some(settings) = &self.browser else {
            return page.child(surfaces::settings_note(tr("Loading…"), cx));
        };
        let enabled = settings.enabled;
        let (line, tone) = browser::status(settings);
        // A heading's explanation is its help icon's tooltip.
        let heading = |title: &'static str, note: &'static str, cx: &App| {
            surfaces::settings_heading_help(
                title,
                SharedString::from(format!("browser-help-{title}")),
                note,
                cx,
            )
        };
        let mut page = page
            .child(surfaces::settings_note_help(
                tr("Lets the model use your own Chrome through the Wisp extension."),
                tr(
                    "Lets the model open pages in your own Chrome or Chromium, with its logins, \
                     through the Wisp extension. A conversation reads and drives only the tabs it \
                     opened itself; your other tabs are off limits, and every use asks first \
                     unless you allowed the tool.",
                ),
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(div().text_sm().text_color(tone_color(tone, cx)).child(line))
                    .child(
                        surfaces::icon_action(
                            "browser-refresh",
                            "refresh",
                            tr("Refresh"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.load_browser(window, cx))),
                    ),
            )
            .child(explained_setting(
                tr("Browser tools"),
                "",
                tr(
                    "On, the app listens on this computer only (127.0.0.1:18765) for the \
                     extension, and conversations get the browser tools from their next turn. \
                     Only one Wisp can listen at a time.",
                ),
                switch("browser-enabled", enabled, tr("Browser tools")).on_change(cx.listener(
                    move |this, _, window, cx| {
                        let reply = this.backend.set_browser_enabled(!enabled);
                        this.receive_browser(reply, None, window, cx);
                    },
                )),
                cx,
            ))
            // The WebView's two behaviours, under its settings keys.
            .child(explained_setting(
                tr("Open browser automatically"),
                "",
                tr(
                    "When a browser tool needs Chrome/Chromium and it isn't running, start it so \
                     the Wisp extension can reconnect. Uses your existing profile, not a \
                     temporary automation browser.",
                ),
                switch(
                    "browser-auto-launch",
                    settings.auto_launch,
                    tr("Open browser automatically"),
                )
                .on_change(cx.listener(|this, on: &bool, window, cx| {
                    let reply = this.backend.set_browser_auto_launch(*on);
                    this.receive_browser(reply, None, window, cx);
                })),
                cx,
            ))
            .child(explained_setting(
                tr("Automatically close browser tabs"),
                "",
                tr(
                    "When a conversation turn ends, close tabs Wisp opened during that turn. \
                     Tabs you already had open are left alone.",
                ),
                switch(
                    "browser-auto-close-tabs",
                    settings.auto_close_tabs,
                    tr("Automatically close browser tabs"),
                )
                .on_change(cx.listener(|this, on: &bool, window, cx| {
                    let reply = this.backend.set_browser_auto_close_tabs(*on);
                    this.receive_browser(reply, None, window, cx);
                })),
                cx,
            ));
        let folder = settings.extension_path.clone();
        page = page.child(explained_setting(
            tr("Extension"),
            "",
            tr(
                "In Chrome or Chromium open chrome://extensions, turn on Developer mode, \
                 press Load unpacked and choose this folder.",
            ),
            match folder {
                Some(folder) => div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .child(surfaces::read_only(folder.clone(), cx).max_w(px(320.)))
                    .child(
                        surfaces::icon_action(
                            "browser-copy-folder",
                            "copy",
                            tr("Copy path"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(folder.clone()));
                        })),
                    ),
                None => div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(tr("The extension folder was not found beside this app.")),
            },
            cx,
        ));
        page.child(heading(
            tr("Blocked hosts"),
            tr(
                "Refused before a tab is opened or a script navigates there. A host covers \
                 itself and all its subdomains.",
            ),
            cx,
        ))
        .child(self.host_list(List::Block, &settings.filters.block, cx))
        .child(heading(
            tr("Preferred hosts"),
            tr("A hint to the model for literature search and the like; other sites still open."),
            cx,
        ))
        .child(self.host_list(List::Prefer, &settings.filters.prefer, cx))
    }
}
