//! Model settings wording and form rules, apart from GPUI so they are covered
//! without a window. Validation itself belongs to the host.
use crate::i18n::tr;
use wisp_dto::NetworkSettings;
use wisp_server::{
    config::default_api_url,
    desktop::{
        Capabilities, ConnectorRow, EnvironmentCheck, McpTransport, ModelProfile, ModelSettings,
        ModelSource, ProbeResult, RUNS_LOCALLY, SkillRow, SshHostRow,
    },
};

/// The pages of Settings, in the order they are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Models,
    /// Programs on this computer that speak the Agent Client Protocol.
    AcpAgents,
    Conversation,
    /// Personas a conversation can be started as.
    Experts,
    /// Requests kept under a name.
    QuickActions,
    /// Goals broken into tasks, run as one conversation.
    Workflows,
    /// The fast endpoint that answers typed questions.
    DecisionModel,
    Skills,
    /// Packages of skills, switched on per project.
    Plugins,
    /// The bridge to the user's own browser.
    Browser,
    Connections,
    Credentials,
    Environments,
    Permissions,
    Usage,
    Storage,
    General,
    Network,
    /// Opening this desktop in a browser through a relay.
    Remote,
    /// Chat apps that talk to Wisp (WeChat, Feishu) and the Wisp Lab.
    Channels,
    /// The animated companion.
    Pet,
}

impl Section {
    pub const ALL: [Section; 21] = [
        Section::General,
        Section::Network,
        Section::Remote,
        Section::Channels,
        Section::Pet,
        Section::Models,
        Section::AcpAgents,
        Section::DecisionModel,
        Section::Conversation,
        Section::Experts,
        Section::QuickActions,
        Section::Workflows,
        Section::Skills,
        Section::Plugins,
        Section::Browser,
        Section::Connections,
        Section::Credentials,
        Section::Environments,
        Section::Permissions,
        Section::Usage,
        Section::Storage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => tr("General"),
            Section::Network => tr("Network"),
            Section::Models => tr("Models"),
            Section::AcpAgents => tr("ACP agents"),
            Section::Conversation => tr("Conversation"),
            Section::Experts => tr("Experts"),
            Section::QuickActions => tr("Quick actions"),
            Section::Workflows => tr("Workflows"),
            Section::Pet => tr("Pet"),
            Section::Remote => tr("Remote access"),
            Section::Channels => tr("Chat apps"),
            Section::DecisionModel => tr("Decision model"),
            Section::Skills => tr("Skills"),
            Section::Plugins => tr("Plugins"),
            Section::Browser => tr("Browser"),
            Section::Connections => tr("Connections"),
            Section::Credentials => tr("Credentials"),
            Section::Environments => tr("Environments"),
            Section::Permissions => tr("Permissions"),
            Section::Usage => tr("Usage"),
            Section::Storage => tr("Storage"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Section::General => "controls",
            Section::Network => "share",
            Section::Models => "bot",
            Section::AcpAgents => "terminal",
            Section::Conversation => "chat",
            Section::Experts => "user",
            Section::QuickActions => "bolt",
            Section::Workflows => "split",
            Section::Pet => "sparkles",
            Section::Remote => "monitor",
            Section::Channels => "bubble",
            Section::DecisionModel => "check-square",
            Section::Skills => "skill",
            Section::Plugins => "grid",
            Section::Browser => "compass",
            Section::Connections => "link",
            Section::Credentials => "lock",
            Section::Environments => "server",
            Section::Permissions => "shield",
            Section::Usage => "gauge",
            Section::Storage => "database",
        }
    }

