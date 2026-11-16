//! Privacy mode, as the WebView has it: projects chosen to be hidden while
//! the screen is recorded or shared. Their cards, their conversations in
//! the recent list and their rows in the palette are left out until it is
//! turned off. The choice is the store's, so both desktops read it.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{
    Workspace,
    shell::glyph,
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, layers::Layer};

impl Workspace {
    /// Reads which projects are hidden and whether that is on.
    pub(super) fn load_privacy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.privacy_mode();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(mode) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.privacy = mode;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Opens the dialog with the hidden projects ticked, or closes it.
    pub(super) fn toggle_privacy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::PrivacyMode) {
            self.privacy_picks.clear();
            cx.notify();
            return;
        }
        self.privacy_picks = self.privacy.project_ids.iter().cloned().collect();
        self.layers.open(Layer::PrivacyMode);
        self.load_privacy(window, cx);
        cx.notify();
    }

    /// Turns privacy mode on for the ticked projects, or off for all.
    fn set_privacy(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        let projects: Vec<String> = if active {
            self.privacy_picks.iter().cloned().collect()
        } else {
            Vec::new()
        };
        self.error = None;
        let reply = self.backend.set_privacy_mode(active, projects);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.layers.close(Layer::PrivacyMode);
                        this.privacy_picks.clear();
                        this.notice = Some(if active {
                            tr("Privacy mode is on.").into()
                        } else {
                            tr("Privacy mode is off.").into()
                        });
                        this.load_privacy(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_privacy(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("privacy-projects")
            .v_flex()
            .max_h(px(320.))
            .overflow_y_scroll();
        for (index, project) in self.projects.iter().enumerate() {
            let picked = self.privacy_picks.contains(&project.id);
            let id = project.id.clone();
            list = list.child(
                div()
                    .id(("privacy-project", index))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(glyph(if picked { "check-square" } else { "circle" }).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(project.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.privacy_picks.remove(&id) {
                            this.privacy_picks.insert(id.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        surfaces::dialog("privacy", DialogWidth::Narrow, tr("Privacy mode"))
            .gap_3()
            .close_on_click_outside(
                cx.listener(|this, _, window, cx| this.toggle_privacy(window, cx)),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Choose projects to conceal while recording. Their project cards, recent \
                 conversations and search results are hidden until it is turned off.",
            )))
            .child(div().text_xs().child(if self.privacy.active {
                tr("Privacy mode is on.")
            } else {
                tr("Privacy mode is off.")
            }))
            .child(list)
            .aside(
                Button::new("privacy-all")
                    .label(tr("Select all"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.privacy_picks = this
                            .projects
                            .iter()
                            .map(|project| project.id.clone())
                            .collect();
                        cx.notify();
                    })),
            )
            .action(
                Button::new("privacy-restore")
                    .label(tr("Restore all"))
                    .small()
                    .disabled(!self.privacy.active)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.set_privacy(false, window, cx)),
                    ),
            )
            .action(
                Button::new("privacy-hide")
                    .label(tr("Hide selected"))
                    .small()
                    .primary()
                    .disabled(self.privacy_picks.is_empty())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.set_privacy(true, window, cx)),
                    ),
            )
    }
}
