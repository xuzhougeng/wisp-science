//! Settings → Models → Decision model: the one System One endpoint behind
//! `map_items`' `decide` step and `desktop_autopilot`.
use crate::app_support::{compose_icon, js_error_text};
use crate::bindings::{invoke_checked, open_external_url};
use crate::dto::{
    DecisionModelProbe, DecisionModelSettings, DecisionPreset, DECISION_PRESETS,
    DECISION_THINK_MODES,
};
use crate::i18n::{localize_backend, t, use_locale, Locale};
use crate::text::{event_target_input, event_target_value};
use leptos::*;
use wasm_bindgen::JsValue;

fn preset_label(locale: Locale, id: &str) -> String {
    t(locale, &format!("decision.preset.{id}"))
}

fn think_label(locale: Locale, mode: &str) -> String {
    t(
        locale,
        &format!(
            "decision.think.{}",
            if mode.is_empty() { "default" } else { mode }
        ),
    )
}

/// Whether `settings` can be called with what is stored.
fn usable(settings: &DecisionModelSettings) -> bool {
    DecisionPreset::find(&settings.preset)
        .is_some_and(|preset| !preset.key_required || settings.has_key(preset.id))
}

/// Switch the form to `preset`'s defaults, keeping the switch and the list
/// of stored keys.
fn with_preset(current: &DecisionModelSettings, preset: &DecisionPreset) -> DecisionModelSettings {
    DecisionModelSettings {
        enabled: current.enabled,
        stored_keys: current.stored_keys.clone(),
        ..DecisionModelSettings::from_preset(preset)
    }
}

