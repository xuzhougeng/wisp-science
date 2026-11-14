//! The project screen: sidebar, top bar, transcript rows, approvals and the
//! composer. State changes live in the parent module; this file only draws.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use serde_json::json;
use wisp_server::desktop::{RUNS_LOCALLY, Remember};

use super::{Workspace, chip, tone_color};
use crate::{
    assets::{icon, icon_path},
    compose::{self, Chip},
    controls::{self, Question},
    layers::Layer,
    panel::PanelTab,
    present::{self, Tone},
    settings,
};

pub(super) fn glyph(name: &str) -> Icon {
    Icon::empty().path(icon_path(name))
}

/// A quiet, icon-only control. The tooltip is its name.
pub(super) fn icon_button(id: impl Into<ElementId>, name: &str, tooltip: &'static str) -> Button {
    Button::new(id)
        .icon(glyph(name))
        .ghost()
        .small()
        // Tooltips are written in English where the button is made.
        .tooltip(tr(tooltip))
}

/// One line of sidebar navigation. The caller attaches the click handler only
/// when `enabled`, so a dimmed row cannot be activated.
pub(super) fn nav_row(
    id: impl Into<ElementId>,
    name: &str,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let accent = theme.sidebar_accent;
    div()
        .id(id)
        .h_flex()
        .gap_2()
        .px_2()
        .h(px(32.))
        .flex_shrink_0()
        .rounded(theme.radius)
        .text_sm()
        .when(selected, |row| row.bg(accent))
        .when(enabled, |row| {
            row.cursor_pointer()
                .hover(|style| style.bg(accent.opacity(0.7)))
        })
        .when(!enabled, |row| row.opacity(0.5))
        .child(icon(name).text_color(theme.muted_foreground))
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
}

/// A soft banner for something that went wrong.
pub(super) fn error_banner(message: &str, cx: &App) -> Div {
    let danger = cx.theme().danger;
    div()
        .h_flex()
        .items_start()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(cx.theme().radius)
        .bg(danger.opacity(0.08))
        .text_sm()
        .text_color(danger)
        .child(icon("circle-alert").mt(px(2.)).text_color(danger))
        .child(div().flex_1().min_w_0().child(message.to_owned()))
}