    /// What the page is about, in the words someone would look for it by.
    /// The page's own name is searched as well.
    fn keywords(self) -> &'static str {
        match self {
            Section::General => {
                "appearance theme dark light system start launch open home language \
                 chinese english 中文 语言 notify notification sound banner alert update version \
                 release upgrade"
            }
            Section::Network => {
                "proxy route socks http direct mirror conda pip pypi index package \
                 certificate ca bundle"
            }
            Section::Models => {
                "model api key provider endpoint url context tokens reasoning effort default \
                 openai anthropic test connection"
            }
            Section::Conversation => {
                "steps iterations turn limit compact compaction context follow-up questions \
                 suggestions suggest next review reviewer automatic check sub-agent subagent \
                 helper explore worker"
            }
            Section::Experts => "expert persona specialist role instructions illustrator 专家",
            Section::AcpAgents => {
                "acp agent client protocol external program coding claude code codex gemini \
                 adapter command 智能体"
            }
            Section::DecisionModel => {
                "decision model decide map_items typesafe jev startlux bamboo systemone yes no \
                 score choice 决策模型"
            }
            Section::Remote => "remote access web browser phone relay tunnel code link 远程接入",
            Section::Channels => {
                "chat app wechat weixin ilink feishu lark bot im message phone qr scan bind owner \
                 wisp lab group member invite relay mail 微信 飞书 课题组 消息 渠道"
            }
            Section::Pet => "pet companion mascot animation sprite 宠物",
            Section::Workflows => "workflow template task stage dependency run 工作流",
            Section::QuickActions => "quick action prompt template shortcut reuse 快捷动作",
            Section::Skills => "skill workflow bundled folder reload",
            Section::Browser => {
                "browser chrome chromium extension web tab block prefer host 浏览器"
            }
            Section::Plugins => "plugin package install extension claude 插件",
            Section::Connections => {
                "mcp server connector tool literature biology pubmed uniprot token http"
            }
            Section::Credentials => "key keychain secret password ncbi openalex email service",
            Section::Environments => {
                "ssh host server cluster gpu remote compute run check probe scheduler slurm \
                 config default"
            }
            Section::Permissions => "approval ask full access allow always tool grant safety",
            Section::Usage => "tokens cost spend usage chart day model project",
            Section::Storage => "disk space data folder database size files location",
        }
    }

    /// The heading the section is listed under.
    pub fn group(self) -> &'static str {
        match self {
            Section::General
            | Section::Network
            | Section::Remote
            | Section::Channels
            | Section::Pet => tr("App"),
            Section::Models
            | Section::AcpAgents
            | Section::DecisionModel
            | Section::Conversation
            | Section::Experts
            | Section::QuickActions
            | Section::Workflows => "AI",
            Section::Skills
            | Section::Plugins
            | Section::Browser
            | Section::Connections
            | Section::Credentials
            | Section::Environments => tr("Capabilities"),
            Section::Permissions => tr("Safety"),
            Section::Usage | Section::Storage => tr("Data"),
        }
    }
}

/// Where a check for a newer version stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    Checking,
    Found(wisp_server::desktop::UpdateCheck),
    Failed(String),
}

/// What the Updates row says, and whether there is a release to open.
/// `at_start` is whether Wisp also looks each time it starts.
pub fn update_note(update: Option<&Update>, at_start: bool) -> (String, bool) {
    match update {
        None if at_start => (
            tr("Wisp looks for a newer version when it starts and when you ask; nothing is downloaded.")
                .to_owned(),
            false,
        ),
        None => (
            tr("Wisp looks for a newer version only when you ask.").to_owned(),
            false,
        ),
        Some(Update::Checking) => (tr("Checking…").to_owned(), false),
        Some(Update::Failed(problem)) => (
            crate::i18n::fill(tr("Could not check: {}"), &[problem]),
            false,
        ),
        Some(Update::Found(found)) if found.newer => (
            crate::i18n::fill(
                tr("Version {} is available; this is {}."),
                &[&found.latest, &found.current],
            ),
            true,
        ),
        Some(Update::Found(found)) => (
            crate::i18n::fill(tr("This is the latest version, {}."), &[&found.current]),
            false,
        ),
    }
}

/// The composer's expert button: the name of the expert the conversation
/// is started as, or the word for choosing one.
pub fn expert_label(experts: &[wisp_server::desktop::Expert], chosen: &str) -> String {
    experts
        .iter()
        .find(|expert| expert.id == chosen)
        .map_or_else(|| tr("Expert").to_owned(), |expert| expert.name.clone())
}

