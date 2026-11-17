//! The strip of conversation tabs above the conversation: the ones opened
//! in this window, to go back to with a click. The open conversation's
//! documents in the centre follow its tab (`centre_view`).
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt};

use super::{Workspace, shell::icon_button};
use crate::tabs;

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

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        // One conversation is just the window, unless it has documents
        // open in the centre to go between.
        let documents = self.centre_tabs(cx);
        if !self.tabs.shown() && documents.is_empty() {
            return None;
        }
        let mut documents = Some(documents);
        // While a document is shown the conversation's own tab is not the
        // one in view, and a click on it brings the conversation back.
        let document_shown = self.centre_document().is_some();
        let muted = cx.theme().muted_foreground;
        let mut strip = div()
            .id("conversation-tabs")
            .h_flex()
            .flex_shrink_0()
            .gap_1()
            .px_3()
            .pt_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .overflow_x_scroll();
        for tab in self.tabs.open() {
            let current = self.state.session.as_deref() == Some(tab.session.as_str());
            let selected = current && !document_shown;
            // The sidebar knows the current name of the conversations it lists.
            let label = match self.known.get(&tab.session) {
                Some(row) if !row.title.trim().is_empty() => crate::present::one_line(&row.title),
                _ => tabs::label(tab),
            };
            let (project, session) = (tab.project.clone(), tab.session.clone());
            let closing = tab.session.clone();
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("tab-{}", tab.session)))
                    .h_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .pl_3()
                    .pr_1()
                    .h(px(30.))
                    .max_w(px(220.))
                    .rounded_t(cx.theme().radius)
                    .text_sm()
                    .when(selected, |tab| tab.bg(cx.theme().accent))
                    .when(!selected, |tab| {
                        tab.text_color(muted)
                            .cursor_pointer()
                            .hover(|tab| tab.bg(cx.theme().accent.opacity(0.5)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if current {
                                    this.show_centre(None, window, cx)
                                } else {
                                    this.open_session(project.clone(), session.clone(), window, cx)
                                }
                            }))
                    })
                    .child(div().min_w_0().truncate().child(label))
                    .child(
                        icon_button(
                            SharedString::from(format!("tab-close-{}", tab.session)),
                            "close",
                            "Close this tab; the conversation is kept",
                        )
                        .xsmall()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                // Not a click on the tab underneath.
                                cx.stop_propagation();
                                this.close_tab(closing.clone(), window, cx)
                            },
                        )),
                    ),
            );
            if current {
                strip = strip.children(documents.take().unwrap_or_default());
            }
        }
        Some(strip.children(documents.unwrap_or_default()))
    }
}
