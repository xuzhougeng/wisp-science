//! Handing the open conversation off to a new one, from What the model is
//! sent or the command palette: the model writes a note from what it is
//! sent now, the user reads and changes it here, and a new conversation in
//! the project starts with the note as its first message and is opened.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{
    backend::receive,
    handoff::{self, Dialog},
    layers::Layer,
};

impl Workspace {
    /// Why the open conversation cannot be handed off now, when it cannot.
    pub(super) fn handoff_refusal(&self) -> Option<&'static str> {
        let snapshot = self.state.snapshot.as_ref();
        handoff::refusal(
            snapshot.is_some_and(|s| s.running),
            self.read_only().is_some(),
            snapshot.is_some_and(|s| s.acp_agent_id.is_some())
                || !self.options().acp_agent.is_empty(),
        )
    }

    /// Opens the dialog on the open conversation and has the note written
    /// at once, as the WebView does. A conversation that cannot be handed
    /// off now says why instead.
    pub(super) fn open_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        let Some(session) = self.state.session.clone() else {
            return;
        };
        if let Some(reason) = self.handoff_refusal() {
            self.notice = Some(reason.to_owned());
            cx.notify();
            return;
        }
        let (dialog, ticket) = Dialog::open(session);
        self.handoff = Some(dialog);
        self.handoff_error = None;
        for field in &self.handoff_fields {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.layers.open(Layer::Handoff);
        self.draft_handoff_note(ticket, window, cx);
        cx.notify();
    }

    /// Has the note written again, with the instruction as it is now.
    fn redraft_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.handoff.as_mut().and_then(Dialog::redraft) else {
            return;
        };
        self.handoff_error = None;
        self.draft_handoff_note(ticket, window, cx);
        cx.notify();
    }

    fn draft_handoff_note(&mut self, ticket: u64, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) = (
            self.state.project.clone(),
            self.handoff.as_ref().map(|dialog| dialog.session.clone()),
        ) else {
            return;
        };
        let instruction = self.handoff_fields[0].read(cx).value().trim().to_owned();
        let reply = self.backend.draft_handoff(
            project,
            session.clone(),
            (!instruction.is_empty()).then_some(instruction),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Closed, on another conversation, or drafted again since.
                if !this.layers.contains(Layer::Handoff) {
                    return;
                }
                let Some(dialog) = this
                    .handoff
                    .as_mut()
                    .filter(|dialog| dialog.session == session)
                else {
                    return;
                };
                if !dialog.drafted(ticket) {
                    return;
                }
                match result {
                    Ok(drafted) => this.handoff_fields[1]
                        .update(cx, |input, cx| input.set_value(drafted.note, window, cx)),
                    // The field keeps what it had, to be written by hand.
                    Err(error) => this.handoff_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts the new conversation from the note as it reads now, and
    /// opens it there.
    fn start_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let note = self.handoff_fields[1].read(cx).value().to_string();
        let Some(dialog) = self.handoff.as_mut() else {
            return;
        };
        if !dialog.start(&note) {
            return;
        }
        let source = dialog.session.clone();
        self.handoff_error = None;
        let reply = self
            .backend
            .start_from_handoff(project.clone(), source.clone(), note);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let here = this.layers.contains(Layer::Handoff)
                    && this
                        .handoff
                        .as_ref()
                        .is_some_and(|dialog| dialog.session == source);
                match result {
                    Ok(id) => {
                        // Its turn runs whether or not it is opened.
                        this.watch_activity(window, cx);
                        if here {
                            this.close_handoff(window, cx);
                            if this.layers.close(Layer::ModelView) {
                                this.clear_model_view();
                            }
                            this.open_session(project.clone(), id, window, cx);
                        } else {
                            this.load_sessions(window, cx);
                        }
                    }
                    Err(error) if here => {
                        if let Some(dialog) = this.handoff.as_mut() {
                            dialog.start_failed();
                        }
                        this.handoff_error = Some(error);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Handoff) {
            self.handoff = None;
            self.handoff_error = None;
            // What the model is sent stays open under it when it was.
            if !self.layers.contains(Layer::ModelView) {
                self.focus_composer(window, cx);
            }
            cx.notify();
        }
    }

    /// The instruction, the note and its progress, then Regenerate, Cancel
    /// and Start. A click beside it does not throw a note away: only
    /// Cancel and Escape leave it.
    pub(super) fn render_handoff(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let dialog = self.handoff.as_ref();
        let editable = dialog.is_some_and(Dialog::editable);
        let note = self.handoff_fields[1].read(cx).value().to_string();
        let starting = dialog.is_some_and(|dialog| dialog.step == handoff::Step::Starting);
        let mut card = surfaces::dialog(
            "handoff",
            DialogWidth::Regular,
            tr("Hand off to a new conversation"),
        )
        .gap_3()
        .child(div().text_sm().text_color(muted).child(tr(
            "The current context is summarized into a note you can edit. Confirm it to open a new conversation with the note as its first message.",
        )))
        .child(label(tr("Optional instruction for the note")))
        .child(
            Input::new(&self.handoff_fields[0])
                .small()
                .disabled(!editable),
        );
        if let Some(progress) = dialog.and_then(Dialog::progress) {
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(glyph("loader"))
                    .child(progress),
            );
        }
        if let Some(error) = &self.handoff_error {
            card = card.child(error_banner(error, cx));
        }
        card.child(label(tr("Handoff note")))
            .child(
                div().h(px(300.)).child(
                    Input::new(&self.handoff_fields[1])
                        .h_full()
                        .disabled(!editable),
                ),
            )
            .action(
                Button::new("handoff-redraft")
                    .label(tr("Regenerate"))
                    .small()
                    .ghost()
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, window, cx| this.redraft_handoff(window, cx))),
            )
            .action(
                Button::new("handoff-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.close_handoff(window, cx))),
            )
            .action(
                Button::new("handoff-start")
                    .icon(glyph("conversation-move"))
                    .label(if starting {
                        tr("Starting the new conversation…")
                    } else {
                        tr("Start new conversation")
                    })
                    .small()
                    .primary()
                    .disabled(!dialog.is_some_and(|dialog| dialog.can_start(&note)))
                    .on_click(cx.listener(|this, _, window, cx| this.start_handoff(window, cx))),
            )
    }
}
