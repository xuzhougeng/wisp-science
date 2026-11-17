//! The home page: every project and the most recent conversations. Removing
//! a project asks first, in a layer that Escape closes.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::ProjectSummary;

use super::{
    Workspace, chip,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth, IconSize, IconTone, StampKind},
};
use crate::{
    backend::receive,
    home,
    layers::Layer,
    present::{self, Tone},
};

impl Workspace {
    /// The stronger question on a project's card: its folder goes too.
    /// Nothing is deleted until the project's name has been typed.
    fn render_delete_folder(&self, question: &home::FolderQuestion, cx: &mut Context<Self>) -> Div {
        let danger = cx.theme().danger;
        let cancel = Button::new("delete-folder-cancel")
            .label(tr("Cancel"))
            .small()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_remove_project(cx)));
        let block = div().v_flex().gap_2().pt_1();
        let folder = match question {
            home::FolderQuestion::Looking => {
                return block
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("Looking at the folder…")),
                    )
                    .child(div().h_flex().justify_end().child(cancel));
            }
            home::FolderQuestion::Refused(reason) => {
                return block
                    .child(div().text_xs().text_color(danger).child(crate::i18n::fill(
                        tr("The folder was not deleted: {}. You can still remove the \
                         project from Wisp and keep its files."),
                        &[&reason],
                    )))
                    .child(div().h_flex().justify_end().child(cancel));
            }
            home::FolderQuestion::Ask(folder) => folder,
        };
        let typed = self.delete_name_input.read(cx).value().to_string();
        let ready = home::name_typed(&typed, folder) && !self.pending;
        block
            .child(
                div()
                    .text_xs()
                    .text_color(danger)
                    .child(home::folder_warning(folder)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::fill(
                        tr("Type the project's name, {}, to go on."),
                        &[&folder.name],
                    )),
            )
            .child(super::surfaces::input(&self.delete_name_input, cx).small())
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("delete-folder-confirm")
                            .label(tr("Delete project and folder"))
                            .small()
                            .danger()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_project_folder(window, cx)
                            })),
                    )
                    .child(cancel),
            )
    }

    /// A project's cell in the grid: its name (Title), its folder (a fact),
    /// a line of icons and numbers, a stamp when something waits for the
    /// user, and its actions, which show under the pointer.
    fn project_cell(
        &self,
        project: &ProjectSummary,
        now: i64,
        home_dir: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = project.id.clone();
        let hover = cx.theme().list_hover;
        let group = SharedString::from(format!("project-cell-{id}"));
        let mut top = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(surfaces::title(project.name.clone()).min_w_0().truncate());
        if project.needs_you_count > 0 {
            top = top.child(surfaces::stamp_with(
                StampKind::Waiting,
                "bell",
                project.needs_you_count.to_string(),
                cx,
            ));
        }
        if let Some((label, tone)) = home::folder_badge(project) {
            top = top.child(chip(label, tone, cx));
        }
        let cell = div()
            .id(SharedString::from(format!("project-{id}")))
            .group(group.clone())
            .v_flex()
            .gap_1()
            .px_4()
            .py_3()
            .child(top)
            .child(
                surfaces::fact(home::short_path(&project.workspace_dir, home_dir), cx).truncate(),
            );
        if self.removing.as_ref() == Some(&id) {
            if let Some(question) = &self.removing_folder {
                return cell.child(self.render_delete_folder(question, cx));
            }
            // Not a link while the question is open: a stray click must not
            // open the project that is about to be removed.
            return cell
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(tr(
                            "Remove this project and its conversations from Wisp? The folder \
                         and its files stay on disk.",
                        )),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("remove-confirm")
                                .label(tr("Remove"))
                                .small()
                                .danger()
                                .disabled(self.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_project(window, cx)
                                })),
                        )
                        .child(
                            Button::new("remove-with-folder")
                                .label(tr("Delete folder too…"))
                                .small()
                                .ghost()
                                .tooltip(tr(
                                    "Also delete the project's folder and its files, for good",
                                ))
                                .disabled(self.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.ask_delete_project_folder(window, cx)
                                })),
                        )
                        .child(
                            Button::new("remove-cancel")
                                .label(tr("Cancel"))
                                .small()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_remove_project(cx)),
                                ),
                        ),
                );
        }
        let starred = project.starred;
        let star_id = id.clone();
        let mut actions = div().h_flex().gap_0p5().child(
            surfaces::icon_action(
                SharedString::from(format!("star-{id}")),
                if starred { "star-filled" } else { "star" },
                if starred {
                    tr("Stop keeping first")
                } else {
                    tr("Keep first")
                },
                if starred {
                    IconTone::Ink
                } else {
                    IconTone::Faded
                },
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // The cell underneath opens the project; this click is not that.
                cx.stop_propagation();
                this.toggle_star(star_id.clone(), !starred, window, cx)
            })),
        );
        let settings_id = id.clone();
        actions = actions.child(
            surfaces::icon_action(
                SharedString::from(format!("project-settings-{id}")),
                "adjustments",
                tr("Project settings"),
                IconTone::Faded,
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_project_settings(settings_id.clone(), window, cx)
            })),
        );
        let window_id = id.clone();
        actions = actions.child(
            surfaces::icon_action(
                SharedString::from(format!("project-window-{id}")),
                "dock",
                tr("Open in a new window"),
                IconTone::Faded,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                crate::open_window_on(this.backend.clone(), Some(window_id.clone()), cx);
            })),
        );
        let (export_id, package_id) = (id.clone(), id.clone());
        actions = actions
            .child(
                surfaces::icon_action(
                    SharedString::from(format!("project-export-{id}")),
                    "archive-export",
                    tr("Export the project as an archive"),
                    IconTone::Faded,
                    cx,
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.export_project(export_id.clone(), window, cx)
                })),
            )
            .child(
                surfaces::icon_action(
                    SharedString::from(format!("project-package-{id}")),
                    "folder-move",
                    tr("Export the project as a folder another computer opens in place"),
                    IconTone::Faded,
                    cx,
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.export_project_folder(package_id.clone(), window, cx)
                })),
            );
        if wisp_server::desktop::removable(&id) {
            let remove_id = id.clone();
            actions = actions.child(
                surfaces::icon_action(
                    SharedString::from(format!("remove-{id}")),
                    "trash",
                    tr("Remove from Wisp"),
                    IconTone::Faded,
                    cx,
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.ask_remove_project(remove_id.clone(), cx)
                })),
            );
        }
        // Icons and numbers: conversations, artifacts, when it was last
        // touched, and what is running. The words are its tooltip.
        let number = |icon: &str, text: String, cx: &App| {
            div()
                .h_flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(surfaces::sized_icon(
                    icon,
                    IconSize::Inline,
                    IconTone::Faded,
                    cx,
                ))
                .child(surfaces::fact(text, cx))
        };
        let mut said = vec![home::counts(project)];
        said.extend(home::activity(project).into_iter().map(|(label, _)| label));
        let said = SharedString::from(said.join(" · "));
        let mut numbers = div()
            .id(SharedString::from(format!("project-numbers-{id}")))
            .h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_3()
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(said.clone()).build(window, cx)
            })
            .child(number("chat", project.session_count.to_string(), cx))
            .child(number("layers", project.artifact_count.to_string(), cx))
            .child(number("clock", home::age(now, project.updated_at), cx));
        if project.running_count > 0 {
            let running = surfaces::stamp_color(StampKind::Running, cx);
            numbers = numbers.child(
                div()
                    .h_flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .child(crate::assets::icon(StampKind::Running.icon()).text_color(running))
                    .child(surfaces::fact(project.running_count.to_string(), cx)),
            );
        }
        // The actions show under the pointer, laid over the cell's top
        // right corner on the cell's hover colour, so they take no height
        // of their own. While hidden they take no click, so the cell opens
        // the project.
        let actions = actions
            .absolute()
            .top_2()
            .right_2()
            .rounded(cx.theme().radius)
            .bg(hover)
            .invisible()
            .group_hover(group, |style| style.visible());
        cell.relative()
            .child(numbers)
            .child(actions)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_click(
                cx.listener(move |this, _, window, cx| this.enter_project(id.clone(), window, cx)),
            )
    }

    /// Opens the dialog that names a new project.
    pub(super) fn open_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_project_note = None;
        self.new_project_layout = true;
        for field in [&self.new_project_name, &self.new_project_description] {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.layers.open(Layer::NewProject);
        self.new_project_name
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Asks where the new project's folder goes, then makes it and opens
    /// the project.
    fn create_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_project_name.read(cx).value().trim().to_owned();
        if name.is_empty() || self.pending {
            return;
        }
        let description = self.new_project_description.read(cx).value().to_string();
        let layout = self.new_project_layout;
        self.new_project_note = None;
        self.pending = true;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Make the project here").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let place = picker
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                match place.and_then(|mut paths| paths.pop()) {
                    Some(parent) => {
                        receive(backend.create_project(parent, name, description, layout))
                            .await
                            .map(Some)
                    }
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Some(id)) => {
                        this.layers.close(Layer::NewProject);
                        this.enter_project(id, window, cx);
                        this.load_projects(window, cx);
                    }
                    // The folder was not chosen: the dialog stays as it was.
                    Ok(None) => {}
                    Err(error) => this.new_project_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The dialog for a new project: its name and description, and whether
    /// its folder gets the standard research layout.
    pub(super) fn render_new_project(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ready = !self.new_project_name.read(cx).value().trim().is_empty() && !self.pending;
        let layout = self.new_project_layout;
        let mut card = surfaces::dialog("new-project", DialogWidth::Narrow, tr("New project"))
            .gap_3()
            .close_on_click_outside(cx.listener(|this, _, _, cx| {
                this.layers.close(Layer::NewProject);
                cx.notify();
            }))
            .child(div().text_xs().text_color(muted).child(tr(
                "A folder with this name is made where you choose next, and the project opens in it.",
            )))
            .child(super::surfaces::input(&self.new_project_name, cx).small())
            .child(super::surfaces::input(&self.new_project_description, cx).h(px(72.)))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        surfaces::switch("new-project-layout", layout, tr("Standard layout"))
                            .on_change(cx.listener(|this, _, _, cx| {
                                this.new_project_layout = !this.new_project_layout;
                                cx.notify();
                            })),
                    )
                    .child(div().text_sm().child(tr(
                        "Standard layout: data, analysis, results, figures, literature and docs folders",
                    ))),
            );
        if let Some(error) = &self.new_project_note {
            card = card.child(chip(crate::i18n::host(error), Tone::Danger, cx));
        }
        card.action(
            Button::new("new-project-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::NewProject);
                    cx.notify();
                })),
        )
        .action(
            Button::new("new-project-create")
                .label(tr("Choose where and create"))
                .small()
                .primary()
                .disabled(!ready)
                .on_click(cx.listener(|this, _, window, cx| this.create_new_project(window, cx))),
        )
    }

    /// The rarer ways to start, under the home page's More button: each an
    /// icon and a word, its longer name as its tooltip. Escape closes it
    /// (`Layer::StartMenu`); choosing one closes it first.
    fn start_menu(&self, cx: &mut Context<Self>) -> Div {
        let hover = cx.theme().list_hover;
        let pending = self.pending;
        type Start = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
        let ways: [(
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            bool,
            Start,
        ); 4] = [
            (
                "home-examples",
                "lightbulb",
                tr("Example conversations"),
                tr("Read the example conversations that ship with Wisp"),
                false,
                |this, window, cx| this.open_examples(window, cx),
            ),
            (
                "home-webview",
                "conversation-import",
                tr("Bring over from WebView"),
                tr("Bring over projects from the WebView desktop"),
                true,
                |this, window, cx| this.open_webview_import(window, cx),
            ),
            (
                "home-join",
                "sync",
                tr("Join from another device"),
                tr("Join a project from another device"),
                true,
                |this, window, cx| this.open_join_synced(window, cx),
            ),
            (
                "home-sample",
                "folder-plus",
                tr("Sample project"),
                tr("Make a sample project to try Wisp on"),
                true,
                |this, window, cx| this.create_sample_project(window, cx),
            ),
        ];
        let mut menu = div()
            .absolute()
            .top(px(surfaces::ICON_BUTTON + 4.))
            .right_0()
            .w(px(260.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md();
        for (id, icon, word, about, waits, start) in ways {
            // Those that change projects wait while another change is under way.
            let live = !(waits && pending);
            menu = menu.child(
                div()
                    .id(id)
                    .group(surfaces::ICON_HOVER)
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(30.))
                    .rounded(cx.theme().radius)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(about).build(window, cx)
                    })
                    .child(surfaces::sized_icon(
                        icon,
                        IconSize::Inline,
                        IconTone::Faded,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(word))
                    .when(!live, |row| row.opacity(0.5))
                    .when(live, |row| {
                        row.cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.layers.close(Layer::StartMenu);
                                start(this, window, cx);
                                cx.notify();
                            }))
                    }),
            );
        }
        menu
    }

    /// What the window's bar holds on the home page: the wordmark, and
    /// opposite it the places to go as icon and word, then the
    /// documentation and the settings as icon buttons.
    pub(super) fn render_home_bar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let place = |id: &'static str, icon: &str, word: &'static str, name: &'static str| {
            surfaces::labelled_action(id, icon, word, IconSize::Navigation, IconTone::Faded, cx)
                .tooltip(name)
        };
        div()
            .h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            // The wordmark keeps the serif's Medium.
            .child(
                surfaces::title("Wisp")
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().primary),
            )
            .child(
                div()
                    .pl_2()
                    .child(chip(tr("GPUI preview"), Tone::Neutral, cx)),
            )
            .child(div().flex_1())
            .child(
                place(
                    "home-assistant",
                    "bot",
                    tr("Research assistant"),
                    tr("Research assistant"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_assistant(window, cx))),
            )
            .child(
                place(
                    "home-mandates",
                    "clipboard",
                    tr("Mandates"),
                    tr("Research mandates"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_mandates(window, cx))),
            )
            .child(
                place(
                    "home-calendar",
                    "calendar",
                    tr("Calendar"),
                    tr("Research calendar"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_calendar(window, cx))),
            )
            .child(
                place(
                    "home-schedules",
                    "clock",
                    tr("Scheduled"),
                    tr("Scheduled tasks"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_schedules(window, cx))),
            )
            .child(
                place("home-favorites", "star", tr("Favorites"), tr("Favorites"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_favorites(window, cx))),
            )
            // What waits for the user, across projects, beside the docs.
            .children(self.render_waiting(window, cx))
            .child(
                surfaces::icon_action(
                    "home-docs",
                    "book",
                    tr("Documentation, in your browser"),
                    IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.open_url(crate::links::DOCS))),
            )
            .child(
                surfaces::icon_action("home-settings", "gear", tr("Settings"), IconTone::Faded, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
    }

    /// The home page under the window's bar (`render_home_bar`): one
    /// centred column with the greeting, the search, the ways to start,
    /// the projects and the recent conversations.
    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ring = cx.theme().ring;
        let line = cx.theme().border;
        let hover = cx.theme().list_hover;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());

        // Looks like a field, opens the palette: there is one search. A
        // white field with a line, as every field is.
        let search = div()
            .id("home-search")
            .h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().input)
            .bg(cx.theme().popover)
            .cursor_pointer()
            .hover(|style| style.border_color(ring.opacity(0.6)))
            .child(surfaces::sized_icon(
                "search",
                IconSize::Inline,
                IconTone::Faded,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .text_color(muted)
                    .child(tr("Search everything")),
            )
            .child(surfaces::fact(
                if cfg!(target_os = "macos") {
                    "⌘K"
                } else {
                    "Ctrl+K"
                },
                cx,
            ))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)));

        // The ways to start, on one line: one filled, the usual two
        // outlined, and the rarer ones in a menu under More at the right.
        // The line does not wrap: a wrapping row was measured at its
        // narrowest and left a band of empty height under it.
        let starting = self.layers.contains(Layer::StartMenu);
        let mut more = div().relative().flex_shrink_0().child(
            surfaces::labelled_action(
                "home-more-start",
                "more",
                tr("More ways to start"),
                IconSize::Inline,
                if starting {
                    IconTone::Ink
                } else {
                    IconTone::Faded
                },
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.layers.close(Layer::StartMenu) {
                    this.layers.open(Layer::StartMenu);
                }
                cx.notify();
            })),
        );
        if starting {
            more = more.child(deferred(self.start_menu(cx)));
        }
        let start = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("new-project")
                    .icon(glyph("plus"))
                    .label(tr("New project"))
                    .primary()
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.open_new_project(window, cx))),
            )
            .child(
                Button::new("open-project")
                    .icon(glyph("folder"))
                    .label(tr("Open folder"))
                    .outline()
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.open_project(window, cx))),
            )
            .child(
                Button::new("home-import")
                    .icon(glyph("archive-import"))
                    .label(tr("Import"))
                    .outline()
                    .tooltip(tr("Import a project archive or folder package"))
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.import_project(window, cx))),
            )
            .child(div().flex_1())
            .child(more);

        // Projects are cells side by side, two to a line, with hairlines
        // between them and round the grid. Privacy mode hides the chosen
        // projects while recording.
        let mut cells = Vec::new();
        for project in self
            .projects
            .iter()
            .filter(|project| !home::concealed(&self.privacy, &project.id))
        {
            cells.push(self.project_cell(project, now, home_dir.as_deref(), cx));
        }
        let projects = (!cells.is_empty()).then(|| {
            let mut grid = div()
                .v_flex()
                .overflow_hidden()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(line);
            let count = cells.len();
            for (index, row) in surfaces::rows_of(cells, 2).into_iter().enumerate() {
                // A last cell alone takes the whole line: no empty bordered
                // half beside it.
                let short = !home::last_cell_spans(count, 2) && row.len() < 2;
                // A plain flex row stretches its cells to one height.
                let mut pair = div()
                    .flex()
                    .when(index > 0, |pair| pair.border_t_1().border_color(line));
                for (at, cell) in row.into_iter().enumerate() {
                    pair = pair.child(
                        cell.flex_1()
                            .min_w_0()
                            .when(at > 0, |cell| cell.border_l_1().border_color(line)),
                    );
                }
                if short {
                    // Keeps a lone cell the width of a column.
                    pair = pair.child(div().flex_1().border_l_1().border_color(line));
                }
                grid = grid.child(pair);
            }
            grid
        });

        // Recent conversations are record rows: when, a conversation's
        // icon, its project and title, its status as a stamp.
        let rows = self
            .recent
            .iter()
            .filter(|row| !home::concealed(&self.privacy, &row.project_id))
            .map(|row| {
                let project = row.project_id.clone();
                let session = row.id.clone();
                let mut record = surfaces::record_row(
                    SharedString::from(format!("recent-{session}")),
                    crate::present::title_line(&row.title),
                )
                .time(home::clock(now, row.activity_at))
                .icon("chat");
                if !row.project_name.is_empty() {
                    record = record.lead(row.project_name.clone());
                }
                if let Some((label, tone)) = present::session_status(&row.status)
                    && let Some(kind) = StampKind::from_tone(tone)
                {
                    record = record.stamp(kind, label);
                }
                record
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_session(project.clone(), session.clone(), window, cx)
                    }))
            })
            .collect::<Vec<_>>();
        let recent = if rows.is_empty() {
            div()
                .py_2()
                .text_color(muted)
                .child(tr("Conversations you start will appear here."))
        } else {
            surfaces::record_group(rows, cx)
        };

        let mut page = div()
            .v_flex()
            .w_full()
            .max_w(px(surfaces::HOME_WIDTH))
            .mx_auto()
            .px_8()
            .pt_8()
            .pb_10()
            .gap_6()
            .child(
                div()
                    .v_flex()
                    .gap_4()
                    .child(surfaces::display(tr(
                        "Rigorous research, with Wisp at your side.",
                    )))
                    .child(search)
                    .child(start),
            );
        if !self.settings.configured() {
            // A bar to act on: white, a line, its icon in the waiting
            // colour, the sentence and the way to Settings.
            page = page.child(
                surfaces::bar("home-configure", cx)
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        crate::assets::icon("circle-alert")
                            .text_color(surfaces::stamp_color(surfaces::StampKind::Waiting, cx)),
                    )
                    .child(div().flex_1().min_w_0().child(tr(
                        "Choose a model before you start: Wisp needs one to answer.",
                    )))
                    .child(
                        surfaces::outlined_action(
                            "home-configure-model",
                            "gear",
                            tr("Open Settings"),
                            cx,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                    ),
            );
        }
        if let Some(error) = &self.error {
            page = page.child(error_banner(error, cx));
        }
        if let Some(notice) = &self.notice {
            page = page.child(self.notice_banner(notice, cx));
        }
        // The home page has no sidebar to hold the card for a newer version.
        if let Some(card) = self.update_card(cx) {
            page = page.child(card);
        }
        div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .child(page.children(projects).child(recent))
    }
}
