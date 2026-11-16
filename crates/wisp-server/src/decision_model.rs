//! The decision model: one System One endpoint (TypeSafe Jev,
//! StartLux-Decision or a custom server) that answers typed yes/no,
//! choice and score questions with a probability for every option. When
//! one is in use, `map_items` offers its `decide` step.
//!
//! The form's checks follow `src-tauri/src/decision_model.rs`, which
//! cannot be shared from here because it lives in the Tauri shell. The
//! form is kept as JSON under the WebView's `decision_model` key; keys
//! stay in the secret store, one per service so switching services keeps
//! each one's key, under this data directory's own names like every
//! other key of this host.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, ensure, Result};
use wisp_dto::{
    DecisionModelProbe, DecisionModelSettings, DecisionPreset, DECISION_PRESETS,
    DECISION_THINK_MODES,
};
use wisp_llm::systemone::{self, DecisionModel, Question};
use wisp_store::Store;

use crate::desktop::SecretStore;

pub(crate) const SETTINGS_KEY: &str = "decision_model";
const FIRST_PRESET: &str = "typesafe";
const PROBE_ID: &str = "probe";
pub(crate) const KEY_REQUIRED: &str = "This decision model service needs an API key.";

/// The secret holding a service's key in this data directory.
pub(crate) fn secret(key_id: &str, preset: &str) -> String {
    format!("desktop.decision.{key_id}.{preset}")
}

/// What is saved, or the first service switched off before anything was.
/// `key` reads one service's key.
fn settings_from(raw: Option<&str>, key: &dyn Fn(&str) -> Option<String>) -> DecisionModelSettings {
    let mut settings = raw
        .and_then(|raw| serde_json::from_str::<DecisionModelSettings>(raw).ok())
        .filter(|saved| DecisionPreset::find(&saved.preset).is_some())
        .unwrap_or_else(|| {
            DecisionModelSettings::from_preset(
                DecisionPreset::find(FIRST_PRESET).expect("the first preset"),
            )
        });
    settings.stored_keys = DECISION_PRESETS
        .iter()
        .filter(|preset| key(preset.id).is_some_and(|key| !key.trim().is_empty()))
        .map(|preset| preset.id.to_owned())
        .collect();
    settings
}

