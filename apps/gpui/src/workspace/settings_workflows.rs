//! The Workflows page of Settings: templates of a goal broken into tasks,
//! run as one conversation of the open project, or as agents, one per
//! task, after the researcher approves them.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use wisp_server::desktop::{
    ModelTier, WORKFLOW_CAPABILITIES, WORKFLOW_TASK_LIMIT, Workflow, WorkflowAgentSettings,
    WorkflowApproval, WorkflowTask,
};

use super::Workspace;
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    panel::PanelTab,
    workflows,
};

/// One task while it is in the form.
pub(super) struct TaskDraft {
    title: Entity<InputState>,
    instructions: Entity<InputState>,
    /// The earlier tasks it needs, by their place in the list.
    after: Vec<usize>,
    /// What its agent may do when the workflow runs as agents.
    capabilities: Vec<String>,
    /// The rest of how its agent works; the typed parts are the fields
    /// below, read when the form is saved.
    agent: WorkflowAgentSettings,
    tokens: Entity<InputState>,
    seconds: Entity<InputState>,
    shape: Entity<InputState>,
    /// The agent settings are open.
    open: bool,
}

impl Workspace {
    pub(super) fn load_workflows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.workflows();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(workflows) => this.workflows = workflows,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the templates and shows the list the host kept.
    fn change_workflows(
        &mut self,
        reply: Reply<Vec<Workflow>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(workflows) => {
                        this.workflows = workflows;
                        this.layers.close(Layer::WorkflowForm);
                        this.layers.close(Layer::ConfirmRemoveWorkflow);
                        this.workflow_editing = None;
                        this.workflow_tasks.clear();
                    }
                    // The form stays open with what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn task_draft(
        &self,
        task: &WorkflowTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TaskDraft {
        let field = |value: &str, lines: bool, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_owned();
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).multi_line(lines);
                input.set_value(value, window, cx);
                input
            })
        };
        let agent = &task.agent;
        let number = |value: Option<String>| value.unwrap_or_default();
        let shape = agent
            .output_schema
            .as_ref()
            .and_then(|schema| serde_json::to_string_pretty(schema).ok())
            .unwrap_or_default();
        TaskDraft {
            title: field(&task.title, false, window, cx),
            instructions: field(&task.instructions, true, window, cx),
            after: task.after.clone(),
            capabilities: task.capabilities.clone(),
            tokens: field(
                &number(agent.max_tokens.map(|n| n.to_string())),
                false,
                window,
                cx,
            ),
            seconds: field(
                &number(agent.timeout_secs.map(|n| n.to_string())),
                false,
                window,
                cx,
            ),
            shape: field(&shape, true, window, cx),
            // Open when anything in it is set.
            open: *agent != WorkflowAgentSettings::default(),
            agent: agent.clone(),
        }
    }

    /// Opens the form on a new workflow, or on `workflow` to change it.
    fn edit_workflow(
        &mut self,
        workflow: Option<&Workflow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blank = Workflow {
            tasks: vec![WorkflowTask::default()],
            ..Workflow::default()
        };
        let shown = workflow.unwrap_or(&blank);
        self.workflow_editing = Some(shown.id.clone());
        self.layers.close(Layer::ConfirmRemoveWorkflow);
        self.layers.open(Layer::WorkflowForm);
        self.settings_error = None;
        for (field, value) in
            self.workflow_fields
                .iter()
                .zip([&shown.name, &shown.goal, &shown.context])
        {
            field.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.workflow_tasks = shown
            .tasks
            .iter()
            .map(|task| self.task_draft(task, window, cx))
            .collect();
        self.workflow_approval = shown.approval;
        self.workflow_pick = None;
        // The experts a task can be done as.
        self.load_experts(window, cx);
        self.workflow_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workflow_editing.clone() else {
            return;
        };
        let [name, goal, context] =
            [0, 1, 2].map(|at| self.workflow_fields[at].read(cx).value().to_string());
        let mut tasks = Vec::with_capacity(self.workflow_tasks.len());
        for (index, draft) in self.workflow_tasks.iter().enumerate() {
            let typed = workflows::agent_fields(
                &draft.tokens.read(cx).value(),
                &draft.seconds.read(cx).value(),
                &draft.shape.read(cx).value(),
            );
            let (max_tokens, timeout_secs, output_schema) = match typed {
                Ok(typed) => typed,
                Err(problem) => {
                    // The form stays open with what was typed.
                    self.settings_error = Some(crate::i18n::fill(
                        tr("Task {}: {}"),
                        &[&(index + 1), &problem],
                    ));
                    cx.notify();
                    return;
                }
            };
            tasks.push(WorkflowTask {
                title: draft.title.read(cx).value().to_string(),
                instructions: draft.instructions.read(cx).value().to_string(),
                after: draft.after.clone(),
                capabilities: draft.capabilities.clone(),
                agent: WorkflowAgentSettings {
                    max_tokens,
                    timeout_secs,
                    output_schema,
                    ..draft.agent.clone()
                },
            });
        }
        let reply = self.backend.save_workflow(Workflow {
            id,
            name,
            goal,
            tasks,
            approval: self.workflow_approval,
            context,
        });
        self.change_workflows(reply, window, cx);
    }

    /// Drafts a workflow as agents in the open project and shows its
    /// conversation with the Agents tab, where it is approved or discarded
    /// (or already running, when it asked for no approval).
    fn run_workflow_as_agents(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.settings_error = None;
        let reply = self.backend.draft_workflow(project.clone(), id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(drafted) => {
                        this.close_settings(window, cx);
                        this.open_session(project, drafted.session, window, cx);
                        this.show_panel_tab(PanelTab::Agents, window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_workflow_task(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.workflow_tasks.len() {
            return;
        }
        let after: Vec<_> = self
            .workflow_tasks
            .iter()
            .map(|draft| draft.after.clone())
            .collect();
        self.workflow_tasks.remove(index);
        // Places move up, so a pick may now be another task.
        self.workflow_pick = None;
        for (draft, needs) in self
            .workflow_tasks
            .iter_mut()
            .zip(workflows::without(&after, index))
        {
            draft.after = needs;
        }
        cx.notify();
    }

    /// Starts a run in the open project and shows its conversation.
    fn run_workflow(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.settings_error = None;
        let reply = self.backend.run_workflow(project.clone(), id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(session) => {
                        this.close_settings(window, cx);
                        this.open_session(project, session, window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn workflow_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut form = div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(660.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(label(tr("Name")))
            .child(Input::new(&self.workflow_fields[0]).small())
            .child(label(tr(
                "Goal: what the whole workflow is to achieve. What it is about is typed \
                 into the conversation after it starts.",
            )))
            .child(
                div()
                    .h(px(90.))
                    .child(Input::new(&self.workflow_fields[1]).h_full()),
            )
            .child(label(tr(
                "Context (optional): scope, conventions and how results are reported, \
                 told to every task.",
            )))
            .child(
                div()
                    .h(px(60.))
                    .child(Input::new(&self.workflow_fields[2]).h_full()),
            );
        // How a run as agents is approved.
        let mut approval = div()
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(label(tr("Run as agents:")));
        for (number, choice) in [WorkflowApproval::ReviewAll, WorkflowApproval::AutoSafe]
            .into_iter()
            .enumerate()
        {
            approval = approval.child(
                Button::new(("workflow-approval", number))
                    .label(workflows::approval_label(choice))
                    .xsmall()
                    .selected(self.workflow_approval == choice)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.workflow_approval = choice;
                        cx.notify();
                    })),
            );
        }
        form = form.child(approval).child(self.workflow_map(cx));
        for (index, draft) in self.workflow_tasks.iter().enumerate() {
            // What its agent may do; none is reading the project.
            let mut allowed = div()
                .h_flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .child(label(tr("As an agent it may")));
            for (number, capability) in WORKFLOW_CAPABILITIES.into_iter().enumerate() {
                allowed = allowed.child(
                    Button::new(("workflow-capability", index * 100 + number))
                        .label(workflows::capability_label(capability))
                        .xsmall()
                        .selected(draft.capabilities.iter().any(|kept| kept == capability))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(draft) = this.workflow_tasks.get_mut(index) {
                                draft.capabilities =
                                    workflows::toggled_capability(&draft.capabilities, capability);
                                cx.notify();
                            }
                        })),
                );
            }
            let mut needs = div().h_flex().flex_wrap().items_center().gap_1();
            if index > 0 {
                needs = needs.child(label(tr("After task")));
            }
            for earlier in 0..index {
                needs = needs.child(
                    Button::new(("workflow-after", index * 100 + earlier))
                        .label(SharedString::from((earlier + 1).to_string()))
                        .xsmall()
                        .selected(draft.after.contains(&earlier))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(draft) = this.workflow_tasks.get_mut(index) {
                                draft.after = workflows::toggled(&draft.after, earlier);
                                cx.notify();
                            }
                        })),
                );
            }
            form = form.child(
                div()
                    .v_flex()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(crate::i18n::fill(tr("Task {}"), &[&(index + 1)])),
                            )
                            .child(div().flex_1().child(Input::new(&draft.title).small()))
                            .child(
                                Button::new(("workflow-task-remove", index))
                                    .label(tr("Remove"))
                                    .xsmall()
                                    .ghost()
                                    .disabled(self.workflow_tasks.len() == 1)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_workflow_task(index, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .h(px(84.))
                            .child(Input::new(&draft.instructions).h_full()),
                    )
                    .child(needs)
                    .child(allowed)
                    .child(self.agent_settings(index, draft, cx)),
            );
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_2()
                .child(
                    Button::new("workflow-task-add")
                        .label(tr("Add task"))
                        .small()
                        .disabled(self.workflow_tasks.len() >= WORKFLOW_TASK_LIMIT)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let draft = this.task_draft(&WorkflowTask::default(), window, cx);
                            draft.title.update(cx, |input, cx| input.focus(window, cx));
                            this.workflow_tasks.push(draft);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("workflow-save")
                        .label(tr("Save"))
                        .small()
                        .primary()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_workflow(window, cx)),
                        ),
                )
                .child(
                    Button::new("workflow-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::WorkflowForm);
                            this.workflow_editing = None;
                            this.workflow_tasks.clear();
                            cx.notify();
                        })),
                ),
        )
    }

    /// The workflow as a map: each task in the column of its stage, with
    /// an edge to each task that needs it. Clicking a task and then another
    /// makes the one listed later need the other, or no longer need it.
    fn workflow_map(&self, cx: &mut Context<Self>) -> Div {
        const WIDTH: f32 = 150.;
        const HEIGHT: f32 = 36.;
        const ACROSS: f32 = 56.;
        const DOWN: f32 = 12.;
        const MARGIN: f32 = 6.;
        let muted = cx.theme().muted_foreground;
        let after: Vec<Vec<usize>> = self
            .workflow_tasks
            .iter()
            .map(|draft| draft.after.clone())
            .collect();
        let (nodes, edges) = workflows::map(&after);
        let columns = nodes.iter().map(|node| node.column + 1).max().unwrap_or(0) as f32;
        let rows = nodes.iter().map(|node| node.row + 1).max().unwrap_or(0) as f32;
        // Nodes are in task order, so a task's place is its index.
        let places: Vec<(f32, f32)> = nodes
            .iter()
            .map(|node| {
                (
                    MARGIN + node.column as f32 * (WIDTH + ACROSS),
                    MARGIN + node.row as f32 * (HEIGHT + DOWN),
                )
            })
            .collect();
        let lines_at = places.clone();
        let edge = muted.opacity(0.8);
        let lines = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mut path = PathBuilder::stroke(px(1.5));
                for &(from, to) in &edges {
                    let ((fx, fy), (tx, ty)) = (lines_at[from], lines_at[to]);
                    let start = point(
                        bounds.origin.x + px(fx + WIDTH),
                        bounds.origin.y + px(fy + HEIGHT / 2.),
                    );
                    let end = point(
                        bounds.origin.x + px(tx),
                        bounds.origin.y + px(ty + HEIGHT / 2.),
                    );
                    let middle = (start.x + end.x) / 2.;
                    path.move_to(start);
                    path.cubic_bezier_to(end, point(middle, start.y), point(middle, end.y));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, edge);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut board = div()
            .relative()
            .flex_shrink_0()
            .w(px(
                MARGIN * 2. + columns * (WIDTH + ACROSS) - ACROSS.min(columns * ACROSS)
            ))
            .h(px(
                MARGIN * 2. + rows * (HEIGHT + DOWN) - DOWN.min(rows * DOWN)
            ))
            .child(lines);
        for node in &nodes {
            let task = node.task;
            let (x, y) = places[task];
            let title = self.workflow_tasks[task]
                .title
                .read(cx)
                .value()
                .trim()
                .to_owned();
            let text = if title.is_empty() {
                crate::i18n::fill(tr("Task {}"), &[&(task + 1)])
            } else {
                format!("{}. {title}", task + 1)
            };
            let picked = self.workflow_pick == Some(task);
            let primary = cx.theme().primary;
            board = board.child(
                div()
                    .id(("workflow-map-node", task))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(WIDTH))
                    .h(px(HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(if picked { primary } else { cx.theme().border })
                    .bg(if picked {
                        primary.opacity(0.12)
                    } else {
                        cx.theme().background
                    })
                    .text_xs()
                    .cursor_pointer()
                    .child(div().truncate().child(text))
                    .on_click(cx.listener(move |this, _, _, cx| this.click_map(task, cx))),
            );
        }
        div()
            .v_flex()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(tr(
                "Map: click a task, then another, to make the later one need the earlier one, or no longer need it.",
            )))
            .child(
                div()
                    .id("workflow-map")
                    .max_w_full()
                    .overflow_x_scroll()
                    .child(board),
            )
    }

    fn click_map(&mut self, task: usize, cx: &mut Context<Self>) {
        let (pick, join) = workflows::map_click(self.workflow_pick, task);
        self.workflow_pick = pick;
        if let Some((earlier, later)) = join {
            if let Some(draft) = self.workflow_tasks.get_mut(later) {
                draft.after = workflows::toggled(&draft.after, earlier);
            }
        }
        cx.notify();
    }

    /// How one task's agent works, as the WebView's Workflow Studio sets
    /// it: the expert it works as, its model, a worktree of its own, a
    /// token budget, a time limit and the shape of its result. Folded
    /// away until asked for or set.
    fn agent_settings(&self, index: usize, draft: &TaskDraft, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let toggle = Button::new(("workflow-agent-open", index))
            .label(if draft.open {
                tr("Fewer agent settings")
            } else {
                tr("Agent settings")
            })
            .xsmall()
            .ghost()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(draft) = this.workflow_tasks.get_mut(index) {
                    draft.open = !draft.open;
                    cx.notify();
                }
            }));
        let mut section = div().v_flex().gap_1().child(div().h_flex().child(toggle));
        if !draft.open {
            return section;
        }
        // Which expert, if any.
        let mut experts = div()
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(label(tr("Done as")));
        let choices = std::iter::once((None, tr("No expert").to_owned())).chain(
            self.experts
                .iter()
                .map(|expert| (Some(expert.id.clone()), expert.name.clone())),
        );
        for (number, (id, name)) in choices.enumerate() {
            let selected = draft.agent.expert == id;
            experts = experts.child(
                Button::new(("workflow-expert", index * 1000 + number))
                    .label(name)
                    .xsmall()
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.workflow_tasks.get_mut(index) {
                            draft.agent.expert = id.clone();
                            cx.notify();
                        }
                    })),
            );
        }
        // An expert the list no longer has is still named, so it is seen.
        if let Some(gone) = draft
            .agent
            .expert
            .as_ref()
            .filter(|id| !self.experts.iter().any(|expert| &expert.id == *id))
        {
            experts = experts.child(label(tr("(an expert that is gone)")).child(gone.clone()));
        }
        let mut models = div()
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(label(tr("Model")));
        for (number, model) in [None, Some(ModelTier::Fast), Some(ModelTier::Inherit)]
            .into_iter()
            .enumerate()
        {
            models = models.child(
                Button::new(("workflow-model", index * 10 + number))
                    .label(workflows::model_label(model))
                    .xsmall()
                    .selected(draft.agent.model == model)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.workflow_tasks.get_mut(index) {
                            draft.agent.model = model;
                            cx.notify();
                        }
                    })),
            );
        }
        models = models.child(
            Button::new(("workflow-isolated", index))
                .label(tr("Own worktree"))
                .xsmall()
                .selected(draft.agent.isolated)
                .tooltip(tr(
                    "Works in a Git worktree of its own; its changes are merged back when it succeeds",
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(draft) = this.workflow_tasks.get_mut(index) {
                        draft.agent.isolated = !draft.agent.isolated;
                        cx.notify();
                    }
                })),
        );
        let limits = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(label(tr("Token budget")))
            .child(div().w(px(110.)).child(Input::new(&draft.tokens).xsmall()))
            .child(label(tr("Time limit (seconds, 0 for none)")))
            .child(div().w(px(90.)).child(Input::new(&draft.seconds).xsmall()));
        section = section
            .child(experts)
            .child(models)
            .child(limits)
            .child(label(tr(
                "Result shape: a JSON object (a JSON Schema) its result must match; empty for none",
            )))
            .child(div().h(px(70.)).child(Input::new(&draft.shape).h_full()));
        section
    }

    fn workflow_row(&self, workflow: &Workflow, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let id = workflow.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveWorkflow)
            && self.workflow_editing.as_deref() == Some(id.as_str());
        let about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(workflow.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(workflows::shape_line(workflow)),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(workflow.goal.lines().next().unwrap_or_default().to_owned()),
            );
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("workflow-remove-yes-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let reply = this.backend.delete_workflow(id.clone());
                            this.change_workflows(reply, window, cx)
                        })),
                )
                .child(
                    Button::new("workflow-remove-no")
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveWorkflow);
                            this.workflow_editing = None;
                            cx.notify();
                        })),
                );
        } else {
            let editing = workflow.clone();
            let (run, agents, removing) = (id.clone(), id.clone(), id.clone());
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("workflow-run-{id}")))
                        .label(tr("Run"))
                        .small()
                        .disabled(self.state.project.is_none())
                        .tooltip(tr("Run it as one conversation"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_workflow(run.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("workflow-agents-{id}")))
                        .label(tr("Run as agents"))
                        .small()
                        .disabled(
                            self.state.project.is_none()
                                || workflow.tasks.len()
                                    > wisp_server::desktop::WORKFLOW_AGENT_LIMIT,
                        )
                        .tooltip(tr(
                            "Each task as its own agent, with what it may do; approved in the Agents tab",
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_workflow_as_agents(agents.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("workflow-edit-{id}")))
                        .label(tr("Change"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_workflow(Some(&editing), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("workflow-remove-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layers.close(Layer::WorkflowForm);
                            this.workflow_tasks.clear();
                            this.workflow_editing = Some(removing.clone());
                            this.layers.open(Layer::ConfirmRemoveWorkflow);
                            cx.notify();
                        })),
                );
        }
        div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(actions)
    }

    pub(super) fn render_workflow_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page = self.settings_page(cx).child(
            div()
                .w(px(640.))
                .pb_2()
                .text_xs()
                .text_color(muted)
                .child(tr(
                    "A workflow is a goal broken into tasks that say which earlier tasks they \
                     need. Run starts a conversation in the open project and gives the model \
                     the tasks stage by stage; it carries them out in that order itself, as \
                     one conversation with the usual approvals. Run as agents gives each task \
                     an agent of its own that may do only what the task allows; the plan waits \
                     in the new conversation's Agents tab to be approved, and the result comes \
                     back to that conversation.",
                )),
        );
        if self.state.project.is_none() {
            page = page.child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Open a project to run a workflow in it.")),
            );
        }
        let form_open = self.layers.contains(Layer::WorkflowForm);
        let editing = self.workflow_editing.as_deref();
        if self.workflows.is_empty() {
            page = page.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("None yet.")),
            );
        }
        for workflow in &self.workflows {
            page = page.child(self.workflow_row(workflow, cx));
            if form_open && editing == Some(workflow.id.as_str()) {
                page = page.child(div().py_2().child(self.workflow_form(cx)));
            }
        }
        if form_open && editing == Some("") {
            page.child(div().py_2().child(self.workflow_form(cx)))
        } else if self.layers.contains(Layer::WorkflowFromSkills) {
            page.child(div().py_2().child(self.from_skills_form(cx)))
        } else {
            page.child(
                div()
                    .pt_3()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("workflow-add")
                            .label(tr("New workflow"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_workflow(None, window, cx)
                            })),
                    )
                    .child(
                        Button::new("workflow-from-skills")
                            .label(tr("From skills"))
                            .small()
                            .disabled(self.state.project.is_none())
                            .tooltip(tr(
                                "Have the sub-agent model write a workflow of agents from skills' method documents",
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_from_skills(window, cx)
                            })),
                    ),
            )
        }
    }

    /// Opens the panel that writes a workflow from skills, with the open
    /// project's skills to pick from.
    fn open_from_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.layers.close(Layer::WorkflowForm);
        self.layers.close(Layer::ConfirmRemoveWorkflow);
        self.workflow_editing = None;
        self.workflow_skill_picks.clear();
        self.settings_error = None;
        self.layers.open(Layer::WorkflowFromSkills);
        self.workflow_request
            .update(cx, |input, cx| input.focus(window, cx));
        let reply = self.backend.skills(Some(project));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(skills) => {
                        this.workflow_skills =
                            skills.into_iter().filter(|skill| skill.enabled).collect()
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Has the workflow written, then opens it in the form to review and
    /// save; nothing is kept before that.
    fn write_from_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        if self.workflow_converting {
            return;
        }
        let request = self.workflow_request.read(cx).value().to_string();
        self.workflow_converting = true;
        self.settings_error = None;
        let reply =
            self.backend
                .workflow_from_skills(project, request, self.workflow_skill_picks.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.workflow_converting = false;
                // Closed meanwhile: what came back is not wanted.
                if !this.layers.contains(Layer::WorkflowFromSkills) {
                    return;
                }
                match result {
                    Ok(workflow) => {
                        this.layers.close(Layer::WorkflowFromSkills);
                        this.workflow_skill_picks.clear();
                        this.edit_workflow(Some(&workflow), window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What the workflow is for, and the skills whose method documents it
    /// is written from; none picked lets the model pick at most three.
    fn from_skills_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut skills = div().h_flex().flex_wrap().gap_1();
        for (index, skill) in self.workflow_skills.iter().enumerate() {
            let name = skill.name.clone();
            let picked = self.workflow_skill_picks.contains(&name);
            skills = skills.child(
                Button::new(("workflow-skill", index))
                    .label(skill.name.clone())
                    .xsmall()
                    .selected(picked)
                    .tooltip(SharedString::from(skill.description.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let picks = &mut this.workflow_skill_picks;
                        match picks.iter().position(|kept| *kept == name) {
                            Some(at) => {
                                picks.remove(at);
                            }
                            None => picks.push(name.clone()),
                        }
                        cx.notify();
                    })),
            );
        }
        if self.workflow_skills.is_empty() {
            skills = skills.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("This project has no skills turned on.")),
            );
        }
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(660.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("A workflow from skills")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "The sub-agent model reads the skills' method documents and writes a workflow of \
                 agents from them, each task with its own instructions and result shape. It opens \
                 here to review before it is saved. Skills that bring scripts of their own cannot \
                 be turned into a workflow yet.",
            )))
            .child(
                div()
                    .h(px(70.))
                    .child(Input::new(&self.workflow_request).h_full()),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Skills to write it from; pick none and the model picks at most three.",
            )))
            .child(skills)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("workflow-skills-write")
                            .label(if self.workflow_converting {
                                tr("Writing the workflow…")
                            } else {
                                tr("Write the workflow")
                            })
                            .small()
                            .primary()
                            .disabled(self.workflow_converting)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.write_from_skills(window, cx)
                            })),
                    )
                    .child(
                        Button::new("workflow-skills-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::WorkflowFromSkills);
                                this.workflow_skill_picks.clear();
                                cx.notify();
                            })),
                    ),
            )
    }
}
