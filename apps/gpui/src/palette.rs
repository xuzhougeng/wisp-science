//! The command palette's state: what is listed for a query, in which order,
//! and which row the keyboard is on. It holds no graphics state. Results
//! arrive from the host out of order, so every search carries a ticket and
//! only the latest one may fill the list.
use crate::i18n::tr;
use wisp_dto::{ProjectSummary, SessionSearchInfo};
use wisp_server::desktop::{
    ArtifactRow, FileMatch, ProjectMemo, ReferenceOption, RuntimeLanguage, RuntimeRow, SshHostRow,
    Workflow,
};

use crate::{compose::Chip, home::Screen};

/// Rows shown per section, so one kind of result never buries the others.
const PROJECTS: usize = 5;
const FILES: usize = 6;
const CONVERSATIONS: usize = 8;
const SKILLS: usize = 8;
const ARTIFACTS: usize = 8;
const ENVIRONMENTS: usize = 6;
const RUNTIMES: usize = 4;
const WORKFLOWS: usize = 6;
const MEMOS: usize = 6;

/// What picking a row does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Go there, or run the command.
    #[default]
    Open,
    /// Add the row to the message being written.
    Attach,
    /// Commands only, as the WebView's action palette (⌘P) lists them.
    Commands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewConversation,
    OpenFolder,
    Home,
    Files,
    Runs,
    Artifacts,
    Memos,
    Memory,
    Highlights,
    Favorites,
    Archived,
    Refresh,
    Settings,
    Docs,
    Feedback,
    ProjectSettings,
    Find,
    Journey,
    Papers,
    Schedules,
    Assistant,
    Mandates,
    Calendar,
    NewWindow,
    Capabilities,
    Trajectory,
    Aside,
    SaveAsSkill,
    Review,
    ModelView,
    Terminal,
    PythonConsole,
    RConsole,
    ResearchArchive,
    Notebook,
    Provenance,
    Agents,
    Environments,
    ClosePanel,
    ToggleSidebar,
    ExportProject,
    ImportConversations,
    NewProject,
    ImportProject,
    WebViewImport,
    Skills,
    CheckUpdates,
    StarUs,
    LightTheme,
    DarkTheme,
    SystemTheme,
    LargerText,
    SmallerText,
    LargerCode,
    SmallerCode,
    Setup,
    PrivacyMode,
}

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Command::NewConversation => tr("New conversation"),
            Command::OpenFolder => tr("Open folder…"),
            Command::Home => tr("All projects"),
            Command::Files => tr("Show or hide Files"),
            Command::Runs => tr("Show runs"),
            Command::Artifacts => tr("Show artifacts"),
            Command::Memos => tr("Show memos"),
            Command::Memory => tr("Show memory"),
            Command::Highlights => tr("Show highlights"),
            Command::Favorites => tr("Favorites"),
            Command::Archived => tr("Show or hide archived conversations"),
            Command::Refresh => tr("Refresh"),
            Command::Settings => tr("Settings"),
            Command::Docs => tr("Documentation"),
            Command::Feedback => tr("Report a problem"),
            Command::ProjectSettings => tr("Project settings"),
            Command::Find => tr("Find in this conversation"),
            Command::Journey => tr("Research journey"),
            Command::Papers => tr("Paper evidence"),
            Command::Schedules => tr("Scheduled tasks"),
            Command::Assistant => tr("Research assistant"),
            Command::Mandates => tr("Research mandates"),
            Command::Calendar => tr("Research calendar"),
            Command::NewWindow => tr("New window"),
            Command::Capabilities => tr("Capabilities of this project"),
            Command::Trajectory => tr("Trajectory of this conversation"),
            Command::Aside => tr("Ask aside about this conversation"),
            Command::SaveAsSkill => tr("Save this conversation as a skill"),
            Command::Review => tr("Review this conversation"),
            Command::ModelView => tr("What the model is sent"),
            Command::Terminal => tr("Open a terminal in the project folder"),
            Command::PythonConsole => tr("Python console for this conversation"),
            Command::RConsole => tr("R console for this conversation"),
            Command::ResearchArchive => tr("Archive this conversation's research"),
            Command::Notebook => tr("Show the notebook"),
            Command::Provenance => tr("Show the provenance of this conversation"),
            Command::Agents => tr("Show the agents"),
            Command::Environments => tr("Show where this conversation's work may run"),
            Command::ClosePanel => tr("Close the side panel"),
            Command::ToggleSidebar => tr("Show or hide the sidebar"),
            Command::ExportProject => tr("Export this project as an archive"),
            Command::ImportConversations => tr("Import conversations from Codex or Claude Code"),
            Command::NewProject => tr("New project…"),
            Command::ImportProject => tr("Import a project archive or folder package"),
            Command::WebViewImport => tr("Bring over projects from the WebView desktop"),
            Command::Skills => tr("Manage skills"),
            Command::CheckUpdates => tr("Check for updates"),
            Command::StarUs => tr("Star Wisp on GitHub"),
            Command::LightTheme => tr("Light appearance"),
            Command::DarkTheme => tr("Dark appearance"),
            Command::SystemTheme => tr("Appearance follows the system"),
            Command::LargerText => tr("Larger text"),
            Command::SmallerText => tr("Smaller text"),
            Command::LargerCode => tr("Larger code"),
            Command::SmallerCode => tr("Smaller code"),
            Command::Setup => tr("Show the first-run guide"),
            Command::PrivacyMode => tr("Privacy mode"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Command::NewConversation => "plus",
            Command::OpenFolder => "folder-plus",
            Command::Home => "arrow-left",
            Command::Files => "folder",
            Command::Runs => "activity-orbit",
            Command::Artifacts => "layers",
            Command::Memos => "note-plus",
            Command::Memory => "memory",
            Command::Highlights | Command::Favorites => "star",
            Command::Archived => "archive",
            Command::Refresh => "refresh",
            Command::Settings => "gear",
            Command::Docs => "book",
            Command::Feedback => "bubble",
            Command::ProjectSettings => "adjustments",
            Command::Find => "search",
            Command::Journey => "timeline",
            Command::Papers => "doc",
            Command::Schedules => "clock",
            Command::Assistant => "bot",
            Command::Mandates => "gauge",
            Command::Calendar => "calendar",
            Command::NewWindow => "dock",
            Command::Capabilities => "sparkles",
            Command::Trajectory => "gauge",
            Command::Aside => "circle-help",
            Command::SaveAsSkill => "skill",
            Command::Review => "review",
            Command::ModelView => "eye",
            Command::Terminal => "terminal",
            Command::PythonConsole => "code",
            Command::RConsole => "runtime-panel",
            Command::ResearchArchive => "archive-export",
            Command::Notebook => "notebook",
            Command::Provenance => "research-trail",
            Command::Agents => "swarm",
            Command::Environments => "server",
            Command::ClosePanel => "close",
            Command::ToggleSidebar => "panel-left",
            Command::ExportProject => "download",
            Command::ImportConversations => "conversation-import",
            Command::NewProject => "square-pen",
            Command::ImportProject => "archive-import",
            Command::WebViewImport => "computer",
            Command::Skills => "lightbulb",
            Command::CheckUpdates => "sync",
            Command::StarUs => "external-link",
            Command::LightTheme => "sun",
            Command::DarkTheme => "moon",
            Command::SystemTheme => "monitor",
            Command::LargerText => "expand",
            Command::SmallerText => "compact",
            Command::LargerCode => "code-increase",
            Command::SmallerCode => "code-decrease",
            Command::Setup => "compass",
            Command::PrivacyMode => "eye-off",
        }
    }

    /// Commands that act on a project are offered only inside one.
    fn available(self, screen: Screen) -> bool {
        match self {
            Command::NewConversation
            | Command::Home
            | Command::Files
            | Command::Runs
            | Command::Artifacts
            | Command::Memos
            | Command::Memory
            | Command::Highlights
            | Command::Archived
            | Command::ProjectSettings
            | Command::Find
            | Command::Journey
            | Command::Papers
            | Command::Capabilities
            | Command::Trajectory
            | Command::Aside
            | Command::SaveAsSkill
            | Command::Review
            | Command::ModelView
            | Command::Terminal
            | Command::PythonConsole
            | Command::RConsole
            | Command::ResearchArchive
            | Command::Notebook
            | Command::Provenance
            | Command::Agents
            | Command::Environments
            | Command::ClosePanel
            | Command::ToggleSidebar
            | Command::ExportProject
            | Command::ImportConversations => screen == Screen::Project,
            Command::OpenFolder
            | Command::Refresh
            | Command::Settings
            | Command::Favorites
            | Command::Docs
            | Command::Feedback
            | Command::Calendar
            | Command::Schedules
            | Command::Assistant
            | Command::Mandates
            | Command::NewWindow
            | Command::NewProject
            | Command::ImportProject
            | Command::WebViewImport
            | Command::Skills
            | Command::CheckUpdates
            | Command::StarUs
            | Command::LightTheme
            | Command::DarkTheme
            | Command::SystemTheme
            | Command::LargerText
            | Command::SmallerText
            | Command::LargerCode
            | Command::SmallerCode
            | Command::Setup
            | Command::PrivacyMode => true,
        }
    }

    const ALL: [Command; 57] = [
        Command::NewConversation,
        Command::OpenFolder,
        Command::Home,
        Command::Files,
        Command::Runs,
        Command::Artifacts,
        Command::Memos,
        Command::Memory,
        Command::Highlights,
        Command::Favorites,
        Command::Archived,
        Command::Refresh,
        Command::Settings,
        Command::Docs,
        Command::Feedback,
        Command::ProjectSettings,
        Command::Find,
        Command::Journey,
        Command::Papers,
        Command::Schedules,
        Command::Assistant,
        Command::Mandates,
        Command::Calendar,
        Command::NewWindow,
        Command::Capabilities,
        Command::Trajectory,
        Command::Aside,
        Command::SaveAsSkill,
        Command::Review,
        Command::ModelView,
        Command::Terminal,
        Command::PythonConsole,
        Command::RConsole,
        Command::ResearchArchive,
        Command::Notebook,
        Command::Provenance,
        Command::Agents,
        Command::Environments,
        Command::ClosePanel,
        Command::ToggleSidebar,
        Command::ExportProject,
        Command::ImportConversations,
        Command::NewProject,
        Command::ImportProject,
        Command::WebViewImport,
        Command::Skills,
        Command::CheckUpdates,
        Command::StarUs,
        Command::LightTheme,
        Command::DarkTheme,
        Command::SystemTheme,
        Command::LargerText,
        Command::SmallerText,
        Command::LargerCode,
        Command::SmallerCode,
        Command::Setup,
        Command::PrivacyMode,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Projects,
    Files,
    Artifacts,
    Environments,
    Runtimes,
    Conversations,
    Skills,
    Workflows,
    Memos,
    Commands,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Projects => tr("Projects"),
            Section::Files => tr("Files"),
            Section::Artifacts => tr("Artifacts"),
            Section::Environments => tr("Environments"),
            Section::Runtimes => tr("Runtimes"),
            Section::Conversations => tr("Conversations"),
            Section::Skills => tr("Skills"),
            Section::Workflows => tr("Workflows"),
            // The WebView's words: a memo is put into the message, and
            // nothing else of it goes anywhere.
            Section::Memos => tr("Memos · local only, nothing is sent"),
            Section::Commands => tr("Commands"),
        }
    }

    /// The sections the add-context box fills from what the window already
    /// reads, rather than from a search of the host.
    const OFFERED: [Section; 5] = [
        Section::Artifacts,
        Section::Environments,
        Section::Runtimes,
        Section::Workflows,
        Section::Memos,
    ];

    fn cap(self) -> usize {
        match self {
            Section::Projects => PROJECTS,
            Section::Files => FILES,
            Section::Artifacts => ARTIFACTS,
            Section::Environments => ENVIRONMENTS,
            Section::Runtimes => RUNTIMES,
            Section::Conversations => CONVERSATIONS,
            Section::Skills => SKILLS,
            Section::Workflows => WORKFLOWS,
            Section::Memos => MEMOS,
            Section::Commands => usize::MAX,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Project {
        id: String,
        name: String,
        path: String,
    },
    File(FileMatch),
    Conversation {
        project: String,
        id: String,
        title: String,
        place: String,
    },
    Skill {
        name: String,
        description: String,
    },
    /// A file the project recorded, to attach by its artifact id.
    Artifact {
        id: String,
        name: String,
        detail: String,
    },
    /// This computer or a saved host, for the turn's work to run on.
    Environment {
        id: String,
        label: String,
        detail: String,
    },
    /// An interpreter that keeps its variables, `python` or `r`.
    Runtime {
        context_id: String,
        language: String,
        title: String,
        detail: String,
    },
    Workflow {
        id: String,
        name: String,
        goal: String,
    },
    /// A memo: picked, its text goes into the message; it is never a
    /// reference the model can follow.
    Memo {
        heading: String,
        preview: String,
        body: String,
    },
    Command(Command),
}

impl Entry {
    pub fn section(&self) -> Section {
        match self {
            Entry::Project { .. } => Section::Projects,
            Entry::File(_) => Section::Files,
            Entry::Skill { .. } => Section::Skills,
            Entry::Conversation { .. } => Section::Conversations,
            Entry::Artifact { .. } => Section::Artifacts,
            Entry::Environment { .. } => Section::Environments,
            Entry::Runtime { .. } => Section::Runtimes,
            Entry::Workflow { .. } => Section::Workflows,
            Entry::Memo { .. } => Section::Memos,
            Entry::Command(_) => Section::Commands,
        }
    }

    /// The icon, the words and the line under them, as the box lists the
    /// row; files, projects, conversations and commands are drawn by the
    /// view.
    pub fn glyph(&self) -> &'static str {
        match self {
            Entry::Project { .. } => "folder",
            Entry::File(_) => "doc",
            Entry::Conversation { .. } => "chat",
            Entry::Skill { .. } => "skill",
            Entry::Artifact { .. } => "layers",
            Entry::Environment { .. } => "server",
            Entry::Runtime { .. } => "terminal",
            Entry::Workflow { .. } => "branch",
            Entry::Memo { .. } => "notebook",
            Entry::Command(command) => command.icon(),
        }
    }

    /// What typing is matched against, for the rows the box filters itself.
    /// As in the WebView, an environment is also found by "server" and a
    /// runtime by "runtime".
    fn search_text(&self) -> String {
        match self {
            Entry::Artifact { name, detail, .. } => format!("{name} {detail}"),
            Entry::Environment { id, label, detail } => {
                format!("server environment {label} {id} {detail}")
            }
            Entry::Runtime {
                context_id,
                language,
                title,
                detail,
            } => format!("runtime {language} {title} {context_id} {detail}"),
            Entry::Workflow { name, goal, .. } => format!("{name} {goal}"),
            Entry::Memo { heading, body, .. } => format!("{heading} {body}"),
            Entry::Skill { name, description } => format!("{name} {description}"),
            _ => String::new(),
        }
    }

    /// What picking this row in the add-context box attaches. A memo
    /// attaches nothing: its text goes into the message instead.
    pub fn chip(&self) -> Option<Chip> {
        Some(match self {
            Entry::File(file) => Chip::File(file.path.clone()),
            Entry::Conversation { id, title, .. } => Chip::Conversation {
                id: id.clone(),
                title: title.clone(),
            },
            Entry::Skill { name, .. } => Chip::Skill(name.clone()),
            Entry::Artifact { id, name, .. } => Chip::Artifact {
                id: id.clone(),
                name: name.clone(),
            },
            Entry::Environment { id, label, .. } => Chip::Environment {
                id: id.clone(),
                label: label.clone(),
            },
            Entry::Runtime {
                context_id,
                language,
                ..
            } => Chip::Runtime {
                context_id: context_id.clone(),
                language: language.clone(),
                label: crate::compose::runtime_chip_label(language, context_id),
            },
            Entry::Workflow { id, name, .. } => Chip::Workflow {
                id: id.clone(),
                name: name.clone(),
            },
            Entry::Project { .. } | Entry::Memo { .. } | Entry::Command(_) => return None,
        })
    }

    /// What this row adds to the message being written in `project`
    /// (conversation `session`, when one is open), found while looking
    /// for something to go to: a file of that project, or another
    /// conversation.
    pub fn chip_in(&self, mode: Mode, project: &str, session: Option<&str>) -> Option<Chip> {
        if mode != Mode::Open {
            return None;
        }
        match self {
            Entry::File(file) if file.project_id == project => Some(Chip::File(file.path.clone())),
            Entry::Conversation { id, title, .. } if Some(id.as_str()) != session => {
                Some(Chip::Conversation {
                    id: id.clone(),
                    title: title.clone(),
                })
            }
            _ => None,
        }
    }

    /// The project this row can also open in a window of its own: a
    /// project found to go to, not one listed to attach.
    pub fn window_project(&self, mode: Mode) -> Option<&str> {
        match (self, mode) {
            (Entry::Project { id, .. }, Mode::Open) => Some(id),
            _ => None,
        }
    }
}

