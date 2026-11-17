//! The research calendar: a page that shows one month across every
//! project, how much happened on each day, and what happened on the day
//! that is chosen.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local};
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, StyledExt};

use super::{
    Workspace,
    shell::{error_banner, nav_row},
    surfaces,
};
use crate::{
    backend::receive,
    calendar,
    journey::{self, Month},
    layers::Layer,
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// Today's day of the month, when `month` is the present one.
fn today_in(month: Month) -> Option<u32> {
    (Month::containing(now(), &Local) == month).then(|| Local::now().day())
}

impl Workspace {
    /// Opens the page on the present month.
    pub(super) fn open_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Calendar);
        self.error = None;
        self.show_calendar_month(Month::containing(now(), &Local), window, cx);
    }

    pub(super) fn close_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Calendar) {
            self.calendar = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Reads `month` for every project. Only the answer to the latest
    /// request is shown.
    fn show_calendar_month(&mut self, month: Month, window: &mut Window, cx: &mut Context<Self>) {
        self.calendar_month = Some(month);
        self.calendar = None;
        self.calendar_day = None;
        self.calendar_request += 1;
        let request = self.calendar_request;
        let (from, until) = month.range(&Local);
        let reply = self.backend.calendar(from, until);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.calendar_request != request {
                    return;
                }
                match result {
                    Ok(projects) => {
                        let counts =
                            calendar::counts(&projects, &this.calendar_hidden, month, &Local);
                        this.calendar_day = calendar::opening_day(&counts, today_in(month));
                        this.calendar = Some(projects);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn project_name(&self, id: &str) -> String {
        self.projects
            .iter()
            .find(|project| project.id == id)
            .map_or_else(
                || tr("A project").to_owned(),
                |project| project.name.clone(),
            )
    }

    pub(super) fn render_calendar(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let hover = cx.theme().list_hover;
        let today_ink = cx.theme().primary;
        let accent = cx.theme().primary;
        let month = self
            .calendar_month
            .unwrap_or_else(|| Month::containing(now(), &Local));
        let this_month = Month::containing(now(), &Local);
        let today = today_in(month);

        // The rail scrolls when its projects run past the window.
        let mut side = div()
            .id("calendar-rail")
            .v_flex()
            .w(px(260.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                nav_row("calendar-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_calendar(window, cx))),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .child(surfaces::title(tr("Research calendar")))
                    // What the page is for, a tooltip away, not a paragraph in
                    // the rail.
                    .child(surfaces::help(
                        "calendar-help",
                        tr(
                "How much happened in your projects on each day. Choose a day to see what.",
                        ),
                        cx,
                    )),
            )
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .px_1()
                    .items_center()
                    // The month as a fact between its arrows; this month a
                    // click away.
                    .child(
                        surfaces::icon_action(
                            "calendar-previous",
                            "chevron-left",
                            tr("Earlier month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_calendar_month(month.previous(), window, cx)
                        })),
                    )
                    .child(
                        div().flex_1().h_flex().justify_center().child(
                            surfaces::fact(month.label(), cx).text_color(cx.theme().foreground),
                        ),
                    )
                    .child(
                        surfaces::icon_action(
                            "calendar-next",
                            "chevron-right",
                            tr("Later month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(month == this_month)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_calendar_month(month.next(), window, cx)
                        })),
                    )
                    .child(
                        surfaces::icon_action(
                            "calendar-today",
                            "calendar",
                            tr("This month"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(month == this_month)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_calendar_month(this_month, window, cx)
                        })),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Projects")),
            );
        for project in &self.projects {
            let id = project.id.clone();
            let shown = !self.calendar_hidden.contains(&project.id);
            side = side.child(
                // A chip that is chosen while its project is shown, with a
                // check; a chip alone in a column would stretch across it.
                div().h_flex().px_1().child(
                    surfaces::choice_chip(
                        SharedString::from(format!("calendar-project-{id}")),
                        shown,
                        cx,
                    )
                    .when(shown, |chip| chip.child(crate::assets::icon("check")))
                    .child(div().max_w(px(180.)).truncate().child(project.name.clone()))
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(if shown {
                            tr("Shown; press to leave it out")
                        } else {
                            tr("Left out; press to show it")
                        })
                        .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.calendar_hidden.remove(&id) {
                            this.calendar_hidden.insert(id.clone());
                        }
                        cx.notify();
                    })),
                ),
            );
        }

        // The page's records in a centred column (`surfaces::page_column`).
        let mut body = surfaces::page_column();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        let Some(projects) = &self.calendar else {
            return div()
                .h_flex()
                .size_full()
                .child(side)
                .child(surfaces::page_scroll(
                    "calendar-body",
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
        // A project that could not be read is said, never silently empty.
        for project in projects.iter().filter(|project| project.error.is_some()) {
            body = body.child(error_banner(
                &crate::i18n::fill(
                    tr("{} could not be read: {}"),
                    &[
                        &self.project_name(&project.project_id),
                        &project.error.clone().unwrap_or_default(),
                    ],
                ),
                cx,
            ));
        }
        let counts = calendar::counts(projects, &self.calendar_hidden, month, &Local);
        let mut grid = div().v_flex().gap_1().w(px(700.));
        let mut heads = div().h_flex().gap_1();
        for name in calendar::WEEKDAYS {
            heads = heads.child(
                div()
                    .w(px(96.))
                    .px_2()
                    .text_xs()
                    .text_color(muted)
                    // The table has the weekdays in each language.
                    .child(tr(name)),
            );
        }
        grid = grid.child(heads);
        for week in calendar::weeks(month) {
            let mut row = div().h_flex().gap_1();
            for place in week {
                let Some(day) = place else {
                    row = row.child(div().w(px(96.)).h(px(64.)));
                    continue;
                };
                let count = counts[day as usize];
                let chosen = self.calendar_day == Some(day);
                let mut cell = div()
                    .id(("calendar-day", day as usize))
                    .v_flex()
                    .justify_between()
                    .w(px(96.))
                    .h(px(64.))
                    .p_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(if chosen {
                        accent
                    } else if today == Some(day) {
                        accent.opacity(0.4)
                    } else {
                        border
                    })
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(
                        div()
                            .text_sm()
                            // Today is said in teal, not in weight.
                            .when(today == Some(day), |day| day.text_color(today_ink))
                            .child(day.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.calendar_day = Some(day);
                        cx.notify();
                    }));
                if chosen {
                    cell = cell.bg(accent.opacity(0.08));
                }
                if count > 0 {
                    cell = cell.child(div().text_xs().text_color(accent).child(records(count)));
                }
                row = row.child(cell);
            }
            grid = grid.child(row);
        }
        body = body.child(grid);

        let Some(day) = self.calendar_day else {
            return div()
                .h_flex()
                .size_full()
                .child(side)
                .child(surfaces::page_scroll(
                    "calendar-body",
                    body.child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(tr("Nothing is recorded for this month.")),
                    ),
                ));
        };
        let items = calendar::on_day(projects, &self.calendar_hidden, month, day, &Local);
        body = body.child(div().pt_2().text_sm().child(format!(
            "{} · {}",
            if crate::i18n::current() == crate::i18n::Locale::Zh {
                format!("{}{day}日", month.label())
            } else {
                format!("{day} {}", month.label())
            },
            match items.len() {
                0 => tr("nothing recorded").to_owned(),
                n => records(n),
            }
        )));
        // The day's records: when, the kind's icon, what happened, the
        // project faded after it, a run's state as a stamp.
        let mut rows = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let entry = item.entry;
            let mut row = surfaces::record_row(("calendar-entry", index), entry.title.clone())
                .time(item.time.clone())
                .icon(journey::kind_icon(&entry.kind))
                .suffix(self.project_name(item.project));
            if let Some(status) = journey::status_label(entry) {
                row = match surfaces::StampKind::from_tone(journey::status_tone(entry)) {
                    Some(kind) => row.stamp(kind, status),
                    None => row,
                };
            }
            // A record that came from a conversation opens it.
            if let Some(frame) = entry.frame_id.clone() {
                let project = item.project.to_owned();
                row = row
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.layers.close(Layer::Calendar);
                        this.calendar = None;
                        this.open_session(project.clone(), frame.clone(), window, cx);
                    }));
            }
            rows.push(row);
        }
        if !rows.is_empty() {
            body = body.child(surfaces::record_group(rows, cx));
        }
        div()
            .h_flex()
            .size_full()
            .child(side)
            .child(surfaces::page_scroll("calendar-body", body))
    }
}

/// How many records, in words: 1 record, 3 records.
fn records(count: usize) -> String {
    match count {
        1 => tr("1 record").into(),
        n => crate::i18n::fill(tr("{} records"), &[&n]),
    }
}
