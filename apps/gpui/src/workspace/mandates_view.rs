//! Research mandates: a page of the standing goals projects' agents work on
//! in rounds, each with where it stands, what it waits on, its latest
//! report, and what can be done with it.
use crate::i18n::{fill, tr};
use chrono::Local;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::MandateOverview;

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, nav_row},
    surfaces::{self, switch},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    mandates,
    present::Tone,
};

impl Workspace {
    pub(super) fn open_mandates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Mandates);
        self.error = None;
        self.load_mandates(window, cx);
    }

    fn close_mandates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Mandates) {
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn load_mandates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.mandates();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(rows) => this.mandates = Some(rows),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change, then reads the page again; a refusal keeps what
    /// was typed.
    fn change_mandates<T: Send + 'static>(
        &mut self,
        reply: Reply<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.layers.close(Layer::MandateForm);
                        this.layers.close(Layer::ConfirmDeleteMandate);
                        this.mandate_deleting = None;
                        this.load_mandates(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form on a mandate, or empty on the open project.
    fn open_mandate_form(
        &mut self,
        mandate: Option<&MandateOverview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.open(Layer::MandateForm);
        self.error = None;
        let record = mandate.map(|overview| &overview.mandate);
        self.mandate_editing = record.map(|mandate| mandate.id.clone());
        self.mandate_project = record
            .map(|mandate| mandate.project_id.clone())
            .or_else(|| {
                self.state
                    .project
                    .clone()
                    .filter(|project| !crate::assistant::is_assistant_project(Some(project)))
            })
            .or_else(|| self.projects.first().map(|project| project.id.clone()));
        self.mandate_interval = record.map_or(86_400, |mandate| mandate.interval_secs);
        self.mandate_report_interval =
            record.map_or(7 * 86_400, |mandate| mandate.report_interval_secs);
        self.mandate_review = record.is_none_or(|mandate| mandate.constraints.review_mutations);
        let values = [
            record
                .map(|mandate| mandate.name.clone())
                .unwrap_or_default(),
            record
                .map(|mandate| mandate.goal.clone())
                .unwrap_or_default(),
            record
                .and_then(|mandate| mandate.kpis.first())
                .map(mandates::kpi_text)
                .unwrap_or_default(),
        ];
        for (field, value) in self.mandate_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.mandate_fields[1].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_mandate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = |index: usize| self.mandate_fields[index].read(cx).value().to_string();
        let Some(draft) = mandates::draft(
            self.mandate_project.as_deref(),
            &text(0),
            &text(1),
            &text(2),
            self.mandate_interval,
            self.mandate_report_interval,
            self.mandate_review,
        ) else {
            self.error = Some(tr("Choose the project the mandate is for.").to_owned());
            cx.notify();
            return;
        };
        let reply = self
            .backend
            .save_mandate(self.mandate_editing.clone(), draft);
        self.change_mandates(reply, window, cx);
    }

    fn send_mandate_reply(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.mandate_fields[3].read(cx).value().to_string();
        self.mandate_fields[3].update(cx, |input, cx| input.set_value("", window, cx));
        self.mandate_replying = None;
        let reply = self.backend.reply_to_mandate(id, text);
        self.change_mandates(reply, window, cx);
    }

    fn mandate_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(tr(text));
        let mut projects = div().h_flex().flex_wrap().gap_1();
        for (index, project) in self.projects.iter().enumerate() {
            let id = project.id.clone();
            projects = projects.child(
                Button::new(("mandate-project", index))
                    .label(project.name.clone())
                    .xsmall()
                    .selected(self.mandate_project.as_deref() == Some(project.id.as_str()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mandate_project = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        let presets = |presets: &'static [(i64, &'static str)],
                       current: i64,
                       id: &'static str,
                       report: bool,
                       cx: &mut Context<Self>| {
            let mut row = div().h_flex().gap_1();
            for (index, (secs, name)) in presets.iter().enumerate() {
                let secs = *secs;
                row = row.child(
                    Button::new((id, index))
                        .label(tr(name))
                        .xsmall()
                        .selected(current == secs)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if report {
                                this.mandate_report_interval = secs;
                            } else {
                                this.mandate_interval = secs;
                            }
                            cx.notify();
                        })),
                );
            }
            row
        };
        let review = self.mandate_review;
        div()
            .v_flex()
            .gap_2()
            .w(px(600.))
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().child(if self.mandate_editing.is_some() {
                tr("Change the mandate")
            } else {
                tr("New mandate")
            }))
            .child(label("Project"))
            .child(projects)
            .child(label(
                "Goal: what the project's agent keeps working towards",
            ))
            .child(
                div()
                    .h(px(96.))
                    .child(super::surfaces::input(&self.mandate_fields[1], cx).h_full()),
            )
            .child(label("Name (optional)"))
            .child(super::surfaces::input(&self.mandate_fields[0], cx).small())
            .child(label(
                "A measure of progress: name, target, period (optional)",
            ))
            .child(super::surfaces::input(&self.mandate_fields[2], cx).small())
            .child(label("Rounds"))
            .child(presets(
                &mandates::CADENCES,
                self.mandate_interval,
                "mandate-cadence",
                false,
                cx,
            ))
            .child(label("Reports"))
            .child(presets(
                &mandates::REPORT_CADENCES,
                self.mandate_report_interval,
                "mandate-report-cadence",
                true,
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        switch(
                            "mandate-review",
                            review,
                            tr("Ask before anything that changes files or runs work"),
                        )
                        .on_change(cx.listener(move |this, _, _, cx| {
                            this.mandate_review = !review;
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("Ask before anything that changes files or runs work")),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("mandate-save")
                            .label(tr("Save"))
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_mandate(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("mandate-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::MandateForm);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// One mandate: where it stands, what it waits on, its latest report
    /// and what can be done with it.
    fn mandate_card(
        &self,
        index: usize,
        overview: &MandateOverview,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let mandate = &overview.mandate;
        let id = mandate.id.clone();
        let (status, tone) = mandates::status(&mandate.status);
        let project = self
            .projects
            .iter()
            .find(|project| project.id == mandate.project_id)
            .map(|project| project.name.clone())
            .unwrap_or_default();
        let mut card = div()
            .v_flex()
            .gap_1()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(super::surfaces::status(status, tone, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(mandate.name.clone()),
                    )
                    .child(chip(project, Tone::Neutral, cx)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(mandate.goal.lines().next().unwrap_or_default().to_owned()),
            );
        for kpi in &mandate.kpis {
            card = card.child(div().text_xs().child(mandates::kpi_line(kpi)));
        }
        if let Some(round) = &overview.last_round {
            card = card.child(div().text_xs().child(fill(
                tr("Round {}: {}"),
                &[&round.seq, &round.done.lines().next().unwrap_or_default()],
            )));
        }
        card = card.child(
            div()
                .text_xs()
                .text_color(muted)
                .child(mandates::next_line(mandate, &Local)),
        );
        if let Some(request) = &overview.request {
            let mut asked = div()
                .v_flex()
                .gap_1()
                .p_2()
                .rounded(cx.theme().radius)
                .bg(super::tone_color(Tone::Attention, cx).opacity(0.08))
                .child(div().text_sm().child(request.what.clone()));
            if !request.why.is_empty() {
                asked = asked.child(div().text_xs().text_color(muted).child(request.why.clone()));
            }
            if self.mandate_replying.as_deref() == Some(id.as_str()) {
                let send = id.clone();
                asked = asked.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(super::surfaces::input(&self.mandate_fields[3], cx).small()),
                        )
                        .child(
                            Button::new(("mandate-send", index))
                                .label(tr("Send"))
                                .small()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.send_mandate_reply(send.clone(), window, cx)
                                })),
                        ),
                );
            } else {
                let open = id.clone();
                asked = asked.child(
                    Button::new(("mandate-answer", index))
                        .label(tr("Answer"))
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.mandate_replying = Some(open.clone());
                            this.mandate_fields[3].update(cx, |input, cx| {
                                input.set_value("", window, cx);
                                input.focus(window, cx);
                            });
                            cx.notify();
                        })),
                );
            }
            card = card.child(asked);
        }
        if let Some(report) = overview.reports.first() {
            let mut shown = div()
                .v_flex()
                .gap_0p5()
                .child(div().text_xs().child(report.body.headline.clone()));
            for item in report
                .body
                .done
                .iter()
                .chain(&report.body.issues)
                .chain(&report.body.next)
                .take(5)
            {
                shown = shown.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("• {}", item.text)),
                );
            }
            card = card.child(shown);
        }
        let closed = mandate.status == "done";
        let paused = mandate.status == "paused";
        let mut actions = div().h_flex().flex_wrap().gap_1().pt_1();
        let button = |name: &'static str, label: &'static str| {
            Button::new((name, index)).label(tr(label)).xsmall().ghost()
        };
        if !closed {
            let run = id.clone();
            actions = actions.child(button("mandate-run", "Run now").on_click(cx.listener(
                move |this, _, window, cx| {
                    let reply = this.backend.run_mandate_now(run.clone());
                    this.change_mandates(reply, window, cx);
                },
            )));
            let toggled = id.clone();
            actions = actions.child(
                button("mandate-pause", if paused { "Resume" } else { "Pause" }).on_click(
                    cx.listener(move |this, _, window, cx| {
                        let status = if paused { "active" } else { "paused" };
                        let reply = this
                            .backend
                            .set_mandate_status(toggled.clone(), status.into());
                        this.change_mandates(reply, window, cx);
                    }),
                ),
            );
        }
        let reported = id.clone();
        actions = actions.child(button("mandate-report", "Report now").on_click(cx.listener(
            move |this, _, window, cx| {
                let reply = this.backend.report_mandate(reported.clone());
                this.change_mandates(reply, window, cx);
            },
        )));
        let edited = overview.clone();
        actions = actions.child(button("mandate-edit", "Edit").on_click(cx.listener(
            move |this, _, window, cx| this.open_mandate_form(Some(&edited), window, cx),
        )));
        let status_id = id.clone();
        actions = actions.child(
            button("mandate-close", if closed { "Reopen" } else { "Close" }).on_click(cx.listener(
                move |this, _, window, cx| {
                    let status = if closed { "active" } else { "done" };
                    let reply = this
                        .backend
                        .set_mandate_status(status_id.clone(), status.into());
                    this.change_mandates(reply, window, cx);
                },
            )),
        );
        if let Some(frame) = mandate.frame_id.clone() {
            let project = mandate.project_id.clone();
            actions = actions.child(button("mandate-open", "Open its conversation").on_click(
                cx.listener(move |this, _, window, cx| {
                    this.layers.close(Layer::Mandates);
                    this.open_session(project.clone(), frame.clone(), window, cx);
                }),
            ));
        }
        let asking = self.layers.contains(Layer::ConfirmDeleteMandate)
            && self.mandate_deleting.as_deref() == Some(id.as_str());
        if asking {
            let deleted = id.clone();
            actions = actions
                .child(div().text_xs().child(tr(
                    "Delete this mandate? Its conversation stays in the project.",
                )))
                .child(
                    Button::new(("mandate-delete-yes", index))
                        .label(tr("Delete"))
                        .xsmall()
                        .danger()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let reply = this.backend.delete_mandate(deleted.clone());
                            this.change_mandates(reply, window, cx);
                        })),
                );
        } else {
            let deleting = id.clone();
            actions = actions.child(button("mandate-delete", "Delete").on_click(cx.listener(
                move |this, _, _, cx| {
                    this.mandate_deleting = Some(deleting.clone());
                    this.layers.open(Layer::ConfirmDeleteMandate);
                    cx.notify();
                },
            )));
        }
        card.child(actions)
    }

    pub(super) fn render_mandates(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
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
                nav_row("mandates-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_mandates(window, cx))),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .child(surfaces::title(tr("Research mandates")))
                    // What the page is for, a tooltip away, not a paragraph in
                    // the rail.
                    .child(surfaces::help(
                        "mandates-help",
                        tr(
                "Standing goals a project's agent works on in rounds, in one conversation of \
                 the project, while the app is open. A mandate asks you when it needs something \
                 only you can give, and reports on a cadence; both reach the research assistant.",
                        ),
                        cx,
                    )),
            )
            .child(
                div().px_1().child(
                    Button::new("mandate-new")
                        .icon(glyph("plus"))
                        .label(tr("New mandate"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_mandate_form(None, window, cx)
                        })),
                ),
            );
        // The page's records in a centred column (`surfaces::page_column`).
        let mut body = surfaces::page_column();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        if self.layers.contains(Layer::MandateForm) {
            body = body.child(self.mandate_form(cx));
        }
        match &self.mandates {
            None if self.error.is_none() => {
                body = body.child(div().text_sm().text_color(muted).child(tr("Loading…")));
            }
            None => {}
            Some(rows) if rows.is_empty() => {
                body = body.child(div().text_sm().text_color(muted).child(tr(
                    "No mandates yet. Give a project a goal its agent keeps working towards.",
                )));
            }
            Some(rows) => {
                for (index, overview) in rows.iter().enumerate() {
                    body = body.child(self.mandate_card(index, overview, cx));
                }
            }
        }
        div()
            .h_flex()
            .size_full()
            .child(side)
            .child(surfaces::page_scroll("mandates-body", body))
    }
}
