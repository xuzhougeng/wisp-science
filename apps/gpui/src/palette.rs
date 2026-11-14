//! The command palette's state: what is listed for a query, in which order,
//! and which row the keyboard is on. It holds no graphics state. Results
//! arrive from the host out of order, so every search carries a ticket and
//! only the latest one may fill the list.
use wisp_dto::{ProjectSummary, SessionSearchInfo};
use wisp_server::desktop::{FileMatch, ReferenceOption};

use crate::home::Screen;

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
    Calendar,
    NewWindow,
    Capabilities,
    Trajectory,
    Aside,
}

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Command::NewConversation => "New conversation",
            Command::OpenFolder => "Open folder…",
            Command::Home => "All projects",
            Command::Files => "Show or hide Files",
            Command::Runs => "Show runs",
            Command::Artifacts => "Show artifacts",
            Command::Memos => "Show memos",
            Command::Memory => "Show memory",
            Command::Highlights => "Show highlights",
            Command::Favorites => "Favorites",
            Command::Archived => "Show or hide archived conversations",
            Command::Refresh => "Refresh",
            Command::Settings => "Settings",
            Command::Docs => "Documentation",
            Command::Feedback => "Report a problem",
            Command::ProjectSettings => "Project settings",
            Command::Find => "Find in this conversation",
            Command::Journey => "Research journey",
            Command::Calendar => "Research calendar",
            Command::NewWindow => "New window",
            Command::Capabilities => "Capabilities of this project",
            Command::Trajectory => "Trajectory of this conversation",
            Command::Aside => "Ask aside about this conversation",
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
            Command::Calendar => "calendar",
            Command::NewWindow => "dock",
            Command::Capabilities => "sparkles",
            Command::Trajectory => "gauge",
            Command::Aside => "circle-help",
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
            | Command::Capabilities
            | Command::Trajectory
            | Command::Aside => screen == Screen::Project,
            Command::OpenFolder
            | Command::Refresh
            | Command::Settings
            | Command::Favorites
            | Command::Docs
            | Command::Feedback
            | Command::Calendar
            | Command::NewWindow => true,
        }
    }

    const ALL: [Command; 23] = [
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
        Command::Calendar,
        Command::NewWindow,
        Command::Capabilities,
        Command::Trajectory,
        Command::Aside,
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
            Section::Projects => "Projects",
            Section::Files => "Files",
            Section::Conversations => "Conversations",
            Section::Skills => "Skills",
            Section::Commands => "Commands",
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
                    title: row.title.clone(),
                    place: if row.shelved {
                        format!("{} · Archived", row.project_name)
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
        assert_eq!(
            sections(&home),
            [
                Section::Projects,
                Section::Projects,
                Section::Conversations,
                Section::Commands,
                Section::Commands,
                Section::Commands,
                Section::Commands,
                Section::Commands,
                Section::Commands,
                Section::Commands,
                Section::Commands
            ]
        );
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
