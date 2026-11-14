//! Model settings wording and form rules, apart from GPUI so they are covered
//! without a window. Validation itself belongs to the host.
use wisp_server::{
    config::default_api_url,
    desktop::{ConnectorRow, ModelProfile, ModelSettings, ModelSource, SkillRow},
};

/// The pages of Settings, in the order they are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Models,
    Conversation,
    Skills,
    Connections,
    Credentials,
    Permissions,
    Usage,
    Storage,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Models,
        Section::Conversation,
        Section::Skills,
        Section::Connections,
        Section::Credentials,
        Section::Permissions,
        Section::Usage,
        Section::Storage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Models => "Models",
            Section::Conversation => "Conversation",
            Section::Skills => "Skills",
            Section::Connections => "Connections",
            Section::Credentials => "Credentials",
            Section::Permissions => "Permissions",
            Section::Usage => "Usage",
            Section::Storage => "Storage",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Section::Models => "bot",
            Section::Conversation => "chat",
            Section::Skills => "skill",
            Section::Connections => "link",
            Section::Credentials => "lock",
            Section::Permissions => "shield",
            Section::Usage => "gauge",
            Section::Storage => "database",
        }
    }

    /// The heading the section is listed under.
    pub fn group(self) -> &'static str {
        match self {
            Section::Models | Section::Conversation => "AI",
            Section::Skills | Section::Connections | Section::Credentials => "Capabilities",
            Section::Permissions => "Safety",
            Section::Usage | Section::Storage => "Data",
        }
    }
}

/// Where a skill comes from, in words.
pub fn skill_source(source: &str) -> &'static str {
    match source {
        "bundled" => "Ships with Wisp",
        "project" => "This project",
        _ => "Added folder",
    }
}

/// One line saying how many skills there are and how many are off.
pub fn skills_summary(rows: &[SkillRow]) -> String {
    let off = rows.iter().filter(|row| !row.enabled).count();
    let total = match rows.len() {
        0 => return "No skills were found.".into(),
        1 => "1 skill".to_string(),
        n => format!("{n} skills"),
    };
    match off {
        0 => format!("{total}, all on."),
        n => format!("{total}, {n} off."),
    }
}