/// Every whitespace-separated word of `query` occurs in `text`, ignoring
/// case. An empty query matches everything.
pub fn matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// Arrow keys move the selection; anything modified is someone else's chord.
pub fn step(key: &str, modified: bool) -> Option<isize> {
    match (key, modified) {
        ("down", false) => Some(1),
        ("up", false) => Some(-1),
        _ => None,
    }
}

/// The project's artifacts to attach, those of the conversation `session`
/// first and then the newest, as the WebView ranks them.
pub fn artifact_entries(rows: &[ArtifactRow], session: Option<&str>) -> Vec<Entry> {
    let mut rows: Vec<&ArtifactRow> = rows.iter().collect();
    rows.sort_by_key(|row| {
        (
            Some(row.session_id.as_str()) != session,
            std::cmp::Reverse(row.created_at),
        )
    });
    rows.into_iter()
        .map(|row| {
            let place = row.path.clone().unwrap_or_else(|| row.kind.clone());
            Entry::Artifact {
                id: row.id.clone(),
                name: row.name.clone(),
                detail: match crate::present::one_line(&row.session_title) {
                    title if title.is_empty() => place,
                    title => format!("{title} · {place}"),
                },
            }
        })
        .collect()
}

/// Where work can run: this computer, then the saved SSH hosts, each by
/// the execution context id the host knows it by.
pub fn environment_entries(hosts: &[SshHostRow]) -> Vec<Entry> {
    let mut entries = vec![Entry::Environment {
        id: "local".into(),
        label: tr("This computer").into(),
        detail: "local".into(),
    }];
    entries.extend(hosts.iter().map(|host| Entry::Environment {
        id: format!("ssh:{}", host.alias),
        label: host.alias.clone(),
        detail: format!("ssh:{}", host.alias),
    }));
    entries
}

