//! Compaction in the open conversation: the dialog that asks which kind of
//! compaction to run before it runs (from `/compact` or What the model is
//! sent), the quiet line the transcript shows where the conversation was
//! compacted, and undoing the latest compaction from that line or from
//! the dialog of what the model is sent; both go through one host call.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};
use wisp_dto::ContextCompactionDto;

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{
    assets::icon,
    backend::receive,
    compaction::{self, Kind},
    i18n::tr,
    layers::Layer,
    slash,
};

impl Workspace {
    /// Asks which kind of compaction to run in the open conversation.
    pub(super) fn open_compact(&mut self, cx: &mut Context<Self>) {
        if self.state.session.is_none() {
            return;
        }
        self.layers.close(Layer::Palette);
        self.compact_error = None;
        self.layers.open(Layer::CompactContext);
        cx.notify();
    }

    fn close_compact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::CompactContext) {
            self.compact_error = None;
            // What the model is sent stays open under it when it was.
            if !self.layers.contains(Layer::ModelView) {
                self.focus_composer(window, cx);
            }
            cx.notify();
        }
    }

    /// Runs the chosen compaction. The dialog shuts when it is done and a
    /// note says what it did; the transcript gets its line from the host.
    fn start_compaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.compacting {
            return;
        }
        self.compacting = true;
        self.compact_error = None;
        self.notice = Some(tr("Compacting…").to_owned());
        let reply = self
            .backend
            .compact_conversation(project, session, self.compact_kind.host());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.compacting = false;
                // Said only where it was asked.
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(done) => {
                        this.notice = Some(slash::compacted_note(
                            done.before,
                            done.after,
                            done.summarised,
                        ));
                        this.close_compact(window, cx);
                        this.after_context_change(window, cx);
                    }
                    Err(error) => {
                        this.notice = None;
                        if this.layers.contains(Layer::CompactContext) {
                            this.compact_error = Some(error);
                        } else {
                            this.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Puts back what the compaction `epoch` replaced: the one host call
    /// both the transcript's line and What the model is sent use. The host
    /// refuses a compaction that is no longer the latest or that the
    /// conversation went on after.
    pub(super) fn undo_compaction_at(
        &mut self,
        epoch: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.undo_compaction(project, session, epoch);
        self.model_view_error = None;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.notice = Some(tr("Compaction undone").to_owned());
                        this.after_context_change(window, cx);
                    }
                    Err(error) if this.layers.contains(Layer::ModelView) => {
                        this.model_view_error = Some(error)
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What the model is sent changed: the transcript's newest page is read
    /// again (its compaction line comes or goes), the context meter too,
    /// and What the model is sent when it is open.
    fn after_context_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_poll(window, cx);
        self.load_status(window, cx);
        if self.layers.contains(Layer::ModelView) {
            self.open_model_view(window, cx);
        }
    }

    /// The line where the conversation was compacted: a hairline rule with
    /// what happened between, Undo while the host allows it, and the
    /// summary a click away when there is one.
    pub(super) fn compaction_row(
        &self,
        index: usize,
        card: ContextCompactionDto,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let rule = || {
            div()
                .flex_1()
                .min_w(px(16.))
                .h(px(1.))
                .bg(cx.theme().border)
        };
        let snapshot = self.state.snapshot.as_ref();
        let undo = compaction::undo_offered(
            &card,
            snapshot.is_some_and(|s| s.running),
            snapshot.is_some_and(|s| s.read_only),
        )
        .then(|| {
            let epoch = card.epoch as i64;
            Button::new(("compaction-undo", index))
                .icon(glyph("undo-compact"))
                .label(tr("Undo the compaction"))
                .xsmall()
                .ghost()
                .disabled(self.pending)
                .tooltip(tr(
                    "Send the model everything again, as before the compaction; only while nothing has been said since",
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.undo_compaction_at(epoch, window, cx)
                }))
        });
        let key = format!("compaction-{}", card.epoch);
        let open = card.checkpoint.is_some() && self.open_tools.contains(&key);
        let mut said = div()
            .id(("compaction-caption", index))
            .h_flex()
            .flex_shrink()
            .min_w_0()
            .gap_1()
            .text_xs()
            .text_color(muted)
            .child(icon("context-compact").flex_shrink_0())
            .child(div().min_w_0().child(compaction::caption(&card)));
        if card.checkpoint.is_some() {
            let toggle = key.clone();
            said = said
                .cursor_pointer()
                .hover(|said| said.text_color(cx.theme().foreground))
                .child(
                    icon(if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    })
                    .flex_shrink_0(),
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_tool(toggle.clone(), index, cx)),
                );
        }
        let mut row = div().w_full().v_flex().gap_2().px_6().py_2().child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(rule())
                .child(said)
                .children(undo)
                .child(rule()),
        );
        if let (true, Some(summary)) = (open, card.checkpoint) {
            let id = SharedString::from(format!("compaction-summary-{key}"));
            row = row.child(
                surfaces::card(SharedString::from(format!("compaction-card-{key}")), cx)
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child(TextView::markdown(id, summary, window, cx).selectable(true)),
            );
        }
        row.into_any_element()
    }

    /// The choice of compaction: the kinds the host does, each with what
    /// it does, then Cancel and the main action.
    pub(super) fn render_compact_dialog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let snapshot = self.state.snapshot.as_ref();
        let running = snapshot.is_some_and(|s| s.running);
        let read_only = snapshot.is_some_and(|s| s.read_only);
        let mut card = surfaces::dialog(
            "compact-context",
            DialogWidth::Narrow,
            tr("Compact context"),
        )
        .gap_3()
        .close_on_click_outside(cx.listener(|this, _, window, cx| this.close_compact(window, cx)))
        .child(div().text_sm().text_color(muted).child(tr(
            "Choose regular pruning or a semantic checkpoint before rewriting the model context.",
        )));
        for (at, kind) in Kind::ALL.into_iter().enumerate() {
            let chosen = self.compact_kind == kind;
            card = card.child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        Button::new(("compact-kind", at))
                            .icon(glyph(if chosen { "circle-check" } else { "circle" }))
                            .label(kind.title())
                            .small()
                            .ghost()
                            .selected(chosen)
                            .disabled(self.compacting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.compact_kind = kind;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .pl(px(30.))
                            .text_xs()
                            .text_color(muted)
                            .child(kind.hint()),
                    ),
            );
        }
        card = card.child(div().text_xs().text_color(muted).child(tr(
            "The full transcript is archived first, and the latest compaction can be undone until the conversation goes on.",
        )));
        if running {
            card = card.child(div().text_xs().text_color(muted).child(tr(
                "A turn is running in this conversation; compact it when the turn has ended",
            )));
        }
        if let Some(error) = &self.compact_error {
            card = card.child(error_banner(error, cx));
        }
        card.action(
            Button::new("compact-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.close_compact(window, cx))),
        )
        .action(
            Button::new("compact-start")
                .icon(glyph("context-compact"))
                .label(if self.compacting {
                    tr("Compacting…")
                } else {
                    tr("Start compaction")
                })
                .small()
                .primary()
                .disabled(self.compacting || running || read_only)
                .on_click(cx.listener(|this, _, window, cx| this.start_compaction(window, cx))),
        )
    }
}
