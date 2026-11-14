//! Model settings wording and form rules, apart from GPUI so they are covered
//! without a window. Validation itself belongs to the host.
use wisp_dto::NetworkSettings;
use wisp_server::{
    config::default_api_url,
    desktop::{
        ConnectorRow, McpTransport, ModelProfile, ModelSettings, ModelSource, SkillRow, SshHostRow,
    },
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
    Environments,
    Permissions,
    Usage,
    Storage,
    General,
    Network,
}

impl Section {
    pub const ALL: [Section; 11] = [
        Section::General,
        Section::Network,
        Section::Models,
        Section::Conversation,
        Section::Skills,
        Section::Connections,
        Section::Credentials,
        Section::Environments,
        Section::Permissions,
        Section::Usage,
        Section::Storage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Network => "Network",
            Section::Models => "Models",
            Section::Conversation => "Conversation",
            Section::Skills => "Skills",
            Section::Connections => "Connections",
            Section::Credentials => "Credentials",
            Section::Environments => "Environments",
            Section::Permissions => "Permissions",
            Section::Usage => "Usage",
            Section::Storage => "Storage",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Section::General => "controls",
            Section::Network => "share",
            Section::Models => "bot",
            Section::Conversation => "chat",
            Section::Skills => "skill",
            Section::Connections => "link",
            Section::Credentials => "lock",
            Section::Environments => "server",
            Section::Permissions => "shield",
            Section::Usage => "gauge",
            Section::Storage => "database",
        }
    }

    /// The heading the section is listed under.
    pub fn group(self) -> &'static str {
        match self {
            Section::General | Section::Network => "App",
            Section::Models | Section::Conversation => "AI",
            Section::Skills
            | Section::Connections
            | Section::Credentials
            | Section::Environments => "Capabilities",
            Section::Permissions => "Safety",
            Section::Usage | Section::Storage => "Data",
        }
    }
}

/// How many of the Network page's fields are routes; the rest are about
/// packages.
pub const NETWORK_ROUTES: usize = 3;

/// The Network page's fields, in the order they are shown.
pub fn network_values(saved: &NetworkSettings) -> [String; 6] {
    [
        saved.model_proxy_url.clone(),
        saved.mcp_proxy_url.clone(),
        saved.command_proxy_url.clone(),
        saved.conda_mirror_url.clone(),
        saved.pip_index_url.clone(),
        saved.ca_bundle_path.clone(),
    ]
}

/// What Save sends: the fields as typed, in the order of `network_values`.
/// The page does not show the sign-in route, so that one is kept as it was.
pub fn network_form(typed: [String; 6], saved: &NetworkSettings) -> NetworkSettings {
    let [model, mcp, command, conda, pip, ca] = typed.map(|value| value.trim().to_owned());
    NetworkSettings {
        model_proxy_url: model,
        subscription_proxy_url: saved.subscription_proxy_url.clone(),
        mcp_proxy_url: mcp,
        command_proxy_url: command,
        conda_mirror_url: conda,
        pip_index_url: pip,
        ca_bundle_path: ca,
    }
}

