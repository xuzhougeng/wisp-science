//! Model settings, shown in place of the transcript. The host validates and
//! stores the form; the API key goes to the system keychain and is never read
//! back into the window.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use wisp_server::desktop::{ModelFields, ModelInput, ModelSettings, ModelSource};

use super::{Workspace, shell::nav_row};
use crate::{backend::receive, layers::Layer, settings};

pub(super) struct SettingsForm {
    provider: String,
    url: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    context: Entity<InputState>,
    output: Entity<InputState>,
    effort: Entity<InputState>,
    /// Last state reported by the host; `None` until it has answered.
    current: Option<ModelSettings>,
    saving: bool,
    saved: bool,
    error: Option<String>,
}

impl SettingsForm {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut field = |placeholder: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let url = field("https://api.openai.com/v1");
        let model = field("Model ID, for example gpt-4o");
        let context = field("From the model catalog");
        let output = field("From the model catalog");
        let effort = field("Provider default");
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(settings::key_placeholder(false))
        });
        Self {
            provider: "openai".into(),
            url,
            model,
            key,
            context,
            output,
            effort,
            current: None,
            saving: false,
            saved: false,
            error: None,
        }
    }

    /// Whether new turns have a model to run on.
    pub(super) fn configured(&self) -> bool {
        self.current
            .as_ref()
            .is_none_or(|current| current.source != ModelSource::None)
    }

    fn editable(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| settings::editable(current.source))
    }
}

impl Workspace {
    /// Reads the host's model settings. On first launch nothing is configured,
    /// so the form opens by itself instead of leaving a dead composer.
    pub(super) fn load_model_settings(
        &mut self,
        open_if_unconfigured: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.model_settings();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(current) => {
                        let unconfigured = current.source == ModelSource::None;
                        this.apply_model_settings(current, window, cx);
                        if open_if_unconfigured && unconfigured {
                            this.open_settings(window, cx);
                        }
                    }
                    Err(error) => this.settings.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_model_settings(
        &mut self,
        current: ModelSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = &mut self.settings;
        form.provider = current.fields.provider.clone();
        for (input, value) in [
            (&form.url, &current.fields.api_url),
            (&form.model, &current.fields.model),
            (&form.context, &current.fields.max_context),
            (&form.output, &current.fields.max_tokens),
            (&form.effort, &current.fields.reasoning_effort),
        ] {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        let placeholder = settings::key_placeholder(current.has_api_key);
        form.key.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        form.current = Some(current);
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The conversation bar is not visible behind settings.
        self.layers.close(Layer::Rename);
        self.layers.close(Layer::ConfirmDelete);
        self.layers.open(Layer::Settings);
        self.settings.saved = false;
        self.settings.error = None;
        let model = self.settings.model.clone();
        model.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Settings) {
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn set_provider(
        &mut self,
        provider: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = &mut self.settings;
        let url = settings::next_api_url(&form.url.read(cx).value(), &form.provider, provider);
        form.url
            .update(cx, |input, cx| input.set_value(url, window, cx));
        form.provider = provider.into();
        form.saved = false;
        cx.notify();
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = &mut self.settings;
        if form.saving || !form.editable() {
            return;
        }
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let key = text(&form.key);
        let input = ModelInput {
            fields: ModelFields {
                provider: form.provider.clone(),
                api_url: text(&form.url),
                model: text(&form.model),
                max_context: text(&form.context),
                max_tokens: text(&form.output),
                reasoning_effort: text(&form.effort),
            },
            // The host keeps the stored key when none is typed.
            api_key: (!key.trim().is_empty()).then_some(key),
        };
        form.saving = true;
        form.saved = false;
        form.error = None;
        let reply = self.backend.save_model_settings(input);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.settings.saving = false;
                match result {
                    Ok(current) => {
                        this.apply_model_settings(current, window, cx);
                        this.settings.saved = true;
                        // The header shows the model new turns will use.
                        this.start_poll(window, cx);
                    }
                    // The typed values, including the key, stay for correction.
                    Err(error) => this.settings.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Div {
        let form = &self.settings;
        let editable = form.editable();
        let muted = cx.theme().muted_foreground;
        let field = |label: &'static str, hint: &'static str, input: &Entity<InputState>| {
            div()
                .v_flex()
                .gap_1()
                .child(div().text_sm().child(label))
                .child(Input::new(input).disabled(!editable))
                .child(div().text_xs().text_color(muted).child(hint))
        };
        let mut providers = div().h_flex().gap_2();
        for (id, label) in settings::PROVIDERS {
            providers = providers.child(
                Button::new(SharedString::from(format!("provider-{id}")))
                    .label(label)
                    .small()
                    .selected(form.provider == id)
                    .disabled(!editable)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.set_provider(id, window, cx)),
                    ),
            );
        }
        let status = match &form.current {
            Some(current) => settings::summary(current),
            None => "Loading settings…".into(),
        };
        let mut body = div()
            .v_flex()
            .gap_4()
            .p_6()
            .max_w(px(640.))
            .child(div().text_sm().text_color(muted).child(status))
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Provider"))
                    .child(providers),
            )
            .child(field(
                "API URL",
                "Leave blank for the provider's own endpoint.",
                &form.url,
            ))
            .child(field(
                "Model",
                "The exact model ID the endpoint expects.",
                &form.model,
            ))
            .child(field(
                "API key",
                "Stored in the system keychain, never in the project database.",
                &form.key,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        field(
                            "Context window (tokens)",
                            "Required for models outside the catalog.",
                            &form.context,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            "Output limit (tokens)",
                            "Required for models outside the catalog.",
                            &form.output,
                        )
                        .flex_1(),
                    ),
            )
            .child(field(
                "Reasoning effort",
                "Optional, for example low, medium or high.",
                &form.effort,
            ));
        if let Some(error) = &form.error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        } else if form.saved {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child("Saved. New messages use this model; running turns keep theirs."),
            );
        }
        body = body.child(
            div().h_flex().child(
                Button::new("settings-save")
                    .label(if form.saving { "Saving…" } else { "Save" })
                    .primary()
                    .disabled(!editable || form.saving)
                    .on_click(cx.listener(|this, _, window, cx| this.save_settings(window, cx))),
            ),
        );
        // Sections are listed the way the WebView groups them; each one is
        // added here as it is ported.
        let nav = div()
            .v_flex()
            .w(px(220.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                nav_row("settings-back", "arrow-left", "Back", false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Settings"),
            )
            .child(div().px_2().pt_2().text_xs().text_color(muted).child("AI"))
            .child(nav_row("settings-model", "bot", "Model", true, false, cx).opacity(1.));
        div().h_flex().size_full().child(nav).child(
            div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    div()
                        .px_6()
                        .h(px(52.))
                        .h_flex()
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Model"),
                )
                .child(
                    div()
                        .id("settings-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(body),
                ),
        )
    }
}
