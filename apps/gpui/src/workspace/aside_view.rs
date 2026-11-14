//! The Aside tab of the side panel: questions about the open conversation
//! that are answered beside it and never become part of it.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};

use super::{Workspace, shell::icon_button};
use crate::{aside, backend::receive, panel::PanelTab};

impl Workspace {
    /// Opens the tab with the caret in its question field.
    pub(super) fn open_aside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_panel_tab(PanelTab::Aside, window, cx);
        self.aside_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Asks what is in the question field about the open conversation.
    pub(super) fn ask_aside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let question = self.aside_input.read(cx).value().to_string();
        let thread = self.asides.entry(session.clone()).or_default();
        let Some(earlier) = thread.ask(&question) else {
            return;
        };
        self.aside_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self
            .backend
            .ask_aside(project, session.clone(), earlier, question);
        cx.spawn_in(window, async move |this, cx| {
            let answer = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Filed with its own conversation, whichever is open now.
                if let Some(thread) = this.asides.get_mut(&session) {
                    thread.answered(answer);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_aside_tab(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let session = self.state.session.clone();
        let thread = session.as_ref().and_then(|id| self.asides.get(id));
        let asking = thread.is_some_and(|thread| thread.asking.is_some());
        let mut list = div()
            .id("aside-thread")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3()
            .p_3()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(aside::hint(session.is_some())),
            );
        for (index, entry) in thread.iter().flat_map(|thread| &thread.entries).enumerate() {
            let mut block = div().v_flex().gap_1().child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(entry.question.clone()),
            );
            block = match &entry.answer {
                Ok(answer) => {
                    let copied = answer.clone();
                    block
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("aside-answer-{index}")),
                                answer.clone(),
                                window,
                                cx,
                            )
                            .selectable(true),
                        )
                        .child(
                            div().h_flex().child(
                                icon_button(
                                    SharedString::from(format!("aside-copy-{index}")),
                                    "copy",
                                    tr("Copy the answer"),
                                )
                                .xsmall()
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copied.clone(),
                                        ));
                                    },
                                )),
                            ),
                        )
                }
                Err(problem) => block.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(problem.clone()),
                ),
            };
            list = list.child(block.pb_2().border_b_1().border_color(border));
        }
        if let Some(question) = thread.and_then(|thread| thread.asking.as_ref()) {
            list = list.child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(question.clone()),
                    )
                    .child(div().text_sm().text_color(muted).child(tr("Asking…"))),
            );
        }
        let mut actions = div().h_flex().gap_2().justify_end();
        if thread.is_some_and(|thread| !thread.is_empty()) {
            actions = actions.child(
                Button::new("aside-clear")
                    .label(tr("Clear"))
                    .small()
                    .ghost()
                    .tooltip(tr("Forget these questions and answers"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(session) = &this.state.session {
                            if let Some(thread) = this.asides.get_mut(session) {
                                thread.clear();
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        actions = actions.child(
            Button::new("aside-ask")
                .label(tr("Ask"))
                .small()
                .primary()
                .disabled(session.is_none() || asking)
                .on_click(cx.listener(|this, _, window, cx| this.ask_aside(window, cx))),
        );
        div().v_flex().flex_1().min_h_0().child(list).child(
            div()
                .v_flex()
                .flex_shrink_0()
                .gap_2()
                .p_3()
                .border_t_1()
                .border_color(border)
                .child(Input::new(&self.aside_input).small())
                .child(actions),
        )
    }
}