/// The interpreters a turn can be pointed at: they run on this computer,
/// Python always (as the WebView offers it there) and R once the
/// conversation has one, each with how the one above the composer is.
pub fn runtime_entries(rows: &[RuntimeRow]) -> Vec<Entry> {
    let row_of = |language: RuntimeLanguage| rows.iter().rev().find(|row| row.language == language);
    [
        ("python", RuntimeLanguage::Python),
        ("r", RuntimeLanguage::R),
    ]
    .into_iter()
    .filter(|(_, language)| *language == RuntimeLanguage::Python || row_of(*language).is_some())
    .map(|(id, language)| Entry::Runtime {
        context_id: "local".into(),
        language: id.into(),
        title: crate::i18n::fill(tr("{} runtime"), &[&crate::compose::language_name(id)]),
        detail: match row_of(language) {
            Some(row) => format!(
                "{} · {}",
                tr("This computer"),
                crate::controls::runtime_label(row)
            ),
            None => tr("This computer").to_owned(),
        },
    })
    .collect()
}

pub fn workflow_entries(workflows: &[Workflow]) -> Vec<Entry> {
    workflows
        .iter()
        .map(|workflow| Entry::Workflow {
            id: workflow.id.clone(),
            name: workflow.name.clone(),
            goal: crate::present::one_line(&workflow.goal),
        })
        .collect()
}

