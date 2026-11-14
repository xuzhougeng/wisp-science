//! The research journey of the open project: a page that shows one month
//! of what happened, grouped by day, and lets the researcher add records
//! of their own.
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::ResearchJournalInput;

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button, nav_row},
};
use crate::{
    assets::icon,
    backend::receive,
    journey::{self, Month},
    layers::Layer,
    present::Tone,
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

impl Workspace {
    /// Opens the page on the current month of the open project.
    pub(super) fn open_journey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.project.is_none() {
            return;
        }
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Journey);
        self.error = None;
        self.journey_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.show_month(Month::containing(now(), &Local), window, cx);
    }

    pub(super) fn close_journey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Journey) {
            self.journey = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Reads `month` of the open project's history. Only the answer to the
    /// latest request is shown.
    fn show_month(&mut self, month: Month, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.journey_month = Some(month);
        self.journey = None;
        self.journey_request += 1;
        let request = self.journey_request;
        let (from, until) = month.range(&Local);
        let reply = self.backend.journey(project, from, until);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.journey_request != request {
                    return;
                }
                match result {
                    Ok(journey) => this.journey = Some(journey),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_journal_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.open(Layer::JournalForm);
        self.error = None;
        for field in &self.journal_fields {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.journal_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Saves the record as of now and shows the month it is in.
    fn save_journal_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::JournalForm) {
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let text = |index: usize| self.journal_fields[index].read(cx).value().to_string();
        let at = now();
        let reply = self.backend.add_journal_entry(
            project,
            ResearchJournalInput {
                title: text(0),
                body: text(1),
                category: self.journal_category.to_owned(),
                occurred_at: at,
            },
        );
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.layers.close(Layer::JournalForm);
                        this.show_month(Month::containing(at, &Local), window, cx);
                    }
                    // A refused record keeps the form and what was typed.
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn journal_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut kinds = div().h_flex().gap_1();
        for (category, label) in journey::CATEGORIES {
            kinds = kinds.child(
                Button::new(SharedString::from(format!("journal-kind-{category}")))
                    .label(label)
                    .xsmall()
                    .selected(self.journal_category == category)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.journal_category = category;
                        cx.notify();
                    })),
            );
        }
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Add a record"),
            )
            .child(div().text_xs().text_color(muted).child(
                "Something you want the project to remember as of now. The model does not \
                 write these; you do.",
            ))
            .child(kinds)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("What happened, in a line"),
            )
            .child(Input::new(&self.journal_fields[0]).small())
            .child(div().text_xs().text_color(muted).child("Details, if any"))
            .child(
                div()
                    .h(px(120.))
                    .child(Input::new(&self.journal_fields[1]).h_full()),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("journal-save")
                            .label("Save")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_journal_entry(window, cx)
                            })),
                    )
                    .child(
                        Button::new("journal-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::JournalForm);
                                cx.notify();
                            })),
                    ),
            )
    }

    pub(super) fn render_journey(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let hover = cx.theme().list_hover;
        let month = self
            .journey_month
            .unwrap_or_else(|| Month::containing(now(), &Local));
        let this_month = Month::containing(now(), &Local);
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_default();

        let side = div()
            .v_flex()
            .w(px(260.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                nav_row("journey-back", "arrow-left", "Back", false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_journey(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Research journey"),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(project_name),
            )
            .child(div().px_2().pb_2().text_xs().text_color(muted).child(
                "What happened in this project, day by day: what you asked, runs, files, \
                     decisions, and what you wrote down yourself.",
            ))
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .px_1()
                    .child(
                        icon_button("journey-previous", "chevron-left", "Earlier month").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.show_month(month.previous(), window, cx)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(month.label()),
                    )
                    .child(
                        icon_button("journey-next", "chevron-right", "Later month")
                            // There is nothing after the present.
                            .disabled(month == this_month)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_month(month.next(), window, cx)
                            })),
                    ),
            )
            .child(
                div().px_1().pt_1().child(
                    Button::new("journey-today")
                        .label("This month")
                        .xsmall()
                        .ghost()
                        .disabled(month == this_month)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_month(this_month, window, cx)
                        })),
                ),
            )
            .child(
                div().pt_2().child(
                    Input::new(&self.journey_search)
                        .small()
                        .prefix(icon("search").text_color(muted)),
                ),
            )
            .child(
                div().px_1().pt_2().child(
                    Button::new("journey-add")
                        .icon(glyph("plus"))
                        .label("Add a record")
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_journal_form(window, cx)),
                        ),
                ),
            );

        let mut body = div()
            .id("journey-body")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .gap_3()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        if self.layers.contains(Layer::JournalForm) {
            body = body.child(div().w(px(560.)).child(self.journal_form(cx)));
        }
        let Some(history) = &self.journey else {
            return div().h_flex().size_full().child(side).child(
                body.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(if self.error.is_some() {
                            ""
                        } else {
                            "Loading…"
                        }),
                ),
            );
        };
        let filter = self.journey_search.read(cx).value().to_string();
        let days = journey::days(history, &filter, &Local);
        let shown = days.iter().map(|day| day.entries.len()).sum();
        body = body.child(
            div()
                .text_xs()
                .text_color(muted)
                .child(journey::summary(history, shown)),
        );
        for (day_index, day) in days.iter().enumerate() {
            let mut group = div().v_flex().gap_1().child(
                div()
                    .pt_2()
                    .pb_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(day.label.clone()),
            );
            for (entry_index, (time, entry)) in day.entries.iter().enumerate() {
                let tone = match (entry.kind.as_str(), entry.status.as_str()) {
                    ("run", "failed") => Tone::Danger,
                    ("run", _) => Tone::Active,
                    _ => Tone::Neutral,
                };
                let mut words = div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_sm().child(entry.title.clone()));
                let detail = entry.summary.lines().next().unwrap_or_default().trim();
                if !detail.is_empty() {
                    words = words.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(detail.to_owned()),
                    );
                }
                let mut row = div()
                    .id(("journey-entry", day_index * 10_000 + entry_index))
                    .h_flex()
                    .items_start()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(time.clone()),
                    )
                    .child(div().w(px(84.)).flex_shrink_0().child(chip(
                        journey::kind_label(&entry.kind),
                        tone,
                        cx,
                    )))
                    .child(words);
                if let Some(status) = journey::status_label(entry) {
                    row = row.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(status),
                    );
                }
                // A record that came from a conversation opens it.
                if let (Some(project), Some(frame)) =
                    (self.state.project.clone(), entry.frame_id.clone())
                {
                    row = row
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.layers.close(Layer::Journey);
                            this.journey = None;
                            this.open_session(project.clone(), frame.clone(), window, cx);
                        }));
                }
                group = group.child(row);
            }
            body = body.child(group.border_b_1().border_color(border).pb_2());
        }
        div().h_flex().size_full().child(side).child(body)
    }
}
