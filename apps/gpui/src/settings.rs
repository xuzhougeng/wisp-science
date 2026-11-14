//! Model settings wording and form rules, apart from GPUI so they are covered
//! without a window. Validation itself belongs to the host.
use wisp_server::{
    config::default_api_url,
    desktop::{ModelSettings, ModelSource},
};

/// Providers the embedded host can run, with the label shown for each.
pub const PROVIDERS: [(&str, &str); 3] = [
    ("openai", "OpenAI-compatible"),
    ("openai_responses", "OpenAI Responses"),
    ("anthropic", "Anthropic"),
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

/// Whether the form can be edited. Environment variables take precedence, so
/// editing a form that cannot take effect would mislead.
pub fn editable(source: ModelSource) -> bool {
    source != ModelSource::Environment
}

/// One line saying which model new turns will use and where it comes from.
pub fn summary(settings: &ModelSettings) -> String {
    match settings.source {
        ModelSource::Environment => format!(
            "Using {} from WISP_* environment variables. Unset them to edit the model here.",
            settings.fields.model
        ),
        ModelSource::Saved => format!("Using {}.", settings.fields.model),
        ModelSource::None => match &settings.problem {
            Some(problem) => format!("Saved settings are not in use: {problem}"),
            None => {
                "No model is configured yet. Messages cannot be sent until one is saved.".into()
            }
        },
    }
}

pub fn key_placeholder(has_api_key: bool) -> &'static str {
    if has_api_key {
        "Saved in the system keychain; leave blank to keep it"
    } else {
        "API key (kept in the system keychain)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::ModelFields;

    fn settings(source: ModelSource, problem: Option<&str>) -> ModelSettings {
        ModelSettings {
            fields: ModelFields {
                model: "lab-model".into(),
                ..Default::default()
            },
            has_api_key: false,
            source,
            problem: problem.map(str::to_owned),
        }
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
    }

    #[test]
    fn summary_states_the_source_and_any_problem() {
        assert!(summary(&settings(ModelSource::Saved, None)).contains("lab-model"));
        let env = summary(&settings(ModelSource::Environment, None));
        assert!(env.contains("lab-model") && env.contains("WISP_"));
        assert!(summary(&settings(ModelSource::None, None)).contains("No model is configured"));
        assert!(
            summary(&settings(
                ModelSource::None,
                Some("The saved API key is missing")
            ))
            .contains("API key is missing")
        );
        assert!(editable(ModelSource::None) && editable(ModelSource::Saved));
        assert!(!editable(ModelSource::Environment));
        assert_ne!(key_placeholder(true), key_placeholder(false));
    }
}