impl Workspace {
    /// Shown until the embedded host is ready, or when it could not start.
    pub(super) fn render_startup(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut body = div()
            .v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8();
        if self.pending {
            return body.child(div().text_color(muted).child(tr("Starting Wisp…")));
        }
        body = body
            .child(div().text_lg().child(tr("Wisp could not start")))
            .child(div().max_w(px(520.)).text_sm().text_color(muted).child(tr(
                "Another Wisp desktop may already be using this data directory. \
                         See docs/gpui-desktop.md for the data and workspace locations.",
            )));
        if let Some(error) = &self.error {
            body = body.child(div().max_w(px(520.)).child(error_banner(error, cx)));
        }
        body.child(
            Button::new("retry")
                .label(tr("Try again"))
                .primary()
                .on_click(cx.listener(|this, _, window, cx| this.initialize(window, cx))),
        )
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let disabled = self.pending;
        let has_project = self.state.project.is_some();
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().sidebar_accent;
        let radius = cx.theme().radius;
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_else(|| tr("Project").into());

        let mut new_chat =
            nav_row(
                "new-chat",
                "plus",
                tr("New conversation"),
                false,
                !disabled && has_project,
                cx,
            )
            .child(div().text_xs().text_color(muted).child(
                if cfg!(target_os = "macos") {
                    "⌘N"
                } else {
                    "Ctrl+N"
                },
            ));
        if !disabled && has_project {
            new_chat = new_chat
                .on_click(cx.listener(|this, _, window, cx| this.create_session(window, cx)));
        }
        let mut files = nav_row(
            "files",
            "folder",
            tr("Files"),
            self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Files,
            has_project,
            cx,
        );
        if has_project {
            files = files.on_click(cx.listener(|this, _, window, cx| {
                // With the panel on another tab, this is a way to Files.
                if this.layers.contains(Layer::FilesPanel) && this.panel_tab != PanelTab::Files {
                    this.show_panel_tab(PanelTab::Files, window, cx)
                } else {
                    this.toggle_files(window, cx)
                }
            }));
        }

        let memos_open =
            self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Memos;
        let mut memos = nav_row(
            "memos",
            "note-plus",
            tr("Memos"),
            memos_open,
            has_project,
            cx,
        );
        if has_project {
            memos = memos.on_click(cx.listener(|this, _, window, cx| {
                if this.layers.contains(Layer::FilesPanel) && this.panel_tab == PanelTab::Memos {
                    this.toggle_files(window, cx)
                } else {
                    this.show_panel_tab(PanelTab::Memos, window, cx)
                }
            }));
        }

        let mut sidebar = div()
            .v_flex()
            .w(px(256.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .h(px(36.))
                    .flex_shrink_0()
                    .child(
                        icon_button("home", "arrow-left", tr("All projects"))
                            .on_click(cx.listener(|this, _, window, cx| this.go_home(window, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(project_name),
                    ),
            )
            .child(new_chat)
            .child(files)
            .child(memos)
            .child(
                nav_row(
                    "capabilities",
                    "sparkles",
                    tr("Capabilities"),
                    false,
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_capabilities(window, cx))),
            )
            .child(
                nav_row("journey", "timeline", tr("Journey"), false, has_project, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_journey(window, cx))),
            )
            .child(
                nav_row("papers", "doc", tr("Papers"), false, has_project, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_papers(window, cx))),
            )
            .child(
                nav_row("favorites", "star", tr("Favorites"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_favorites(window, cx))),
            )
            .child(
                div().py_1().child(
                    Input::new(&self.search)
                        .small()
                        .prefix(icon("search").text_color(muted)),
                ),
            );

        // The conversation on screen is being attended to already.
        let waiting: Vec<_> = self
            .inbox
            .iter()
            .filter(|row| Some(&row.id) != self.state.session.as_ref())
            .collect();
        if !waiting.is_empty() {
            let attention = tone_color(Tone::Attention, cx);
            let mut inbox = div()
                .id("inbox")
                .v_flex()
                .flex_shrink_0()
                .max_h(px(150.))
                .overflow_y_scroll();
            for row in waiting {
                let project = row.project_id.clone();
                let session = row.id.clone();
                let mut line = div()
                    .id(SharedString::from(format!("inbox-{session}")))
                    .v_flex()
                    .px_2()
                    .py_1()
                    .rounded(radius)
                    .child(div().truncate().text_sm().child(row.title.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(row.project_name.clone()),
                    );
                if !disabled {
                    line = line
                        .cursor_pointer()
                        .hover(|style| style.bg(accent.opacity(0.7)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_session(project.clone(), session.clone(), window, cx)
                        }));
                }
                inbox = inbox.child(line);
            }
            sidebar = sidebar
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .px_2()
                        .pt_2()
                        .text_xs()
                        .text_color(attention)
                        .child(icon("bell").text_color(attention))
                        .child(tr("Needs you")),
                )
                .child(inbox);
        }

        sidebar = sidebar
            .child(self.render_list_heading(cx))
            .child(self.render_session_list(cx));
        if let Some(bar) = self.render_batch_bar(cx) {
            sidebar = sidebar.child(bar);
        }

        sidebar.child(
            nav_row("settings", "gear", tr("Settings"), false, true, cx)
                .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
        )
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> Div {
        let title = self
            .current_row()
            .map(|row| row.title.clone())
            .unwrap_or_else(|| tr("New research").into());
        let mut heading = div().v_flex().flex_1().min_w_0().child(
            div()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        );
        if let Some(snapshot) = &self.state.snapshot {
            heading = heading.child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(snapshot.model_id.clone()),
            );
        }
        div()
            .h_flex()
            .gap_3()
            .px_4()
            .h(px(52.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(heading)
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        icon_button("palette", "search", tr("Search everything")).on_click(
                            cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)),
                        ),
                    )
                    .child({
                        let open = self.layers.contains(Layer::Outline);
                        let button = div().relative().child(
                            icon_button("outline", "list", tr("Outline: what you asked, in order"))
                                .selected(open)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.layers.close(Layer::Outline) {
                                        this.focus_composer(window, cx);
                                    } else {
                                        this.layers.open(Layer::Outline);
                                    }
                                    cx.notify();
                                })),
                        );
                        if open {
                            // Painted last, over the transcript below.
                            button.child(deferred(self.outline_menu(cx)))
                        } else {
                            button
                        }
                    })
                    .child(
                        icon_button("refresh", "refresh", tr("Refresh"))
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.error = None;
                                this.refresh_lists(window, cx);
                                this.start_poll(window, cx);
                                this.reload_files(window, cx);
                            })),
                    )
                    .child(
                        icon_button("files-toggle", "panel", tr("Files"))
                            .selected(self.layers.contains(Layer::FilesPanel))
                            .disabled(self.state.project.is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_files(window, cx)),
                            ),
                    ),
            )
    }

    /// The small actions under a message.
    fn message_actions(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let copied = self.copied == Some(index);
        let actions = div()
            .h_flex()
            .child(
                icon_button(
                    ("copy-row", index),
                    if copied { "check" } else { "copy" },
                    if copied { tr("Copied") } else { tr("Copy") },
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, window, cx| this.copy_row(index, window, cx))),
            )
            .child(
                icon_button(("remember-row", index), "memory", tr("Save to memory"))
                    .xsmall()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.remember_row(index, window, cx)
                        }),
                    ),
            );
        let starred = self
            .rows
            .get(index)
            .is_some_and(|row| crate::favorites::saved_id(&self.saved, &row.text).is_some());
        let actions = actions.child(
            icon_button(
                ("star-row", index),
                if starred { "star-filled" } else { "star" },
                if starred {
                    tr("In your favorites. Click to remove")
                } else {
                    tr("Save to favorites")
                },
            )
            .xsmall()
            .on_click(cx.listener(move |this, _, window, cx| this.star_row(index, window, cx))),
        );
        // Rewinding and branching work on saved turns of a conversation
        // that is at rest and still part of everyday work.
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let archived = self.current_row().is_none_or(|row| row.shelved);
        let idle = !running && !self.pending;
        match self.rows.get(index).map(|row| row.role.as_str()) {
            Some("user") if !archived => actions.child(
                icon_button(("edit-row", index), "edit", tr("Edit and send again"))
                    .xsmall()
                    .disabled(!idle)
                    .on_click(cx.listener(move |this, _, _, cx| this.ask_rewind(index, cx))),
            ),
            Some("assistant") if controls::turn_of(&self.rows, index).is_some() => actions
                .child(
                    icon_button(("branch-row", index), "branch", tr("Branch from here"))
                        .xsmall()
                        .disabled(!idle)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.branch_from(index, window, cx)
                        })),
                )
                .child(
                    icon_button(("review-row", index), "review", tr("Review this turn"))
                        .xsmall()
                        .disabled(!idle)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_review_of(index, window, cx)
                        })),
                ),
            _ => actions,
        }
    }

    /// A question the model asked. While it waits, each option is one click.
    fn question_card(
        &self,
        index: usize,
        question: Question,
        id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let waiting = controls::awaiting_answer(&self.rows, index, running);
        let info = tone_color(Tone::Active, cx);
        let muted = cx.theme().muted_foreground;
        let mut card = div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(info.opacity(0.4))
            .bg(info.opacity(0.05))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(icon("circle-help").text_color(info))
                    .child(div().font_weight(FontWeight::MEDIUM).child(if waiting {
                        tr("Wisp needs your decision")
                    } else {
                        tr("Wisp asked")
                    })),
            )
            .child(TextView::markdown(id, question.question, window, cx).selectable(true));
        if waiting {
            for (n, (label, description)) in question.options.into_iter().enumerate() {
                let answer = label.clone();
                let mut option = div()
                    .id(("answer", index * 100 + n))
                    .h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::MEDIUM)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(description),
                    );
                if !self.pending {
                    option = option
                        .cursor_pointer()
                        .hover(|style| style.border_color(info))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.send_text(answer.clone(), false, window, cx)
                        }));
                }
                card = card.child(option);
            }
            card = card.child(div().text_xs().text_color(muted).child(
                if question.allow_freeform {
                    tr("Or type your own answer below.")
                } else {
                    tr("Pick one of these to continue.")
                },
            ));
        }
        div().w_full().px_6().py_2().child(card).into_any_element()
    }

    /// A plan the model submitted. While it is the one waiting, it can be
    /// approved here; approving leaves plan mode and starts the work.
    fn plan_card(
        &self,
        index: usize,
        steps: Vec<(String, wisp_dto::PlanStatus)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let planning = self.options().plan_first;
        let waiting = controls::awaiting_approval(&self.rows, index, running, planning);
        let accent = cx.theme().primary;
        let muted = cx.theme().muted_foreground;
        let mut card = div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(accent.opacity(0.4))
            .bg(accent.opacity(0.04))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(icon("plan").text_color(accent))
                    .child(div().font_weight(FontWeight::MEDIUM).child(if waiting {
                        tr("Proposed plan — waiting for you")
                    } else {
                        tr("Proposed plan")
                    })),
            );
        for (number, (step, status)) in steps.into_iter().enumerate() {
            let (glyph_name, color) = match status {
                wisp_dto::PlanStatus::Completed => ("circle-check", cx.theme().success),
                wisp_dto::PlanStatus::InProgress => ("activity-orbit", cx.theme().info),
                wisp_dto::PlanStatus::Pending => ("circle", muted),
            };
            card = card.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_2()
                    .child(icon(glyph_name).mt(px(3.)).text_color(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(format!("{}. {step}", number + 1)),
                    ),
            );
        }
        if waiting {
            card = card
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .pt_1()
                        .child(
                            Button::new(("plan-approve", index))
                                .icon(glyph("play"))
                                .label(controls::APPROVE_PLAN)
                                .small()
                                .primary()
                                .disabled(self.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.decide_plan(true, window, cx)
                                })),
                        )
                        .child(
                            Button::new(("plan-exit", index))
                                .label(tr("Leave plan mode"))
                                .small()
                                .disabled(self.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.decide_plan(false, window, cx)
                                })),
                        ),
                )
                .child(div().text_xs().text_color(muted).child(tr(
                    "To change the plan, say what to change below and send. \
                     Anything typed there is sent with the approval.",
                )));
        }
        div().w_full().px_6().py_2().child(card).into_any_element()
    }

    /// One transcript row. Your messages sit in a bubble on the right, Wisp's
    /// answers are plain text, and tool calls fold into one line each.
    pub(super) fn row(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let id = SharedString::from(format!(
            "message-{}-{index}",
            self.state.selection().generation
        ));
        match item.role.as_str() {
            "user" => {
                let text = compose::user_text(item);
                let mut message = div().w_full().v_flex().items_end().gap_1().px_6().py_2();
                if !text.is_empty() {
                    message = message.child(
                        div()
                            .max_w(px(680.))
                            .px_4()
                            .py_2()
                            .rounded(cx.theme().radius_lg)
                            .bg(cx.theme().muted)
                            .child(TextView::markdown(id, text, window, cx).selectable(true)),
                    );
                }
                if !item.attachments.is_empty() {
                    // The files the message carried, by name; the full path
                    // is in the copied text.
                    let mut files = div().h_flex().flex_wrap().justify_end().gap_1();
                    for path in &item.attachments {
                        files = files.child(
                            Self::chip_box(cx)
                                .child(icon("doc").text_color(cx.theme().muted_foreground))
                                .child(crate::files::file_name(path).to_owned()),
                        );
                    }
                    message = message.child(files);
                }
                message
                    .child(self.message_actions(index, cx))
                    .into_any_element()
            }
            "tool" => {
                if let Some(question) = controls::question(item) {
                    return self.question_card(index, question, id, window, cx);
                }
                if let Some(steps) = controls::plan_steps(item) {
                    return self.plan_card(index, steps, cx);
                }
                let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
                let key = present::tool_key(item, index);
                let open = self.open_tools.contains(&key);
                let muted = cx.theme().muted_foreground;
                let mut header = div()
                    .id(SharedString::from(format!("tool-{key}")))
                    .h_flex()
                    .gap_2()
                    .px_3()
                    .h(px(34.))
                    .cursor_pointer()
                    .child(
                        icon(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .text_color(muted),
                    )
                    .child(icon("wrench").text_color(muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(present::heading(item)),
                    );
                if let Some((label, tone)) = present::tool_status(item, running) {
                    header = header.child(chip(label, tone, cx));
                }
                let header = header.on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_tool(key.clone(), index, cx)),
                );
                let mut card = div()
                    .v_flex()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(header);
                if open {
                    card = card.child(div().px_3().pb_3().child(
                        TextView::markdown(id, present::body(item), window, cx).selectable(true),
                    ));
                }
                div().w_full().px_6().py_1().child(card).into_any_element()
            }
            _ if item.text.is_empty() => div().into_any_element(),
            _ => div()
                .w_full()
                .v_flex()
                .gap_1()
                .px_6()
                .py_2()
                .child(TextView::markdown(id, item.text.clone(), window, cx).selectable(true))
                .children(self.inline_pictures(&item.text))
                .child(self.message_actions(index, cx))
                .into_any_element(),
        }
    }

    fn render_approvals(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let snapshot = self.state.snapshot.as_ref()?;
        if snapshot.approvals.is_empty() {
            return None;
        }
        let attention = tone_color(Tone::Attention, cx);
        let mut approvals = div()
            .id("approvals")
            .v_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .pb_2()
            .max_h(px(260.))
            .overflow_y_scroll();
        for approval in &snapshot.approvals {
            let id = approval.approval_id.clone();
            let answer = |label: &'static str, glyph_name: &'static str, approved, remember| {
                let id = id.clone();
                Button::new(SharedString::from(format!("{label}-{id}")))
                    .icon(glyph(glyph_name))
                    .label(label)
                    .small()
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.answer_approval(id.clone(), approved, remember, window, cx)
                    }))
            };
            let mut buttons = div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .child(answer(tr("Allow once"), "check", true, Remember::No).primary());
            if controls::can_allow_for_conversation(approval) {
                buttons = buttons
                    .child(answer(
                        tr("Allow for this conversation"),
                        "circle-check",
                        true,
                        Remember::Conversation,
                    ))
                    .child(
                        answer(
                            tr("Always in this project"),
                            "shield",
                            true,
                            Remember::Project,
                        )
                        .tooltip(tr("Take it back in Settings, under Permissions")),
                    );
            }
            buttons = buttons.child(answer(tr("Deny"), "ban", false, Remember::No));
            approvals =
                approvals.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .p_3()
                        .rounded(cx.theme().radius_lg)
                        .border_1()
                        .border_color(attention.opacity(0.5))
                        .bg(attention.opacity(0.06))
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(icon("shield").text_color(attention))
                                .child(div().font_weight(FontWeight::MEDIUM).child(
                                    crate::i18n::fill(
                                        tr("{} needs your approval"),
                                        &[&approval.tool],
                                    ),
                                )),
                        )
                        .child(div().text_sm().child(approval.message.clone()))
                        // A long preview scrolls by itself; the answer stays in view.
                        .child(
                            div()
                                .id(SharedString::from(format!("approval-preview-{id}")))
                                .max_h(px(110.))
                                .overflow_y_scroll()
                                .p_2()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().muted)
                                .text_xs()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(approval.preview.clone()),
                        )
                        .child(buttons),
                );
        }
        Some(approvals)
    }

    /// The card a composer menu's rows sit in, drawn just above the button
    /// that opens it.
    fn composer_menu(width: f32, cx: &App) -> Div {
        div()
            .absolute()
            .bottom(px(34.))
            .left_0()
            .w(px(width))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
    }

    pub(super) fn menu_row(
        id: SharedString,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        chosen: bool,
        cx: &App,
    ) -> Stateful<Div> {
        let hover = cx.theme().list_hover;
        div()
            .id(id)
            .h_flex()
            .gap_2()
            .px_2()
            .h(px(30.))
            .rounded(cx.theme().radius)
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(hover))
            .child(div().flex_shrink_0().child(label.into()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail.into()),
            )
            .when(chosen, |row| {
                row.child(icon("check").text_color(cx.theme().primary))
            })
    }

    fn effort_menu(&self, current: &str, cx: &mut Context<Self>) -> Div {
        let mut menu = Self::composer_menu(190., cx);
        for (effort, label) in controls::EFFORTS {
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("effort-{effort}")),
                    tr(label),
                    "",
                    effort == current,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.choose_effort(effort, window, cx)),
                ),
            );
        }
        menu
    }

    /// The experts, with "none" first.
    fn expert_menu(&self, current: &str, cx: &mut Context<Self>) -> Div {
        let mut menu = Self::composer_menu(320., cx);
        let chosen = self.experts.iter().any(|expert| expert.id == current);
        menu = menu.child(
            Self::menu_row("expert-none".into(), tr("No expert"), "", !chosen, cx).on_click(
                cx.listener(|this, _, window, cx| this.choose_expert(String::new(), window, cx)),
            ),
        );
        for expert in &self.experts {
            let id = expert.id.clone();
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("expert-choice-{id}")),
                    expert.name.clone(),
                    expert.description.clone(),
                    expert.id == current,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_expert(id.clone(), window, cx)
                })),
            );
        }
        menu
    }

    /// The saved models, with "follow the default" first.
    fn model_menu(&self, current: &str, cx: &mut Context<Self>) -> Div {
        let mut menu = Self::composer_menu(300., cx);
        let Some(models) = self.settings.current() else {
            return menu;
        };
        let default = settings::default_profile(models)
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let chosen = models.profiles.iter().any(|profile| profile.id == current);
        menu = menu.child(
            Self::menu_row("model-default".into(), tr("Default"), default, !chosen, cx).on_click(
                cx.listener(|this, _, window, cx| this.choose_model(String::new(), window, cx)),
            ),
        );
        for profile in &models.profiles {
            let id = profile.id.clone();
            menu =
                menu.child(
                    Self::menu_row(
                        SharedString::from(format!("model-choice-{id}")),
                        profile.name.clone(),
                        match &profile.problem {
                            Some(_) => tr("Needs attention in Settings").to_owned(),
                            None => profile.fields.model.clone(),
                        },
                        profile.id == current,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_model(id.clone(), window, cx)
                    })),
                );
        }
        menu
    }

    /// Where the conversation's work runs: the default, this computer, or
    /// one of the saved hosts.
    fn runs_on_menu(&self, current: &str, cx: &mut Context<Self>) -> Div {
        let mut menu = Self::composer_menu(320., cx);
        let Some(view) = &self.environments else {
            return menu;
        };
        let choose = |runs_on: String| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                this.choose_runs_on(runs_on.clone(), window, cx)
            })
        };
        menu = menu
            .child(
                Self::menu_row(
                    "runs-on-default".into(),
                    tr("Default"),
                    crate::i18n::fill(
                        tr("{}, as Settings says"),
                        &[&settings::runs_on_label("", view.default.as_deref())],
                    ),
                    current.is_empty(),
                    cx,
                )
                .on_click(choose(String::new())),
            )
            .child(
                Self::menu_row(
                    "runs-on-local".into(),
                    tr("This computer"),
                    tr("Runs start here"),
                    current == RUNS_LOCALLY,
                    cx,
                )
                .on_click(choose(RUNS_LOCALLY.into())),
            );
        for host in &view.hosts {
            let check = view.checks.iter().find(|check| check.alias == host.alias);
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("runs-on-{}", host.alias)),
                    host.alias.clone(),
                    match settings::host_caution(check) {
                        Some(caution) => caution.to_owned(),
                        None => settings::ssh_target(host),
                    },
                    current == host.alias,
                    cx,
                )
                .on_click(choose(host.alias.clone())),
            );
        }
        menu
    }

    /// What the user asked in the open conversation, as places to jump to.
    fn outline_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let entries = crate::outline::entries(&self.rows);
        let mut menu = div()
            .id("outline-menu")
            .absolute()
            .top(px(34.))
            .right_0()
            .w(px(420.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md();
        if entries.is_empty() {
            menu = menu.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Nothing has been asked here yet.")),
            );
        }
        if self.before.is_some() {
            menu = menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Earlier messages are listed once they are loaded.")),
            );
        }
        for entry in entries {
            let row = entry.row;
            menu = menu.child(
                div()
                    .id(("outline-entry", row))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .h(px(28.))
                    .flex_shrink_0()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(
                        div()
                            .w(px(24.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(entry.number.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(entry.title),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.layers.close(Layer::Outline);
                        this.list.scroll_to_reveal_item(row);
                        this.focus_composer(window, cx);
                        cx.notify();
                    })),
            );
        }
        menu
    }

    /// The small rounded box a file or reference is shown in.
    fn chip_box(cx: &App) -> Div {
        div()
            .h_flex()
            .gap_1()
            .px_2()
            .h(px(24.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_xs()
    }

    fn render_composer(&self, running: bool, cx: &mut Context<Self>) -> Div {
        let attached = self.attached();
        let typed = self.composer.read(cx).value();
        // Files alone are a message.
        let has_files = attached.iter().any(|chip| matches!(chip, Chip::File(_)));
        let sendable = if typed.trim().is_empty() && has_files {
            "files"
        } else {
            typed.as_ref()
        };
        let can_send = self.state.can_send(sendable, self.pending) && !self.loading_history;
        let options = self.options();
        let muted = cx.theme().muted_foreground;
        let attention = tone_color(Tone::Attention, cx);
        let full = options.full_access;

        let planning = options.plan_first;
        let plan = Button::new("plan-first")
            .icon(glyph("plan"))
            .label(if planning {
                tr("Planning")
            } else {
                tr("Plan first")
            })
            .small()
            .ghost()
            .selected(planning)
            .tooltip(if planning {
                tr("Wisp investigates and proposes a plan; it changes nothing until you approve")
            } else {
                tr("Have Wisp propose a plan before it changes anything")
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_plan_first(window, cx)));

        let access = Button::new("access")
            .icon(glyph(if full { "bolt" } else { "shield" }))
            .label(if full {
                tr("Full access")
            } else {
                tr("Ask first")
            })
            .small()
            .ghost()
            .selected(full || self.layers.contains(Layer::ConfirmFullAccess))
            .tooltip(if full {
                tr("Wisp changes files and runs commands without asking")
            } else {
                tr("Wisp asks before it changes files or runs commands")
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_full_access(window, cx)));
        let effort_open = self.layers.contains(Layer::EffortMenu);
        let mut effort = div().relative().child(
            Button::new("effort")
                .icon(glyph("gauge"))
                .label(crate::i18n::fill(
                    tr("Effort: {}"),
                    &[&controls::effort_label(&options.reasoning_effort)],
                ))
                .small()
                .ghost()
                .selected(effort_open)
                .tooltip(tr("How hard the model thinks, from the next message"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::EffortMenu, window, cx)
                })),
        );
        if effort_open {
            // Painted after everything else: a parent's border is drawn over
            // its own children, and the composer box has one.
            effort = effort.child(deferred(self.effort_menu(&options.reasoning_effort, cx)));
        }

        let model_open = self.layers.contains(Layer::ModelMenu);
        let mut model = div().relative().child(
            Button::new("model")
                .icon(glyph("bot"))
                .label(settings::model_label(
                    self.settings.current(),
                    &options.model,
                ))
                .small()
                .ghost()
                .selected(model_open)
                .tooltip(tr(
                    "The model this conversation runs on, from the next message",
                ))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::ModelMenu, window, cx)
                })),
        );
        if model_open {
            model = model.child(deferred(self.model_menu(&options.model, cx)));
        }

        // Requests kept under a name, to put into the message box.
        let quick = (!self.quick_actions.is_empty()).then(|| {
            let open = self.layers.contains(Layer::QuickMenu);
            let button = div().relative().child(
                Button::new("quick-actions")
                    .icon(glyph("bolt"))
                    .small()
                    .ghost()
                    .selected(open)
                    .tooltip(tr("Quick actions: put a saved request into the message"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        // One may have been added or removed since.
                        this.load_quick_actions(window, cx);
                        this.toggle_composer_menu(Layer::QuickMenu, window, cx)
                    })),
            );
            if open {
                let mut menu = Self::composer_menu(320., cx);
                for action in &self.quick_actions {
                    let prompt = action.prompt.clone();
                    menu = menu.child(
                        Self::menu_row(
                            SharedString::from(format!("quick-choice-{}", action.id)),
                            action.name.clone(),
                            "",
                            false,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.use_quick_action(prompt.clone(), window, cx)
                            },
                        )),
                    );
                }
                button.child(deferred(menu))
            } else {
                button
            }
        });

        // Who the conversation is started as. It is chosen before the
        // first message and only named afterwards.
        let started = !self.rows.is_empty() || running;
        let expert =
            (!self.experts.is_empty() && (!started || !options.expert.is_empty())).then(|| {
                let open = self.layers.contains(Layer::ExpertMenu);
                let button = div().relative().child(
                    Button::new("expert")
                        .icon(glyph("user"))
                        .label(settings::expert_label(&self.experts, &options.expert))
                        .small()
                        .ghost()
                        .selected(open)
                        .disabled(started)
                        .tooltip(if started {
                            tr("The expert this conversation was started as")
                        } else {
                            tr("Start this conversation as an expert")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            // One may have been added or removed since.
                            this.load_experts(window, cx);
                            this.toggle_composer_menu(Layer::ExpertMenu, window, cx)
                        })),
                );
                if open {
                    button.child(deferred(self.expert_menu(&options.expert, cx)))
                } else {
                    button
                }
            });

        // Offered once there is a host to choose, or a choice to undo.
        let hosts = self.environments.as_ref();
        let runs_on = (hosts.is_some_and(|view| !view.hosts.is_empty())
            || !options.runs_on.is_empty())
        .then(|| {
            let open = self.layers.contains(Layer::RunsOnMenu);
            let place = div().relative().child(
                Button::new("runs-on")
                    .icon(glyph("server"))
                    .label(settings::runs_on_label(
                        &options.runs_on,
                        hosts.and_then(|view| view.default.as_deref()),
                    ))
                    .small()
                    .ghost()
                    .selected(open)
                    .tooltip(tr(
                        "Where this conversation's runs start, from the next message",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        // A host may have been added or checked since.
                        this.load_environments(window, cx);
                        this.toggle_composer_menu(Layer::RunsOnMenu, window, cx)
                    })),
            );
            if open {
                place.child(deferred(self.runs_on_menu(&options.runs_on, cx)))
            } else {
                place
            }
        });

        let mut actions = div().h_flex().gap_2();
        let usage = self
            .status
            .as_ref()
            .and_then(|status| status.usage.as_ref());
        if let Some(percent) = usage.and_then(controls::context_percent) {
            actions = actions.child(
                div()
                    .id("context-meter")
                    .text_xs()
                    .text_color(if percent >= 90 {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .cursor_pointer()
                    .hover(|meter| meter.text_color(cx.theme().foreground))
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(tr("See what the model is sent"))
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_model_view(window, cx)))
                    .child(crate::i18n::fill(tr("Context {}%"), &[&percent])),
            );
        }
        if running {
            actions = actions.child(
                Button::new("stop")
                    .icon(glyph("stop"))
                    .label(tr("Stop"))
                    .small()
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.control("native_conversation_stop", json!({}), window, cx)
                    })),
            );
        }
        // While a turn runs, what is written waits for it to end.
        let can_queue = running
            && !self.pending
            && !self.loading_history
            && !sendable.trim().is_empty()
            && self.state.snapshot.as_ref().is_some_and(|s| !s.read_only);
        actions = actions.child(
            Button::new("send")
                .icon(glyph(if running { "clock" } else { "arrow-up" }))
                .label(if running { tr("Queue") } else { tr("Send") })
                .small()
                .primary()
                .disabled(!(can_send || can_queue))
                .tooltip(if running {
                    tr("Send it when this turn ends")
                } else {
                    tr("Send")
                })
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
        );

        let mut area = div().v_flex().flex_shrink_0().gap_1().px_4().pb_4().pt_1();
        // What the last turn used, and what this conversation may do unasked.
        let allowed = self
            .status
            .as_ref()
            .map(|status| status.allowed_tools.join(", "))
            .unwrap_or_default();
        let runtimes = self
            .status
            .as_ref()
            .map(|status| status.runtimes.as_slice())
            .unwrap_or_default();
        if usage.is_some() || !allowed.is_empty() || !runtimes.is_empty() {
            let mut line = div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .px_1()
                .text_xs()
                .text_color(muted);
            // The interpreters this conversation keeps between tool calls.
            for row in runtimes {
                let mut chip = div()
                    .id(SharedString::from(format!("runtime-{}", row.id)))
                    .h_flex()
                    .gap_1()
                    .child(
                        div()
                            .id(SharedString::from(format!("runtime-label-{}", row.id)))
                            .child(controls::runtime_label(row))
                            .tooltip({
                                let detail = controls::runtime_detail(row);
                                move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(detail.clone())
                                        .build(window, cx)
                                }
                            }),
                    );
                if let Some((label, hint)) = controls::runtime_action(row) {
                    let id = row.id.clone();
                    chip = chip.child(
                        Button::new(SharedString::from(format!("runtime-stop-{}", row.id)))
                            .label(label)
                            .xsmall()
                            .ghost()
                            .tooltip(hint)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.stop_runtime(id.clone(), window, cx)
                            })),
                    );
                }
                line = line.child(chip);
            }
            if !allowed.is_empty() {
                line = line
                    .child(crate::i18n::fill(
                        tr("Runs without asking in this conversation: {}"),
                        &[&allowed],
                    ))
                    .child(
                        Button::new("revoke-allowed")
                            .label(tr("Ask again"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.revoke_allowed_tools(window, cx)
                            })),
                    );
            }
            line = line.child(div().flex_1());
            if let Some(usage) = usage {
                line = line.child(controls::usage_line(usage));
            }
            area = area.child(line);
        }
        // What the model is thinking, while it thinks.
        let thinking = self
            .status
            .as_ref()
            .map(|status| status.reasoning.as_str())
            .filter(|text| running && !text.trim().is_empty());
        if let Some(thinking) = thinking {
            area = area.child(
                div()
                    .v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(muted)
                            .child(icon("brain").text_color(muted))
                            .child(tr("Thinking")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(controls::thinking_tail(thinking, 600)),
                    ),
            );
        }
        if let Some(strip) = self.queue_strip(running, cx) {
            area = area.child(strip);
        }
        if self.layers.contains(Layer::ConfirmFullAccess) {
            area = area.child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(cx.theme().radius)
                    .bg(attention.opacity(0.08))
                    .child(icon("bolt").text_color(attention))
                    .child(div().flex_1().min_w_0().text_sm().child(tr(
                        "Let Wisp change files and run commands in this conversation without \
                         asking? Dangerous shell commands still ask.",
                    )))
                    .child(
                        Button::new("full-access-confirm")
                            .label(tr("Turn on"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_full_access(window, cx)
                            })),
                    )
                    .child(
                        Button::new("full-access-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_full_access(window, cx)
                            })),
                    ),
            );
        }
        let mut editor = div()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().input)
            .bg(cx.theme().popover);
        if !attached.is_empty() {
            let mut chips = div().h_flex().flex_wrap().gap_1();
            for (index, chip) in attached.iter().enumerate() {
                chips = chips.child(
                    Self::chip_box(cx)
                        .child(icon(chip.icon()).text_color(muted))
                        .child(div().max_w(px(220.)).truncate().child(chip.label()))
                        .child(
                            icon_button(("detach", index), "close", tr("Remove"))
                                .xsmall()
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.detach(index, cx)),
                                ),
                        ),
                );
            }
            editor = editor.child(chips);
        }
        let attach = div()
            .h_flex()
            .child(
                icon_button("attach-files", "attach", tr("Attach files"))
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.attach_files(window, cx))),
            )
            .child(
                icon_button(
                    "attach-context",
                    "link",
                    tr("Add a project file, a conversation or a skill"),
                )
                .on_click(
                    cx.listener(|this, _, window, cx| this.toggle_attach_palette(window, cx)),
                ),
            );
        area.child(
            editor
                .child(
                    Input::new(&self.composer)
                        .appearance(false)
                        .disabled(self.pending),
                )
                .child(
                    div()
                        .h_flex()
                        .justify_between()
                        .child(
                            div()
                                .h_flex()
                                .gap_1()
                                .child(attach)
                                .children(quick)
                                .child(plan)
                                .child(access)
                                .child(model)
                                .child(effort)
                                .children(runs_on)
                                .children(expert)
                                .child(
                                    Button::new("auto-review")
                                        .icon(glyph("review"))
                                        .label(if options.auto_review {
                                            tr("Reviewing")
                                        } else {
                                            tr("Review turns")
                                        })
                                        .small()
                                        .ghost()
                                        .selected(options.auto_review)
                                        .tooltip(if options.auto_review {
                                            tr("Each turn that used a tool is reviewed when it ends")
                                        } else {
                                            tr("Have each turn that used a tool reviewed when it ends")
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_auto_review(window, cx)
                                        })),
                                ),
                        )
                        .child(actions),
                ),
        )
    }

    /// Everything to the right of the sidebar.
    pub(super) fn render_conversation(&self, cx: &mut Context<Self>) -> Div {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let drop_tint = cx.theme().primary.opacity(0.06);
        let mut content = div()
            .v_flex()
            .flex_1()
            .min_w(px(300.))
            .h_full()
            // Files dragged in from outside are attached to the message
            // being written.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, window, cx| {
                this.attach_dropped(dropped.paths(), window, cx)
            }))
            .children(self.render_tabs(cx))
            .child(self.render_topbar(cx));
        if let Some(bar) = self.render_conversation_bar(cx) {
            content = content.child(bar);
        }
        if !self.settings.configured() {
            let attention = tone_color(Tone::Attention, cx);
            content = content.child(
                div()
                    .h_flex()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .flex_shrink_0()
                    .bg(attention.opacity(0.08))
                    .child(icon("circle-alert").text_color(attention))
                    .child(div().flex_1().text_sm().child(tr(
                        "No model is configured, so messages cannot be sent yet.",
                    )))
                    .child(
                        Button::new("configure-model")
                            .label(tr("Open Settings"))
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    ),
            );
        }
        if let Some(error) = self
            .error
            .as_ref()
            .or_else(|| self.state.snapshot.as_ref().and_then(|s| s.error.as_ref()))
        {
            content = content.child(div().px_4().pt_2().child(error_banner(error, cx)));
        }
        if let Some(notice) = &self.notice {
            content = content.child(div().px_4().pt_2().child(self.notice_banner(notice, cx)));
        }
        if self.state.session.is_none() {
            let can_start = !self.pending && self.state.project.is_some();
            return content.child(
                div()
                    .v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .p_8()
                    .child(
                        div()
                            .text_2xl()
                            .child(tr("What would you like to research today?")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("Start a conversation, or pick one on the left.")),
                    )
                    .child(
                        Button::new("empty-new-chat")
                            .icon(glyph("plus"))
                            .label(tr("New conversation"))
                            .primary()
                            .disabled(!can_start)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.create_session(window, cx)),
                            ),
                    ),
            );
        }
        if self.before.is_some() {
            content = content.child(
                div().h_flex().justify_center().py_2().child(
                    Button::new("older")
                        .icon(glyph("history"))
                        .label(tr("Load earlier messages"))
                        .small()
                        .ghost()
                        .disabled(self.loading_history || self.pending || running)
                        .on_click(cx.listener(|this, _, window, cx| this.older(window, cx))),
                ),
            );
        }
        if self.layers.contains(Layer::Find) {
            content = content.child(self.render_find_bar(cx));
        }
        let weak = cx.entity().downgrade();
        // The pet stands in the transcript's lower right corner.
        let pet = self
            .pet
            .as_ref()
            .is_some_and(|pet| pet.look.is_some())
            .then(|| {
                div()
                    .absolute()
                    .right_4()
                    .bottom_2()
                    .child(self.pet_view.clone())
            });
        content = content.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    list(self.list.clone(), move |index, window, cx| {
                        weak.update(cx, |this, cx| this.found_row(index, window, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .flex_1()
                    .min_h_0(),
                )
                .children(pet),
        );
        if let Some(approvals) = self.render_approvals(cx) {
            content = content.child(approvals);
        }
        if let Some(review) = self.render_review_banner(cx) {
            content = content.child(review);
        }
        if let Some(follow_ups) = self.render_follow_ups(cx) {
            content = content.child(follow_ups);
        }
        content.child(self.render_composer(running, cx))
    }
}
