//! Cards in the transcript for the calls that hand work to other agents:
//! a helper sent to explore, a subagent's conversation, a batch of
//! delegated tasks; and one quiet line for a call that checks on, stops or
//! reads back that work. Each is a summary with a way in (the conversation,
//! the Agents tab), not a second Agents tab. A card is inline, not a layer,
//! so it is not on the Escape stack; Tab reaches its folds and buttons, and
//! Enter or Space presses them.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};

use super::{
    Workspace, chip,
    generated_view::Press,
    shell::{glyph, icon_button},
    surfaces,
};
use crate::{
    agents::{self, BatchCard, Card, Helper, Mention, SubagentCard},
    assets::icon,
    i18n::{fill, tr},
    panel::PanelTab,
    present::{self, Tone},
};

impl Workspace {
    /// Works out again which rows are drawn as cards, and from what, and
    /// has the list measure the rows whose card changed.
    pub(super) fn place_agent_cards(&mut self) {
        let session = self.state.session.clone().unwrap_or_default();
        let subagents = agents::subagents_of(&session, &self.dispatched, &self.known);
        let data = agents::CardData {
            batches: (self.batches_for.as_deref() == Some(session.as_str()))
                .then_some(&self.batches[..]),
            subagents: &subagents,
            turn_running: self.state.snapshot.as_ref().is_some_and(|s| s.running),
        };
        let placed = agents::cards(&self.rows, &data);
        let changed: Vec<usize> = placed
            .iter()
            .filter(|(index, card)| self.agent_cards.get(index) != Some(card))
            .map(|(index, _)| *index)
            .chain(
                self.agent_cards
                    .keys()
                    .filter(|index| !placed.contains_key(index))
                    .copied(),
            )
            .collect();
        self.agent_cards = placed;
        for index in changed {
            if index < self.rows.len() {
                self.list.splice(index..index + 1, 1);
            }
        }
    }

    /// The row at `index` drawn as its card.
    pub(super) fn agent_card(
        &self,
        index: usize,
        card: &Card,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let key = present::tool_key(item, index);
        let open = self.open_tools.contains(&key);
        let body = match card {
            Card::Explore(helper) => self
                .explore_card(index, &key, open, helper, window, cx)
                .into_any_element(),
            Card::Subagent(subagent) => self
                .subagent_card(index, &key, subagent, cx)
                .into_any_element(),
            Card::Batch(batch) => self.batch_card(index, &key, batch, cx).into_any_element(),
            Card::Mention(mention) => {
                // What the call was given and returned, a click away.
                let mut line = div().v_flex().child(fold(
                    format!("agent-mention-{key}"),
                    open,
                    true,
                    div()
                        .h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(icon(mention.icon).text_color(cx.theme().muted_foreground))
                        .child(mention_text(mention, cx)),
                    key.clone(),
                    index,
                    cx,
                ));
                if open {
                    line = line.child(
                        div().px_3().pb_2().child(
                            TextView::markdown(
                                SharedString::from(format!("agent-mention-body-{key}")),
                                present::body(item),
                                window,
                                cx,
                            )
                            .selectable(true),
                        ),
                    );
                }
                line.into_any_element()
            }
        };
        div().w_full().px_6().py_1().child(body).into_any_element()
    }

