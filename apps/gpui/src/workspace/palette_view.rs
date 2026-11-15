//! The command palette: one box that finds projects, files and conversations
//! in every project and runs the window's commands. It is a layer in the
//! Escape stack, drawn over whatever screen is showing.
use crate::i18n::tr;
use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt, input::Input};

use super::Workspace;
use crate::{
    assets::icon,
    backend::receive,
    compose::Chip,
    files,
    home::{self, Screen},
    layers::Layer,
    palette::{self, Command, Entry, Mode},
    panel::PanelTab,
};

impl Workspace {
    pub(super) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_palette(Mode::Open, window, cx);
    }

    /// The same box, but picking a row adds it to the message being written.
    pub(super) fn toggle_attach_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.session.is_some() {
            self.show_palette(Mode::Attach, window, cx);
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
                Mode::Attach => tr("Add a file, a conversation or a skill to this message…"),
            };
            self.palette_input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            // The list with nothing typed is the recent conversations.
            self.load_recent(window, cx);
            // Clearing the box does not report a change when it was empty.
            self.search_palette(window, cx);
        }
        cx.notify();
    }

    fn palette_entries(&self) -> Vec<Entry> {
        let mut entries = self
            .palette
            .entries(&self.projects, &self.recent, self.screen);
        if self.palette.mode == Mode::Attach {
            // A conversation is not context for itself.
            entries.retain(|entry| {
                !matches!(entry, Entry::Conversation { id, .. }
                    if Some(id) == self.state.session.as_ref())
            });
        }
        entries
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
        let attach = self.palette.mode == Mode::Attach;
        // To attach, files and skills are this project's; skills are listed
        // even before anything is typed.
        let project = self.state.project.clone().filter(|_| attach);
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
                let skills = receive(backend.skill_options(project, query.clone(), 8)).await;
                let _ = this.update(cx, |this, cx| {
                    match skills {
                        Ok(skills) => {
                            this.palette.accept_skills(ticket, skills);
                        }
                        Err(error) if this.palette.current(ticket) => this.error = Some(error),
                        Err(_) => {}
                    }
                    cx.notify();
                });
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
                        this.palette.accept_conversations(ticket, rows);
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
                        this.palette.accept_files(ticket, found);
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
        if self.palette.mode == Mode::Attach {
            let chip = match entry {
                Entry::File(file) => Chip::File(file.path),
                Entry::Conversation { id, title, .. } => Chip::Conversation { id, title },
                Entry::Skill { name, .. } => Chip::Skill(name),
                Entry::Project { .. } | Entry::Command(_) => return,
            };
            self.attach(chip, cx);
            self.focus_composer(window, cx);
            return;
        }
        match entry {
            // Skills are only listed for attaching.
            Entry::Skill { .. } => {}
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
            Command::Feedback => return cx.open_url(crate::links::ISSUES),
            Command::Find => return self.open_find(window, cx),
            Command::Journey => return self.open_journey(window, cx),
            Command::Papers => return self.open_papers(window, cx),
            Command::Schedules => return self.open_schedules(window, cx),
            Command::Assistant => return self.open_assistant(window, cx),
            Command::Mandates => return self.open_mandates(window, cx),
            Command::Calendar => return self.open_calendar(window, cx),
            Command::Capabilities => return self.open_capabilities(window, cx),
            Command::Trajectory => return self.open_trajectory(window, cx),
            Command::Review => return self.open_review(window, cx),
            Command::ModelView => return self.open_model_view(window, cx),
            Command::ResearchArchive => return self.open_research_archive(window, cx),
            Command::Terminal => {
                self.layers.close(Layer::Palette);
                return self.open_terminal(cx);
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
        let mut section = None;
        for (index, entry) in entries.iter().enumerate() {
            if section != Some(entry.section()) {
                section = Some(entry.section());
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted)
                        .child(entry.section().title()),
                );
            }
            let (glyph, title, detail) = match entry {
                Entry::Project { name, path, .. } => (
                    "folder",
                    name.clone(),
                    home::short_path(path, home_dir.as_deref()),
                ),
                Entry::File(file) => (
                    "doc",
                    files::file_name(&file.path).to_owned(),
                    format!("{} · {}", file.project_name, file.path),
                ),
                Entry::Conversation { title, place, .. } => ("chat", title.clone(), place.clone()),
                Entry::Skill { name, description } => ("skill", name.clone(), description.clone()),
                Entry::Command(command) => {
                    (command.icon(), command.label().to_owned(), String::new())
                }
            };
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_palette(Some(index), window, cx)
                    })),
            );
        }
        if entries.is_empty() {
            list = list.child(div().p_4().text_sm().text_color(muted).child(
                if self.palette.query.is_empty() {
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
                        Mode::Open => "↑ ↓ to move · Return to open · Esc to close",
                        Mode::Attach => "↑ ↓ to move · Return to add · Esc to close",
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
