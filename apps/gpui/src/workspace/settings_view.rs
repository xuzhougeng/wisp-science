//! Settings, shown in place of the rest of the window. The Models section
//! lists the saved model profiles and edits one at a time. The host validates
//! and stores every form; API keys go to the system keychain and are never
//! read back into the window.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use wisp_server::{
    config::default_api_url,
    desktop::{ModelFields, ModelInput, ModelProfile, ModelSettings, ModelSource},
};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, nav_row},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    present::Tone,
    settings::{self, Section},
};

pub(super) struct SettingsForm {
    provider: String,
    name: Entity<InputState>,
    url: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    context: Entity<InputState>,
    output: Entity<InputState>,
    effort: Entity<InputState>,
    /// Last state reported by the host; `None` until it has answered.
    current: Option<ModelSettings>,
    /// Profile the form is editing; `None` is a model not saved yet.
    editing: Option<String>,
    busy: bool,
    /// What the last action achieved, shown until the next one.
    notice: Option<String>,
    error: Option<String>,
}

impl SettingsForm {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut field = |placeholder: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = field(tr("Shown in lists; defaults to the model ID"));
        let url = field("https://api.openai.com/v1");
        let model = field(tr("Model ID, for example gpt-4o"));
        let context = field(tr("From the model catalog"));
        let output = field(tr("From the model catalog"));
        let effort = field(tr("Provider default"));
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(settings::key_placeholder(false))
        });
        Self {
            provider: "openai".into(),
            name,
            url,
            model,
            key,
            context,
            output,
            effort,
            current: None,
            editing: None,
            busy: false,
            notice: None,
            error: None,
        }
    }

    /// Whether new turns have a model to run on.
    pub(super) fn configured(&self) -> bool {
        self.current
            .as_ref()
            .is_none_or(|current| current.source != ModelSource::None)
    }

    pub(super) fn current(&self) -> Option<&ModelSettings> {
        self.current.as_ref()
    }

    fn editable(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| settings::editable(current.source))
    }

    fn profile(&self) -> Option<&ModelProfile> {
        let id = self.editing.as_ref()?;
        self.current
            .as_ref()?
            .profiles
            .iter()
            .find(|profile| &profile.id == id)
    }
}

impl Workspace {
    /// Reads the host's model settings. On first launch nothing is configured,
    /// so Settings opens by itself instead of leaving a dead composer.
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
                        this.apply_model_settings(current, None, window, cx);
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

    /// Takes the host's list and points the form at `select`, or at the
    /// profile it was already editing if that still exists.
    fn apply_model_settings(
        &mut self,
        current: ModelSettings,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wanted = select.or_else(|| self.settings.editing.clone());
        let editing = settings::selection_after(&current, wanted.as_deref());
        self.settings.current = Some(current);
        self.edit_profile(editing, window, cx);
    }

    /// Takes a list the host returned after a change made elsewhere on the
    /// page (a subscription signed in or out).
    pub(super) fn apply_model_settings_from(
        &mut self,
        current: ModelSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_model_settings(current, None, window, cx);
    }

