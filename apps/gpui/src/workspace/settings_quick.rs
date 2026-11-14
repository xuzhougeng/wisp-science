//! The Quick actions page of Settings: requests kept under a name, to put
//! into the composer with one click. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{QuickAction, QuickActionInput};

use super::Workspace;
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
        self.quick_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_quick_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.quick_editing.clone() else {
            return;
        };
        let [name, prompt] = [0, 1].map(|at| self.quick_fields[at].read(cx).value().to_string());
        let reply = self
            .backend
            .save_quick_action(QuickActionInput { id, name, prompt });
        self.change_quick_actions(reply, window, cx);
    }

    fn quick_action_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(label(tr("Name")))
            .child(Input::new(&self.quick_fields[0]).small())
            .child(label(tr(
                "The request. End it where you will go on typing, for example \"Question: \".",
            )))
            .child(
                div()
                    .h(px(180.))
                    .child(Input::new(&self.quick_fields[1]).h_full()),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::QuickActionForm);
                                this.quick_editing = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn quick_action_row(&self, action: &QuickAction, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let id = action.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveQuickAction)
            && self.quick_editing.as_deref() == Some(id.as_str());
        let about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(action.name.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(action.prompt.lines().next().unwrap_or_default().to_owned()),
            );
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
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
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveQuickAction);
                            this.quick_editing = None;
                            cx.notify();
                        })),
                );
        } else {
            let editing = action.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("quick-edit-{id}")))
                        .label(tr("Change"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_quick_action(Some(&editing), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("quick-remove-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layers.close(Layer::QuickActionForm);
                            this.quick_editing = Some(id.clone());
                            this.layers.open(Layer::ConfirmRemoveQuickAction);
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

    pub(super) fn render_quick_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page =
            self.settings_page(cx)
                .child(div().pb_2().text_xs().text_color(muted).child(tr(
                    "Requests you make often, kept under a name. The composer's quick actions \
                 menu puts one into the message box, to finish and send.",
                )));
        let form_open = self.layers.contains(Layer::QuickActionForm);
        let editing = self.quick_editing.as_deref();
        if self.quick_actions.is_empty() {
            page = page.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("None yet.")),
            );
        }
        for action in &self.quick_actions {
            page = page.child(self.quick_action_row(action, cx));
            if form_open && editing == Some(action.id.as_str()) {
                page = page.child(div().py_2().child(self.quick_action_form(cx)));
            }
        }
        if form_open && editing == Some("") {
            page.child(div().py_2().child(self.quick_action_form(cx)))
        } else {
            page.child(
                div().pt_3().h_flex().child(
                    Button::new("quick-add")
                        .label(tr("Add a quick action"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit_quick_action(None, window, cx)
                        })),
                ),
            )
        }
    }
}