/// The pages that have to do with what was typed, in the order they are
/// listed. Every word has to be found, in the page's name or in what it is
/// about; a word may be the beginning of one. Nothing typed lists them all.
pub fn search_sections(query: &str) -> Vec<Section> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    Section::ALL
        .into_iter()
        .filter(|section| {
            let label = section.label().to_lowercase();
            words.iter().all(|word| {
                label.contains(word)
                    || section
                        .keywords()
                        .split_whitespace()
                        .any(|keyword| keyword.starts_with(word))
            })
        })
        .collect()
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
        "" => tr("Follows the system").into(),
        "none" => tr("Connects directly").into(),
        proxy => crate::i18n::fill(tr("Through {}"), &[&proxy]),
    }
}

/// Where a skill comes from, in words.
pub fn skill_source(source: &str) -> &'static str {
    match source {
        "bundled" => tr("Ships with Wisp"),
        "project" => tr("This project"),
        _ => tr("Added folder"),
    }
}

/// One line saying how many skills there are and how many are off.
pub fn skills_summary(rows: &[SkillRow]) -> String {
    let off = rows.iter().filter(|row| !row.enabled).count();
    let total = match rows.len() {
        0 => return tr("No skills were found.").into(),
        1 => tr("1 skill").to_string(),
        n => crate::i18n::fill(tr("{} skills"), &[&n]),
    };
    match off {
        0 => crate::i18n::fill(tr("{}, all on."), &[&total]),
        n => crate::i18n::fill(tr("{}, {} off."), &[&total, &n]),
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

/// Tags as typed, comma-separated: trimmed, without empty ones.
pub fn parse_tags(text: &str) -> Vec<String> {
    text.split([',', '，'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
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
        0 => tr("all on").to_string(),
        n => crate::i18n::fill(tr("{} off"), &[&n]),
    };
    let count = match rows.len() {
        1 => tr("1 connector").to_string(),
        n => crate::i18n::fill(tr("{} connectors"), &[&n]),
    };
    crate::i18n::fill(
        tr("{}, {}; {} for the model."),
        &[&count, &state, &tool_count(tools)],
    )
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
        1 => tr("1 tool").into(),
        n => crate::i18n::fill(tr("{} tools"), &[&n]),
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
        return tr("From the entry of that name in your SSH config").into();
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
    if host.password_sign_in {
        line.push_str(if host.has_password {
            tr(" · password")
        } else {
            tr(" · password not saved")
        });
    } else if !host.identity_file.is_empty() {
        line.push_str(&crate::i18n::fill(tr(" · key {}"), &[&host.identity_file]));
    }
    if !host.proxy_jump.is_empty() {
        line.push_str(&crate::i18n::fill(tr(" · via {}"), &[&host.proxy_jump]));
    }
    line
}

/// The lines of the capabilities overview: what it is, how much of it
/// there is, and the Settings page that manages it, if one does.
pub fn capability_lines(found: &Capabilities) -> Vec<(&'static str, String, Option<Section>)> {
    let of = |(on, total): (usize, usize), none: &str| match total {
        0 => none.to_owned(),
        _ if on == total => crate::i18n::fill(tr("{}, all on"), &[&total]),
        _ => crate::i18n::fill(tr("{} on of {}"), &[&on, &total]),
    };
    vec![
        (
            tr("Skills"),
            of(found.skills, tr("None found")),
            Some(Section::Skills),
        ),
        (
            tr("Connectors"),
            of(found.connectors, tr("None")),
            Some(Section::Connections),
        ),
        (
            tr("MCP servers"),
            of(found.mcp_servers, tr("None added")),
            Some(Section::Connections),
        ),
        (
            tr("SSH hosts"),
            match found.hosts {
                (_, 0) => tr("None added").to_owned(),
                (ready, total) if ready == total => {
                    crate::i18n::fill(tr("{}, all checked"), &[&total])
                }
                (ready, total) => crate::i18n::fill(tr("{} checked of {}"), &[&ready, &total]),
            },
            Some(Section::Environments),
        ),
        (
            tr("Memory"),
            if found.memory_on {
                crate::i18n::fill(
                    tr("On · {} · {}"),
                    &[
                        &match found.memory_notes {
                            1 => tr("1 day of notes").to_owned(),
                            n => crate::i18n::fill(tr("{} days of notes"), &[&n]),
                        },
                        &crate::memory::habits(found.habits),
                    ],
                )
            } else {
                tr("Off").to_owned()
            },
            None,
        ),
    ]
}

/// What the composer says about where a conversation's work runs.
/// `choice` is the conversation's own (empty follows `default`, the host
/// Settings names, if any).
pub fn runs_on_label(choice: &str, default: Option<&str>) -> String {
    match (choice, default) {
        ("", None) | (RUNS_LOCALLY, _) => tr("This computer").into(),
        ("", Some(host)) => host.to_owned(),
        (host, _) => host.to_owned(),
    }
}

/// Why work cannot run on a host yet, if it cannot: a host has to pass a
/// check first.
pub fn host_caution(check: Option<&EnvironmentCheck>) -> Option<&'static str> {
    match check {
        None => Some(tr("Not checked yet; check it in Settings")),
        Some(check) if check.error.is_some() => Some(tr("Last check failed; see Settings")),
        Some(_) => None,
    }
}

/// What the last check of a host found, as labelled facts in the order
/// they matter when choosing where to run something.
pub fn check_facts(found: &ProbeResult) -> Vec<(&'static str, String)> {
    let known = |value: &Option<String>| value.clone().filter(|text| !text.trim().is_empty());
    let mut facts = Vec::new();
    let system: Vec<String> = [&found.hostname, &found.os, &found.arch]
        .into_iter()
        .filter_map(known)
        .collect();
    if !system.is_empty() {
        // Not the "System" of the language and appearance choices.
        facts.push(("System", system.join(" · ")));
    }
    if let Some(count) = found.cpu_count {
        facts.push((tr("Processors"), count.to_string()));
    }
    // The model is told not to plan GPU work without one, so say so here.
    facts.push((
        "GPU",
        known(&found.gpu_summary).unwrap_or_else(|| tr("None found").into()),
    ));
    if let Some(scheduler) = known(&found.scheduler) {
        facts.push((tr("Scheduler"), scheduler));
    }
    let interpreter =
        |version: &Option<String>, path: &Option<String>| match (known(version), known(path)) {
            (Some(version), Some(path)) => Some(format!("{version} · {path}")),
            (version, path) => version.or(path),
        };
    if let Some(python) = interpreter(&found.python_version, &found.python_executable) {
        facts.push(("Python", python));
    }
    if let Some(r) = interpreter(&found.r_version, &found.rscript_executable) {
        facts.push(("R", r));
    }
    facts
}

/// Whether work can run on a host, on one line. `checking` is true while a
/// check of this host is under way.
pub fn check_line(check: Option<&EnvironmentCheck>, checking: bool, now: i64) -> String {
    if checking {
        return tr("Checking…").into();
    }
    match check {
        None => tr("Not checked yet. Work runs here only after a check passes.").into(),
        Some(check) => {
            let when = crate::home::age(now, check.at);
            match &check.error {
                None => crate::i18n::fill(tr("Reachable · checked {}"), &[&when]),
                Some(_) => crate::i18n::fill(tr("Could not be used · checked {}"), &[&when]),
            }
        }
    }
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
        _ => Err(tr("A port is a number from 1 to 65535.").into()),
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
            Launch::Home => tr("Home page"),
            Launch::Resume => tr("Where I left off"),
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

/// Which key sends what is written in the composer. The other one starts
/// a new line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SendKey {
    /// Return sends; Shift+Return starts a new line. The WebView's default.
    #[default]
    Enter,
    /// Cmd+Return (Ctrl+Enter off macOS) sends; Return starts a new line.
    Modifier,
}

impl SendKey {
    pub const ALL: [SendKey; 2] = [SendKey::Enter, SendKey::Modifier];

    pub fn label(self) -> &'static str {
        match (self, cfg!(target_os = "macos")) {
            (SendKey::Enter, true) => tr("Return"),
            (SendKey::Enter, false) => tr("Enter"),
            (SendKey::Modifier, true) => tr("⌘Return"),
            (SendKey::Modifier, false) => tr("Ctrl+Enter"),
        }
    }

    /// The words the WebView's own choice is saved as.
    pub fn key(self) -> &'static str {
        match self {
            SendKey::Enter => "enter",
            SendKey::Modifier => "modifier_enter",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key.trim())
            .unwrap_or_default()
    }

    /// The composer's key context. Return sends only inside
    /// [`SEND_ON_ENTER`]; anywhere else it is the input's own new line.
    pub fn context(self) -> &'static str {
        match self {
            SendKey::Enter => SEND_ON_ENTER,
            SendKey::Modifier => "Composer",
        }
    }

    /// What the empty composer says, so the key is never a guess.
    pub fn placeholder(self) -> &'static str {
        match (self, cfg!(target_os = "macos")) {
            (SendKey::Enter, true) => {
                tr("Describe your research task…  Return to send, Shift+Return for a new line")
            }
            (SendKey::Enter, false) => {
                tr("Describe your research task…  Enter to send, Shift+Enter for a new line")
            }
            (SendKey::Modifier, true) => tr("Describe your research task…  ⌘Return to send"),
            (SendKey::Modifier, false) => tr("Describe your research task…  Ctrl+Enter to send"),
        }
    }
}

