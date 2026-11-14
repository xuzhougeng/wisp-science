//! The review of the open conversation: a dialog that has the model read
//! the conversation as a reviewer and lists what it found, with a way to
//! ask for the corrections.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::error_banner, tone_color};
use crate::{
    backend::receive,
    layers::Layer,
    review::{self, Asked},
};

impl Workspace {
    /// Opens the dialog and asks for a review of the open conversation.
    pub(super) fn open_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Review);
        self.review = Some(Asked::Reading);
        let reply = self.backend.review_conversation(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Closed, or another conversation, while it was read. A
                // report that nobody is waiting for is dropped.
                if !this.layers.contains(Layer::Review)
                    || this.state.session.as_deref() != Some(session.as_str())
                    || this.review != Some(Asked::Reading)
                {
                    return;
                }
                this.review = Some(match result {
                    Ok(found) => Asked::Done(found),
                    Err(error) => Asked::Failed(error),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the dialog on the review that followed the latest turn.
    fn see_review(&mut self, found: wisp_server::desktop::Review, cx: &mut Context<Self>) {
        self.review = Some(Asked::Done(found));
        self.layers.open(Layer::Review);
        cx.notify();
    }

    /// Puts away the line about the review that followed the latest turn.
    fn dismiss_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        // Gone at once here; the host forgets it for every window.
        if let Some(status) = &mut self.status {
            status.review = None;
        }
        let reply = self.backend.dismiss_review(project, session);
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
        cx.notify();
    }

    /// The line above the composer about the review of the latest turn.
    pub(super) fn render_review_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let outcome = self.status.as_ref()?.review.as_ref()?;
        let (said, tone, opens) = review::banner(outcome);
        let mut line = div()
            .h_flex()
            .flex_shrink_0()
            .gap_2()
            .mx_4()
            .mb_2()
            .px_3()
            .py_1p5()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone_color(tone, cx))
                    .child(said),
            );
        if let (true, wisp_server::desktop::ReviewOutcome::Done(found)) = (opens, outcome) {
            let found = found.clone();
            line = line.child(
                Button::new("review-banner-see")
                    .label("See")
                    .small()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.see_review(found.clone(), cx)),
                    ),
            );
        }
        if *outcome != wisp_server::desktop::ReviewOutcome::Reading {
            line = line.child(
                super::shell::icon_button("review-banner-dismiss", "close", "Put this away")
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| this.dismiss_review(window, cx))),
            );
        }
        Some(line)
    }

    fn close_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Review) {
            self.review = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Puts the request to correct what was found in the composer, to be
    /// sent or changed, and closes the dialog.
    fn ask_for_corrections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Asked::Done(found)) = &self.review else {
            return;
        };
        let text = review::with_correction(&self.composer.read(cx).value(), found);
        self.state.save_draft(text.clone());
        self.composer
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.close_review(window, cx);
    }

    pub(super) fn render_review(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut card = div()
            .id("review")
            .v_flex()
            .gap_2()
            .w(px(680.))
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
                    .child("Review"),
            )
            .child(div().text_xs().text_color(muted).child(
                "The conversation's model reads it as a reviewer and reports claims that \
                 its own tool output does not support. Nothing is added to the conversation.",
            ));
        let mut actions = div().h_flex().gap_2().justify_end().pt_2();
        match &self.review {
            None | Some(Asked::Reading) => {
                card = card.child(
                    div()
                        .py_4()
                        .text_sm()
                        .text_color(muted)
                        .child("Reading the conversation…"),
                );
            }
            Some(Asked::Failed(error)) => {
                card = card.child(error_banner(error, cx));
                actions = actions.child(self.review_again(cx));
            }
            Some(Asked::Done(found)) => {
                let (outcome, tone) = review::outcome(found);
                card = card
                    .child(
                        div()
                            .pt_2()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tone_color(tone, cx))
                            .child(outcome),
                    )
                    .child(div().text_sm().child(found.summary.clone()));
                for finding in &found.findings {
                    let (heading, tone) = review::finding_heading(finding);
                    let line = |label: &'static str, text: &str| {
                        div()
                            .h_flex()
                            .items_start()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(76.))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .pt_0p5()
                                    .text_color(muted)
                                    .child(label),
                            )
                            .child(div().flex_1().min_w_0().text_sm().child(text.to_owned()))
                    };
                    card = card.child(
                        div()
                            .v_flex()
                            .gap_1()
                            .p_3()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(border)
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(tone_color(tone, cx))
                                    .child(heading),
                            )
                            .child(line("Claim", &finding.claim))
                            .child(line("Evidence", &finding.evidence))
                            .child(line("Fix", &finding.fix)),
                    );
                }
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("Reviewed by {}.", found.model)),
                );
                actions = actions.child(self.review_again(cx));
                if !found.findings.is_empty() {
                    actions = actions.child(
                        Button::new("review-correct")
                            .label("Ask Wisp to correct")
                            .small()
                            .primary()
                            .tooltip("Put the corrections in the composer, to send or change")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.ask_for_corrections(window, cx)
                            })),
                    );
                }
            }
        }
        actions = actions.child(
            Button::new("review-close")
                .label("Close")
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.close_review(window, cx))),
        );
        div()
            .id("review-backdrop")
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
                cx.listener(|this, _, window, cx| this.close_review(window, cx)),
            )
            .child(card.child(actions))
    }

    fn review_again(&self, cx: &mut Context<Self>) -> Button {
        Button::new("review-again")
            .label("Review again")
            .small()
            .ghost()
            .on_click(cx.listener(|this, _, window, cx| this.open_review(window, cx)))
    }
}
