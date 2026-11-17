//! Compaction in the open conversation: the dialog that asks which kind of
//! compaction to run before it runs (from `/compact` or What the model is
//! sent), the quiet line the transcript shows where the conversation was
//! compacted, and undoing the latest compaction from that line or from
//! the dialog of what the model is sent; both go through one host call.
//! And the bar above the composer when the model refused a turn because
//! the conversation outgrew its context window, with the WebView's three
//! ways on: compact and continue the turn, continue in a new
//! conversation, or pause.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};
use wisp_dto::{ContextCompactionDto, native_conversations::SendRequest};

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{
    assets::icon,
    backend::receive,
    compaction::{self, Kind, Recovery},
    compose::Chip,
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
        // Asked for on its own, it only compacts.
        self.compact_resume = None;
        self.layers.open(Layer::CompactContext);
        cx.notify();
    }

    /// The WebView's Compact and continue: the compact dialog, set to
    /// semantic, and once that compaction is done the turn the model
    /// refused goes on. Cancelling the dialog cancels the going on.
    fn compact_and_continue(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        self.open_compact(cx);
        self.compact_kind = Kind::Semantic;
        self.compact_resume = Some(session);
    }

    fn close_compact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::CompactContext) {
            self.compact_error = None;
            self.compact_resume = None;
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
                        // Asked for to go on with a refused turn: it does,
                        // in the conversation it was asked in.
                        let resume = this
                            .compact_resume
                            .take()
                            .is_some_and(|resume| selection.session.as_ref() == Some(&resume));
                        this.notice = Some(slash::compacted_note(
                            done.before,
                            done.after,
                            done.summarised,
                        ));
                        this.close_compact(window, cx);
                        this.after_context_change(window, cx);
                        if resume {
                            this.resume_turn(window, cx);
                        }
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

    /// Goes on with the turn the model refused, from what was saved: no
    /// new message, and the tool calls that already ran are not run again.
    fn resume_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.read_only().is_some() {
            return;
        }
        self.pending = true;
        self.error = None;
        self.poll = None;
        let reply = self.backend.send_message(
            project,
            SendRequest {
                session_id: session,
                request_id: uuid::Uuid::new_v4().to_string(),
                message: String::new(),
                resume: true,
                resume_after: None,
                attachments: Vec::new(),
                references: Vec::new(),
                acp_agent_id: None,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if result.is_ok() {
                    this.watch_activity(window, cx);
                }
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(_) => {
                        if let Some(snapshot) = &mut this.state.snapshot {
                            snapshot.running = true;
                        }
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The WebView's Continue in a new conversation: a new one in this
    /// project is asked to go on with the unfinished request, with this
    /// one attached as context, and opened.
    fn continue_elsewhere(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(source)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let prompt = compaction::continue_prompt().to_owned();
        let title = self
            .known
            .get(&source)
            .map(|row| row.title.clone())
            .unwrap_or_default();
        let reply = self.backend.continue_in_new_conversation(
            project.clone(),
            source.clone(),
            prompt.clone(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(id) => {
                        this.watch_activity(window, cx);
                        // The conversation it read shows on the message.
                        this.sent_references.entry(id.clone()).or_default().push((
                            prompt.clone(),
                            vec![Chip::Conversation {
                                id: source.clone(),
                                title: title.clone(),
                            }],
                        ));
                        if this.state.current(&selection) {
                            this.open_session(project.clone(), id, window, cx);
                        } else {
                            this.load_sessions(window, cx);
                        }
                    }
                    Err(error) if this.state.current(&selection) => this.error = Some(error),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The WebView's Pause conversation: nothing more is sent, the bar goes
    /// and the error stays; a later turn that fails the same way shows it
    /// again.
    fn pause_after_too_long(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = &self.state.snapshot {
            self.paused_failures.insert(compaction::failure(snapshot));
        }
        cx.notify();
    }

    /// Whether the open conversation shows the bar for a turn the model
    /// refused as too long.
    pub(super) fn recovery_shown(&self) -> bool {
        self.state.snapshot.as_ref().is_some_and(|snapshot| {
            compaction::recovery_offered(
                snapshot,
                self.read_only().is_some(),
                self.paused_failures
                    .contains(&compaction::failure(snapshot)),
            )
        })
    }

    /// The bar above the composer after a turn the model refused because
    /// the conversation outgrew its context window: what happened, the
    /// provider's words under it, and the three ways on.
    pub(super) fn render_context_recovery(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.recovery_shown() {
            return None;
        }
        let snapshot = self.state.snapshot.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let attention = cx.theme().warning;
        let busy = self.pending || self.compacting;
        let mut choices = div().h_flex().flex_wrap().gap_2();
        for (at, recovery) in Recovery::ALL.into_iter().enumerate() {
            let button = Button::new(("context-recovery", at))
                .label(recovery.label())
                .small()
                .disabled(busy)
                .tooltip(recovery.hint());
            let button = match recovery {
                Recovery::CompactAndContinue => button
                    .icon(glyph("context-compact"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.compact_and_continue(cx))),
                Recovery::NewConversation => button.icon(glyph("chat")).on_click(
                    cx.listener(|this, _, window, cx| this.continue_elsewhere(window, cx)),
                ),
                Recovery::Pause => button
                    .icon(glyph("pause"))
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| this.pause_after_too_long(cx))),
            };
            choices = choices.child(button);
        }
        Some(
            surfaces::bar("context-recovery", cx)
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(icon("circle-alert").text_color(attention))
                        .child(
                            div()
                                .child(tr("This conversation is too long")),
                        ),
                )
                .child(div().text_xs().text_color(muted).child(tr(
                    "The model rejected the request because the conversation exceeded its context window. Choose how Wisp should preserve and continue the work.",
                )))
                .children(
                    snapshot
                        .error
                        .clone()
                        .map(|error| div().text_xs().text_color(muted).truncate().child(error)),
                )
                .child(choices.pt_1()),
        )
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
        if self.compact_resume.is_some() {
            card = card.child(div().text_xs().text_color(muted).child(tr(
                "Once it is compacted, the unfinished response goes on from what was saved, without running finished tools again.",
            )));
        }
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
                } else if self.compact_resume.is_some() {
                    tr("Compact and continue")
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
