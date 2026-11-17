//! The Quick actions page of Settings: requests kept under a name, to put
//! into the composer with one click, or workflows of the library started
//! as agents on what was typed. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{QuickAction, QuickActionInput};

use super::{
    Workspace,
    settings_sections::{setting, stacked_setting},
    surfaces,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
};

impl Workspace {
    pub(super) fn load_quick_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.quick_actions();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(actions) => this.quick_actions = actions,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the actions and shows the list the host kept.
    fn change_quick_actions(
        &mut self,
        reply: Reply<Vec<QuickAction>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(actions) => {
                        this.quick_actions = actions;
                        this.layers.close(Layer::QuickActionForm);
                        this.layers.close(Layer::ConfirmRemoveQuickAction);
                        this.quick_editing = None;
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

    /// Opens the form on a new action, or on `action` to change it.
    fn edit_quick_action(
        &mut self,
        action: Option<&QuickAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blank = QuickAction::default();
        let shown = action.unwrap_or(&blank);
        self.quick_editing = Some(shown.id.clone());
        self.layers.close(Layer::ConfirmRemoveQuickAction);
        self.layers.open(Layer::QuickActionForm);
        self.settings_error = None;
        for (field, value) in self.quick_fields.iter().zip([&shown.name, &shown.prompt]) {
            field.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.quick_workflow = shown.workflow.clone();
        // The workflows it may start.
        self.load_workflows(window, cx);
        self.quick_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_quick_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.quick_editing.clone() else {
            return;
        };
        let [name, prompt] = [0, 1].map(|at| self.quick_fields[at].read(cx).value().to_string());
        let reply = self.backend.save_quick_action(QuickActionInput {
            id,
            name,
            prompt,
            workflow: self.quick_workflow.clone(),
        });
        self.change_quick_actions(reply, window, cx);
    }

    /// The form of a new or changed action, under the list in the same
    /// column: a setting row for each field, then Save and Cancel.
    fn quick_action_form(&self, cx: &mut Context<Self>) -> Div {
        let adding = self.quick_editing.as_deref() == Some("");
        div()
            .v_flex()
            .w_full()
            .child(surfaces::settings_heading(
                if adding {
                    tr("Add a quick action")
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
                    .child(surfaces::input(&self.quick_fields[0], cx)),
                cx,
            ))
            .child(stacked_setting(
                tr("What it does"),
                "",
                self.quick_workflow_choice(cx),
                cx,
            ))
            .child(stacked_setting(
                tr("Prompt"),
                if self.quick_workflow.is_empty() {
                    tr("The request; end it where you will go on typing.")
                } else {
                    tr("Told to every task before what was typed. Optional.")
                },
                div()
                    .h(px(180.))
                    .child(surfaces::input(&self.quick_fields[1], cx).h_full()),
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        Button::new("quick-save")
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_quick_action(window, cx)
                            })),
                    )
                    .child(
                        Button::new("quick-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::QuickActionForm);
                                this.quick_editing = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    /// Whether the action puts its request into the composer or starts one
    /// of the library's workflows: one chip each, in a group that wraps.
    fn quick_workflow_choice(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div().h_flex().flex_wrap().gap_1().child(
            surfaces::choice_chip("quick-starts-none", self.quick_workflow.is_empty(), cx)
                .child(surfaces::sized_icon(
                    "chat",
                    surfaces::IconSize::Inline,
                    surfaces::IconTone::Faded,
                    cx,
                ))
                .child(tr("Puts the request into the composer"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.quick_workflow.clear();
                    cx.notify();
                })),
        );
        for (index, workflow) in self.workflows.iter().enumerate() {
            let id = workflow.id.clone();
            row = row.child(
                surfaces::choice_chip(
                    ("quick-starts", index),
                    self.quick_workflow == workflow.id,
                    cx,
                )
                .child(surfaces::sized_icon(
                    "swarm",
                    surfaces::IconSize::Inline,
                    surfaces::IconTone::Faded,
                    cx,
                ))
                .child(crate::i18n::fill(tr("Starts {}"), &[&workflow.name]))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.quick_workflow = id.clone();
                    cx.notify();
                })),
            );
        }
        row
    }

    /// One action as a record row: its name, the first line of its request
    /// (or the workflow it starts) after it; Change as an icon button,
    /// Remove last in the danger colour, the remove question under it.
    fn quick_action_row(
        &self,
        action: &QuickAction,
        cx: &mut Context<Self>,
    ) -> surfaces::RecordRow {
        let id = action.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveQuickAction)
            && self.quick_editing.as_deref() == Some(id.as_str());
        let what = if action.workflow.is_empty() {
            action.prompt.lines().next().unwrap_or_default().to_owned()
        } else {
            crate::i18n::fill(
                tr("Starts {} as agents on what was typed"),
                &[&self
                    .workflows
                    .iter()
                    .find(|workflow| workflow.id == action.workflow)
                    .map_or(action.workflow.as_str(), |workflow| workflow.name.as_str())],
            )
        };
        let mut row = surfaces::record_row(
            SharedString::from(format!("quick-{id}")),
            action.name.clone(),
        )
        .icon(if action.workflow.is_empty() {
            "file-play"
        } else {
            "swarm"
        });
        if !what.is_empty() {
            row = row.suffix(what);
        }
        let editing = action.clone();
        let removing = id.clone();
        row = row
            .action(
                surfaces::icon_action(
                    SharedString::from(format!("quick-edit-{id}")),
                    "edit",
                    tr("Change"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.edit_quick_action(Some(&editing), window, cx)
                })),
            )
            .action(
                surfaces::danger_action(
                    SharedString::from(format!("quick-remove-{id}")),
                    "trash",
                    tr("Remove"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.layers.close(Layer::QuickActionForm);
                    this.quick_editing = Some(removing.clone());
                    this.layers.open(Layer::ConfirmRemoveQuickAction);
                    cx.notify();
                })),
            );
        let question = asking.then(|| {
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(div().flex_1())
                .child(
                    Button::new(SharedString::from(format!("quick-remove-yes-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, window, cx| {
                                let reply = this.backend.delete_quick_action(id.clone());
                                this.change_quick_actions(reply, window, cx)
                            }
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("quick-remove-no-{id}")))
                        .label(tr("Keep"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveQuickAction);
                            this.quick_editing = None;
                            cx.notify();
                        })),
                )
                .into_any_element()
        });
        row.below(question)
    }

    pub(super) fn render_quick_settings(&self, cx: &mut Context<Self>) -> Div {
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("Requests you make often, kept under a name."),
                tr(
                    "Requests you make often, kept under a name. The composer's quick actions \
                 menu puts one into the message box, to finish and send, or starts a workflow \
                 of the library as agents on what is typed there.",
                ),
                cx,
            )
            .pb_3(),
        );
        let form_open = self.layers.contains(Layer::QuickActionForm);
        let editing = self.quick_editing.as_deref();
        if self.quick_actions.is_empty() {
            page = page.child(surfaces::settings_note(tr("None yet."), cx).pb_2());
        } else {
            let hover = cx.theme().list_hover;
            let paper = cx.theme().popover;
            let rows: Vec<_> = self
                .quick_actions
                .iter()
                .map(|action| {
                    let chosen = form_open && editing == Some(action.id.as_str());
                    self.quick_action_row(action, cx)
                        .when(chosen, |row| row.bg(paper))
                        .when(!chosen, |row| row.hover(move |style| style.bg(hover)))
                })
                .collect();
            page = page.child(surfaces::record_group(rows, cx));
        }
        if form_open {
            page.child(self.quick_action_form(cx))
        } else {
            page.child(
                div().pt_3().h_flex().child(
                    surfaces::outlined_action("quick-add", "plus", tr("Add a quick action"), cx)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit_quick_action(None, window, cx)
                        })),
                ),
            )
        }
    }
}
