//! The research journey of the open project: a page that shows one month
//! of what happened, grouped by day, and lets the researcher add records
//! of their own.
use crate::i18n::{fill, tr};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use wisp_dto::{ResearchJournalInput, ResearchRecap};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button, nav_row},
    surfaces,
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

    /// Drafts the recap of the local day `[from, until)` again, or for the
    /// first time, and shows the month with it.
    fn draft_recap(&mut self, from: i64, until: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.recap_drafting = Some(from);
        self.error = None;
        let reply = self.backend.draft_recap(project, from, until);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.recap_drafting = None;
                match result {
                    Ok(Some(_)) => this.reload_journey(window, cx),
                    Ok(None) => {
                        this.error = Some(
                            tr("Nothing was recorded that day, so there is nothing to recap.")
                                .to_owned(),
                        )
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Confirms or dismisses a recap as it is.
    fn review_recap(
        &mut self,
        recap: &ResearchRecap,
        status: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.error = None;
        let reply = self
            .backend
            .update_recap(project, journey::reviewed(recap, status));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.reload_journey(window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the form on a recap, filled with what it says now.
    fn edit_recap(&mut self, recap: &ResearchRecap, window: &mut Window, cx: &mut Context<Self>) {
        let values = [
            recap.headline.clone(),
            journey::as_lines(&recap.done),
            journey::as_lines(&recap.findings),
            journey::as_lines(&recap.issues),
            journey::as_lines(&recap.next),
        ];
        for (field, value) in self.recap_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.recap_editing = Some(recap.id.clone());
        self.layers.open(Layer::RecapForm);
        self.recap_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Keeps the changed recap, as it stands otherwise.
    fn save_recap(&mut self, recap: &ResearchRecap, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let [headline, done, findings, issues, next] =
            [0, 1, 2, 3, 4].map(|at| self.recap_fields[at].read(cx).value().to_string());
        let edit = wisp_dto::ResearchRecapEdit {
            id: recap.id.clone(),
            status: recap.status.clone(),
            headline: headline.trim().to_owned(),
            done: journey::from_lines(&recap.done, &done),
            findings: journey::from_lines(&recap.findings, &findings),
            issues: journey::from_lines(&recap.issues, &issues),
            next: journey::from_lines(&recap.next, &next),
        };
        self.error = None;
        let reply = self.backend.update_recap(project, edit);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.recap_editing = None;
                        this.layers.close(Layer::RecapForm);
                        this.reload_journey(window, cx);
                    }
                    // The form stays with what was typed.
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn recap_form(&self, recap: &ResearchRecap, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let list = |field: &Entity<InputState>| div().h(px(72.)).child(Input::new(field).h_full());
        let saved = recap.clone();
        div()
            .v_flex()
            .gap_1()
            .child(label(tr("Headline")))
            .child(Input::new(&self.recap_fields[0]).small())
            .child(label(tr("Done, one a line")))
            .child(list(&self.recap_fields[1]))
            .child(label(tr("Found, one a line")))
            .child(list(&self.recap_fields[2]))
            .child(label(tr("In the way, one a line")))
            .child(list(&self.recap_fields[3]))
            .child(label(tr("Next, one a line")))
            .child(list(&self.recap_fields[4]))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("recap-save")
                            .label(tr("Save"))
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_recap(&saved, window, cx)
                            })),
                    )
                    .child(
                        Button::new("recap-cancel")
                            .label(tr("Cancel"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.recap_editing = None;
                                this.layers.close(Layer::RecapForm);
                                cx.notify();
                            })),
                    ),
            )
    }

    fn reload_journey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(month) = self.journey_month {
            self.show_month(month, window, cx);
        }
    }

    /// A day's recap: how it stands, its headline, what it says in each
    /// section, and its review buttons.
    fn recap_card(&self, index: usize, recap: &ResearchRecap, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (status, tone) = journey::recap_status(&recap.status);
        let mut head = div().h_flex().gap_2().child(chip(status, tone, cx)).child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(recap.headline.clone()),
        );
        if recap.status != "confirmed" {
            let confirmed = recap.clone();
            head = head.child(
                Button::new(("recap-confirm", index))
                    .label(tr("Confirm"))
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.review_recap(&confirmed, "confirmed", window, cx)
                    })),
            );
        }
        let edited = recap.clone();
        head = head.child(
            Button::new(("recap-edit", index))
                .label(tr("Change"))
                .xsmall()
                .ghost()
                .on_click(
                    cx.listener(move |this, _, window, cx| this.edit_recap(&edited, window, cx)),
                ),
        );
        let dismissed = recap.clone();
        head = head.child(
            Button::new(("recap-dismiss", index))
                .label(tr("Dismiss"))
                .xsmall()
                .ghost()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.review_recap(&dismissed, "dismissed", window, cx)
                })),
        );
        let mut card = div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(head);
        if self.layers.contains(Layer::RecapForm)
            && self.recap_editing.as_deref() == Some(recap.id.as_str())
        {
            return card.child(self.recap_form(recap, cx));
        }
        for (label, items) in journey::recap_sections(recap) {
            let mut section = div()
                .v_flex()
                .gap_0p5()
                .child(div().text_xs().text_color(muted).child(label));
            for item in items {
                section = section.child(div().text_sm().child(format!("• {item}")));
            }
            card = card.child(section);
        }
        card.child(div().text_xs().text_color(muted).child(fill(
            tr("Drafted by {} from this day's records."),
            &[&recap.model],
        )))
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
                    .label(tr(label))
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
                    .child(tr("Add a record")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Something you want the project to remember as of now. The model does not \
                 write these; you do.",
            )))
            .child(kinds)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("What happened, in a line")),
            )
            .child(Input::new(&self.journal_fields[0]).small())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Details, if any")),
            )
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
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_journal_entry(window, cx)
                            })),
                    )
                    .child(
                        Button::new("journal-cancel")
                            .label(tr("Cancel"))
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
                nav_row("journey-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_journey(window, cx))),
            )
            .child(surfaces::title(tr("Research journey")).px_2().pt_3())
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(project_name),
            )
            .child(div().px_2().pb_2().text_xs().text_color(muted).child(tr(
                "What happened in this project, day by day: what you asked, runs, files, \
                     decisions, and what you wrote down yourself.",
            )))
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .px_1()
                    .child(
                        icon_button("journey-previous", "chevron-left", tr("Earlier month"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_month(month.previous(), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(month.label()),
                    )
                    .child(
                        icon_button("journey-next", "chevron-right", tr("Later month"))
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
                        .label(tr("This month"))
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
                        .label(tr("Add a record"))
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
                            tr("Loading…")
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
            let recap = journey::recap_for(history, day.date, &Local);
            let (from, until) = journey::day_range(day.date, &Local);
            let drafting = self.recap_drafting == Some(from);
            let header = div()
                .h_flex()
                .gap_2()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(day.label.clone()),
                )
                .child(div().flex_1())
                .child(
                    Button::new(("recap-draft", day_index))
                        .label(match (drafting, recap.is_some()) {
                            (true, _) => tr("Drafting…"),
                            (false, true) => tr("Draft again"),
                            (false, false) => tr("Draft a recap"),
                        })
                        .xsmall()
                        .ghost()
                        .disabled(self.recap_drafting.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.draft_recap(from, until, window, cx)
                        })),
                );
            let mut group = div().v_flex().gap_1().child(header);
            if let Some(recap) = recap {
                group = group.child(self.recap_card(day_index, recap, cx));
            }
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