#[component]
pub(crate) fn DecisionModelSettingsView() -> impl IntoView {
    let locale = use_locale();
    let saved = create_rw_signal(None::<DecisionModelSettings>);
    let draft = create_rw_signal(DecisionModelSettings::default());
    let api_key = create_rw_signal(String::new());
    let busy = create_rw_signal(false);
    let message = create_rw_signal(None::<(bool, String)>);
    spawn_local(async move {
        let result = invoke_checked("get_decision_model", JsValue::UNDEFINED)
            .await
            .map_err(js_error_text)
            .and_then(|value| {
                serde_wasm_bindgen::from_value::<DecisionModelSettings>(value)
                    .map_err(|e| e.to_string())
            });
        if busy.try_get_untracked().is_none() {
            return;
        }
        match result {
            Ok(value) => {
                draft.set(value.clone());
                saved.set(Some(value));
            }
            Err(error) => message.set(Some((false, error))),
        }
    });
    let preset = move || DecisionPreset::find(&draft.get().preset).unwrap_or(&DECISION_PRESETS[0]);
    let key_stored = move || {
        let draft = draft.get();
        draft.has_key(&draft.preset)
    };
    // The hosted services reject keyless calls; say so before the round trip.
    let missing_key =
        move || preset().key_required && !key_stored() && api_key.get_untracked().trim().is_empty();
    let args = move || {
        serde_wasm_bindgen::to_value(&serde_json::json!({
            "settings": draft.get_untracked(),
            "apiKey": api_key.get_untracked(),
        }))
        .unwrap()
    };
    let save = move |_| {
        if busy.get_untracked() || saved.get_untracked().is_none() {
            return;
        }
        if draft.get_untracked().enabled && missing_key() {
            message.set(Some((
                false,
                t(locale.get_untracked(), "decision.key_required"),
            )));
            return;
        }
        busy.set(true);
        message.set(None);
        let args = args();
        spawn_local(async move {
            let result = invoke_checked("save_decision_model", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<DecisionModelSettings>(value)
                        .map_err(|e| e.to_string())
                });
            if busy.try_get_untracked().is_none() {
                return;
            }
            match result {
                Ok(value) => {
                    draft.set(value.clone());
                    saved.set(Some(value));
                    api_key.set(String::new());
                    message.set(Some((true, t(locale.get_untracked(), "decision.saved"))));
                }
                Err(error) => message.set(Some((
                    false,
                    localize_backend(locale.get_untracked(), &error),
                ))),
            }
            busy.set(false);
        });
    };
    let test = move |_| {
        if busy.get_untracked() || saved.get_untracked().is_none() {
            return;
        }
        if missing_key() {
            message.set(Some((
                false,
                t(locale.get_untracked(), "decision.key_required"),
            )));
            return;
        }
        busy.set(true);
        message.set(Some((true, t(locale.get_untracked(), "decision.testing"))));
        let args = args();
        spawn_local(async move {
            let result = invoke_checked("test_decision_model", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<DecisionModelProbe>(value)
                        .map_err(|e| e.to_string())
                });
            if busy.try_get_untracked().is_none() {
                return;
            }
            let locale = locale.get_untracked();
            message.set(Some(match result {
                Ok(probe) => (
                    true,
                    t(locale, "decision.test_ok")
                        .replace(
                            "{model}",
                            if probe.model.is_empty() {
                                "?"
                            } else {
                                &probe.model
                            },
                        )
                        .replace("{ms}", &probe.elapsed_ms.to_string()),
                ),
                Err(error) => (
                    false,
                    t(locale, "decision.test_failed")
                        .replace("{error}", &localize_backend(locale, &error)),
                ),
            }));
            busy.set(false);
        });
    };
    let clear_key = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        message.set(None);
        let preset = draft.get_untracked().preset;
        spawn_local(async move {
            let args =
                serde_wasm_bindgen::to_value(&serde_json::json!({ "preset": preset })).unwrap();
            let result = invoke_checked("clear_decision_model_key", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<DecisionModelSettings>(value)
                        .map_err(|e| e.to_string())
                });
            if busy.try_get_untracked().is_none() {
                return;
            }
            match result {
                Ok(value) => {
                    // Only the stored keys changed; keep unsaved form edits.
                    draft.update(|d| d.stored_keys = value.stored_keys.clone());
                    saved.set(Some(value));
                    message.set(Some((
                        true,
                        t(locale.get_untracked(), "decision.key_cleared"),
                    )));
                }
                Err(error) => message.set(Some((
                    false,
                    localize_backend(locale.get_untracked(), &error),
                ))),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="decision-model-settings" data-testid="decision-model-settings">
            <p class="hint">{move || t(locale.get(), "decision.desc")}</p>
            <Show when=move || saved.get().is_some() fallback=move || view! {
                {move || message.get().map(|(_, text)| view! { <div class="settings-status fail" role="alert">{text}</div> })}
            }>
                <p class="decision-status" data-testid="decision-status"
                    class:on=move || saved.get().is_some_and(|s| s.enabled && usable(&s))>
                    {move || {
                        let locale = locale.get();
                        let Some(current) = saved.get() else { return String::new() };
                        if !current.enabled {
                            t(locale, "decision.status_off")
                        } else if !usable(&current) {
                            t(locale, "decision.status_no_key")
                                .replace("{preset}", &preset_label(locale, &current.preset))
                        } else {
                            let target = if current.model.is_empty() { current.base_url.clone() } else { current.model.clone() };
                            t(locale, "decision.status_on")
                                .replace("{preset}", &preset_label(locale, &current.preset))
                                .replace("{model}", &target)
                        }
                    }}
                </p>
                <div class="decision-enable-row">
                    <span>{move || t(locale.get(), "decision.enabled")}</span>
                    <label class="toggle" title=move || t(locale.get(), "decision.enabled")>
                        <input type="checkbox" data-testid="decision-enabled"
                            aria-label=move || t(locale.get(), "decision.enabled")
                            disabled=move || busy.get()
                            prop:checked=move || draft.get().enabled
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                draft.update(|d| d.enabled = on);
                                message.set(None);
                            } />
                        <span class="toggle-track"></span>
                    </label>
                </div>
                <div class="settings-form-grid">
                    <label class="span-2" for="decision-preset">{move || t(locale.get(), "decision.preset")}
                        <select id="decision-preset" data-testid="decision-preset" disabled=move || busy.get()
                            prop:value=move || draft.get().preset
                            on:change=move |ev| {
                                if let Some(next) = DecisionPreset::find(&event_target_value(&ev)) {
                                    draft.update(|d| *d = with_preset(d, next));
                                    api_key.set(String::new());
                                    message.set(None);
                                }
                            }>
                            {DECISION_PRESETS.iter().map(|p| view! {
                                <option value=p.id prop:selected=move || draft.get().preset == p.id>
                                    {move || preset_label(locale.get(), p.id)}
                                </option>
                            }).collect_view()}
                        </select>
                        <span class="settings-field-hint decision-preset-hint">
                            {move || t(locale.get(), &format!("decision.preset_hint.{}", preset().id))}
                            {move || {
                                let p = preset();
                                (!p.key_url.is_empty()).then(|| {
                                    let url = p.key_url;
                                    view! {
                                        <button type="button" class="cred-external-link" data-testid="decision-preset-link"
                                            on:click=move |_| open_external_url(url.into())>
                                            <span>{move || t(locale.get(), &format!("decision.link.{}", p.id))}</span>
                                            <span aria-hidden="true">{compose_icon("external-link")}</span>
                                        </button>
                                    }
                                })
                            }}
                        </span>
                    </label>
                    <label class="span-2" for="decision-base-url">{move || t(locale.get(), "decision.base_url")}
                        <input id="decision-base-url" data-testid="decision-base-url" disabled=move || busy.get()
                            placeholder=move || { let p = preset(); if p.base_url.is_empty() { "https://decide.example.org".to_string() } else { p.base_url.to_string() } }
                            prop:value=move || draft.get().base_url
                            on:input=move |ev| { let v = event_target_input(&ev).value(); draft.update(|d| d.base_url = v); message.set(None); } />
                    </label>
                    <label for="decision-model-id">{move || t(locale.get(), "decision.model")}
                        <input id="decision-model-id" data-testid="decision-model-id" disabled=move || busy.get()
                            placeholder=move || t(locale.get(), "decision.model_placeholder")
                            prop:value=move || draft.get().model
                            on:input=move |ev| { let v = event_target_input(&ev).value(); draft.update(|d| d.model = v); message.set(None); } />
                    </label>
                    <label for="decision-think-mode">{move || t(locale.get(), "decision.think")}
                        <select id="decision-think-mode" data-testid="decision-think-mode" disabled=move || busy.get()
                            prop:value=move || draft.get().think_mode
                            on:change=move |ev| { let v = event_target_value(&ev); draft.update(|d| d.think_mode = v); message.set(None); }>
                            {DECISION_THINK_MODES.iter().map(|&mode| view! {
                                <option value=mode prop:selected=move || draft.get().think_mode == mode>{move || think_label(locale.get(), mode)}</option>
                            }).collect_view()}
                        </select>
                    </label>
                    <label for="decision-timeout">{move || t(locale.get(), "decision.timeout")}
                        <input id="decision-timeout" data-testid="decision-timeout" type="number" min="1" max="600" step="1"
                            disabled=move || busy.get()
                            prop:value=move || draft.get().timeout_secs.to_string()
                            on:input=move |ev| {
                                let v = event_target_input(&ev).value().trim().parse::<u64>().unwrap_or(0);
                                draft.update(|d| d.timeout_secs = v);
                                message.set(None);
                            } />
                    </label>
                    <span class="settings-field-hint span-2">{move || t(locale.get(), "decision.think_hint")}</span>
                    <label class="span-2" for="decision-api-key">
                        <span class="cred-field-head">
                            <span>{move || format!("{} — {}", t(locale.get(), "decision.api_key"),
                                if key_stored() { t(locale.get(), "cred.stored") }
                                else if preset().key_required { t(locale.get(), "cred.not_stored") }
                                else { t(locale.get(), "decision.key_optional") })}</span>
                            {move || key_stored().then(|| view! {
                                <button type="button" class="linklike" data-testid="decision-clear-key"
                                    disabled=move || busy.get() on:click=clear_key>{move || t(locale.get(), "cred.clear")}</button>
                            })}
                        </span>
                        <input id="decision-api-key" data-testid="decision-api-key" type="password" autocomplete="off"
                            disabled=move || busy.get()
                            placeholder=move || if key_stored() { t(locale.get(), "settings.stored_key") } else { String::new() }
                            prop:value=move || api_key.get()
                            on:input=move |ev| { api_key.set(event_target_input(&ev).value()); message.set(None); } />
                    </label>
                </div>
                <p class="settings-field-hint">{move || t(locale.get(), "decision.egress")}</p>
                {move || message.get().map(|(ok, text)| view! {
                    <div class="settings-status" data-testid="decision-message" class:ok=ok class:fail=!ok
                        role=if ok { "status" } else { "alert" }>{text}</div>
                })}
                <div class="row settings-footer">
                    <button type="button" data-testid="decision-test" disabled=move || busy.get() on:click=test>
                        {move || t(locale.get(), "decision.test")}
                    </button>
                    <button type="button" class="primary" data-testid="decision-save" disabled=move || busy.get() on:click=save>
                        {move || t(locale.get(), "settings.save")}
                    </button>
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_presets_keeps_the_switch_and_stored_keys() {
        let current = DecisionModelSettings {
            enabled: true,
            preset: "typesafe".into(),
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-latest".into(),
            think_mode: String::new(),
            timeout_secs: 10,
            stored_keys: vec!["typesafe".into()],
        };
        let next = with_preset(&current, DecisionPreset::find("startlux_bamboo").unwrap());
        assert!(next.enabled);
        assert_eq!(next.base_url, "https://api.bamboonode.dev");
        assert_eq!(next.model, "startlux-decision-27b-autothink");
        assert_eq!(next.timeout_secs, 60);
        assert_eq!(next.stored_keys, ["typesafe"]);
        assert!(!usable(&next), "Bamboo has no stored key yet");
        assert!(usable(&current));
        assert!(usable(&with_preset(
            &current,
            DecisionPreset::find("startlux_local").unwrap()
        )));
    }
}
