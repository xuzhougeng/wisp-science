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
    input::InputState,
};
use wisp_dto::{ResearchJournalInput, ResearchRecap};

use super::{
    Workspace,
    shell::{error_banner, glyph, nav_row},
    surfaces::{self, DialogWidth},
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
        self.forget_journey_extras();
        self.journey_graph_shown = false;
        self.journey_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.show_month(Month::containing(now(), &Local), window, cx);
    }

    pub(super) fn close_journey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Journey) {
            self.journey = None;
            self.forget_journey_extras();
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
                    Ok(journey) => {
                        // As in the WebView, the month's first output is
                        // traced unless one that is still there already is.
                        let kept = this.journey_output.as_ref().is_some_and(|output| {
                            journey.entries.iter().any(|entry| entry.id == output.id)
                        });
                        let first = journey
                            .entries
                            .iter()
                            .find(|entry| entry.kind == "artifact")
                            .cloned();
                        this.journey = Some(journey);
                        if !kept {
                            match first {
                                Some(entry) => this.trace_output(entry, cx),
                                None => {
                                    this.journey_output = None;
                                    this.journey_source = None;
                                }
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Drops what the page read besides the month: the graph, the traced
    /// output and the run record, and closes what showed them.
    pub(super) fn forget_journey_extras(&mut self) {
        self.journey_graph = None;
        self.journey_edge = None;
        self.journey_output = None;
        self.journey_source = None;
        self.journey_run = None;
        self.layers.close(Layer::JourneyEdge);
        self.layers.close(Layer::JourneyRun);
    }

    /// Shows the days, or the project's research graph read afresh.
    pub(super) fn show_journey_graph(
        &mut self,
        shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.journey_graph_shown = shown;
        self.journey_edge = None;
        self.layers.close(Layer::JourneyEdge);
        cx.notify();
        if !shown {
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.research_graph(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(graph) => this.journey_graph = Some(graph),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the details of one relation of the graph.
    pub(super) fn open_journey_edge(
        &mut self,
        edge: wisp_dto::ResearchEdge,
        cx: &mut Context<Self>,
    ) {
        self.journey_edge = Some(edge);
        self.layers.open(Layer::JourneyEdge);
        cx.notify();
    }

    /// Shows where an output of the month came from, beside the days.
    fn trace_output(&mut self, entry: wisp_dto::ResearchJourneyEntry, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let version = entry.source_id.clone();
        self.journey_output = Some(entry);
        self.journey_source = None;
        let reply = self.backend.journey_source(project, version.clone());
        cx.spawn(async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Only the answer for the output still chosen is shown.
                if this
                    .journey_output
                    .as_ref()
                    .is_some_and(|output| output.source_id == version)
                {
                    this.journey_source = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens a run's record over the page.
    fn open_journey_run(&mut self, run: String, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.journey_run = Some((run.clone(), None));
        self.layers.open(Layer::JourneyRun);
        let reply = self.backend.journey_run(project, run.clone());
        cx.spawn(async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| match this.journey_run.as_mut() {
                Some((shown, record)) if *shown == run => {
                    *record = Some(result);
                    cx.notify();
                }
                _ => {}
            });
        })
        .detach();
        cx.notify();
    }

    fn close_journey_run(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::JourneyRun);
        self.journey_run = None;
        cx.notify();
    }

    /// Leaves the page for a file of the project, shown in the Files panel.
    fn open_journey_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_journey(window, cx);
        self.reveal_file(path, window, cx);
        cx.notify();
    }

    /// Leaves the page for one of the project's conversations.
    fn open_journey_conversation(
        &mut self,
        frame: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.layers.close(Layer::Journey);
        self.journey = None;
        self.forget_journey_extras();
        self.open_session(project, frame, window, cx);
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
        let list = |field: &Entity<InputState>| {
            div()
                .h(px(72.))
                .child(super::surfaces::input(field, cx).h_full())
        };
        let saved = recap.clone();
        div()
            .v_flex()
            .gap_1()
            .child(label(tr("Headline")))
            .child(super::surfaces::input(&self.recap_fields[0], cx).small())
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
        let mut head = div()
            .h_flex()
            .gap_2()
            .child(super::surfaces::status(status, tone, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
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
            .child(div().text_sm().child(tr("Add a record")))
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
            .child(super::surfaces::input(&self.journal_fields[0], cx).small())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Details, if any")),
            )
            .child(
                div()
                    .h(px(120.))
                    .child(super::surfaces::input(&self.journal_fields[1], cx).h_full()),
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
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pt_3()
                    .child(surfaces::title(tr("Research journey")))
                    // What the page is for, a tooltip away, not a paragraph
                    // in the rail.
                    .child(surfaces::help(
                        "journey-help",
                        tr(
                            "What happened in this project, day by day: what you asked, runs, files, \
                     decisions, and what you wrote down yourself.",
                        ),
                        cx,
                    )),
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
            // By day or as a graph of relations: one segmented choice.
            .child(
                div().px_1().pt_2().pb_2().child(surfaces::segmented(
                    [
                        surfaces::segment(
                            "journey-by-day",
                            "calendar",
                            tr("By day"),
                            !self.journey_graph_shown,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_journey_graph(false, window, cx)
                        }))
                        .into_any_element(),
                        surfaces::segment(
                            "journey-relationships",
                            "share",
                            tr("Relationships"),
                            self.journey_graph_shown,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_journey_graph(true, window, cx)
                        }))
                        .into_any_element(),
                    ],
                    cx,
                )),
            )
            // The month, as a fact between its arrows; this month a click
            // away.
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_0p5()
                    .px_1()
                    .child(
                        surfaces::icon_action(
                            "journey-previous",
                            "chevron-left",
                            tr("Earlier month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_month(month.previous(), window, cx)
                        })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_flex()
                            .justify_center()
                            .child(surfaces::fact(month.label(), cx).text_color(cx.theme().foreground)),
                    )
                    .child(
                        surfaces::icon_action(
                            "journey-next",
                            "chevron-right",
                            tr("Later month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        // There is nothing after the present.
                        .disabled(month == this_month)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_month(month.next(), window, cx)
                        })),
                    )
                    .child(
                        surfaces::icon_action(
                            "journey-today",
                            "calendar",
                            tr("This month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(month == this_month)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_month(this_month, window, cx)
                        })),
                    ),
            )
            .child(
                div().pt_2().child(
                    surfaces::field(cx).child(
                        surfaces::input(&self.journey_search, cx)
                            .small()
                            .appearance(false)
                            .prefix(icon("search").text_color(muted)),
                    ),
                ),
            )
            .child(
                div().px_1().pt_2().child(
                    surfaces::outlined_action("journey-add", "plus", tr("Add a record"), cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_journal_form(window, cx)),
                        ),
                ),
            );

        if self.journey_graph_shown {
            return div()
                .h_flex()
                .size_full()
                .child(side)
                .child(self.render_journey_graph(cx));
        }

        // The records in a reading column centred in the page.
        let mut body = div()
            .v_flex()
            .w_full()
            .max_w(px(surfaces::READING_WIDTH))
            .mx_auto()
            .p_6()
            .gap_3();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        if self.layers.contains(Layer::JournalForm) {
            body = body.child(div().w(px(560.)).child(self.journal_form(cx)));
        }
        let scroller = |body: Div| {
            div()
                .id("journey-body")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(body)
        };
        let Some(history) = &self.journey else {
            return div().h_flex().size_full().child(side).child(scroller(
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
            ));
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
            // The day: a faded heading, its date as a fact; drafting its
            // recap is an icon and a word at its right.
            let header = div()
                .h_flex()
                .items_center()
                .gap_2()
                .pt_3()
                .pb_1()
                .child(surfaces::fact(day.label.clone(), cx))
                .child(div().flex_1())
                .child(
                    surfaces::labelled_action(
                        ("recap-draft", day_index),
                        "square-pen",
                        match (drafting, recap.is_some()) {
                            (true, _) => tr("Drafting…"),
                            (false, true) => tr("Draft again"),
                            (false, false) => tr("Draft a recap"),
                        },
                        surfaces::IconSize::Inline,
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .disabled(self.recap_drafting.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.draft_recap(from, until, window, cx)
                    })),
                );
            let mut group = div().v_flex().gap_1().child(header);
            if let Some(recap) = recap {
                group = group.child(self.recap_card(day_index, recap, cx));
            }
            // Its records: when, the kind's icon (no word), what happened
            // and the first line of what it says, its state as a stamp.
            let mut rows = Vec::new();
            for (entry_index, (time, entry)) in day.entries.iter().enumerate() {
                let mut row = surfaces::record_row(
                    ("journey-entry", day_index * 10_000 + entry_index),
                    crate::present::title_line(&entry.title),
                )
                .time(time.clone())
                .icon(journey::kind_icon(&entry.kind));
                let detail = entry.summary.lines().next().unwrap_or_default().trim();
                if !detail.is_empty() {
                    row = row.suffix(detail.to_owned());
                }
                if let Some(status) = journey::status_label(entry) {
                    row = match surfaces::StampKind::from_tone(journey::status_tone(entry)) {
                        Some(kind) => row.stamp(kind, status),
                        None => row.suffix(status),
                    };
                }
                // An output is traced beside the days, as in the WebView;
                // another record that came from a conversation opens it.
                if entry.kind == "artifact" {
                    let traced = (*entry).clone();
                    let chosen = self
                        .journey_output
                        .as_ref()
                        .is_some_and(|output| output.id == entry.id);
                    row =
                        row.cursor_pointer()
                            .when(chosen, |row| row.bg(hover))
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.trace_output(traced.clone(), cx)
                            }));
                } else if let (Some(project), Some(frame)) =
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
                rows.push(row);
            }
            if !rows.is_empty() {
                group = group.child(surfaces::record_group(rows, cx));
            }
            body = body.child(group);
        }
        div()
            .h_flex()
            .size_full()
            .child(side)
            .child(scroller(body))
            .child(self.journey_source_panel(cx))
    }

    /// Where the traced output came from, as the WebView's source details
    /// show it: its version, the data its run read, the run itself, and
    /// the way to the output, the run's record and the conversation.
    fn journey_source_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        // The page's right margin, drawn as the side panel is: its frame,
        // a header, sections in faded caption, the panel's inset.
        let frame = surfaces::side_panel(cx).w(px(300.)).flex_shrink_0();
        let mut panel = div()
            .id("journey-source")
            .v_flex()
            .flex_1()
            .min_h_0()
            .px_3()
            .pb_3()
            .gap_2()
            .overflow_y_scroll();
        let header = surfaces::panel_header(tr("Source details"), None, cx);
        let Some(output) = &self.journey_output else {
            return frame.child(header).child(surfaces::panel_empty(
                tr("Choose an output to trace where it came from."),
                cx,
            ));
        };
        let heading = |text: &'static str, glyph_name: &str| {
            surfaces::panel_section(text, cx)
                .h_flex()
                .items_center()
                .gap_1()
                .child(icon(glyph_name).text_color(muted))
        };
        panel = panel
            .child(div().text_sm().child(output.title.clone()))
            .child(div().text_xs().text_color(muted).child(format!(
                "{} · {}",
                journey::local_moment(output.occurred_at, &Local),
                fill(tr("Version {}"), &[&output.version_number.unwrap_or(1)])
            )));
        if !output.summary.trim().is_empty() {
            panel = panel.child(div().text_xs().child(output.summary.clone()));
        }
        match &self.journey_source {
            None => {
                panel = panel.child(div().text_xs().text_color(muted).child(tr("Reading…")));
            }
            Some(Err(error)) => {
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(error)),
                );
            }
            Some(Ok(found)) => {
                panel = panel.child(heading(tr("Input data"), "database"));
                if found.source.inputs.is_empty() {
                    panel = panel.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("No inputs recorded")),
                    );
                }
                for (index, input) in found.source.inputs.iter().enumerate() {
                    let path = found.input_paths.get(index).cloned().flatten();
                    let name: AnyElement = match path {
                        Some(path) => Button::new(("journey-input", index))
                            .label(input.title.clone())
                            .xsmall()
                            .ghost()
                            .tooltip(tr("Open this file in the Files panel"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_journey_file(path.clone(), window, cx)
                            }))
                            .into_any_element(),
                        None => div()
                            .text_xs()
                            .truncate()
                            .child(input.title.clone())
                            .into_any_element(),
                    };
                    panel = panel.child(
                        div().v_flex().child(name).child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(journey::input_detail(input)),
                        ),
                    );
                }
                let (run_title, run_lines) = journey::source_run_lines(&found.source, &Local);
                panel = panel
                    .child(heading(tr("Experiment"), "flask"))
                    .child(div().text_xs().child(run_title));
                for line in run_lines {
                    panel = panel.child(div().text_xs().text_color(muted).child(line));
                }
                if let Some(run) = found.source.run_id.clone() {
                    panel = panel.child(
                        Button::new("journey-run-record")
                            .icon(glyph("flask"))
                            .label(tr("View run record"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_journey_run(run.clone(), cx)
                            })),
                    );
                }
                let path = found.path.clone().filter(|_| !output.source_discarded);
                panel = panel.child(
                    Button::new("journey-open-output")
                        .icon(glyph("external-link"))
                        .label(if path.is_some() {
                            tr("Open output")
                        } else {
                            tr("Source unavailable")
                        })
                        .xsmall()
                        .ghost()
                        .disabled(path.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(path) = path.clone() {
                                this.open_journey_file(path, window, cx)
                            }
                        })),
                );
            }
        }
        if let Some(frame) = output.frame_id.clone() {
            panel = panel.child(
                Button::new("journey-output-conversation")
                    .icon(glyph("chat"))
                    .label(tr("Open the conversation"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_journey_conversation(frame.clone(), window, cx)
                    })),
            );
        }
        frame.child(header).child(panel)
    }

    /// A run's record over the page: how it ended, where it ran, when, its
    /// command, and the last of what it wrote.
    pub(super) fn render_journey_run(
        &self,
        (_, record): &(
            String,
            Option<Result<wisp_server::desktop::JourneyRun, String>>,
        ),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut card = surfaces::dialog("journey-run", DialogWidth::Regular, tr("Run record"))
            .close_on_click_outside(cx.listener(|this, _, _, cx| this.close_journey_run(cx)))
            .action(
                Button::new("journey-run-close")
                    .label(tr("Close"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.close_journey_run(cx))),
            );
        let run = match record {
            None => return card.child(div().text_sm().text_color(muted).child(tr("Reading…"))),
            Some(Err(error)) => {
                return card.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(error)),
                );
            }
            Some(Ok(run)) => run,
        };
        let not_recorded = || tr("Not recorded").to_owned();
        let moment = |at: Option<i64>| {
            at.map(|at| journey::local_moment(at, &Local))
                .unwrap_or_else(not_recorded)
        };
        let tone = match run.status.as_str() {
            "succeeded" => Tone::Success,
            "failed" | "lost" => Tone::Danger,
            _ => Tone::Neutral,
        };
        let fact = |label: &'static str, value: String| {
            div()
                .h_flex()
                .gap_2()
                .text_xs()
                .child(div().w(px(150.)).text_color(muted).child(label))
                .child(div().flex_1().min_w_0().child(value))
        };
        let text = |label: &'static str, value: &Option<String>, empty: &'static str| {
            let block = div()
                .v_flex()
                .gap_1()
                .pt_1()
                .child(div().text_xs().text_color(muted).child(label));
            match value.as_deref().filter(|value| !value.trim().is_empty()) {
                Some(value) => block.child(
                    div()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(value.to_owned()),
                ),
                None => block.child(div().text_xs().text_color(muted).child(empty)),
            }
        };
        card = card
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_sm().child(run.title.clone()))
                    .child(super::surfaces::status(
                        journey::run_status(&run.status),
                        tone,
                        cx,
                    )),
            )
            .child(fact(tr("Execution context"), run.context.clone()))
            .child(fact(
                tr("Exit code"),
                run.exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(not_recorded),
            ))
            .child(fact(tr("Started (local time)"), moment(run.started_at)))
            .child(fact(tr("Ended (local time)"), moment(run.ended_at)))
            .child(text(tr("Command"), &run.command, tr("No command recorded")))
            .child(text(
                tr("Standard output · log tail"),
                &run.stdout_tail,
                tr("No standard output recorded"),
            ))
            .child(text(
                tr("Standard error · log tail"),
                &run.stderr_tail,
                tr("No standard error recorded"),
            ));
        card
    }
}
