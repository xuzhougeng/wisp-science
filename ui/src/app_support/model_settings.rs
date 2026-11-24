//! Model & specialist settings domain: the form signals behind the Settings
//! "Models" and "Specialists" panes plus their save/validate/test handlers.
//! `App` constructs `ModelSettingsState` with the cross-domain signals it
//! depends on (profile list, settings, busy flag, locale) and passes the
//! handlers down to `SettingsView` as callbacks.

use super::*;
use crate::bindings::invoke_timeout;
use crate::network_settings::route_error;

/// Wait this long for `save_model`. A reply lost on the way back from the
/// backend (macOS 12 stall) must not leave the Models panel busy forever.
const SAVE_MODEL_TIMEOUT_MS: u32 = 60_000;

/// Invoke arguments as plain objects and `null`, never an ES `Map` and
/// `undefined`. `serde_wasm_bindgen::to_value` builds a `Map`; on a macOS 12
/// WKWebView the Models panel's save and validate flows stopped inside that
/// path before the IPC request left the renderer, and resumed with this shape.
/// Every command deserializes JSON, so plain objects are the exact contract.
/// A serialization error is returned, not unwrapped: a wasm panic is a trap
/// that leaves `settings_busy` set for good.
fn json_args(value: &serde_json::Value) -> Result<JsValue, serde_wasm_bindgen::Error> {
    serde::Serialize::serialize(value, &serde_wasm_bindgen::Serializer::json_compatible())
}

/// The backend committed the profile, but the reply did not deserialize. The
/// form closes (a retry would add a duplicate), so the notice is also toasted.
fn report_unreadable_reply(loc: Locale, err: &serde_wasm_bindgen::Error) -> String {
    let text = tf(
        loc,
        "status.save_response_unreadable",
        &[("msg", &err.to_string())],
    );
    show_actionable_warning_toast(&text);
    text
}

/// Resolve selectable efforts from the exact catalog entry. A keyed result
/// prevents a slow lookup for the previous model or endpoint leaking into the
/// current picker. Unknown entries never imply support for every effort.
pub(crate) fn use_model_efforts(
    source: impl Fn() -> Option<(String, String, String)> + 'static,
) -> Memo<Option<Vec<String>>> {
    let key = create_memo(move |_| source());
    let result = create_rw_signal(None::<((String, String, String), Option<Vec<String>>)>);
    create_effect(move |_| {
        let Some(request_key) = key.get() else {
            result.set(None);
            return;
        };
        result.set(None);
        spawn_local(async move {
            let args = json_args(&serde_json::json!({
                "provider": request_key.0,
                "apiUrl": request_key.1,
                "model": request_key.2,
            }));
            let reply = match args {
                Ok(args) => invoke_checked("model_catalog_lookup", args).await.ok(),
                Err(_) => None,
            };
            let efforts = reply
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<Option<CatalogEntryDto>>(value).ok()
                })
                .flatten()
                .map(|entry| entry.efforts);
            if key.try_get_untracked().flatten().as_ref() == Some(&request_key) {
                result.set(Some((request_key, efforts)));
            }
        });
    });
    create_memo(move |_| {
        let (result_key, efforts) = result.get()?;
        (key.get().as_ref() == Some(&result_key))
            .then_some(efforts)
            .flatten()
    })
}

pub(crate) const DEFAULT_FORM_MAX_TOKENS: u64 = 8192;
pub(crate) const DEFAULT_FORM_CONTEXT_WINDOW: u64 = 128_000;

/// Output ceiling, context window and endpoint-reported efforts for a new
/// profile: the exact catalog entry first, then the facts the endpoint's own
/// model list gave for this exact id, then the usual defaults.
async fn new_profile_limits(
    provider: &str,
    api_url: &str,
    model: &str,
    facts: Option<&EndpointModelDto>,
) -> (u64, u64, Vec<String>) {
    let catalog = match json_args(&serde_json::json!({
        "provider": provider,
        "apiUrl": api_url,
        "model": model,
    })) {
        Ok(args) => invoke_checked("model_catalog_lookup", args)
            .await
            .ok()
            .and_then(|value| serde_wasm_bindgen::from_value::<Option<CatalogEntryDto>>(value).ok())
            .flatten(),
        Err(_) => None,
    };
    match (catalog, facts) {
        (Some(dto), _) => (dto.max_tokens, dto.context_window, Vec::new()),
        (None, Some(facts)) => (
            facts.max_output_tokens.unwrap_or(DEFAULT_FORM_MAX_TOKENS),
            facts.context_window.unwrap_or(DEFAULT_FORM_CONTEXT_WINDOW),
            facts.reasoning_levels.clone().unwrap_or_default(),
        ),
        (None, None) => (
            DEFAULT_FORM_MAX_TOKENS,
            DEFAULT_FORM_CONTEXT_WINDOW,
            Vec::new(),
        ),
    }
}

/// The endpoint's facts for exactly this model id (never a prefix match).
pub(crate) fn endpoint_facts<'a>(
    models: &'a [EndpointModelDto],
    model: &str,
) -> Option<&'a EndpointModelDto> {
    let model = model.trim();
    models.iter().find(|entry| entry.id == model)
}

/// Copy the fetched facts for the form's model id into the form. Token limits
/// the user typed in this form stay; a later exact catalog match still wins
/// (`apply_catalog_limits` runs after this).
pub(crate) fn sync_endpoint_facts(form: &mut ModelForm) {
    let facts = endpoint_facts(&form.endpoint_models, &form.model).cloned();
    form.endpoint_efforts = facts
        .as_ref()
        .and_then(|facts| facts.reasoning_levels.clone())
        .unwrap_or_default();
    let Some(facts) = facts else {
        return;
    };
    if let Some(output) = facts.max_output_tokens.filter(|_| !form.max_tokens_edited) {
        form.max_tokens = output;
    }
    if let Some(context) = facts.context_window.filter(|_| !form.context_window_edited) {
        form.context_window = context;
    }
}

