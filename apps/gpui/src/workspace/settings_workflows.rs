//! The Workflows page of Settings: templates of a goal broken into tasks,
//! run as one conversation of the open project, or as agents, one per
//! task, after the researcher approves them.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::InputState,
};
use wisp_server::desktop::{
    ModelTier, WORKFLOW_CAPABILITIES, WORKFLOW_TASK_LIMIT, Workflow, WorkflowAgentSettings,
    WorkflowApproval, WorkflowTask,
};

use super::{
    Workspace,
    settings_sections::{setting, stacked_setting},
    surfaces,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    panel::PanelTab,
    workflows,
};

/// A task dragged on the workflow's map, from the form open when the drag
/// began, to be dropped on the task that is to need it.
#[derive(Clone)]
struct MapLink {
    task: usize,
    form: Option<String>,
}

/// What follows the pointer while a task is dragged on the map: nothing,
/// as the map draws the line from the task to the pointer itself.
struct MapLinkGhost;

impl Render for MapLinkGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

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

    /// The form of a new or changed workflow, under the list in the same
    /// column: its fields as setting rows, the map, then each task under a
    /// heading of its own; Add task, Save and Cancel at the end.
    fn workflow_form(&self, cx: &mut Context<Self>) -> Div {
        let adding = self.workflow_editing.as_deref() == Some("");
        let mut form = div()
            .v_flex()
            .w_full()
            .child(surfaces::settings_heading(
                if adding {
                    tr("New workflow")
                } else {
                    tr("Change")
                },
                cx,
            ))
            .child(setting(
                tr("Name"),
                "",
                div()
                    .w(px(300.))
                    .child(surfaces::input(&self.workflow_fields[0], cx)),
                cx,
            ))
            .child(stacked_setting(
                tr("Goal"),
                tr("What the whole workflow is to achieve."),
                div()
                    .h(px(90.))
                    .child(surfaces::input(&self.workflow_fields[1], cx).h_full()),
                cx,
            ))
            .child(stacked_setting(
                tr("Context"),
                tr("Optional: scope and conventions, told to every task."),
                div()
                    .h(px(60.))
                    .child(surfaces::input(&self.workflow_fields[2], cx).h_full()),
                cx,
            ));
        // How a run as agents is approved.
        let approval = surfaces::segmented(
            [WorkflowApproval::ReviewAll, WorkflowApproval::AutoSafe]
                .into_iter()
                .enumerate()
                .map(|(number, choice)| {
                    surfaces::plain_segment(
                        ("workflow-approval", number),
                        workflows::approval_label(choice),
                        self.workflow_approval == choice,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.workflow_approval = choice;
                        cx.notify();
                    }))
                    .into_any_element()
                }),
            cx,
        );
        form = form
            .child(setting(tr("Run as agents"), "", approval, cx))
            .child(stacked_setting(
                tr("Map"),
                tr("Drag a task onto another, or click two, to link them."),
                self.workflow_map(cx),
                cx,
            ));
        for (index, draft) in self.workflow_tasks.iter().enumerate() {
            // What its agent may do; none is reading the project.
            let mut allowed = div().h_flex().flex_wrap().gap_1();
            for (number, capability) in WORKFLOW_CAPABILITIES.into_iter().enumerate() {
                allowed = allowed.child(
                    surfaces::choice_chip(
                        ("workflow-capability", index * 100 + number),
                        draft.capabilities.iter().any(|kept| kept == capability),
                        cx,
                    )
                    .child(workflows::capability_label(capability))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.workflow_tasks.get_mut(index) {
                            draft.capabilities =
                                workflows::toggled_capability(&draft.capabilities, capability);
                            cx.notify();
                        }
                    })),
                );
            }
            let mut needs = div().h_flex().flex_wrap().gap_1();
            for earlier in 0..index {
                needs = needs.child(
                    surfaces::choice_chip(
                        ("workflow-after", index * 100 + earlier),
                        draft.after.contains(&earlier),
                        cx,
                    )
                    .min_w(px(28.))
                    .justify_center()
                    .child(surfaces::fact((earlier + 1).to_string(), cx).text_color(
                        if draft.after.contains(&earlier) {
                            cx.theme().primary
                        } else {
                            cx.theme().foreground
                        },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.workflow_tasks.get_mut(index) {
                            draft.after = workflows::toggled(&draft.after, earlier);
                            cx.notify();
                        }
                    })),
                );
            }
            form = form
                .child(
                    div()
                        .h_flex()
                        .items_end()
                        .gap_2()
                        .child(
                            surfaces::settings_heading(
                                crate::i18n::fill(tr("Task {}"), &[&(index + 1)]),
                                cx,
                            )
                            .flex_1(),
                        )
                        .child(
                            surfaces::danger_action(
                                ("workflow-task-remove", index),
                                "trash",
                                tr("Remove"),
                                cx,
                            )
                            .disabled(self.workflow_tasks.len() == 1)
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.remove_workflow_task(index, cx),
                            )),
                        ),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .py_3()
                        .border_t_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(surfaces::input(&draft.title, cx))
                        .child(
                            div()
                                .h(px(84.))
                                .child(surfaces::input(&draft.instructions, cx).h_full()),
                        ),
                );
            if index > 0 {
                form = form.child(stacked_setting(tr("After task"), "", needs, cx));
            }
            form = form
                .child(stacked_setting(tr("As an agent it may"), "", allowed, cx))
                .child(self.agent_settings(index, draft, cx));
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_3()
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
                    surfaces::outlined_action("workflow-task-add", "plus", tr("Add task"), cx)
                        .disabled(self.workflow_tasks.len() >= WORKFLOW_TASK_LIMIT)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let draft = this.task_draft(&WorkflowTask::default(), window, cx);
                            draft.title.update(cx, |input, cx| input.focus(window, cx));
                            this.workflow_tasks.push(draft);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("workflow-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .ghost()
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
    /// an edge to each task that needs it. Dragging a task onto another
    /// makes that one need it, as on the WebView's graph editor; clicking a
    /// task and then another makes the one listed later need the other, or
    /// no longer need it.
    fn workflow_map(&self, cx: &mut Context<Self>) -> Stateful<Div> {
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
        let primary = cx.theme().primary;
        // While a task is dragged: a line from it to the pointer.
        let dragging = self
            .workflow_drag
            .filter(|(task, _)| cx.has_active_drag() && *task < places.len());
        let lines = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let curve = |path: &mut PathBuilder, start: Point<Pixels>, end: Point<Pixels>| {
                    let middle = (start.x + end.x) / 2.;
                    path.move_to(start);
                    path.cubic_bezier_to(end, point(middle, start.y), point(middle, end.y));
                };
                let out = |(x, y): (f32, f32)| {
                    point(
                        bounds.origin.x + px(x + WIDTH),
                        bounds.origin.y + px(y + HEIGHT / 2.),
                    )
                };
                let mut path = PathBuilder::stroke(px(1.5));
                for &(from, to) in &edges {
                    let (tx, ty) = lines_at[to];
                    let end = point(
                        bounds.origin.x + px(tx),
                        bounds.origin.y + px(ty + HEIGHT / 2.),
                    );
                    curve(&mut path, out(lines_at[from]), end);
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, edge);
                }
                if let Some((task, at)) = dragging {
                    let mut path = PathBuilder::stroke(px(1.5));
                    curve(&mut path, out(lines_at[task]), bounds.origin + at);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, primary);
                    }
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
            .on_drag_move(cx.listener(|this, moved: &DragMoveEvent<MapLink>, _, cx| {
                let task = moved.drag(cx).task;
                this.workflow_drag = Some((task, moved.event.position - moved.bounds.origin));
                cx.notify();
            }))
            // A drag let go anywhere but on a task leaves no line behind.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_map_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_map_drag(cx)),
            )
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
            let link = MapLink {
                task,
                form: self.workflow_editing.clone(),
            };
            let target = primary.opacity(0.2);
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
                    .on_click(cx.listener(move |this, _, _, cx| this.click_map(task, cx)))
                    // Dragged onto another task, that one comes to need it;
                    // a press that does not move is still a click.
                    .on_drag(link, |_, _, _, cx| cx.new(|_| MapLinkGhost))
                    .drag_over::<MapLink>(move |style, link, _, _| {
                        if link.task == task {
                            style
                        } else {
                            style.border_color(primary).bg(target)
                        }
                    })
                    .on_drop(cx.listener(move |this, link: &MapLink, _, cx| {
                        this.drop_on_map(link, task, cx)
                    })),
            );
        }
        div()
            .id("workflow-map")
            .max_w_full()
            .overflow_x_scroll()
            .child(board)
    }

    /// A task dragged on the map was dropped on task `task`, which comes to
    /// need it; the tasks are put back in an order where each needs only
    /// tasks before it. A drop that would make a circle joins nothing and
    /// says why.
    fn drop_on_map(&mut self, link: &MapLink, task: usize, cx: &mut Context<Self>) {
        self.workflow_drag = None;
        if link.form != self.workflow_editing {
            return cx.notify();
        }
        let after: Vec<Vec<usize>> = self
            .workflow_tasks
            .iter()
            .map(|draft| draft.after.clone())
            .collect();
        match workflows::dropped(&after, link.task, task) {
            Ok(Some((order, needs))) => {
                let mut drafts: Vec<Option<TaskDraft>> =
                    self.workflow_tasks.drain(..).map(Some).collect();
                self.workflow_tasks = order
                    .iter()
                    .filter_map(|&old| drafts.get_mut(old).and_then(Option::take))
                    .collect();
                for (draft, needs) in self.workflow_tasks.iter_mut().zip(needs) {
                    draft.after = needs;
                }
                // Places may have moved, so a pick may now be another task.
                self.workflow_pick = None;
                self.settings_error = None;
            }
            Ok(None) => {}
            Err(workflows::DropRefused::Circle) => {
                self.settings_error =
                    Some(tr("Not joined: the tasks would need each other in a circle.").to_owned());
            }
        }
        cx.notify();
    }

    fn end_map_drag(&mut self, cx: &mut Context<Self>) {
        if self.workflow_drag.take().is_some() {
            cx.notify();
        }
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
    /// away until asked for or set; open, each is a setting row.
    fn agent_settings(&self, index: usize, draft: &TaskDraft, cx: &mut Context<Self>) -> Div {
        let toggle = surfaces::labelled_action(
            ("workflow-agent-open", index),
            if draft.open {
                "chevron-up"
            } else {
                "chevron-down"
            },
            if draft.open {
                tr("Fewer agent settings")
            } else {
                tr("Agent settings")
            },
            surfaces::IconSize::Inline,
            surfaces::IconTone::Faded,
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(draft) = this.workflow_tasks.get_mut(index) {
                draft.open = !draft.open;
                cx.notify();
            }
        }));
        let mut section = div().v_flex().child(div().h_flex().py_1().child(toggle));
        if !draft.open {
            return section;
        }
        // Which expert, if any.
        let mut experts = div().h_flex().flex_wrap().gap_1();
        let choices = std::iter::once((None, tr("No expert").to_owned())).chain(
            self.experts
                .iter()
                .map(|expert| (Some(expert.id.clone()), expert.name.clone())),
        );
        for (number, (id, name)) in choices.enumerate() {
            let selected = draft.agent.expert == id;
            experts = experts.child(
                surfaces::choice_chip(("workflow-expert", index * 1000 + number), selected, cx)
                    .child(name)
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
            experts = experts.child(
                surfaces::choice_chip(("workflow-expert-gone", index), true, cx)
                    .child(tr("(an expert that is gone)"))
                    .child(gone.clone()),
            );
        }
        let models = surfaces::segmented(
            [None, Some(ModelTier::Fast), Some(ModelTier::Inherit)]
                .into_iter()
                .enumerate()
                .map(|(number, model)| {
                    surfaces::plain_segment(
                        ("workflow-model", index * 10 + number),
                        workflows::model_label(model),
                        draft.agent.model == model,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.workflow_tasks.get_mut(index) {
                            draft.agent.model = model;
                            cx.notify();
                        }
                    }))
                    .into_any_element()
                }),
            cx,
        );
        let isolated = surfaces::switch(
            ("workflow-isolated", index),
            draft.agent.isolated,
            tr("Works in a Git worktree of its own; its changes are merged back when it succeeds"),
        )
        .on_change(cx.listener(move |this, _, _, cx| {
            if let Some(draft) = this.workflow_tasks.get_mut(index) {
                draft.agent.isolated = !draft.agent.isolated;
                cx.notify();
            }
        }));
        section = section
            .child(stacked_setting(tr("Done as"), "", experts, cx))
            .child(setting(tr("Model"), "", models, cx))
            .child(setting(tr("Own worktree"), "", isolated, cx))
            .child(setting(
                tr("Token budget"),
                "",
                div()
                    .w(px(140.))
                    .child(surfaces::input(&draft.tokens, cx).small()),
                cx,
            ))
            .child(setting(
                tr("Time limit (seconds, 0 for none)"),
                "",
                div()
                    .w(px(140.))
                    .child(surfaces::input(&draft.seconds, cx).small()),
                cx,
            ))
            .child(stacked_setting(
                tr("Result shape"),
                tr("A JSON Schema its result must match; empty for none"),
                div()
                    .h(px(70.))
                    .child(surfaces::input(&draft.shape, cx).h_full()),
                cx,
            ));
        section
    }

    /// One workflow as a record row: its name and its shape (tasks and
    /// stages); its goal's first line under it. Run is the action used
    /// most, outlined; Run as agents and Change are icon buttons, Remove
    /// last in the danger colour. The remove question opens under it.
    fn workflow_row(&self, workflow: &Workflow, cx: &mut Context<Self>) -> surfaces::RecordRow {
        let muted = cx.theme().muted_foreground;
        let id = workflow.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveWorkflow)
            && self.workflow_editing.as_deref() == Some(id.as_str());
        let goal = workflow.goal.lines().next().unwrap_or_default().to_owned();
        let mut under = div().v_flex().gap_1();
        let opened = asking || !goal.is_empty();
        if !goal.is_empty() {
            under = under.child(div().truncate().text_xs().text_color(muted).child(goal));
        }
        if asking {
            let removed = id.clone();
            under = under.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1())
                    .child(
                        Button::new(SharedString::from(format!("workflow-remove-yes-{id}")))
                            .label(tr("Remove"))
                            .small()
                            .danger()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let reply = this.backend.delete_workflow(removed.clone());
                                this.change_workflows(reply, window, cx)
                            })),
                    )
                    .child(
                        Button::new("workflow-remove-no")
                            .label(tr("Keep"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveWorkflow);
                                this.workflow_editing = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let editing = workflow.clone();
        let (run, agents, removing) = (id.clone(), id.clone(), id.clone());
        surfaces::record_row(
            SharedString::from(format!("workflow-{id}")),
            workflow.name.clone(),
        )
        .icon("split")
        .suffix(workflows::shape_line(workflow))
        .action(
            surfaces::outlined_action(
                SharedString::from(format!("workflow-run-{id}")),
                "play",
                tr("Run"),
                cx,
            )
            .disabled(self.state.project.is_none())
            .tooltip(tr("Run it as one conversation"))
            .on_click(
                cx.listener(move |this, _, window, cx| this.run_workflow(run.clone(), window, cx)),
            ),
        )
        .action(
            surfaces::icon_action(
                SharedString::from(format!("workflow-agents-{id}")),
                "swarm",
                tr("Each task as its own agent, with what it may do; approved in the Agents tab"),
                surfaces::IconTone::Faded,
                cx,
            )
            .disabled(
                self.state.project.is_none()
                    || workflow.tasks.len() > wisp_server::desktop::WORKFLOW_AGENT_LIMIT,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.run_workflow_as_agents(agents.clone(), window, cx)
            })),
        )
        .action(
            surfaces::icon_action(
                SharedString::from(format!("workflow-edit-{id}")),
                "edit",
                tr("Change"),
                surfaces::IconTone::Faded,
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.edit_workflow(Some(&editing), window, cx)
            })),
        )
        .action(
            surfaces::danger_action(
                SharedString::from(format!("workflow-remove-{id}")),
                "trash",
                tr("Remove"),
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.layers.close(Layer::WorkflowForm);
                this.workflow_tasks.clear();
                this.workflow_editing = Some(removing.clone());
                this.layers.open(Layer::ConfirmRemoveWorkflow);
                cx.notify();
            })),
        )
        .below(opened.then(|| under.into_any_element()))
    }

    pub(super) fn render_workflow_settings(&self, cx: &mut Context<Self>) -> Div {
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("A goal broken into tasks, run as one conversation or as agents."),
                tr(
                    "A workflow is a goal broken into tasks that say which earlier tasks they \
                     need. Run starts a conversation in the open project and gives the model \
                     the tasks stage by stage; it carries them out in that order itself, as \
                     one conversation with the usual approvals. Run as agents gives each task \
                     an agent of its own that may do only what the task allows; the plan waits \
                     in the new conversation's Agents tab to be approved, and the result comes \
                     back to that conversation.",
                ),
                cx,
            )
            .pb_3(),
        );
        if self.state.project.is_none() {
            page = page.child(
                surfaces::settings_note(tr("Open a project to run a workflow in it."), cx).pb_2(),
            );
        }
        let form_open = self.layers.contains(Layer::WorkflowForm);
        let editing = self.workflow_editing.as_deref();
        if self.workflows.is_empty() {
            page = page.child(surfaces::settings_note(tr("None yet."), cx).pb_2());
        } else {
            let hover = cx.theme().list_hover;
            let paper = cx.theme().popover;
            let rows: Vec<_> = self
                .workflows
                .iter()
                .map(|workflow| {
                    let chosen = form_open && editing == Some(workflow.id.as_str());
                    self.workflow_row(workflow, cx)
                        .when(chosen, |row| row.bg(paper))
                        .when(!chosen, |row| row.hover(move |style| style.bg(hover)))
                })
                .collect();
            page = page.child(surfaces::record_group(rows, cx));
        }
        if form_open {
            page.child(self.workflow_form(cx))
        } else if self.layers.contains(Layer::WorkflowFromSkills) {
            page.child(self.from_skills_form(cx))
        } else {
            page.child(
                div()
                    .pt_3()
                    .h_flex()
                    .gap_2()
                    .child(
                        surfaces::outlined_action("workflow-add", "plus", tr("New workflow"), cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_workflow(None, window, cx)
                            })),
                    )
                    .child(
                        surfaces::labelled_action(
                            "workflow-from-skills",
                            "skill",
                            tr("From skills"),
                            surfaces::IconSize::Inline,
                            surfaces::IconTone::Faded,
                            cx,
                        )
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
    /// is written from; none picked lets the model pick at most three. In
    /// the page's column under the list, as setting rows.
    fn from_skills_form(&self, cx: &mut Context<Self>) -> Div {
        let mut skills = div().h_flex().flex_wrap().gap_1();
        for (index, skill) in self.workflow_skills.iter().enumerate() {
            let name = skill.name.clone();
            let picked = self.workflow_skill_picks.contains(&name);
            let about = SharedString::from(skill.description.clone());
            skills = skills.child(
                surfaces::choice_chip(("workflow-skill", index), picked, cx)
                    .child(skill.name.clone())
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(about.clone()).build(window, cx)
                    })
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
            skills = skills.child(surfaces::settings_note(
                tr("This project has no skills turned on."),
                cx,
            ));
        }
        div()
            .v_flex()
            .w_full()
            .child(surfaces::settings_heading(tr("A workflow from skills"), cx))
            .child(
                surfaces::settings_note_help(tr("The sub-agent model writes a workflow from skills."),
                    tr(
                        "The sub-agent model reads the skills' method documents and writes a workflow of \
                 agents from them, each task with its own instructions and result shape. It opens \
                 here to review before it is saved. Skills that bring scripts of their own cannot \
                 be turned into a workflow yet.",
                    ),
                    cx,
                )
                .pb_1(),
            )
            .child(stacked_setting(
                tr("What it is for"),
                "",
                div()
                    .h(px(70.))
                    .child(surfaces::input(&self.workflow_request, cx).h_full()),
                cx,
            ))
            .child(stacked_setting(
                tr("Skills"),
                tr("Skills to write it from; none lets the model pick three."),
                skills,
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        surfaces::outlined_action(
                            "workflow-skills-write",
                            "sparkles",
                            if self.workflow_converting {
                                tr("Writing the workflow…")
                            } else {
                                tr("Write the workflow")
                            },
                            cx,
                        )
                        .disabled(self.workflow_converting)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.write_from_skills(window, cx)
                        })),
                    )
                    .child(
                        Button::new("workflow-skills-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::WorkflowFromSkills);
                                this.workflow_skill_picks.clear();
                                cx.notify();
                            })),
                    ),
            )
    }
}
