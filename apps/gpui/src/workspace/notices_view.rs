//! The cards for conversations in other projects that finished or failed
//! while this window shows something else (`crate::notices`), in the
//! window's corner. They join the Escape stack under whatever is already
//! open; opening one goes to the window that already shows its project.
use crate::i18n::tr;
use std::time::Instant;

use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Root, Sizable, StyledExt, button::Button};

use super::{Workspace, shell::icon_button, tone_color};
use crate::{
    assets::icon,
    home::Screen,
    layers::Layer,
    notices::{self, Opens},
    present::{self, Tone},
};

impl Workspace {
    /// The project this window shows, or none on the home page.
    pub(super) fn shown_project(&self) -> Option<String> {
        (self.screen == Screen::Project)
            .then(|| self.state.project.clone())
            .flatten()
    }

    /// Puts the cards' place in the Escape stack and their timer in step
    /// with the cards: finished ones whose linger is over go, and the next
    /// one is woken for.
    pub(super) fn sync_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let due = self.notices.expire(Instant::now());
        if self.notices.is_empty() {
            self.layers.close(Layer::SessionNotices);
        } else {
            self.layers.open_beneath(Layer::SessionNotices);
        }
        // A new timer replaces, and so cancels, the one before.
        self.notice_timer = due.map(|due| {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(due).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.sync_notices(window, cx);
                    cx.notify();
                });
            })
        });
        cx.notify();
    }

    /// Opens a card's conversation: here on the home page, else in the
    /// window that already shows its project, else in a new one.
    fn open_notice(
        &mut self,
        project: String,
        session: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notices.remove(&session);
        self.sync_notices(window, cx);
        let here = window.window_handle();
        let mut others = Vec::new();
        for handle in cx.windows() {
            if handle == here {
                continue;
            }
            let Some(root) = handle.downcast::<Root>() else {
                continue;
            };
            let Ok(view) = root.read_with(cx, |root, _| root.view().clone()) else {
                continue;
            };
            if let Ok(workspace) = view.downcast::<Workspace>() {
                others.push((handle, workspace));
            }
        }
        let shown: Vec<Option<String>> = others
            .iter()
            .map(|(_, workspace)| workspace.read(cx).shown_project())
            .collect();
        match notices::opens(self.shown_project().as_deref(), &shown, &project) {
            Opens::Here => self.open_session(project, session, window, cx),
            Opens::Window(at) => {
                let (handle, workspace) = others.swap_remove(at);
                let _ = handle.update(cx, |_, window, cx| {
                    window.activate_window();
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_session(project, session, window, cx);
                        cx.notify();
                    });
                });
            }
            Opens::NewWindow => {
                crate::open_window_at(self.backend.clone(), project, session, cx);
            }
        }
    }

    /// The cards, newest at the top, over the page's lower right corner.
    pub(super) fn render_session_notices(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.notices.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let mut stack = div()
            .absolute()
            .bottom_4()
            .right_4()
            .w(px(340.))
            .v_flex()
            .gap_2();
        for card in self.notices.cards() {
            let tone = if card.failed {
                Tone::Danger
            } else {
                Tone::Success
            };
            let (project, session) = (card.project_id.clone(), card.session_id.clone());
            let (hover_id, close_id) = (session.clone(), session.clone());
            let title = present::one_line(&card.title);
            let title = if title.is_empty() {
                tr("Untitled conversation").to_owned()
            } else {
                title
            };
            stack = stack.child(
                div()
                    .id(("session-notice", card.arrival as usize))
                    .h_flex()
                    .gap_3()
                    .p_3()
                    .occlude()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_md()
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        this.notices.hover(&hover_id, *hovered, Instant::now());
                        this.sync_notices(window, cx);
                    }))
                    .child(
                        icon(if card.failed {
                            "circle-alert"
                        } else {
                            "circle-check"
                        })
                        .text_color(tone_color(tone, cx)),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(card.project_name.clone()),
                            )
                            .child(div().truncate().child(title))
                            .child(div().text_xs().text_color(muted).child(if card.failed {
                                tr("Task failed")
                            } else {
                                tr("Task completed")
                            })),
                    )
                    .child(
                        Button::new(("session-notice-open", card.arrival as usize))
                            .label(tr("Open conversation"))
                            .xsmall()
                            .outline()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_notice(project.clone(), session.clone(), window, cx)
                            })),
                    )
                    .child(
                        icon_button(
                            ("session-notice-close", card.arrival as usize),
                            "close",
                            tr("Close"),
                        )
                        .xsmall()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.notices.remove(&close_id);
                                this.sync_notices(window, cx);
                            },
                        )),
                    ),
            );
        }
        Some(stack)
    }
}