/// Field ids a Save / Test connection rejects, in form order: `api_url`,
/// `api_key`, `model`, `endpoint_suffix` (per add-form row,
/// `endpoint_suffix:<row id>`). An API key is optional on a local gateway.
pub(crate) fn model_form_problems(
    form: &ModelForm,
    key: &str,
    has_stored_key: bool,
) -> Vec<String> {
    let subscription = matches!(
        provider_value(&form.provider),
        "openai_codex" | "openai_chatgpt" | "xai_oauth"
    );
    let mut problems = Vec::new();
    if !subscription && form.api_url.trim().is_empty() {
        problems.push("api_url".to_string());
    }
    if !subscription
        && key.trim().is_empty()
        && !has_stored_key
        && !crate::text::is_loopback_api_url(&form.api_url)
    {
        problems.push("api_key".to_string());
    }
    let model_missing = if form.id.is_none() {
        form.entries
            .iter()
            .all(|entry| entry.model.trim().is_empty())
    } else {
        form.model.trim().is_empty()
    };
    if model_missing {
        problems.push("model".to_string());
    }
    if form.id.is_none() {
        for entry in &form.entries {
            if crate::text::endpoint_suffix_error(&entry.endpoint_suffix).is_some() {
                problems.push(format!("endpoint_suffix:{}", entry.row_id));
            }
        }
    } else if crate::text::endpoint_suffix_error(&form.endpoint_suffix).is_some() {
        problems.push("endpoint_suffix".to_string());
    }
    problems
}

/// The DOM id of the input a problem names (see `model_form_problems`).
pub(crate) fn model_field_dom_id(field: &str) -> String {
    match field {
        "api_url" => "model-form-api-url".into(),
        "api_key" => "model-form-api-key".into(),
        "model" => "model-form-model".into(),
        other => match other.strip_prefix("endpoint_suffix:") {
            Some(row) => format!("provider-endpoint-suffix-{row}"),
            None => "model-form-endpoint-suffix".into(),
        },
    }
}

/// One status line naming every problem field.
fn model_form_problem_text(loc: Locale, problems: &[String]) -> String {
    let mut names: Vec<String> = Vec::new();
    for problem in problems {
        let key = match problem.split(':').next().unwrap_or_default() {
            "api_url" => "settings.api_url",
            "api_key" => "models.field.api_key",
            "model" => "settings.model",
            _ => "models.field.endpoint_suffix",
        };
        let name = t(loc, key);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let separator = if loc == Locale::Zh { "、" } else { ", " };
    let mut text = tf(
        loc,
        "models.fix_fields",
        &[("fields", &names.join(separator))],
    );
    if problems.iter().any(|p| p.starts_with("endpoint_suffix")) {
        text.push(' ');
        text.push_str(&t(loc, "err.endpoint_suffix_invalid"));
    }
    text
}

/// Open any collapsed disclosure around the field, then focus it.
pub(crate) fn focus_model_field_soon(field: &str) {
    let id = model_field_dom_id(field);
    let focus = Closure::once(move || {
        let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(&id))
        else {
            return;
        };
        if let Ok(Some(details)) = element.closest("details") {
            let _ = details.set_attribute("open", "");
        }
        if let Some(input) = element.dyn_ref::<web_sys::HtmlElement>() {
            let _ = input.focus();
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            focus.as_ref().unchecked_ref(),
            0,
        );
    }
    focus.forget();
}

/// Mark every problem field, focus the first and report them in one line.
/// Returns false when the form may proceed.
fn report_model_form_problems(
    model_form: RwSignal<Option<ModelForm>>,
    model_form_msg: RwSignal<Option<(bool, String)>>,
    loc: Locale,
    problems: Vec<String>,
) -> bool {
    model_form.update(|form| {
        if let Some(form) = form {
            form.invalid_fields = problems.clone();
        }
    });
    let Some(first) = problems.first() else {
        return false;
    };
    focus_model_field_soon(first);
    model_form_msg.set(Some((false, model_form_problem_text(loc, &problems))));
    true
}

/// The Specialists page's import/export dialog. One layer at a time: either a
/// package preview awaiting explicit confirmation, or an export paused because
/// the instructions look like they contain a secret.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecialistTransfer {
    Import {
        preview: specialist_package::SpecialistImportPreview,
        /// Local profile to bind ("" follows the active model).
        model_id: String,
        reviewed: bool,
        error: Option<String>,
    },
    SecretWarning {
        id: String,
        name: String,
        findings: Vec<specialist_package::SecretFinding>,
    },
}

/// Owned form state plus injected cross-domain wiring. `RwSignal` is `Copy`,
/// so the whole struct is `Copy` and can be captured by every handler closure.
#[derive(Clone, Copy)]
pub(crate) struct ModelSettingsState {
    pub(crate) model_form: RwSignal<Option<ModelForm>>,
    pub(crate) model_catalog_limits: RwSignal<Option<CatalogEntryDto>>,
    pub(crate) model_form_key: RwSignal<String>,
    pub(crate) model_form_msg: RwSignal<Option<(bool, String)>>,
    pub(crate) specialists: RwSignal<Vec<Specialist>>,
    pub(crate) specialist_form: RwSignal<Option<Specialist>>,
    pub(crate) specialist_transfer: RwSignal<Option<SpecialistTransfer>>,
    /// Global default subagent model profile id; "" follows the conversation.
    pub(crate) subagent_model: RwSignal<String>,
    /// Set while Magpie overrides every HTTP model for this process.
    pub(crate) magpie_model: RwSignal<Option<MagpieModelDto>>,
    // Cross-domain signals injected by `App`.
    pub(crate) models: RwSignal<Vec<ModelProfile>>,
    pub(crate) acp_agents: RwSignal<Vec<AcpAgentProfile>>,
    pub(crate) settings: RwSignal<Settings>,
    pub(crate) settings_busy: RwSignal<bool>,
    pub(crate) settings_message: RwSignal<Option<(bool, String)>>,
    pub(crate) locale: RwSignal<Locale>,
}

