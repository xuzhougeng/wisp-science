//! The command palette: one box that finds projects, files and conversations
//! in every project and runs the window's commands. It is a layer in the
//! Escape stack, drawn over whatever screen is showing.
use crate::i18n::tr;
use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt, input::Input};

use super::{Workspace, shell::icon_button};
use crate::{
    assets::icon,
    backend::receive,
    files,
    home::{self, Screen},
    layers::Layer,
    palette::{self, Command, Entry, Mode, PaletteState, Section},
    panel::PanelTab,
};

impl Workspace {
    pub(super) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_palette(Mode::Open, window, cx);
    }

    /// The same box with only its commands (⌘P).
    pub(super) fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_palette(Mode::Commands, window, cx);
    }

    /// The same box, but picking a row adds it to the message being written.
    pub(super) fn toggle_attach_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.session.is_some() && self.read_only().is_none() {
            self.show_palette(Mode::Attach, window, cx);
        }
    }

    /// The same box listing only the project's memos, as the WebView's
    /// `/memo` turns its `/` menu into a memo picker: `query` is what was
    /// typed after the command. Picking one puts its text into the message
    /// after what is written there; Escape leaves the message as it was.
    pub(super) fn open_memo_picker(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.session.is_none() || self.read_only().is_some() {
            return;
        }
        self.layers.close(Layer::Palette);
        self.show_palette(Mode::Memos, window, cx);
        if !query.is_empty() {
            // Reported as a change, which filters the list.
            self.palette_input
                .update(cx, |input, cx| input.set_value(query, window, cx));
        }
    }

    fn show_palette(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Palette) {
            self.focus_composer(window, cx);
        } else {
            self.palette.reset(mode);
            self.layers.open(Layer::Palette);
            let placeholder = match mode {
                Mode::Open => tr("Search conversations, projects and files…"),
                Mode::Attach => tr(
                    "Add a file, an artifact, an environment, a conversation, a skill, a workflow or a memo…",
                ),
                Mode::Commands => tr("Run a command…"),
                Mode::Memos => tr("Search memos"),
            };
            self.palette_input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            // The list with nothing typed is the recent conversations.
            self.load_recent(window, cx);
            match mode {
                Mode::Attach => self.offer_mentions(window, cx),
                Mode::Memos => self.offer_memos(window, cx),
                Mode::Open | Mode::Commands => {}
            }
            // Clearing the box does not report a change when it was empty.
            self.search_palette(window, cx);
        }
        cx.notify();
    }

    fn palette_entries(&self) -> Vec<Entry> {
        let mut entries = self
            .palette
            .entries(&self.projects, &self.recent, self.screen);
        // Privacy mode hides the chosen projects' rows while recording.
        entries.retain(|entry| {
            let project = match entry {
                Entry::Project { id, .. } => id.as_str(),
                Entry::File(found) => found.project_id.as_str(),
                Entry::Conversation { project, .. } => project.as_str(),
                Entry::Artifact {
                    project: Some(project),
                    ..
                } => project.as_str(),
                // The rest belongs to the project that is open.
                Entry::Skill { .. }
                | Entry::Artifact { .. }
                | Entry::Environment { .. }
                | Entry::Runtime { .. }
                | Entry::Workflow { .. }
                | Entry::Memo { .. }
                | Entry::Command(_) => return true,
            };
            !crate::home::concealed(&self.privacy, project)
        });
        if self.palette.mode == Mode::Attach {
            // A conversation is not context for itself.
            entries.retain(|entry| {
                !matches!(entry, Entry::Conversation { id, .. }
                    if Some(id) == self.state.session.as_ref())
            });
        }
        entries
    }

    /// Offers what else a message can carry, from the lists the window
    /// reads for its own tabs: what it has now at once, then what the same
    /// loaders read fresh. The project's artifacts (the Artifacts tab's list
    /// for the whole project), where work can run (the Environments tab's
    /// hosts), the interpreters (the conversation's at once, then those the
    /// host can start, on this computer and the saved SSH hosts), the
    /// workflow library and the memos.
    fn offer_mentions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let opening = self.palette.opening();
        let runtimes = self
            .status
            .as_ref()
            .map_or(&[][..], |status| status.runtimes.as_slice());
        let now = [
            (
                Section::Runtimes,
                Some(palette::runtime_entries(&[], runtimes)),
            ),
            (
                Section::Environments,
                Some(palette::environment_entries(
                    self.environments
                        .as_ref()
                        .map_or(&[][..], |view| view.hosts.as_slice()),
                )),
            ),
            (
                Section::Workflows,
                (!self.workflows.is_empty()).then(|| palette::workflow_entries(&self.workflows)),
            ),
            (
                Section::Memos,
                self.memos.as_deref().map(palette::memo_entries),
            ),
        ];
        for (section, entries) in now {
            if let Some(entries) = entries {
                self.palette.offer(opening, section, entries);
            }
        }
        let session = selection.session.clone();
        let artifacts = self
            .backend
            .artifacts(project.clone(), None, String::new(), 200);
        let environments = self.backend.environments();
        let runtimes = self.backend.runtime_references();
        let workflows = self.backend.workflows();
        let memos = self.backend.memos(project);
        // Each arrives on its own; one that fails leaves its section as it was.
        let offer = move |this: &mut Self, section: Section, entries: Vec<Entry>| {
            this.accept_palette_results(|palette| palette.offer(opening, section, entries));
        };
        cx.spawn_in(window, async move |this, cx| {
            let artifacts = receive(artifacts).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(rows) = artifacts {
                    let entries = palette::artifact_entries(&rows, session.as_deref());
                    offer(this, Section::Artifacts, entries);
                }
                cx.notify();
            });
            let environments = receive(environments).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(view) = environments {
                    offer(
                        this,
                        Section::Environments,
                        palette::environment_entries(&view.hosts),
                    );
                }
                cx.notify();
            });
            let runtimes = receive(runtimes).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(offers) = runtimes {
                    let rows = this
                        .status
                        .as_ref()
                        .map_or(&[][..], |status| status.runtimes.as_slice());
                    let entries = palette::runtime_entries(&offers, rows);
                    offer(this, Section::Runtimes, entries);
                }
                cx.notify();
            });
            let workflows = receive(workflows).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(workflows) = workflows {
                    offer(
                        this,
                        Section::Workflows,
                        palette::workflow_entries(&workflows),
                    );
                }
                cx.notify();
            });
            let memos = receive(memos).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(memos) = memos {
                    offer(this, Section::Memos, palette::memo_entries(&memos));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Offers the project's memos to the `/memo` picker: those the Memos
    /// tab already read at once, then the host's list.
    fn offer_memos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let opening = self.palette.opening();
        if let Some(memos) = self.memos.as_deref() {
            self.palette
                .offer(opening, Section::Memos, palette::memo_entries(memos));
        }
        let memos = self.backend.memos(project);
        cx.spawn_in(window, async move |this, cx| {
            let memos = receive(memos).await;
            let _ = this.update(cx, |this, cx| {
                match memos {
                    Ok(memos) => this.accept_palette_results(|palette| {
                        palette.offer(opening, Section::Memos, palette::memo_entries(&memos))
                    }),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts a memo's text into the message box after what is written there,
    /// as the WebView's `/memo` does: the model reads a memo only this way.
    fn put_memo(&mut self, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = crate::memos::insert(&self.composer.read(cx).value(), body);
        self.state.save_draft(text.clone());
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
    }

    /// Lists results that came back from the host, keeping the highlighted
    /// row highlighted however many are listed above it.
    fn accept_palette_results(&mut self, accept: impl FnOnce(&mut PaletteState) -> bool) {
        let before = self.palette_entries();
        if accept(&mut self.palette) {
            let after = self.palette_entries();
            self.palette.follow_selection(&before, &after);
            if let Some(selected) = self.palette.selection(after.len()) {
                self.palette_scroll
                    .scroll_to_item(palette::child_index(&after, selected));
            }
        }
    }

    /// Searches the host for what is typed, after a pause so fast typing
    /// costs one search rather than one per letter.
    pub(super) fn search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::Palette) {
            return;
        }
        let typed = self.palette_input.read(cx).value().to_string();
        let ticket = self.palette.begin(&typed);
        self.palette_scroll.scroll_to_item(0);
        cx.notify();
        let query = self.palette.query.clone();
        // Commands and memos are listed here; nothing is asked of the host.
        if matches!(self.palette.mode, Mode::Commands | Mode::Memos) {
            return;
        }
        let attach = self.palette.mode == Mode::Attach;
        // To attach, files and skills are this project's; skills are listed
        // even before anything is typed.
        let project = self.state.project.clone().filter(|_| attach);
        // Other projects' artifacts, for a message in this conversation.
        let session = self.state.session.clone().filter(|_| attach);
        if query.is_empty() && project.is_none() {
            return;
        }
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            if !this
                .update(cx, |this, _| this.palette.current(ticket))
                .unwrap_or(false)
            {
                return;
            }
            if let Some(project) = project.clone() {
                let skills =
                    receive(backend.skill_options(project.clone(), query.clone(), 8)).await;
                let _ = this.update(cx, |this, cx| {
                    match skills {
                        Ok(skills) => {
                            this.accept_palette_results(|palette| {
                                palette.accept_skills(ticket, skills)
                            });
                        }
                        Err(error) if this.palette.current(ticket) => this.error = Some(error),
                        Err(_) => {}
                    }
                    cx.notify();
                });
                if let Some(session) = session {
                    let found = backend.artifacts_elsewhere(project, session, query.clone(), 40);
                    let found = receive(found).await;
                    let _ = this.update(cx, |this, cx| {
                        // One that cannot be read leaves the list as it was.
                        if let Ok(rows) = found {
                            let entries = palette::elsewhere_entries(&rows);
                            this.accept_palette_results(|palette| {
                                palette.accept_elsewhere(ticket, entries)
                            });
                        }
                        cx.notify();
                    });
                }
            }
            if query.is_empty() {
                return;
            }
            let conversations = backend.find_conversations(query.clone(), 12);
            let found = backend.find_files(project, query, 6);
            let conversations = receive(conversations).await;
            let _ = this.update(cx, |this, cx| {
                match conversations {
                    Ok(rows) => {
                        this.accept_palette_results(|palette| {
                            palette.accept_conversations(ticket, rows)
                        });
                    }
                    Err(error) if this.palette.current(ticket) => this.error = Some(error),
                    Err(_) => {}
                }
                cx.notify();
            });
            let found = receive(found).await;
            let _ = this.update(cx, |this, cx| {
                match found {
                    Ok(found) => {
                        this.accept_palette_results(|palette| palette.accept_files(ticket, found));
                    }
                    Err(error) if this.palette.current(ticket) => this.error = Some(error),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Moves the selection with the arrow keys. Returns false when the
    /// palette is closed, so the keys keep their usual meaning elsewhere.
    pub(super) fn step_palette(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if !self.layers.contains(Layer::Palette) {
            return false;
        }
        let entries = self.palette_entries();
        self.palette.move_selection(delta, entries.len());
        if let Some(selected) = self.palette.selection(entries.len()) {
            self.palette_scroll
                .scroll_to_item(palette::child_index(&entries, selected));
        }
        cx.notify();
        true
    }

    /// Opens the clicked row, or the selected one when Enter was pressed.
    pub(super) fn activate_palette(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.layers.contains(Layer::Palette) {
            return;
        }
        let entries = self.palette_entries();
        let Some(entry) = index
            .or_else(|| self.palette.selection(entries.len()))
            .and_then(|index| entries.into_iter().nth(index))
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        if matches!(self.palette.mode, Mode::Attach | Mode::Memos) {
            if let Entry::Memo { body, .. } = &entry {
                self.put_memo(body, window, cx);
            } else if let Entry::Command(command) = entry {
                // The `/memo` picker's one row in a project with no memos.
                return self.run_command(command, window, cx);
            } else if let Some(chip) = entry.chip() {
                self.attach(chip, cx);
            }
            self.focus_composer(window, cx);
            cx.notify();
            return;
        }
        match entry {
            // These are only listed for attaching.
            Entry::Skill { .. }
            | Entry::Artifact { .. }
            | Entry::Environment { .. }
            | Entry::Runtime { .. }
            | Entry::Workflow { .. }
            | Entry::Memo { .. } => {}
            Entry::Command(command) => self.run_command(command, window, cx),
            // Everything else goes somewhere that Settings would cover.
            Entry::Project { id, .. } => {
                self.layers.close(Layer::Settings);
                self.enter_project(id, window, cx);
                self.focus_composer(window, cx);
            }
            Entry::Conversation { project, id, .. } => {
                self.layers.close(Layer::Settings);
                self.open_session(project, id, window, cx);
            }
            Entry::File(file) => {
                self.layers.close(Layer::Settings);
                if self.screen != Screen::Project
                    || self.state.project.as_ref() != Some(&file.project_id)
                {
                    self.enter_project(file.project_id, window, cx);
                }
                self.reveal_file(file.path, window, cx);
                self.focus_composer(window, cx);
            }
        }
        cx.notify();
    }

    fn run_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Settings => return self.open_settings(window, cx),
            // These leave the app: they open in the system's browser.
            Command::Docs => return cx.open_url(crate::links::DOCS),
            Command::Feedback => {
                self.layers.close(Layer::Settings);
                return self.report_problem(window, cx);
            }
            Command::Find => return self.open_find(window, cx),
            Command::Journey => return self.open_journey(window, cx),
            Command::Papers => return self.open_papers(window, cx),
            Command::Schedules => return self.open_schedules(window, cx),
            Command::Assistant => return self.open_assistant(window, cx),
            Command::Mandates => return self.open_mandates(window, cx),
            Command::PythonConsole => {
                return self.start_console(
                    wisp_server::desktop::RuntimeLanguage::Python,
                    window,
                    cx,
                );
            }
            Command::RConsole => {
                return self.start_console(wisp_server::desktop::RuntimeLanguage::R, window, cx);
            }
            Command::Calendar => return self.open_calendar(window, cx),
            Command::Capabilities => return self.open_capabilities(window, cx),
            Command::Trajectory => return self.open_trajectory(window, cx),
            Command::Review => return self.open_review(window, cx),
            Command::ModelView => return self.open_model_view(window, cx),
            Command::Handoff => return self.open_handoff(window, cx),
            Command::ResearchArchive => return self.open_research_archive(window, cx),
            Command::Terminal => {
                self.layers.close(Layer::Palette);
                return self.open_terminal_here(window, cx);
            }
            Command::SaveAsSkill => {
                self.layers.close(Layer::Palette);
                return self.ask_for_skill(window, cx);
            }
            Command::Aside => {
                self.layers.close(Layer::Palette);
                return self.open_aside(window, cx);
            }
            Command::NewWindow => {
                crate::open_window(self.backend.clone(), cx);
                return;
            }
            Command::ProjectSettings => {
                if let Some(project) = self.state.project.clone() {
                    self.open_project_settings(project, window, cx);
                }
                return;
            }
            Command::Favorites => return self.open_favorites(window, cx),
            Command::StarUs => return cx.open_url(crate::links::REPOSITORY),
            Command::CheckUpdates => {
                self.open_settings(window, cx);
                self.show_settings_section(crate::settings::Section::General, window, cx);
                return self.check_for_update(window, cx);
            }
            Command::Skills => {
                self.open_settings(window, cx);
                return self.show_settings_section(crate::settings::Section::Skills, window, cx);
            }
            Command::NewProject => return self.open_new_project(window, cx),
            Command::PrivacyMode => {
                self.layers.close(Layer::Palette);
                return self.toggle_privacy(window, cx);
            }
            Command::Setup => {
                self.layers.close(Layer::Palette);
                return self.open_onboarding(cx);
            }
            Command::ImportProject => return self.import_project(window, cx),
            Command::WebViewImport => return self.open_webview_import(window, cx),
            Command::ExportProject => {
                if let Some(project) = self.state.project.clone() {
                    self.export_project(project, window, cx);
                }
                return;
            }
            Command::ImportConversations => return self.open_imports(window, cx),
            Command::LightTheme | Command::DarkTheme | Command::SystemTheme => {
                let choice = match command {
                    Command::LightTheme => crate::theme::Appearance::Light,
                    Command::DarkTheme => crate::theme::Appearance::Dark,
                    _ => crate::theme::Appearance::System,
                };
                self.set_appearance(choice, window, cx);
            }
            Command::LargerText | Command::SmallerText => {
                let by = if command == Command::LargerText {
                    1.
                } else {
                    -1.
                };
                self.step_size(
                    self.look.text_size,
                    by,
                    |look, size| look.text_size = size,
                    window,
                    cx,
                );
            }
            Command::LargerCode | Command::SmallerCode => {
                let by = if command == Command::LargerCode {
                    1.
                } else {
                    -1.
                };
                self.step_size(
                    self.look.code_size,
                    by,
                    |look, size| look.code_size = size,
                    window,
                    cx,
                );
            }
            Command::ToggleSidebar => self.toggle_sidebar(window, cx),
            Command::ClosePanel => {
                if self.layers.contains(Layer::FilesPanel) {
                    self.toggle_files(window, cx);
                }
            }
            Command::Notebook => self.show_panel_tab(PanelTab::Notebook, window, cx),
            Command::Provenance => self.show_panel_tab(PanelTab::Provenance, window, cx),
            Command::Agents => self.show_panel_tab(PanelTab::Agents, window, cx),
            Command::Environments => self.show_panel_tab(PanelTab::Environments, window, cx),
            Command::OpenFolder => return self.open_project(window, cx),
            Command::Refresh => {
                self.error = None;
                self.refresh_lists(window, cx);
                self.start_poll(window, cx);
                self.reload_files(window, cx);
            }
            Command::NewConversation
            | Command::Home
            | Command::Files
            | Command::Runs
            | Command::Artifacts
            | Command::Memos
            | Command::Memory
            | Command::Highlights
            | Command::Archived => {
                self.layers.close(Layer::Settings);
                match command {
                    Command::NewConversation => self.create_session(window, cx),
                    Command::Home => self.go_home(window, cx),
                    Command::Files => self.toggle_files(window, cx),
                    Command::Runs => self.show_panel_tab(PanelTab::Runs, window, cx),
                    Command::Artifacts => self.show_panel_tab(PanelTab::Artifacts, window, cx),
                    Command::Memos => self.show_panel_tab(PanelTab::Memos, window, cx),
                    Command::Memory => self.show_panel_tab(PanelTab::Memory, window, cx),
                    Command::Highlights => self.show_panel_tab(PanelTab::Highlights, window, cx),
                    _ => {
                        self.show_archived = !self.show_archived;
                        self.sessions.clear();
                        self.load_sessions(window, cx);
                    }
                }
            }
        }
        self.focus_composer(window, cx);
    }

    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.palette_entries();
        let selected = self.palette.selection(entries.len());
        let muted = cx.theme().muted_foreground;
        let active = cx.theme().list_active;
        let hover = cx.theme().list_hover;
        let border = cx.theme().border;
        let radius = cx.theme().radius;
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());

        let mut list = div()
            .id("palette-results")
            .track_scroll(&self.palette_scroll)
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_1();
        let memo_picker = self.palette.mode == Mode::Memos;
        let mut section = None;
        for (index, entry) in entries.iter().enumerate() {
            if section != Some(entry.section()) {
                section = Some(entry.section());
                // The memo picker's row for a project without memos is
                // under the memos' heading too.
                let heading = if memo_picker {
                    Section::Memos.title()
                } else {
                    entry.section().title()
                };
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted)
                        .child(heading),
                );
            }
            let glyph = entry.glyph();
            let (title, detail) = match entry {
                Entry::Project { name, path, .. } => {
                    (name.clone(), home::short_path(path, home_dir.as_deref()))
                }
                Entry::File(file) => (
                    files::file_name(&file.path).to_owned(),
                    format!("{} · {}", file.project_name, file.path),
                ),
                Entry::Conversation { title, place, .. } => (title.clone(), place.clone()),
                Entry::Skill { name, description } => (name.clone(), description.clone()),
                Entry::Artifact { name, detail, .. } => (name.clone(), detail.clone()),
                Entry::Environment { label, detail, .. } => (label.clone(), detail.clone()),
                Entry::Runtime { title, detail, .. } => (title.clone(), detail.clone()),
                Entry::Workflow { name, goal, .. } => (name.clone(), goal.clone()),
                Entry::Memo {
                    heading, preview, ..
                } => (heading.clone(), preview.clone()),
                // The WebView's words for a project with no memos yet.
                Entry::Command(Command::Memos) if memo_picker => (
                    tr("This project has no memos yet").to_owned(),
                    tr("Open Memos").to_owned(),
                ),
                Entry::Command(command) => (command.label().to_owned(), String::new()),
            };
            // A project can also be opened in a window of its own.
            let new_window = entry.window_project(self.palette.mode).map(|id| {
                let id = id.to_owned();
                icon_button(
                    ("palette-window", index),
                    "dock",
                    tr("Open in a new window"),
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.layers.close(Layer::Palette);
                    crate::open_window_on(this.backend.clone(), Some(id.clone()), cx);
                    cx.notify();
                }))
            });
            // A file or conversation found can join the message without
            // leaving the conversation.
            let attach = self
                .state
                .project
                .as_deref()
                .filter(|_| self.screen == Screen::Project)
                .and_then(|project| {
                    entry.chip_in(self.palette.mode, project, self.state.session.as_deref())
                })
                .map(|chip| {
                    icon_button(
                        ("palette-attach", index),
                        "attach",
                        tr("Add to the message"),
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.layers.close(Layer::Palette);
                        this.attach(chip.clone(), cx);
                        this.focus_composer(window, cx);
                        cx.notify();
                    }))
                });
            list = list.child(
                div()
                    .id(("palette-row", index))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .h(px(34.))
                    .flex_shrink_0()
                    .rounded(radius)
                    .cursor_pointer()
                    .hover(|style| style.bg(hover))
                    .when(selected == Some(index), |row| row.bg(active))
                    .child(icon(glyph).text_color(muted))
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(340.))
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(detail),
                    )
                    .children(attach)
                    .children(new_window)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_palette(Some(index), window, cx)
                    })),
            );
        }
        if entries.is_empty() {
            list = list.child(div().p_4().text_sm().text_color(muted).child(
                if memo_picker && self.palette.query.is_empty() {
                    tr("Loading…").to_owned()
                } else if self.palette.query.is_empty() {
                    tr("Type a file name, a word from a conversation, or a skill.").to_owned()
                } else {
                    crate::i18n::fill(tr("Nothing matches “{}”."), &[&self.palette.query])
                },
            ));
        }

        let card = div()
            .id("palette")
            .v_flex()
            .w(px(640.))
            .max_h(px(520.))
            .overflow_hidden()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            // A click inside is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h_flex()
                    .px_2()
                    .h(px(46.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Input::new(&self.palette_input)
                            .appearance(false)
                            .prefix(icon("search").text_color(muted)),
                    ),
            )
            .child(list)
            .child(
                div()
                    .h_flex()
                    .px_3()
                    .h(px(30.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(match self.palette.mode {
                        Mode::Open | Mode::Commands => {
                            "↑ ↓ to move · Return to open · Esc to close"
                        }
                        Mode::Attach => "↑ ↓ to move · Return to add · Esc to close",
                        Mode::Memos => {
                            tr("↑ ↓ to move · Return to put it into the message · Esc to close")
                        }
                    }),
            );

        div()
            .id("palette-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(96.))
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.layers.close(Layer::Palette) {
                        this.focus_composer(window, cx);
                        cx.notify();
                    }
                }),
            )
            .child(card)
    }
}
