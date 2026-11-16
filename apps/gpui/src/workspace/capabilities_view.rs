//! The capabilities overview: a dialog that counts what Wisp can draw on
//! in the open project and which command-line tools this computer has.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::error_banner};
use crate::{backend::receive, layers::Layer, settings};

impl Workspace {
    pub(super) fn open_capabilities(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Capabilities);
        self.capabilities = None;
        self.capabilities_error = None;
        let reply = self.backend.capabilities(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Closed, or another project, while this was read.
                if !this.layers.contains(Layer::Capabilities)
                    || this.state.project.as_deref() != Some(project.as_str())
                {
                    return;
                }
                match result {
                    Ok(found) => this.capabilities = Some(found),
                    Err(error) => this.capabilities_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn close_capabilities(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Capabilities) {
            self.capabilities = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_capabilities(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut card = div()
            .id("capabilities")
            .v_flex()
            .gap_2()
            .w(px(560.))
            .max_h(px(620.))
            .overflow_y_scroll()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            // A click inside is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Capabilities")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "What Wisp can draw on in this project, and the tools it found on this \
                 computer.",
            )));
        if let Some(error) = &self.capabilities_error {
            card = card.child(error_banner(error, cx));
        }
        match &self.capabilities {
            None if self.capabilities_error.is_none() => {
                card = card.child(div().text_sm().text_color(muted).child(tr("Looking…")));
            }
            None => {}
            Some(found) => {
                for (label, value, section) in settings::capability_lines(found) {
                    let mut row = div()
                        .h_flex()
                        .gap_3()
                        .py_1()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            div()
                                .w(px(130.))
                                .flex_shrink_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(div().flex_1().min_w_0().text_sm().child(value));
                    if let Some(section) = section {
                        row = row.child(
                            Button::new(SharedString::from(format!("capability-{label}")))
                                .label(tr("Settings"))
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.layers.close(Layer::Capabilities);
                                    this.capabilities = None;
                                    this.open_settings(window, cx);
                                    this.show_settings_section(section, window, cx);
                                })),
                        );
                    }
                    card = card.child(row);
                }
                card = card.child(
                    div()
                        .pt_3()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(tr("Tools on this computer")),
                );
                for tool in &found.tools {
                    card = card.child(
                        div()
                            .h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(130.))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(tool.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(match &tool.path {
                                        Some(path) => format!("{} · {path}", tool.what),
                                        None => {
                                            crate::i18n::fill(tr("{} · not found"), &[&tool.what])
                                        }
                                    }),
                            ),
                    );
                }
            }
        }
        card = card.child(
            div().h_flex().justify_end().pt_2().child(
                Button::new("capabilities-close")
                    .label(tr("Close"))
                    .small()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.close_capabilities(window, cx)),
                    ),
            ),
        );
        div()
            .id("capabilities-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Nothing underneath is clickable while the dialog is open.
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_capabilities(window, cx)),
            )
            .child(card)
    }
}
