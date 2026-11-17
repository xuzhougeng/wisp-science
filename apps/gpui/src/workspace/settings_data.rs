//! The Settings pages about what Wisp has used and what it keeps: Usage and
//! Storage. Both only read.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt, tooltip::Tooltip};

use super::{Workspace, settings_sections::setting, surfaces};
use crate::{
    backend::receive,
    controls::compact,
    files::size_label,
    home,
    usage::{self, DAYS},
};

/// How tall the busiest day's bar is.
const BAR: f32 = 64.;

/// A headline figure on the paper: the count as a fact in ink, its name
/// under it in faded caption. Only read, so no ground of its own.
fn figure(value: String, name: &'static str, cx: &App) -> Div {
    div()
        .v_flex()
        .flex_1()
        .gap_0p5()
        .py_2()
        .child(surfaces::read_only(value, cx))
        .child(surfaces::settings_note(name, cx))
}

/// A heading over a group of the page: faded caption.
fn heading(text: &'static str, cx: &App) -> Div {
    surfaces::settings_heading(text, cx)
}

/// A thin meter under a row: the share of the whole it stands for.
fn meter(share: f32, cx: &App) -> Div {
    div().h(px(4.)).rounded(px(2.)).bg(cx.theme().muted).child(
        div()
            .h_full()
            .w(relative(share.clamp(0., 1.)))
            .rounded(px(2.))
            .bg(cx.theme().primary),
    )
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
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        if view.projects.is_empty() {
            return page.child(div().text_sm().text_color(muted).child(tr(
                "Nothing has been used yet. Tokens are counted from what the model \
                 reports after each call.",
            )));
        }
        let totals = usage::totals(view);
        let mut page = page.child(
            div()
                .h_flex()
                .gap_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(figure(compact(totals.input), tr("Input tokens"), cx))
                .child(figure(compact(totals.output), tr("Output tokens"), cx))
                .child(figure(compact(totals.cached), tr("Read from cache"), cx))
                .child(figure(compact(totals.reasoning), tr("Reasoning"), cx)),
        );

        // One series, so one colour and no legend: the heading names it.
        let days = usage::recent(&view.days, DAYS);
        let spent: u64 = days.iter().map(|(_, tokens)| tokens).sum();
        let mut strip = div().h_flex().items_end().gap(px(2.)).h(px(BAR));
        for (index, (height, (date, tokens))) in
            usage::heights(days, BAR).into_iter().zip(days).enumerate()
        {
            let label = crate::i18n::fill(
                tr("{}: {} tokens"),
                &[&usage::day_label(date), &compact(*tokens)],
            );
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
            (Some((first, _)), Some((last, _))) => crate::i18n::fill(
                tr("{} to {}"),
                &[&usage::day_label(first), &usage::day_label(last)],
            ),
            _ => String::new(),
        };
        let busiest = usage::busiest(days).map_or_else(
            || tr("No use in this period.").to_owned(),
            |(date, tokens)| {
                crate::i18n::fill(
                    tr("Busiest: {} with {} tokens."),
                    &[&usage::day_label(date), &compact(*tokens)],
                )
            },
        );
        page = page
            .child(heading(tr("Input and output per day"), cx))
            .child(
                surfaces::settings_note(
                    crate::i18n::fill(
                        tr("{}: {} tokens. {}"),
                        &[&range, &compact(spent), &busiest],
                    ),
                    cx,
                )
                .pb_2(),
            )
            .child(strip)
            .child(div().h(px(1.)).bg(cx.theme().border));

        let all: u64 = view.models.iter().map(|(_, tokens)| tokens).sum();
        page = page.child(heading(tr("By model"), cx));
        let mut rows = Vec::new();
        for (index, (model, tokens)) in view.models.iter().enumerate() {
            let share = usage::percent(*tokens, all);
            rows.push(
                surfaces::record_row(("usage-model", index), model.clone())
                    .icon("bot")
                    .mono()
                    .suffix(format!("{share}%"))
                    .value(compact(*tokens))
                    .below(Some(meter(share as f32 / 100., cx).into_any_element())),
            );
        }
        if !rows.is_empty() {
            page = page.child(surfaces::record_group(rows, cx));
        }

        // What the model reached for: skills and MCP tools, by calls.
        let tools = usage::tool_rows(view);
        page = page.child(heading(tr("SKILL / MCP calls"), cx));
        if tools.is_empty() {
            page = page.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("No SKILL or MCP calls recorded yet.")),
            );
        }
        let most = tools.first().map_or(1, |(_, _, calls)| (*calls).max(1));
        let mut rows = Vec::new();
        for (index, (kind, name, calls)) in tools.iter().enumerate() {
            let (icon, kind) = match kind.as_str() {
                "skill" => ("skill", tr("Skill")),
                "mcp" => ("link", "MCP"),
                _ => ("wrench", tr("Other")),
            };
            rows.push(
                surfaces::record_row(("usage-tool", index), name.clone())
                    .icon(icon)
                    .lead(kind)
                    .value(calls.to_string())
                    .below(Some(
                        meter(*calls as f32 / most as f32, cx).into_any_element(),
                    )),
            );
        }
        if !rows.is_empty() {
            page = page.child(surfaces::record_group(rows, cx));
        }

        page = page.child(heading(tr("By project"), cx));
        let rows = view.projects.iter().enumerate().map(|(index, row)| {
            surfaces::record_row(("usage-project", index), row.name.clone())
                .icon("notebook")
                .suffix(crate::organise::conversations(row.conversations as usize))
                .action(surfaces::fact(
                    crate::i18n::fill(
                        tr("Input {} · output {}"),
                        &[&compact(row.input), &compact(row.output)],
                    ),
                    cx,
                ))
        });
        page = page.child(surfaces::record_group(rows, cx));
        page
    }

    pub(super) fn render_storage_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(view) = &self.storage else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let folder = std::path::PathBuf::from(&view.data_dir);
        // Each part of the data folder is a setting's row: what it is on the
        // left, its size as a fact on the right.
        let line = |name: &'static str, note: &'static str, bytes: u64| {
            setting(name, note, surfaces::read_only(size_label(bytes), cx), cx)
        };
        let mut page = page
            .child(setting(
                tr("Data folder"),
                "",
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(surfaces::read_only(view.data_dir.clone(), cx))
                    .child(
                        surfaces::outlined_action("storage-reveal", "folder", tr("Show"), cx)
                            .flex_shrink_0()
                            .tooltip(tr("Show the folder in your file manager"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&folder))),
                    ),
                cx,
            ))
            .child(line(
                tr("Database"),
                tr("Conversations, settings and run records of every project."),
                view.database_bytes,
            ))
            .child(line(
                tr("Favorites"),
                tr("Copies of what you saved, apart from the conversations."),
                view.favorites_bytes,
            ))
            .child(line(
                tr("Python, R and Node.js"),
                tr("What Settings, Environments installed into the data folder."),
                view.tools_bytes,
            ))
            .child(line(
                tr("Plugins"),
                tr("Installed plugins, every version kept."),
                view.plugins_bytes,
            ))
            .child(line(
                tr("Other"),
                tr("Everything else in the data folder."),
                view.other_bytes(),
            ))
            .child(line(
                tr("Total"),
                tr("The whole data folder."),
                view.data_bytes,
            ))
            .child(heading(tr("In each project's folder"), cx))
            .child(surfaces::settings_note_help(tr("Each project's folder, and what Wisp keeps in it."),
                tr(
                    "The whole folder, your own files included, and of it what Wisp itself keeps \
                     there: its .wisp folder (memory, snapshots) and the files you attached from \
                     elsewhere. A very large folder is counted up to 50,000 entries.",
                ),
                cx,
            )
            .pb_1());
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());
        for row in &view.projects {
            let detail = if row.missing {
                tr("The folder is not there.").to_owned()
            } else {
                let whole = if row.workspace_more {
                    crate::i18n::fill(tr("over {}"), &[&size_label(row.workspace_bytes)])
                } else {
                    size_label(row.workspace_bytes)
                };
                crate::i18n::fill(
                    tr("{} · .wisp {} · uploads {}"),
                    &[
                        &whole,
                        &size_label(row.wisp_bytes),
                        &size_label(row.uploads_bytes),
                    ],
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
                            .child(
                                surfaces::fact(
                                    format!(
                                        "{} · {}",
                                        home::short_path(&row.workspace_dir, home_dir.as_deref()),
                                        crate::organise::conversations(row.conversations as usize)
                                    ),
                                    cx,
                                )
                                .truncate(),
                            ),
                    )
                    .child(
                        surfaces::fact(detail, cx)
                            .flex_shrink_0()
                            .when(row.missing, |text| text.text_color(cx.theme().warning)),
                    ),
            );
        }
        page
    }
}