    /// A helper sent to explore: its question, how it stands and, folded,
    /// what it found.
    fn explore_card(
        &self,
        index: usize,
        key: &str,
        open: bool,
        helper: &Helper,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let status = helper
            .status
            .clone()
            .map(|(word, tone)| (SharedString::from(word), tone));
        let mut card = card_box(key, cx)
            .child(card_head("search", tr("Explore"), None, status, cx))
            .child(div().text_sm().child(match helper.question.as_str() {
                "" => tr("A question that could not be read").to_owned(),
                question => question.to_owned(),
            }));
        if !helper.focus.is_empty() {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(fill(tr("Looking for: {}"), &[&helper.focus])),
            );
        }
        if !helper.findings.is_empty() {
            card = card.child(div().h_flex().child(fold(
                format!("agent-findings-{key}"),
                open,
                false,
                div().text_xs().text_color(muted).child(tr("Findings")),
                key.to_owned(),
                index,
                cx,
            )));
            if open {
                card = card.child(
                    TextView::markdown(
                        SharedString::from(format!("agent-findings-text-{key}")),
                        helper.findings.clone(),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
        }
        card
    }

    /// A subagent: the conversation doing the work, how it stands, what it
    /// was told, and the way into that conversation.
    fn subagent_card(
        &self,
        index: usize,
        key: &str,
        subagent: &SubagentCard,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let (word, tone) = subagent.status;
        let label = if subagent.follow_up {
            tr("Subagent follow-up")
        } else {
            tr("Subagent")
        };
        let session = subagent.session.clone();
        card_box(key, cx)
            .child(card_head("bot", label, None, Some((word.into(), tone)), cx))
            .child(div().text_sm().truncate().child(subagent.title.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .line_clamp(3)
                    .child(subagent.task.clone()),
            )
            .child(
                div().h_flex().child(
                    Button::new(("agent-card-subagent", index))
                        .icon(glyph("chat"))
                        .label(tr("Open its conversation"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(project) = this.state.project.clone() {
                                this.open_session(project, session.clone(), window, cx);
                            }
                        })),
                ),
            )
    }

    /// A batch of delegated tasks: its goal, how far it came, a line for
    /// each task with a way into its conversation, and the batch in the
    /// Agents tab or Stop while it runs.
    fn batch_card(
        &self,
        index: usize,
        key: &str,
        card: &BatchCard,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let batch = &card.batch;
        let (word, tone) = agents::batch_status(&batch.status);
        let background = card.background.then(|| tr("In the background"));
        let mut body = card_box(key, cx)
            .child(card_head(
                "swarm",
                tr("Delegated tasks"),
                background,
                Some((word.into(), tone)),
                cx,
            ))
            .child(div().text_sm().line_clamp(3).child(batch.goal.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::batch_counts(batch)),
            );
        for (number, task) in batch.tasks.iter().enumerate() {
            let (word, tone) = agents::task_status(&task.status);
            let mut line = div()
                .h_flex()
                .gap_2()
                .min_h(px(28.))
                .child(div().w(px(72.)).flex_shrink_0().child(chip(word, tone, cx)))
                .child(div().flex_shrink_0().text_xs().child(task.id.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(
                            task.instruction
                                .lines()
                                .next()
                                .unwrap_or_default()
                                .to_owned(),
                        ),
                );
            if let Some(conversation) = task.conversation.clone() {
                line = line.child(
                    icon_button(
                        SharedString::from(format!("agent-card-task-{index}-{number}")),
                        "chat",
                        "Open its conversation",
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(project) = this.state.project.clone() {
                            this.open_session(project, conversation.clone(), window, cx);
                        }
                    })),
                );
            }
            body = body.child(line);
        }
        let mut actions = div().h_flex().gap_1().child(
            Button::new(("agent-card-show", index))
                .icon(glyph("bot"))
                .label(tr("Show in Agents"))
                .xsmall()
                .ghost()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_panel_tab(PanelTab::Agents, window, cx);
                })),
        );
        if batch.running() {
            let id = batch.id.clone();
            actions = actions.child(
                Button::new(("agent-card-stop", index))
                    .icon(glyph("stop"))
                    .label(tr("Stop"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.stop_batch(id.clone(), window, cx);
                    })),
            );
        }
        body.child(actions)
    }
}

/// The card every call that hands work off is drawn on.
fn card_box(key: &str, cx: &App) -> Stateful<Div> {
    surfaces::card(SharedString::from(format!("agent-card-{key}")), cx)
        .v_flex()
        .gap_1()
        .px_3()
        .py_2()
}

/// A card's first line: what kind of work it is, a note, and how it
/// stands as a word in the theme's status colour.
fn card_head(
    name: &str,
    label: &'static str,
    note: Option<&'static str>,
    status: Option<(SharedString, Tone)>,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    div()
        .h_flex()
        .gap_2()
        .child(icon(name).text_color(muted))
        .child(div().flex_shrink_0().text_sm().child(label))
        .children(note.map(|note| div().text_xs().text_color(muted).child(note)))
        .child(div().flex_1())
        .children(status.map(|(word, tone)| chip(word, tone, cx)))
}

/// A mention's words and how its call went.
fn mention_text(mention: &Mention, cx: &App) -> Div {
    div()
        .h_flex()
        .flex_1()
        .min_w_0()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(mention.text.clone()),
        )
        .children(
            mention
                .status
                .clone()
                .map(|(word, tone)| chip(word, tone, cx)),
        )
}

/// A line that opens or shuts what `key` folds, at `index`, with a chevron
/// that says which; `wide` when it is the whole row, as a mention is.
fn fold(
    id: String,
    open: bool,
    wide: bool,
    content: Div,
    key: String,
    index: usize,
    cx: &mut Context<Workspace>,
) -> Press {
    let muted = cx.theme().muted_foreground;
    Press {
        id: SharedString::from(id).into(),
        content: div()
            .h_flex()
            .gap_1()
            .px_1()
            .when(wide, |line| line.w_full().py_1())
            .child(
                icon(if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                })
                .text_color(muted),
            )
            .child(content),
        tooltip: None,
        on_press: Some(Box::new(cx.listener(move |this, _, _, cx| {
            this.toggle_tool(key.clone(), index, cx)
        }))),
    }
}
