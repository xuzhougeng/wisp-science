//! Scheduled tasks: a page of prompts that are sent into a new
//! conversation of a project on a cadence, while the app is open.
use crate::i18n::tr;
use chrono::Local;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{ScheduleInput, ScheduleRow};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, nav_row},
    surfaces::{self, switch},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    present::Tone,
    schedules::{self, Repeat},
};

impl Workspace {
    pub(super) fn open_schedules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Schedules);
        self.error = None;
        self.schedule_note = None;
        self.load_schedules(window, cx);
        self.load_daily_recap(window, cx);
    }

    fn load_daily_recap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.daily_recap();
        self.show_daily_recap(reply, true, window, cx);
    }

    /// Shows how the daily recap run stands once `reply` comes; `fill`
    /// puts its time in the field.
    fn show_daily_recap(
        &mut self,
        reply: Reply<wisp_dto::DailyRecapAutomation>,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(daily) => {
                        if fill {
                            let time = daily.time.clone();
                            this.recap_time
                                .update(cx, |input, cx| input.set_value(time, window, cx));
                        }
                        // While it drafts, look again shortly.
                        if daily.running && this.layers.contains(Layer::Schedules) {
                            cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_secs(3))
                                    .await;
                                let _ = this.update_in(cx, |this, window, cx| {
                                    let reply = this.backend.daily_recap();
                                    this.show_daily_recap(reply, false, window, cx);
                                });
                            })
                            .detach();
                        }
                        this.daily_recap = Some(daily);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves the daily run's switch and the time in the field.
    fn save_daily_recap(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        let time = self.recap_time.read(cx).value().to_string();
        self.error = None;
        let reply = self.backend.set_daily_recap(enabled, time);
        self.show_daily_recap(reply, true, window, cx);
    }

    /// The built-in daily recap: on or off, when, and how it last went.
    fn daily_recap_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut card = div()
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
                    .child(tr("Daily recap")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Each day after this time, a recap is drafted from the records of each \
                 project's last three days that have none yet. Drafts wait on the project's \
                 research journey for you to confirm or dismiss.",
            )));
        let Some(daily) = &self.daily_recap else {
            return card.child(div().text_xs().text_color(muted).child(tr("Loading…")));
        };
        let enabled = daily.enabled;
        let running = daily.running;
        card = card
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        switch("daily-recap-enabled", enabled, tr("Daily recap")).on_change(
                            cx.listener(move |this, _, window, cx| {
                                this.save_daily_recap(!enabled, window, cx)
                            }),
                        ),
                    )
                    .child(div().w(px(80.)).child(Input::new(&self.recap_time).small()))
                    .child(
                        Button::new("daily-recap-time")
                            .label(tr("Save"))
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_daily_recap(enabled, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("daily-recap-now")
                            .label(tr("Run now"))
                            .small()
                            .ghost()
                            .disabled(running)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let reply = this.backend.run_daily_recap_now();
                                this.show_daily_recap(reply, false, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if daily.error.is_some() && !running {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .child(schedules::recap_line(daily, &Local)),
            );
        card
    }

    pub(super) fn close_schedules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Schedules) {
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn load_schedules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.schedules();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(rows) => this.schedules = Some(rows),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change, then reads the list again. `note` is said when
    /// it worked; a refusal keeps the form open.
    fn change_schedules<T: Send + 'static>(
        &mut self,
        reply: Reply<T>,
        note: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.schedule_note = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.layers.close(Layer::ScheduleForm);
                        this.layers.close(Layer::ConfirmDeleteSchedule);
                        this.schedule_deleting = None;
                        this.schedule_note = note.map(|note| tr(note).to_owned());
                        this.load_schedules(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form on the open project, or the first, with `prompt`.
    fn open_schedule_form(
        &mut self,
        name: &str,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::ConfirmDeleteSchedule);
        self.schedule_deleting = None;
        self.layers.open(Layer::ScheduleForm);
        self.error = None;
        self.schedule_note = None;
        if self.schedule_project.is_none() {
            self.schedule_project = self
                .state
                .project
                .clone()
                .or_else(|| self.projects.first().map(|project| project.id.clone()));
        }
        let values = [name, prompt, "6", "09:00"];
        for (field, value) in self.schedule_fields.iter().zip(values) {
            field.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        }
        self.schedule_fields[1].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [name, prompt, hours, time] = self
            .schedule_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let Some(project) = self.schedule_project.clone() else {
            self.error = Some(tr("Choose a project and write a prompt.").to_owned());
            cx.notify();
            return;
        };
        let (interval_secs, start_at) = match schedules::cadence(
            self.schedule_repeat,
            &hours,
            &time,
            self.schedule_weekday,
            &Local::now(),
        ) {
            Ok(cadence) => cadence,
            Err(problem) => {
                self.error = Some(problem.to_owned());
                cx.notify();
                return;
            }
        };
        let reply = self.backend.create_schedule(ScheduleInput {
            project,
            name,
            prompt,
            interval_secs,
            start_at,
            session: None,
        });
        self.change_schedules(reply, None, window, cx);
    }

    fn schedule_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut projects = div().h_flex().flex_wrap().gap_1();
        for project in &self.projects {
            let id = project.id.clone();
            projects = projects.child(
                Button::new(SharedString::from(format!(
                    "schedule-project-{}",
                    project.id
                )))
                .label(project.name.clone())
                .small()
                .selected(self.schedule_project.as_deref() == Some(project.id.as_str()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.schedule_project = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        let mut repeats = div().h_flex().gap_1();
        for repeat in Repeat::ALL {
            repeats = repeats.child(
                Button::new(SharedString::from(format!("schedule-repeat-{repeat:?}")))
                    .label(repeat.label())
                    .small()
                    .selected(self.schedule_repeat == repeat)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.schedule_repeat = repeat;
                        cx.notify();
                    })),
            );
        }
        let mut when = div().h_flex().items_center().gap_2();
        match self.schedule_repeat {
            Repeat::Hours => {
                when = when.child(label(tr("Every (hours)"))).child(
                    div()
                        .w(px(70.))
                        .child(Input::new(&self.schedule_fields[2]).small()),
                );
            }
            Repeat::Daily | Repeat::Weekly => {
                if self.schedule_repeat == Repeat::Weekly {
                    let mut days = div().h_flex().gap_1();
                    for (index, day) in schedules::WEEKDAYS.iter().enumerate() {
                        let index = index as u32;
                        days = days.child(
                            Button::new(SharedString::from(format!("schedule-day-{index}")))
                                .label(tr(day))
                                .xsmall()
                                .selected(self.schedule_weekday == index)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.schedule_weekday = index;
                                    cx.notify();
                                })),
                        );
                    }
                    when = when.child(days);
                }
                when = when.child(label(tr("Time (HH:MM)"))).child(
                    div()
                        .w(px(80.))
                        .child(Input::new(&self.schedule_fields[3]).small()),
                );
            }
        }
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(640.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("New scheduled task")),
            )
            .child(label(tr("Project")))
            .child(projects)
            .child(label(tr("Prompt")))
            .child(
                div()
                    .h(px(140.))
                    .child(Input::new(&self.schedule_fields[1]).h_full()),
            )
            .child(label(tr("Name. Empty takes the prompt's first line")))
            .child(Input::new(&self.schedule_fields[0]).small())
            .child(label(tr("Repeat")))
            .child(repeats)
            .child(when)
            .child(div().text_xs().text_color(muted).child(tr(
                "When due, a new conversation is started in the project and this prompt is \
                 sent. Tasks run only while the app is open; a missed time runs once the \
                 next time it is.",
            )))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("schedule-save")
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_schedule(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("schedule-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ScheduleForm);
                                cx.notify();
                            })),
                    ),
            )
    }

    fn schedule_row(&self, row: &ScheduleRow, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let id = row.id.clone();
        let asking = self.layers.contains(Layer::ConfirmDeleteSchedule)
            && self.schedule_deleting.as_deref() == Some(id.as_str());
        let last = row.runs.first();
        let mut history = div()
            .id(SharedString::from(format!("schedule-last-{id}")))
            .text_xs()
            .text_color(if last.is_some_and(|run| run.error.is_some()) {
                cx.theme().danger
            } else {
                muted
            })
            .child(schedules::last_label(last, &Local));
        // The latest firing opens the conversation it was sent into.
        if let Some(session) = last.and_then(|run| run.session.clone()) {
            let project = row.project.clone();
            history = history
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.layers.close(Layer::Schedules);
                    this.open_session(project.clone(), session.clone(), window, cx);
                }));
        }
        let words = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(row.name.clone()),
                    )
                    .child(chip(row.project_name.clone(), Tone::Neutral, cx))
                    .children(
                        row.session
                            .is_some()
                            .then(|| chip(tr("A conversation's timer"), Tone::Active, cx)),
                    ),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(row.prompt.lines().next().unwrap_or_default().to_owned()),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(chip(
                        schedules::cadence_label(row.interval_secs),
                        Tone::Active,
                        cx,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(schedules::next_label(row, &Local)),
                    )
                    .child(history),
            );
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("schedule-delete-yes-{id}")))
                        .label(tr("Delete"))
                        .small()
                        .danger()
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, window, cx| {
                                let reply = this.backend.delete_schedule(id.clone());
                                this.change_schedules(reply, None, window, cx)
                            }
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("schedule-delete-no-{id}")))
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmDeleteSchedule);
                            this.schedule_deleting = None;
                            cx.notify();
                        })),
                );
        } else {
            let enabled = row.enabled;
            let (run, toggle, delete) = (id.clone(), id.clone(), id.clone());
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("schedule-run-{id}")))
                        .label(tr("Run now"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let reply = this.backend.run_schedule_now(run.clone());
                            this.change_schedules(
                                reply,
                                Some("Started. The result appears in a new conversation of that project."),
                                window,
                                cx,
                            )
                        })),
                )
                .child(
                    switch(
                        SharedString::from(format!("schedule-toggle-{id}")),
                        enabled,
                        tr("Run this task on its schedule"),
                    )
                    .on_change(cx.listener(move |this, _, window, cx| {
                        let reply = this.backend.set_schedule_enabled(toggle.clone(), !enabled);
                        this.change_schedules(reply, None, window, cx)
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!("schedule-delete-{id}")))
                        .label(tr("Delete"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layers.close(Layer::ScheduleForm);
                            this.schedule_deleting = Some(delete.clone());
                            this.layers.open(Layer::ConfirmDeleteSchedule);
                            cx.notify();
                        })),
                );
        }
        div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(words)
            .child(actions)
    }

    pub(super) fn render_schedules(&self, cx: &mut Context<Self>) -> Div {
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
                nav_row("schedules-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_schedules(window, cx))),
            )
            .child(surfaces::title(tr("Scheduled tasks")).px_2().pt_3())
            .child(div().px_2().pb_2().text_xs().text_color(muted).child(tr(
                "Prompts sent on a schedule while the app is open, or any time you need \
                 them. Each firing is a conversation of its own in the task's project.",
            )))
            .child(
                div().px_1().child(
                    Button::new("schedule-new")
                        .icon(glyph("plus"))
                        .label(tr("New scheduled task"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_schedule_form("", "", window, cx)
                        })),
                ),
            );

        let mut body = div()
            .id("schedules-body")
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
        if let Some(note) = &self.schedule_note {
            body = body.child(div().text_xs().text_color(muted).child(note.clone()));
        }
        if self.layers.contains(Layer::ScheduleForm) {
            body = body.child(self.schedule_form(cx));
        }
        body = body.child(self.daily_recap_card(cx));
        let Some(rows) = &self.schedules else {
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
        if rows.is_empty() {
            body = body.child(div().text_sm().text_color(muted).child(tr(
                "No scheduled tasks yet. Start from a template below or write your own prompt.",
            )));
        }
        for row in rows {
            body = body.child(self.schedule_row(row, cx));
        }
        let mut templates = div().h_flex().flex_wrap().gap_2();
        for (index, (name, prompt)) in schedules::TEMPLATES.iter().enumerate() {
            templates = templates.child(
                Button::new(("schedule-template", index))
                    .label(tr(name))
                    .small()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_schedule_form(tr(name), prompt, window, cx)
                    })),
            );
        }
        body = body
            .child(
                div()
                    .pt_3()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Templates")),
            )
            .child(templates);
        div().h_flex().size_full().child(side).child(body)
    }
}
