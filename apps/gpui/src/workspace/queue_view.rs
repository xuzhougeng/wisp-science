//! The queue of messages written while a turn runs: adding to it, sending
//! from it when the turn ends, and the strip above the composer that shows
//! what is waiting.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::glyph};
use crate::{assets::icon, controls, queue};

impl Workspace {
    /// Keeps what is typed, and what is attached to it, until the running
    /// turn ends.
    pub(super) fn enqueue(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        if self.pending || self.loading_history {
            return;
        }
        let carried = self.chips.get(&session).cloned().unwrap_or_default();
        match self.queue.push(&session, &text, carried) {
            Ok(_) => {
                self.error = None;
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.state.save_draft(String::new());
                self.chips.remove(&session);
            }
            // An empty message is simply not queued; a full queue says so.
            Err(problem) if !text.trim().is_empty() => self.error = Some(problem),
            Err(_) => {}
        }
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

    /// Sends the next waiting message if nothing stands in its way.
    pub(super) fn drain_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_hold().is_none() {
            self.send_next_queued(window, cx);
        }
    }

    /// Sends the next waiting message now. The host still refuses it while
    /// a turn runs, in which case it stays first in line.
    fn send_next_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let idle = self
            .state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.running && !snapshot.read_only);
        if !idle || self.pending || self.loading_history {
            return;
        }
        if let Some(next) = self.queue.next(&session) {
            self.deliver(
                next.text.clone(),
                next.carried.clone(),
                false,
                Some(next),
                window,
                cx,
            );
        }
    }

    fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(session) = self.state.session.clone() {
            self.queue.remove(&session, id);
            cx.notify();
        }
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
        if let Some(queued) = self.queue.remove(&session, id) {
            self.state.save_draft(queued.text.clone());
            self.composer
                .update(cx, |input, cx| input.set_value(queued.text, window, cx));
            if !queued.carried.chips().is_empty() {
                self.chips.insert(session, queued.carried);
            }
            self.focus_composer(window, cx);
        }
        cx.notify();
    }

    /// What waits in the open conversation, above the composer; nothing
    /// when nothing waits.
    pub(super) fn queue_strip(&self, running: bool, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.state.session.as_ref()?;
        let waiting = self.queue.list(session);
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
                    (1, None) => "1 message waits for this turn to end".to_owned(),
                    (n, None) => format!("{n} messages wait for this turn to end, in this order"),
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
        let mut strip = div().v_flex().gap_1().px_1().child(heading);
        for queued in waiting {
            let id = queued.id;
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
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_queued(id, cx)),
                            ),
                    ),
            );
        }
        Some(strip)
    }
}
