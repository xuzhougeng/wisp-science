//! The queue of messages written while a turn runs: adding to it, sending
//! from it when the turn ends, and the strip above the composer that shows
//! what is waiting.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::glyph, surfaces};
use crate::{assets::icon, backend::receive, compose::Pending, controls, queue};
use wisp_dto::native_conversations::SendRequest;
use wisp_server::desktop::QueuedMessage;

impl Workspace {
    /// Hands what is typed, and what is attached to it, to the host, which
    /// sends it when the running turn ends.
    pub(super) fn enqueue(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only().is_some() {
            return;
        }
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending || self.loading_history {
            return;
        }
        let carried = self.chips.get(&session).cloned().unwrap_or_default();
        if text.trim().is_empty() && carried.attachments().is_empty() {
            return;
        }
        self.pending = true;
        let referenced = carried.reference_chips();
        let reply = self.backend.queue_message(
            project,
            SendRequest {
                session_id: session.clone(),
                request_id: String::new(),
                message: text.trim().to_owned(),
                resume: false,
                resume_after: None,
                attachments: carried.attachments(),
                references: carried.references(),
                acp_agent_id: None,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(_) => {
                        this.error = None;
                        // What it carries, shown on it once it is sent.
                        if !referenced.is_empty() {
                            let sent = this.sent_references.entry(session.clone()).or_default();
                            sent.push((text.trim().to_owned(), referenced));
                            let over = sent.len().saturating_sub(crate::compose::SENT_KEPT);
                            sent.drain(..over);
                        }
                        this.composer
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.state.save_draft(String::new());
                        this.chips.remove(&session);
                    }
                    Err(problem) => this.error = Some(problem),
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Whether a question or a plan in the open conversation waits for the
    /// user's own answer.
    fn asking(&self) -> bool {
        let planning = self.options().plan_first;
        self.rows.iter().enumerate().any(|(index, item)| {
            (controls::question(item).is_some()
                && controls::awaiting_answer(&self.rows, index, false))
                || (controls::plan_steps(item).is_some()
                    && controls::awaiting_approval(&self.rows, index, false, planning))
        })
    }

    /// Why the open conversation's queue is not moving though no turn runs.
    fn queue_hold(&self) -> Option<&'static str> {
        let failed = self
            .state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.error.is_some());
        queue::hold(failed, self.asking())
    }

    /// What waits in the open conversation, first to go first.
    fn waiting(&self) -> &[QueuedMessage] {
        self.status
            .as_ref()
            .map_or(&[], |status| status.queued.as_slice())
    }

    /// Runs a queue call on the open conversation, then reads the queue
    /// again; a refusal is said.
    fn change_queue<T: 'static>(
        &mut self,
        call: impl FnOnce(&crate::backend::Backend, String, String) -> crate::backend::Reply<T>,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = call(&self.backend, project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(value) => then(this, value, window, cx),
                    Err(problem) => this.error = Some(problem),
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Sends the next waiting message now, past a failed turn or an
    /// unanswered question: the user said so.
    fn send_next_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.change_queue(
            |backend, project, session| backend.send_queued_now(project, session),
            |this, sent, window, cx| {
                if sent {
                    this.history.clear();
                    if let Some(snapshot) = &mut this.state.snapshot {
                        snapshot.running = true;
                    }
                    this.watch_activity(window, cx);
                    this.start_poll(window, cx);
                    this.load_sessions(window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// Gives a waiting message to the running turn now, which reads it at
    /// its next step. If the turn has just stopped reading, the message
    /// stays in line and goes as the next turn, as it would have anyway.
    fn guide_with_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.change_queue(
            move |backend, project, session| backend.guide_with_queued(project, session, id),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    fn remove_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.change_queue(
            move |backend, project, session| backend.take_queued(project, session, id),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    fn move_queued_up(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.change_queue(
            move |backend, project, session| backend.move_queued_up(project, session, id),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    /// Puts a waiting message back into the composer to be changed. What is
    /// being written is not thrown away for it.
    fn take_back_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let writing = !self.composer.read(cx).value().trim().is_empty()
            || self
                .chips
                .get(&session)
                .is_some_and(|chips| !chips.chips().is_empty());
        if writing {
            self.error =
                Some(tr("Send or clear what you are writing before changing another").into());
            return cx.notify();
        }
        self.change_queue(
            move |backend, project, session| backend.take_queued(project, session, id),
            move |this, taken, window, cx| {
                let Some(queued) = taken else {
                    return;
                };
                // What it referred to is named as the sidebar, the
                // Artifacts tab and the workflow library name it.
                let carried =
                    Pending::from_sent(&queued.attachments, &queued.references, |reference| {
                        match reference {
                            wisp_dto::ComposerReferenceArg::Session { id } => this
                                .sessions
                                .iter()
                                .find(|row| &row.id == id)
                                .map(|row| row.title.clone()),
                            wisp_dto::ComposerReferenceArg::Artifact { id } => this
                                .artifacts
                                .iter()
                                .find(|row| &row.id == id)
                                .map(|row| row.name.clone()),
                            wisp_dto::ComposerReferenceArg::Workflow { id } => this
                                .workflows
                                .iter()
                                .find(|workflow| &workflow.id == id)
                                .map(|workflow| workflow.name.clone()),
                            _ => None,
                        }
                    });
                this.state.save_draft(queued.message.clone());
                this.composer
                    .update(cx, |input, cx| input.set_value(queued.message, window, cx));
                if !carried.chips().is_empty() {
                    this.chips.insert(session, carried);
                }
                this.focus_composer(window, cx);
            },
            window,
            cx,
        );
    }

    /// What waits in the open conversation, above the composer; nothing
    /// when nothing waits.
    pub(super) fn queue_strip(
        &self,
        running: bool,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        self.state.session.as_ref()?;
        let waiting = self.waiting();
        if waiting.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let hold = (!running).then(|| self.queue_hold()).flatten();
        let mut heading =
            div()
                .h_flex()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .child(match (waiting.len(), hold) {
                    (_, Some(why)) => why.to_owned(),
                    (1, None) => tr("1 message waits for this turn to end").to_owned(),
                    (n, None) => crate::i18n::fill(
                        tr("{} messages wait for this turn to end, in this order"),
                        &[&n],
                    ),
                });
        if hold.is_some() {
            heading = heading.child(
                Button::new("queue-send-next")
                    .label(tr("Send the next one"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.send_next_queued(window, cx))),
            );
        }
        let mut strip = surfaces::bar("queue", cx).v_flex().gap_1().child(heading);
        for queued in waiting {
            let id = queued.id;
            // Only words can be read mid-turn; attachments wait for a turn.
            let guidable = queue::guidable(queued, running);
            let guide = guidable.then(|| {
                Button::new(SharedString::from(format!("queue-guide-{id}")))
                    .icon(glyph("bolt"))
                    .xsmall()
                    .ghost()
                    .tooltip(tr("Give it to the running turn now"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.guide_with_queued(id, window, cx)
                    }))
            });
            strip = strip.child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .h(px(26.))
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(icon("clock").text_color(muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(queue::label(queued)),
                    )
                    .children(guide)
                    .child(
                        Button::new(SharedString::from(format!("queue-up-{id}")))
                            .icon(glyph("arrow-up"))
                            .xsmall()
                            .ghost()
                            .tooltip(tr("Send it earlier"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.move_queued_up(id, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("queue-edit-{id}")))
                            .icon(glyph("edit"))
                            .xsmall()
                            .ghost()
                            .tooltip(tr("Take it back to change it"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.take_back_queued(id, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("queue-remove-{id}")))
                            .icon(glyph("close"))
                            .xsmall()
                            .ghost()
                            .tooltip(tr("Do not send it"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove_queued(id, window, cx)
                            })),
                    ),
            );
        }
        Some(strip)
    }
}