/// Whether a skill matches what was typed: every word, in its name, its
/// description or one of its tags.
pub fn skill_matches(row: &SkillRow, query: &str) -> bool {
    let text = format!("{} {} {}", row.name, row.description, row.tags.join(" ")).to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// One line saying how many connectors there are and how many are off.
pub fn connectors_summary(rows: &[ConnectorRow]) -> String {
    let tools: usize = rows
        .iter()
        .filter(|row| row.enabled)
        .map(|row| row.tools.len())
        .sum();
    let off = rows.iter().filter(|row| !row.enabled).count();
    let state = match off {
        0 => "all on".to_string(),
        n => format!("{n} off"),
    };
    let count = match rows.len() {
        1 => "1 connector".to_string(),
        n => format!("{n} connectors"),
    };
    format!("{count}, {state}; {} for the model.", tool_count(tools))
}

/// Whether a connector matches what was typed: every word, in its name,
/// its description or the name of one of its tools.
pub fn connector_matches(row: &ConnectorRow, query: &str) -> bool {
    let text = format!("{} {} {}", row.domain, row.description, row.tools.join(" ")).to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// How many tools a connector brings, in words.
pub fn tool_count(count: usize) -> String {
    match count {
        1 => "1 tool".into(),
        n => format!("{n} tools"),
    }
}

/// What a credential's field says before anything is typed.
pub fn credential_placeholder(saved: bool, secret: bool) -> &'static str {
    match (saved, secret) {
        (true, _) => "Saved. Type a new value to replace it",
        (false, true) => "Paste the key",
        (false, false) => "Not set",
    }
}

/// Reads the steps-per-turn field. The host has the last word; this keeps
/// an obviously wrong entry from being sent.
pub fn steps(text: &str, range: std::ops::RangeInclusive<usize>) -> Result<usize, String> {
    let wrong = || {
        format!(
            "Enter a whole number from {} to {}.",
            range.start(),
            range.end()
        )
    };
    let steps: usize = text.trim().parse().map_err(|_| wrong())?;
    if range.contains(&steps) {
        Ok(steps)
    } else {
        Err(wrong())
    }
}

/// The compaction threshold one press up or down from `current`: to the next
/// multiple of five in that direction, kept inside the host's range.
pub fn step_percent(current: u8, up: bool, range: (u8, u8)) -> u8 {
    let next = if up {
        (current / 5 + 1).saturating_mul(5)
    } else {
        current.saturating_sub(1) / 5 * 5
    };
    next.clamp(range.0, range.1)
}

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

    #[test]
    fn sections_are_listed_under_their_groups_in_order() {
        let groups: Vec<_> = Section::ALL.iter().map(|section| section.group()).collect();
        assert_eq!(
            groups,
            [
                "AI",
                "AI",
                "Capabilities",
                "Capabilities",
                "Capabilities",
                "Safety",
                "Data",
                "Data"
            ]
        );
        assert_eq!(Section::default(), Section::Models);
        // No two sections share a name or a drawing.
        for (index, section) in Section::ALL.iter().enumerate() {
            for other in &Section::ALL[index + 1..] {
                assert_ne!(section.label(), other.label());
                assert_ne!(section.icon(), other.icon());
            }
        }
    }

    #[test]
    fn skills_are_counted_described_and_found_by_any_of_their_words() {
        let skill = |name: &str, enabled: bool| SkillRow {
            name: name.into(),
            description: "Quality control for single-cell data".into(),
            tags: vec!["scRNA".into(), "QC".into()],
            source: "bundled",
            enabled,
        };
        assert_eq!(skills_summary(&[]), "No skills were found.");
        assert_eq!(skills_summary(&[skill("a", true)]), "1 skill, all on.");
        assert_eq!(
            skills_summary(&[skill("a", true), skill("b", false), skill("c", false)]),
            "3 skills, 2 off."
        );
        let row = skill("singlecell-qc", true);
        assert!(skill_matches(&row, ""));
        assert!(skill_matches(&row, "QUALITY scrna"));
        assert!(skill_matches(&row, "singlecell"));
        assert!(!skill_matches(&row, "quality proteomics"));
        assert_eq!(skill_source("bundled"), "Ships with Wisp");
        assert_eq!(skill_source("project"), "This project");
        assert_eq!(skill_source("extra"), "Added folder");
    }

    #[test]
    fn connectors_are_counted_with_the_tools_that_are_on() {
        let connector = |domain: &str, tools: &[&str], enabled: bool| ConnectorRow {
            domain: domain.into(),
            description: "Search biomedical literature".into(),
            links: vec![],
            tools: tools.iter().map(|tool| tool.to_string()).collect(),
            enabled,
        };
        let rows = [
            connector("pubmed", &["pubmed_search", "pubmed_fetch"], true),
            connector("uniprot", &["uniprot_lookup"], false),
        ];
        assert_eq!(
            connectors_summary(&rows),
            "2 connectors, 1 off; 2 tools for the model."
        );
        assert_eq!(
            connectors_summary(&rows[..1]),
            "1 connector, all on; 2 tools for the model."
        );
        assert!(connector_matches(&rows[0], "LITERATURE pubmed"));
        // The name of a tool finds the connector that brings it.
        assert!(connector_matches(&rows[1], "uniprot_lookup"));
        assert!(!connector_matches(&rows[0], "uniprot"));
        assert_eq!(tool_count(1), "1 tool");
        assert_eq!(tool_count(4), "4 tools");
    }

    #[test]
    fn a_saved_credential_is_never_shown_only_said_to_be_saved() {
        assert!(credential_placeholder(true, true).starts_with("Saved"));
        assert!(credential_placeholder(true, false).starts_with("Saved"));
        assert_eq!(credential_placeholder(false, true), "Paste the key");
        assert_eq!(credential_placeholder(false, false), "Not set");
    }

    #[test]
    fn steps_must_be_a_whole_number_in_range() {
        assert_eq!(steps(" 200 ", 1..=1000), Ok(200));
        assert_eq!(steps("1", 1..=1000), Ok(1));
        for wrong in ["", "0", "1001", "2.5", "many", "-3"] {
            assert_eq!(
                steps(wrong, 1..=1000),
                Err("Enter a whole number from 1 to 1000.".into()),
                "{wrong}"
            );
        }
    }

    #[test]
    fn the_threshold_moves_in_fives_and_stops_at_the_ends() {
        let range = (10, 95);
        assert_eq!(step_percent(80, true, range), 85);
        assert_eq!(step_percent(80, false, range), 75);
        // An odd saved value snaps to the nearest five in that direction.
        assert_eq!(step_percent(82, true, range), 85);
        assert_eq!(step_percent(82, false, range), 80);
        assert_eq!(step_percent(95, true, range), 95);
        assert_eq!(step_percent(10, false, range), 10);
        assert_eq!(step_percent(12, false, range), 10);
        assert_eq!(step_percent(255, true, range), 95);
    }
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
