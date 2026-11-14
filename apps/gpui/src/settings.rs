//! Model settings wording and form rules, apart from GPUI so they are covered
//! without a window. Validation itself belongs to the host.
use wisp_server::{
    config::default_api_url,
    desktop::{ModelProfile, ModelSettings, ModelSource},
};

/// Providers the embedded host can run, with the label shown for each.
pub const PROVIDERS: [(&str, &str); 3] = [
    ("openai", "OpenAI-compatible"),
    ("openai_responses", "OpenAI Responses"),
    ("anthropic", "Anthropic"),
];

/// One-click starting points for a new model: label, endpoint and a suggested
/// model ID. The list is the WebView's. The "Coding" entries are the monthly
/// coding-plan endpoints; those keys only work there.
pub const PRESETS: [(&str, &str, &str); 6] = [
    ("OpenCode", "https://opencode.ai/zen/go/v1", ""),
    ("Kimi", "https://api.moonshot.cn/v1", "kimi-k3"),
    ("GLM", "https://open.bigmodel.cn/api/paas/v4", "glm-5"),
    ("DeepSeek", "https://api.deepseek.com", "deepseek-v4-flash"),
    (
        "Kimi Coding",
        "https://api.kimi.com/coding/v1",
        "kimi-coding",
    ),
    (
        "GLM Coding",
        "https://open.bigmodel.cn/api/coding/paas/v4",
        "glm-5.2",
    ),
];

/// Endpoint to show after the provider changes. A URL the user typed is kept;
/// a blank field or the previous provider's default follows the new provider.
pub fn next_api_url(current: &str, previous: &str, next: &str) -> String {
    let current = current.trim();
    if current.is_empty() || current == default_api_url(previous) {
        default_api_url(next).into()
    } else {
        current.into()
    }
}

/// Whether models can be edited. Environment variables take precedence, so
/// editing a form that cannot take effect would mislead.
pub fn editable(source: ModelSource) -> bool {
    source != ModelSource::Environment
}

pub fn default_profile(settings: &ModelSettings) -> Option<&ModelProfile> {
    settings.profiles.iter().find(|profile| profile.is_default)
}

/// One line saying which model new conversations use and where it comes from.
pub fn summary(settings: &ModelSettings) -> String {
    match (settings.source, default_profile(settings)) {
        (ModelSource::Environment, Some(profile)) => format!(
            "Using {} from WISP_* environment variables. Unset them to edit models here.",
            profile.fields.model
        ),
        (_, Some(profile)) => match &profile.problem {
            Some(problem) => format!("{} cannot be used: {problem}", profile.name),
            None => format!("New conversations use {}.", profile.name),
        },
        (_, None) => {
            "No model is configured yet. Messages cannot be sent until one is saved.".into()
        }
    }
}

pub fn key_placeholder(has_api_key: bool) -> &'static str {
    if has_api_key {
        "Saved in the system keychain; leave blank to keep it"
    } else {
        "API key (kept in the system keychain)"
    }
}

/// The profile the form should show: the one asked for if it still exists,
/// otherwise the default, otherwise nothing (a blank form for a new model).
pub fn selection_after(settings: &ModelSettings, wanted: Option<&str>) -> Option<String> {
    wanted
        .and_then(|id| settings.profiles.iter().find(|profile| profile.id == id))
        .or_else(|| default_profile(settings))
        .or_else(|| settings.profiles.first())
        .map(|profile| profile.id.clone())
}

/// The profile a save just added: the only id that was not there before.
pub fn added(before: Option<&ModelSettings>, after: &ModelSettings) -> Option<String> {
    after
        .profiles
        .iter()
        .find(|profile| {
            before.is_none_or(|before| before.profiles.iter().all(|old| old.id != profile.id))
        })
        .map(|profile| profile.id.clone())
}

