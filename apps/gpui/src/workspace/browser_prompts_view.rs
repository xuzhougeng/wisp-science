//! What the app asks or tells about the user's own browser over the open
//! project, as the WebView does: a dialog that offers to close the tabs an
//! ended turn opened, a dialog that asks the user to complete a
//! human-verification challenge in the browser (and goes on with the
//! conversation once they have), and the bar above the composer that says
//! the browser extension has to be updated. The host is asked every two
//! seconds, as the WebView polls the extension.
use crate::i18n::{fill, tr};
use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::{BrowserNeedsHumanTab, native_conversations::SendRequest};

use super::{
    Workspace,
    shell::glyph,
    surfaces::{self, DialogWidth},
};
use crate::{assets::icon, backend::receive, browser, layers::Layer};

impl Workspace {
    /// Asks the host about the browser every two seconds, for as long as
    /// the window is open.
    pub(super) fn watch_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        self.browser_watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let project = this
                    .update(cx, |this, _| this.shown_project())
                    .ok()
                    .flatten();
                let alerts = match &project {
                    Some(project) => receive(backend.browser_alerts(project.clone())).await.ok(),
                    None => Some(Default::default()),
                };
                let gone = this
                    .update_in(cx, |this, window, cx| {
                        // Another project's questions are not this one's.
                        if this.browser_project != project {
                            this.browser_prompts = Default::default();
                            this.browser_project = project.clone();
                        }
                        if let Some(alerts) = alerts.filter(|_| this.shown_project() == project) {
                            this.browser_prompts.accept(alerts);
                            this.sync_browser_layers(window, cx);
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        }));
    }

    /// Asks the host again now, after an answer changed what it holds.
    fn refresh_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.shown_project() else {
            return;
        };
        let reply = self.backend.browser_alerts(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.shown_project().as_ref() != Some(&project) {
                    return;
                }
                if let Ok(alerts) = result {
                    this.browser_prompts.accept(alerts);
                    this.sync_browser_layers(window, cx);
                }
            });
        })
        .detach();
    }

    /// The two dialogs join the Escape stack while they show, the question
    /// about a human over the one about tabs, as the WebView draws them.
    fn sync_browser_layers(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let cleanup = self.browser_prompts.cleanup.is_some();
        let human = !self.browser_prompts.human.is_empty();
        let mut opened = false;
        if cleanup && !self.layers.contains(Layer::BrowserTabCleanup) {
            self.layers.open(Layer::BrowserTabCleanup);
            opened = true;
        } else if !cleanup {
            self.layers.close(Layer::BrowserTabCleanup);
        }
        if human && (opened || !self.layers.contains(Layer::BrowserNeedsHuman)) {
            self.layers.open(Layer::BrowserNeedsHuman);
        } else if !human {
            self.layers.close(Layer::BrowserNeedsHuman);
        }
        // The update bar is reached by Escape after everything else.
        if self.browser_prompts.banner().is_some() {
            self.layers.open_beneath(Layer::BrowserExtensionUpdate);
        } else {
            self.layers.close(Layer::BrowserExtensionUpdate);
        }
        cx.notify();
    }

    /// Escape on the tab question is Keep all; on the human question,
    /// Later; on the update bar, Dismiss. While an answer is on the way the
    /// dialog stays.
    pub(super) fn escape_browser_prompt(
        &mut self,
        layer: Layer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match layer {
            Layer::BrowserTabCleanup if self.browser_prompts.cleanup_busy => {
                self.layers.open(Layer::BrowserTabCleanup)
            }
            Layer::BrowserTabCleanup => self.keep_browser_tabs(window, cx),
            Layer::BrowserNeedsHuman if self.browser_prompts.human_busy => {
                self.layers.open(Layer::BrowserNeedsHuman)
            }
            Layer::BrowserNeedsHuman => self.browser_prompts.put_off(),
            _ => self.browser_prompts.dismiss_banner(),
        }
        cx.notify();
    }

    fn keep_browser_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompts = &mut self.browser_prompts;
        let Some(turn) = prompts
            .cleanup
            .as_ref()
            .map(|prompt| prompt.turn_id.clone())
        else {
            return;
        };
        if prompts.cleanup_busy {
            return;
        }
        prompts.cleanup_busy = true;
        let reply = self.backend.dismiss_browser_tab_cleanup(turn);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => this.browser_prompts.cleanup_answered(),
                    Err(error) => {
                        this.browser_prompts.cleanup_busy = false;
                        this.browser_prompts.cleanup_error = Some(error);
                    }
                }
                this.sync_browser_layers(window, cx);
                this.refresh_browser(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn close_browser_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompts = &mut self.browser_prompts;
        let Some(turn) = prompts
            .cleanup
            .as_ref()
            .map(|prompt| prompt.turn_id.clone())
        else {
            return;
        };
        if prompts.cleanup_busy {
            return;
        }
        prompts.cleanup_busy = true;
        prompts.cleanup_error = None;
        let reply = self
            .backend
            .confirm_browser_tab_cleanup(turn, prompts.chosen());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.browser_prompts.cleanup_answered(),
                    // The question stays, with what went wrong.
                    Err(error) => {
                        this.browser_prompts.cleanup_busy = false;
                        this.browser_prompts.cleanup_error = Some(error);
                    }
                }
                this.sync_browser_layers(window, cx);
                this.refresh_browser(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// I completed verification: the host looks at the tabs again; with the
    /// challenge gone, the conversation that met it is told to go on.
    fn browser_verified(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompts = &mut self.browser_prompts;
        if prompts.human_busy || prompts.human.is_empty() {
            return;
        }
        prompts.human_busy = true;
        prompts.human_error = None;
        let reply = self
            .backend
            .confirm_browser_needs_human(prompts.human.clone());
        let project = self.shown_project();
        let fallback = self.state.session.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ =
                this.update_in(cx, |this, window, cx| {
                    let here = this.shown_project() == project;
                    if here {
                        this.browser_prompts.human_busy = false;
                    }
                    match result {
                        Ok(result) if result.still_required.is_empty() => {
                            if here {
                                this.browser_prompts.human.clear();
                                this.browser_prompts.human_error = None;
                            }
                            let frame = result
                                .cleared
                                .first()
                                .map(|tab| tab.frame_id.clone())
                                .filter(|frame| !frame.is_empty())
                                .or(fallback);
                            if let (Some(project), Some(frame)) = (project, frame) {
                                this.continue_after_verification(project, frame, window, cx);
                            }
                        }
                        Ok(result) if here => {
                            this.browser_prompts.human = result.still_required;
                            this.browser_prompts.human_error = Some(
                            tr("The challenge is still on the page. Complete it in the browser \
                                tab, then try again.")
                                .to_owned(),
                        );
                        }
                        Ok(_) => {}
                        Err(_) if here => {
                            this.browser_prompts.human_error = Some(
                                tr("Could not re-check the tab. The browser extension may be \
                                disconnected.")
                                .to_owned(),
                            );
                        }
                        Err(_) => {}
                    }
                    this.sync_browser_layers(window, cx);
                });
        })
        .detach();
        cx.notify();
    }

    /// Sends the WebView's message that goes on with the conversation the
    /// challenge stopped, as a new turn of it: it can be stopped like any.
    fn continue_after_verification(
        &mut self,
        project: String,
        frame: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.send_message(
            project,
            SendRequest {
                session_id: frame.clone(),
                request_id: uuid::Uuid::new_v4().to_string(),
                message: tr("I completed the human verification in the browser. Continue.")
                    .to_owned(),
                resume: false,
                resume_after: None,
                attachments: Vec::new(),
                references: Vec::new(),
                acp_agent_id: None,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.watch_activity(window, cx);
                        this.load_sessions(window, cx);
                        if this.state.session.as_deref() == Some(frame.as_str()) {
                            this.start_poll(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_browser_tab(&mut self, tab: i64, cx: &mut Context<Self>) {
        if self.browser_prompts.human_busy {
            return;
        }
        // Bringing the tab forward is best effort, as in the WebView.
        let reply = self.backend.focus_browser_needs_human(tab);
        cx.spawn(async move |_, _| {
            let _ = receive(reply).await;
        })
        .detach();
    }

    fn update_browser_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompts = &mut self.browser_prompts;
        if prompts.update_busy {
            return;
        }
        prompts.update_busy = true;
        prompts.update_error = None;
        let reply = self.backend.update_browser_extension();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let prompts = &mut this.browser_prompts;
                prompts.update_busy = false;
                match result {
                    Ok(result) => {
                        prompts.update_error =
                            result.error.clone().or_else(|| result.status.error.clone());
                        if result.outcome == "updated" {
                            prompts.extension = None;
                            prompts.update_outcome = None;
                        } else {
                            // The folder to load is copied for the picker,
                            // as the WebView copies it.
                            if result.outcome == "manual_reload_required" {
                                if let Some(path) = result.status.extension_path.clone() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                                }
                            }
                            prompts.update_outcome = Some(result.outcome);
                            prompts.extension = Some(result.status);
                        }
                    }
                    Err(error) => prompts.update_error = Some(error),
                }
                this.sync_browser_layers(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn open_browser_extension_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.open_browser_extension_page();
        cx.spawn_in(window, async move |this, cx| {
            let opened = receive(reply).await.is_ok_and(|setup| setup.opened);
            let _ = this.update_in(cx, |this, _, cx| {
                if !opened {
                    this.browser_prompts.update_error = Some(
                        tr("Could not open the extension page automatically. Open \
                            chrome://extensions in your browser.")
                        .to_owned(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The bar above the composer while the extension has to be updated.
    pub(super) fn render_browser_extension_bar(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let banner = self.browser_prompts.banner()?;
        let muted = cx.theme().muted_foreground;
        let busy = self.browser_prompts.update_busy;
        let mut copy = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Browser extension update")),
            )
            .child(div().text_sm().child(banner.title))
            .child(div().text_xs().text_color(muted).child(banner.body));
        if let Some(path) = &banner.path {
            copy = copy.child(
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .truncate()
                    .child(path.clone()),
            );
        }
        if banner.manual {
            for (index, step) in browser::setup_steps(cfg!(target_os = "macos"))
                .into_iter()
                .enumerate()
            {
                copy = copy.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(fill("{}. {}", &[&(index + 1), &step])),
                );
            }
        }
        if let Some(error) = banner.error {
            copy = copy.child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(icon("circle-alert").text_color(cx.theme().danger))
                    .child(div().text_xs().child(error)),
            );
        }
        let mut actions = div().h_flex().flex_wrap().gap_1();
        if !banner.manual {
            actions =
                actions.child(
                    Button::new("browser-extension-update")
                        .label(if busy {
                            tr("Updating…")
                        } else {
                            tr("Update extension")
                        })
                        .small()
                        .primary()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.update_browser_extension(window, cx)
                        })),
                );
        }
        if let Some(path) = banner.path {
            actions = actions.child(
                Button::new("browser-extension-copy")
                    .label(tr("Copy extension path"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                    })),
            );
        }
        actions = actions
            .child(
                Button::new("browser-extension-page")
                    .label(tr("Open extension page"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_browser_extension_page(window, cx)
                    })),
            )
            .child(
                Button::new("browser-extension-recheck")
                    .label(tr("Recheck"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.refresh_browser(window, cx))),
            )
            .child(
                Button::new("browser-extension-dismiss")
                    .label(tr("Dismiss"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.browser_prompts.dismiss_banner();
                        this.sync_browser_layers(window, cx);
                    })),
            );
        Some(
            surfaces::bar("browser-extension-update", cx)
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .child(icon("refresh").text_color(cx.theme().warning))
                        .child(copy),
                )
                .child(actions),
        )
    }

    /// Close tabs Wisp opened? The ended turn's tabs, all ticked.
    pub(super) fn render_browser_cleanup(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompts = &self.browser_prompts;
        let prompt = prompts.cleanup.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let busy = prompts.cleanup_busy;
        let mut list = div()
            .id("browser-cleanup-tabs")
            .v_flex()
            .max_h(px(320.))
            .overflow_y_scroll();
        for (index, tab) in prompt.tabs.iter().enumerate() {
            let picked = prompts.selected.contains(&tab.tab_id);
            let id = tab.tab_id;
            list = list.child(
                div()
                    .id(("browser-cleanup-tab", index))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(glyph(if picked { "check-square" } else { "square" }).small())
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(browser::tab_name(&tab.title, &tab.url)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .truncate()
                                    .child(tab.url.clone()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.browser_prompts.cleanup_busy {
                            this.browser_prompts.toggle(id);
                            cx.notify();
                        }
                    })),
            );
        }
        let dialog = surfaces::dialog(
            "browser-cleanup",
            DialogWidth::Regular,
            tr("Close tabs Wisp opened?"),
        )
        .gap_3()
        .child(div().text_xs().text_color(muted).child(fill(
            tr(
                "This turn opened {} tabs. Selected tabs will be closed. Uncheck any you want \
                to keep.",
            ),
            &[&prompt.tabs.len()],
        )))
        .child(list)
        .children(
            prompts
                .cleanup_error
                .clone()
                .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
        )
        .action(
            Button::new("browser-cleanup-keep")
                .label(tr("Keep all"))
                .small()
                .ghost()
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.keep_browser_tabs(window, cx))),
        )
        .action(
            Button::new("browser-cleanup-close")
                .label(tr("Close selected"))
                .small()
                .primary()
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.close_browser_tabs(window, cx))),
        );
        Some(dialog.into_any_element())
    }

    /// Human verification needed: the tabs whose page waits for the user.
    pub(super) fn render_browser_needs_human(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompts = &self.browser_prompts;
        if prompts.human.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let busy = prompts.human_busy;
        let mut list = div()
            .id("browser-human-tabs")
            .v_flex()
            .max_h(px(320.))
            .overflow_y_scroll();
        for (index, tab) in prompts.human.iter().enumerate() {
            let shown: BrowserNeedsHumanTab = tab.clone();
            list = list.child(
                div()
                    .id(("browser-human-tab", index))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(glyph("shield").small())
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(browser::tab_name(&tab.title, &tab.url)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .truncate()
                                    .child(tab.url.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("Show this tab")),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.show_browser_tab(shown.tab_id, cx)),
                    ),
            );
        }
        let dialog = surfaces::dialog(
            "browser-human",
            DialogWidth::Regular,
            tr("Human verification needed"),
        )
        .gap_3()
        .child(div().text_sm().child(tr(
            "This page needs you to complete a human-verification challenge in the visible \
             browser tab. Wisp will not click or bypass it, and will keep the tab open until \
             you finish.",
        )))
        .child(div().text_xs().text_color(muted).child(tr(
            "Complete the challenge in the browser. Do not close that tab or quit the browser \
             until it succeeds.",
        )))
        .child(list)
        .children(
            prompts
                .human_error
                .clone()
                .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
        )
        .action(
            Button::new("browser-human-later")
                .label(tr("Later"))
                .small()
                .ghost()
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.browser_prompts.put_off();
                    this.layers.close(Layer::BrowserNeedsHuman);
                    cx.notify();
                })),
        )
        .action(
            Button::new("browser-human-done")
                .label(tr("I completed verification"))
                .small()
                .primary()
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.browser_verified(window, cx))),
        );
        Some(dialog.into_any_element())
    }
}
