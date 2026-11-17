//! The Experts page of Settings: the personas a conversation can be
//! started as. The form and the remove question are layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{Expert, ExpertInput};

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
        self.expert_skills = shown.skills.clone();
        self.expert_connectors = shown.connectors.clone();
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
            skills: self.expert_skills.clone(),
            connectors: self.expert_connectors.clone(),
        });
        self.change_experts(reply, window, cx);
    }

    /// The form of a new or changed expert, under the list in the same
    /// column: a setting row for each field, then Save and Cancel.
    fn expert_form(&self, builtin: bool, cx: &mut Context<Self>) -> Div {
        let field = |at: usize| {
            div()
                .w(px(300.))
                .child(surfaces::input(&self.expert_fields[at], cx))
        };
        let adding = self.expert_editing.as_deref() == Some("");
        let mut form = div()
            .v_flex()
            .w_full()
            .child(surfaces::settings_heading(
                if adding {
                    tr("Add an expert")
                } else {
                    tr("Change")
                },
                cx,
            ))
            .child(setting(tr("Name"), "", field(0), cx))
            .child(setting(
                tr("Description"),
                tr("For you: what the expert is for. The model is not shown it."),
                field(1),
                cx,
            ));
        form = if builtin {
            form.child(setting(
                tr("Instructions"),
                tr("This expert ships with Wisp: its instructions cannot be changed."),
                div(),
                cx,
            ))
        } else {
            form.child(stacked_setting(
                tr("Instructions"),
                tr("What the model works under for the whole conversation."),
                div()
                    .h(px(220.))
                    .child(surfaces::input(&self.expert_fields[2], cx).h_full()),
                cx,
            ))
        };
        // The model it runs on: the conversation's, or one of its own.
        let mut models = div().h_flex().flex_wrap().gap_1().child(
            surfaces::choice_chip("expert-model-own", self.expert_model.is_empty(), cx)
                .child(tr("The conversation's"))
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
                surfaces::choice_chip(("expert-model", index), self.expert_model == profile.id, cx)
                    .child(profile.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expert_model = id.clone();
                        cx.notify();
                    })),
            );
        }
        let form = form.child(stacked_setting(
            tr("Model"),
            tr("A conversation started as this expert runs on it."),
            models,
            cx,
        ));
        // What its conversations may use, as the WebView's specialists.
        let skills: Vec<(String, String)> = self
            .skills
            .iter()
            .flatten()
            .map(|skill| (skill.name.clone(), skill.name.clone()))
            .collect();
        let mut connectors: Vec<(String, String)> = self
            .connectors
            .iter()
            .flatten()
            .map(|row| (row.domain.clone(), row.domain.clone()))
            .collect();
        connectors.extend(
            self.mcp_servers
                .iter()
                .flatten()
                .map(|row| (row.server.id.clone(), row.server.name.clone())),
        );
        let form = form
            .child(stacked_setting(
                tr("Skills its conversations may use"),
                "",
                self.expert_reach_choice(true, &skills, cx),
                cx,
            ))
            .child(stacked_setting(
                tr("Connectors its conversations may use"),
                "",
                self.expert_reach_choice(false, &connectors, cx),
                cx,
            ));
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_3()
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
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ExpertForm);
                            this.expert_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    /// Every one, or only those picked from `options` (id and label): the
    /// skills (`skills`) or the connectors the expert in the form may use.
    /// The two ways are a segmented control; the ones to pick, chips.
    fn expert_reach_choice(
        &self,
        skills: bool,
        options: &[(String, String)],
        cx: &mut Context<Self>,
    ) -> Div {
        let kind = if skills { "skills" } else { "connectors" };
        let chosen = if skills {
            &self.expert_skills
        } else {
            &self.expert_connectors
        };
        let set = move |this: &mut Self, list: Option<Vec<String>>| {
            if skills {
                this.expert_skills = list;
            } else {
                this.expert_connectors = list;
            }
        };
        let ways = surfaces::segmented(
            [
                surfaces::plain_segment(
                    SharedString::from(format!("expert-{kind}-every")),
                    if skills {
                        tr("Every skill")
                    } else {
                        tr("Every connector")
                    },
                    chosen.is_none(),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    set(this, None);
                    cx.notify();
                }))
                .into_any_element(),
                surfaces::plain_segment(
                    SharedString::from(format!("expert-{kind}-only")),
                    tr("Only these"),
                    chosen.is_some(),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let current = if skills {
                        &this.expert_skills
                    } else {
                        &this.expert_connectors
                    };
                    if current.is_none() {
                        set(this, Some(Vec::new()));
                    }
                    cx.notify();
                }))
                .into_any_element(),
            ],
            cx,
        );
        let choice = div().h_flex().child(ways);
        let Some(picked) = chosen else {
            return choice;
        };
        let mut chips = div().h_flex().flex_wrap().gap_1();
        for (index, (id, name)) in options.iter().enumerate() {
            let on = picked.contains(id);
            let id = id.clone();
            chips = chips.child(
                surfaces::choice_chip(SharedString::from(format!("expert-{kind}-{index}")), on, cx)
                    .child(name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let current = if skills {
                            &this.expert_skills
                        } else {
                            &this.expert_connectors
                        };
                        let next = crate::settings::toggled(current, &id);
                        set(this, next);
                        cx.notify();
                    })),
            );
        }
        // Picked before but no longer there: shown, so it can be let go.
        for (index, id) in picked
            .iter()
            .filter(|id| !options.iter().any(|(known, _)| known == *id))
            .enumerate()
        {
            let gone = id.clone();
            chips = chips.child(
                surfaces::choice_chip(
                    SharedString::from(format!("expert-{kind}-gone-{index}")),
                    true,
                    cx,
                )
                .child(id.clone())
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(tr("Not found here any more"))
                        .build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let current = if skills {
                        &this.expert_skills
                    } else {
                        &this.expert_connectors
                    };
                    let next = crate::settings::toggled(current, &gone);
                    set(this, next);
                    cx.notify();
                })),
            );
        }
        div().v_flex().gap_2().child(choice).child(chips)
    }

    /// One expert as a record row: its name, its description (or where it
    /// comes from) after it; the model it runs on and what it may reach on
    /// lines under it. Change is an icon button, Remove
    /// last in the danger colour; the remove question opens under it.
    fn expert_row(&self, expert: &Expert, cx: &mut Context<Self>) -> surfaces::RecordRow {
        let muted = cx.theme().muted_foreground;
        let id = expert.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveExpert)
            && self.expert_editing.as_deref() == Some(id.as_str());
        let description = match (expert.description.is_empty(), expert.builtin) {
            (false, _) => expert.description.clone(),
            (true, true) => tr("Ships with Wisp").into(),
            (true, false) => String::new(),
        };
        let mut row = surfaces::record_row(
            SharedString::from(format!("expert-{id}")),
            expert.name.clone(),
        )
        .icon("user");
        if !description.is_empty() {
            row = row.suffix(description);
        }
        let mut under = div().v_flex().gap_0p5();
        let mut opened = false;
        if let Some(note) =
            crate::settings::expert_model_note(self.settings.current(), &expert.model)
        {
            opened = true;
            under = under.child(div().text_xs().text_color(muted).child(note));
        }
        if let Some(reach) = crate::settings::expert_reach(expert) {
            opened = true;
            under = under.child(div().text_xs().text_color(muted).child(reach));
        }
        if asking {
            opened = true;
            under = under.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(tr(
                                "Remove this expert? Conversations started as it keep the instructions \
                     they were given.",
                            )),
                    )
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
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveExpert);
                                this.expert_editing = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let editing = expert.clone();
        row =
            row.action(
                surfaces::icon_action(
                    SharedString::from(format!("expert-edit-{id}")),
                    "edit",
                    tr("Change"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.edit_expert(Some(&editing), window, cx)
                })),
            );
        if !expert.builtin {
            let removing = id.clone();
            row = row.action(
                surfaces::danger_action(
                    SharedString::from(format!("expert-remove-{id}")),
                    "trash",
                    tr("Remove"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.layers.close(Layer::ExpertForm);
                    this.expert_editing = Some(removing.clone());
                    this.layers.open(Layer::ConfirmRemoveExpert);
                    cx.notify();
                })),
            );
        }
        row.below(opened.then(|| under.into_any_element()))
    }

    pub(super) fn render_expert_settings(&self, cx: &mut Context<Self>) -> Div {
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("Personas a conversation can be started as."),
                tr(
                    "An expert is a persona a conversation is started as: a name and the \
                 instructions the model works under throughout. Choose one in the composer \
                 before the first message.",
                ),
                cx,
            )
            .pb_3(),
        );
        let form_open = self.layers.contains(Layer::ExpertForm);
        let editing = self.expert_editing.as_deref();
        let hover = cx.theme().list_hover;
        let paper = cx.theme().popover;
        let rows: Vec<_> = self
            .experts
            .iter()
            .map(|expert| {
                let chosen = form_open && editing == Some(expert.id.as_str());
                self.expert_row(expert, cx)
                    .when(chosen, |row| row.bg(paper))
                    .when(!chosen, |row| row.hover(move |style| style.bg(hover)))
            })
            .collect();
        if !rows.is_empty() {
            page = page.child(surfaces::record_group(rows, cx));
        }
        if form_open {
            let builtin = self
                .experts
                .iter()
                .find(|expert| Some(expert.id.as_str()) == editing)
                .is_some_and(|expert| expert.builtin);
            page.child(self.expert_form(builtin, cx))
        } else {
            page.child(div().pt_3().h_flex().child(
                surfaces::outlined_action("expert-add", "plus", tr("Add an expert"), cx).on_click(
                    cx.listener(|this, _, window, cx| this.edit_expert(None, window, cx)),
                ),
            ))
        }
    }
}
