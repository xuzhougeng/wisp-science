//! The Decision model page of Settings: one fast endpoint that answers
//! yes/no, choice and score questions, used by the model's map_items tool.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::{DECISION_PRESETS, DECISION_THINK_MODES, DecisionModelSettings, DecisionPreset};

use super::{
    Workspace,
    settings_sections::{explained_setting, setting},
    shell::glyph,
    surfaces::{self, switch},
};
use crate::{
    backend::{Reply, receive},
    decision,
};

impl Workspace {
    pub(super) fn load_decision_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.decision_note = None;
        let reply = self.backend.decision_model();
        self.receive_decision_model(reply, None, window, cx);
    }

    /// Takes what the host keeps: it becomes the draft and fills the
    /// fields, and the key field is emptied. `note` is said on success.
    fn receive_decision_model(
        &mut self,
        reply: Reply<DecisionModelSettings>,
        note: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(kept) => {
                        this.show_decision_draft(kept.clone(), window, cx);
                        this.decision_fields[3]
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.decision = Some(kept);
                        this.decision_note = note.map(|note| tr(note).to_owned());
                    }
                    // A refused form changes nothing and keeps what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Makes `draft` the form: its address, model and timeout go into the
    /// fields.
    fn show_decision_draft(
        &mut self,
        draft: DecisionModelSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (field, value) in self.decision_fields.iter().zip(decision::values(&draft)) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.decision_draft = draft;
    }

    /// The form as it stands, and the key typed into it.
    fn typed_decision_model(&self, cx: &App) -> (DecisionModelSettings, Option<String>) {
        let [url, model, timeout, key] = self
            .decision_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let key = key.trim().to_owned();
        (
            decision::form(&self.decision_draft, [url, model, timeout]),
            (!key.is_empty()).then_some(key),
        )
    }

    fn save_decision_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        self.decision_note = None;
        let (form, key) = self.typed_decision_model(cx);
        let reply = self.backend.save_decision_model(form, key);
        self.receive_decision_model(reply, Some("Saved"), window, cx);
        cx.notify();
    }

    fn test_decision_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.decision_testing {
            return;
        }
        self.settings_error = None;
        self.decision_note = None;
        self.decision_testing = true;
        let (form, key) = self.typed_decision_model(cx);
        let reply = self.backend.test_decision_model(form, key);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.decision_testing = false;
                match result {
                    Ok(probe) => this.decision_note = Some(decision::probe_note(&probe)),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_decision_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(saved) = &self.decision else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let draft = &self.decision_draft;
        let (form, key) = self.typed_decision_model(cx);
        let enabled = draft.enabled;
        let stored = saved.has_key(&draft.preset);

        // One service of a few, and one way of thinking of a few: a
        // segmented control each.
        let services = surfaces::segmented(
            DECISION_PRESETS.iter().map(|preset| {
                surfaces::plain_segment(
                    SharedString::from(format!("decision-preset-{}", preset.id)),
                    decision::preset_label(preset.id),
                    draft.preset == preset.id,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let next = decision::picked(&this.decision_draft, preset);
                    this.show_decision_draft(next, window, cx);
                    this.decision_note = None;
                    this.settings_error = None;
                    cx.notify();
                }))
                .into_any_element()
            }),
            cx,
        );
        let thinking = surfaces::segmented(
            DECISION_THINK_MODES.iter().map(|mode| {
                surfaces::plain_segment(
                    SharedString::from(format!("decision-think-{mode}")),
                    decision::think_label(mode),
                    draft.think_mode == *mode,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.decision_draft.think_mode = (*mode).to_owned();
                    this.decision_note = None;
                    this.settings_error = None;
                    cx.notify();
                }))
                .into_any_element()
            }),
            cx,
        );
        let field = |index: usize, width: f32| {
            div()
                .w(px(width))
                .child(surfaces::input(&self.decision_fields[index], cx).small())
        };
        let mut key_control = div().h_flex().items_center().gap_1().child(field(3, 260.));
        if stored {
            let preset = draft.preset.clone();
            key_control = key_control.child(
                surfaces::danger_action("decision-forget-key", "trash", tr("Forget key"), cx)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.settings_error = None;
                        let reply = this.backend.clear_decision_model_key(preset.clone());
                        this.receive_decision_model(reply, None, window, cx);
                    })),
            );
        }
        let needs_key =
            DecisionPreset::find(&draft.preset).is_some_and(|preset| preset.key_required);
        let key_note = match (stored, needs_key) {
            (true, _) => tr("A key is saved for this service. Type another to replace it."),
            (false, true) => tr("This service needs an API key. It is kept in the system keyring."),
            (false, false) => tr("Only if the server was started with one."),
        };

        let mut actions = div()
            .h_flex()
            .items_center()
            .gap_2()
            .pt_4()
            .child(
                Button::new("decision-save")
                    .icon(glyph("save"))
                    .label(tr("Save"))
                    .small()
                    .primary()
                    .disabled(!decision::changed(
                        &form,
                        saved,
                        key.as_deref().unwrap_or_default(),
                    ))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.save_decision_model(window, cx)),
                    ),
            )
            .child(
                surfaces::outlined_action(
                    "decision-test",
                    "bolt",
                    if self.decision_testing {
                        tr("Testing…")
                    } else {
                        tr("Test connection")
                    },
                    cx,
                )
                .disabled(self.decision_testing)
                .on_click(cx.listener(|this, _, window, cx| this.test_decision_model(window, cx))),
            );
        if let Some(note) = &self.decision_note {
            actions = actions.child(surfaces::settings_note(note.clone(), cx));
        }

        page.child(
            div()
                .v_flex()
                .gap_1()
                .pb_2()
                .child(surfaces::settings_note_help(
                    tr("Answers typed questions with a probability for each option."),
                    tr(
                        "A decision model answers yes/no, choice and score questions with a \
                     probability for every option instead of writing text, usually within \
                     milliseconds. With one in use, the model's map_items tool can decide \
                     over many items at once. It applies from a conversation's next turn.",
                    ),
                    cx,
                ))
                .child(div().text_sm().child(decision::status(saved))),
        )
        .child(explained_setting(
            tr("Use a decision model"),
            "",
            tr("Off, map_items still runs its worker step on the sub-agent model."),
            switch("decision-enabled", enabled, tr("Use a decision model")).on_change(cx.listener(
                |this, _, _, cx| {
                    this.decision_draft.enabled = !this.decision_draft.enabled;
                    this.decision_note = None;
                    this.settings_error = None;
                    cx.notify();
                },
            )),
            cx,
        ))
        .child(setting(
            tr("Service"),
            decision::preset_hint(&draft.preset),
            services,
            cx,
        ))
        .child(explained_setting(
            tr("Base URL"),
            "",
            tr("Where the service answers. Empty takes the service's own address."),
            field(0, 300.),
            cx,
        ))
        .child(setting(
            tr("Model"),
            tr("The model to ask for. Empty lets the server pick."),
            field(1, 300.),
            cx,
        ))
        .child(setting(tr("API key"), key_note, key_control, cx))
        .child(explained_setting(
            tr("Thinking"),
            "",
            tr("For servers that can think before answering when they are unsure."),
            thinking,
            cx,
        ))
        .child(explained_setting(
            tr("Timeout"),
            "",
            tr("Seconds to wait for an answer, from 1 to 600. Empty takes the service's own."),
            field(2, 90.),
            cx,
        ))
        .child(actions)
    }
}
