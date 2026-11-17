//! Find in the open conversation: the bar over the transcript, moving
//! between matches, and marking the rows that match.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt};

use super::{Workspace, shell::icon_button};
use crate::{assets::icon, layers::Layer};

impl Workspace {
    /// Opens the bar, or puts the caret back in it when it is open.
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.session.is_none() {
            return;
        }
        self.layers.open(Layer::Find);
        self.find_input
            .update(cx, |input, cx| input.focus(window, cx));
        self.search_find(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Find) {
            self.find.clear();
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Searches for what the bar holds and shows the current match.
    pub(super) fn search_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        self.find.search(&self.rows, &query);
        self.show_found();
        cx.notify();
    }

    /// Moves to the match before (`older`) or after the current one.
    pub(super) fn step_find(&mut self, older: bool, cx: &mut Context<Self>) {
        self.find.step(older);
        self.show_found();
        cx.notify();
    }

    fn show_found(&self) {
        if let Some(row) = self.find.current() {
            self.list.scroll_to_reveal_item(row);
        }
    }

    /// A transcript row, marked when it matches what is being looked for.
    pub(super) fn found_row(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = self.row(index, window, cx);
        if !self.layers.contains(Layer::Find) || !self.find.is_hit(index) {
            return row;
        }
        let mark = cx.theme().primary;
        let current = self.find.current() == Some(index);
        // A tint only, so the row's edges stay where they are.
        div()
            .w_full()
            .bg(mark.opacity(if current { 0.12 } else { 0.07 }))
            .child(row)
            .into_any_element()
    }

    pub(super) fn render_find_bar(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        div()
            .h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .h(px(40.))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div().w(px(320.)).child(
                    super::surfaces::input(&self.find_input, cx)
                        .appearance(false)
                        .prefix(icon("search").text_color(muted)),
                ),
            )
            .child(
                div()
                    .w(px(90.))
                    .text_xs()
                    .text_color(muted)
                    .child(self.find.label()),
            )
            .child(
                icon_button("find-older", "chevron-up", tr("Earlier match (Return)"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_find(true, cx))),
            )
            .child(
                icon_button("find-newer", "chevron-down", tr("Later match"))
                    .on_click(cx.listener(|this, _, _, cx| this.step_find(false, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(if self.before.is_some() {
                        tr("Searches the messages that are loaded; load earlier ones to search them")
                    } else {
                        ""
                    }),
            )
            .child(
                icon_button("find-close", "close", tr("Close (Esc)"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
    }
}
