//! Settings → Models → Decision model: one System One endpoint (TypeSafe Jev,
//! StartLux-Decision, or a custom server) for `map_items`' `decide` step and
//! `desktop_autopilot`.
//!
//! The form persists as JSON under `decision_model`; keys stay in the keyring,
//! one entry per preset so switching presets keeps each service's key. The
//! TypeSafe preset keeps the existing `typesafe_api_key` entry, so a key saved
//! under Credentials before this setting existed keeps the decision model on
//! with no migration.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use tauri::State;
use wisp_dto::{
    DecisionModelProbe, DecisionModelSettings, DecisionPreset, DECISION_PRESETS,
    DECISION_THINK_MODES,
};
use wisp_llm::systemone::{self, DecisionModel, Question};
use wisp_store::Store;

use crate::models::{secret_del, secret_get, secret_set};

const SETTINGS_KEY: &str = "decision_model";
/// The preset that applies before the card was ever saved.
const LEGACY_PRESET: &str = "typesafe";
const PROBE_ID: &str = "probe";
pub(crate) const KEY_REQUIRED: &str = "This decision model service needs an API key.";
pub(crate) const URL_INVALID: &str =
    "The decision model base URL must start with http:// or https://.";

/// The keyring entry holding a preset's key.
fn key_secret(preset: &str) -> String {
    if preset == LEGACY_PRESET {
        "typesafe_api_key".into()
    } else {
        format!("decision_api_key:{preset}")
    }
}

/// The saved settings, or TypeSafe — on when its key exists — before the card
/// was ever saved. `key` reads one keyring entry.
fn settings_from(raw: Option<&str>, key: &dyn Fn(&str) -> String) -> DecisionModelSettings {
    let mut settings = raw
        .and_then(|raw| serde_json::from_str::<DecisionModelSettings>(raw).ok())
        .filter(|saved| DecisionPreset::find(&saved.preset).is_some())
        .unwrap_or_else(|| {
            let mut legacy = DecisionModelSettings::from_preset(
                DecisionPreset::find(LEGACY_PRESET).expect("TypeSafe preset"),
            );
            legacy.enabled = !key(&key_secret(LEGACY_PRESET)).trim().is_empty();
            legacy
        });
    settings.stored_keys = DECISION_PRESETS
        .iter()
        .filter(|preset| !key(&key_secret(preset.id)).trim().is_empty())
        .map(|preset| preset.id.to_string())
        .collect();
    settings
}

/// Validate one form submission. An empty base URL or timeout takes the
/// preset's default.
fn normalize(mut settings: DecisionModelSettings) -> Result<DecisionModelSettings, String> {
    let preset = DecisionPreset::find(settings.preset.trim())
        .ok_or_else(|| format!("Unknown decision model preset: {}", settings.preset))?;
    settings.preset = preset.id.into();
    settings.base_url = settings.base_url.trim().trim_end_matches('/').to_string();
    if settings.base_url.is_empty() {
        settings.base_url = preset.base_url.into();
    }
    if !(settings.base_url.starts_with("https://") || settings.base_url.starts_with("http://"))
        || settings.base_url.chars().any(char::is_whitespace)
    {
        return Err(URL_INVALID.into());
    }
    settings.model = settings.model.trim().to_string();
    if settings.model.chars().any(char::is_whitespace) {
        return Err("The decision model ID must not contain spaces.".into());
    }
    settings.think_mode = settings.think_mode.trim().to_ascii_lowercase();
    if !DECISION_THINK_MODES.contains(&settings.think_mode.as_str()) {
        return Err(format!("Unknown think mode: {}", settings.think_mode));
    }
    settings.timeout_secs = match settings.timeout_secs {
        0 => preset.timeout_secs,
        secs => secs.clamp(1, 600),
    };
    settings.stored_keys.clear();
    Ok(settings)
}

fn build(settings: &DecisionModelSettings, api_key: &str) -> DecisionModel {
    let mut model = DecisionModel::new(&settings.base_url, api_key, &settings.model);
    model.think_mode = settings.think_mode.clone();
    model.timeout = Duration::from_secs(settings.timeout_secs.clamp(1, 600));
    model
}

