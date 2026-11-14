mod aside_view;
mod calendar_view;
mod capabilities_view;
mod conversation_actions;
mod favorites_view;
mod files_panel;
mod find_view;
mod follow_ups_view;
mod home_view;
mod inline_view;
mod journey_view;
mod memory_view;
mod memos_view;
mod model_view_dialog;
mod notebook_view;
mod organise_view;
mod palette_view;
mod panel_tabs;
mod project_settings;
mod queue_view;
mod review_view;
mod settings_data;
mod settings_environments;
mod settings_mcp;
mod settings_network;
mod settings_sections;
mod settings_view;
mod shell;
mod trajectory_view;
mod transfer_view;

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, StyledExt,
    input::{InputEvent, InputState},
};
use serde_json::{Value, json};
use wisp_dto::{
    ProjectSummary, SessionSearchInfo,
    native_conversations::{Item, Snapshot},
};

use wisp_dto::native_conversations::SendRequest;
use wisp_server::desktop::{
    ArtifactRow, ConversationOptions, ConversationStatus, Favorite, Folder, LibraryItem,
    LibraryItemSummary, MemoryView, ProjectMemo, TurnRef,
};

use crate::{
    backend::{Backend, receive},
    compose::{self, Chip, Pending},
    controls,
    files::FilesState,
    home::{self, Screen},
    layers::{EscapeStack, Layer, Shortcut, is_escape, shortcut},
    organise::{self, Arrangement, Picks},
    palette::{self, PaletteState},
    panel::{self, PanelTab},
    present::{self, Tone},
    state::WorkspaceState,
};

