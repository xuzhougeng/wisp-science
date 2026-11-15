//! The command palette's state: what is listed for a query, in which order,
//! and which row the keyboard is on. It holds no graphics state. Results
//! arrive from the host out of order, so every search carries a ticket and
//! only the latest one may fill the list.
use crate::i18n::tr;
use wisp_dto::{ProjectSummary, SessionSearchInfo};
use wisp_server::desktop::{FileMatch, ReferenceOption};

use crate::{compose::Chip, home::Screen};

/// Rows shown per section, so one kind of result never buries the others.
const PROJECTS: usize = 5;
const FILES: usize = 6;
const CONVERSATIONS: usize = 8;
const SKILLS: usize = 8;

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
            | Command::Setup => true,
        }
    }

    const ALL: [Command; 55] = [
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
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Projects,
    Files,
    Conversations,
    Skills,
    Commands,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Projects => tr("Projects"),
            Section::Files => tr("Files"),
            Section::Conversations => tr("Conversations"),
            Section::Skills => tr("Skills"),
            Section::Commands => tr("Commands"),
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
    Command(Command),
}

impl Entry {
    pub fn section(&self) -> Section {
        match self {
            Entry::Project { .. } => Section::Projects,
            Entry::File(_) => Section::Files,
            Entry::Skill { .. } => Section::Skills,
            Entry::Conversation { .. } => Section::Conversations,
            Entry::Command(_) => Section::Commands,
        }
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

#[derive(Default)]
pub struct PaletteState {
    generation: u64,
    pub query: String,
    conversations: Vec<SessionSearchInfo>,
    files: Vec<FileMatch>,
    skills: Vec<ReferenceOption>,
    pub selected: usize,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

impl PaletteState {
    /// Starts over for a newly opened palette.
    pub fn reset(&mut self, mode: Mode) {
        *self = Self {
            generation: self.generation + 1,
            mode,
            ..Self::default()
        };
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
    /// commands. To attach: files, conversations and skills. With nothing
    /// typed the conversations are the `recent` ones.
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
