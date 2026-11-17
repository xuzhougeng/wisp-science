//! Settings, shown in place of the rest of the window. The Models section
//! lists the saved model profiles and edits one at a time. The host validates
//! and stores every form; API keys go to the system keychain and are never
//! read back into the window.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::InputState,
};
use wisp_server::{
    config::default_api_url,
    desktop::{ModelFields, ModelInput, ModelProfile, ModelSettings, ModelSource},
};

use super::{
    Workspace,
    settings_sections::setting,
    shell::{error_banner, nav_row},
    surfaces,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
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
    /// The model's own route; empty follows the Network page.
    route: Entity<InputState>,
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
        let route = field(tr("Follows the Network page"));
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
            route,
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
            (&form.route, &fields.proxy),
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
                proxy: text(&form.route),
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

    /// The saved models as record rows, the chosen one on white with a
    /// hairline; the way to add one and the providers to start from under
    /// them. When the model comes from `WISP_*` variables, one bar says so
    /// at the top.
    fn render_model_list(&self, cx: &mut Context<Self>) -> Div {
        let form = &self.settings;
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let mut list = div().v_flex().w_full();
        match &form.current {
            Some(current) if !form.editable() => {
                list = list.child(
                    surfaces::bar("models-environment", cx)
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .mb_3()
                        .child(
                            crate::assets::icon("circle-help")
                                .size(rems(1.))
                                .flex_none()
                                .text_color(cx.theme().info),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .truncate()
                                .child(settings::summary(current)),
                        ),
                );
            }
            Some(current) => {
                list = list.child(
                    div()
                        .pb_2()
                        .text_sm()
                        .text_color(muted)
                        .child(settings::summary(current)),
                );
            }
            None => {
                list = list.child(
                    div()
                        .pb_2()
                        .text_sm()
                        .text_color(muted)
                        .child(tr("Loading settings…")),
                );
            }
        }
        let mut rows = Vec::new();
        for profile in form.current.iter().flat_map(|current| &current.profiles) {
            let id = profile.id.clone();
            let chosen = form.editing.as_ref() == Some(&id);
            let mut row = surfaces::record_row(
                SharedString::from(format!("model-{id}")),
                profile.name.clone(),
            )
            .icon("bot")
            .suffix(match &profile.problem {
                Some(problem) => crate::i18n::host(problem),
                None => profile.fields.model.clone(),
            });
            if profile.is_default {
                row = row.stamp(surfaces::StampKind::Success, tr("Default"));
            }
            if profile.problem.is_some() {
                row = row.stamp(surfaces::StampKind::Failed, tr("Failed"));
            }
            rows.push(
                row.cursor_pointer()
                    .when(chosen, |row| row.bg(cx.theme().popover))
                    .when(!chosen, |row| row.hover(move |style| style.bg(hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_profile(Some(id.clone()), window, cx)
                    })),
            );
        }
        if !rows.is_empty() {
            list = list.child(surfaces::record_group(rows, cx));
        }
        if !form.editable() {
            return list;
        }
        let mut presets = div().h_flex().flex_wrap().gap_1();
        for (index, (label, _, _)) in settings::PRESETS.iter().enumerate() {
            presets = presets.child(
                surfaces::outlined_action(("preset", index), "bot", *label, cx).on_click(
                    cx.listener(move |this, _, window, cx| this.use_preset(index, window, cx)),
                ),
            );
        }
        list.child(
            div()
                .h_flex()
                .items_center()
                .gap_3()
                .pt_3()
                .child(
                    surfaces::outlined_action("model-add", "plus", tr("Add a model"), cx).on_click(
                        cx.listener(|this, _, window, cx| this.select_profile(None, window, cx)),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Or start from a provider:")),
                ),
        )
        .child(div().pt_2().child(presets))
    }

    /// The chosen model's form, under the list in the same column: one
    /// setting row for each field. A model from `WISP_*` variables shows
    /// its values as facts on the paper, not as grey fields.
    fn render_model_form(&self, cx: &mut Context<Self>) -> Div {
        let form = &self.settings;
        let editable = form.editable();
        let saved = form.profile();
        // While the model can be changed, a field and the one line that
        // says how to fill it; read only (`WISP_*` variables), the name and
        // the value alone, and for an empty value what it means, faded.
        let field = |label: &'static str,
                     hint: &'static str,
                     empty: &'static str,
                     input: &Entity<InputState>| {
            if editable {
                let control = div().w(px(300.)).child(super::surfaces::input(input, cx));
                return setting(label, hint, control, cx);
            }
            let value = input.read(cx).value().trim().to_string();
            let control = if value.is_empty() {
                surfaces::fact(empty, cx).into_any_element()
            } else {
                surfaces::read_only(value, cx).into_any_element()
            };
            setting(label, "", control, cx)
        };
        let provider_name = settings::PROVIDERS
            .iter()
            .find(|(id, _)| *id == form.provider)
            .map_or(form.provider.clone(), |(_, label)| (*label).to_owned());
        let providers = if !editable {
            surfaces::read_only(provider_name, cx)
        } else {
            surfaces::segmented(
                settings::PROVIDERS.map(|(id, label)| {
                    surfaces::plain_segment(
                        SharedString::from(format!("provider-{id}")),
                        label,
                        form.provider == id,
                        cx,
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.set_provider(id, window, cx)),
                    )
                    .into_any_element()
                }),
                cx,
            )
        };
        let mut body = div()
            .v_flex()
            .w_full()
            .child(
                surfaces::settings_heading(
                    match saved {
                        Some(profile) => profile.name.clone(),
                        None => tr("New model").into(),
                    },
                    cx,
                )
                .pt_0(),
            )
            .child(field(
                tr("Name"),
                tr("What this model is called in lists and in the composer."),
                "—",
                &form.name,
            ))
            .child(setting(tr("Provider"), "", providers, cx))
            .child(field(
                tr("API URL"),
                tr("Leave blank for the provider's own endpoint."),
                tr("The provider's own endpoint"),
                &form.url,
            ))
            .child(field(
                tr("Model"),
                tr("The exact model ID the endpoint expects."),
                "—",
                &form.model,
            ))
            .child(if editable {
                field(
                    tr("API key"),
                    tr("Kept in the system keychain, not in the project."),
                    "",
                    &form.key,
                )
            } else {
                // The key is never read back: say whether there is one.
                let kept = saved.is_some_and(|profile| profile.has_api_key);
                setting(
                    tr("API key"),
                    "",
                    surfaces::fact(if kept { tr("Saved") } else { tr("Not set") }, cx),
                    cx,
                )
            })
            .child(field(
                tr("Context window (tokens)"),
                tr("Required for models outside the catalog."),
                tr("From the model catalog"),
                &form.context,
            ))
            .child(field(
                tr("Output limit (tokens)"),
                tr("Required for models outside the catalog."),
                tr("From the model catalog"),
                &form.output,
            ))
            .child(field(
                tr("Reasoning effort"),
                tr("The default for conversations that do not choose one."),
                tr("Provider default"),
                &form.effort,
            ))
            .child(field(
                tr("Route"),
                tr("A proxy for this model alone; empty follows Network."),
                tr("Follows the Network page"),
                &form.route,
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
        // A model from `WISP_*` variables is changed where they are set:
        // nothing here saves, tests, chooses or removes it.
        if !editable {
            return body;
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
        // Save is the page's one filled button; testing is the one most
        // used besides, outlined; the rest are icon buttons, Remove last in
        // the danger colour.
        let mut actions = div()
            .h_flex()
            .items_center()
            .gap_2()
            .pt_3()
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
                surfaces::outlined_action(
                    "settings-test",
                    "bolt",
                    if form.busy {
                        tr("Working…")
                    } else {
                        tr("Test connection")
                    },
                    cx,
                )
                .disabled(form.busy)
                .on_click(cx.listener(|this, _, window, cx| this.test_connection(window, cx))),
            );
        if let Some(profile) = saved {
            actions = actions
                .child(
                    surfaces::icon_action(
                        "settings-default",
                        "star",
                        tr("Use for new conversations"),
                        if profile.is_default {
                            surfaces::IconTone::On
                        } else {
                            surfaces::IconTone::Faded
                        },
                        cx,
                    )
                    .disabled(locked || profile.is_default)
                    .on_click(cx.listener(|this, _, window, cx| this.make_default(window, cx))),
                )
                .child(div().flex_1())
                .child(
                    surfaces::danger_action("settings-delete", "trash", tr("Remove"), cx)
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
            .child(surfaces::title(tr("Settings")).px_2().pt_3().pb_1());
        let query = self.settings_search.read(cx).value().to_string();
        let found = settings::search_sections(&query);
        let mut nav = nav.child(
            div().px_1().pb_1().child(
                surfaces::input(&self.settings_search, cx)
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
            // The list on top, the chosen model's form under it, in the
            // one column.
            Section::Models => div()
                .v_flex()
                .gap_6()
                .child(self.render_model_list(cx))
                .child(self.render_model_form(cx))
                .child(self.render_subscriptions(cx)),
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
        // Every page in one centred column on the paper, its name a Title
        // at the top.
        div().h_flex().size_full().child(nav).child(
            div()
                .id("settings-body")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(surfaces::settings_column(
                    self.settings_section.label(),
                    body,
                )),
        )
    }
}
