//! Model & specialist settings domain: the form signals behind the Settings
//! "Models" and "Specialists" panes plus their save/validate/test handlers.
//! `App` constructs `ModelSettingsState` with the cross-domain signals it
//! depends on (profile list, settings, busy flag, locale) and passes the
//! handlers down to `SettingsView` as callbacks.

use super::*;
use crate::bindings::invoke_timeout;

async fn catalog_limits_or_default(provider: &str, api_url: &str, model: &str) -> (u64, u64) {
    let args = to_value(&serde_json::json!({
        "provider": provider,
        "apiUrl": api_url,
        "model": model,
    }))
    .unwrap();
    match invoke_checked("model_catalog_lookup", args).await {
        Ok(value) => serde_wasm_bindgen::from_value::<Option<CatalogEntryDto>>(value)
            .ok()
            .flatten()
            .map(|dto| (dto.max_tokens, dto.context_window))
            .unwrap_or((8192, 128_000)),
        Err(_) => (8192, 128_000),
    }
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
            models, acp_agents, ..
        } = self;
        spawn_local(async move {
            let v = invoke("list_models", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                models.set(list);
            }
            let v = invoke("list_acp_agents", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(v) {
                acp_agents.set(list);
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
            let arg = to_value(&serde_json::json!({
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
                    "service_tier": profile.service_tier,
                    "supports_vision": profile.supports_vision,
                    "use_for_vision": profile.use_for_vision,
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
            }))
            .unwrap();
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
        let cfg = model_form_to_settings(&form, has_key && key.is_empty());
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            let text = tf(loc, "status.save_failed", &[("msg", &err)]);
            model_form_msg.set(Some((false, text)));
            return;
        }
        // A catalog-known chat model has a documented output ceiling; saving a
        // larger max_tokens only ever surfaces as a provider 400 mid-turn.
        // Image and video models do not take token limits.
        if !is_image_generation_model(&form.model) && !is_video_generation_model(&form.model) {
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
        let profile = serde_json::json!({
            "id": form.id.clone().unwrap_or_default(),
            "label": form.label.trim(),
            "provider": provider,
            "api_url": form.api_url.trim(),
            "endpoint_suffix": form.endpoint_suffix.trim(),
            "model": form.model.trim(),
            "max_tokens": form.max_tokens,
            "context_window": form.context_window,
            "reasoning_effort": form.reasoning_effort.trim(),
            "service_tier": form.service_tier.trim(),
            "supports_vision": form.supports_vision,
            "use_for_vision": form.use_for_vision,
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
        let key_arg = if key.is_empty() { None } else { Some(key) };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "profile": profile,
                "key": key_arg,
                "useForVision": form.use_for_vision,
                "useForImageGeneration": form.use_for_image_generation,
                "useForVideoGeneration": form.use_for_video_generation,
            }))
            .unwrap();
            match invoke_checked("save_model", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                    let v = invoke("get_settings", JsValue::UNDEFINED).await;
                    if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                        settings.set(normalized_settings(cfg));
                    }
                    model_form.set(None);
                    model_form_key.set(String::new());
                    model_form_msg.set(Some((true, t(loc, "status.settings_saved").into())));
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
        if api_url.is_empty() {
            let err = t(loc, "err.api_url_required");
            model_form_msg.set(Some((
                false,
                tf(loc, "status.save_failed", &[("msg", &err)]),
            )));
            return;
        }
        let mut entries: Vec<ModelFormEntry> = form
            .entries
            .into_iter()
            .filter(|entry| !entry.model.trim().is_empty())
            .collect();
        if entries.is_empty() {
            let err = t(loc, "err.model_required");
            model_form_msg.set(Some((
                false,
                tf(loc, "status.save_failed", &[("msg", &err)]),
            )));
            return;
        }
        let existing = models.get();
        let has_chat = entries
            .iter()
            .any(|entry| !entry.is_image_model() && !entry.is_video_model())
            || existing.iter().any(|profile| profile.is_chat_model());
        if !has_chat {
            model_form_msg.set(Some((false, t(loc, "models.need_chat").into())));
            return;
        }
        if key.trim().is_empty() && !endpoint_has_stored_key(&existing, &api_url) {
            let err = t(loc, "err.api_key_required");
            model_form_msg.set(Some((
                false,
                tf(loc, "status.save_failed", &[("msg", &err)]),
            )));
            return;
        }
        // Image and video models first, then extra chat models, first chat
        // last so `save_model` leaves the cheaper/default chat profile active.
        let first_chat = entries
            .iter()
            .position(|entry| !entry.is_image_model() && !entry.is_video_model());
        let mut ordered = Vec::with_capacity(entries.len());
        if let Some(first) = first_chat {
            let default_chat = entries.remove(first);
            let mut media = Vec::new();
            let mut other = Vec::new();
            for entry in entries {
                if entry.is_image_model() || entry.is_video_model() {
                    media.push(entry);
                } else {
                    other.push(entry);
                }
            }
            ordered.append(&mut media);
            ordered.append(&mut other);
            ordered.push(default_chat);
        } else {
            ordered = entries;
        }
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
                let (max_tokens, context_window) =
                    catalog_limits_or_default(&provider, &effective_api_url, entry.model.trim())
                        .await;
                let image = entry.is_image_model();
                let video = entry.is_video_model();
                let media = image || video;
                let profile = serde_json::json!({
                    "id": "",
                    "label": entry.label.trim(),
                    "provider": provider,
                    "api_url": api_url,
                    "endpoint_suffix": endpoint_suffix,
                    "model": entry.model.trim(),
                    "max_tokens": max_tokens,
                    "context_window": context_window,
                    "reasoning_effort": "",
                    "service_tier": "",
                    "supports_vision": entry.supports_vision && !media,
                    "use_for_vision": entry.use_for_vision && !media,
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
                let arg = to_value(&serde_json::json!({
                    "profile": profile,
                    "key": key_arg,
                    "useForVision": entry.use_for_vision && !media,
                    "useForImageGeneration": image,
                    "useForVideoGeneration": video,
                }))
                .unwrap();
                match invoke_checked("save_model", arg).await {
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
            if let Some(v) = last_ok {
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                    models.set(list);
                }
            }
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                settings.set(normalized_settings(cfg));
            }
            model_form.set(None);
            model_form_key.set(String::new());
            model_form_msg.set(Some((true, t(loc, "status.settings_saved").into())));
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
        let (form, has_key, profile_id) = if form.id.is_none() {
            let Some(entry) = form
                .entries
                .iter()
                .find(|entry| !entry.model.trim().is_empty())
            else {
                let err = t(loc, "err.model_required");
                model_form_msg.set(Some((
                    false,
                    tf(loc, "status.validation_failed", &[("msg", &err)]),
                )));
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
            let has_key = endpoint_has_stored_key(&listed, &form.api_url);
            let profile_id = sibling_profile_id(&listed, &form.api_url).map(str::to_string);
            (probe, has_key, profile_id)
        } else {
            let has_key = listed
                .iter()
                .find(|m| Some(m.id.as_str()) == form.id.as_deref())
                .map(|m| m.has_api_key)
                .unwrap_or(false);
            let profile_id = form.id.clone();
            (form, has_key, profile_id)
        };
        let cfg = model_form_to_settings(&form, has_key);
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            model_form_msg.set(Some((
                false,
                tf(loc, "status.validation_failed", &[("msg", &err)]),
            )));
            return;
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.validating").into())));
        // The backend probes with a test image when "supports images" is on,
        // so both outcomes say which probe ran — a checked box was never
        // proof that the model takes images.
        let vision = cfg.supports_vision;
        spawn_local(async move {
            let res = invoke_timeout(
                "validate_settings",
                to_value(&serde_json::json!({
                    "settings": cfg,
                    "key": key,
                    "profileId": profile_id,
                }))
                .unwrap(),
                35_000,
            )
            .await;
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
