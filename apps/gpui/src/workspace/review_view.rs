//! The review of the open conversation: a dialog that has the model read
//! the conversation as a reviewer and lists what it found, with a way to
//! ask for the corrections.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{
    Workspace,
    shell::error_banner,
    surfaces::{self, DialogWidth},
    tone_color,
};
use crate::{
    assets::icon,
    backend::receive,
    layers::Layer,
    review::{self, Asked},
};

impl Workspace {
    /// Opens the dialog and asks for a review of the open conversation.
    pub(super) fn open_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review_turn = None;
        self.ask_review(window, cx);
    }

    /// Opens the dialog and asks for a review of the turn the transcript
    /// row at `index` belongs to, without what came after it.
    pub(super) fn open_review_of(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(turn) = crate::controls::turn_of(&self.rows, index) else {
            return;
        };
        self.review_turn = Some(turn);
        self.ask_review(window, cx);
    }

    /// Asks for the review the dialog is about: one turn, or all of it.
    fn ask_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        // A frozen branch is not reviewed; asked from the palette, it says
        // so in the host's words instead.
        if !self.offered().review {
            self.notice = Some(crate::i18n::host(
                wisp_server::desktop::FROZEN_BRANCH_CHANGE_REFUSAL,
            ));
            cx.notify();
            return;
        }
        self.layers.open(Layer::Review);
        self.review = Some(Asked::Reading);
        let reply = match self.review_turn.clone() {
            Some(turn) => self.backend.review_turn(project, session.clone(), turn),
            None => self.backend.review_conversation(project, session.clone()),
        };
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
    pub(super) fn see_review(
        &mut self,
        found: wisp_server::desktop::Review,
        cx: &mut Context<Self>,
    ) {
        self.review = Some(Asked::Done(found));
        // Review again then reviews the conversation as it is now.
        self.review_turn = None;
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
    pub(super) fn render_review_banner(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let outcome = self.status.as_ref()?.review.as_ref()?;
        let (said, tone, opens) = review::banner(outcome);
        let tone = tone_color(tone, cx);
        let reading = *outcome == wisp_server::desktop::ReviewOutcome::Reading;
        let mut line = surfaces::bar("review-banner", cx)
            .h_flex()
            .gap_2()
            .child(icon("review").text_color(tone))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone)
                    .child(said),
            )
            // While the reviewer reads, the WebView's line under its title
            // follows it; once the report is in, the result stands alone.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .when(reading, |note| note.child(review::reading().1)),
            );
        if let (true, wisp_server::desktop::ReviewOutcome::Done(found)) = (opens, outcome) {
            let found = found.clone();
            line = line.child(
                Button::new("review-banner-see")
                    .label(tr("See"))
                    .small()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.see_review(found.clone(), cx)),
                    ),
            );
        }
        if !reading {
            line = line.child(
                super::shell::icon_button("review-banner-dismiss", "close", tr("Put this away"))
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
        self.fill_composer(text, window, cx);
        self.close_review(window, cx);
    }

    pub(super) fn render_review(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let title = if self.review_turn.is_some() {
            tr("Review of one turn")
        } else {
            tr("Review")
        };
        let mut card = surfaces::dialog("review", DialogWidth::Regular, title)
            .close_on_click_outside(
                cx.listener(|this, _, window, cx| this.close_review(window, cx)),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "The conversation's model reads it as a reviewer and reports claims that \
                 its own tool output does not support. Nothing is added to the conversation.",
            )));
        // The main action, after Close at the right end.
        let mut main = None;
        match &self.review {
            None | Some(Asked::Reading) => {
                // In the WebView's words for a review that is running.
                let (title, note) = review::reading();
                card = card.child(
                    div()
                        .py_4()
                        .v_flex()
                        .gap_0p5()
                        .child(div().text_sm().child(title))
                        .child(div().text_xs().text_color(muted).child(note)),
                );
            }
            Some(Asked::Failed(error)) => {
                card = card
                    .child(error_banner(error, cx))
                    .action(self.review_again(cx));
            }
            Some(Asked::Done(found)) => {
                let (outcome, tone) = review::outcome(found);
                card = card
                    .child(
                        div()
                            .pt_2()
                            .text_sm()
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
                                    .text_color(tone_color(tone, cx))
                                    .child(heading),
                            )
                            .child(line(tr("Claim"), &finding.claim))
                            .child(line(tr("Evidence"), &finding.evidence))
                            .child(line(tr("Fix"), &finding.fix)),
                    );
                }
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::fill(tr("Reviewed by {}."), &[&found.model])),
                );
                card = card.action(self.review_again(cx));
                if !found.findings.is_empty() {
                    main = Some(
                        Button::new("review-correct")
                            .label(tr("Ask Wisp to correct"))
                            .small()
                            .primary()
                            .tooltip(tr("Put the corrections in the composer, to send or change"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.ask_for_corrections(window, cx)
                            })),
                    );
                }
            }
        }
        card.action(
            Button::new("review-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.close_review(window, cx))),
        )
        .actions(main)
    }

    fn review_again(&self, cx: &mut Context<Self>) -> Button {
        Button::new("review-again")
            .label(tr("Review again"))
            .small()
            .ghost()
            .on_click(cx.listener(|this, _, window, cx| this.ask_review(window, cx)))
    }
}