/// The key context of a composer in which Return sends.
pub const SEND_ON_ENTER: &str = "ComposerEnter";

/// What the send key setting says about itself.
pub fn send_key_note() -> &'static str {
    if cfg!(target_os = "macos") {
        tr(
            "The key that sends your message. When Return sends, Shift+Return starts a new line; when ⌘Return sends, Return does. While an input method is composing, Return still goes to it.",
        )
    } else {
        tr(
            "The key that sends your message. When Enter sends, Shift+Enter starts a new line; when Ctrl+Enter sends, Enter does. While an input method is composing, Enter still goes to it.",
        )
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
        (true, _) => tr("Saved. Type a new value to replace it"),
        (false, true) => tr("Paste the key"),
        (false, false) => tr("Not set"),
    }
}

/// Reads the steps-per-turn field. The host has the last word; this keeps
/// an obviously wrong entry from being sent.
pub fn steps(text: &str, range: std::ops::RangeInclusive<usize>) -> Result<usize, String> {
    let wrong = || {
        crate::i18n::fill(
            tr("Enter a whole number from {} to {}."),
            &[range.start(), range.end()],
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

/// How many times a turn may carry on by itself after one step down or
/// up, within the host's range.
pub fn step_continues(current: usize, up: bool) -> usize {
    let range = wisp_server::desktop::AUTO_CONTINUE_RANGE;
    let next = if up {
        current.saturating_add(1)
    } else {
        current.saturating_sub(1)
    };
    next.clamp(*range.start(), *range.end())
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
        (ModelSource::Environment, Some(profile)) => crate::i18n::fill(
            tr("Using {} from WISP_* environment variables. Unset them to edit models here."),
            &[&profile.fields.model],
        ),
        (_, Some(profile)) => match &profile.problem {
            Some(problem) => crate::i18n::fill(
                tr("{} cannot be used: {}"),
                &[&profile.name, &crate::i18n::host(problem)],
            ),
            None => crate::i18n::fill(tr("New conversations use {}."), &[&profile.name]),
        },
        (_, None) => {
            tr("No model is configured yet. Messages cannot be sent until one is saved.").into()
        }
    }
}

pub fn key_placeholder(has_api_key: bool) -> &'static str {
    if has_api_key {
        tr("Saved in the system keychain; leave blank to keep it")
    } else {
        tr("API key (kept in the system keychain)")
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
        return tr("Model").into();
    };
    settings
        .profiles
        .iter()
        .find(|profile| !choice.is_empty() && profile.id == choice)
        .or_else(|| default_profile(settings))
        .map_or_else(|| tr("No model").into(), |profile| profile.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_updates_row_says_where_the_check_stands() {
        let found = |newer| {
            Update::Found(wisp_server::desktop::UpdateCheck {
                current: "1.24.0".into(),
                latest: if newer { "1.25.0" } else { "1.24.0" }.into(),
                newer,
                url: "https://github.com/xuzhougeng/wisp-science/releases".into(),
                notes: String::new(),
            })
        };
        assert_eq!(
            update_note(None, false),
            (
                "Wisp looks for a newer version only when you ask.".into(),
                false
            )
        );
        assert!(update_note(None, true).0.contains("when it starts"));
        assert_eq!(
            update_note(Some(&Update::Checking), true),
            ("Checking…".into(), false)
        );
        assert_eq!(
            update_note(Some(&found(true)), true),
            ("Version 1.25.0 is available; this is 1.24.0.".into(), true)
        );
        assert_eq!(
            update_note(Some(&found(false)), true),
            ("This is the latest version, 1.24.0.".into(), false)
        );
        assert_eq!(
            update_note(Some(&Update::Failed("no network".into())), true),
            ("Could not check: no network".into(), false)
        );
    }

    #[test]
    fn the_expert_button_names_the_expert_or_offers_to_choose_one() {
        let experts = [wisp_server::desktop::Expert {
            id: "sp-1".into(),
            name: "Statistician".into(),
            ..Default::default()
        }];
        assert_eq!(expert_label(&experts, "sp-1"), "Statistician");
        assert_eq!(expert_label(&experts, ""), "Expert");
        // One that was removed since reads as none.
        assert_eq!(expert_label(&experts, "sp-gone"), "Expert");
        // It has a page of its own in Settings, found by what it is about.
        assert!(search_sections("persona").contains(&Section::Experts));
    }

    #[test]
    fn sections_are_listed_under_their_groups_in_order() {
        let groups: Vec<_> = Section::ALL.iter().map(|section| section.group()).collect();
        assert_eq!(
            groups,
            [
                "App",
                "App",
                "App",
                "App",
                "App",
                "AI",
                "AI",
                "AI",
                "AI",
                "AI",
                "AI",
                "AI",
                "Capabilities",
                "Capabilities",
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
        assert_eq!(
            parse_tags(" rna, , single cell，qc "),
            ["rna", "single cell", "qc"]
        );
        assert!(parse_tags(" , ").is_empty());
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
        // A host signing in by password says so, and whether it is saved.
        host.password_sign_in = true;
        assert_eq!(
            ssh_target(&host),
            "lab@10.0.0.8:2222 · password not saved · via bastion"
        );
        host.has_password = true;
        assert_eq!(
            ssh_target(&host),
            "lab@10.0.0.8:2222 · password · via bastion"
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
    fn the_send_key_is_saved_in_the_webviews_words_and_return_is_the_default() {
        assert_eq!(SendKey::default(), SendKey::Enter);
        for choice in SendKey::ALL {
            assert_eq!(SendKey::from_key(choice.key()), choice);
        }
        assert_eq!(SendKey::from_key("modifier_enter"), SendKey::Modifier);
        assert_eq!(SendKey::from_key(""), SendKey::Enter);
        // Only the Return choice puts the composer where Return sends.
        assert_eq!(SendKey::Enter.context(), SEND_ON_ENTER);
        assert_ne!(SendKey::Modifier.context(), SEND_ON_ENTER);
        // The empty composer names the key that sends.
        assert!(SendKey::Enter.placeholder().contains("to send, Shift+"));
        assert!(!SendKey::Modifier.placeholder().contains("Shift+"));
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
        assert_eq!(step_continues(10, true), 11);
        assert_eq!(step_continues(1, false), 1);
        assert_eq!(step_continues(100, true), 100);
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

    #[test]
    fn a_check_says_whether_work_can_run_and_what_the_host_is() {
        let now = 1_800_000_000;
        assert!(check_line(None, false, now).starts_with("Not checked yet"));
        assert_eq!(check_line(None, true, now), "Checking…");
        let check = |error: Option<&str>, found: Option<ProbeResult>| EnvironmentCheck {
            alias: "gpu".into(),
            at: now - 120,
            error: error.map(String::from),
            found,
        };
        assert_eq!(
            check_line(Some(&check(None, None)), false, now),
            "Reachable · checked 2 min ago"
        );
        assert_eq!(
            check_line(Some(&check(Some("refused"), None)), false, now),
            "Could not be used · checked 2 min ago"
        );
        // While it is checked again the old verdict is not repeated.
        assert_eq!(check_line(Some(&check(None, None)), true, now), "Checking…");

        let found = ProbeResult {
            os: Some("Linux".into()),
            arch: Some("x86_64".into()),
            hostname: Some("lab-node-7".into()),
            cpu_count: Some(64),
            scheduler: Some("slurm".into()),
            python_executable: Some("/opt/conda/bin/python3".into()),
            python_version: Some("3.11.9".into()),
            rscript_executable: Some("/usr/bin/Rscript".into()),
            gpu_summary: Some("  ".into()),
            ..Default::default()
        };
        assert_eq!(
            check_facts(&found),
            [
                ("System", "lab-node-7 · Linux · x86_64".to_string()),
                ("Processors", "64".to_string()),
                // No GPU is said, not left out: it decides where work goes.
                ("GPU", "None found".to_string()),
                ("Scheduler", "slurm".to_string()),
                ("Python", "3.11.9 · /opt/conda/bin/python3".to_string()),
                ("R", "/usr/bin/Rscript".to_string()),
            ]
        );
        assert_eq!(
            check_facts(&ProbeResult::default()),
            [("GPU", "None found".to_string())]
        );
    }

    #[test]
    fn the_composer_names_where_a_conversation_runs_and_warns_about_unchecked_hosts() {
        // With no choice of its own a conversation follows Settings.
        assert_eq!(runs_on_label("", None), "This computer");
        assert_eq!(runs_on_label("", Some("gpu")), "gpu");
        // Its own choice wins over the default, this computer included.
        assert_eq!(runs_on_label(RUNS_LOCALLY, Some("gpu")), "This computer");
        assert_eq!(runs_on_label("lab", Some("gpu")), "lab");
        assert_eq!(runs_on_label("lab", None), "lab");

        let check = |error: Option<&str>| EnvironmentCheck {
            alias: "gpu".into(),
            at: 1,
            error: error.map(String::from),
            found: None,
        };
        assert!(host_caution(None).unwrap().starts_with("Not checked yet"));
        assert!(
            host_caution(Some(&check(Some("refused"))))
                .unwrap()
                .starts_with("Last check failed")
        );
        assert_eq!(host_caution(Some(&check(None))), None);
    }

    #[test]
    fn settings_are_found_by_what_they_are_about() {
        // Nothing typed, or only spaces, lists every page in order.
        assert_eq!(search_sections(""), Section::ALL);
        assert_eq!(search_sections("   "), Section::ALL);
        // By name, in any case, and by the beginning of a word about it.
        assert_eq!(search_sections("NETW"), [Section::Network]);
        assert_eq!(search_sections("prox"), [Section::Network]);
        assert_eq!(search_sections("ssh"), [Section::Environments]);
        assert_eq!(search_sections("dark"), [Section::General]);
        // A word several pages are about finds them all, in listed order.
        assert_eq!(
            search_sections("key"),
            [Section::Models, Section::Credentials]
        );
        assert_eq!(search_sections("tokens"), [Section::Models, Section::Usage]);
        // Every word has to fit: more words narrow it.
        assert_eq!(search_sections("api key"), [Section::Models]);
        assert_eq!(search_sections("key ncbi"), [Section::Credentials]);
        // The middle of a word is not the word.
        assert!(search_sections("roxy").is_empty());
        assert!(search_sections("nothing like this").is_empty());
        // Every page can be found by at least one word about it.
        for section in Section::ALL {
            let word = section.keywords().split_whitespace().next().unwrap();
            assert!(search_sections(word).contains(&section), "{word}");
        }
    }

    #[test]
    fn the_overview_counts_what_is_on_and_points_at_its_settings() {
        let found = Capabilities {
            skills: (40, 42),
            connectors: (18, 18),
            mcp_servers: (0, 0),
            hosts: (1, 3),
            memory_on: true,
            memory_notes: 1,
            habits: 4,
            tools: vec![],
        };
        let lines = capability_lines(&found);
        let read: Vec<_> = lines
            .iter()
            .map(|(label, value, _)| (*label, value.as_str()))
            .collect();
        assert_eq!(
            read,
            [
                ("Skills", "40 on of 42"),
                ("Connectors", "18, all on"),
                ("MCP servers", "None added"),
                ("SSH hosts", "1 checked of 3"),
                ("Memory", "On · 1 day of notes · 4 habits"),
            ]
        );
        // Each kind that Settings manages leads there; memory is in the panel.
        assert_eq!(lines[0].2, Some(Section::Skills));
        assert_eq!(lines[3].2, Some(Section::Environments));
        assert_eq!(lines[4].2, None);
        let off = Capabilities {
            hosts: (2, 2),
            ..Default::default()
        };
        let lines = capability_lines(&off);
        assert_eq!(lines[3].1, "2, all checked");
        assert_eq!(lines[4].1, "Off");
        assert_eq!(lines[0].1, "None found");
    }
}