impl ModelSettingsState {
    pub(crate) fn new(
        models: RwSignal<Vec<ModelProfile>>,
        acp_agents: RwSignal<Vec<AcpAgentProfile>>,
        settings: RwSignal<Settings>,
        settings_busy: RwSignal<bool>,
        settings_message: RwSignal<Option<(bool, String)>>,
        locale: RwSignal<Locale>,
    ) -> Self {
        Self {
            model_form: create_rw_signal(None::<ModelForm>),
            model_catalog_limits: create_rw_signal(None::<CatalogEntryDto>),
            model_form_key: create_rw_signal(String::new()),
            model_form_msg: create_rw_signal(None::<(bool, String)>),
            specialists: create_rw_signal(vec![]),
            specialist_form: create_rw_signal(None::<Specialist>),
            specialist_transfer: create_rw_signal(None::<SpecialistTransfer>),
            subagent_model: create_rw_signal(String::new()),
            magpie_model: create_rw_signal(None::<MagpieModelDto>),
            models,
            acp_agents,
            settings,
            settings_busy,
            settings_message,
            locale,
        }
    }

    pub(crate) fn refresh_models(self) {
        let Self {
            models,
            acp_agents,
            subagent_model,
            magpie_model,
            ..
        } = self;
        spawn_local(async move {
            if let Ok(value) = invoke_checked("magpie_model_info", JsValue::UNDEFINED).await {
                let model = serde_wasm_bindgen::from_value::<Option<MagpieModelDto>>(value)
                    .ok()
                    .flatten();
                if magpie_model.get_untracked() != model {
                    magpie_model.set(model);
                }
            }
            let v = invoke("list_models", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                models.set(list);
            }
            let v = invoke("list_acp_agents", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(v) {
                acp_agents.set(list);
            }
            // The global subagent default is validated by the backend, so a
            // deleted profile reads back as "follow the conversation".
            if let Ok(value) = invoke_checked("get_subagent_model", JsValue::UNDEFINED).await {
                subagent_model.set(value.as_string().unwrap_or_default());
            }
        })
    }

    /// Persist the global default subagent model (Settings → Specialists).
    /// "" clears it so subagents follow the conversation again.
    pub(crate) fn apply_subagent_model(self, id: String) {
        let Self {
            subagent_model,
            settings_message,
            locale,
            ..
        } = self;
        let previous = subagent_model.get_untracked();
        subagent_model.set(id.clone());
        spawn_local(async move {
            let args = match json_args(&serde_json::json!({ "id": id })) {
                Ok(args) => args,
                Err(_) => {
                    subagent_model.set(previous);
                    return;
                }
            };
            match invoke_checked("set_subagent_model", args).await {
                Ok(value) => subagent_model.set(value.as_string().unwrap_or_default()),
                Err(error) => {
                    subagent_model.set(previous);
                    let msg = crate::app_support::js_error_text(error);
                    settings_message.set(Some((
                        false,
                        tf(
                            locale.get_untracked(),
                            "status.save_failed",
                            &[("msg", &msg)],
                        ),
                    )));
                }
            }
        })
    }

    pub(crate) fn refresh_specialists(self) {
        let specialists = self.specialists;
        spawn_local(async move {
            let v = invoke("list_specialists", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                specialists.set(list);
            }
        })
    }

    /// Persist a per-model reasoning-effort choice from the composer's picker.
    pub(crate) fn apply_model_effort(self, id: String, effort: String) {
        let models = self.models;
        let Some(profile) = models.get_untracked().into_iter().find(|m| m.id == id) else {
            return;
        };
        spawn_local(async move {
            let arg = json_args(&serde_json::json!({
                "profile": {
                    "id": profile.id,
                    "label": profile.label,
                    "provider": profile.provider,
                    "api_url": profile.api_url,
                    "endpoint_suffix": profile.endpoint_suffix,
                    "model": profile.model,
                    "max_tokens": profile.max_tokens,
                    "context_window": profile.context_window,
                    "reasoning_effort": effort,
                    "endpoint_efforts": profile.endpoint_efforts,
                    "service_tier": profile.service_tier,
                    "user_agent": profile.user_agent,
                    "send_user_agent": profile.send_user_agent,
                    "send_session_id": profile.send_session_id,
                    "session_header_name": profile.session_header_name,
                    "supports_vision": profile.supports_vision,
                    "use_for_vision": profile.use_for_vision,
                    "use_for_expert": profile.use_for_expert,
                    "use_for_image_generation": profile.use_for_image_generation,
                    "image_size": profile.image_size,
                    "image_quality": profile.image_quality,
                    "image_aspect_ratio": profile.image_aspect_ratio,
                    "image_resolution": profile.image_resolution,
                    "use_for_video_generation": profile.use_for_video_generation,
                    "video_duration_secs": profile.video_duration_secs,
                    "video_aspect_ratio": profile.video_aspect_ratio,
                    "video_resolution": profile.video_resolution,
                },
                // No key field: the backend keeps the stored key.
                "key": Option::<String>::None,
                "useForVision": profile.use_for_vision,
                "useForImageGeneration": profile.use_for_image_generation,
                "useForVideoGeneration": profile.use_for_video_generation,
            }));
            let arg = match arg {
                Ok(arg) => arg,
                Err(err) => return show_warning_toast(&err.to_string()),
            };
            match invoke_checked("save_model", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                }
                Err(err) => show_warning_toast(&js_error_text(err)),
            }
        });
    }

