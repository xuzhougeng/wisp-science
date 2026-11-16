//! What the model is sent for the open conversation: a dialog that lists
//! the saved working set row by row, with how much of it each row is.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Sizable, StyledExt, button::Button};

use super::{
    Workspace,
    shell::{error_banner, glyph},
};
use crate::{backend::receive, layers::Layer, model_view};

impl Workspace {
    pub(super) fn open_model_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::ModelView);
        self.clear_model_view();
        let reply = self.backend.model_view(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Closed, or another conversation, while this was read.
                if !this.layers.contains(Layer::ModelView)
                    || this.state.session.as_deref() != Some(session.as_str())
                {
                    return;
                }
                match result {
                    Ok(found) => this.model_view = Some(found),
                    Err(error) => this.model_view_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Forgets what the dialog was showing, once its layer is gone.
    pub(super) fn clear_model_view(&mut self) {
        self.model_view = None;
        self.model_view_error = None;
        self.model_view_open.clear();
    }

    /// Undoes the compaction the dialog shows; the dialog then shows the
    /// working set as it is again.
    fn undo_compaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(epoch) = self.model_view.as_ref().map(|view| view.epoch) {
            self.undo_compaction_at(epoch, window, cx);
        }
    }

    fn close_model_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ModelView) {
            self.clear_model_view();
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_model_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut card = div()
            .id("model-view")
            .v_flex()
            .gap_1()
            .w(px(760.))
            .max_w(relative(0.94))
            .max_h(relative(0.9))
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
                    .child(tr("What the model is sent")),
            );
        if let Some(error) = &self.model_view_error {
            card = card.child(error_banner(error, cx));
        }
        match &self.model_view {
            None if self.model_view_error.is_none() => {
                card = card.child(div().text_sm().text_color(muted).child(tr("Reading…")));
            }
            None => {}
            Some(view) => {
                card = card.child(
                    div()
                        .pb_2()
                        .text_xs()
                        .text_color(muted)
                        .child(model_view::summary(view)),
                );
                // What it is made of, as the WebView's context usage shows.
                let kinds = model_view::by_kind(view);
                if !kinds.is_empty() {
                    let mut split = div().v_flex().gap_1().pb_3();
                    for (name, tokens, percent) in kinds {
                        split = split.child(
                            div()
                                .h_flex()
                                .gap_2()
                                .text_xs()
                                .child(div().w(px(150.)).flex_shrink_0().child(name))
                                .child(
                                    div()
                                        .flex_1()
                                        .h(px(4.))
                                        .rounded(px(2.))
                                        .bg(cx.theme().muted)
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative(percent as f32 / 100.))
                                                .rounded(px(2.))
                                                .bg(cx.theme().primary),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(96.))
                                        .flex_shrink_0()
                                        .text_color(muted)
                                        .child(format!("≈{tokens} · {percent}%")),
                                ),
                        );
                    }
                    card = card.child(split.child(div().text_xs().text_color(muted).child(tr(
                        "Tool definitions are made when a turn starts and are not counted here.",
                    ))));
                }
                for (index, row) in view.rows.iter().enumerate() {
                    let opens = model_view::opens(row);
                    let open = opens && self.model_view_open.contains(&index);
                    let mut line = div()
                        .id(("model-view-row", index))
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .py_1()
                        .px_1()
                        .rounded(cx.theme().radius)
                        .child(
                            // The arrow keeps its place on rows that do not
                            // open, so the labels line up.
                            div()
                                .w(px(14.))
                                .pt_0p5()
                                .flex_shrink_0()
                                .children(opens.then(|| {
                                    glyph(if open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    })
                                    .xsmall()
                                    .text_color(muted)
                                })),
                        )
                        .child(
                            div()
                                .w(px(96.))
                                .flex_shrink_0()
                                .truncate()
                                .text_xs()
                                .pt_0p5()
                                .font_weight(FontWeight::MEDIUM)
                                .child(model_view::label(row)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(model_view::line(row)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .pt_0p5()
                                .text_color(muted)
                                .child(model_view::share(row, view)),
                        );
                    if opens {
                        line = line
                            .cursor_pointer()
                            .hover(|line| line.bg(cx.theme().accent))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.model_view_open.remove(&index) {
                                    this.model_view_open.insert(index);
                                }
                                cx.notify();
                            }));
                    }
                    card = card.child(line);
                    if open {
                        card = card.child(
                            div()
                                .ml(px(24.))
                                .mb_1()
                                .p_2()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().muted)
                                .text_xs()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(row.text.clone()),
                        );
                    }
                }
            }
        }
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let undo = self
            .model_view
            .as_ref()
            .filter(|view| model_view::can_undo(view))
            .map(|_| {
                Button::new("model-view-undo")
                    .icon(glyph("undo-compact"))
                    .label(tr("Undo the compaction"))
                    .small()
                    .disabled(running)
                    .tooltip(tr(
                        "Send the model everything again, as before the compaction; only while nothing has been said since",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.undo_compaction(window, cx)))
            });
        // Compacting is chosen in a dialog over this one.
        let compact = Button::new("model-view-compact")
            .icon(glyph("context-compact"))
            .label(tr("Compact…"))
            .small()
            .disabled(running || self.compacting)
            .on_click(cx.listener(|this, _, _, cx| this.open_compact(cx)));
        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .justify_end()
                .pt_2()
                .children(undo)
                .child(compact)
                .child(
                    Button::new("model-view-close")
                        .label(tr("Close"))
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_model_view(window, cx)),
                        ),
                ),
        );
        div()
            .id("model-view-backdrop")
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
                cx.listener(|this, _, window, cx| this.close_model_view(window, cx)),
            )
            .child(card)
    }
}