pub struct Workspace {
    backend: Backend,
    state: WorkspaceState,
    /// Home page or one project, under whatever layers are open.
    screen: Screen,
    projects: Vec<ProjectSummary>,
    /// Newest conversations in any project, for the home page.
    recent: Vec<SessionSearchInfo>,
    /// Project waiting for the remove confirmation on the home page.
    removing: Option<String>,
    /// The second, stronger question: delete the project's folder too.
    removing_folder: Option<home::FolderQuestion>,
    delete_name_input: Entity<InputState>,
    /// Tool rows whose input and output are unfolded.
    open_tools: HashSet<String>,
    /// Files and references attached to the message being written, per
    /// conversation, like the drafts.
    chips: HashMap<String, Pending>,
    /// Messages written while a turn runs, waiting for it to end.
    queue: crate::queue::Queue,
    /// Options, usage and remembered approvals of the open conversation.
    status: Option<ConversationStatus>,
    /// Counts snapshot polls, so the status is refreshed about once a second.
    status_tick: u8,
    /// Transcript row whose text was just copied, for a moment of feedback.
    copied: Option<usize>,
    palette: PaletteState,
    palette_input: Entity<InputState>,
    /// The overview of what the open project can draw on, once read, and
    /// why it could not be.
    capabilities: Option<wisp_server::desktop::Capabilities>,
    capabilities_error: Option<String>,
    model_view: Option<wisp_server::desktop::ModelView>,
    model_view_error: Option<String>,
    /// The rows opened to show their whole text.
    model_view_open: HashSet<usize>,
    /// The review the dialog is showing or waiting for.
    review: Option<crate::review::Asked>,
    trajectory: Option<wisp_dto::TrajectorySnapshotDto>,
    trajectory_error: Option<String>,
    /// The steps opened to show their whole text: (turn, step).
    trajectory_open: HashSet<(usize, usize)>,
    /// The calendar across projects: the month shown, every project's
    /// records once read, the counter that drops late answers, the day
    /// that is chosen, and the projects left out.
    calendar_month: Option<crate::journey::Month>,
    calendar: Option<Vec<wisp_dto::ResearchCalendarProject>>,
    calendar_request: u64,
    calendar_day: Option<u32>,
    calendar_hidden: HashSet<String>,
    /// The month of the open project's history that is shown, its records
    /// once read, the counter that drops late answers, the filter field,
    /// and the form for a record of the researcher's own (title, details)
    /// with the kind chosen for it.
    journey_month: Option<crate::journey::Month>,
    journey: Option<wisp_dto::ResearchJourney>,
    journey_request: u64,
    journey_search: Entity<InputState>,
    journal_fields: [Entity<InputState>; 2],
    journal_category: &'static str,
    /// What the Files panel's name field is for, and the field; the entry
    /// the trash question is about; why the last change was refused; and
    /// where the last trashed entry went.
    file_naming: Option<crate::files::Naming>,
    file_name_input: Entity<InputState>,
    file_trashing: Option<String>,
    file_problem: Option<String>,
    file_notice: Option<String>,
    /// What is being looked for in the open conversation, and its field.
    find: crate::find::Find,
    find_input: Entity<InputState>,
    aside_input: Entity<InputState>,
    /// What was asked aside, by conversation. Kept until the app closes.
    asides: HashMap<String, crate::aside::Thread>,
    palette_scroll: ScrollHandle,
    sessions: Vec<SessionSearchInfo>,
    /// Conversations in any project that are waiting for the user.
    inbox: Vec<SessionSearchInfo>,
    /// Rows seen in any list, so the open conversation keeps its title and
    /// state while a search or the archive view does not include it.
    known: HashMap<String, SessionSearchInfo>,
    pinned: Vec<String>,
    show_archived: bool,
    /// Only the latest list request may replace the rows on screen.
    list_request: u64,
    /// How the conversation list is sorted and sectioned.
    arrangement: Arrangement,
    /// The project's groups, in the user's order.
    folders: Vec<Folder>,
    /// Conversation id to the id of its group.
    grouped: HashMap<String, String>,
    /// Branch id to the id of the conversation it was branched from.
    branched: HashMap<String, String>,
    /// The message the rewind question is about.
    rewind_turn: Option<TurnRef>,
    /// Keys of the sections that are folded.
    collapsed: HashSet<String>,
    /// Conversations picked in select mode.
    picks: Picks,
    /// The group whose actions, name field or delete question is showing.
    /// `None` while the name field is open means a new group.
    folder_target: Option<String>,
    folder_name: Entity<InputState>,
    /// The transfer menu copies instead of moving.
    transfer_copy: bool,
    /// Something that went as asked and is worth a line.
    notice: Option<String>,
    /// The project's memos; `None` until they were read.
    memos: Option<Vec<ProjectMemo>>,
    /// Only the latest memo request may fill the list.
    memos_request: u64,
    /// The memo being edited or asked about; `None` in the editor means a
    /// new one.
    memo_target: Option<String>,
    memo_title: Entity<InputState>,
    memo_body: Entity<InputState>,
    /// What is remembered, as the Memory tab shows it; `None` until read.
    memory: Option<MemoryView>,
    /// Only the latest memory request may fill the tab.
    memory_request: u64,
    /// What the memory editor is writing, or the delete question is about.
    memory_target: Option<crate::memory::Target>,
    memory_text: Entity<InputState>,
    /// Which palette the window wears.
    appearance: crate::theme::Appearance,
    /// What the app opens on.
    launch: crate::settings::Launch,
    /// The Settings page that is showing.
    settings_section: crate::settings::Section,
    /// How turns run and new conversations start; `None` until read.
    turn_defaults: Option<wisp_server::desktop::TurnDefaults>,
    steps_input: Entity<InputState>,
    /// Tools that are always allowed, project by project.
    grants: Vec<wisp_server::desktop::ProjectGrants>,
    /// What went wrong on a Settings page other than Models.
    settings_error: Option<String>,
    /// Every skill there is, as the Skills page lists them; `None` until read.
    skills: Option<Vec<wisp_server::desktop::SkillRow>>,
    skills_search: Entity<InputState>,
    /// The field that narrows the list of Settings pages.
    settings_search: Entity<InputState>,
    /// The bundled connectors, as the Connections page lists them.
    connectors: Option<Vec<wisp_server::desktop::ConnectorRow>>,
    connectors_search: Entity<InputState>,
    /// The connector whose tools and links are unfolded.
    open_connector: Option<String>,
    /// The SSH hosts and the default place for new work; `None` until read.
    environments: Option<wisp_server::desktop::EnvironmentsView>,
    /// What the Network page last read from the host, its fields, and
    /// whether the last Save was kept.
    network: Option<wisp_dto::NetworkSettings>,
    network_fields: [Entity<InputState>; 6],
    network_saved: bool,
    /// The host the form changes or the remove question is about; `None`
    /// in the form means a new one.
    env_target: Option<String>,
    /// The hosts being checked right now.
    env_checking: std::collections::HashSet<String>,
    /// The project whose settings dialog is open; its fields (name,
    /// description, rules); whether they hold what is saved yet; and why
    /// the last Save was refused.
    project_form: Option<String>,
    project_fields: [Entity<InputState>; 3],
    project_form_ready: bool,
    project_form_error: Option<String>,
    /// The host form's fields: name, address, user, port, key file, jump
    /// host, notes.
    env_fields: [Entity<InputState>; 7],
    /// The user's MCP servers; `None` until read.
    mcp_servers: Option<Vec<wisp_server::desktop::McpServerRow>>,
    /// The server the form changes or the remove question is about; `None`
    /// in the form means a new one.
    mcp_editing: Option<String>,
    /// The form is for a server over HTTP rather than a local program.
    mcp_http: bool,
    mcp_name: Entity<InputState>,
    mcp_target: Entity<InputState>,
    mcp_token: Entity<InputState>,
    /// What testing a server found: `None` while it runs, then its tools
    /// or why it failed.
    mcp_tested: HashMap<String, Option<Result<Vec<String>, String>>>,
    /// The service keys, as the Credentials page lists them, each with the
    /// field a new value is typed into.
    credentials: Vec<(wisp_server::desktop::CredentialRow, Entity<InputState>)>,
    /// Tokens used, as the Usage page shows them; `None` until read.
    usage: Option<wisp_server::desktop::UsageView>,
    /// What Wisp keeps and where; `None` until read.
    storage: Option<wisp_server::desktop::StorageView>,
    /// What the open conversation saved to the favorites, newest first.
    saved: Vec<LibraryItem>,
    saved_request: u64,
    /// The Favorites page's list; `None` until it was read.
    favorites: Option<Vec<LibraryItemSummary>>,
    favorites_request: u64,
    favorites_filter: crate::favorites::Filter,
    favorites_search: Entity<InputState>,
    /// The item open on the Favorites page, and its picture if it has one.
    favorite: Option<Favorite>,
    favorite_image: Option<std::sync::Arc<Image>>,
    /// Pictures the open project's answers point at, by path. `None` is a
    /// picture that is being read or could not be.
    inline_images: HashMap<String, Option<std::sync::Arc<Image>>>,
    /// The picture being previewed in the Files panel, with its path.
    file_image: Option<(String, std::sync::Arc<Image>)>,
    favorite_request: u64,
    rename: Entity<InputState>,
    settings: settings_view::SettingsForm,
    composer: Entity<InputState>,
    search: Entity<InputState>,
    rows: Vec<Item>,
    history: Vec<Item>,
    before: Option<i64>,
    list: ListState,
    /// Open dismissible surfaces; the only place that decides what Escape closes.
    layers: EscapeStack,
    files: FilesState,
    file_view: Entity<InputState>,
    expanded_run: Option<String>,
    /// Which tab the side panel shows.
    panel_tab: PanelTab,
    artifacts: Vec<ArtifactRow>,
    /// The Artifacts tab lists the whole project instead of the conversation.
    artifacts_all: bool,
    /// Only the latest artifact request may fill the list.
    artifacts_request: u64,
    ready: bool,
    pending: bool,
    loading_history: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
    poll: Option<Task<()>>,
    watching: bool,
    watch: Option<Task<()>>,
}

fn tone_color(tone: Tone, cx: &App) -> Hsla {
    match tone {
        Tone::Neutral => cx.theme().muted_foreground,
        Tone::Active => cx.theme().info,
        Tone::Success => cx.theme().success,
        Tone::Danger => cx.theme().danger,
        Tone::Attention => cx.theme().warning,
    }
}

fn chip(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .text_xs()
        .text_color(tone_color(tone, cx))
        .child(label.into())
}

