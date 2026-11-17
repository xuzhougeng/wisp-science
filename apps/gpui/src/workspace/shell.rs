//! The project screen: sidebar, top bar, transcript rows, approvals and the
//! composer. State changes live in the parent module; this file only draws.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use serde_json::json;
use wisp_server::desktop::{RUNS_LOCALLY, Remember};

use super::{Workspace, surfaces, tone_color};
use crate::{
    assets::{icon, icon_path},
    compose::{self, Chip},
    controls::{self, Question},
    layers::Layer,
    panel::PanelTab,
    present::{self, Tone},
    settings,
};

/// The hover group of one message: its actions show while the pointer is
/// anywhere over it.
const MESSAGE: &str = "message";

/// The time now, in Unix seconds.
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

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

/// One line of sidebar navigation: its icon (faded, ink when the row is
/// chosen or under the pointer) and its word. The caller attaches the
/// click handler only when `enabled`, so a dimmed row cannot be activated.
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
        .group(surfaces::ICON_HOVER)
        .child(surfaces::sized_icon(
            name,
            surfaces::IconSize::Inline,
            if selected {
                surfaces::IconTone::Ink
            } else {
                surfaces::IconTone::Faded
            },
            cx,
        ))
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
        .child(div().flex_1().min_w_0().child(crate::i18n::host(message)))
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
            .child(surfaces::title(tr("Wisp could not start")))
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

    /// The sidebar's navigation folded into one row of icon buttons, for a
    /// window too short to give the conversations half the sidebar: the
    /// same seven places in the same order, each its name as tooltip, the
    /// open one in ink on white. It wraps only when the sidebar is too
    /// narrow for seven.
    fn folded_nav(&self, disabled: bool, has_project: bool, cx: &mut Context<Self>) -> Div {
        use surfaces::IconTone;
        let panel_on =
            |tab: PanelTab| self.layers.contains(Layer::FilesPanel) && self.panel_tab == tab;
        let paper = cx.theme().popover;
        let line = cx.theme().border;
        let place = |id: &'static str,
                     icon_name: &str,
                     name: SharedString,
                     open: bool,
                     enabled: bool,
                     cx: &mut Context<Self>| {
            surfaces::icon_action(
                id,
                icon_name,
                name,
                if open { IconTone::Ink } else { IconTone::Faded },
                cx,
            )
            .when(open, |button| {
                button.bg(paper).border_1().border_color(line)
            })
            .disabled(!enabled)
        };
        let new_chat_key = if cfg!(target_os = "macos") {
            "⌘N"
        } else {
            "Ctrl+N"
        };
        div()
            .h_flex()
            .flex_wrap()
            .flex_shrink_0()
            .gap_0p5()
            .py_1()
            .child(
                place(
                    "new-chat",
                    "plus",
                    crate::i18n::fill(tr("New conversation ({})"), &[&new_chat_key]).into(),
                    false,
                    !disabled && has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.create_session(window, cx))),
            )
            .child(
                place(
                    "files",
                    "folder",
                    tr("Files").into(),
                    panel_on(PanelTab::Files),
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    // With the panel on another tab, this is a way to Files.
                    if this.layers.contains(Layer::FilesPanel) && this.panel_tab != PanelTab::Files
                    {
                        this.show_panel_tab(PanelTab::Files, window, cx)
                    } else {
                        this.toggle_files(window, cx)
                    }
                })),
            )
            .child(
                place(
                    "memos",
                    "note-plus",
                    tr("Memos").into(),
                    panel_on(PanelTab::Memos),
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.layers.contains(Layer::FilesPanel) && this.panel_tab == PanelTab::Memos
                    {
                        this.toggle_files(window, cx)
                    } else {
                        this.show_panel_tab(PanelTab::Memos, window, cx)
                    }
                })),
            )
            .child(
                place(
                    "capabilities",
                    "sparkles",
                    tr("Capabilities").into(),
                    false,
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_capabilities(window, cx))),
            )
            .child(
                place(
                    "journey",
                    "research-trail",
                    tr("Journey").into(),
                    false,
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_journey(window, cx))),
            )
            .child(
                place(
                    "papers",
                    "book",
                    tr("Papers").into(),
                    false,
                    has_project,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_papers(window, cx))),
            )
            .child(
                place("favorites", "star", tr("Favorites").into(), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_favorites(window, cx))),
            )
    }

    pub(super) fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let disabled = self.pending;
        let has_project = self.state.project.is_some();
        let muted = cx.theme().muted_foreground;
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_else(|| {
                // The assistant's own project is not one of the researcher's.
                if crate::assistant::is_assistant_project(self.state.project.as_deref()) {
                    tr("Research assistant").into()
                } else {
                    tr("Project").into()
                }
            });
        // It has one conversation, its own.
        let has_project =
            has_project && !crate::assistant::is_assistant_project(self.state.project.as_deref());

        let mut new_chat = nav_row(
            "new-chat",
            "plus",
            tr("New conversation"),
            false,
            !disabled && has_project,
            cx,
        )
        .child(surfaces::fact(
            if cfg!(target_os = "macos") {
                "⌘N"
            } else {
                "Ctrl+N"
            },
            cx,
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

        let sidebar_height = f32::from(window.viewport_size().height) - surfaces::TITLE_BAR_HEIGHT;
        let fold = present::nav_folds(sidebar_height);
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
                    .child(surfaces::title(project_name).flex_1().min_w_0().truncate()),
            )
            // Short of room, the navigation folds into one row of icons so
            // the conversations keep at least half the sidebar.
            .when(fold, |sidebar| {
                sidebar.child(self.folded_nav(disabled, has_project, cx))
            })
            .when(!fold, |sidebar| {
                sidebar
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
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_capabilities(window, cx)),
                        ),
                    )
                    .child(
                        nav_row(
                            "journey",
                            "research-trail",
                            tr("Journey"),
                            false,
                            has_project,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.open_journey(window, cx))),
                    )
                    .child(
                        nav_row("papers", "book", tr("Papers"), false, has_project, cx).on_click(
                            cx.listener(|this, _, window, cx| this.open_papers(window, cx)),
                        ),
                    )
                    .child(
                        nav_row("favorites", "star", tr("Favorites"), false, true, cx).on_click(
                            cx.listener(|this, _, window, cx| this.open_favorites(window, cx)),
                        ),
                    )
            })
            .child(
                // White, with the fields' line, like the home page's.
                div().py_1().child(
                    surfaces::field(cx).child(
                        Input::new(&self.search)
                            .small()
                            .appearance(false)
                            .prefix(icon("search").text_color(muted)),
                    ),
                ),
            );

        // What waits for the user is under the bell in the window's bar
        // (`render_waiting`), so the conversations have this room.
        sidebar = sidebar
            .child(self.render_list_heading(cx))
            .child(self.render_session_list(cx));
        if let Some(bar) = self.render_batch_bar(cx) {
            sidebar = sidebar.child(bar);
        }
        // A newer version, at the foot as the WebView has it.
        if let Some(card) = self.update_card(cx) {
            sidebar = sidebar.child(card);
        }

        sidebar.child(
            nav_row("settings", "gear", tr("Settings"), false, true, cx)
                .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
        )
    }

    /// The small actions under a message. They show while the pointer is
    /// over the message, and stay under the latest message (a compaction
    /// line after it is not one), one just copied, and one whose More menu
    /// is open.
    fn message_actions(&self, index: usize, over: bool, cx: &mut Context<Self>) -> Div {
        let actions = self.message_action_items(index, over, cx);
        // The turn a range of turns to quote starts at stays marked.
        let starts_range = self
            .reuse_start()
            .and_then(|selection| selection.row(&self.rows))
            == Some(index);
        if controls::latest_message(&self.rows) == Some(index)
            || self.copied == Some(index)
            || starts_range
            || self.message_menu == Some(index)
        {
            return actions;
        }
        actions
            .invisible()
            .group_hover(MESSAGE, |style| style.visible())
    }

    /// Three icon buttons (copy, favourite, ask aside), More for the rest
    /// of what can be done with the message, and at the right end what the
    /// turn cost, as a fact.
    /// `over` lays them over a question's corner, on the paper, with no
    /// line of their own.
    fn message_action_items(&self, index: usize, over: bool, cx: &mut Context<Self>) -> Div {
        use surfaces::IconTone;
        let copied = self.copied == Some(index);
        let starred = self
            .rows
            .get(index)
            .is_some_and(|row| crate::favorites::saved_id(&self.saved, &row.text).is_some());
        let menu_open = self.message_menu == Some(index);
        let mut more = div().relative().child(
            surfaces::icon_action(
                ("more-row", index),
                "more",
                tr("More actions"),
                if menu_open {
                    IconTone::Ink
                } else {
                    IconTone::Faded
                },
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_message_menu(index, cx))),
        );
        if menu_open {
            more = more.child(deferred(self.message_menu(index, over, cx)));
        }
        // What the turn cost, after its last answer.
        let tokens = controls::turn_tokens_at(&self.rows, index, &self.turn_tokens).map(|tokens| {
            let calls = tokens.calls;
            div()
                .id(("turn-tokens", index))
                .child(surfaces::fact(controls::turn_tokens_line(tokens), cx))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(crate::i18n::fill(
                        tr("This turn, over {} model calls"),
                        &[&calls],
                    ))
                    .build(window, cx)
                })
        });
        div()
            .when(!over, |actions| actions.w_full())
            .when(over, |actions| {
                actions.rounded(cx.theme().radius).bg(cx.theme().background)
            })
            .h_flex()
            .items_center()
            .gap_0p5()
            .child(
                surfaces::icon_action(
                    ("copy-row", index),
                    if copied { "check" } else { "copy" },
                    if copied { tr("Copied") } else { tr("Copy") },
                    IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| this.copy_row(index, window, cx))),
            )
            .child(
                surfaces::icon_action(
                    ("star-row", index),
                    if starred { "star-filled" } else { "star" },
                    if starred {
                        tr("In your favorites. Click to remove")
                    } else {
                        tr("Save to favorites")
                    },
                    if starred {
                        IconTone::Ink
                    } else {
                        IconTone::Faded
                    },
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| this.star_row(index, window, cx))),
            )
            .child(
                surfaces::icon_action(
                    ("aside-row", index),
                    "bubble",
                    tr("Ask aside about this"),
                    IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ask_aside_about_row(index, window, cx)
                })),
            )
            .child(more)
            .when(!over, |actions| actions.child(div().flex_1()))
            .children(tokens)
    }

    /// Opens or closes the More menu under one message.
    fn toggle_message_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.message_menu == Some(index) && self.layers.close(Layer::MessageMenu) {
            self.message_menu = None;
        } else {
            self.message_menu = Some(index);
            self.layers.open(Layer::MessageMenu);
        }
        cx.notify();
    }

    /// The rest of what can be done with a message, in a menu over its More
    /// button: save it to memory, quote it, reuse a range of turns, and by
    /// its kind edit it, branch, review or explore from it. Each is an icon
    /// and a word; one that cannot be taken now is dimmed and inert.
    /// It opens downward from a question's corner and upward from under an
    /// answer (`down`), towards the room there is.
    fn message_menu(&self, index: usize, down: bool, cx: &mut Context<Self>) -> Div {
        type Run = fn(&mut Workspace, usize, &mut Window, &mut Context<Workspace>);
        let item = |id: &'static str,
                    icon_name: &str,
                    label: &'static str,
                    enabled: bool,
                    run: Run,
                    cx: &mut Context<Self>| {
            let row = Self::menu_item((id, index), icon_name, label, cx);
            if !enabled {
                return row.opacity(0.5).cursor_default();
            }
            row.on_click(cx.listener(move |this, _, window, cx| {
                this.layers.close(Layer::MessageMenu);
                this.message_menu = None;
                run(this, index, window, cx);
                cx.notify();
            }))
        };
        // Rewinding and branching work on saved turns of a conversation
        // that is at rest and still part of everyday work.
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let archived = self.current_row().is_none_or(|row| row.shelved);
        let idle = !running && !self.pending;
        // A read-only conversation's messages are not edited, a branch is
        // not branched again, and a frozen one is not reviewed.
        let offered = self.offered();
        let mut menu = div()
            .absolute()
            .when(down, |menu| {
                menu.top(px(surfaces::ICON_BUTTON + 4.)).right_0()
            })
            .when(!down, |menu| {
                menu.bottom(px(surfaces::ICON_BUTTON + 4.)).left_0()
            })
            .w(px(250.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(item(
                "remember-row",
                "memory",
                tr("Save to memory"),
                true,
                |this, index, window, cx| this.remember_row(index, window, cx),
                cx,
            ))
            .child(item(
                "quote-row",
                "square-pen",
                tr("Quote in the composer"),
                true,
                |this, index, window, cx| this.quote_row(index, window, cx),
                cx,
            ));
        // Quote / reuse only reads the turns, so it is offered in a
        // read-only conversation too, on the user's messages.
        if crate::reuse::offered(&self.rows, index, self.reuse_acp(), running) {
            let start = self
                .reuse_start()
                .and_then(|selection| selection.row(&self.rows));
            let (name, tooltip) = crate::reuse::action(start.is_some());
            menu = menu.child(item(
                "reuse-row",
                name,
                tooltip,
                !self.reuse_loading,
                |this, index, window, cx| this.reuse_turn(index, window, cx),
                cx,
            ));
        }
        match self.rows.get(index).map(|row| row.role.as_str()) {
            Some("user") if !archived && offered.edit => menu.child(item(
                "edit-row",
                "edit",
                tr("Edit and send again"),
                idle,
                |this, index, _, cx| this.ask_rewind(index, cx),
                cx,
            )),
            Some("assistant") if controls::turn_of(&self.rows, index).is_some() => menu
                .when(offered.branch, |menu| {
                    menu.child(item(
                        "branch-row",
                        "branch",
                        tr("Branch from here"),
                        idle,
                        |this, index, window, cx| this.branch_from(index, window, cx),
                        cx,
                    ))
                })
                .when(offered.review, |menu| {
                    menu.child(item(
                        "review-row",
                        "review",
                        tr("Review this turn"),
                        idle,
                        |this, index, window, cx| this.open_review_of(index, window, cx),
                        cx,
                    ))
                })
                .when(self.can_explore(), |menu| {
                    menu.child(item(
                        "explore-row",
                        "compass",
                        tr("Start an exploration from here"),
                        idle,
                        |this, index, window, cx| this.ask_explore_from(index, window, cx),
                        cx,
                    ))
                }),
            _ => menu,
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
                    .child(div().child(if waiting {
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
                    .child(div().flex_shrink_0().child(label))
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
    /// The working plan as the model last updated it, step by step.
    fn progress_card(
        &self,
        steps: &[wisp_dto::execution_plan::PlanStep],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use wisp_dto::execution_plan::PlanStatus;
        let muted = cx.theme().muted_foreground;
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
                    .text_sm()
                    .child(icon("plan").text_color(muted))
                    .child(div().child(tr("Plan"))),
            );
        for step in steps {
            let (glyph_name, color) = match step.status {
                PlanStatus::Done => ("circle-check", cx.theme().success),
                PlanStatus::Running => ("loader", cx.theme().info),
                PlanStatus::Pending => ("circle", muted),
                PlanStatus::Cancelled => ("circle-minus", muted),
            };
            let mut text = div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .child(step.content.clone());
            if matches!(step.status, PlanStatus::Cancelled | PlanStatus::Done) {
                text = text.text_color(muted);
            }
            card = card.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_2()
                    .child(icon(glyph_name).mt(px(3.)).text_color(color))
                    .child(text),
            );
        }
        div().w_full().px_6().py_1().child(card).into_any_element()
    }

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
                    .child(div().child(if waiting {
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

    /// One transcript row as part of the record (`surfaces::entry_row`): a
    /// message of the user's opens an entry, with its number and time in the
    /// gutter and its dot on the record line. The cards of runs no row names
    /// are placed with it: above a message of yours sent after the run was
    /// made, or below the last row.
    pub(super) fn row(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = self.row_body(index, window, cx);
        let number = present::entry_number(&self.rows, index);
        let time = number
            .and_then(|_| self.rows.get(index)?.timestamp)
            .map(|at| crate::home::clock(unix_now(), at))
            .filter(|time| !time.is_empty())
            .map(SharedString::from);
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let dot = match number {
            None => surfaces::EntryDot::None,
            Some(_) if running && present::latest_entry(&self.rows) == Some(index) => {
                surfaces::EntryDot::Live
            }
            Some(_) => surfaces::EntryDot::Open,
        };
        if !self.own_run_cards.contains_key(&index) {
            return surfaces::entry_row(number, time, dot, body, cx).into_any_element();
        }
        let (above, below) = self.own_run_cards_at(index, window, cx);
        let above = (!above.is_empty()).then(|| {
            surfaces::entry_row(
                None,
                None,
                surfaces::EntryDot::None,
                div().w_full().v_flex().children(above).into_any_element(),
                cx,
            )
        });
        div()
            .w_full()
            .v_flex()
            .children(above)
            .child(surfaces::entry_row(
                number,
                time,
                dot,
                div()
                    .w_full()
                    .v_flex()
                    .child(body)
                    .children(below)
                    .into_any_element(),
                cx,
            ))
            .into_any_element()
    }

    /// Whether row `index` is drawn as a record row: a tool call (not a
    /// question, plan or agents' card), a thought, a review or a branch
    /// brought back. Consecutive ones read as one group.
    fn is_record_row(&self, index: usize) -> bool {
        let Some(item) = self.rows.get(index) else {
            return false;
        };
        match item.role.as_str() {
            "tool" => {
                controls::question(item).is_none()
                    && controls::plan_steps(item).is_none()
                    && controls::execution_steps(item).is_none()
                    && !self.agent_cards.contains_key(&index)
            }
            "reasoning" => !item.text.trim().is_empty(),
            "review" => wisp_server::desktop::Review::from_report_json(&item.text).is_some(),
            "branch_merge" => true,
            _ => false,
        }
    }

    /// The row of a call that started or watched a run: the run's title,
    /// where it ran after it, the run's own state as a stamp (live while it
    /// goes on), how long it has run as the value, and two icon buttons:
    /// ask aside about the run, and open it in the Runs tab. The rest of the
    /// run's card is what the row opens to.
    fn run_record(
        &self,
        index: usize,
        run: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) -> surfaces::RecordRow {
        use surfaces::{IconTone, StampKind};
        let item = self.rows.get(index);
        let tool_ms = item.and_then(|row| row.duration_ms);
        let record = self
            .state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.run_cards.iter().find(|card| card.id == run));
        let row = match record {
            Some(card) => self.run_record_row(key, card, tool_ms, cx),
            // The state the row was saved with, when the record is not
            // there, and the call's own time.
            None => {
                let (state, tone) = present::run_state(
                    item.and_then(|row| row.run.as_ref())
                        .map_or("", |run| run.status.as_str()),
                    None,
                );
                let mut row = surfaces::record_row(
                    SharedString::from(format!("run-row-{key}")),
                    item.map(present::heading)
                        .unwrap_or_else(|| tr("Run").to_owned()),
                )
                .icon("flask")
                .value_column();
                if let Some(ms) = tool_ms {
                    row = row.value(present::duration_label(ms));
                }
                if let Some(kind) = StampKind::from_tone(tone) {
                    row = row.stamp(kind, state);
                }
                row
            }
        };
        let shown = run.to_owned();
        row.action(
            surfaces::icon_action(
                SharedString::from(format!("run-row-show-{key}")),
                "external-link",
                tr("The run's whole output, log and clean-up in the Runs tab"),
                IconTone::Faded,
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.expanded_run = Some(shown.clone());
                this.show_panel_tab(PanelTab::Runs, window, cx);
            })),
        )
    }

    /// How far a row of the user's stands below what comes before it: the
    /// space between entries (28 px), less the row of actions an answer
    /// already ends on; the first row stands close to the top.
    fn entry_gap(&self, index: usize) -> Pixels {
        let Some(before) = index.checked_sub(1).and_then(|at| self.rows.get(at)) else {
            return px(8.);
        };
        let answer = !matches!(
            before.role.as_str(),
            "user" | "tool" | "reasoning" | "review" | "branch_merge" | "compaction"
        ) && !before.text.is_empty();
        if answer { px(4.) } else { px(28.) }
    }

    /// A record row in the transcript, inset as the answers are; the first
    /// of a run of them carries the group's hairline above it.
    fn transcript_record(
        &self,
        index: usize,
        record: surfaces::RecordRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let first = index == 0 || !self.is_record_row(index - 1);
        let row = if first {
            surfaces::record_group([record], cx).into_any_element()
        } else {
            record.into_any_element()
        };
        // The row's own inset (4 px) puts its icon on the answers' text
        // edge; a group stands 8 px below what comes before it.
        div()
            .w_full()
            .pl(px(20.))
            .pr_6()
            .when(first, |row| row.pt_2())
            .child(row)
            .into_any_element()
    }

    /// One transcript row. Your messages are the questions that open the
    /// entries, set as titles on the paper; Wisp's answers are plain text;
    /// tool calls, thoughts, reviews and branches brought back are record
    /// rows that open to what they hold.
    fn row_body(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        use surfaces::{IconSize, IconTone, StampKind};
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
                // Its actions lie over its top right corner, so they take
                // no height under the question.
                let mut message = div()
                    .group(MESSAGE)
                    .relative()
                    .w_full()
                    .v_flex()
                    .items_start()
                    .gap_1()
                    .px_6()
                    .pt(self.entry_gap(index));
                if !text.is_empty() {
                    // A question past three lines is folded until opened.
                    let key = format!("question-{}", present::tool_key(item, index));
                    let long = present::long_question(&text);
                    let open = self.open_tools.contains(&key);
                    message = message.child(
                        surfaces::question(present::title_question(&text), long && !open)
                            .w_full()
                            .child(TextView::markdown(id, text, window, cx).selectable(true)),
                    );
                    if long {
                        message = message.child(
                            surfaces::labelled_action(
                                ("question-fold", index),
                                if open { "chevron-up" } else { "chevron-down" },
                                if open {
                                    tr("Show less")
                                } else {
                                    tr("Show all")
                                },
                                IconSize::Inline,
                                IconTone::Faded,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.toggle_tool(key.clone(), index, cx),
                            )),
                        );
                    }
                }
                if !item.attachments.is_empty() {
                    // The files the message carried, by name; the full path
                    // is in the copied text.
                    let mut files = div().h_flex().flex_wrap().gap_1();
                    for path in &item.attachments {
                        files = files.child(
                            Self::chip_box(cx)
                                .child(icon("doc").text_color(cx.theme().muted_foreground))
                                .child(crate::files::file_name(path).to_owned()),
                        );
                    }
                    message = message.child(files);
                }
                // The conversations and skills it was sent with from here.
                let referenced = self
                    .state
                    .session
                    .as_ref()
                    .and_then(|session| self.sent_references.get(session))
                    .map_or(&[][..], |sent| compose::sent_with(sent, &self.rows, index));
                if !referenced.is_empty() {
                    let mut chips = div().h_flex().flex_wrap().gap_1();
                    for chip in referenced {
                        chips = chips.child(
                            Self::chip_box(cx)
                                .child(icon(chip.icon()).text_color(cx.theme().muted_foreground))
                                .child(chip.label()),
                        );
                    }
                    message = message.child(chips);
                }
                message
                    .child(
                        self.message_actions(index, true, cx)
                            .absolute()
                            .top(self.entry_gap(index))
                            .right_6(),
                    )
                    .into_any_element()
            }
            "tool" => {
                if let Some(question) = controls::question(item) {
                    return self.question_card(index, question, id, window, cx);
                }
                if let Some(steps) = controls::plan_steps(item) {
                    return self.plan_card(index, steps, cx);
                }
                if let Some(steps) = controls::execution_steps(item) {
                    return self.progress_card(steps, cx);
                }
                // A call that handed work to other agents, once that work
                // is known: a card in place of the tool row.
                if let Some(card) = self.agent_cards.get(&index) {
                    return self.agent_card(index, card, window, cx);
                }
                let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
                let key = present::tool_key(item, index);
                let open = self.open_tools.contains(&key);
                let toggle = key.clone();
                // A call that started or watched a run is that run's row.
                if let Some((run, _)) = self.run_card_rows.get(&index) {
                    let record = self.run_record(index, run, &key, cx);
                    let below = open.then(|| self.run_card(Some(index), run, true, window, cx));
                    let record = record.below(below).cursor_pointer().on_click(cx.listener(
                        move |this, _, _, cx| this.toggle_tool(toggle.clone(), index, cx),
                    ));
                    return self.transcript_record(index, record, cx);
                }
                // A built-in tool is named by an identifier, set as one; what
                // it works on, when its input says, leads.
                let name = present::heading(item);
                let mut record = match present::tool_summary(item) {
                    Some(summary) => {
                        surfaces::record_row(SharedString::from(format!("tool-{key}")), summary)
                            .lead(name)
                            .mono()
                    }
                    None if item.kind.is_some() => {
                        // An ACP agent's call is named by its title.
                        surfaces::record_row(SharedString::from(format!("tool-{key}")), name)
                    }
                    None => {
                        surfaces::record_row(SharedString::from(format!("tool-{key}")), name).mono()
                    }
                }
                .icon(present::tool_icon(item))
                .value_column();
                if let Some(ms) = item.duration_ms {
                    record = record.value(present::duration_label(ms));
                }
                if let Some((word, tone)) = present::tool_outcome(item, running)
                    && let Some(kind) = StampKind::from_tone(tone)
                {
                    record = record.stamp(kind, word);
                }
                // The call's input and output while it is open.
                let below = open.then(|| {
                    TextView::markdown(id, present::body(item), window, cx)
                        .selectable(true)
                        .into_any_element()
                });
                let record = record.below(below).cursor_pointer().on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_tool(toggle.clone(), index, cx)),
                );
                self.transcript_record(index, record, cx)
            }
            "reasoning" if item.text.trim().is_empty() => div().into_any_element(),
            // What the model thought before answering, folded like a tool
            // call so the answer stays the thing that is read.
            "reasoning" => {
                let key = present::tool_key(item, index);
                let open = self.open_tools.contains(&key);
                let below = open.then(|| {
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            TextView::markdown(id, item.text.clone(), window, cx).selectable(true),
                        )
                        .into_any_element()
                });
                let record = surfaces::record_row(
                    SharedString::from(format!("thought-{key}")),
                    tr("Thought"),
                )
                .icon("brain")
                .value_column()
                .below(below)
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_tool(key.clone(), index, cx)),
                );
                self.transcript_record(index, record, cx)
            }
            // A review kept with the conversation, after what it read: what
            // it found in a line, and the whole report a click away.
            "review" => match wisp_server::desktop::Review::from_report_json(&item.text) {
                None => div().into_any_element(),
                Some(found) => {
                    let (said, tone) = crate::review::outcome(&found);
                    let mut record = surfaces::record_row(
                        SharedString::from(format!("review-row-see-{index}")),
                        found.summary.clone(),
                    )
                    .icon("review")
                    .value_column();
                    if let Some(kind) = StampKind::from_tone(tone) {
                        record = record.stamp(kind, said);
                    } else {
                        record = record.lead(said);
                    }
                    let record = record
                        .cursor_pointer()
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new(tr("See")).build(window, cx)
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.see_review(found.clone(), cx)),
                        );
                    self.transcript_record(index, record, cx)
                }
            },
            // A branch's summary brought back here: a row naming the branch,
            // opening to what was brought back, as in the WebView.
            "branch_merge" => {
                let key = present::tool_key(item, index);
                let open = self.open_tools.contains(&key);
                let title = item.tool_name.clone().unwrap_or_default();
                let below = open.then(|| {
                    let mut below = div().v_flex().gap_2().child(
                        TextView::markdown(id, item.text.clone(), window, cx).selectable(true),
                    );
                    if let (Some(project), Some(branch)) =
                        (self.state.project.clone(), item.input.clone())
                    {
                        below = below.child(
                            div().h_flex().child(
                                Button::new(SharedString::from(format!("merged-open-{index}")))
                                    .icon(glyph("branch"))
                                    .label(tr("Open the branch"))
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_session(
                                            project.clone(),
                                            branch.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                        );
                    }
                    below.into_any_element()
                });
                let record =
                    surfaces::record_row(SharedString::from(format!("merged-{key}")), title)
                        .icon("branch")
                        .lead(tr("Merged branch result"))
                        .value_column()
                        .below(below)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_tool(key.clone(), index, cx)
                        }));
                self.transcript_record(index, record, cx)
            }
            // Where the conversation was compacted, as the host placed it.
            "compaction" => match crate::compaction::card(item) {
                Some(card) => self.compaction_row(index, card, window, cx),
                None => div().into_any_element(),
            },
            _ if item.text.is_empty() => div().into_any_element(),
            _ => {
                // An answer the entry goes on after (a tool call, a thought)
                // lays its actions over its corner: a line of them would
                // stand between the text and what follows. The entry's
                // last answer keeps them under it.
                let ends_entry = self
                    .rows
                    .get(index + 1)
                    .is_none_or(|next| next.role == "user");
                let actions = self.message_actions(index, !ends_entry, cx);
                let actions = if ends_entry {
                    actions
                } else {
                    actions.absolute().top_2().right_6()
                };
                div()
                    .group(MESSAGE)
                    .relative()
                    .w_full()
                    .v_flex()
                    .gap_1()
                    .px_6()
                    .pt_2()
                    .child(TextView::markdown(id, item.text.clone(), window, cx).selectable(true))
                    .children(self.inline_pictures(&item.text, cx))
                    .children(self.written_pictures(index, cx))
                    .children(self.generated_section(index, cx))
                    .child(actions)
                    .into_any_element()
            }
        }
    }

    fn render_approvals(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let snapshot = self.state.snapshot.as_ref()?;
        if snapshot.approvals.is_empty() {
            return None;
        }
        let attention = tone_color(Tone::Attention, cx);
        // Several at once scroll here, so they never push the transcript
        // out of the window.
        let mut approvals = div()
            .id("approvals")
            .v_flex()
            .flex_shrink_0()
            .gap_2()
            .max_h(px(260.))
            .overflow_y_scroll();
        for approval in &snapshot.approvals {
            let id = approval.approval_id.clone();
            if let Some(steps) = controls::plan_approval(&approval.message) {
                approvals = approvals.child(self.plan_approval_card(id, steps, cx));
                continue;
            }
            // A picture too large for the model is asked about on its own,
            // as the WebView does: only for this once, never remembered.
            if let Some(picture) = controls::image_resize(&approval.message) {
                let (resize, cancel) = (id.clone(), id.clone());
                approvals = approvals.child(
                    surfaces::bar(SharedString::from(format!("approval-{id}")), cx)
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(icon("image").text_color(attention))
                                .child(div().child(tr("Resize large image for model input?"))),
                        )
                        .child(div().text_sm().truncate().child(picture.to_owned()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("Wisp will resize a temporary copy for the model. The \
                                     original file will not be changed.")),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new(SharedString::from(format!("resize-{resize}")))
                                        .icon(glyph("check"))
                                        .label(tr("Resize and continue"))
                                        .small()
                                        .primary()
                                        .disabled(self.pending)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.answer_approval(
                                                resize.clone(),
                                                true,
                                                Remember::No,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "resize-cancel-{cancel}"
                                    )))
                                    .label(tr("Cancel"))
                                    .small()
                                    .disabled(self.pending)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.answer_approval(
                                            cancel.clone(),
                                            false,
                                            Remember::No,
                                            window,
                                            cx,
                                        )
                                    })),
                                ),
                        ),
                );
                continue;
            }
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
                    )
                    .child(
                        answer(
                            tr("Always, in every project"),
                            "layers",
                            true,
                            Remember::Everywhere,
                        )
                        .tooltip(tr("Take it back in Settings, under Permissions")),
                    );
            }
            buttons = buttons.child(answer(tr("Deny"), "ban", false, Remember::No));
            approvals = approvals.child(
                surfaces::bar(SharedString::from(format!("approval-{id}")), cx)
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(icon("shield").text_color(attention))
                            .child(div().child(crate::i18n::fill(
                                tr("{} needs your approval"),
                                &[&approval.tool],
                            ))),
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

    /// A plan the model asks to start on: its steps, to approve, to turn
    /// down, or to send back with what should change.
    fn plan_approval_card(
        &self,
        id: String,
        steps: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let attention = tone_color(Tone::Attention, cx);
        let mut card = surfaces::bar(SharedString::from(format!("approval-{id}")), cx)
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(icon("plan").text_color(attention))
                    .child(div().child(tr("Wisp proposes a plan — approve it to start"))),
            );
        for (number, step) in steps.iter().enumerate() {
            card = card.child(div().text_sm().child(format!("{}. {step}", number + 1)));
        }
        let approve = id.clone();
        let refuse = id.clone();
        let feedback = id;
        let ready = !self.plan_feedback.read(cx).value().trim().is_empty();
        card.child(Input::new(&self.plan_feedback).small()).child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("plan-approve-{approve}")))
                        .icon(glyph("check"))
                        .label(tr("Approve"))
                        .small()
                        .primary()
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.answer_approval(
                                approve.clone(),
                                true,
                                wisp_server::desktop::Remember::No,
                                window,
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-feedback-{feedback}")))
                        .icon(glyph("edit"))
                        .label(tr("Send feedback"))
                        .small()
                        .disabled(self.pending || !ready)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.send_plan_feedback(feedback.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plan-refuse-{refuse}")))
                        .icon(glyph("ban"))
                        .label(tr("Turn down"))
                        .small()
                        .ghost()
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.answer_approval(
                                refuse.clone(),
                                false,
                                wisp_server::desktop::Remember::No,
                                window,
                                cx,
                            )
                        })),
                ),
        )
    }

    /// The card a composer menu's rows sit in, drawn just above the button
    /// that opens it.
    pub(super) fn composer_menu(width: f32, cx: &App) -> Div {
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

    /// One item of a menu: its icon and its word, the icon faded until the
    /// pointer is over the item. The caller adds what a click does, or
    /// dims it.
    pub(super) fn menu_item(
        id: impl Into<ElementId>,
        icon_name: &str,
        label: impl Into<SharedString>,
        cx: &App,
    ) -> Stateful<Div> {
        let hover = cx.theme().list_hover;
        div()
            .id(id)
            .group(surfaces::ICON_HOVER)
            .h_flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(30.))
            .rounded(cx.theme().radius)
            .text_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(surfaces::sized_icon(
                icon_name,
                surfaces::IconSize::Inline,
                surfaces::IconTone::Faded,
                cx,
            ))
            .child(div().flex_1().min_w_0().truncate().child(label.into()))
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

    /// The ACP agent's modes and options, each a button with its list or,
    /// for one that is on or off, a switch.
    fn acp_settings(&self, cx: &mut Context<Self>) -> Vec<Div> {
        use wisp_server::desktop::{AcpPick, AcpValue};
        let Some(controls) = self.status.as_ref().and_then(|status| status.acp.as_ref()) else {
            return vec![];
        };
        let mut shown = Vec::new();
        for setting in controls.mode.iter().chain(&controls.options) {
            let id = setting.id.clone();
            let name = if id == "mode" && controls.mode.as_ref() == Some(setting) {
                tr("Mode").to_owned()
            } else {
                setting.name.clone()
            };
            // A segment of the conditions line: a flag is on (teal) or off,
            // a choice shows what is chosen; the tooltip names the setting.
            let button = match &setting.value {
                AcpValue::Flag(on) => {
                    let (target, on) = (id.clone(), *on);
                    surfaces::condition(
                        SharedString::from(format!("acp-setting-{id}")),
                        if on { "check" } else { "square" },
                        name.clone(),
                        name,
                        if on {
                            surfaces::IconTone::On
                        } else {
                            surfaces::IconTone::Faded
                        },
                        cx,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_acp_setting(target.clone(), AcpPick::Flag(!on), window, cx)
                    }))
                }
                AcpValue::Choice { current, choices } => {
                    let label = choices
                        .iter()
                        .find(|choice| &choice.id == current)
                        .map(|choice| choice.name.clone())
                        .filter(|label| !label.is_empty())
                        .unwrap_or_else(|| current.clone());
                    let target = id.clone();
                    surfaces::condition(
                        SharedString::from(format!("acp-setting-{id}")),
                        if setting.id == "mode" {
                            "controls"
                        } else {
                            "adjustments"
                        },
                        label,
                        name,
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_acp_setting(target.clone(), window, cx)
                    }))
                }
            };
            let mut place = div().relative().child(button);
            let open = self.layers.contains(Layer::AcpSettingMenu)
                && self.acp_setting_open.as_deref() == Some(setting.id.as_str());
            if let (true, AcpValue::Choice { current, choices }) = (open, &setting.value) {
                let mut menu = Self::composer_menu(280., cx);
                for choice in choices {
                    let (target, value) = (setting.id.clone(), choice.id.clone());
                    menu = menu.child(
                        Self::menu_row(
                            SharedString::from(format!("acp-choice-{}-{}", setting.id, choice.id)),
                            if choice.name.is_empty() {
                                choice.id.clone()
                            } else {
                                choice.name.clone()
                            },
                            choice.description.clone(),
                            &choice.id == current,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.set_acp_setting(
                                    target.clone(),
                                    AcpPick::Choice(value.clone()),
                                    window,
                                    cx,
                                )
                            },
                        )),
                    );
                }
                place = place.child(deferred(menu));
            }
            shown.push(place);
        }
        shown
    }

    /// Wisp, then the ACP agents that could run a new conversation.
    fn agent_menu(&self, cx: &mut Context<Self>) -> Div {
        let mut menu = Self::composer_menu(320., cx).child(
            Self::menu_row(
                "agent-wisp".into(),
                "Wisp",
                tr("Wisp's own model, tools and skills"),
                true,
                cx,
            )
            .on_click(
                cx.listener(|this, _, window, cx| this.choose_acp_agent(String::new(), window, cx)),
            ),
        );
        for row in self.acp_agents.iter().flatten() {
            let id = row.agent.id.clone();
            let detail = if row.found {
                crate::acp::command_line(&row.agent.command, &row.agent.args)
            } else {
                tr("Program not found").to_owned()
            };
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("agent-choice-{id}")),
                    row.agent.label.clone(),
                    detail,
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_acp_agent(id.clone(), window, cx)
                })),
            );
        }
        menu
    }

    /// The saved models, with "follow the default" first.
    fn model_menu(&self, current: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        // Two lists of every model: it scrolls rather than leave the window.
        let mut menu = Self::composer_menu(300., cx)
            .id("model-menu")
            .max_h(px(440.))
            .overflow_y_scroll();
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
        // What this conversation's sub-agents work on.
        let theirs = self.options().subagent_model;
        let choose = |choice: Option<String>| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                this.choose_subagent_model(choice.clone(), window, cx)
            })
        };
        menu = menu
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("Sub-agents work on")),
            )
            .child(
                Self::menu_row(
                    "subagent-settings".into(),
                    tr("As in Settings"),
                    "",
                    theirs.is_none(),
                    cx,
                )
                .on_click(choose(None)),
            )
            .child(
                Self::menu_row(
                    "subagent-own".into(),
                    tr("This conversation's model"),
                    "",
                    theirs.as_deref() == Some(""),
                    cx,
                )
                .on_click(choose(Some(String::new()))),
            );
        for profile in &models.profiles {
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("subagent-choice-{}", profile.id)),
                    profile.name.clone(),
                    profile.fields.model.clone(),
                    theirs.as_deref() == Some(profile.id.as_str()),
                    cx,
                )
                .on_click(choose(Some(profile.id.clone()))),
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
    pub(super) fn outline_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("Earlier messages are listed once they are loaded.")),
                    )
                    .child(
                        Button::new("outline-load-all")
                            .label(if self.loading_history {
                                tr("Loading…")
                            } else {
                                tr("Load earlier")
                            })
                            .xsmall()
                            .disabled(self.loading_history)
                            .tooltip(tr("Load the whole conversation, so all of it is listed"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_whole_history(window, cx)
                            })),
                    ),
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
    pub(super) fn chip_box(cx: &App) -> Div {
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
        type Toggle = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
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
        let started = !self.rows.is_empty() || running;
        let by_agent = !options.acp_agent.is_empty();
        let fast_offered = self
            .status
            .as_ref()
            .is_some_and(|status| status.fast_available);

        // The `+` button at the left end: what can be put into the message,
        // and the modes that change what Wisp does, in one menu. The menus
        // it leads to (quick actions, the `/` list, also opened by typing
        // `/`) open above it.
        let plus_open = self.layers.contains(Layer::PlusMenu);
        let mut plus = div().relative().flex_none().child(
            surfaces::icon_action(
                "composer-plus",
                "plus",
                tr("Put something into the message, or switch a mode on"),
                if plus_open {
                    surfaces::IconTone::Ink
                } else {
                    surfaces::IconTone::Faded
                },
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_composer_menu(Layer::PlusMenu, window, cx)
            })),
        );
        if plus_open {
            // Painted after everything else: a parent's border is drawn over
            // its own children, and the composer box has one.
            plus = plus.child(deferred(self.plus_menu(by_agent, fast_offered, cx)));
        }
        if let Some(menu) = self.slash_menu(cx) {
            plus = plus.child(deferred(menu));
        }
        if self.layers.contains(Layer::QuickMenu) {
            let mut menu = Self::composer_menu(320., cx);
            for action in &self.quick_actions {
                let (prompt, id) = (action.prompt.clone(), action.id.clone());
                let starts = !action.workflow.is_empty();
                menu = menu.child(
                    Self::menu_row(
                        SharedString::from(format!("quick-choice-{}", action.id)),
                        action.name.clone(),
                        if starts {
                            tr("Starts a workflow on what is typed")
                        } else {
                            ""
                        },
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if starts {
                            this.start_quick_action(id.clone(), window, cx)
                        } else {
                            this.use_quick_action(prompt.clone(), window, cx)
                        }
                    })),
                );
            }
            plus = plus.child(deferred(menu));
        }

        // The conditions line: one segment for each condition, its icon and
        // its value; a click opens the picker it had before.
        let mut conditions: Vec<AnyElement> = Vec::new();
        // Who runs the conversation: Wisp, or an ACP agent chosen before
        // the first message and named for good afterwards.
        let agents = self.acp_agents.as_deref().unwrap_or_default();
        if by_agent || (!agents.is_empty() && !started) {
            let open = self.layers.contains(Layer::AgentMenu);
            let segment = surfaces::condition(
                "acp-agent",
                "swarm",
                if by_agent {
                    crate::acp::agent_label(agents, &options.acp_agent)
                } else {
                    "Wisp".to_owned()
                },
                if by_agent {
                    tr("An ACP agent runs this conversation; start a new one for Wisp")
                } else {
                    tr("Who runs this conversation: Wisp or an ACP agent")
                },
                surfaces::IconTone::Faded,
                cx,
            )
            .when(!(by_agent || started), |segment| {
                segment
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        // One may have been added or removed since.
                        this.load_acp_agents(window, cx);
                        this.toggle_composer_menu(Layer::AgentMenu, window, cx)
                    }))
            });
            let mut place = div().relative().child(segment);
            if open {
                place = place.child(deferred(self.agent_menu(cx)));
            }
            conditions.push(place.into_any_element());
        }
        conditions.extend(
            self.acp_settings(cx)
                .into_iter()
                .map(IntoElement::into_any_element),
        );
        if !by_agent {
            let model_open = self.layers.contains(Layer::ModelMenu);
            let mut model = div().relative().child(
                surfaces::condition(
                    "model",
                    "bot",
                    settings::model_label(self.settings.current(), &options.model),
                    tr("The model this conversation runs on, from the next message"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::ModelMenu, window, cx)
                })),
            );
            if model_open {
                model = model.child(deferred(self.model_menu(&options.model, cx)));
            }
            conditions.push(model.into_any_element());
            let effort_open = self.layers.contains(Layer::EffortMenu);
            let mut effort = div().relative().child(
                surfaces::condition(
                    "effort",
                    "gauge",
                    controls::effort_label(&options.reasoning_effort),
                    tr("How hard the model thinks, from the next message"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::EffortMenu, window, cx)
                })),
            );
            if effort_open {
                effort = effort.child(deferred(self.effort_menu(&options.reasoning_effort, cx)));
            }
            conditions.push(effort.into_any_element());
            // Offered once there is a host to choose, or a choice to undo.
            let hosts = self.environments.as_ref();
            if hosts.is_some_and(|view| !view.hosts.is_empty()) || !options.runs_on.is_empty() {
                let default = hosts.and_then(|view| view.default.as_deref());
                let open = self.layers.contains(Layer::RunsOnMenu);
                let mut place = div().relative().child(
                    surfaces::condition(
                        "runs-on",
                        settings::runs_on_icon(&options.runs_on, default),
                        settings::runs_on_label(&options.runs_on, default),
                        tr("Where this conversation's runs start, from the next message"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        // A host may have been added or checked since.
                        this.load_environments(window, cx);
                        this.toggle_composer_menu(Layer::RunsOnMenu, window, cx)
                    })),
                );
                if open {
                    place = place.child(deferred(self.runs_on_menu(&options.runs_on, cx)));
                }
                conditions.push(place.into_any_element());
            }
            if !self.experts.is_empty() && (!started || !options.expert.is_empty()) {
                let open = self.layers.contains(Layer::ExpertMenu);
                let mut place = div().relative().child(
                    surfaces::condition(
                        "expert",
                        "user",
                        settings::expert_label(&self.experts, &options.expert),
                        if started {
                            tr("The expert this conversation was started as")
                        } else {
                            tr("Start this conversation as an expert")
                        },
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .when(!started, |segment| {
                        segment
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                // One may have been added or removed since.
                                this.load_experts(window, cx);
                                this.toggle_composer_menu(Layer::ExpertMenu, window, cx)
                            }))
                    }),
                );
                if open {
                    place = place.child(deferred(self.expert_menu(&options.expert, cx)));
                }
                conditions.push(place.into_any_element());
            }
        }
        // A mode that changes what Wisp does shows only while it is on, in
        // teal; a click turns it off. At its usual value it is in `+`.
        for (on, id, glyph_name, word, tooltip, toggle) in [
            (
                planning && !by_agent,
                "plan-first",
                "plan",
                tr("Planning"),
                tr("Wisp investigates and proposes a plan; it changes nothing until you approve"),
                (|this, window, cx| this.toggle_plan_first(window, cx)) as Toggle,
            ),
            (
                full,
                "access",
                "shield",
                tr("Full access"),
                tr("Wisp changes files and runs commands without asking"),
                |this, window, cx| this.toggle_full_access(window, cx),
            ),
            (
                options.auto_review && !by_agent,
                "auto-review",
                "review",
                tr("Reviewing"),
                tr("Each turn that used a tool is reviewed when it ends"),
                |this, window, cx| this.toggle_auto_review(window, cx),
            ),
            (
                options.fast && fast_offered,
                "fast",
                "bolt",
                tr("Fast"),
                tr(
                    "Fast is on: requests ask for priority processing, which may use more quota. Click to turn it off from the next message",
                ),
                |this, window, cx| this.toggle_fast(window, cx),
            ),
        ] {
            if on {
                conditions.push(
                    surfaces::condition(id, glyph_name, word, tooltip, surfaces::IconTone::On, cx)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| toggle(this, window, cx)))
                        .into_any_element(),
                );
            }
        }

        // The right end: how full the context is, then send, or stop while
        // a turn runs.
        let mut actions = div().h_flex().flex_none().items_center().gap_2();
        let usage = self
            .status
            .as_ref()
            .and_then(|status| status.usage.as_ref());
        if let Some(percent) = usage.and_then(controls::context_percent) {
            let warning = surfaces::stamp_color(surfaces::StampKind::Waiting, cx);
            actions = actions.child(
                div()
                    .id("context-meter")
                    .cursor_pointer()
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(crate::i18n::fill(
                            tr("Context {}% full. Click to see what the model is sent"),
                            &[&percent],
                        ))
                        .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_model_view(window, cx)))
                    .child(
                        surfaces::fact(format!("{percent}%"), cx)
                            .when(percent >= 90, |meter| meter.text_color(warning))
                            .hover(|meter| meter.text_color(cx.theme().foreground)),
                    ),
            );
        }
        // While a turn runs, what is written waits for it to end.
        let can_queue = running
            && !self.pending
            && !self.loading_history
            && !sendable.trim().is_empty()
            && self.state.snapshot.as_ref().is_some_and(|s| !s.read_only);
        if running {
            if can_queue {
                actions = actions.child(
                    surfaces::icon_action(
                        "queue",
                        "clock",
                        tr("Send it when this turn ends"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                );
            }
            actions = actions.child(
                surfaces::send_button("stop", "stop", tr("Stop"), cx)
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.control("native_conversation_stop", json!({}), window, cx)
                    })),
            );
        } else {
            actions = actions.child(
                surfaces::send_button(
                    "send",
                    "arrow-up",
                    crate::i18n::fill(tr("Send ({})"), &[&self.send_key.label()]),
                    cx,
                )
                .disabled(!can_send)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            );
        }

        // `px_4` is the inset `surfaces::above_composer` shares, so the bars
        // there line up with the box below.
        let mut area = div().v_flex().flex_shrink_0().gap_2().px_4().pb_4().pt_1();
        if let Some(console) = self.render_console(cx) {
            area = area.child(console);
        }
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
        // What the turn uses so far, while it runs; once it is saved the
        // entry's own actions say it.
        let usage = usage.filter(|_| running);
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
                            .child(controls::runtime_chip_label(row))
                            .tooltip({
                                let detail = controls::runtime_detail(row, unix_now());
                                move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(detail.clone())
                                        .build(window, cx)
                                }
                            }),
                    );
                {
                    let id = row.id.clone();
                    chip = chip.child(
                        Button::new(SharedString::from(format!("runtime-console-{}", row.id)))
                            .label(tr("Console"))
                            .xsmall()
                            .ghost()
                            .tooltip(tr("Its variables, and code of your own to run in it"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_console(id.clone(), window, cx)
                            })),
                    );
                }
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
                line = line.child(surfaces::fact(controls::usage_line(usage), cx));
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
                surfaces::bar("thinking", cx)
                    .v_flex()
                    .gap_1()
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
        // Where the working plan stands, while it has steps to go.
        if let Some(progress) = controls::plan_progress(&self.rows) {
            let mut line = div()
                .h_flex()
                .gap_2()
                .px_1()
                .text_xs()
                .text_color(muted)
                .child(icon("plan").text_color(muted))
                .child(crate::i18n::fill(
                    tr("Plan: {} of {} done"),
                    &[&progress.done, &progress.total],
                ));
            if let Some(current) = progress.current {
                line = line.child(div().flex_1().min_w_0().truncate().child(current));
            }
            area = area.child(line);
        }
        if let Some(strip) = self.queue_strip(running, cx) {
            area = area.child(strip);
        }
        if let Some(strip) = self.timer_strip(cx) {
            area = area.child(strip);
        }
        // A conversation that cannot be written to says why in place of
        // the box, so nothing looks typed into and refused.
        if let Some(why) = self.read_only() {
            return area.child(self.read_only_card(&why, cx));
        }
        if self.layers.contains(Layer::ConfirmFullAccess) {
            area = area.child(
                surfaces::bar("full-access-question", cx)
                    .h_flex()
                    .gap_2()
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
        area.child(
            editor
                .child(
                    // Return sends here when that is the chosen key: the
                    // context lets the composer's binding outrank the
                    // input's own new line, and only in this input.
                    div()
                        .key_context(self.send_key.context())
                        .on_action(cx.listener(|this, _: &super::SendMessage, window, cx| {
                            this.send(window, cx)
                        }))
                        .child(
                            Input::new(&self.composer)
                                .appearance(false)
                                .disabled(self.pending),
                        ),
                )
                .child(
                    // `+` first and send last on the first line; the
                    // conditions between wrap onto a second line where the
                    // column is narrower than they are.
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .child(plus)
                        .child(
                            div()
                                .h_flex()
                                .flex_1()
                                .min_w_0()
                                .min_h(px(surfaces::ICON_BUTTON))
                                .items_center()
                                .flex_wrap()
                                .gap_x(px(surfaces::CONDITION_GAP))
                                .gap_y_1()
                                .children(conditions),
                        )
                        .child(actions),
                ),
        )
    }

    /// The composer's `+` menu: first what can be put into the message
    /// (files, a project file, conversation or skill, a quick action, a
    /// command, workflow or skill from the `/` list), each an icon and a
    /// word with its key at the right; then the modes that change what Wisp
    /// does, each checked while it is on.
    fn plus_menu(&self, by_agent: bool, fast_offered: bool, cx: &mut Context<Self>) -> Div {
        type Run = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
        let options = self.options();
        let line = cx.theme().border;
        let teal = cx.theme().primary;
        let item = |id: &'static str,
                    icon_name: &str,
                    label: &'static str,
                    key: Option<&'static str>,
                    checked: Option<bool>,
                    enabled: bool,
                    run: Run,
                    cx: &mut Context<Self>| {
            let row = Self::menu_item(id, icon_name, label, cx)
                .children(key.map(|key| surfaces::fact(key, cx)))
                .children(
                    checked
                        .filter(|on| *on)
                        .map(|_| icon("check").text_color(teal)),
                );
            if !enabled {
                return row.opacity(0.5).cursor_default();
            }
            row.on_click(cx.listener(move |this, _, window, cx| {
                this.layers.close(Layer::PlusMenu);
                run(this, window, cx);
                cx.notify();
            }))
        };
        let mut menu = Self::composer_menu(300., cx)
            .child(item(
                "attach-files",
                "attach",
                tr("Attach files"),
                None,
                None,
                !self.pending,
                |this, window, cx| this.attach_files(window, cx),
                cx,
            ))
            .child(item(
                "attach-context",
                "link",
                tr("Add a project file, a conversation or a skill"),
                Some("@"),
                None,
                true,
                |this, window, cx| this.toggle_attach_palette(window, cx),
                cx,
            ));
        // Requests kept under a name, to put into the message box.
        if !self.quick_actions.is_empty() {
            menu = menu.child(item(
                "quick-actions",
                "file-play",
                tr("Quick actions"),
                None,
                None,
                true,
                |this, window, cx| {
                    // One may have been added or removed since.
                    this.load_quick_actions(window, cx);
                    this.toggle_composer_menu(Layer::QuickMenu, window, cx)
                },
                cx,
            ));
        }
        menu = menu
            .child(item(
                "slash-commands",
                "skill",
                tr("Commands, workflows and skills"),
                Some("/"),
                None,
                true,
                |this, window, cx| this.open_slash_menu(window, cx),
                cx,
            ))
            .child(div().h(px(1.)).mx_2().my_1().flex_shrink_0().bg(line));
        if !by_agent {
            menu = menu.child(item(
                "plan-first-switch",
                "plan",
                tr("Plan first"),
                None,
                Some(options.plan_first),
                true,
                |this, window, cx| this.toggle_plan_first(window, cx),
                cx,
            ));
        }
        // Turning full access on still asks first.
        menu = menu.child(item(
            "access-switch",
            "shield",
            tr("Full access"),
            None,
            Some(options.full_access),
            true,
            |this, window, cx| this.toggle_full_access(window, cx),
            cx,
        ));
        if !by_agent {
            menu = menu.child(item(
                "auto-review-switch",
                "review",
                tr("Review turns"),
                None,
                Some(options.auto_review),
                true,
                |this, window, cx| this.toggle_auto_review(window, cx),
                cx,
            ));
        }
        if fast_offered {
            menu = menu.child(item(
                "fast-switch",
                "bolt",
                tr("Fast"),
                None,
                Some(options.fast),
                true,
                |this, window, cx| this.toggle_fast(window, cx),
                cx,
            ));
        }
        menu
    }

    /// What stands in the composer's place in a read-only conversation: a
    /// bar with the reason and the one way on.
    fn read_only_card(&self, why: &compose::ReadOnly, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let parent_title = match why {
            compose::ReadOnly::Subagent {
                parent: Some(parent),
            } => self
                .known
                .get(parent)
                .map(|row| row.title.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let action = compose::read_only_action(why).map(|(label, action)| {
            Button::new("read-only-action")
                .label(label)
                .small()
                .on_click(cx.listener(move |this, _, window, cx| match &action {
                    compose::ReadOnlyAction::Open(session) => {
                        if let Some(project) = this.state.project.clone() {
                            this.open_session(project, session.clone(), window, cx);
                        }
                    }
                    // Only an archive that is there: never a new draft.
                    compose::ReadOnlyAction::Archive => {
                        this.open_frozen_research_archive(window, cx)
                    }
                }))
        });
        surfaces::bar("read-only", cx)
            .h_flex()
            .gap_2()
            .child(icon(compose::read_only_icon(why)).text_color(muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(compose::read_only_reason(why, &parent_title)),
            )
            .children(action)
    }

    /// Everything to the right of the sidebar.
    pub(super) fn render_conversation(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let drop_tint = cx.theme().primary.opacity(0.06);
        // The tabs and the conversation's actions are in the window's bar
        // (`render_project_bar`); what follows is the conversation, or a
        // document shown in the centre.
        let page = div()
            .v_flex()
            .flex_1()
            .min_w(px(300.))
            .h_full()
            // Files dragged in from outside are attached to the message
            // being written.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, window, cx| {
                this.attach_dropped(dropped.paths(), window, cx)
            }));
        let mut content = div().v_flex().flex_1().min_w_0().min_h_0();
        if let Some(bar) = self.render_conversation_bar(cx) {
            content = content.child(bar);
        }
        if let Some(bar) = self.render_exploration_bar(cx) {
            content = content.child(bar);
        }
        if let Some(bar) = self.render_assistant_bar(cx) {
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
        // A turn refused as too long is said by its bar above the composer,
        // with the ways on; its raw error shows again once paused on.
        let turn_error = self
            .state
            .snapshot
            .as_ref()
            .filter(|_| !self.recovery_shown())
            .and_then(|s| s.error.as_ref());
        if let Some(error) = self.error.as_ref().or(turn_error) {
            // In the transcript's column, inset as the bars above the
            // composer are, so their edges line up.
            content = content.child(surfaces::reading_column(
                div().px_4().pt_2().child(error_banner(error, cx)),
            ));
        }
        if let Some(notice) = &self.notice {
            content = content.child(surfaces::reading_column(
                div().px_4().pt_2().child(self.notice_banner(notice, cx)),
            ));
        }
        if self.state.session.is_none() {
            let can_start = !self.pending && self.state.project.is_some();
            return page.child(
                content.child(
                    div()
                        .v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .p_8()
                        .child(surfaces::display(tr(
                            "What would you like to research today?",
                        )))
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
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_session(window, cx)
                                })),
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
        // The pet stands in the transcript's lower right corner, in a margin
        // of its own so it never covers a message.
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
        let margin = if pet.is_some() {
            px(crate::pet::CELL.0 + 24.)
        } else {
            px(0.)
        };
        content = content.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .pr(margin)
                .child(
                    // Every row keeps the reading column, centred in what the
                    // sidebar and the side panel leave.
                    list(self.list.clone(), move |index, window, cx| {
                        weak.update(cx, |this, cx| this.found_row(index, window, cx))
                            .map(|row| surfaces::reading_column(row).into_any_element())
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .flex_1()
                    .min_h_0(),
                )
                .children(pet),
        );
        // What waits on the user between the transcript and the composer
        // keeps the composer's column, with its edges on the composer box's.
        let bars = [
            self.render_browser_extension_bar(cx)
                .map(IntoElement::into_any_element),
            self.render_stream_recovery(cx)
                .map(IntoElement::into_any_element),
            self.render_context_recovery(cx)
                .map(IntoElement::into_any_element),
            self.render_approvals(cx).map(IntoElement::into_any_element),
            self.render_review_banner(cx)
                .map(IntoElement::into_any_element),
            self.render_hook_failures(cx)
                .map(IntoElement::into_any_element),
            self.render_memory_proposal(cx)
                .map(IntoElement::into_any_element),
            self.render_follow_ups(cx)
                .map(IntoElement::into_any_element),
            self.render_transfers(cx).map(IntoElement::into_any_element),
            self.render_reuse_range(cx)
                .map(IntoElement::into_any_element),
        ];
        // The composer sits in the same column as what it answers.
        let content = content
            .children(surfaces::above_composer(
                bars.into_iter().flatten().collect(),
            ))
            .child(surfaces::reading_column(self.render_composer(running, cx)).flex_shrink_0());
        // A document opened in the centre takes the conversation's place,
        // or sits beside it; the terminals stay at the foot of the page.
        self.render_centre(page, content, window, cx)
            .children(self.render_terminal_dock(cx))
    }
}
