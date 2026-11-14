//! The project screen: sidebar, top bar, transcript rows, approvals and the
//! composer. State changes live in the parent module; this file only draws.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use serde_json::json;

use super::{Workspace, chip, tone_color};
use crate::{
    assets::{icon, icon_path},
    controls::{self, Question},
    layers::Layer,
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
        .tooltip(tooltip)
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
            return body.child(div().text_color(muted).child("Starting Wisp…"));
        }
        body = body
            .child(div().text_lg().child("Wisp could not start"))
            .child(div().max_w(px(520.)).text_sm().text_color(muted).child(
                "Another Wisp desktop may already be using this data directory. \
                         See docs/gpui-desktop.md for the data and workspace locations.",
            ));
        if let Some(error) = &self.error {
            body = body.child(div().max_w(px(520.)).child(error_banner(error, cx)));
        }
        body.child(
            Button::new("retry")
                .label("Try again")
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
            .unwrap_or_else(|| "Project".into());

        let mut new_chat =
            nav_row(
                "new-chat",
                "plus",
                "New conversation",
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
            "Files",
            self.layers.contains(Layer::FilesPanel),
            has_project,
            cx,
        );
        if has_project {
            files =
                files.on_click(cx.listener(|this, _, window, cx| this.toggle_files(window, cx)));
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
                        icon_button("home", "arrow-left", "All projects")
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
                        .child("Needs you"),
                )
                .child(inbox);
        }

        sidebar =
            sidebar.child(
                div()
                    .h_flex()
                    .px_2()
                    .pt_2()
                    .flex_shrink_0()
                    .child(div().flex_1().text_xs().text_color(muted).child(
                        if self.show_archived {
                            "Archived"
                        } else {
                            "Conversations"
                        },
                    ))
                    .child(
                        icon_button(
                            "archive-view",
                            "archive",
                            if self.show_archived {
                                "Back to conversations"
                            } else {
                                "Archived conversations"
                            },
                        )
                        .xsmall()
                        .selected(self.show_archived)
                        .disabled(disabled || !has_project)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_archived = !this.show_archived;
                            this.sessions.clear();
                            this.load_sessions(window, cx);
                            cx.notify();
                        })),
                    ),
            );

        let searching = !self.search.read(cx).value().trim().is_empty();
        let mut sessions = div()
            .id("sessions")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.sessions.is_empty() && has_project {
            sessions = sessions.child(div().px_2().py_1().text_xs().text_color(muted).child(
                if searching {
                    "No conversation matches."
                } else if self.show_archived {
                    "Nothing is archived."
                } else {
                    "No conversations yet."
                },
            ));
        }
        for session in &self.sessions {
            let id = session.id.clone();
            let selected = Some(&id) == self.state.session.as_ref();
            let badge = present::session_badge(session, self.pinned.contains(&id));
            let mut row = div()
                .id(SharedString::from(format!("session-{id}")))
                .h_flex()
                .gap_2()
                .px_2()
                .h(px(30.))
                .flex_shrink_0()
                .rounded(radius)
                .text_sm()
                .when(selected, |row| row.bg(accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(session.title.clone()),
                );
            if let Some((label, tone)) = badge {
                row = row.child(chip(label, tone, cx));
            }
            if !disabled {
                row = row
                    .cursor_pointer()
                    .hover(|style| style.bg(accent.opacity(0.7)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_session(id.clone(), window, cx)
                    }));
            }
            sessions = sessions.child(row);
        }

        sidebar.child(sessions).child(
            nav_row("settings", "gear", "Settings", false, true, cx)
                .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
        )
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> Div {
        let title = self
            .current_row()
            .map(|row| row.title.clone())
            .unwrap_or_else(|| "New research".into());
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
                        icon_button("palette", "search", "Search everything").on_click(
                            cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("refresh", "refresh", "Refresh")
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.error = None;
                                this.refresh_lists(window, cx);
                                this.start_poll(window, cx);
                                this.reload_files(window, cx);
                            })),
                    )
                    .child(
                        icon_button("files-toggle", "panel", "Files")
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
        div().h_flex().child(
            icon_button(
                ("copy-row", index),
                if copied { "check" } else { "copy" },
                if copied { "Copied" } else { "Copy" },
            )
            .xsmall()
            .on_click(cx.listener(move |this, _, window, cx| this.copy_row(index, window, cx))),
        )
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
                        "Wisp needs your decision"
                    } else {
                        "Wisp asked"
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
                    "Or type your own answer below."
                } else {
                    "Pick one of these to continue."
                },
            ));
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
            "user" => div()
                .w_full()
                .v_flex()
                .items_end()
                .gap_1()
                .px_6()
                .py_2()
                .child(
                    div()
                        .max_w(px(680.))
                        .px_4()
                        .py_2()
                        .rounded(cx.theme().radius_lg)
                        .bg(cx.theme().muted)
                        .child(
                            TextView::markdown(id, item.text.clone(), window, cx).selectable(true),
                        ),
                )
                .child(self.message_actions(index, cx))
                .into_any_element(),
            "tool" => {
                if let Some(question) = controls::question(item) {
                    return self.question_card(index, question, id, window, cx);
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
            let answer = |label: &'static str, glyph_name: &'static str, approved, lasting| {
                let id = id.clone();
                Button::new(SharedString::from(format!("{label}-{id}")))
                    .icon(glyph(glyph_name))
                    .label(label)
                    .small()
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.answer_approval(id.clone(), approved, lasting, window, cx)
                    }))
            };
            let mut buttons = div()
                .h_flex()
                .gap_2()
                .child(answer("Allow once", "check", true, false).primary());
            if controls::can_allow_for_conversation(approval) {
                buttons = buttons.child(answer(
                    "Allow for this conversation",
                    "circle-check",
                    true,
                    true,
                ));
            }
            buttons = buttons.child(answer("Deny", "ban", false, false));
            approvals = approvals.child(
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
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("{} needs your approval", approval.tool)),
                            ),
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

    fn menu_row(
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
                    label,
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
            Self::menu_row("model-default".into(), "Default", default, !chosen, cx).on_click(
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
                            Some(_) => "Needs attention in Settings".to_owned(),
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

    fn render_composer(&self, running: bool, cx: &mut Context<Self>) -> Div {
        let can_send = self
            .state
            .can_send(&self.composer.read(cx).value(), self.pending)
            && !self.loading_history;
        let options = self.options();
        let muted = cx.theme().muted_foreground;
        let attention = tone_color(Tone::Attention, cx);
        let full = options.full_access;

        let access = Button::new("access")
            .icon(glyph(if full { "bolt" } else { "shield" }))
            .label(if full { "Full access" } else { "Ask first" })
            .small()
            .ghost()
            .selected(full || self.layers.contains(Layer::ConfirmFullAccess))
            .tooltip(if full {
                "Wisp changes files and runs commands without asking"
            } else {
                "Wisp asks before it changes files or runs commands"
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_full_access(window, cx)));
        let effort_open = self.layers.contains(Layer::EffortMenu);
        let mut effort = div().relative().child(
            Button::new("effort")
                .icon(glyph("gauge"))
                .label(format!(
                    "Effort: {}",
                    controls::effort_label(&options.reasoning_effort)
                ))
                .small()
                .ghost()
                .selected(effort_open)
                .tooltip("How hard the model thinks, from the next message")
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
                .tooltip("The model this conversation runs on, from the next message")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::ModelMenu, window, cx)
                })),
        );
        if model_open {
            model = model.child(deferred(self.model_menu(&options.model, cx)));
        }

        let mut actions = div().h_flex().gap_2();
        let usage = self
            .status
            .as_ref()
            .and_then(|status| status.usage.as_ref());
        if let Some(percent) = usage.and_then(controls::context_percent) {
            actions = actions.child(
                div()
                    .text_xs()
                    .text_color(if percent >= 90 {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .child(format!("Context {percent}%")),
            );
        }
        if running {
            actions = actions.child(
                Button::new("stop")
                    .icon(glyph("stop"))
                    .label("Stop")
                    .small()
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.control("native_conversation_stop", json!({}), window, cx)
                    })),
            );
        }
        actions = actions.child(
            Button::new("send")
                .icon(glyph("arrow-up"))
                .label("Send")
                .small()
                .primary()
                .disabled(!can_send)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
        );

        let mut area = div().v_flex().flex_shrink_0().gap_1().px_4().pb_4().pt_1();
        // What the last turn used, and what this conversation may do unasked.
        let allowed = self
            .status
            .as_ref()
            .map(|status| status.allowed_tools.join(", "))
            .unwrap_or_default();
        if usage.is_some() || !allowed.is_empty() {
            let mut line = div().h_flex().gap_2().px_1().text_xs().text_color(muted);
            if !allowed.is_empty() {
                line = line
                    .child(format!(
                        "Runs without asking in this conversation: {allowed}"
                    ))
                    .child(
                        Button::new("revoke-allowed")
                            .label("Ask again")
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
                    .child(div().flex_1().min_w_0().text_sm().child(
                        "Let Wisp change files and run commands in this conversation without \
                         asking? Dangerous shell commands still ask.",
                    ))
                    .child(
                        Button::new("full-access-confirm")
                            .label("Turn on")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_full_access(window, cx)
                            })),
                    )
                    .child(
                        Button::new("full-access-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_full_access(window, cx)
                            })),
                    ),
            );
        }
        area.child(
            div()
                .v_flex()
                .gap_1()
                .p_2()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().input)
                .bg(cx.theme().popover)
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
                                .child(access)
                                .child(model)
                                .child(effort),
                        )
                        .child(actions),
                ),
        )
    }

    /// Everything to the right of the sidebar.
    pub(super) fn render_conversation(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let mut content = div()
            .v_flex()
            .flex_1()
            .min_w(px(300.))
            .h_full()
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
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .child("No model is configured, so messages cannot be sent yet."),
                    )
                    .child(
                        Button::new("configure-model")
                            .label("Open Settings")
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
                            .child("What would you like to research today?"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Start a conversation, or pick one on the left."),
                    )
                    .child(
                        Button::new("empty-new-chat")
                            .icon(glyph("plus"))
                            .label("New conversation")
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
                        .label("Load earlier messages")
                        .small()
                        .ghost()
                        .disabled(self.loading_history || self.pending || running)
                        .on_click(cx.listener(|this, _, window, cx| this.older(window, cx))),
                ),
            );
        }
        let weak = cx.entity().downgrade();
        content = content.child(
            list(self.list.clone(), move |index, window, cx| {
                weak.update(cx, |this, cx| this.row(index, window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .flex_1()
            .min_h_0(),
        );
        if let Some(snapshot) = &self.state.snapshot {
            if !snapshot.run_cards.is_empty() {
                content = content.child(self.render_runs(snapshot, window, cx));
            }
        }
        if let Some(approvals) = self.render_approvals(cx) {
            content = content.child(approvals);
        }
        content.child(self.render_composer(running, cx))
    }
}