/// Checks one form. An empty address or timeout takes the service's own.
fn normalize(mut settings: DecisionModelSettings) -> Result<DecisionModelSettings> {
    let preset = DecisionPreset::find(settings.preset.trim())
        .ok_or_else(|| anyhow!("Unknown decision model preset: {}", settings.preset))?;
    settings.preset = preset.id.into();
    settings.base_url = settings.base_url.trim().trim_end_matches('/').to_owned();
    if settings.base_url.is_empty() {
        settings.base_url = preset.base_url.into();
    }
    ensure!(
        (settings.base_url.starts_with("https://") || settings.base_url.starts_with("http://"))
            && !settings.base_url.chars().any(char::is_whitespace),
        "The decision model base URL must start with http:// or https://."
    );
    settings.model = settings.model.trim().to_owned();
    ensure!(
        !settings.model.chars().any(char::is_whitespace),
        "The decision model ID must not contain spaces."
    );
    settings.think_mode = settings.think_mode.trim().to_ascii_lowercase();
    ensure!(
        DECISION_THINK_MODES.contains(&settings.think_mode.as_str()),
        "Unknown think mode: {}",
        settings.think_mode
    );
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

/// The endpoint the settings describe when it can be called: switched
/// on, and holding a key when the service needs one.
fn model_from(settings: &DecisionModelSettings, api_key: &str) -> Option<DecisionModel> {
    let preset = DecisionPreset::find(&settings.preset)?;
    if !settings.enabled || (preset.key_required && api_key.trim().is_empty()) {
        return None;
    }
    Some(build(settings, api_key))
}

/// Reads one service's key. With no key id yet nothing was ever saved.
fn stored(secrets: &dyn SecretStore, key_id: Option<&str>, preset: &str) -> Option<String> {
    secrets
        .get(&secret(key_id?, preset))
        .filter(|key| !key.trim().is_empty())
}

pub(crate) async fn load(
    store: &Store,
    secrets: &dyn SecretStore,
    key_id: Option<&str>,
) -> DecisionModelSettings {
    let raw = store.get_setting(SETTINGS_KEY).await.ok().flatten();
    settings_from(raw.as_deref(), &|preset| stored(secrets, key_id, preset))
}

/// The decision model in use, on the model API's route; `None` when it
/// is off or lacks a key its service needs.
pub(crate) async fn configured(
    store: &Store,
    secrets: &dyn SecretStore,
    key_id: Option<&str>,
) -> Option<DecisionModel> {
    let settings = load(store, secrets, key_id).await;
    let key = stored(secrets, key_id, &settings.preset).unwrap_or_default();
    let network = crate::network::load(store).await;
    model_from(&settings, &key)
        .map(|model| model.with_proxy(crate::network::route(&network.model_proxy_url)))
}

/// The key a save or a test uses: the one typed, else the stored one.
fn effective_key(
    secrets: &dyn SecretStore,
    key_id: Option<&str>,
    preset: &str,
    typed: Option<&str>,
) -> String {
    match typed.map(str::trim) {
        Some(typed) if !typed.is_empty() => typed.to_owned(),
        _ => stored(secrets, key_id, preset).unwrap_or_default(),
    }
}

/// Keeps the form. A typed key replaces the service's stored one; none
/// keeps it. A refused form changes nothing.
pub(crate) async fn save(
    store: &Store,
    secrets: &dyn SecretStore,
    key_id: &str,
    settings: DecisionModelSettings,
    api_key: Option<&str>,
) -> Result<DecisionModelSettings> {
    let settings = normalize(settings)?;
    let preset = DecisionPreset::find(&settings.preset).expect("a checked preset");
    let typed = api_key.map(str::trim).unwrap_or_default();
    ensure!(
        typed.len() <= 4_096 && !typed.contains(char::is_whitespace),
        "An API key is one word of at most 4096 characters"
    );
    if settings.enabled
        && preset.key_required
        && effective_key(secrets, Some(key_id), preset.id, api_key).is_empty()
    {
        bail!(KEY_REQUIRED);
    }
    if !typed.is_empty() {
        secrets.set(&secret(key_id, preset.id), typed)?;
    }
    store
        .set_setting(SETTINGS_KEY, &serde_json::to_string(&settings)?)
        .await?;
    Ok(load(store, secrets, Some(key_id)).await)
}

/// A key as a form would take it: one word of at most 4096 characters.
pub(crate) fn usable_key(key: &str) -> Option<&str> {
    let key = key.trim();
    (!key.is_empty() && key.len() <= 4_096 && !key.contains(char::is_whitespace)).then_some(key)
}

/// Takes a form another desktop saved, as these checks take it, with the
/// keys it kept for its services in `keys`, by service; a key a form would
/// refuse is not kept. Unlike a save, a switched-on service whose key is
/// missing is taken as it is, off in use until its key is entered here.
/// Gives the services whose keys were kept; a refused form changes nothing.
pub(crate) async fn adopt(
    store: &Store,
    secrets: &dyn SecretStore,
    key_id: &str,
    settings: DecisionModelSettings,
    keys: &[(String, String)],
) -> Result<Vec<String>> {
    let settings = normalize(settings)?;
    let mut kept = Vec::new();
    for (preset, key) in keys {
        let (Some(preset), Some(key)) = (DecisionPreset::find(preset), usable_key(key)) else {
            continue;
        };
        if secrets.set(&secret(key_id, preset.id), key).is_ok() {
            kept.push(preset.id.to_owned());
        }
    }
    store
        .set_setting(SETTINGS_KEY, &serde_json::to_string(&settings)?)
        .await?;
    Ok(kept)
}

/// A form as a save would keep it, or why a save would refuse it.
pub(crate) fn checked(settings: DecisionModelSettings) -> Result<DecisionModelSettings> {
    normalize(settings)
}

/// Forgets one service's key.
pub(crate) fn forget_key(secrets: &dyn SecretStore, key_id: &str, preset: &str) -> Result<()> {
    let preset = DecisionPreset::find(preset.trim())
        .ok_or_else(|| anyhow!("Unknown decision model preset: {preset}"))?;
    secrets.set(&secret(key_id, preset.id), "")
}

/// Asks the form's endpoint one fixed question, so the address, key and
/// model are known to work before they are saved. Nothing is saved.
pub(crate) async fn test(
    store: &Store,
    secrets: &dyn SecretStore,
    key_id: Option<&str>,
    settings: DecisionModelSettings,
    api_key: Option<&str>,
) -> Result<DecisionModelProbe> {
    let settings = normalize(settings)?;
    let preset = DecisionPreset::find(&settings.preset).expect("a checked preset");
    let key = effective_key(secrets, key_id, preset.id, api_key);
    ensure!(!(preset.key_required && key.is_empty()), KEY_REQUIRED);
    let network = crate::network::load(store).await;
    let model = build(&settings, &key).with_proxy(crate::network::route(&network.model_proxy_url));
    let questions = BTreeMap::from([(
        PROBE_ID.to_owned(),
        Question::Noul {
            instructions: "Is `statement` arithmetically correct?".into(),
            criteria: None,
        },
    )]);
    let started = Instant::now();
    let decisions = systemone::evaluate(
        &model,
        &serde_json::json!({"statement": "2 + 2 = 4"}),
        &questions,
    )
    .await?;
    ensure!(
        decisions.answers.contains_key(PROBE_ID),
        "The decision model answered without the test question."
    );
    Ok(DecisionModelProbe {
        model: decisions.model,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::MemorySecrets;

    fn form(preset: &str) -> DecisionModelSettings {
        DecisionModelSettings::from_preset(DecisionPreset::find(preset).unwrap())
    }

    #[test]
    fn a_form_is_checked_and_takes_the_services_own_address_and_timeout() {
        let mut custom = form("custom");
        assert!(normalize(custom.clone()).is_err(), "no address");
        custom.base_url = " http://lab.example:8090/v1/ ".into();
        custom.think_mode = " Auto ".into();
        custom.timeout_secs = 9_000;
        custom.stored_keys = vec!["typesafe".into()];
        let checked = normalize(custom.clone()).unwrap();
        assert_eq!(checked.base_url, "http://lab.example:8090/v1");
        assert_eq!(checked.think_mode, "auto");
        assert_eq!(checked.timeout_secs, 600);
        assert!(checked.stored_keys.is_empty());

        let mut hosted = form("typesafe");
        hosted.base_url.clear();
        hosted.timeout_secs = 0;
        let hosted = normalize(hosted).unwrap();
        assert_eq!(hosted.base_url, "https://api.typesafe.ai");
        assert_eq!(hosted.timeout_secs, 10);

        for broken in [
            DecisionModelSettings {
                preset: "nobody".into(),
                ..form("custom")
            },
            DecisionModelSettings {
                base_url: "ftp://lab.example".into(),
                ..form("custom")
            },
            DecisionModelSettings {
                model: "two words".into(),
                ..form("typesafe")
            },
            DecisionModelSettings {
                think_mode: "sometimes".into(),
                ..form("typesafe")
            },
        ] {
            assert!(normalize(broken).is_err());
        }
    }

    #[tokio::test]
    async fn it_is_in_use_only_switched_on_and_with_the_key_its_service_needs() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let secrets = MemorySecrets::default();
        // Before anything is saved: the first service, off, and no key id
        // is needed to say so.
        let start = load(&store, &secrets, None).await;
        assert_eq!((start.preset.as_str(), start.enabled), ("typesafe", false));
        assert!(configured(&store, &secrets, None).await.is_none());

        // A hosted service is not switched on without a key.
        let on = DecisionModelSettings {
            enabled: true,
            ..form("typesafe")
        };
        let refused = save(&store, &secrets, "k1", on.clone(), None).await;
        assert_eq!(refused.unwrap_err().to_string(), KEY_REQUIRED);
        assert!(save(&store, &secrets, "k1", on.clone(), Some("two words"))
            .await
            .is_err());
        assert_eq!(load(&store, &secrets, Some("k1")).await, start);

        let saved = save(&store, &secrets, "k1", on.clone(), Some(" sk-made-up "))
            .await
            .unwrap();
        assert!(saved.enabled && saved.has_key("typesafe"));
        let model = configured(&store, &secrets, Some("k1")).await.unwrap();
        assert_eq!(model.http.model, "jev-latest");
        assert_eq!(model.http.api_key, "sk-made-up");
        assert_eq!(model.timeout, Duration::from_secs(10));
        // The key is in the secret store only.
        let kept = store.get_setting(SETTINGS_KEY).await.unwrap().unwrap();
        assert!(!kept.contains("sk-made-up") && !kept.contains("stored_keys\":[\""));
        // Another data directory's key id does not see it.
        assert!(configured(&store, &secrets, Some("k2")).await.is_none());

        // Saving again without typing a key keeps the stored one.
        let again = save(&store, &secrets, "k1", on, None).await.unwrap();
        assert!(again.has_key("typesafe"));

        // A self-hosted server needs none; the hosted service's key stays.
        let local = DecisionModelSettings {
            enabled: true,
            think_mode: "never".into(),
            ..form("startlux_local")
        };
        let local = save(&store, &secrets, "k1", local, None).await.unwrap();
        assert_eq!(local.stored_keys, ["typesafe"]);
        let model = configured(&store, &secrets, Some("k1")).await.unwrap();
        assert_eq!(model.http.api_key, "");
        assert_eq!(model.think_mode, "never");

        // Forgetting the key leaves the hosted service on paper but not in use.
        forget_key(&secrets, "k1", "typesafe").unwrap();
        assert!(forget_key(&secrets, "k1", "nobody").is_err());
        let hosted = DecisionModelSettings {
            enabled: false,
            ..form("typesafe")
        };
        let hosted = save(&store, &secrets, "k1", hosted, None).await.unwrap();
        assert!(hosted.stored_keys.is_empty());
        assert!(configured(&store, &secrets, Some("k1")).await.is_none());

        // A test of a hosted service without a key is refused before any
        // request is made.
        let untested = test(&store, &secrets, Some("k1"), form("typesafe"), None).await;
        assert_eq!(untested.unwrap_err().to_string(), KEY_REQUIRED);
    }

    #[tokio::test]
    async fn a_form_from_another_desktop_is_taken_with_the_keys_it_could_bring() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let secrets = MemorySecrets::default();
        // A form a save would refuse is not taken.
        let broken = DecisionModelSettings {
            base_url: "ftp://lab.example".into(),
            ..form("custom")
        };
        assert!(adopt(&store, &secrets, "k1", broken, &[]).await.is_err());
        assert_eq!(store.get_setting(SETTINGS_KEY).await.unwrap(), None);

        // Switched on without the key its service needs: taken as it is,
        // and not in use until the key is entered here. A key a form
        // would refuse, or of no service, is not kept.
        let on = DecisionModelSettings {
            enabled: true,
            ..form("typesafe")
        };
        let keys = [
            ("typesafe".to_owned(), "two words".to_owned()),
            ("startlux_bamboo".to_owned(), " sk-made-up ".to_owned()),
            ("nobody".to_owned(), "sk-other".to_owned()),
        ];
        let kept = adopt(&store, &secrets, "k1", on, &keys).await.unwrap();
        assert_eq!(kept, ["startlux_bamboo"]);
        let taken = load(&store, &secrets, Some("k1")).await;
        assert!(taken.enabled && !taken.has_key("typesafe"));
        assert_eq!(taken.stored_keys, ["startlux_bamboo"]);
        assert!(configured(&store, &secrets, Some("k1")).await.is_none());
        assert_eq!(
            secrets.get(&secret("k1", "startlux_bamboo")).as_deref(),
            Some("sk-made-up")
        );
    }
}