impl Workspace {
    pub fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 9)
                .placeholder(if cfg!(target_os = "macos") {
                    "Describe your research task…  ⌘Return to send"
                } else {
                    "Describe your research task…  Ctrl+Enter to send"
                })
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search conversations…"));
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Conversation title"));
        let folder_name = cx.new(|cx| InputState::new(window, cx).placeholder("Group name"));
        let delete_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type the project's name"));
        let memo_title = cx.new(|cx| InputState::new(window, cx).placeholder("Title (optional)"));
        let memo_body = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("A prompt you reuse, a server address, a reminder…")
        });
        let settings = settings_view::SettingsForm::new(window, cx);
        let palette_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search conversations, projects and files…")
        });
        // In-editor search would be a nested surface outside the Escape stack.
        let file_view = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .searchable(false)
        });
        let memory_text = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        let steps_input = cx.new(|cx| InputState::new(window, cx));
        let mcp_name = cx.new(|cx| InputState::new(window, cx).placeholder("Lab notebook"));
        let env_fields = ["gpu", "10.0.0.8", "", "22", "~/.ssh/id_ed25519", "", ""]
            .map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let network_fields = [
            "http://127.0.0.1:7890",
            "http://127.0.0.1:7890",
            "http://127.0.0.1:7890",
            "https://conda.example.org/conda-forge",
            "https://pypi.example.org/simple",
            "/etc/ssl/certs/lab-ca.pem",
        ]
        .map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let project_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .placeholder("Always report effect sizes with confidence intervals.")
            }),
        ];
        let find_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find in this conversation…"));
        let file_name_input = cx.new(|cx| InputState::new(window, cx));
        let aside_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Ask about this conversation…"));
        let journey_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter this month…"));
        let journal_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("The reads are clean")),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let mcp_target = cx.new(|cx| InputState::new(window, cx));
        let mcp_token = cx.new(|cx| InputState::new(window, cx).masked(true));
        let skills_search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a skill…"));
        let settings_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        let connectors_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a connector or a tool…"));
        let favorites_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search your favorites…"));
        let handle = window.window_handle();
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.state
                        .save_draft(this.composer.read(cx).value().to_string());
                    cx.notify();
                }
                // Let the input component and platform IME consume composition.
                // Enter inserts a newline; platform secondary+Enter sends.
                if matches!(event, InputEvent::PressEnter { secondary: true }) {
                    this.send(window, cx);
                }
            }),
            // Search runs in the host: titles and message text, archive included.
            cx.subscribe_in(&search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_sessions(window, cx);
                }
            }),
            cx.subscribe_in(
                &palette_input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => this.search_palette(window, cx),
                    InputEvent::PressEnter { .. } => this.activate_palette(None, window, cx),
                    _ => {}
                },
            ),
            // Filtering is done as the page is drawn.
            cx.subscribe_in(&journey_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&aside_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.ask_aside(window, cx);
                }
            }),
            cx.subscribe_in(&file_name_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_file_name(window, cx);
                }
            }),
            cx.subscribe_in(&find_input, window, |this, _, event, _, cx| match event {
                InputEvent::Change => this.search_find(cx),
                // Return goes back through the conversation; with the
                // modifier it goes forward.
                InputEvent::PressEnter { secondary } => this.step_find(!secondary, cx),
                _ => {}
            }),
            cx.subscribe_in(&rename, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_rename(window, cx);
                }
            }),
            // Filtering is done here, over the list already read.
            cx.subscribe_in(
                &settings_search,
                window,
                |this, _, event, window, cx| match event {
                    // The list is filtered as it is drawn.
                    InputEvent::Change => cx.notify(),
                    // Return opens the first page that is left.
                    InputEvent::PressEnter { .. } => {
                        let query = this.settings_search.read(cx).value().to_string();
                        if let Some(first) = crate::settings::search_sections(&query).first() {
                            this.show_settings_section(*first, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&skills_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&connectors_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&steps_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_steps(window, cx);
                }
            }),
            cx.subscribe_in(&favorites_search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_favorites(window, cx);
                }
            }),
            cx.subscribe_in(&delete_name_input, window, |this, _, event, window, cx| {
                match event {
                    // The button follows what is typed.
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => this.delete_project_folder(window, cx),
                    _ => {}
                }
            }),
            cx.subscribe_in(&folder_name, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_folder_name(window, cx);
                }
            }),
            // Window-level and ahead of key dispatch: Escape closes the topmost
            // layer wherever focus is, including nowhere. Platform IME handling
            // runs earlier still, so cancelling a composition is not seen here.
            cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle() != handle {
                    return;
                }
                let key = &event.keystroke.key;
                let held = event.keystroke.modifiers;
                let handled = if is_escape(key, held.modified()) {
                    weak.update(cx, |this, cx| this.dismiss_top(window, cx))
                } else if let Some(delta) = palette::step(key, held.modified()) {
                    // Arrow keys belong to the palette only while it is open.
                    weak.update(cx, |this, cx| this.step_palette(delta, cx))
                } else {
                    // Whatever is held besides the platform's command key.
                    let others = held.shift
                        || held.alt
                        || held.function
                        || if cfg!(target_os = "macos") {
                            held.control
                        } else {
                            held.platform
                        };
                    match shortcut(key, held.secondary(), others) {
                        Some(command) => {
                            weak.update(cx, |this, cx| this.run_shortcut(command, window, cx))
                        }
                        None => return,
                    }
                };
                if handled.unwrap_or(false) {
                    cx.stop_propagation();
                }
            }),
            // A system change only matters while the system is followed.
            cx.observe_window_appearance(window, |this, window, cx| {
                crate::theme::apply(this.appearance, window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| this.mark_seen(window, cx)),
        ];
        let mut view = Self {
            backend,
            state: WorkspaceState::default(),
            screen: Screen::default(),
            projects: vec![],
            recent: vec![],
            removing: None,
            removing_folder: None,
            delete_name_input,
            open_tools: HashSet::new(),
            chips: HashMap::new(),
            queue: Default::default(),
            status: None,
            status_tick: 0,
            copied: None,
            palette: PaletteState::default(),
            palette_input,
            find: Default::default(),
            find_input,
            aside_input,
            asides: HashMap::new(),
            capabilities: None,
            capabilities_error: None,
            model_view: None,
            model_view_error: None,
            model_view_open: HashSet::new(),
            review: None,
            trajectory: None,
            trajectory_error: None,
            trajectory_open: HashSet::new(),
            calendar_month: None,
            calendar: None,
            calendar_request: 0,
            calendar_day: None,
            calendar_hidden: HashSet::new(),
            journey_month: None,
            journey: None,
            journey_request: 0,
            journey_search,
            journal_fields,
            journal_category: "progress",
            file_naming: None,
            file_name_input,
            file_trashing: None,
            file_problem: None,
            file_notice: None,
            palette_scroll: ScrollHandle::new(),
            sessions: vec![],
            inbox: vec![],
            known: HashMap::new(),
            pinned: vec![],
            show_archived: false,
            list_request: 0,
            arrangement: Arrangement::default(),
            folders: vec![],
            grouped: HashMap::new(),
            branched: HashMap::new(),
            rewind_turn: None,
            collapsed: HashSet::new(),
            picks: Picks::default(),
            folder_target: None,
            folder_name,
            transfer_copy: false,
            notice: None,
            memos: None,
            memos_request: 0,
            memo_target: None,
            memo_title,
            memo_body,
            memory: None,
            memory_request: 0,
            memory_target: None,
            memory_text,
            appearance: Default::default(),
            launch: Default::default(),
            settings_section: Default::default(),
            turn_defaults: None,
            steps_input,
            grants: vec![],
            settings_error: None,
            skills: None,
            skills_search,
            settings_search,
            connectors: None,
            connectors_search,
            open_connector: None,
            environments: None,
            network: None,
            network_fields,
            network_saved: false,
            env_target: None,
            env_checking: Default::default(),
            project_form: None,
            project_fields,
            project_form_ready: false,
            project_form_error: None,
            env_fields,
            mcp_servers: None,
            mcp_editing: None,
            mcp_http: true,
            mcp_name,
            mcp_target,
            mcp_token,
            mcp_tested: HashMap::new(),
            credentials: vec![],
            usage: None,
            storage: None,
            saved: vec![],
            saved_request: 0,
            favorites: None,
            favorites_request: 0,
            favorites_filter: Default::default(),
            favorites_search,
            favorite: None,
            favorite_image: None,
            file_image: None,
            inline_images: HashMap::new(),
            favorite_request: 0,
            rename,
            settings,
            composer,
            search,
            rows: vec![],
            history: vec![],
            before: None,
            list: ListState::new(0, ListAlignment::Bottom, px(300.)),
            layers: EscapeStack::default(),
            files: FilesState::default(),
            file_view,
            expanded_run: None,
            panel_tab: PanelTab::default(),
            artifacts: vec![],
            artifacts_all: false,
            artifacts_request: 0,
            ready: false,
            pending: false,
            loading_history: false,
            error: None,
            _subscriptions: subscriptions,
            poll: None,
            watching: false,
            watch: None,
        };
        view.initialize(window, cx);
        view
    }

    /// Closes the topmost open layer. Returns false when nothing was open, so
    /// the key continues to whatever else handles Escape.
    fn dismiss_top(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.layers.dismiss_top() {
            Some(Layer::FilePreview) => {
                self.files.close_preview();
                self.file_image = None;
                self.restore_focus(window, cx);
                // A file revealed from the palette has no listing under it yet.
                self.ensure_files(window, cx);
            }
            Some(Layer::FilesPanel) => self.restore_focus(window, cx),
            // These layers own the focused field, which goes away with them.
            Some(Layer::Settings | Layer::Rename | Layer::ConfirmDelete) => {
                self.focus_composer(window, cx)
            }
            Some(Layer::ConfirmRemoveProject) => {
                self.removing = None;
                self.removing_folder = None;
            }
            Some(
                Layer::Palette
                | Layer::ConfirmFullAccess
                | Layer::EffortMenu
                | Layer::ModelMenu
                | Layer::RunsOnMenu,
            ) => self.focus_composer(window, cx),
            // Settings stays open underneath and keeps its focus.
            Some(Layer::ConfirmDeleteModel) => {}
            Some(Layer::SelectMode) => self.picks.clear(),
            Some(Layer::FolderName) => {
                self.folder_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::FolderMenu | Layer::ConfirmDeleteFolder) => self.folder_target = None,
            Some(Layer::ConfirmRewind) => self.rewind_turn = None,
            Some(Layer::MemoEditor) => {
                self.memo_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ConfirmDeleteMemo) => self.memo_target = None,
            Some(Layer::MemoryEditor) => {
                self.memory_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ConfirmDeleteMemory) => self.memory_target = None,
            Some(Layer::Favorites) => {
                self.favorite = None;
                self.favorite_image = None;
                self.focus_composer(window, cx);
            }
            // The page stays open underneath and keeps its focus.
            Some(Layer::ConfirmRemoveFavorite) => {}
            Some(Layer::McpForm | Layer::ConfirmRemoveMcp) => self.mcp_editing = None,
            Some(Layer::EnvironmentForm | Layer::ConfirmRemoveEnvironment) => {
                self.env_target = None
            }
            Some(Layer::ProjectSettings) => {
                self.project_form = None;
                self.project_form_error = None;
            }
            Some(Layer::Find) => {
                self.find.clear();
                self.focus_composer(window, cx);
            }
            Some(Layer::FileName) => {
                self.file_naming = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Outline) => self.focus_composer(window, cx),
            Some(Layer::Journey) => {
                self.journey = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Calendar) => {
                self.calendar = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Capabilities) => {
                self.capabilities = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Trajectory) => {
                self.clear_trajectory();
                self.focus_composer(window, cx);
            }
            Some(Layer::Review) => {
                self.review = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ModelView) => {
                self.clear_model_view();
                self.focus_composer(window, cx);
            }
            // The page stays open underneath.
            Some(Layer::JournalForm) => {}
            Some(Layer::ConfirmTrashFile) => self.file_trashing = None,
            // Menus and questions that hold no focus of their own.
            Some(
                Layer::SortMenu
                | Layer::BatchGroupMenu
                | Layer::ConfirmBatchDelete
                | Layer::GroupMenu
                | Layer::TransferMenu,
            ) => {}
            None => return false,
        }
        cx.notify();
        true
    }

    /// Runs a window-wide shortcut. Returns false when it does not apply
    /// here, so the key reaches whatever else is listening.
    fn run_shortcut(
        &mut self,
        command: Shortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.ready {
            return false;
        }
        match command {
            Shortcut::Settings => {
                self.layers.close(Layer::Palette);
                if !self.layers.contains(Layer::Settings) {
                    self.open_settings(window, cx);
                }
                true
            }
            Shortcut::Palette => {
                self.toggle_palette(window, cx);
                true
            }
            Shortcut::Find => {
                // Only where a conversation is what is showing.
                if self.screen != Screen::Project
                    || self.state.session.is_none()
                    || self.layers.contains(Layer::Settings)
                    || self.layers.contains(Layer::Calendar)
                    || self.layers.contains(Layer::Journey)
                    || self.layers.contains(Layer::Favorites)
                    || self.layers.contains(Layer::Palette)
                    || self.layers.contains(Layer::ProjectSettings)
                {
                    return false;
                }
                self.open_find(window, cx);
                true
            }
            Shortcut::NewConversation => {
                if self.screen != Screen::Project
                    || self.layers.contains(Layer::Settings)
                    || self.layers.contains(Layer::Calendar)
                    || self.layers.contains(Layer::Journey)
                    || self.layers.contains(Layer::Favorites)
                    || self.layers.contains(Layer::Palette)
                {
                    return false;
                }
                self.create_session(window, cx);
                true
            }
        }
    }

    /// A closed layer takes its focused element with it; keep typing possible.
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_view.read(cx).focus_handle(cx).is_focused(window) {
            self.focus_composer(window, cx);
        }
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Changes the selection. File state belongs to a project, so a preview
    /// from the previous project must not stay open over the new one.
    fn choose(&mut self, project: Option<String>, session: Option<String>) {
        if self.state.project != project {
            self.layers.close(Layer::FilePreview);
            // The list, its groups and the picks belong to the project.
            self.layers.close(Layer::SelectMode);
            for layer in Layer::SIDEBAR {
                self.layers.close(layer);
            }
            self.picks.clear();
            self.folders.clear();
            self.grouped.clear();
            self.branched.clear();
            self.folder_target = None;
            self.notice = None;
            // Memos are the project's, and so is an edit in progress.
            self.layers.close(Layer::MemoEditor);
            self.layers.close(Layer::ConfirmDeleteMemo);
            self.memos = None;
            self.memo_target = None;
            self.layers.close(Layer::MemoryEditor);
            self.layers.close(Layer::ConfirmDeleteMemory);
            self.memory = None;
            self.memory_target = None;
        }
        // These act on the conversation that is being left.
        for layer in [
            Layer::Rename,
            Layer::ConfirmDelete,
            Layer::GroupMenu,
            Layer::TransferMenu,
            Layer::ConfirmRewind,
            Layer::ConfirmFullAccess,
            Layer::EffortMenu,
            Layer::ModelMenu,
            Layer::RunsOnMenu,
            Layer::Find,
            Layer::Outline,
            Layer::Trajectory,
            Layer::Review,
            Layer::ModelView,
        ] {
            self.layers.close(layer);
        }
        self.clear_trajectory();
        self.clear_model_view();
        self.review = None;
        self.find.clear();
        if self.state.session != session {
            self.saved.clear();
            self.rewind_turn = None;
            self.status = None;
            self.copied = None;
            // The list belonged to the conversation, or project, being left.
            self.artifacts.clear();
        }
        if self.state.project != project {
            // Paths are the project's own; another project's mean others.
            self.inline_images.clear();
        }
        self.files.set_project(project.as_deref());
        self.state.select(project, session);
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = true;
        self.error = None;
        let reply = self.backend.initialize();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.ready = true;
                        this.load_projects(window, cx);
                        this.load_recent(window, cx);
                        this.load_inbox(window, cx);
                        this.load_model_settings(true, window, cx);
                        this.load_arrangement(window, cx);
                        this.load_general(window, cx);
                        // The composer offers the saved hosts.
                        this.load_environments(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self
            .backend
            .request::<Vec<ProjectSummary>>("list_projects", None, json!({}));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(projects) => this.projects = home::ordered(projects),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.recent_conversations(12);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(recent) => this.recent = recent,
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a project from the home page.
    fn enter_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Project;
        self.layers.close(Layer::ConfirmRemoveProject);
        self.removing = None;
        self.removing_folder = None;
        self.select_project(id, window, cx);
        cx.notify();
    }

    /// Leaves the project for the home page. Turns keep running in the host.
    fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.layers.close(Layer::FilesPanel);
        self.choose(None, None);
        self.sessions.clear();
        self.pinned.clear();
        self.show_archived = false;
        self.clear_transcript();
        for input in [&self.composer, &self.search] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.screen = Screen::Home;
        self.refresh_lists(window, cx);
        cx.notify();
    }

    fn toggle_star(
        &mut self,
        id: String,
        starred: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Shown at once; the reload below confirms or corrects it.
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            project.starred = starred;
        }
        self.projects = home::ordered(std::mem::take(&mut self.projects));
        let reply = self.backend.star_project(id, starred);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_projects(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ask_remove_project(&mut self, id: String, cx: &mut Context<Self>) {
        self.error = None;
        self.removing = Some(id);
        self.removing_folder = None;
        self.layers.open(Layer::ConfirmRemoveProject);
        cx.notify();
    }

    fn cancel_remove_project(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ConfirmRemoveProject) {
            self.removing = None;
            self.removing_folder = None;
            cx.notify();
        }
    }

    /// Turns the question into the stronger one: the folder goes too. The
    /// host says what is in it, or why it may not be deleted.
    fn ask_delete_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.removing.clone() else {
            return;
        };
        self.removing_folder = Some(home::FolderQuestion::Looking);
        self.delete_name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.backend.project_folder(id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Cancelled, or asked about another project, meanwhile.
                if this.removing.as_ref() != Some(&id)
                    || this.removing_folder != Some(home::FolderQuestion::Looking)
                {
                    return;
                }
                this.removing_folder = Some(match result {
                    Ok(folder) => {
                        this.delete_name_input
                            .update(cx, |input, cx| input.focus(window, cx));
                        home::FolderQuestion::Ask(folder)
                    }
                    Err(reason) => home::FolderQuestion::Refused(reason),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Deletes the project and its folder, once its name has been typed.
    fn delete_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::ConfirmRemoveProject) {
            return;
        }
        let (Some(id), Some(home::FolderQuestion::Ask(folder))) =
            (self.removing.clone(), self.removing_folder.clone())
        else {
            return;
        };
        let typed = self.delete_name_input.read(cx).value().to_string();
        if !home::name_typed(&typed, &folder) {
            return;
        }
        self.pending = true;
        self.error = None;
        let reply = self.backend.delete_project_and_folder(id, typed);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                this.layers.close(Layer::ConfirmRemoveProject);
                this.removing = None;
                this.removing_folder = None;
                match result {
                    Ok(()) => {
                        this.notice = Some(format!("Deleted \"{}\" and its folder.", folder.name));
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::ConfirmRemoveProject) {
            return;
        }
        let Some(id) = self.removing.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let reply = self.backend.remove_project(id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                this.layers.close(Layer::ConfirmRemoveProject);
                this.removing = None;
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Folds or unfolds one tool row and has the list measure it again.
    fn toggle_tool(&mut self, key: String, index: usize, cx: &mut Context<Self>) {
        if !self.open_tools.remove(&key) {
            self.open_tools.insert(key);
        }
        if index < self.rows.len() {
            self.list.splice(index..index + 1, 1);
        }
        cx.notify();
    }

    fn load_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.request::<Vec<SessionSearchInfo>>(
            "native_conversation_inbox",
            None,
            json!({}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(inbox) => this.inbox = inbox,
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_projects(window, cx);
        self.load_recent(window, cx);
        self.load_sessions(window, cx);
        self.load_inbox(window, cx);
    }

    fn select_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.choose(Some(id), None);
        self.sessions.clear();
        self.pinned.clear();
        self.show_archived = false;
        self.clear_transcript();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.load_sessions(window, cx);
    }

    fn clear_transcript(&mut self) {
        self.poll = None;
        self.rows.clear();
        self.history.clear();
        self.before = None;
        self.loading_history = false;
        self.error = None;
        self.expanded_run = None;
        self.open_tools.clear();
        self.list.reset(0);
    }

    fn load_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let query = self.search.read(cx).value().to_string();
        let archived = self.show_archived;
        // Search results and the archive are for looking, not for landing in.
        let browsing = query.trim().is_empty() && !archived;
        self.list_request += 1;
        let request = self.list_request;
        let reply = self.backend.conversations(project, query, archived);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.list_request != request {
                    return;
                }
                match result {
                    Ok(list) => {
                        for row in &list.rows {
                            this.known.insert(row.id.clone(), row.clone());
                        }
                        this.sessions = present::pinned_first(list.rows, &list.pinned);
                        this.pinned = list.pinned;
                        this.folders = list.folders;
                        this.grouped = list.grouped;
                        this.branched = list.branched;
                        this.picks.keep(&this.sessions);
                        if browsing && this.state.session.is_none() && !this.pending {
                            // The first one on screen, whatever the sections.
                            let shown = organise::visible(&this.session_rows(cx), &this.sessions);
                            if let Some(first) = shown.into_iter().next() {
                                this.select_session(first, window, cx);
                            }
                        }
                        this.mark_seen(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_session(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.choose(self.state.project.clone(), Some(id));
        self.clear_transcript();
        let draft = self.state.draft();
        self.composer.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.start_poll(window, cx);
        self.load_status(window, cx);
        self.load_saved(window, cx);
        self.ensure_files(window, cx);
        self.reload_artifacts(window, cx);
        self.remember_place(window, cx);
    }

    /// Refreshes the Artifacts tab if that is what the panel is showing.
    fn reload_artifacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Artifacts {
            self.load_artifacts(window, cx);
        }
    }

    /// Opens a conversation from any project, as the inbox needs.
    fn open_session(
        &mut self,
        project: String,
        session: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.screen = Screen::Project;
        if self.state.project.as_ref() != Some(&project) {
            self.state
                .save_draft(self.composer.read(cx).value().to_string());
            self.choose(Some(project), None);
            self.sessions.clear();
            self.pinned.clear();
            self.show_archived = false;
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.select_session(session, window, cx);
        self.load_sessions(window, cx);
    }

    /// Marks the open conversation read once its answer is actually on screen.
    fn mark_seen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.state.session.clone() else {
            return;
        };
        let Some(row) = self.known.get_mut(&id) else {
            return;
        };
        if !present::should_mark_seen(
            &row.status,
            self.state.snapshot.as_ref(),
            window.is_window_active(),
        ) {
            return;
        }
        // Applied locally first so repeated refreshes do not resend it.
        row.status = "complete".into();
        if let Some(row) = self.sessions.iter_mut().find(|row| row.id == id) {
            row.status = "complete".into();
        }
        self.inbox.retain(|row| row.id != id);
        let selection = self.state.selection();
        let reply = self.backend.request::<Value>(
            "native_conversation_seen",
            selection.project.clone(),
            json!({"session_id": id}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.load_projects(window, cx);
                        this.load_inbox(window, cx);
                    }
                    Err(error) if this.state.current(&selection) => this.error = Some(error),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps project, conversation and inbox status fresh while any turn runs,
    /// including turns in conversations that are not on screen. Stops by itself
    /// once the host reports nothing running, after one final refresh.
    fn watch_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watching {
            return;
        }
        self.watching = true;
        let backend = self.backend.clone();
        self.watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let result = receive(backend.request::<Vec<ProjectSummary>>(
                    "list_projects",
                    None,
                    json!({}),
                ))
                .await;
                let busy = this
                    .update_in(cx, |this, window, cx| {
                        let busy = match result {
                            Ok(projects) => {
                                let busy = projects.iter().any(|p| p.running_count > 0);
                                this.projects = home::ordered(projects);
                                this.load_recent(window, cx);
                                this.load_sessions(window, cx);
                                this.load_inbox(window, cx);
                                busy
                            }
                            Err(error) => {
                                this.error = Some(error);
                                false
                            }
                        };
                        this.watching = busy;
                        cx.notify();
                        busy
                    })
                    .unwrap_or(false);
                if !busy {
                    break;
                }
            }
        }));
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        if selection.session.is_none() {
            return;
        }
        let backend = self.backend.clone();
        self.poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let result = receive(backend.request::<Snapshot>(
                    "native_conversation_snapshot",
                    selection.project.clone(),
                    json!({"session_id": selection.session}),
                ))
                .await;
                let next = this
                    .update_in(cx, |this, window, cx| {
                        if !this.state.current(&selection) {
                            return None;
                        }
                        match result {
                            Ok(snapshot) => {
                                let was_running =
                                    this.state.snapshot.as_ref().is_some_and(|s| s.running);
                                let running = snapshot.running;
                                let active_runs = panel::active_runs(&snapshot.run_cards);
                                if this.state.accept(&selection, snapshot) {
                                    this.update_rows();
                                    this.load_inline_images(window, cx);
                                    if running {
                                        this.watch_activity(window, cx);
                                        // Usage grows while the turn runs.
                                        this.status_tick = this.status_tick.wrapping_add(1);
                                        if this.status_tick % 7 == 0 {
                                            this.load_status(window, cx);
                                        }
                                    } else if was_running {
                                        this.load_status(window, cx);
                                        // The turn may have written files and
                                        // changed what needs attention.
                                        this.refresh_lists(window, cx);
                                        this.reload_files(window, cx);
                                        this.reload_artifacts(window, cx);
                                        // What could be asked next arrives
                                        // a little after the answer.
                                        this.await_follow_ups(window, cx);
                                    }
                                    this.mark_seen(window, cx);
                                    // The turn is over: what was written
                                    // meanwhile goes next.
                                    if !running {
                                        this.drain_queue(window, cx);
                                    }
                                    cx.notify();
                                }
                                panel::poll_delay(running, active_runs)
                            }
                            Err(error) => {
                                this.error = Some(error);
                                cx.notify();
                                None
                            }
                        }
                    })
                    .ok()
                    .flatten();
                let Some(delay) = next else {
                    break;
                };
                cx.background_executor().timer(delay).await;
            }
        }));
    }

    fn update_rows(&mut self) {
        let Some(snapshot) = &self.state.snapshot else {
            return;
        };
        if self.history.is_empty() {
            self.before = snapshot.next_before_seq;
        }
        let mut rows = self.history.clone();
        rows.extend(snapshot.items.clone());
        // Keep cached heights for unchanged rows. Streaming invalidates just its
        // changing suffix; GPUI lays out only visible rows and the overdraw area.
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| present::same_row(a, b))
            .count();
        self.list
            .splice(prefix..self.rows.len(), rows.len() - prefix);
        self.rows = rows;
        // The same words, in the transcript as it is now.
        if self.layers.contains(Layer::Find) {
            self.find.refresh(&self.rows);
        }
    }

    fn older(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(before) = self.before else {
            return;
        };
        if self.loading_history {
            return;
        }
        self.loading_history = true;
        let selection = self.state.selection();
        let reply = self.backend.request::<Snapshot>(
            "native_conversation_snapshot",
            selection.project.clone(),
            json!({"session_id":selection.session,"before_seq":before}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                this.loading_history = false;
                match result {
                    Ok(page) => {
                        let mut history = page.items;
                        history.append(&mut this.history);
                        this.history = history;
                        this.before = page.next_before_seq;
                        this.update_rows();
                        this.load_inline_images(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.state.project.is_none() {
            return;
        }
        self.pending = true;
        let selection = self.state.selection();
        // Through the desktop's own call, so the conversation starts the
        // way Settings says new ones should.
        let Some(project) = selection.project.clone() else {
            return;
        };
        let reply = self.backend.create_conversation(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(id) => {
                        this.select_session(id, window, cx);
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        // While a turn runs the message waits for it to end.
        if self.state.snapshot.as_ref().is_some_and(|s| s.running) {
            return self.enqueue(text, window, cx);
        }
        self.send_text(text, true, window, cx);
    }

    /// Sends `text` as the next message. An answer picked on a question card
    /// is sent as it is and leaves whatever is being typed alone.
    fn send_text(
        &mut self,
        text: String,
        from_composer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // What the composer has attached goes with what it has typed.
        let carried = match (&self.state.session, from_composer) {
            (Some(session), true) => self.chips.get(session).cloned().unwrap_or_default(),
            _ => Pending::default(),
        };
        self.deliver(text, carried, from_composer, None, window, cx);
    }

    /// Hands a message to the host. `queued` is the queue entry it was
    /// taken from, which goes back to the front if the host refuses it.
    fn deliver(
        &mut self,
        text: String,
        carried: Pending,
        from_composer: bool,
        queued: Option<crate::queue::Queued>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let attachments = carried.attachments();
        // Files alone are a message; the host names it after the first one.
        let sendable = if text.trim().is_empty() && !attachments.is_empty() {
            "files"
        } else {
            text.as_str()
        };
        if !self.state.can_send(sendable, self.pending) || self.loading_history {
            if let Some(queued) = queued {
                self.queue.put_back(&session, queued);
            }
            return;
        }
        self.pending = true;
        self.error = None;
        self.poll = None;
        let reply = self.backend.send_message(
            project,
            SendRequest {
                session_id: session.clone(),
                request_id: uuid::Uuid::new_v4().to_string(),
                message: text.trim().to_owned(),
                resume: false,
                resume_after: None,
                attachments,
                references: carried.references(),
                acp_agent_id: None,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // The turn runs whether or not its conversation is still open.
                if result.is_ok() {
                    this.watch_activity(window, cx);
                } else if let Some(queued) = queued {
                    // Refused: it keeps its place at the front of the queue.
                    this.queue.put_back(&session, queued);
                }
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(_) => {
                        // Clear only after durable acceptance. Failure preserves the draft.
                        if from_composer {
                            this.composer
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.state.save_draft(String::new());
                            this.chips.remove(&session);
                        }
                        this.history.clear();
                        if let Some(snapshot) = &mut this.state.snapshot {
                            snapshot.running = true;
                        }
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What is attached to the message being written in this conversation.
    fn attached(&self) -> &[Chip] {
        self.state
            .session
            .as_ref()
            .and_then(|session| self.chips.get(session))
            .map_or(&[], Pending::chips)
    }

    fn attach(&mut self, chip: Chip, cx: &mut Context<Self>) {
        if let Some(session) = self.state.session.clone() {
            self.chips.entry(session).or_default().add(chip);
            cx.notify();
        }
    }

    fn detach(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(pending) = self
            .state
            .session
            .as_ref()
            .and_then(|session| self.chips.get_mut(session))
        {
            pending.remove(index);
            cx.notify();
        }
    }

    /// Picks files with the system dialog. Files outside the project are
    /// copied into its `uploads` folder; files inside it are used in place.
    fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(_)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let picked = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match picked {
                    Some(files) if !files.is_empty() => {
                        receive(backend.import_attachments(project, files)).await
                    }
                    _ => Ok(vec![]),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attach_imported(&selection, result, window, cx)
            });
        })
        .detach();
    }

    /// Attaches files dropped onto the conversation, the way the picker
    /// does: outside files are copied into `uploads`, inside ones are used
    /// in place. Folders are left out, and it says so.
    pub(super) fn attach_dropped(
        &mut self,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(_)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let (files, left_out) = compose::dropped_files(paths, |path| path.is_file());
        self.error = compose::drop_note(left_out);
        if files.is_empty() {
            return cx.notify();
        }
        let reply = self.backend.import_attachments(project, files);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attach_imported(&selection, result, window, cx)
            });
        })
        .detach();
        cx.notify();
    }

    /// Puts imported files on the message being written.
    fn attach_imported(
        &mut self,
        selection: &crate::state::Selection,
        result: Result<Vec<String>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The files belong to the conversation they were picked for.
        if !self.state.current(selection) {
            return;
        }
        match result {
            Ok(paths) => {
                for path in paths {
                    self.attach(Chip::File(path), cx);
                }
                self.reload_files(window, cx);
                self.focus_composer(window, cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn load_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.conversation_status(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(status) => this.status = Some(status),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves the conversation's options; they apply from its next turn.
    fn set_options(
        &mut self,
        options: ConversationOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        self.error = None;
        let reply = self
            .backend
            .set_conversation_options(project, session, options);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match (result, &mut this.status) {
                    (Ok(options), Some(status)) => status.options = options,
                    (Ok(_), None) => {}
                    (Err(error), _) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn options(&self) -> ConversationOptions {
        self.status
            .as_ref()
            .map(|status| status.options.clone())
            .unwrap_or_default()
    }

    /// Turning full access on asks first; turning it off does not.
    fn toggle_full_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for menu in Layer::COMPOSER_MENUS {
            self.layers.close(menu);
        }
        if self.options().full_access {
            self.layers.close(Layer::ConfirmFullAccess);
            let options = ConversationOptions {
                full_access: false,
                ..self.options()
            };
            self.set_options(options, window, cx);
        } else if !self.layers.close(Layer::ConfirmFullAccess) {
            self.layers.open(Layer::ConfirmFullAccess);
        }
        cx.notify();
    }

    fn confirm_full_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.close(Layer::ConfirmFullAccess) {
            return;
        }
        let options = ConversationOptions {
            full_access: true,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Opens one of the menus above the composer, or closes it if it is the
    /// one already open. Only one shows at a time.
    fn toggle_composer_menu(&mut self, menu: Layer, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmFullAccess);
        let was_open = self.layers.contains(menu);
        for other in Layer::COMPOSER_MENUS {
            self.layers.close(other);
        }
        if was_open {
            self.focus_composer(window, cx);
        } else {
            self.layers.open(menu);
        }
        cx.notify();
    }

    /// An empty id follows the default model.
    fn choose_model(&mut self, model: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ModelMenu);
        let options = ConversationOptions {
            model,
            ..self.options()
        };
        self.set_options(options, window, cx);
        // The header names the model this conversation runs on.
        self.start_poll(window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Plan first: the model investigates and proposes; nothing is changed.
    fn toggle_plan_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.options();
        let options = ConversationOptions {
            plan_first: !options.plan_first,
            ..options
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Leaves plan mode and, when `execute`, tells the model to go ahead:
    /// with what is typed in the composer, or the standard words. Leaving
    /// first matters: a message sent while still planning would be refused
    /// the tools it needs.
    fn decide_plan(&mut self, execute: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let options = ConversationOptions {
            plan_first: false,
            ..self.options()
        };
        let reply = self
            .backend
            .set_conversation_options(project, session, options);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(options) => {
                        if let Some(status) = &mut this.status {
                            status.options = options;
                        }
                        if execute {
                            let typed = this.composer.read(cx).value().to_string();
                            if typed.trim().is_empty() {
                                this.send_text(controls::APPROVE_PLAN.into(), false, window, cx);
                            } else {
                                this.send(window, cx);
                            }
                        } else {
                            this.start_poll(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Where this conversation's work runs: empty follows Settings.
    fn choose_runs_on(&mut self, runs_on: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::RunsOnMenu);
        let options = ConversationOptions {
            runs_on,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn choose_effort(&mut self, effort: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::EffortMenu);
        let options = ConversationOptions {
            reasoning_effort: effort.into(),
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn answer_approval(
        &mut self,
        approval: String,
        approved: bool,
        remember: wisp_server::desktop::Remember,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let reply = self
            .backend
            .answer_approval(project, session, approval, approved, remember);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // An answered approval resumes a turn the watcher may have
                // stopped following while it waited.
                this.watch_activity(window, cx);
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.start_poll(window, cx);
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops one of the conversation's interpreters, or dismisses a
    /// stopped one, and reads the status again.
    fn stop_runtime(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.stop_runtime(project, session, id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke_allowed_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.revoke_allowed_tools(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Copies a transcript row's text and shows a tick on it for a moment.
    fn copy_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.rows.get(index) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(item.text.clone()));
        self.copied = Some(index);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copied == Some(index) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn control(
        &mut self,
        command: &'static str,
        mut args: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let selection = self.state.selection();
        args["session_id"] = json!(selection.session);
        let reply = self
            .backend
            .request::<Value>(command, selection.project.clone(), args);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // An answered approval resumes a turn the watcher may have
                // stopped following while it waited.
                this.watch_activity(window, cx);
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.pending = true;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open research folder".into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match paths.and_then(|mut paths| paths.pop()) {
                    Some(path) => receive(backend.register_project(path)).await.map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Some(id)) => {
                        this.enter_project(id, window, cx);
                        this.load_projects(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if !self.ready {
            return root.child(self.render_startup(cx));
        }
        // A page takes the whole window; of two, the one opened last shows.
        let page = self.layers.top_of(&[
            Layer::Settings,
            Layer::Favorites,
            Layer::Journey,
            Layer::Calendar,
        ]);
        let root = if page == Some(Layer::Settings) {
            root.child(self.render_settings(cx))
        } else if page == Some(Layer::Calendar) {
            root.child(self.render_calendar(cx))
        } else if page == Some(Layer::Journey) {
            root.child(self.render_journey(cx))
        } else if page == Some(Layer::Favorites) {
            root.child(self.render_favorites(window, cx))
        } else {
            match self.screen {
                Screen::Home => root.child(self.render_home(cx)),
                Screen::Project => {
                    let mut row = div()
                        .h_flex()
                        .size_full()
                        .child(self.render_sidebar(cx))
                        .child(self.render_conversation(cx));
                    if self.layers.contains(Layer::FilesPanel) {
                        row = row.child(self.render_files(window, cx));
                    }
                    root.child(row)
                }
            }
        };
        // Last, so they are drawn over whichever screen or layer is showing.
        let root = if self.layers.contains(Layer::ProjectSettings) {
            root.child(self.render_project_settings(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Capabilities) {
            root.child(self.render_capabilities(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Trajectory) {
            root.child(self.render_trajectory(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Review) {
            root.child(self.render_review(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ModelView) {
            root.child(self.render_model_view(cx))
        } else {
            root
        };
        if self.layers.contains(Layer::Palette) {
            root.child(self.render_palette(cx))
        } else {
            root
        }
    }
}