    /// Fills the form from a saved profile, or blanks it for a new model.
    fn edit_profile(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmDeleteModel);
        self.settings.editing = id;
        let (name, fields, has_key) = match self.settings.profile() {
            Some(profile) => (
                profile.name.clone(),
                profile.fields.clone(),
                profile.has_api_key,
            ),
            None => (
                String::new(),
                ModelFields {
                    provider: "openai".into(),
                    api_url: default_api_url("openai").into(),
                    ..Default::default()
                },
                false,
            ),
        };
        let form = &mut self.settings;
        form.provider = if fields.provider.is_empty() {
            "openai".into()
        } else {
            fields.provider.clone()
        };
        for (input, value) in [
            (&form.name, &name),
            (&form.url, &fields.api_url),
            (&form.model, &fields.model),
            (&form.context, &fields.max_context),
            (&form.output, &fields.max_tokens),
            (&form.effort, &fields.reasoning_effort),
        ] {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        let placeholder = settings::key_placeholder(has_key);
        form.key.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    fn clear_messages(&mut self) {
        self.settings.notice = None;
        self.settings.error = None;
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // None of these is visible behind Settings.
        for layer in [
            Layer::Rename,
            Layer::ConfirmDelete,
            Layer::ConfirmFullAccess,
            Layer::EffortMenu,
            Layer::ModelMenu,
            Layer::RunsOnMenu,
        ] {
            self.layers.close(layer);
        }
        self.layers.open(Layer::Settings);
        // The Models page, where Settings opens, lists the subscriptions.
        self.load_subscriptions(window, cx);
        // Every visit starts with all the pages listed.
        self.settings_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.clear_messages();
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

    fn select_profile(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_messages();
        self.edit_profile(id, window, cx);
        let model = self.settings.model.clone();
        model.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Starts a new model from one of the well-known endpoints.
    fn use_preset(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((label, url, model)) = settings::PRESETS.get(index).copied() else {
            return;
        };
        self.select_profile(None, window, cx);
        let form = &self.settings;
        for (input, value) in [(&form.name, label), (&form.url, url), (&form.model, model)] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        // The key is the one thing a preset cannot know.
        let key = form.key.clone();
        key.update(cx, |input, cx| input.focus(window, cx));
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
        form.notice = None;
        cx.notify();
    }

    fn form_input(&self, cx: &App) -> ModelInput {
        let form = &self.settings;
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let key = text(&form.key);
        ModelInput {
            id: form.editing.clone(),
            name: text(&form.name),
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
        }
    }

    /// Runs one change to the saved models and shows the list it returns.
    /// `select` picks the profile for the form: `Some(None)` means the one
    /// the change added.
    fn change_models(
        &mut self,
        reply: Reply<ModelSettings>,
        notice: &'static str,
        select: Option<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.busy = true;
        self.clear_messages();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.settings.busy = false;
                match result {
                    Ok(current) => {
                        let select = match select {
                            Some(None) => settings::added(this.settings.current(), &current),
                            Some(id) => id,
                            None => None,
                        };
                        this.apply_model_settings(current, select, window, cx);
                        this.settings.notice = Some(notice.into());
                        // The header shows the model the conversation uses.
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

    fn can_change_models(&self) -> bool {
        !self.settings.busy && self.settings.editable()
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_change_models() {
            return;
        }
        let input = self.form_input(cx);
        let select = match &input.id {
            Some(id) => Some(Some(id.clone())),
            None => Some(None),
        };
        let reply = self.backend.save_model_profile(input);
        self.change_models(
            reply,
            tr("Saved. New messages use it; running turns keep their model."),
            select,
            window,
            cx,
        );
    }

    fn make_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.settings.editing.clone() else {
            return;
        };
        if !self.can_change_models() {
            return;
        }
        let reply = self.backend.set_default_model(id);
        self.change_models(
            reply,
            tr("New conversations now use this model."),
            None,
            window,
            cx,
        );
    }

    fn ask_delete_model(&mut self, cx: &mut Context<Self>) {
        if self.settings.editing.is_some() {
            self.clear_messages();
            self.layers.open(Layer::ConfirmDeleteModel);
            cx.notify();
        }
    }

    fn cancel_delete_model(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ConfirmDeleteModel) {
            cx.notify();
        }
    }

    fn delete_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.settings.editing.clone() else {
            return;
        };
        if !self.can_change_models() || !self.layers.close(Layer::ConfirmDeleteModel) {
            return;
        }
        let reply = self.backend.delete_model_profile(id);
        self.change_models(reply, tr("Removed."), None, window, cx);
    }

    /// Asks the model for one word with the form as it is typed.
    fn test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.busy {
            return;
        }
        self.settings.busy = true;
        self.clear_messages();
        let reply = self.backend.test_model(self.form_input(cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.settings.busy = false;
                match result {
                    Ok(answer) => {
                        this.settings.notice = Some(crate::i18n::fill(
                            tr("It works. The model said: {}"),
                            &[&answer],
                        ))
                    }
                    Err(error) => this.settings.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_model_list(&self, cx: &mut Context<Self>) -> Div {
        let form = &self.settings;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().list_active;
        let hover = cx.theme().list_hover;
        let mut list = div().v_flex().w(px(280.)).flex_shrink_0().gap_1().child(
            div()
                .pb_2()
                .text_sm()
                .text_color(muted)
                .child(match &form.current {
                    Some(current) => settings::summary(current),
                    None => tr("Loading settings…").into(),
                }),
        );
        for profile in form.current.iter().flat_map(|current| &current.profiles) {
            let id = profile.id.clone();
            let mut heading = div().h_flex().gap_2().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(profile.name.clone()),
            );
            if profile.is_default {
                heading = heading.child(chip(tr("Default"), Tone::Success, cx));
            }
            let detail = match &profile.problem {
                Some(problem) => div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(crate::i18n::host(problem)),
                None => div()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(profile.fields.model.clone()),
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("model-{id}")))
                    .v_flex()
                    .px_3()
                    .py_2()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|style| style.bg(hover))
                    .when(form.editing.as_ref() == Some(&id), |row| row.bg(accent))
                    .child(heading)
                    .child(detail)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_profile(Some(id.clone()), window, cx)
                    })),
            );
        }
        if !form.editable() {
            return list;
        }
        let mut presets = div().h_flex().flex_wrap().gap_1();
        for (index, (label, _, _)) in settings::PRESETS.iter().enumerate() {
            presets = presets.child(
                Button::new(("preset", index))
                    .label(*label)
                    .xsmall()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.use_preset(index, window, cx)),
                    ),
            );
        }
        list.child(
            div().pt_2().child(
                Button::new("model-add")
                    .icon(glyph("plus"))
                    .label(tr("Add a model"))
                    .small()
                    .selected(form.editing.is_none())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.select_profile(None, window, cx)),
                    ),
            ),
        )
        .child(
            div()
                .pt_3()
                .text_xs()
                .text_color(muted)
                .child(tr("Or start from a provider:")),
        )
        .child(presets)
    }

    fn render_model_form(&self, cx: &mut Context<Self>) -> Div {
        let form = &self.settings;
        let editable = form.editable();
        let muted = cx.theme().muted_foreground;
        let saved = form.profile();
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
        let mut body = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .max_w(px(620.))
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(match saved {
                        Some(profile) => profile.name.clone(),
                        None => tr("New model").into(),
                    }),
            )
            .child(field(
                tr("Name"),
                tr("What this model is called in lists and in the composer."),
                &form.name,
            ))
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(div().text_sm().child(tr("Provider")))
                    .child(providers),
            )
            .child(field(
                tr("API URL"),
                tr("Leave blank for the provider's own endpoint."),
                &form.url,
            ))
            .child(field(
                tr("Model"),
                tr("The exact model ID the endpoint expects."),
                &form.model,
            ))
            .child(field(
                tr("API key"),
                tr("Stored in the system keychain, never in the project database."),
                &form.key,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        field(
                            tr("Context window (tokens)"),
                            tr("Required for models outside the catalog."),
                            &form.context,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            tr("Output limit (tokens)"),
                            tr("Required for models outside the catalog."),
                            &form.output,
                        )
                        .flex_1(),
                    ),
            )
            .child(field(
                tr("Reasoning effort"),
                tr("The default for conversations that do not choose one."),
                &form.effort,
            ));
        if let Some(error) = &form.error {
            body = body.child(error_banner(error, cx));
        } else if let Some(notice) = &form.notice {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(notice.clone()),
            );
        }
        if self.layers.contains(Layer::ConfirmDeleteModel) {
            return body.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(tr(
                                "Remove this model and its key? Conversations that use it go \
                                 back to the default.",
                            )),
                    )
                    .child(
                        Button::new("model-delete-confirm")
                            .label(tr("Remove"))
                            .small()
                            .danger()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.delete_model(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("model-delete-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_delete_model(cx))),
                    ),
            );
        }
        let locked = !editable || form.busy;
        let mut actions = div()
            .h_flex()
            .gap_2()
            .child(
                Button::new("settings-save")
                    .label(if saved.is_some() {
                        tr("Save")
                    } else {
                        tr("Add")
                    })
                    .primary()
                    .disabled(locked)
                    .on_click(cx.listener(|this, _, window, cx| this.save_settings(window, cx))),
            )
            .child(
                Button::new("settings-test")
                    .icon(glyph("bolt"))
                    .label(if form.busy {
                        tr("Working…")
                    } else {
                        tr("Test connection")
                    })
                    .disabled(form.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.test_connection(window, cx))),
            );
        if let Some(profile) = saved {
            actions = actions
                .child(
                    Button::new("settings-default")
                        .label(tr("Use for new conversations"))
                        .disabled(locked || profile.is_default)
                        .on_click(cx.listener(|this, _, window, cx| this.make_default(window, cx))),
                )
                .child(div().flex_1())
                .child(
                    Button::new("settings-delete")
                        .icon(glyph("trash"))
                        .label(tr("Remove"))
                        .ghost()
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| this.ask_delete_model(cx))),
                );
        }
        body.child(actions)
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
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
                nav_row("settings-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Settings")),
            );
        let query = self.settings_search.read(cx).value().to_string();
        let found = settings::search_sections(&query);
        let mut nav = nav.child(
            div().px_1().pb_1().child(
                Input::new(&self.settings_search)
                    .small()
                    .prefix(crate::assets::icon("search").text_color(muted)),
            ),
        );
        if found.is_empty() {
            nav = nav.child(
                div()
                    .px_2()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("No page is about that.")),
            );
        }
        let mut group = "";
        for section in found {
            if section.group() != group {
                group = section.group();
                nav = nav.child(div().px_2().pt_2().text_xs().text_color(muted).child(group));
            }
            let selected = self.settings_section == section;
            nav = nav.child(
                nav_row(
                    SharedString::from(format!("settings-{}", section.label())),
                    section.icon(),
                    section.label(),
                    selected,
                    true,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.show_settings_section(section, window, cx)
                })),
            );
        }
        // Help is not a page: both rows open in the system's browser.
        nav = nav
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Help")),
            )
            .child(
                nav_row(
                    "settings-docs",
                    "book",
                    tr("Documentation"),
                    false,
                    true,
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.open_url(crate::links::DOCS))),
            )
            .child(
                nav_row(
                    "settings-feedback",
                    "bubble",
                    tr("Report a problem"),
                    false,
                    true,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.report_problem(window, cx))),
            )
            .child(
                nav_row(
                    "settings-issues",
                    "external-link",
                    tr("Known problems"),
                    false,
                    true,
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.open_url(crate::links::ISSUES))),
            );
        let body = match self.settings_section {
            Section::Models => div()
                .h_flex()
                .items_start()
                .gap_8()
                .p_6()
                .child(
                    div()
                        .v_flex()
                        .w(px(280.))
                        .flex_shrink_0()
                        .child(self.render_model_list(cx))
                        .child(self.render_subscriptions(cx)),
                )
                .child(self.render_model_form(cx)),
            Section::General => self.render_general_settings(cx),
            Section::Conversation => self.render_conversation_settings(cx),
            Section::Skills => self.render_skill_settings(cx),
            Section::Connections => self.render_connection_settings(cx),
            Section::Credentials => self.render_credential_settings(cx),
            Section::Environments => self.render_environment_settings(cx),
            Section::Experts => self.render_expert_settings(cx),
            Section::AcpAgents => self.render_acp_settings(cx),
            Section::QuickActions => self.render_quick_settings(cx),
            Section::Pet => self.render_pet_settings(cx),
            Section::Browser => self.render_browser_settings(cx),
            Section::Workflows => self.render_workflow_settings(cx),
            Section::Remote => self.render_remote_settings(cx),
            Section::Channels => self.render_channel_settings(cx),
            Section::Plugins => self.render_plugin_settings(cx),
            Section::DecisionModel => self.render_decision_settings(cx),
            Section::Network => self.render_network_settings(cx),
            Section::Usage => self.render_usage_settings(cx),
            Section::Storage => self.render_storage_settings(cx),
            Section::Permissions => self.render_permission_settings(cx),
            Section::Hooks => self.render_hook_settings(cx),
        };
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
                        .child(self.settings_section.label()),
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