/// The endpoint the settings describe when it can be called: enabled, and
/// holding a key when the service needs one.
fn model_from(settings: &DecisionModelSettings, api_key: &str) -> Option<DecisionModel> {
    let preset = DecisionPreset::find(&settings.preset)?;
    if !settings.enabled || (preset.key_required && api_key.trim().is_empty()) {
        return None;
    }
    Some(build(settings, api_key))
}

/// The key a save or test uses: the one typed into the form, else the stored one.
fn effective_key(preset: &str, typed: Option<String>) -> String {
    match typed.as_deref().map(str::trim) {
        Some(typed) if !typed.is_empty() => typed.to_string(),
        _ => secret_get(&key_secret(preset)),
    }
}

pub(crate) async fn load(store: &Store) -> DecisionModelSettings {
    let raw = store.get_setting(SETTINGS_KEY).await.ok().flatten();
    settings_from(raw.as_deref(), &|name: &str| secret_get(name))
}

/// The configured decision model on the model-traffic proxy, or `None` when
/// it is off or lacks a required key.
pub(crate) async fn configured(store: &Store) -> Option<DecisionModel> {
    let settings = load(store).await;
    model_from(&settings, &secret_get(&key_secret(&settings.preset)))
        .map(|model| model.with_proxy(crate::llm_proxy()))
}

#[tauri::command]
pub(crate) async fn get_decision_model(
    state: State<'_, crate::AppState>,
) -> Result<DecisionModelSettings, String> {
    Ok(load(&state.store).await)
}

/// Save the form. A non-empty `api_key` replaces the preset's stored key; an
/// empty one keeps it.
#[tauri::command]
pub(crate) async fn save_decision_model(
    state: State<'_, crate::AppState>,
    settings: DecisionModelSettings,
    api_key: Option<String>,
) -> Result<DecisionModelSettings, String> {
    let settings = normalize(settings)?;
    let preset = DecisionPreset::find(&settings.preset).expect("normalized preset");
    let typed = api_key.as_deref().map(str::trim).unwrap_or_default();
    if settings.enabled
        && preset.key_required
        && effective_key(preset.id, api_key.clone()).trim().is_empty()
    {
        return Err(KEY_REQUIRED.into());
    }
    if !typed.is_empty() {
        secret_set(&key_secret(preset.id), typed)?;
    }
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(SETTINGS_KEY, &json)
        .await
        .map_err(|e| e.to_string())?;
    // Agents built earlier hold the old endpoint (or none) in their tools.
    crate::clear_idle_agents(&state).await;
    Ok(load(&state.store).await)
}

#[tauri::command]
pub(crate) async fn clear_decision_model_key(
    state: State<'_, crate::AppState>,
    preset: String,
) -> Result<DecisionModelSettings, String> {
    let preset = DecisionPreset::find(preset.trim())
        .ok_or_else(|| format!("Unknown decision model preset: {preset}"))?;
    // Clearing a never-stored key is fine; the cache records "absent".
    let _ = secret_del(&key_secret(preset.id));
    crate::clear_idle_agents(&state).await;
    Ok(load(&state.store).await)
}

fn probe_questions() -> BTreeMap<String, Question> {
    BTreeMap::from([(
        PROBE_ID.to_string(),
        Question::Noul {
            instructions: "Is `statement` arithmetically correct?".into(),
            criteria: None,
        },
    )])
}