/// The project's memos, pinned ones first as the Memos tab lists them.
pub fn memo_entries(memos: &[ProjectMemo]) -> Vec<Entry> {
    let mut memos: Vec<&ProjectMemo> = memos.iter().collect();
    memos.sort_by_key(|memo| !memo.pinned);
    memos
        .into_iter()
        .filter(|memo| !memo.body.trim().is_empty())
        .map(|memo| Entry::Memo {
            heading: crate::memos::heading(memo),
            preview: crate::memos::preview(memo),
            body: memo.body.clone(),
        })
        .collect()
}

#[derive(Default)]
pub struct PaletteState {
    generation: u64,
    /// Counts the times the box was opened; rows offered for an earlier
    /// opening are not listed.
    opening: u64,
    pub query: String,
    conversations: Vec<SessionSearchInfo>,
    files: Vec<FileMatch>,
    skills: Vec<ReferenceOption>,
    /// Artifacts, environments, runtimes, workflows and memos to attach,
    /// filtered here as the query changes.
    offered: Vec<Entry>,
    pub selected: usize,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

/// One opening of the box, for rows offered once while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening(u64);

impl PaletteState {
    /// Starts over for a newly opened palette.
    pub fn reset(&mut self, mode: Mode) {
        *self = Self {
            generation: self.generation + 1,
            opening: self.opening + 1,
            mode,
            ..Self::default()
        };
    }

    pub fn opening(&self) -> Opening {
        Opening(self.opening)
    }

    /// Offers `entries` as the rows of `section` while the box stays open
    /// from `opening`, in place of what that section offered before.
    /// Only rows of the sections filled this way are taken.
    pub fn offer(&mut self, opening: Opening, section: Section, entries: Vec<Entry>) -> bool {
        if opening.0 != self.opening || !Section::OFFERED.contains(&section) {
            return false;
        }
        self.offered.retain(|entry| entry.section() != section);
        self.offered.extend(
            entries
                .into_iter()
                .filter(|entry| entry.section() == section),
        );
        true
    }

    /// The rows offered for `section` that match what is typed.
    fn offered_in(&self, section: Section) -> impl Iterator<Item = Entry> + '_ {
        self.offered
            .iter()
            .filter(move |entry| {
                entry.section() == section && matches(&entry.search_text(), &self.query)
            })
            .take(section.cap())
            .cloned()
    }

    /// Records the typed query. Host results for earlier text are dropped
    /// from now on, and the selection returns to the first row.
    pub fn begin(&mut self, query: &str) -> Ticket {
        self.generation += 1;
        self.query = query.trim().to_owned();
        self.selected = 0;
        // Until the host answers, keep only what still fits the new text:
        // no flicker while typing, and nothing listed that does not match.
        let query = self.query.clone();
        if query.is_empty() {
            self.conversations.clear();
            self.files.clear();
        } else {
            self.files.retain(|file| matches(&file.path, &query));
            self.conversations.retain(|row| matches(&row.title, &query));
        }
        // Skills are listed even with nothing typed, so they are filtered
        // rather than cleared.
        self.skills
            .retain(|skill| matches(&format!("{} {}", skill.label, skill.detail), &query));
        Ticket(self.generation)
    }

    pub fn current(&self, ticket: Ticket) -> bool {
        ticket.0 == self.generation
    }

    pub fn accept_conversations(&mut self, ticket: Ticket, rows: Vec<SessionSearchInfo>) -> bool {
        let current = self.current(ticket);
        if current {
            self.conversations = rows;
        }
        current
    }

