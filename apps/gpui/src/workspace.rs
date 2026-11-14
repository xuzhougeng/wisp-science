mod conversation_actions;
mod files_panel;
mod home_view;
mod organise_view;
mod palette_view;
mod panel_tabs;
mod settings_view;
mod shell;

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
use wisp_server::desktop::{ArtifactRow, ConversationOptions, ConversationStatus, Folder, TurnRef};

use crate::{
    backend::{Backend, receive},
    compose::{Chip, Pending},
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
    /// Tool rows whose input and output are unfolded.
    open_tools: HashSet<String>,
    /// Files and references attached to the message being written, per
    /// conversation, like the drafts.
    chips: HashMap<String, Pending>,
    /// Options, usage and remembered approvals of the open conversation.
    status: Option<ConversationStatus>,
    /// Counts snapshot polls, so the status is refreshed about once a second.
    status_tick: u8,
    /// Transcript row whose text was just copied, for a moment of feedback.
    copied: Option<usize>,
    palette: PaletteState,
    palette_input: Entity<InputState>,
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
            cx.subscribe_in(&rename, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_rename(window, cx);
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
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::follow_system(window, cx)
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
            open_tools: HashSet::new(),
            chips: HashMap::new(),
            status: None,
            status_tick: 0,
            copied: None,
            palette: PaletteState::default(),
            palette_input,
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
                self.restore_focus(window, cx);
                // A file revealed from the palette has no listing under it yet.
                self.ensure_files(window, cx);
            }
            Some(Layer::FilesPanel) => self.restore_focus(window, cx),
            // These layers own the focused field, which goes away with them.
            Some(Layer::Settings | Layer::Rename | Layer::ConfirmDelete) => {
                self.focus_composer(window, cx)
            }
            Some(Layer::ConfirmRemoveProject) => self.removing = None,
            Some(
                Layer::Palette | Layer::ConfirmFullAccess | Layer::EffortMenu | Layer::ModelMenu,
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
            Shortcut::NewConversation => {
                if self.screen != Screen::Project
                    || self.layers.contains(Layer::Settings)
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
        ] {
            self.layers.close(layer);
        }
        if self.state.session != session {
            self.rewind_turn = None;
            self.status = None;
            self.copied = None;
            // The list belonged to the conversation, or project, being left.
            self.artifacts.clear();
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
        self.layers.open(Layer::ConfirmRemoveProject);
        cx.notify();
    }

    fn cancel_remove_project(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ConfirmRemoveProject) {
            self.removing = None;
            cx.notify();
        }
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
        self.ensure_files(window, cx);
        self.reload_artifacts(window, cx);
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
                                    }
                                    this.mark_seen(window, cx);
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
            let _ = this.update(cx, |this, cx| {
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
        let reply = self.backend.request::<String>(
            "native_conversation_create",
            selection.project.clone(),
            json!({}),
        );
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
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        // What the composer has attached goes with what it has typed.
        let carried = if from_composer {
            self.chips.get(&session).cloned().unwrap_or_default()
        } else {
            Pending::default()
        };
        let attachments = carried.attachments();
        // Files alone are a message; the host names it after the first one.
        let sendable = if text.trim().is_empty() && !attachments.is_empty() {
            "files"
        } else {
            text.as_str()
        };
        if !self.state.can_send(sendable, self.pending) || self.loading_history {
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
                // The files belong to the conversation they were picked for.
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(paths) => {
                        for path in paths {
                            this.attach(Chip::File(path), cx);
                        }
                        this.reload_files(window, cx);
                        this.focus_composer(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
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
        for_conversation: bool,
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
        let reply =
            self.backend
                .answer_approval(project, session, approval, approved, for_conversation);
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
        let root = if self.layers.contains(Layer::Settings) {
            root.child(self.render_settings(cx))
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
        // Last, so it is drawn over whichever screen or layer is showing.
        if self.layers.contains(Layer::Palette) {
            root.child(self.render_palette(cx))
        } else {
            root
        }
    }
}
