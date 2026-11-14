//! The Browser page of Settings: the bridge to the Wisp extension in the
//! user's own browser, and the hosts to refuse or prefer.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{BrowserHostRule, BrowserSettings};

use super::{Workspace, settings_sections::setting, tone_color};
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

    fn host_list(&self, list: List, rules: &[BrowserHostRule], cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (name, at) = match list {
            List::Block => ("block", 0),
            List::Prefer => ("prefer", 2),
        };
        let mut rows = div().v_flex().gap_1().w(px(640.));
        if rules.is_empty() {
            rows = rows.child(div().text_xs().text_color(muted).child(match list {
                List::Block => tr("No blocked hosts."),
                List::Prefer => tr("No preferred hosts."),
            }));
        }
        for (index, rule) in rules.iter().enumerate() {
            rows = rows.child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(220.))
                            .truncate()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(rule.host.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(rule.reason.clone()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("browser-{name}-remove-{index}")))
                            .label(tr("Remove"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(settings) = &this.browser else {
                                    return;
                                };
                                let filters = browser::without_rule(&settings.filters, list, index);
                                let reply = this.backend.save_browser_filters(filters);
                                this.receive_browser(reply, None, window, cx);
                            })),
                    ),
            );
        }
        rows.child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
                .child(
                    div()
                        .w(px(220.))
                        .child(Input::new(&self.browser_fields[at]).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&self.browser_fields[at + 1]).small()),
                )
                .child(
                    Button::new(SharedString::from(format!("browser-{name}-add")))
                        .label(tr("Add"))
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.add_browser_host(list, window, cx)
                        })),
                ),
        )
    }

    pub(super) fn render_browser_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(settings) = &self.browser else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let enabled = settings.enabled;
        let (line, tone) = browser::status(settings);
        let heading = |title: &'static str, note: &'static str| {
            div()
                .v_flex()
                .gap_1()
                .pt_4()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().w(px(640.)).text_xs().text_color(muted).child(note))
        };
        let mut page = page
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "Lets the model open pages in your own Chrome or Chromium, with its logins, \
                 through the Wisp extension. A conversation reads and drives only the tabs it \
                 opened itself; your other tabs are off limits, and every use asks first \
                 unless you allowed the tool.",
            )))
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .py_2()
                    .child(div().text_sm().text_color(tone_color(tone, cx)).child(line))
                    .child(
                        Button::new("browser-refresh")
                            .label(tr("Refresh"))
                            .xsmall()
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_browser(window, cx)),
                            ),
                    ),
            )
            .child(setting(
                tr("Browser tools"),
                tr(
                    "On, the app listens on this computer only (127.0.0.1:18765) for the \
                     extension, and conversations get the browser tools from their next turn. \
                     Only one Wisp can listen at a time.",
                ),
                Button::new("browser-enabled")
                    .label(if enabled { tr("On") } else { tr("Off") })
                    .small()
                    .selected(enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let reply = this.backend.set_browser_enabled(!enabled);
                        this.receive_browser(reply, None, window, cx);
                    })),
                cx,
            ));
        let folder = settings.extension_path.clone();
        page = page.child(setting(
            tr("Extension"),
            tr(
                "In Chrome or Chromium open chrome://extensions, turn on Developer mode, \
                 press Load unpacked and choose this folder.",
            ),
            match folder {
                Some(folder) => div()
                    .v_flex()
                    .items_end()
                    .gap_1()
                    .child(
                        div()
                            .max_w(px(360.))
                            .truncate()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(folder.clone()),
                    )
                    .child(
                        Button::new("browser-copy-folder")
                            .label(tr("Copy path"))
                            .small()
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
        ))
        .child(self.host_list(List::Block, &settings.filters.block, cx))
        .child(heading(
            tr("Preferred hosts"),
            tr("A hint to the model for literature search and the like; other sites still open."),
        ))
        .child(self.host_list(List::Prefer, &settings.filters.prefer, cx))
    }
}