/// Ask the form's endpoint one fixed question, so the user sees whether the
/// URL, key, and model work before saving. Nothing is saved.
#[tauri::command]
pub(crate) async fn test_decision_model(
    settings: DecisionModelSettings,
    api_key: Option<String>,
) -> Result<DecisionModelProbe, String> {
    let settings = normalize(settings)?;
    let preset = DecisionPreset::find(&settings.preset).expect("normalized preset");
    let key = effective_key(preset.id, api_key);
    if preset.key_required && key.trim().is_empty() {
        return Err(KEY_REQUIRED.into());
    }
    let model = build(&settings, &key).with_proxy(crate::llm_proxy());
    let started = Instant::now();
    let decisions = systemone::evaluate(
        &model,
        &serde_json::json!({"statement": "2 + 2 = 4"}),
        &probe_questions(),
    )
    .await
    .map_err(|e| e.to_string())?;
    if !decisions.answers.contains_key(PROBE_ID) {
        return Err("The decision model answered without the test question.".into());
    }
    Ok(DecisionModelProbe {
        model: decisions.model,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(stored: &'static [&'static str]) -> impl Fn(&str) -> String {
        move |name: &str| {
            if stored.contains(&name) {
                "sk-stored".into()
            } else {
                String::new()
            }
        }
    }

    #[test]
    fn an_old_typesafe_key_turns_the_unsaved_card_on() {
        let off = settings_from(None, &keys(&[]));
        assert_eq!(off.preset, "typesafe");
        assert!(!off.enabled);
        assert!(off.stored_keys.is_empty());

        let legacy = settings_from(None, &keys(&["typesafe_api_key"]));
        assert!(legacy.enabled);
        assert_eq!(legacy.base_url, "https://api.typesafe.ai");
        assert_eq!(legacy.model, "jev-latest");
        assert_eq!(legacy.stored_keys, ["typesafe"]);
        let model = model_from(&legacy, "sk-stored").unwrap();
        assert_eq!(model.http.model, "jev-latest");
        assert_eq!(model.timeout, Duration::from_secs(10));
    }

    #[test]
    fn saved_settings_win_and_keep_every_presets_key() {
        let raw = r#"{"enabled":true,"preset":"startlux_bamboo","base_url":"https://api.bamboonode.dev","model":"startlux-decision-27b-autothink","think_mode":"never","timeout_secs":45}"#;
        let saved = settings_from(
            Some(raw),
            &keys(&["typesafe_api_key", "decision_api_key:startlux_bamboo"]),
        );
        assert_eq!(saved.preset, "startlux_bamboo");
        assert_eq!(saved.stored_keys, ["typesafe", "startlux_bamboo"]);
        let model = model_from(&saved, "bk-key").unwrap();
        assert_eq!(model.http.base_url, "https://api.bamboonode.dev");
        assert_eq!(model.http.api_key, "bk-key");
        assert_eq!(model.think_mode, "never");
        assert_eq!(model.timeout, Duration::from_secs(45));
        // A hosted preset without its key is not callable.
        assert!(model_from(&saved, " ").is_none());
        // Turned off, nothing is offered even with a key.
        let off = DecisionModelSettings {
            enabled: false,
            ..saved
        };
        assert!(model_from(&off, "bk-key").is_none());
        // An unreadable saved value falls back like an unsaved card.
        assert_eq!(
            settings_from(Some(r#"{"preset":"gone"}"#), &keys(&[])).preset,
            "typesafe"
        );
    }

    #[test]
    fn a_keyless_self_hosted_server_is_callable() {
        let local = normalize(DecisionModelSettings {
            enabled: true,
            preset: "startlux_local".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(local.base_url, "http://127.0.0.1:8090");
        assert_eq!(local.timeout_secs, 60);
        let model = model_from(&local, "").unwrap();
        assert_eq!(model.http.api_key, "");
        assert_eq!(model.http.model, "");
    }

    #[test]
    fn normalize_rejects_what_the_server_could_not_use() {
        let form = |preset: &str, base_url: &str, model: &str, think: &str| DecisionModelSettings {
            enabled: true,
            preset: preset.into(),
            base_url: base_url.into(),
            model: model.into(),
            think_mode: think.into(),
            timeout_secs: 9000,
            stored_keys: vec!["typesafe".into()],
        };
        let ok = normalize(form(
            "custom",
            " https://decide.example.org/v1/ ",
            " m1 ",
            " AUTO ",
        ))
        .unwrap();
        assert_eq!(ok.base_url, "https://decide.example.org/v1");
        assert_eq!(ok.model, "m1");
        assert_eq!(ok.think_mode, "auto");
        assert_eq!(ok.timeout_secs, 600);
        assert!(ok.stored_keys.is_empty(), "stored_keys is read-only");

        assert_eq!(
            normalize(form("custom", "", "", "")).unwrap_err(),
            URL_INVALID
        );
        assert_eq!(
            normalize(form("custom", "ftp://x", "", "")).unwrap_err(),
            URL_INVALID
        );
        assert!(normalize(form("custom", "https://x", "two words", "")).is_err());
        assert!(normalize(form("custom", "https://x", "", "sometimes")).is_err());
        assert!(normalize(form("other", "https://x", "", "")).is_err());
    }

    #[test]
    fn typesafe_keeps_its_credential_entry() {
        assert_eq!(key_secret("typesafe"), "typesafe_api_key");
        assert_eq!(
            key_secret("startlux_bamboo"),
            "decision_api_key:startlux_bamboo"
        );
    }
}