    pub(crate) fn save_model_form(self) {
        let Self {
            model_form,
            model_catalog_limits,
            model_form_key,
            model_form_msg,
            models,
            settings,
            settings_busy,
            locale,
            ..
        } = self;
        if settings_busy.get() {
            return;
        }
        let Some(form) = model_form.get() else {
            return;
        };
        if form.id.is_none() {
            self.save_provider_form(form);
            return;
        }
        let loc = locale.get();
        let key = model_form_key.get();
        let has_key = form
            .id
            .as_ref()
            .and_then(|id| {
                models
                    .get()
                    .iter()
                    .find(|m| &m.id == id)
                    .map(|m| m.has_api_key)
            })
            .unwrap_or(false);
        let problems = model_form_problems(&form, &key, has_key);
        if report_model_form_problems(model_form, model_form_msg, loc, problems) {
            return;
        }
        if let Some(err_key) = route_error(
            form.proxy_url.as_deref().unwrap_or_default(),
            form.proxy_custom,
        ) {
            let err = t(loc, err_key);
            let text = tf(loc, "status.save_failed", &[("msg", &err)]);
            model_form_msg.set(Some((false, text)));
            return;
        }
        // A catalog-known chat model has a documented output ceiling; saving a
        // larger max_tokens only ever surfaces as a provider 400 mid-turn.
        // Image and video models do not take token limits.
        if !matches!(provider_value(&form.provider), "openai_codex" | "xai_oauth")
            && !form.is_image_model()
            && !is_video_generation_model(&form.model)
        {
            if let Some(dto) = model_catalog_limits.get() {
                if form.max_tokens > dto.max_tokens {
                    let text = tf(
                        loc,
                        "err.max_tokens_ceiling",
                        &[
                            ("model", form.model.trim()),
                            ("max", &dto.max_tokens.to_string()),
                        ],
                    );
                    model_form_msg.set(Some((false, text)));
                    return;
                }
            }
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        let provider = provider_value(&form.provider);
        let mut profile = serde_json::json!({
            "id": form.id.clone().unwrap_or_default(),
            "label": form.label.trim(),
            "provider": provider,
            "api_url": form.api_url.trim(),
            "endpoint_suffix": form.endpoint_suffix.trim(),
            "model": form.model.trim(),
            "max_tokens": form.max_tokens,
            "context_window": form.context_window,
            "reasoning_effort": form.reasoning_effort.trim(),
            "endpoint_efforts": form.endpoint_efforts,
            "service_tier": form.service_tier.trim(),
            "user_agent": form.user_agent,
            "send_user_agent": form.send_user_agent,
            "send_session_id": form.send_session_id,
            "session_header_name": form.session_header_name,
            "supports_vision": form.supports_vision,
            "use_for_vision": form.use_for_vision,
            "use_for_expert": form.use_for_expert && !form.is_image_model() && !is_video_generation_model(&form.model) && !form.use_for_video_generation,
            "use_for_image_generation": form.use_for_image_generation,
            "image_size": form.image_size.trim(),
            "image_quality": form.image_quality.trim(),
            "image_aspect_ratio": form.image_aspect_ratio.trim(),
            "image_resolution": form.image_resolution.trim(),
            "use_for_video_generation": form.use_for_video_generation,
            "video_duration_secs": form.video_duration_secs,
            "video_aspect_ratio": form.video_aspect_ratio,
            "video_resolution": form.video_resolution,
        });
        // Omitted keeps the endpoint's saved route.
        if let Some(proxy_url) = form.proxy_url.as_deref() {
            profile["proxy_url"] = proxy_url.trim().into();
        }
        let key_arg = if key.is_empty() { None } else { Some(key) };
        spawn_local(async move {
            let arg = match json_args(&serde_json::json!({
                "profile": profile,
                "key": key_arg,
                "useForVision": form.use_for_vision,
                "useForImageGeneration": form.use_for_image_generation,
                "useForVideoGeneration": form.use_for_video_generation,
                "restoreChatModel": form.restore_chat_model,
            })) {
                Ok(arg) => arg,
                Err(err) => {
                    let text = tf(loc, "status.save_failed", &[("msg", &err.to_string())]);
                    model_form_msg.set(Some((false, text)));
                    settings_busy.set(false);
                    return;
                }
            };
            match invoke_timeout("save_model", arg, SAVE_MODEL_TIMEOUT_MS).await {
                Ok(v) => {
                    let outcome: (bool, String) =
                        match serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                            Ok(list) => {
                                models.set(list);
                                (true, t(loc, "status.settings_saved").into())
                            }
                            Err(err) => {
                                // The profile is already stored: say so and
                                // reload the list instead of showing a stale one.
                                self.refresh_models();
                                (false, report_unreadable_reply(loc, &err))
                            }
                        };
                    let v = invoke("get_settings", JsValue::UNDEFINED).await;
                    if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                        settings.set(normalized_settings(cfg));
                    }
                    model_form.set(None);
                    model_form_key.set(String::new());
                    model_form_msg.set(Some(outcome));
                }
                Err(err) => {
                    model_form_msg.set(Some((false, localize_backend(loc, &js_error_text(err)))));
                }
            }
            settings_busy.set(false);
        });
    }

    fn save_provider_form(self, form: ModelForm) {
        let Self {
            model_form,
            model_form_key,
            model_form_msg,
            models,
            settings,
            settings_busy,
            locale,
            ..
        } = self;
        let loc = locale.get();
        let key = model_form_key.get();
        let api_url = form.api_url.trim().to_string();
        let existing = models.get();
        let problems =
            model_form_problems(&form, &key, endpoint_has_stored_key(&existing, &api_url));
        if report_model_form_problems(model_form, model_form_msg, loc, problems) {
            return;
        }
        let endpoint_models = form.endpoint_models.clone();
        let mut entries: Vec<ModelFormEntry> = form
            .entries
            .into_iter()
            .filter(|entry| !entry.model.trim().is_empty())
            .collect();
        let has_chat = entries.iter().any(|entry| {
            !entry.is_image_model() && !entry.is_video_model() && !entry.use_for_expert
        }) || existing
            .iter()
            .any(|profile| profile.is_chat_model() && !profile.use_for_expert);
        if !has_chat {
            model_form_msg.set(Some((false, t(loc, "models.need_chat").into())));
            return;
        }
        if let Some(err_key) = route_error(
            form.proxy_url.as_deref().unwrap_or_default(),
            form.proxy_custom,
        ) {
            let err = t(loc, err_key);
            model_form_msg.set(Some((
                false,
                tf(loc, "status.save_failed", &[("msg", &err)]),
            )));
            return;
        }
        let proxy_url = form.proxy_url.as_deref().map(str::trim).map(str::to_string);
        // The first chat model first: `save_model` makes a new chat profile
        // the default only while there is no usable default, so on a fresh
        // install that is this one, and an existing default is never replaced.
        let first_chat = entries.iter().position(|entry| {
            !entry.is_image_model() && !entry.is_video_model() && !entry.use_for_expert
        });
        if let Some(first) = first_chat {
            let default_chat = entries.remove(first);
            entries.insert(0, default_chat);
        }
        let ordered = entries;
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        let key_arg = if key.trim().is_empty() {
            None
        } else {
            Some(key)
        };
        spawn_local(async move {
            let mut last_ok = None;
            let mut last_err = None;
            for entry in ordered {
                let provider = provider_value(&entry.provider).to_string();
                let endpoint_suffix = entry.endpoint_suffix.trim().to_string();
                let effective_api_url = join_api_url(&api_url, &endpoint_suffix);
                let (max_tokens, context_window, endpoint_efforts) = new_profile_limits(
                    &provider,
                    &effective_api_url,
                    entry.model.trim(),
                    endpoint_facts(&endpoint_models, &entry.model),
                )
                .await;
                let image = entry.is_image_model();
                let video = entry.is_video_model();
                let media = image || video;
                let mut profile = serde_json::json!({
                    "id": "",
                    "label": entry.label.trim(),
                    "provider": provider,
                    "api_url": api_url,
                    "endpoint_suffix": endpoint_suffix,
                    "model": entry.model.trim(),
                    "max_tokens": max_tokens,
                    "context_window": context_window,
                    "reasoning_effort": "",
                    "endpoint_efforts": endpoint_efforts,
                    "service_tier": "",
                    "user_agent": form.user_agent,
                    "send_user_agent": form.send_user_agent,
                    "send_session_id": form.send_session_id,
                    "session_header_name": form.session_header_name,
                    "supports_vision": entry.supports_vision && !media,
                    "use_for_vision": entry.use_for_vision && !media,
                    "use_for_expert": entry.use_for_expert && !media,
                    "use_for_image_generation": image,
                    "image_size": "",
                    "image_quality": "",
                    "image_aspect_ratio": "",
                    "image_resolution": "",
                    "use_for_video_generation": video,
                    "video_duration_secs": Option::<u32>::None,
                    "video_aspect_ratio": Option::<String>::None,
                    "video_resolution": Option::<String>::None,
                });
                if let Some(proxy_url) = &proxy_url {
                    profile["proxy_url"] = proxy_url.as_str().into();
                }
                let arg = match json_args(&serde_json::json!({
                    "profile": profile,
                    "key": key_arg,
                    "useForVision": entry.use_for_vision && !media,
                    "useForImageGeneration": image,
                    "useForVideoGeneration": video,
                })) {
                    Ok(arg) => arg,
                    Err(err) => {
                        last_err = Some(err.to_string());
                        break;
                    }
                };
                match invoke_timeout("save_model", arg, SAVE_MODEL_TIMEOUT_MS).await {
                    Ok(v) => last_ok = Some(v),
                    Err(err) => {
                        last_err = Some(js_error_text(err));
                        break;
                    }
                }
            }
            if let Some(err) = last_err {
                model_form_msg.set(Some((false, localize_backend(loc, &err))));
                settings_busy.set(false);
                return;
            }
            let mut outcome: (bool, String) = (true, t(loc, "status.settings_saved").into());
            if let Some(v) = last_ok {
                match serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                    Ok(list) => models.set(list),
                    Err(err) => {
                        // Every profile is already stored: say so and reload
                        // the list instead of showing a stale one.
                        self.refresh_models();
                        outcome = (false, report_unreadable_reply(loc, &err));
                    }
                }
            }
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                settings.set(normalized_settings(cfg));
            }
            model_form.set(None);
            model_form_key.set(String::new());
            model_form_msg.set(Some(outcome));
            settings_busy.set(false);
        });
    }

    pub(crate) fn validate_model_form(self) {
        let Self {
            model_form,
            model_form_key,
            model_form_msg,
            models,
            settings_busy,
            locale,
            ..
        } = self;
        if settings_busy.get() {
            return;
        }
        let Some(form) = model_form.get() else {
            return;
        };
        let loc = locale.get();
        let key = model_form_key.get();
        let listed = models.get();
        let has_key = if form.id.is_none() {
            endpoint_has_stored_key(&listed, &form.api_url)
        } else {
            listed
                .iter()
                .find(|m| Some(m.id.as_str()) == form.id.as_deref())
                .map(|m| m.has_api_key)
                .unwrap_or(false)
        };
        let problems = model_form_problems(&form, &key, has_key);
        if report_model_form_problems(model_form, model_form_msg, loc, problems) {
            return;
        }
        let (form, profile_id) = if form.id.is_none() {
            let Some(entry) = form
                .entries
                .iter()
                .find(|entry| !entry.model.trim().is_empty())
            else {
                return;
            };
            let mut probe = form.clone();
            probe.provider = entry.provider.clone();
            probe.endpoint_suffix = entry.endpoint_suffix.clone();
            probe.model = entry.model.clone();
            probe.label = entry.label.clone();
            probe.supports_vision = entry.supports_vision;
            probe.use_for_vision = entry.use_for_vision;
            probe.use_for_image_generation = entry.use_for_image_generation;
            probe.use_for_video_generation = entry.use_for_video_generation;
            let profile_id = sibling_profile_id(&listed, &form.api_url).map(str::to_string);
            (probe, profile_id)
        } else {
            let profile_id = form.id.clone();
            (form, profile_id)
        };
        let cfg = model_form_to_settings(&form, has_key);
        let proxy_url = model_form_proxy(&listed, &form);
        if let Some(err_key) = route_error(&proxy_url, form.proxy_custom) {
            let err = t(loc, err_key);
            model_form_msg.set(Some((
                false,
                tf(loc, "status.validation_failed", &[("msg", &err)]),
            )));
            return;
        }
        // A model the catalog does not know: once the connection works, read
        // the endpoint's own facts for it (context, output, effort levels).
        let fetch_facts = form.endpoint_models.is_empty()
            && !form.is_image_model()
            && self.model_catalog_limits.get_untracked().is_none()
            && matches!(
                provider_value(&form.provider),
                "openai" | "openai_responses" | "anthropic"
            );
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.validating").into())));
        // The backend probes with a test image when "supports images" is on,
        // so both outcomes say which probe ran — a checked box was never
        // proof that the model takes images.
        let image = form.is_image_model();
        let vision = cfg.supports_vision && !image;
        spawn_local(async move {
            let args = match json_args(&serde_json::json!({
                "settings": cfg,
                "key": key,
                "profileId": profile_id,
                "useForImageGeneration": image,
                "proxyUrl": proxy_url.trim(),
            })) {
                Ok(args) => args,
                Err(err) => {
                    let text = tf(
                        loc,
                        "status.validation_failed",
                        &[("msg", &err.to_string())],
                    );
                    model_form_msg.set(Some((false, text)));
                    settings_busy.set(false);
                    return;
                }
            };
            let res = invoke_timeout("validate_settings", args, 35_000).await;
            match res {
                Ok(v) => {
                    let raw = v
                        .as_string()
                        .unwrap_or_else(|| t(loc, "status.validation_succeeded").into());
                    let mut msg = localize_backend(loc, &raw);
                    if vision {
                        msg.push_str(&t(loc, "status.vision_ok"));
                    }
                    model_form_msg.set(Some((true, msg)));
                    if fetch_facts {
                        settings_busy.set(false);
                        self.fetch_endpoint_models(true);
                    }
                }
                Err(err) => {
                    let mut msg = tf(
                        loc,
                        "status.validation_failed",
                        &[("msg", &localize_backend(loc, &js_error_text(err)))],
                    );
                    if vision {
                        msg.push_str(&t(loc, "err.vision_probe_failed"));
                    }
                    model_form_msg.set(Some((false, msg)));
                }
            }
            settings_busy.set(false);
        });
    }

    /// `GET {base}/models` for the open form: the IDs become Model ID
    /// suggestions and, for a model the catalog does not know, the endpoint's
    /// facts prefill context window, max output and effort levels. `quiet`
    /// (after Test connection) reports nothing when the list is unavailable.
    pub(crate) fn fetch_endpoint_models(self, quiet: bool) {
        let Self {
            model_form,
            model_form_key,
            model_form_msg,
            model_catalog_limits,
            models,
            locale,
            ..
        } = self;
        let Some(form) = model_form.get_untracked() else {
            return;
        };
        if form.endpoint_models_busy {
            return;
        }
        let loc = locale.get_untracked();
        let listed = models.get_untracked();
        let api_url = form.api_url.trim().to_string();
        if api_url.is_empty() {
            if !quiet {
                report_model_form_problems(model_form, model_form_msg, loc, vec!["api_url".into()]);
            }
            return;
        }
        // The add form lists from the shared root with its first row's protocol.
        let (provider, suffix) = if form.id.is_none() {
            form.entries
                .iter()
                .find(|entry| !crate::dto::is_gemini_provider(&entry.provider))
                .map(|entry| (entry.provider.clone(), entry.endpoint_suffix.clone()))
                .unwrap_or_else(|| ("openai".into(), String::new()))
        } else {
            (form.provider.clone(), form.endpoint_suffix.clone())
        };
        let profile_id = if form.id.is_some() {
            form.id.clone()
        } else {
            sibling_profile_id(&listed, &api_url).map(str::to_string)
        };
        let proxy_url = model_form_proxy(&listed, &form);
        let key = model_form_key.get_untracked();
        let requested_url = join_api_url(&api_url, &suffix);
        model_form.update(|form| {
            if let Some(form) = form {
                form.endpoint_models_busy = true;
                if !quiet {
                    form.endpoint_models_status = None;
                }
            }
        });
        spawn_local(async move {
            let args = json_args(&serde_json::json!({
                "provider": provider_value(&provider),
                "apiUrl": requested_url,
                "key": (!key.trim().is_empty()).then_some(key),
                "profileId": profile_id,
                "proxyUrl": proxy_url.trim(),
            }));
            let reply = match args {
                Ok(args) => invoke_timeout("list_endpoint_models", args, 30_000)
                    .await
                    .map_err(js_error_text)
                    .and_then(|value| {
                        serde_wasm_bindgen::from_value::<Vec<EndpointModelDto>>(value)
                            .map_err(|error| error.to_string())
                    }),
                Err(error) => Err(error.to_string()),
            };
            model_form.update(|form| {
                let Some(form) = form else {
                    return;
                };
                form.endpoint_models_busy = false;
                // The address changed while the list was loading.
                if form.api_url.trim() != api_url {
                    return;
                }
                match reply {
                    Ok(list) => {
                        let count = list.len().to_string();
                        form.endpoint_models = list;
                        let prefilled = form.id.is_some()
                            && model_catalog_limits.get_untracked().is_none()
                            && endpoint_facts(&form.endpoint_models, &form.model).is_some();
                        if prefilled {
                            sync_endpoint_facts(form);
                        }
                        let text = if prefilled {
                            tf(
                                loc,
                                "models.fetch_prefilled",
                                &[("count", &count), ("model", form.model.trim())],
                            )
                        } else {
                            tf(loc, "models.fetch_found", &[("count", &count)])
                        };
                        form.endpoint_models_status = Some((true, text));
                    }
                    Err(error) if !quiet => {
                        form.endpoint_models_status = Some((
                            false,
                            tf(
                                loc,
                                "models.fetch_failed",
                                &[("msg", &localize_backend(loc, &error))],
                            ),
                        ));
                    }
                    Err(_) => {}
                }
            });
        });
    }

    pub(crate) fn test_reviewer_form(self) {
        let Self {
            specialist_form,
            model_form_msg,
            settings_busy,
            locale,
            ..
        } = self;
        let Some(spec) = specialist_form.get() else {
            return;
        };
        if spec.id != "reviewer" || settings_busy.get() {
            return;
        }
        let loc = locale.get();
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "specialists.reviewer.testing").into())));
        spawn_local(async move {
            let result = invoke_timeout(
                "test_reviewer_backend",
                to_value(&serde_json::json!({ "reviewer": spec })).unwrap(),
                120_000,
            )
            .await;
            match result {
                Ok(value) => {
                    match serde_wasm_bindgen::from_value::<ReviewerBackendTestResult>(value) {
                        Ok(result) => {
                            let backend = match result.backend.as_str() {
                                "acp_agent" => "ACP",
                                "http_model" => "HTTP",
                                other => other,
                            };
                            let headline = tf(
                                loc,
                                "specialists.reviewer.test_ok",
                                &[
                                    ("backend", backend),
                                    ("model", &result.model),
                                    ("status", &result.status),
                                ],
                            );
                            model_form_msg.set(Some((
                                true,
                                if result.summary.trim().is_empty() {
                                    headline
                                } else {
                                    format!("{headline} {}", result.summary.trim())
                                },
                            )));
                        }
                        Err(error) => model_form_msg.set(Some((false, error.to_string()))),
                    }
                }
                Err(error) => model_form_msg.set(Some((
                    false,
                    tf(
                        loc,
                        "specialists.reviewer.test_failed",
                        &[("msg", &localize_backend(loc, &js_error_text(error)))],
                    ),
                ))),
            }
            settings_busy.set(false);
        });
    }

    pub(crate) fn save_specialist_form(self) {
        let Self {
            specialists,
            specialist_form,
            model_form_msg,
            settings_busy,
            settings_message,
            locale,
            ..
        } = self;
        let Some(spec) = specialist_form.get() else {
            return;
        };
        let loc = locale.get();
        if spec.name.trim().is_empty() {
            model_form_msg.set(Some((false, t(loc, "specialists.name_required").into())));
            return;
        }
        let saved_id = spec.id.clone();
        let keep_open = saved_id == "reviewer";
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "spec": spec })).unwrap();
            match invoke_checked("save_specialist_cmd", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                    Ok(value) => {
                        let saved = value.iter().find(|item| item.id == saved_id).cloned();
                        specialists.set(value);
                        if keep_open {
                            specialist_form.set(saved);
                            model_form_msg.set(Some((true, t(loc, "specialists.saved").into())));
                        } else {
                            specialist_form.set(None);
                            settings_message.set(Some((true, t(loc, "specialists.saved").into())));
                        }
                    }
                    Err(error) => model_form_msg.set(Some((false, error.to_string()))),
                },
                Err(error) => model_form_msg.set(Some((false, js_error_text(error)))),
            }
            settings_busy.set(false);
        });
    }

    /// Export a saved custom specialist to a `.wisp-specialist.json` package.
    /// The backend pauses with a secret warning until the user acknowledges it.
    pub(crate) fn export_specialist(self, id: String, acknowledge_secrets: bool) {
        let Self {
            specialists,
            specialist_form,
            specialist_transfer,
            model_form_msg,
            settings_busy,
            settings_message,
            locale,
            ..
        } = self;
        let report = move |ok: bool, text: String| {
            if specialist_form.get_untracked().is_some() {
                model_form_msg.set(Some((ok, text)));
            } else {
                settings_message.set(Some((ok, text)));
            }
        };
        let loc = locale.get_untracked();
        let name = specialists
            .get_untracked()
            .into_iter()
            .find(|spec| spec.id == id)
            .map(|spec| spec.name)
            .unwrap_or_default();
        settings_busy.set(true);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "id": id,
                "acknowledgeSecrets": acknowledge_secrets,
            }))
            .unwrap();
            match invoke_checked("export_specialist", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<
                    specialist_package::SpecialistExportOutcome,
                >(value)
                {
                    Ok(specialist_package::SpecialistExportOutcome::SecretWarning { findings }) => {
                        specialist_transfer.set(Some(SpecialistTransfer::SecretWarning {
                            id,
                            name,
                            findings,
                        }));
                    }
                    Ok(specialist_package::SpecialistExportOutcome::Cancelled) => {
                        specialist_transfer.set(None);
                    }
                    Ok(specialist_package::SpecialistExportOutcome::Exported { path }) => {
                        specialist_transfer.set(None);
                        report(true, tf(loc, "specialists.export.done", &[("path", &path)]));
                    }
                    Err(error) => {
                        specialist_transfer.set(None);
                        report(false, error.to_string());
                    }
                },
                Err(error) => {
                    specialist_transfer.set(None);
                    report(
                        false,
                        tf(
                            loc,
                            "specialists.export.failed",
                            &[("msg", &localize_backend(loc, &js_error_text(error)))],
                        ),
                    );
                }
            }
            settings_busy.set(false);
        });
    }

    /// Pick a package and open its preview. Nothing is written until the user
    /// confirms the preview.
    pub(crate) fn pick_specialist_import(self) {
        let Self {
            specialist_transfer,
            settings_busy,
            settings_message,
            locale,
            ..
        } = self;
        let loc = locale.get_untracked();
        let fail = move |error: JsValue| {
            settings_message.set(Some((
                false,
                tf(
                    loc,
                    "specialists.import.failed",
                    &[("msg", &localize_backend(loc, &js_error_text(error)))],
                ),
            )));
        };
        settings_message.set(None);
        spawn_local(async move {
            let path = match invoke_checked("pick_specialist_package", JsValue::UNDEFINED).await {
                Ok(value) => value.as_string(),
                Err(error) => return fail(error),
            };
            let Some(path) = path else {
                return;
            };
            settings_busy.set(true);
            let args = to_value(&serde_json::json!({ "path": path })).unwrap();
            match invoke_checked("preview_specialist_import", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<
                    specialist_package::SpecialistImportPreview,
                >(value)
                {
                    Ok(preview) => {
                        // Only an exact provider + model match is preselected;
                        // anything else follows the active model until chosen.
                        let model_id = preview
                            .model_matches
                            .iter()
                            .find(|matched| matched.same_provider)
                            .map(|matched| matched.id.clone())
                            .unwrap_or_default();
                        specialist_transfer.set(Some(SpecialistTransfer::Import {
                            preview,
                            model_id,
                            reviewed: false,
                            error: None,
                        }));
                    }
                    Err(error) => settings_message.set(Some((false, error.to_string()))),
                },
                Err(error) => fail(error),
            }
            settings_busy.set(false);
        });
    }

    pub(crate) fn confirm_specialist_import(self) {
        let Self {
            specialists,
            specialist_transfer,
            settings_busy,
            settings_message,
            locale,
            ..
        } = self;
        let Some(SpecialistTransfer::Import {
            preview,
            model_id,
            reviewed,
            ..
        }) = specialist_transfer.get_untracked()
        else {
            return;
        };
        if !reviewed || preview.name_conflict.is_some() || settings_busy.get_untracked() {
            return;
        }
        let loc = locale.get_untracked();
        let name = preview.package.name.trim().to_string();
        settings_busy.set(true);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "package": preview.package,
                "modelId": model_id,
            }))
            .unwrap();
            let failure = match invoke_checked("import_specialist", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                    Ok(list) => {
                        specialists.set(list);
                        specialist_transfer.set(None);
                        settings_message.set(Some((
                            true,
                            tf(loc, "specialists.import.done", &[("name", &name)]),
                        )));
                        None
                    }
                    Err(error) => Some(error.to_string()),
                },
                Err(error) => Some(localize_backend(loc, &js_error_text(error))),
            };
            if let Some(message) = failure {
                specialist_transfer.update(|transfer| {
                    if let Some(SpecialistTransfer::Import { error, .. }) = transfer {
                        *error = Some(message);
                    }
                });
            }
            settings_busy.set(false);
        });
    }

    pub(crate) fn remove_specialist(self, id: String) {
        let Self {
            specialists,
            settings_message,
            ..
        } = self;
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id })).unwrap();
            match invoke_checked("remove_specialist", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                    Ok(value) => specialists.set(value),
                    Err(error) => settings_message.set(Some((false, error.to_string()))),
                },
                Err(error) => settings_message.set(Some((false, js_error_text(error)))),
            }
        });
    }
}

