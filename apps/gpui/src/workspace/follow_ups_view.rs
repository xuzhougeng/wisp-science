//! The questions the model suggests after an answer: a short list above
//! the composer. Picking one puts it in the composer to be sent or changed;
//! the list can be put away, and is gone when the next turn starts.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt};

use super::{
    Workspace,
    shell::{glyph, icon_button},
};
use crate::{backend::receive, compose};

impl Workspace {
    /// Once a turn has ended a model may still be asked about it: what
    /// could come next, and a review. When that is settled the conversation
    /// and its status are read once more, so both show without anything
    /// being polled for.
    pub(super) fn await_after_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.after_turn_settled(project, session);
        cx.spawn_in(window, async move |this, cx| {
            // A failure only means no questions are shown.
            let _ = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let idle = this.state.snapshot.as_ref().is_some_and(|s| !s.running);
                // Another conversation, or a turn that began meanwhile, is
                // being read already.
                if this.state.current(&selection) && idle && !this.pending {
                    this.start_poll(window, cx);
                    this.load_status(window, cx);
                }
            });
        })
        .detach();
    }

    fn use_follow_up(&mut self, question: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state.save_draft(question.clone());
        self.composer.update(cx, |input, cx| {
            input.set_value(question, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn dismiss_follow_ups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        // Gone at once here; the host forgets them for every window.
        if let Some(snapshot) = &mut self.state.snapshot {
            snapshot.follow_ups.clear();
        }
        let reply = self.backend.dismiss_follow_ups(project, session);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    if this.state.current(&selection) {
                        this.error = Some(error);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub(super) fn render_follow_ups(&self, cx: &mut Context<Self>) -> Option<Div> {
        let questions = compose::suggestions(self.state.snapshot.as_ref()?);
        if questions.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let mut block = div()
            .v_flex()
            .flex_shrink_0()
            .gap_0p5()
            .mx_4()
            .mb_2()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("You could ask next"),
                    )
                    .child(
                        icon_button("follow-ups-dismiss", "close", "Put these away")
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dismiss_follow_ups(window, cx)
                            })),
                    ),
            );
        for (index, question) in questions.iter().enumerate() {
            let picked = question.clone();
            block = block.child(
                div()
                    .id(("follow-up", index))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.use_follow_up(picked.clone(), window, cx)
                    }))
                    .child(glyph("arrow-right").xsmall().text_color(muted))
                    .child(div().flex_1().min_w_0().text_sm().child(question.clone())),
            );
        }
        Some(block)
    }
}