/// What a route's value means, in words.
pub fn route_note(value: &str) -> String {
    match value.trim() {
        "" => "Follows the system".into(),
        "none" => "Connects directly".into(),
        proxy => format!("Through {proxy}"),
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

/// A command line as typed, taken apart into the program and what it is
/// given. Quotes keep words with spaces together; nothing else is special.
pub fn split_command(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    for c in line.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => word.push(c),
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

/// The command line `split_command` reads back the same way.
pub fn join_command(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(|word| {
            if word.is_empty() || word.contains(char::is_whitespace) {
                format!("\"{word}\"")
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a server is reached, on one line.
pub fn transport_line(transport: &McpTransport) -> String {
    match transport {
        McpTransport::Http { url } => url.clone(),
        McpTransport::Stdio { command, args } => join_command(command, args),
    }
}

/// How an SSH host is reached, on one line.
pub fn ssh_target(host: &SshHostRow) -> String {
    if host.host_name.is_empty() {
        return "From the entry of that name in your SSH config".into();
    }
    let mut line = String::new();
    if !host.user.is_empty() {
        line.push_str(&host.user);
        line.push('@');
    }
    line.push_str(&host.host_name);
    if let Some(port) = host.port {
        line.push_str(&format!(":{port}"));
    }
    if !host.identity_file.is_empty() {
        line.push_str(&format!(" · key {}", host.identity_file));
    }
    if !host.proxy_jump.is_empty() {
        line.push_str(&format!(" · via {}", host.proxy_jump));
    }
    line
}

/// Reads the port field: empty leaves it to SSH, anything else must be a
/// port.
pub fn port(text: &str) -> Result<Option<u16>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u16>() {
        Ok(port) if port > 0 => Ok(Some(port)),
        _ => Err("A port is a number from 1 to 65535.".into()),
    }
}

/// What the app opens on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Launch {
    /// The home page with every project.
    #[default]
    Home,
    /// The conversation that was open when the app was last used.
    Resume,
}

impl Launch {
    pub const ALL: [Launch; 2] = [Launch::Home, Launch::Resume];

    pub fn label(self) -> &'static str {
        match self {
            Launch::Home => "Home page",
            Launch::Resume => "Where I left off",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Launch::Home => "home",
            Launch::Resume => "resume",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key.trim())
            .unwrap_or_default()
    }
}

/// The conversation that was open, as it is saved: project, then
/// conversation, on two lines.
pub fn encode_place(project: &str, session: &str) -> String {
    format!("{project}\n{session}")
}

/// Reads a saved place back. Half a place is no place.
pub fn decode_place(saved: &str) -> Option<(String, String)> {
    let (project, session) = saved.split_once('\n')?;
    let (project, session) = (project.trim(), session.trim());
    (!project.is_empty() && !session.is_empty() && !session.contains('\n'))
        .then(|| (project.to_owned(), session.to_owned()))
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
                "App",
                "App",
                "AI",
                "AI",
                "Capabilities",
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
    fn an_ssh_host_is_described_by_what_was_filled_in() {
        let mut host = SshHostRow {
            alias: "gpu".into(),
            ..Default::default()
        };
        assert_eq!(
            ssh_target(&host),
            "From the entry of that name in your SSH config"
        );
        host.host_name = "10.0.0.8".into();
        assert_eq!(ssh_target(&host), "10.0.0.8");
        host.user = "lab".into();
        host.port = Some(2222);
        host.identity_file = "~/.ssh/id_lab".into();
        host.proxy_jump = "bastion".into();
        assert_eq!(
            ssh_target(&host),
            "lab@10.0.0.8:2222 · key ~/.ssh/id_lab · via bastion"
        );
        assert_eq!(port(""), Ok(None));
        assert_eq!(port(" 22 "), Ok(Some(22)));
        for wrong in ["0", "65536", "ssh", "-1"] {
            assert!(port(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn a_command_line_is_split_on_spaces_and_kept_whole_in_quotes() {
        assert_eq!(
            split_command("uvx lab-mcp --port 8000"),
            ["uvx", "lab-mcp", "--port", "8000"]
        );
        assert_eq!(
            split_command("  python  \"my server.py\" --name 'Lab One'  "),
            ["python", "my server.py", "--name", "Lab One"]
        );
        // An empty quoted word is still a word.
        assert_eq!(split_command("run \"\" x"), ["run", "", "x"]);
        assert!(split_command("   ").is_empty());
        for line in [
            "uvx lab-mcp",
            "python \"my server.py\" --flag",
            "run \"\" x",
        ] {
            let words = split_command(line);
            assert_eq!(split_command(&join_command(&words[0], &words[1..])), words);
        }
        assert_eq!(
            transport_line(&McpTransport::Http {
                url: "https://mcp.example/".into()
            }),
            "https://mcp.example/"
        );
    }

    #[test]
    fn the_place_to_resume_survives_a_round_trip_or_is_no_place() {
        let saved = encode_place("lab", "c-1");
        assert_eq!(decode_place(&saved), Some(("lab".into(), "c-1".into())));
        for wrong in ["", "lab", "lab\n", "\nc-1", "lab\nc-1\nextra"] {
            assert_eq!(decode_place(wrong), None, "{wrong:?}");
        }
        for choice in Launch::ALL {
            assert_eq!(Launch::from_key(choice.key()), choice);
        }
        assert_eq!(Launch::from_key("somewhere"), Launch::Home);
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

    #[test]
    fn the_network_form_sends_what_is_typed_and_keeps_what_it_does_not_show() {
        let saved = NetworkSettings {
            model_proxy_url: "http://old.test:1".into(),
            subscription_proxy_url: "socks5://signin.test:1080".into(),
            ..Default::default()
        };
        // The fields are shown in the order they are sent back.
        assert_eq!(network_form(network_values(&saved), &saved), saved);
        let typed = [
            " none ",
            "http://127.0.0.1:7890",
            "",
            "https://conda.test/forge ",
            "",
            " /etc/ca.pem",
        ]
        .map(String::from);
        let form = network_form(typed, &saved);
        assert_eq!(form.model_proxy_url, "none");
        assert_eq!(form.mcp_proxy_url, "http://127.0.0.1:7890");
        assert_eq!(form.command_proxy_url, "");
        assert_eq!(form.conda_mirror_url, "https://conda.test/forge");
        assert_eq!(form.ca_bundle_path, "/etc/ca.pem");
        // The sign-in route has no field here; clearing the page keeps it.
        assert_eq!(form.subscription_proxy_url, "socks5://signin.test:1080");
        // The first fields are the routes, and only those get a reading.
        let values = network_values(&form);
        assert_eq!(
            values[..NETWORK_ROUTES],
            ["none", "http://127.0.0.1:7890", ""]
        );
        assert_eq!(route_note(" "), "Follows the system");
        assert_eq!(route_note("none"), "Connects directly");
        assert_eq!(
            route_note(" socks5://localhost:1080 "),
            "Through socks5://localhost:1080"
        );
    }
}