#[cfg(test)]
mod form_tests {
    use super::{model_field_dom_id, model_form_problems, sync_endpoint_facts};
    use crate::dto::{EndpointModelDto, ModelForm, ModelFormEntry};

    fn edit_form(api_url: &str, model: &str) -> ModelForm {
        ModelForm {
            id: Some("p1".into()),
            provider: "openai".into(),
            api_url: api_url.into(),
            model: model.into(),
            max_tokens: 8192,
            context_window: 128_000,
            ..Default::default()
        }
    }

    #[test]
    fn every_missing_field_is_reported_at_once_in_form_order() {
        let form = edit_form("", "");
        assert_eq!(
            model_form_problems(&form, "", false),
            ["api_url", "api_key", "model"]
        );
        assert_eq!(model_field_dom_id("api_key"), "model-form-api-key");
        let mut add = ModelForm {
            api_url: "https://api.deepseek.com".into(),
            entries: vec![ModelFormEntry {
                row_id: 7,
                endpoint_suffix: "坏网关".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            model_form_problems(&add, "", true),
            ["model", "endpoint_suffix:7"]
        );
        assert_eq!(
            model_field_dom_id("endpoint_suffix:7"),
            "provider-endpoint-suffix-7"
        );
        add.entries[0].model = "deepseek-v4-flash".into();
        add.entries[0].endpoint_suffix.clear();
        assert!(model_form_problems(&add, "", true).is_empty());
    }

    #[test]
    fn local_gateways_need_no_api_key() {
        let form = edit_form("http://127.0.0.1:3425/v1", "deepseek/deepseek-flash");
        assert!(model_form_problems(&form, "", false).is_empty());
        let remote = edit_form("https://api.deepseek.com", "deepseek-v4-flash");
        assert_eq!(model_form_problems(&remote, "", false), ["api_key"]);
        assert!(model_form_problems(&remote, "", true).is_empty());
    }

    #[test]
    fn endpoint_facts_prefill_only_the_exact_id_and_never_typed_limits() {
        let mut form = edit_form("http://127.0.0.1:3425/v1", "deepseek/deepseek-flash");
        form.endpoint_models = vec![
            EndpointModelDto {
                id: "deepseek/deepseek-flash-2".into(),
                context_window: Some(2_000_000),
                ..Default::default()
            },
            EndpointModelDto {
                id: "deepseek/deepseek-flash".into(),
                context_window: Some(1_000_000),
                max_output_tokens: Some(384_000),
                reasoning_levels: Some(vec![
                    "none".into(),
                    "low".into(),
                    "high".into(),
                    "max".into(),
                ]),
            },
        ];
        form.max_tokens_edited = true;
        form.max_tokens = 4096;
        sync_endpoint_facts(&mut form);
        assert_eq!(form.context_window, 1_000_000);
        assert_eq!(form.max_tokens, 4096);
        assert_eq!(form.endpoint_efforts, ["none", "low", "high", "max"]);
        // A prefix of a listed id is not that model.
        form.model = "deepseek/deepseek".into();
        sync_endpoint_facts(&mut form);
        assert!(form.endpoint_efforts.is_empty());
        assert_eq!(form.context_window, 1_000_000);
    }
}
