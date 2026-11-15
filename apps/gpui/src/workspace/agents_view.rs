//! The Agents tab of the side panel: the task batches the open
//! conversation's agent delegated, each task with how it went and the
//! conversation it worked in, and the helpers it sent off to explore,
//! with what each was asked and what it found.
use crate::i18n::{fill, tr};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use wisp_server::desktop::DelegatedBatch;

use super::{Workspace, chip, shell::icon_button, tone_color};
use crate::{agents, backend::receive, layers::Layer, panel::PanelTab, present::Tone};

impl Workspace {
    /// Whether the Agents tab is showing.
    pub(super) fn agents_visible(&self) -> bool {
        self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Agents
    }

    /// Lists the open conversation's delegated batches.
    pub(super) fn load_batches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            self.batches.clear();
            return;
        };
        self.batches_request += 1;
        let request = self.batches_request;
        let swarm = self.backend.swarm_progress(session.clone());
        let reply = self.backend.delegated_batches(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let swarm = receive(swarm).await.ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.batches_request != request {
                    return;
                }
                this.swarm = swarm;
                // A batch that cannot be read is left out, not an error of
                // the conversation's.
                this.batches = result.unwrap_or_default();
                // A batch may run on after the turn: look again while one
                // does and the tab is showing.
                let swarming = this
                    .swarm
                    .as_ref()
                    .is_some_and(|swarm| !swarm.phase.terminal());
                if this.agents_visible()
                    && (swarming || this.batches.iter().any(DelegatedBatch::running))
                {
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(2))
                            .await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if this.batches_request == request {
                                this.load_batches(window, cx);
                            }
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs a failed or stopped batch again, with the token budgets given
    /// to its tasks meanwhile; its result comes back to the conversation.
    fn retry_batch(&mut self, batch: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let budgets = self
            .batches
            .iter()
            .find(|shown| shown.id == batch)
            .map(|shown| agents::rerun_budgets(shown, &self.budget_changes))
            .unwrap_or_default();
        self.layers.close(Layer::TaskBudget);
        self.budget_target = None;
        let reply = self
            .backend
            .retry_delegated_batch(session, batch.clone(), budgets);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    // What was given is the plan's now.
                    Ok(()) => this.budget_changes.retain(|(id, _), _| *id != batch),
                    Err(error) => this.error = Some(error),
                }
                this.load_batches(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the field for a task's token budget, with what it is now.
    fn edit_budget(
        &mut self,
        key: (String, String),
        current: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = current.map(|tokens| tokens.to_string()).unwrap_or_default();
        self.budget_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        self.budget_target = Some(key);
        self.layers.open(Layer::TaskBudget);
        cx.notify();
    }

    /// Keeps the typed budget for the task's next run.
    pub(super) fn save_budget(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.budget_target.clone() else {
            return;
        };
        match agents::parse_budget(&self.budget_input.read(cx).value()) {
            Ok(tokens) => {
                self.budget_changes.insert(key, tokens);
                self.budget_target = None;
                self.layers.close(Layer::TaskBudget);
                self.error = None;
            }
            Err(message) => self.error = Some(message.to_owned()),
        }
        cx.notify();
    }

    fn cancel_budget(&mut self, cx: &mut Context<Self>) {
        self.budget_target = None;
        self.layers.close(Layer::TaskBudget);
        cx.notify();
    }

    /// A task's token budget, with Change while its batch can run again.
    fn render_budget(
        &self,
        batch: &DelegatedBatch,
        task: &wisp_server::desktop::DelegatedTask,
        id: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let key = (batch.id.clone(), task.id.clone());
        let changed = self.budget_changes.get(&key).copied();
        if self.layers.contains(Layer::TaskBudget) && self.budget_target.as_ref() == Some(&key) {
            return div()
                .h_flex()
                .gap_1()
                .child(
                    div()
                        .w(px(120.))
                        .child(Input::new(&self.budget_input).xsmall()),
                )
                .child(
                    Button::new(("budget-save", id))
                        .label(tr("Keep"))
                        .xsmall()
                        .on_click(cx.listener(|this, _, window, cx| this.save_budget(window, cx))),
                )
                .child(
                    Button::new(("budget-cancel", id))
                        .label(tr("Cancel"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_budget(cx))),
                );
        }
        let current = changed.or(task.max_tokens);
        div()
            .h_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::budget_line(task, changed)),
            )
            .child(
                Button::new(("budget-edit", id))
                    .label(tr("Change"))
                    .xsmall()
                    .ghost()
                    .tooltip(tr("Give the task another token budget for its next run"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_budget(key.clone(), current, window, cx)
                    })),
            )
    }

    /// Stops a running batch and every task in it.
    fn stop_batch(&mut self, batch: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let reply = self.backend.cancel_delegated_batch(session, batch);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_batches(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Work over many items: what it is, how far it came, and each part.
    fn render_swarm(&self, progress: &wisp_dto::SwarmProgress, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (phase, tone) = agents::swarm_phase(progress.phase);
        let mut card = div()
            .v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div().h_flex().gap_2().child(chip(phase, tone, cx)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_semibold()
                        .child(progress.title.clone()),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::swarm_line(progress)),
            );
        for task in progress.tasks.iter().take(50) {
            let (status, tone) = agents::swarm_task(task.status);
            card = card.child(
                div().h_flex().gap_2().child(chip(status, tone, cx)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .child(task.title.clone()),
                ),
            );
        }
        if !progress.summary.is_empty() {
            card = card.child(div().text_xs().child(progress.summary.clone()));
        }
        card
    }

    /// One delegated batch: its goal, how far it came, and each task.
    fn render_batch(&self, index: usize, batch: &DelegatedBatch, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (status, tone) = agents::batch_status(&batch.status);
        let mut head = div()
            .h_flex()
            .gap_2()
            .child(chip(status, tone, cx))
            .children(batch.nested.then(|| chip(tr("Nested"), Tone::Neutral, cx)))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::batch_progress(batch)),
            )
            .child(div().flex_1());
        if agents::can_retry(batch) {
            let id = batch.id.clone();
            head = head.child(
                Button::new(("batch-retry", index))
                    .label(tr("Run again"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.retry_batch(id.clone(), window, cx);
                    })),
            );
        }
        if batch.running() {
            let id = batch.id.clone();
            head = head.child(
                Button::new(("batch-stop", index))
                    .label(tr("Stop"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.stop_batch(id.clone(), window, cx);
                    })),
            );
        }
        let mut card = div()
            .v_flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(head)
            .child(div().text_sm().font_semibold().child(batch.goal.clone()));
        for (number, task) in batch.tasks.iter().enumerate() {
            let (status, tone) = agents::task_status(&task.status);
            let mut line = div()
                .h_flex()
                .gap_2()
                .child(div().text_xs().font_semibold().child(task.id.clone()))
                .child(chip(status, tone, cx))
                .children(
                    task.expert
                        .clone()
                        .map(|expert| chip(expert, Tone::Neutral, cx)),
                )
                .children(
                    task.isolated
                        .then(|| chip(tr("Own worktree"), Tone::Neutral, cx)),
                )
                .child(div().flex_1());
            if let Some(conversation) = task.conversation.clone() {
                line = line.child(
                    icon_button(
                        ("batch-task-open", index * 100 + number),
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
            let mut row = div()
                .v_flex()
                .gap_1()
                .pl_2()
                .border_l_2()
                .border_color(tone_color(tone, cx).opacity(0.5))
                .child(line)
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(task.instruction.clone()),
                );
            if !task.depends_on.is_empty() {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(fill(tr("After {}"), &[&task.depends_on.join(", ")])),
                );
            }
            if let Some(summary) = &task.summary {
                row = row.child(div().text_xs().child(summary.clone()));
            }
            if let Some(error) = task.error.as_ref().filter(|_| task.status != "succeeded") {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(tone_color(Tone::Danger, cx))
                        .child(error.clone()),
                );
            }
            if let Some(usage) = agents::task_usage(task) {
                row = row.child(div().text_xs().text_color(muted).child(usage));
            }
            if agents::budget_editable(batch, task) {
                row = row.child(self.render_budget(batch, task, index * 100 + number, cx));
            }
            card = card.child(row);
        }
        card
    }

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
            .overflow_y_scroll();
        if let Some(progress) = &self.swarm {
            list = list.child(self.render_swarm(progress, cx));
        }
        if !self.batches.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child(tr("Delegated tasks")),
            );
            for (index, batch) in self.batches.iter().enumerate() {
                list = list.child(self.render_batch(index, batch, cx));
            }
        }
        // The helpers' line says there are none only when nothing else is
        // listed.
        if !helpers.is_empty() || (self.batches.is_empty() && self.swarm.is_none()) {
            list = list.child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(muted)
                    .child(agents::summary(&helpers, self.state.session.is_some())),
            );
        }
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
