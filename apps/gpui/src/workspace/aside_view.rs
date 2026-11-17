//! The Aside tab of the side panel: questions about the open conversation
//! that are answered beside it and never become part of it. A passage
//! selected in a file's text can be quoted into the next question, as the
//! WebView's "Quote in side chat" does.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::icon_button, surfaces};
use crate::{aside, assets::icon, backend::receive, panel::PanelTab};

impl Workspace {
    /// Opens the tab with the caret in its question field.
    pub(super) fn open_aside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_panel_tab(PanelTab::Aside, window, cx);
        self.aside_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Opens the tab to ask about one run, its live snapshot going along.
    pub(super) fn open_aside_about_run(
        &mut self,
        run: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aside_run = Some((run, title));
        self.open_aside(window, cx);
    }

    /// Quotes the passage selected in the previewed file's text into the
    /// next side question and opens the tab, as the WebView's "Quote in
    /// side chat" does with a selection. With nothing selected it says so.
    pub(super) fn quote_selection_aside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.files.preview_path.clone() else {
            return;
        };
        // UTF-16 offsets, as the input gives them.
        let selected = self.file_view.update(cx, |input, cx| {
            let range = input.selected_text_range(false, window, cx)?.range;
            if range.is_empty() {
                return None;
            }
            input.text_for_range(range, &mut None, window, cx)
        });
        match selected.and_then(|text| aside::Quote::of_selection(&path, &text)) {
            Some(quote) => {
                self.file_notice = None;
                self.aside_quotes.push(quote);
                self.open_aside(window, cx);
            }
            None => self.file_notice = Some(tr("Select a passage of the text first.").into()),
        }
        cx.notify();
    }

    /// Asks what is in the question field about the open conversation,
    /// with the quoted passages before it.
    pub(super) fn ask_aside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let typed = self.aside_input.read(cx).value().to_string();
        let question = aside::question_with_quotes(&typed, &self.aside_quotes);
        let thread = self.asides.entry(session.clone()).or_default();
        let Some(earlier) = thread.ask(&question) else {
            return;
        };
        self.aside_quotes.clear();
        self.aside_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = match self.aside_run.clone() {
            Some((run, _)) => {
                self.backend
                    .ask_aside_about_run(project, session.clone(), run, earlier, question)
            }
            None => self
                .backend
                .ask_aside(project, session.clone(), earlier, question),
        };
        cx.spawn_in(window, async move |this, cx| {
            let answer = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Filed with its own conversation, whichever is open now.
                if let Some(thread) = this.asides.get_mut(&session) {
                    thread.answered_with(answer);
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
        // Before anything is asked the hint is what the tab has to say.
        let hint = aside::hint(session.is_some());
        let mut list = div()
            .id("aside-thread")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(if thread.is_some_and(|thread| !thread.is_empty()) {
                surfaces::panel_note(hint, cx).pb_1()
            } else {
                surfaces::panel_empty(hint, cx)
            });
        for (index, entry) in thread.iter().flat_map(|thread| &thread.entries).enumerate() {
            let mut block = surfaces::panel_entry(cx).child(div().child(entry.question.clone()));
            block = match &entry.answer {
                Ok(answer) => {
                    let copied = answer.clone();
                    block
                        .child(
                            surfaces::markdown(
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
                        .child(crate::i18n::host(problem)),
                ),
            };
            // The passages from earlier it was given, as the answer cites them.
            if !entry.sources.is_empty() {
                let mut sources = div().v_flex().gap_1().child(surfaces::panel_section(
                    crate::i18n::fill(tr("Earlier passages ({})"), &[&entry.sources.len()]),
                    cx,
                ));
                for source in &entry.sources {
                    sources = sources.child(
                        div()
                            .v_flex()
                            .pl_2()
                            .border_l_2()
                            .border_color(border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(aside::source_line(source)),
                            )
                            .child(div().text_xs().line_clamp(3).child(source.excerpt.clone())),
                    );
                }
                block = block.child(sources);
            }
            list = list.child(block);
        }
        if let Some(question) = thread.and_then(|thread| thread.asking.as_ref()) {
            list = list.child(
                surfaces::panel_entry(cx)
                    .child(div().child(question.clone()))
                    .child(div().text_color(muted).child(tr("Asking…"))),
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
        // A question about one run says so, and can go back to the
        // conversation as a whole.
        let about = self.aside_run.as_ref().map(|(_, title)| {
            div()
                .h_flex()
                .gap_1()
                .text_xs()
                .text_color(muted)
                .child(div().flex_1().min_w_0().truncate().child(crate::i18n::fill(
                    tr("About the run “{}”: its live state goes along"),
                    &[title],
                )))
                .child(
                    icon_button(
                        "aside-run-clear",
                        "close",
                        tr("Ask about the conversation instead"),
                    )
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.aside_run = None;
                        cx.notify();
                    })),
                )
        });
        // The passages quoted into the next question, each taken back
        // with its ×, as the WebView's side chat shows its quote chips.
        let quotes = (!self.aside_quotes.is_empty()).then(|| {
            let mut chips = div().h_flex().flex_wrap().gap_1();
            for (index, quote) in self.aside_quotes.iter().enumerate() {
                let full = SharedString::from(quote.text.clone());
                chips = chips.child(
                    Self::chip_box(cx)
                        .id(("aside-quote", index))
                        .tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(full.clone()).build(window, cx)
                        })
                        .child(icon("doc").text_color(muted))
                        .child(div().max_w(px(220.)).truncate().child(quote.label()))
                        .child(
                            icon_button(("aside-quote-remove", index), "close", tr("Remove"))
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if index < this.aside_quotes.len() {
                                        this.aside_quotes.remove(index);
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            chips
        });
        div().v_flex().flex_1().min_h_0().child(list).child(
            div()
                .v_flex()
                .flex_shrink_0()
                .gap_2()
                .p_3()
                .border_t_1()
                .border_color(border)
                .children(about)
                .children(quotes)
                .child(super::surfaces::input(&self.aside_input, cx).small())
                .child(actions),
        )
    }
}
