//! What the model is sent for the open conversation: a dialog that lists
//! the saved working set row by row, with how much of it each row is, under
//! the WebView's context usage: the latest request kind by kind, tools
//! included, while the host has it.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Sizable, StyledExt, button::Button};

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{
    backend::receive,
    controls::compact,
    layers::Layer,
    model_view::{self, Holds},
};
use wisp_server::desktop::SentContext;

/// A kind's share as a thin bar, filled in the accent colour.
fn share_bar(percent: usize, cx: &App) -> Div {
    div()
        .flex_1()
        .h(px(4.))
        .rounded(px(2.))
        .bg(cx.theme().muted)
        .child(
            div()
                .h_full()
                .w(relative(percent.min(100) as f32 / 100.))
                .rounded(px(2.))
                .bg(cx.theme().primary),
        )
}

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
        self.model_view_kind = None;
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

    /// The latest request kind by kind, as the WebView's context usage
    /// lists it: each kind's tokens and share of the context window, and,
    /// opened, the tools it holds with what the model reads about each, or
    /// its text.
    fn render_sent(&self, sent: &SentContext, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut split = div().v_flex().gap_1().pb_3().child(
            div()
                .pb_1()
                .text_xs()
                .text_color(muted)
                .child(model_view::sent_line(sent)),
        );
        for (at, (kind, tokens, percent)) in model_view::sent_kinds(sent).into_iter().enumerate() {
            let opens = model_view::sent_opens(sent, kind);
            let open = opens && self.model_view_kind == Some(kind);
            let mut line = div()
                .id(("model-view-kind", at))
                .h_flex()
                .gap_2()
                .px_1()
                .rounded(cx.theme().radius)
                .text_xs()
                .child(
                    // As on the rows below, the arrow keeps its place.
                    div().w(px(14.)).flex_shrink_0().children(opens.then(|| {
                        glyph(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .xsmall()
                        .text_color(muted)
                    })),
                )
                .child(div().w(px(150.)).flex_shrink_0().child(kind.label()))
                .child(share_bar(percent, cx))
                .child(
                    div()
                        .w(px(96.))
                        .flex_shrink_0()
                        .text_color(muted)
                        .child(format!("≈{} · {percent}%", compact(tokens as u64))),
                );
            if opens {
                line = line
                    .cursor_pointer()
                    .hover(|line| line.bg(cx.theme().accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model_view_kind = (this.model_view_kind != Some(kind)).then_some(kind);
                        cx.notify();
                    }));
            }
            split = split.child(line);
            if !open {
                continue;
            }
            let holds = div()
                .ml(px(24.))
                .mb_1()
                .p_2()
                .rounded(cx.theme().radius)
                .bg(cx.theme().muted)
                .text_xs();
            split = split.child(match model_view::holds(sent, kind) {
                Holds::Tools(tools) => {
                    let mut list = holds.v_flex().gap_0p5();
                    for tool in tools {
                        list = list.child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(150.))
                                        .flex_shrink_0()
                                        .truncate()
                                        .child(tool.name.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(muted)
                                        .child(model_view::tool_line(tool).to_owned()),
                                ),
                        );
                    }
                    list
                }
                Holds::Text(text) => holds
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(model_view::opened_text(text)),
                Holds::Nothing => holds,
            });
        }
        split
    }

    pub(super) fn render_model_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut card = surfaces::dialog(
            "model-view",
            DialogWidth::Wide,
            tr("What the model is sent"),
        )
        .gap_1()
        .close_on_click_outside(
            cx.listener(|this, _, window, cx| this.close_model_view(window, cx)),
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
                // What it is made of, as the WebView's context usage shows:
                // the latest request kind by kind, tools included, while
                // the host has it; the saved working set otherwise.
                if let Some(sent) = &view.sent {
                    card = card.child(self.render_sent(sent, cx));
                } else {
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
                                    .child(share_bar(percent, cx))
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
        // So is handing the conversation off, as from the WebView's usage
        // panel; the dialog stays open under it.
        let handoff = Button::new("model-view-handoff")
            .icon(glyph("conversation-move"))
            .label(tr("Hand off…"))
            .small()
            .disabled(self.handoff_refusal().is_some() || self.compacting)
            .tooltip(tr(
                "Summarize the current context into a note you can edit, then start a new conversation from it",
            ))
            .on_click(cx.listener(|this, _, window, cx| this.open_handoff(window, cx)));
        card.actions(undo).action(compact).action(handoff).action(
            Button::new("model-view-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.close_model_view(window, cx))),
        )
    }
}