    pub fn accept_files(&mut self, ticket: Ticket, files: Vec<FileMatch>) -> bool {
        let current = self.current(ticket);
        if current {
            self.files = files;
        }
        current
    }

    pub fn accept_skills(&mut self, ticket: Ticket, skills: Vec<ReferenceOption>) -> bool {
        let current = self.current(ticket);
        if current {
            self.skills = skills;
        }
        current
    }

    /// The rows on screen. To open: projects, files, conversations and
    /// commands. To attach: files, artifacts, environments, runtimes,
    /// conversations, skills, workflows and memos. With nothing typed the
    /// conversations are the `recent` ones.
    pub fn entries(
        &self,
        projects: &[ProjectSummary],
        recent: &[SessionSearchInfo],
        screen: Screen,
    ) -> Vec<Entry> {
        let query = self.query.as_str();
        let attach = self.mode == Mode::Attach;
        if self.mode == Mode::Commands {
            return Command::ALL
                .into_iter()
                .filter(|command| command.available(screen) && matches(command.label(), query))
                .map(Entry::Command)
                .collect();
        }
        let mut entries: Vec<Entry> = projects
            .iter()
            .filter(|project| {
                !attach
                    && (matches(&project.name, query)
                        || (!query.is_empty() && matches(&project.workspace_dir, query)))
            })
            .take(PROJECTS)
            .map(|project| Entry::Project {
                id: project.id.clone(),
                name: project.name.clone(),
                path: project.workspace_dir.clone(),
            })
            .collect();
        entries.extend(self.files.iter().take(FILES).cloned().map(Entry::File));
        if attach {
            for section in [Section::Artifacts, Section::Environments, Section::Runtimes] {
                entries.extend(self.offered_in(section));
            }
        }
        let conversations = if query.is_empty() {
            recent
        } else {
            &self.conversations
        };
        entries.extend(
            conversations
                .iter()
                .take(CONVERSATIONS)
                .map(|row| Entry::Conversation {
                    project: row.project_id.clone(),
                    id: row.id.clone(),
                    title: crate::present::one_line(&row.title),
                    place: if row.shelved {
                        crate::i18n::fill(tr("{} · Archived"), &[&row.project_name])
                    } else {
                        row.project_name.clone()
                    },
                }),
        );
        if attach {
            entries.extend(self.skills.iter().take(SKILLS).filter_map(
                |skill| match &skill.reference {
                    wisp_dto::ComposerReferenceArg::Skill { name } => Some(Entry::Skill {
                        name: name.clone(),
                        description: skill.detail.clone(),
                    }),
                    _ => None,
                },
            ));
            for section in [Section::Workflows, Section::Memos] {
                entries.extend(self.offered_in(section));
            }
            return entries;
        }
        entries.extend(
            Command::ALL
                .into_iter()
                .filter(|command| command.available(screen) && matches(command.label(), query))
                .map(Entry::Command),
        );
        entries
    }

    /// Moves the selection by `delta` rows, wrapping at both ends.
    pub fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        let len = len as isize;
        self.selected =
            (self.selected.min(len as usize - 1) as isize + delta).rem_euclid(len) as usize;
    }

    /// The selected row, clamped: results can shrink under the selection.
    pub fn selection(&self, len: usize) -> Option<usize> {
        (len > 0).then(|| self.selected.min(len - 1))
    }

    /// Keeps the selection on the row it was on when results that arrive
    /// later are listed above it, so Enter opens what was highlighted. A
    /// row that is gone leaves the selection where it was.
    pub fn follow_selection(&mut self, before: &[Entry], after: &[Entry]) {
        let Some(selected) = self.selection(before.len()).map(|index| &before[index]) else {
            return;
        };
        if let Some(index) = after.iter().position(|entry| entry == selected) {
            self.selected = index;
        }
    }
}

