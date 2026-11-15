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
    WORKFLOW_CAPABILITIES, WORKFLOW_TASK_LIMIT, Workflow, WorkflowApproval, WorkflowTask,
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
        TaskDraft {
            title: field(&task.title, false, window, cx),
            instructions: field(&task.instructions, true, window, cx),
            after: task.after.clone(),
            capabilities: task.capabilities.clone(),
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
        for (field, value) in self.workflow_fields.iter().zip([&shown.name, &shown.goal]) {
            field.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.workflow_tasks = shown
            .tasks
            .iter()
            .map(|task| self.task_draft(task, window, cx))
            .collect();
        self.workflow_approval = shown.approval;
        self.workflow_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workflow_editing.clone() else {
            return;
        };
        let [name, goal] = [0, 1].map(|at| self.workflow_fields[at].read(cx).value().to_string());
        let tasks = self
            .workflow_tasks
            .iter()
            .map(|draft| WorkflowTask {
                title: draft.title.read(cx).value().to_string(),
                instructions: draft.instructions.read(cx).value().to_string(),
                after: draft.after.clone(),
                capabilities: draft.capabilities.clone(),
            })
            .collect();
        let reply = self.backend.save_workflow(Workflow {
            id,
            name,
            goal,
            tasks,
            approval: self.workflow_approval,
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
        form = form.child(approval);
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
                    .child(allowed),
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
        } else {
            page.child(
                div().pt_3().h_flex().child(
                    Button::new("workflow-add")
                        .label(tr("New workflow"))
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_workflow(None, window, cx)),
                        ),
                ),
            )
        }
    }
}
