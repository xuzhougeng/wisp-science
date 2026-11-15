//! The Experts page of Settings: the personas a conversation can be
//! started as. The form and the remove question are layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{Expert, ExpertInput};

use super::Workspace;
use crate::{
    backend::{Reply, receive},
    layers::Layer,
};

impl Workspace {
    pub(super) fn load_experts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.experts();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(experts) => this.experts = experts,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the experts and shows the list the host kept.
    fn change_experts(
        &mut self,
        reply: Reply<Vec<Expert>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(experts) => {
                        this.experts = experts;
                        this.layers.close(Layer::ExpertForm);
                        this.layers.close(Layer::ConfirmRemoveExpert);
                        this.expert_editing = None;
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

    /// Opens the form on a new expert, or on `expert` to change it.
    fn edit_expert(
        &mut self,
        expert: Option<&Expert>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blank = Expert::default();
        let shown = expert.unwrap_or(&blank);
        self.expert_editing = Some(shown.id.clone());
        self.expert_model = shown.model.clone();
        self.layers.close(Layer::ConfirmRemoveExpert);
        self.layers.open(Layer::ExpertForm);
        self.settings_error = None;
        let values = [&shown.name, &shown.description, &shown.instructions];
        for (field, value) in self.expert_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.expert_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_expert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.expert_editing.clone() else {
            return;
        };
        let [name, description, instructions] =
            [0, 1, 2].map(|at| self.expert_fields[at].read(cx).value().to_string());
        let reply = self.backend.save_expert(ExpertInput {
            id,
            name,
            description,
            instructions,
            model: self.expert_model.clone(),
        });
        self.change_experts(reply, window, cx);
    }

    fn expert_form(&self, builtin: bool, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut form = div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(label(tr("Name")))
            .child(Input::new(&self.expert_fields[0]).small())
            .child(label(tr(
                "Description, for you: what the expert is for. The model is not shown it.",
            )))
            .child(Input::new(&self.expert_fields[1]).small());
        form = if builtin {
            form.child(label(tr(
                "This expert ships with Wisp: its instructions cannot be changed.",
            )))
        } else {
            form.child(label(tr(
                "Instructions: what the model works under for the whole conversation.",
            )))
            .child(
                div()
                    .h(px(220.))
                    .child(Input::new(&self.expert_fields[2]).h_full()),
            )
        };
        // The model it runs on: the conversation's, or one of its own.
        let mut models = div().h_flex().flex_wrap().gap_1().child(
            Button::new("expert-model-own")
                .label(tr("The conversation's"))
                .small()
                .selected(self.expert_model.is_empty())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expert_model.clear();
                    cx.notify();
                })),
        );
        let profiles = self
            .settings
            .current()
            .map(|settings| settings.profiles.clone())
            .unwrap_or_default();
        for (index, profile) in profiles.into_iter().enumerate() {
            let id = profile.id.clone();
            models = models.child(
                Button::new(("expert-model", index))
                    .label(profile.name.clone())
                    .small()
                    .selected(self.expert_model == profile.id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expert_model = id.clone();
                        cx.notify();
                    })),
            );
        }
        let form = form
            .child(label(tr(
                "Model: a conversation started as this expert runs on it.",
            )))
            .child(models);
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("expert-save")
                        .label(tr("Save"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.save_expert(window, cx))),
                )
                .child(
                    Button::new("expert-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ExpertForm);
                            this.expert_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    fn expert_row(&self, expert: &Expert, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let id = expert.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveExpert)
            && self.expert_editing.as_deref() == Some(id.as_str());
        let mut about = div().v_flex().flex_1().min_w_0().gap_1().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(expert.name.clone()),
        );
        let description = match (expert.description.is_empty(), expert.builtin) {
            (false, _) => expert.description.clone(),
            (true, true) => tr("Ships with Wisp").into(),
            (true, false) => String::new(),
        };
        if let Some(note) =
            crate::settings::expert_model_note(self.settings.current(), &expert.model)
        {
            about = about.child(div().text_xs().text_color(muted).child(note));
        }
        if !description.is_empty() {
            about = about.child(div().text_xs().text_color(muted).child(description));
        }
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("expert-remove-yes-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, window, cx| {
                                let reply = this.backend.delete_expert(id.clone());
                                this.change_experts(reply, window, cx)
                            }
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("expert-remove-no-{id}")))
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveExpert);
                            this.expert_editing = None;
                            cx.notify();
                        })),
                );
        } else {
            let editing = expert.clone();
            actions = actions.child(
                Button::new(SharedString::from(format!("expert-edit-{id}")))
                    .label(tr("Change"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_expert(Some(&editing), window, cx)
                    })),
            );
            if !expert.builtin {
                actions = actions.child(
                    Button::new(SharedString::from(format!("expert-remove-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layers.close(Layer::ExpertForm);
                            this.expert_editing = Some(id.clone());
                            this.layers.open(Layer::ConfirmRemoveExpert);
                            cx.notify();
                        })),
                );
            }
        }
        let mut row = div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(actions);
        if asking {
            row = div().v_flex().child(row).child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(tr(
                        "Remove this expert? Conversations started as it keep the instructions \
                     they were given.",
                    )),
            );
        }
        row
    }

    pub(super) fn render_expert_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page =
            self.settings_page(cx)
                .child(div().pb_2().text_xs().text_color(muted).child(tr(
                    "An expert is a persona a conversation is started as: a name and the \
                 instructions the model works under throughout. Choose one in the composer \
                 before the first message.",
                )));
        let form_open = self.layers.contains(Layer::ExpertForm);
        let editing = self.expert_editing.as_deref();
        for expert in &self.experts {
            page = page.child(self.expert_row(expert, cx));
            if form_open && editing == Some(expert.id.as_str()) {
                page = page.child(div().py_2().child(self.expert_form(expert.builtin, cx)));
            }
        }
        if form_open && editing == Some("") {
            page = page.child(div().py_2().child(self.expert_form(false, cx)));
        } else {
            page = page.child(
                div().pt_3().h_flex().child(
                    Button::new("expert-add")
                        .label(tr("Add an expert"))
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_expert(None, window, cx)),
                        ),
                ),
            );
        }
        page
    }
}