/// What the composer's model button says for a conversation that chose
/// `choice` (empty follows the default). A choice that no longer exists
/// reads as the default, which is what the host will run.
pub fn model_label(settings: Option<&ModelSettings>, choice: &str) -> String {
    let Some(settings) = settings else {
        return "Model".into();
    };
    settings
        .profiles
        .iter()
        .find(|profile| !choice.is_empty() && profile.id == choice)
        .or_else(|| default_profile(settings))
        .map_or_else(|| "No model".into(), |profile| profile.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::ModelFields;

    fn profile(id: &str, is_default: bool, problem: Option<&str>) -> ModelProfile {
        ModelProfile {
            id: id.into(),
            name: format!("Name {id}"),
            fields: ModelFields {
                model: format!("model-{id}"),
                ..Default::default()
            },
            has_api_key: true,
            is_default,
            problem: problem.map(str::to_owned),
        }
    }

    fn settings(source: ModelSource, profiles: Vec<ModelProfile>) -> ModelSettings {
        ModelSettings { profiles, source }
    }

    #[test]
    fn switching_provider_follows_defaults_but_keeps_a_typed_endpoint() {
        let openai = default_api_url("openai");
        let anthropic = default_api_url("anthropic");
        assert_ne!(openai, anthropic);
        assert_eq!(next_api_url("", "openai", "anthropic"), anthropic);
        assert_eq!(next_api_url(openai, "openai", "anthropic"), anthropic);
        assert_eq!(
            next_api_url(anthropic, "anthropic", "openai_responses"),
            openai
        );
        assert_eq!(
            next_api_url(" http://localhost:11434/v1 ", "openai", "anthropic"),
            "http://localhost:11434/v1"
        );
        // Every listed provider is one the host accepts by name.
        for (id, label) in PROVIDERS {
            assert!(!label.is_empty() && !default_api_url(id).is_empty());
        }
        for (label, url, _) in PRESETS {
            assert!(!label.is_empty() && url.starts_with("https://"));
        }
    }

    #[test]
    fn summary_states_the_source_and_any_problem() {
        let saved = settings(
            ModelSource::Saved,
            vec![profile("a", false, None), profile("b", true, None)],
        );
        assert_eq!(summary(&saved), "New conversations use Name b.");
        let env = summary(&settings(
            ModelSource::Environment,
            vec![profile("env", true, None)],
        ));
        assert!(env.contains("model-env") && env.contains("WISP_"));
        assert!(summary(&settings(ModelSource::None, vec![])).contains("No model is configured"));
        let broken = settings(
            ModelSource::None,
            vec![profile("a", true, Some("The saved API key is missing"))],
        );
        assert_eq!(
            summary(&broken),
            "Name a cannot be used: The saved API key is missing"
        );
        assert!(editable(ModelSource::None) && editable(ModelSource::Saved));
        assert!(!editable(ModelSource::Environment));
        assert_ne!(key_placeholder(true), key_placeholder(false));
    }

    #[test]
    fn the_form_follows_the_list_as_it_changes() {
        let two = settings(
            ModelSource::Saved,
            vec![profile("a", false, None), profile("b", true, None)],
        );
        assert_eq!(selection_after(&two, Some("a")).as_deref(), Some("a"));
        // A profile that was deleted falls back to the default one.
        assert_eq!(selection_after(&two, Some("gone")).as_deref(), Some("b"));
        assert_eq!(selection_after(&two, None).as_deref(), Some("b"));
        assert_eq!(
            selection_after(&settings(ModelSource::None, vec![]), None),
            None
        );

        let one = settings(ModelSource::Saved, vec![profile("b", true, None)]);
        assert_eq!(added(Some(&one), &two).as_deref(), Some("a"));
        assert_eq!(added(None, &one).as_deref(), Some("b"));
        assert_eq!(added(Some(&two), &two), None);
    }

    #[test]
    fn the_composer_names_the_model_the_conversation_will_run_on() {
        let two = settings(
            ModelSource::Saved,
            vec![profile("a", false, None), profile("b", true, None)],
        );
        assert_eq!(model_label(Some(&two), ""), "Name b");
        assert_eq!(model_label(Some(&two), "a"), "Name a");
        // A deleted choice runs on the default, so that is what it says.
        assert_eq!(model_label(Some(&two), "gone"), "Name b");
        assert_eq!(
            model_label(Some(&settings(ModelSource::None, vec![])), ""),
            "No model"
        );
        assert_eq!(model_label(None, ""), "Model");
    }
}
