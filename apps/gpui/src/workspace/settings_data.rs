//! The Settings pages about what Wisp has used and what it keeps: Usage and
//! Storage. Both only read.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt, button::Button, tooltip::Tooltip};

use super::{Workspace, shell::glyph};
use crate::{
    backend::receive,
    controls::compact,
    files::size_label,
    home,
    usage::{self, DAYS},
};

/// How tall the busiest day's bar is.
const BAR: f32 = 64.;

/// A headline figure with its name under it.
fn figure(value: String, name: &'static str, cx: &App) -> Div {
    div()
        .v_flex()
        .flex_1()
        .p_3()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .child(div().text_xl().font_weight(FontWeight::MEDIUM).child(value))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(name),
        )
}

fn heading(text: &'static str) -> Div {
    div()
        .pt_5()
        .pb_1()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

impl Workspace {
    pub(super) fn load_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.usage();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(usage) => this.usage = Some(usage),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_storage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.storage();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(storage) => this.storage = Some(storage),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_usage_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(view) = &self.usage else {
            return page.child(div().text_sm().text_color(muted).child("Loading…"));
        };
        if view.projects.is_empty() {
            return page.child(div().text_sm().text_color(muted).child(
                "Nothing has been used yet. Tokens are counted from what the model \
                 reports after each call.",
            ));
        }
        let totals = usage::totals(view);
        let mut page = page.child(
            div()
                .h_flex()
                .gap_2()
                .child(figure(compact(totals.input), "Input tokens", cx))
                .child(figure(compact(totals.output), "Output tokens", cx))
                .child(figure(compact(totals.cached), "Read from cache", cx))
                .child(figure(compact(totals.reasoning), "Reasoning", cx)),
        );

        // One series, so one colour and no legend: the heading names it.
        let days = usage::recent(&view.days, DAYS);
        let spent: u64 = days.iter().map(|(_, tokens)| tokens).sum();
        let mut strip = div().h_flex().items_end().gap(px(2.)).h(px(BAR));
        for (index, (height, (date, tokens))) in
            usage::heights(days, BAR).into_iter().zip(days).enumerate()
        {
            let label = format!("{}: {} tokens", usage::day_label(date), compact(*tokens));
            // The whole column answers the pointer, not just the bar in it.
            strip = strip.child(
                div()
                    .id(("usage-day", index))
                    .flex_1()
                    .h_full()
                    .v_flex()
                    .justify_end()
                    .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    .child(
                        div()
                            .w_full()
                            .h(px(height))
                            .rounded_t(px(2.))
                            .bg(cx.theme().primary),
                    ),
            );
        }
        let range = match (days.first(), days.last()) {
            (Some((first, _)), Some((last, _))) => {
                format!("{} to {}", usage::day_label(first), usage::day_label(last))
            }
            _ => String::new(),
        };
        let busiest = usage::busiest(days).map_or_else(
            || "No use in this period.".to_owned(),
            |(date, tokens)| {
                format!(
                    "Busiest: {} with {} tokens.",
                    usage::day_label(date),
                    compact(*tokens)
                )
            },
        );
        page = page
            .child(heading("Input and output per day"))
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{range}: {} tokens. {busiest}", compact(spent))),
            )
            .child(strip)
            .child(div().h(px(1.)).bg(cx.theme().border));

        let all: u64 = view.models.iter().map(|(_, tokens)| tokens).sum();
        page = page.child(heading("By model"));
        for (model, tokens) in &view.models {
            let share = usage::percent(*tokens, all);
            page = page.child(
                div()
                    .v_flex()
                    .gap_1()
                    .py_2()
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .text_sm()
                            .child(div().flex_1().min_w_0().truncate().child(model.clone()))
                            .child(
                                div()
                                    .text_color(muted)
                                    .child(format!("{} · {share}%", compact(*tokens))),
                            ),
                    )
                    .child(
                        div().h(px(4.)).rounded(px(2.)).bg(cx.theme().muted).child(
                            div()
                                .h_full()
                                .w(relative(share as f32 / 100.))
                                .rounded(px(2.))
                                .bg(cx.theme().primary),
                        ),
                    ),
            );
        }

        page = page.child(heading("By project"));
        for row in &view.projects {
            page = page.child(
                div()
                    .h_flex()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(row.name.clone()))
                            .child(div().text_xs().text_color(muted).child(
                                match row.conversations {
                                    1 => "1 conversation".to_owned(),
                                    n => format!("{n} conversations"),
                                },
                            )),
                    )
                    .child(div().text_color(muted).child(format!(
                        "Input {} · output {}",
                        compact(row.input),
                        compact(row.output)
                    ))),
            );
        }
        page
    }

    pub(super) fn render_storage_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(view) = &self.storage else {
            return page.child(div().text_sm().text_color(muted).child("Loading…"));
        };
        let folder = std::path::PathBuf::from(&view.data_dir);
        let line = |name: &'static str, note: &'static str, bytes: u64| {
            div()
                .h_flex()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(name))
                        .child(div().text_xs().text_color(muted).child(note)),
                )
                .child(div().text_sm().child(size_label(bytes)))
        };
        let mut page = page
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Data folder"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(view.data_dir.clone()),
                            ),
                    )
                    .child(
                        Button::new("storage-reveal")
                            .icon(glyph("folder"))
                            .label("Show")
                            .small()
                            .flex_shrink_0()
                            .tooltip("Show the folder in your file manager")
                            .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&folder))),
                    ),
            )
            .child(div().pt_3())
            .child(line(
                "Database",
                "Conversations, settings and run records of every project.",
                view.database_bytes,
            ))
            .child(line(
                "Favorites",
                "Copies of what you saved, apart from the conversations.",
                view.favorites_bytes,
            ))
            .child(heading("In each project's folder"))
            .child(div().pb_1().text_xs().text_color(muted).child(
                "What Wisp itself keeps there: its .wisp folder (memory, snapshots) and the \
                 files you attached from elsewhere. Your own files are not counted.",
            ));
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());
        for row in &view.projects {
            let detail = if row.missing {
                "The folder is not there.".to_owned()
            } else {
                format!(
                    ".wisp {} · uploads {}",
                    size_label(row.wisp_bytes),
                    size_label(row.uploads_bytes)
                )
            };
            page = page.child(
                div()
                    .h_flex()
                    .gap_6()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_sm().child(row.name.clone()))
                            .child(div().truncate().text_xs().text_color(muted).child(format!(
                                "{} · {}",
                                home::short_path(&row.workspace_dir, home_dir.as_deref()),
                                match row.conversations {
                                    1 => "1 conversation".to_owned(),
                                    n => format!("{n} conversations"),
                                }
                            ))),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .when(row.missing, |text| text.text_color(cx.theme().warning))
                            .child(detail),
                    ),
            );
        }
        page
    }
}
