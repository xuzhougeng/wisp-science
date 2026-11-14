//! The Agents tab of the side panel: the helpers the open conversation
//! sent off to explore, with what each was asked and what it found.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt, text::TextView};

use super::{Workspace, chip, shell::icon_button};
use crate::agents;

impl Workspace {
    pub(super) fn render_agents_tab(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let helpers = agents::helpers(&self.rows, running);
        let mut list = div()
            .id("agents")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::summary(&helpers, self.state.session.is_some())),
            );
        for helper in &helpers {
            let row = helper.row;
            let mut head = div().h_flex().gap_2().child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("#{}", helper.number)),
            );
            if let Some((status, tone)) = &helper.status {
                head = head.child(chip(status.clone(), *tone, cx));
            }
            head = head.child(div().flex_1()).child(
                icon_button(
                    ("agent-show", helper.number),
                    "arrow-right",
                    "Show it in the conversation",
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.list.scroll_to_reveal_item(row);
                    cx.notify();
                })),
            );
            let mut card = div()
                .v_flex()
                .gap_1()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(head)
                .child(div().text_sm().child(match helper.question.as_str() {
                    "" => tr("A question that could not be read").to_owned(),
                    question => question.to_owned(),
                }));
            if !helper.focus.is_empty() {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::fill(tr("Looking for: {}"), &[&helper.focus])),
                );
            }
            if !helper.findings.is_empty() {
                card = card.child(
                    TextView::markdown(
                        SharedString::from(format!("agent-findings-{}", helper.number)),
                        helper.findings.clone(),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
            list = list.child(card);
        }
        list.into_any_element()
    }
}
