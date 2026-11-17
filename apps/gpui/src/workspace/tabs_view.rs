//! The window's bar on a project's page: the sidebar's toggle, the tabs of
//! the conversations opened in this window, to go back to with a click
//! (the open conversation's documents in the centre follow its tab,
//! `centre_view`), and the conversation's four icon buttons.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, StyledExt};

use super::{
    Workspace,
    surfaces::{self, IconTone},
};
use crate::{i18n::tr, layers::Layer, present, tabs};

impl Workspace {
    /// Closes a tab; the one in view hands over to its neighbour, and the
    /// last one leaves the project's empty page.
    fn close_tab(&mut self, session: String, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.state.session.clone();
        let was_current = current.as_deref() == Some(session.as_str());
        match self.tabs.close(&session, current.as_deref()) {
            Some(next) => self.open_session(next.project, next.session, window, cx),
            None if was_current => self.leave_conversation(window, cx),
            None => {}
        }
        cx.notify();
    }

    /// What the window's bar holds on a project's page: the sidebar's
    /// toggle; the tabs, one for each conversation opened in this window
    /// (always one for the conversation in view) with the open
    /// conversation's documents after its own; and at the right end four
    /// icon buttons: search, the terminals, the side panel and More, whose
    /// menu holds everything else the conversation offers.
    pub(super) fn render_project_bar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let sidebar = surfaces::icon_action(
            "toggle-sidebar",
            "panel-left",
            if self.sidebar_hidden {
                tr("Show the sidebar")
            } else {
                tr("Hide the sidebar")
            },
            if self.sidebar_hidden {
                IconTone::Faded
            } else {
                IconTone::Ink
            },
            cx,
        )
        .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)));
        div()
            .h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap_2()
            .child(sidebar)
            .child(self.render_tabs(cx))
            .children(self.render_waiting(window, cx))
            .child(self.render_bar_actions(cx))
    }

    /// The conversation's name, as its tab and the window say it.
    pub(super) fn conversation_name(&self) -> String {
        self.current_row()
            .map(|row| present::one_line(&row.title))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                // The assistant's conversation is not among the listed ones.
                if self
                    .state
                    .session
                    .as_deref()
                    .is_some_and(wisp_server::desktop::is_assistant)
                {
                    tr("Research assistant").into()
                } else {
                    tr("New research").into()
                }
            })
    }

    /// The tabs: a conversation's is its chat icon and name, a document's
    /// its document icon and name. The one in view is on the paper.
    fn render_tabs(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut documents = Some(self.centre_tabs(cx));
        // While a document is shown the conversation's own tab is not the
        // one in view, and a click on it brings the conversation back.
        let document_shown = self.centre_document().is_some();
        let mut strip = div()
            .id("conversation-tabs")
            .h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .overflow_x_scroll();
        let mut tabs: Vec<(String, String, String)> = self
            .tabs
            .open()
            .iter()
            .map(|tab| {
                // The sidebar knows the current name of the conversations it
                // lists.
                let label = match self.known.get(&tab.session) {
                    Some(row) if !row.title.trim().is_empty() => present::one_line(&row.title),
                    _ => tabs::label(tab),
                };
                (tab.project.clone(), tab.session.clone(), label)
            })
            .collect();
        // The conversation in view always has a tab, also when this window
        // has not noted it (the research assistant's).
        if let (Some(project), Some(session)) = (&self.state.project, &self.state.session)
            && !tabs.iter().any(|(_, open, _)| open == session)
        {
            tabs.push((project.clone(), session.clone(), self.conversation_name()));
        }
        for (project, session, label) in tabs {
            let current = self.state.session.as_deref() == Some(session.as_str());
            let selected = current && !document_shown;
            let closing = session.clone();
            let mut tab = surfaces::window_tab(
                SharedString::from(format!("tab-{session}")),
                "chat",
                label,
                selected,
                cx,
            )
            .child(
                surfaces::tab_close(
                    SharedString::from(format!("tab-close-{session}")),
                    tr("Close this tab; the conversation is kept"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    // Not a click on the tab underneath.
                    cx.stop_propagation();
                    this.close_tab(closing.clone(), window, cx)
                })),
            );
            if !selected {
                tab = tab.on_click(cx.listener(move |this, _, window, cx| {
                    if current {
                        this.show_centre(None, window, cx)
                    } else {
                        this.open_session(project.clone(), session.clone(), window, cx)
                    }
                }));
            }
            strip = strip.child(tab);
            if current {
                strip = strip.children(documents.take().unwrap_or_default());
            }
        }
        strip.children(documents.unwrap_or_default())
    }

    /// The four icon buttons at the right end of the bar.
    fn render_bar_actions(&self, cx: &mut Context<Self>) -> Div {
        let tone = |on: bool| if on { IconTone::Ink } else { IconTone::Faded };
        let open = self.layers.contains(Layer::ConversationMenu);
        let outline = self.layers.contains(Layer::Outline);
        let mut more = div().relative().child(
            surfaces::icon_action(
                "conversation-menu",
                "more",
                tr("Conversation actions"),
                tone(open || outline),
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                // The outline opened from the menu closes with a click here.
                if this.layers.close(Layer::Outline) {
                    this.focus_composer(window, cx);
                    cx.notify();
                    return;
                }
                this.toggle_conversation_menu(window, cx)
            })),
        );
        // Painted last, over the page below.
        if open {
            more = more.child(deferred(self.conversation_menu(cx)));
        } else if outline {
            more = more.child(deferred(self.outline_menu(cx)));
        }
        div()
            .h_flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .child(
                surfaces::icon_action(
                    "palette",
                    "search",
                    tr("Search everything"),
                    IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_palette(window, cx))),
            )
            .child(
                surfaces::icon_action(
                    "terminal-toggle",
                    "terminal",
                    tr("Terminals"),
                    tone(self.terminal.open),
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal_dock(window, cx))),
            )
            .child(
                surfaces::icon_action(
                    "files-toggle",
                    "panel",
                    tr("Show or hide the side panel"),
                    tone(self.layers.contains(Layer::FilesPanel)),
                    cx,
                )
                .disabled(self.state.project.is_none())
                .on_click(cx.listener(|this, _, window, cx| this.toggle_files(window, cx))),
            )
            .child(more)
    }

    /// What waits for the user, other than the conversation on screen.
    fn waiting_rows(&self) -> Vec<&wisp_dto::SessionSearchInfo> {
        self.inbox
            .iter()
            .filter(|row| Some(&row.id) != self.state.session.as_ref())
            .collect()
    }

    /// The bell in the window's bar: its count in the waiting colour, only
    /// while something waits for the user. It is not one of the bar's icon
    /// buttons; a gap sets it apart from them. A click opens the list of
    /// what waits under it.
    pub(super) fn render_waiting(&self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let count = self.waiting_rows().len();
        if count == 0 {
            return None;
        }
        let ink = surfaces::stamp_color(surfaces::StampKind::Waiting, cx);
        let hover = cx.theme().list_hover;
        let open = self.layers.contains(Layer::Inbox);
        let mut place = div().relative().flex_none().mr_3().child(
            div()
                .id("waiting-bell")
                // In the window's bar: on Windows, not the caption to drag.
                .occlude()
                .h_flex()
                .items_center()
                .gap_1()
                .px_1p5()
                .h(px(surfaces::ICON_BUTTON))
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .when(open, |bell| bell.bg(hover))
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(tr("Needs you")).build(window, cx)
                })
                .child(crate::assets::icon("bell").text_color(ink))
                .child(surfaces::fact(count.to_string(), cx).text_color(ink))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.layers.close(Layer::Inbox) {
                        this.layers.open(Layer::Inbox);
                    }
                    cx.notify();
                })),
        );
        if open {
            // A clear ground over the whole window under the list: a press
            // anywhere beside the list, the bell included, closes it.
            let size = window.viewport_size();
            let backdrop = anchored()
                .position_mode(AnchoredPositionMode::Window)
                .position(point(px(0.), px(0.)))
                .child(
                    div()
                        .id("waiting-backdrop")
                        .w(size.width)
                        .h(size.height)
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::Inbox);
                                cx.notify();
                            }),
                        ),
                );
            place = place
                .child(deferred(backdrop).with_priority(1))
                .child(deferred(self.waiting_list(cx)).with_priority(2));
        }
        Some(place)
    }

    /// The list of what waits for the user, floating under the bell at the
    /// right: one record row for each conversation, when, its title, and
    /// its project only when that is another one. A click opens it, in its
    /// project, and closes the list; so do Escape and a click beside it.
    fn waiting_list(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let now = super::shell::unix_now();
        let hover = cx.theme().list_hover;
        let rows: Vec<_> = self
            .waiting_rows()
            .into_iter()
            .map(|row| {
                let (project, session) = (row.project_id.clone(), row.id.clone());
                let mut record = surfaces::record_row(
                    SharedString::from(format!("inbox-{session}")),
                    present::title_line(&row.title),
                )
                .time(crate::home::clock(now, row.activity_at))
                .icon("chat");
                if self.state.project.as_deref() != Some(row.project_id.as_str())
                    && !row.project_name.is_empty()
                {
                    record = record.lead(row.project_name.clone());
                }
                // No stamp: the whole list is what waits, and the title
                // gets the room.
                record
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.layers.close(Layer::Inbox);
                        this.open_session(project.clone(), session.clone(), window, cx)
                    }))
            })
            .collect();
        div()
            .id("waiting-list")
            .absolute()
            .top(px(surfaces::ICON_BUTTON + 6.))
            .right_0()
            .w(px(360.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .v_flex()
            .p_2()
            .occlude()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(surfaces::record_group(rows, cx))
    }
}