/// Position of entry `index` among the list's children, where every section
/// starts with one heading row. Used to scroll the selection into view.
pub fn child_index(entries: &[Entry], index: usize) -> usize {
    let mut headings = 0;
    let mut previous = None;
    for entry in entries.iter().take(index + 1) {
        if previous != Some(entry.section()) {
            headings += 1;
            previous = Some(entry.section());
        }
    }
    index + headings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_file_or_another_conversation_found_can_join_the_message() {
        let file = |project: &str| {
            Entry::File(FileMatch {
                project_id: project.into(),
                project_name: "Lab".into(),
                path: "data/reads.csv".into(),
            })
        };
        assert_eq!(
            file("p").chip_in(Mode::Open, "p", Some("s")),
            Some(Chip::File("data/reads.csv".into()))
        );
        // Its path means nothing in another project.
        assert_eq!(file("q").chip_in(Mode::Open, "p", Some("s")), None);
        let other = Entry::Conversation {
            project: "q".into(),
            id: "t".into(),
            title: "Counts".into(),
            place: "Lab".into(),
        };
        assert_eq!(
            other.chip_in(Mode::Open, "p", Some("s")),
            Some(Chip::Conversation {
                id: "t".into(),
                title: "Counts".into()
            })
        );
        // Not context for itself, and not offered twice while attaching.
        assert_eq!(other.chip_in(Mode::Open, "p", Some("t")), None);
        assert_eq!(file("p").chip_in(Mode::Attach, "p", None), None);
        assert_eq!(
            Entry::Command(Command::NewWindow).chip_in(Mode::Open, "p", None),
            None
        );
    }

    #[test]
    fn a_project_found_to_go_to_can_open_in_a_new_window() {
        let found = Entry::Project {
            id: "p1".into(),
            name: "Lab".into(),
            path: "/lab".into(),
        };
        assert_eq!(found.window_project(Mode::Open), Some("p1"));
        assert_eq!(found.window_project(Mode::Attach), None);
        assert_eq!(
            Entry::Command(Command::NewWindow).window_project(Mode::Open),
            None
        );
    }

    fn project(name: &str, path: &str) -> ProjectSummary {
        serde_json::from_value(json!({"id": name, "name": name, "workspace_dir": path})).unwrap()
    }

    fn conversation(id: &str, title: &str, shelved: bool) -> SessionSearchInfo {
        serde_json::from_value(json!({
            "id": id, "project_id": "p", "project_name": "Lab", "title": title,
            "ts": 1, "activity_at": 1, "status": "complete", "shelved": shelved
        }))
        .unwrap()
    }

    fn file(path: &str) -> FileMatch {
        FileMatch {
            project_id: "p".into(),
            project_name: "Lab".into(),
            path: path.into(),
        }
    }

    fn sections(entries: &[Entry]) -> Vec<Section> {
        entries.iter().map(Entry::section).collect()
    }

    #[test]
    fn nothing_typed_lists_projects_recent_conversations_and_commands() {
        let state = PaletteState::default();
        let projects = [project("Lab", "/data/lab"), project("Field", "/data/field")];
        let recent = [conversation("a", "Alignment", false)];
        let home = state.entries(&projects, &recent, Screen::Home);
        let everywhere = Command::ALL
            .into_iter()
            .filter(|command| command.available(Screen::Home))
            .count();
        let mut expected = vec![Section::Projects, Section::Projects, Section::Conversations];
        expected.extend(std::iter::repeat_n(Section::Commands, everywhere));
        assert_eq!(sections(&home), expected);
        // The WebView's commands are here: appearance and text size from
        // anywhere, the side panel's tabs inside a project.
        for command in [Command::DarkTheme, Command::LargerText, Command::Skills] {
            assert!(home.contains(&Entry::Command(command)));
        }
        // Scheduled tasks span projects, so they are offered at home too,
        // and so is the research assistant.
        assert!(home.contains(&Entry::Command(Command::Schedules)));
        assert!(home.contains(&Entry::Command(Command::Assistant)));
        // Project-only commands appear once a project is open.
        let inside = state.entries(&projects, &recent, Screen::Project);
        assert!(inside.contains(&Entry::Command(Command::NewConversation)));
        assert!(!home.contains(&Entry::Command(Command::NewConversation)));
        // A project's settings are offered inside it; from the home page
        // each card has its own button.
        assert!(inside.contains(&Entry::Command(Command::ProjectSettings)));
        assert!(!home.contains(&Entry::Command(Command::ProjectSettings)));
        assert!(home.contains(&Entry::Command(Command::Settings)));
        // Help is offered wherever you are, and is found by what it is for.
        for help in [Command::Docs, Command::Feedback] {
            assert!(home.contains(&Entry::Command(help)));
            assert!(inside.contains(&Entry::Command(help)));
        }
        // Favorites span projects, so they are offered from the home page;
        // one conversation's highlights are not.
        assert!(home.contains(&Entry::Command(Command::Favorites)));
        assert!(!home.contains(&Entry::Command(Command::Highlights)));
        assert!(inside.contains(&Entry::Command(Command::Highlights)));
        for command in [
            Command::Environments,
            Command::Provenance,
            Command::ExportProject,
        ] {
            assert!(inside.contains(&Entry::Command(command)));
            assert!(!home.contains(&Entry::Command(command)));
        }
        // Each command has a name and an icon of its own.
        let mut labels: Vec<&str> = Command::ALL.iter().map(|command| command.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), Command::ALL.len());
    }

    #[test]
    fn a_query_filters_every_section_and_uses_host_results() {
        let mut state = PaletteState::default();
        let projects = [project("Lab", "/data/lab"), project("Field", "/data/trips")];
        let recent = [conversation("r", "Unrelated recent", false)];
        let ticket = state.begin("  SET  ");
        assert_eq!(state.query, "SET");
        assert!(state.accept_files(ticket, vec![file("data/dataset.csv")]));
        assert!(
            state.accept_conversations(ticket, vec![conversation("a", "Reset the pipeline", true)])
        );
        let entries = state.entries(&projects, &recent, Screen::Home);
        assert_eq!(
            entries,
            [
                Entry::File(file("data/dataset.csv")),
                Entry::Conversation {
                    project: "p".into(),
                    id: "a".into(),
                    title: "Reset the pipeline".into(),
                    place: "Lab · Archived".into(),
                },
                Entry::Command(Command::Settings),
            ]
        );
        // A project is found by its folder as well as its name.
        state.begin("trips");
        let entries = state.entries(&projects, &recent, Screen::Home);
        assert_eq!(sections(&entries), [Section::Projects]);
    }

    #[test]
    fn results_for_earlier_text_never_fill_the_list() {
        let mut state = PaletteState::default();
        let old = state.begin("ali");
        let new = state.begin("alig");
        assert!(!state.accept_conversations(old, vec![conversation("old", "Old", false)]));
        assert!(!state.accept_files(old, vec![file("old.txt")]));
        assert!(state.accept_conversations(new, vec![conversation("new", "New", false)]));
        let entries = state.entries(&[], &[], Screen::Home);
        assert_eq!(entries.len(), 1);
        // Clearing the box drops host results and falls back to recent ones.
        let cleared = state.begin("");
        assert!(
            state
                .entries(&[], &[], Screen::Home)
                .iter()
                .all(|e| e.section() == Section::Commands)
        );
        assert!(!state.current(new) && state.current(cleared));
        // Reopening the palette also invalidates what was in flight.
        state.reset(Mode::Open);
        assert!(!state.current(cleared));
    }

    #[test]
    fn the_command_palette_lists_only_commands() {
        let mut state = PaletteState::default();
        state.reset(Mode::Commands);
        let ticket = state.begin("o");
        state.accept_files(ticket, vec![file("notes.txt")]);
        state.accept_conversations(ticket, vec![conversation("1", "notes", false)]);
        let projects = vec![project("notes", "/x")];
        let entries = state.entries(&projects, &[], Screen::Home);
        assert!(!entries.is_empty());
        assert!(
            entries
                .iter()
                .all(|entry| entry.section() == Section::Commands)
        );
    }

    #[test]
    fn sections_are_capped_so_none_buries_the_others() {
        let mut state = PaletteState::default();
        let ticket = state.begin("a");
        state.accept_files(
            ticket,
            (0..20).map(|i| file(&format!("a{i}.txt"))).collect(),
        );
        state.accept_conversations(
            ticket,
            (0..20)
                .map(|i| conversation(&i.to_string(), "a", false))
                .collect(),
        );
        let projects: Vec<_> = (0..20).map(|i| project(&format!("a{i}"), "/x")).collect();
        let entries = state.entries(&projects, &[], Screen::Home);
        let count = |section| entries.iter().filter(|e| e.section() == section).count();
        assert_eq!(count(Section::Projects), PROJECTS);
        assert_eq!(count(Section::Files), FILES);
        assert_eq!(count(Section::Conversations), CONVERSATIONS);
    }

    #[test]
    fn the_selection_wraps_resets_and_survives_shrinking_results() {
        let mut state = PaletteState::default();
        state.move_selection(1, 3);
        state.move_selection(1, 3);
        assert_eq!(state.selected, 2);
        state.move_selection(1, 3);
        assert_eq!(state.selected, 0);
        state.move_selection(-1, 3);
        assert_eq!(state.selected, 2);
        // Fewer rows than before: the selection is clamped, not dangling.
        assert_eq!(state.selection(2), Some(1));
        state.move_selection(-1, 2);
        assert_eq!(state.selected, 0);
        assert_eq!(state.selection(0), None);
        state.move_selection(1, 0);
        assert_eq!(state.selected, 0);
        state.selected = 2;
        state.begin("x");
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn late_results_do_not_move_the_highlighted_row() {
        let mut state = PaletteState::default();
        let ticket = state.begin("python");
        let before = state.entries(&[], &[], Screen::Project);
        let console = before
            .iter()
            .position(|entry| *entry == Entry::Command(Command::PythonConsole))
            .expect("the command matches what is typed");
        state.selected = console;
        // Conversations found by the host are listed above the commands.
        state.accept_conversations(ticket, vec![conversation("c1", "print in python", false)]);
        let after = state.entries(&[], &[], Screen::Project);
        assert_ne!(after[console], before[console]);
        state.follow_selection(&before, &after);
        let selected = state.selection(after.len()).unwrap();
        assert_eq!(after[selected], Entry::Command(Command::PythonConsole));
        // A row that is no longer listed leaves the selection alone.
        state.selected = 0;
        let gone = vec![Entry::Command(Command::Settings)];
        state.follow_selection(&gone, &after);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn matching_and_keys() {
        assert!(matches("New conversation", "conv new"));
        assert!(matches("Show or hide archived conversations", ""));
        assert!(!matches("Settings", "set x"));
        assert_eq!(step("down", false), Some(1));
        assert_eq!(step("up", false), Some(-1));
        assert_eq!(step("down", true), None);
        assert_eq!(step("j", false), None);
    }

    #[test]
    fn child_positions_count_one_heading_per_section() {
        let entries = [
            Entry::Project {
                id: "p".into(),
                name: "P".into(),
                path: "/p".into(),
            },
            Entry::Project {
                id: "q".into(),
                name: "Q".into(),
                path: "/q".into(),
            },
            Entry::File(file("a.txt")),
            Entry::Command(Command::Settings),
            Entry::Command(Command::Refresh),
        ];
        assert_eq!(
            (0..5).map(|i| child_index(&entries, i)).collect::<Vec<_>>(),
            [1, 2, 4, 6, 7]
        );
    }

    fn artifact(id: &str, session: &str, created_at: i64) -> ArtifactRow {
        ArtifactRow {
            id: id.into(),
            name: format!("{id}.png"),
            kind: "image/png".into(),
            path: Some(format!("figures/{id}.png")),
            created_at,
            size_bytes: None,
            session_id: session.into(),
            session_title: format!("Talk {session}"),
        }
    }

    fn interpreter(language: RuntimeLanguage) -> RuntimeRow {
        RuntimeRow {
            id: "rt".into(),
            language,
            state: wisp_server::desktop::RuntimeState::Ready,
            interpreter: None,
            version: Some("4.4".into()),
            memory_bytes: None,
            last_error: None,
            process_id: None,
            generation: 1,
            started_at_ms: 0,
            last_activity_at_ms: 0,
        }
    }

    #[test]
    fn what_the_window_reads_is_offered_in_the_webviews_order() {
        // This conversation's artifacts first, then the newest.
        let rows = [
            artifact("old", "s", 1),
            artifact("new", "t", 9),
            artifact("mid", "t", 5),
        ];
        let ids: Vec<String> = artifact_entries(&rows, Some("s"))
            .into_iter()
            .map(|entry| match entry {
                Entry::Artifact { id, detail, .. } => {
                    assert!(detail.starts_with("Talk ") && detail.contains("figures/"));
                    id
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(ids, ["old", "new", "mid"]);
        // This computer, then each saved host by its context id.
        let hosts = [SshHostRow {
            alias: "gpu".into(),
            ..Default::default()
        }];
        let places: Vec<Option<Chip>> = environment_entries(&hosts)
            .iter()
            .map(Entry::chip)
            .collect();
        assert_eq!(
            places,
            [
                Some(Chip::Environment {
                    id: "local".into(),
                    label: "This computer".into()
                }),
                Some(Chip::Environment {
                    id: "ssh:gpu".into(),
                    label: "gpu".into()
                }),
            ]
        );
        // Python always; R once the conversation has one.
        let runtimes = runtime_entries(&[]);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(
            runtimes[0].chip(),
            Some(Chip::Runtime {
                context_id: "local".into(),
                language: "python".into(),
                label: "Python · This computer".into(),
            })
        );
        let with_r = runtime_entries(&[interpreter(RuntimeLanguage::R)]);
        assert!(
            matches!(&with_r[1], Entry::Runtime { language, title, detail, .. }
            if language == "r" && title == "R runtime" && detail.contains("R 4.4"))
        );
        // Pinned memos first; an empty one has nothing to put in.
        let memo = |title: &str, body: &str, pinned: bool| ProjectMemo {
            title: title.into(),
            body: body.into(),
            pinned,
            ..Default::default()
        };
        let memos = memo_entries(&[
            memo("Later", "plain", false),
            memo("", "   ", true),
            memo("First", "kept on top", true),
        ]);
        let headings: Vec<_> = memos
            .iter()
            .map(|entry| match entry {
                Entry::Memo { heading, .. } => heading.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(headings, ["First", "Later"]);
        // A memo is never a reference: its text goes into the message.
        assert_eq!(memos[0].chip(), None);
        let workflow = Workflow {
            id: "w1".into(),
            name: "Screen".into(),
            goal: "Find\nthe hits".into(),
            ..Default::default()
        };
        assert_eq!(
            workflow_entries(&[workflow]),
            [Entry::Workflow {
                id: "w1".into(),
                name: "Screen".into(),
                goal: crate::present::one_line("Find\nthe hits"),
            }]
        );
    }

    #[test]
    fn attaching_lists_every_kind_under_its_own_heading_and_filters_it() {
        let mut state = PaletteState::default();
        state.reset(Mode::Attach);
        let opening = state.opening();
        let hosts = [SshHostRow {
            alias: "gpu".into(),
            ..Default::default()
        }];
        let workflow = Workflow {
            id: "w1".into(),
            name: "Screen".into(),
            goal: "Find the hits".into(),
            ..Default::default()
        };
        let memo = ProjectMemo {
            title: "Primers".into(),
            body: "F: ACGT".into(),
            ..Default::default()
        };
        assert!(state.offer(opening, Section::Memos, memo_entries(&[memo])));
        assert!(state.offer(opening, Section::Workflows, workflow_entries(&[workflow])));
        assert!(state.offer(opening, Section::Runtimes, runtime_entries(&[])));
        assert!(state.offer(opening, Section::Environments, environment_entries(&hosts)));
        assert!(state.offer(
            opening,
            Section::Artifacts,
            artifact_entries(&[artifact("fig", "s", 1)], Some("s"))
        ));
        // Rows of another kind are not taken for a section, and only the
        // add-context sections are filled this way.
        assert!(
            state.offer(
                opening,
                Section::Memos,
                environment_entries(&[])
                    .into_iter()
                    .chain(memo_entries(&[ProjectMemo {
                        title: "Primers".into(),
                        body: "F: ACGT".into(),
                        ..Default::default()
                    }]))
                    .collect()
            )
        );
        assert!(!state.offer(opening, Section::Commands, vec![]));
        state.begin("");
        let recent = [conversation("r", "Earlier alignment", false)];
        assert_eq!(
            sections(&state.entries(&[], &recent, Screen::Project)),
            [
                Section::Artifacts,
                Section::Environments,
                Section::Environments,
                Section::Runtimes,
                Section::Conversations,
                Section::Workflows,
                Section::Memos,
            ]
        );
        // Each kind is found by its own words, and by the WebView's.
        let found = |state: &mut PaletteState, query: &str| {
            state.begin(query);
            sections(&state.entries(&[], &[], Screen::Project))
        };
        assert_eq!(found(&mut state, "server"), [Section::Environments; 2]);
        assert_eq!(found(&mut state, "gpu"), [Section::Environments]);
        assert_eq!(found(&mut state, "runtime python"), [Section::Runtimes]);
        assert_eq!(found(&mut state, "figures"), [Section::Artifacts]);
        assert_eq!(found(&mut state, "hits"), [Section::Workflows]);
        assert_eq!(found(&mut state, "acgt"), [Section::Memos]);
        assert!(found(&mut state, "nothing like it").is_empty());
        // What was offered while the box was open before is not listed.
        state.reset(Mode::Attach);
        assert!(!state.offer(opening, Section::Memos, vec![]));
        assert!(
            state
                .entries(&[], &[], Screen::Project)
                .iter()
                .all(|entry| entry.section() != Section::Memos)
        );
        // Nor is any of it when looking for somewhere to go.
        let opening = state.opening();
        state.offer(opening, Section::Runtimes, runtime_entries(&[]));
        state.mode = Mode::Open;
        assert!(
            state
                .entries(&[], &[], Screen::Project)
                .iter()
                .all(|entry| entry.section() != Section::Runtimes)
        );
    }

    #[test]
    fn attaching_offers_files_conversations_and_skills_only() {
        let skill = |name: &str, detail: &str| ReferenceOption {
            reference: wisp_dto::ComposerReferenceArg::Skill { name: name.into() },
            label: name.into(),
            detail: detail.into(),
        };
        let mut state = PaletteState::default();
        state.reset(Mode::Attach);
        let projects = [project("Lab", "/data/lab")];
        let recent = [conversation("r", "Earlier alignment", false)];
        let ticket = state.begin("");
        state.accept_skills(
            ticket,
            vec![skill("align", "Align reads"), skill("plot", "Draw figures")],
        );
        // Nothing typed: recent conversations and every skill; no projects,
        // no commands.
        assert_eq!(
            sections(&state.entries(&projects, &recent, Screen::Project)),
            [Section::Conversations, Section::Skills, Section::Skills]
        );
        // Typing narrows the skills at once, before the host answers.
        let ticket = state.begin("figure");
        assert_eq!(
            state.entries(&projects, &recent, Screen::Project),
            [Entry::Skill {
                name: "plot".into(),
                description: "Draw figures".into()
            }]
        );
        assert!(state.accept_files(ticket, vec![file("figures/fig1.png")]));
        assert_eq!(
            sections(&state.entries(&projects, &recent, Screen::Project)),
            [Section::Files, Section::Skills]
        );
        // Opening again to navigate starts clean.
        state.reset(Mode::Open);
        assert!(
            state
                .entries(&projects, &recent, Screen::Project)
                .iter()
                .all(|entry| entry.section() != Section::Skills)
        );
    }
}
