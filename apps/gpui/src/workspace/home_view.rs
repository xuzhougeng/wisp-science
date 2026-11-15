//! The home page: every project and the most recent conversations. Removing
//! a project asks first, in a layer that Escape closes.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::ProjectSummary;

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    backend::receive,
    home,
    layers::Layer,
    present::{self, Tone},
};

fn section(title: &'static str, cx: &App) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

/// The shared look of a project or conversation card.
fn card(id: SharedString, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
        .py_3()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
}

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
            .child(Input::new(&self.delete_name_input).small())
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

    fn project_card(
        &self,
        project: &ProjectSummary,
        now: i64,
        home_dir: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = project.id.clone();
        let muted = cx.theme().muted_foreground;
        let ring = cx.theme().ring;
        let mut top = div().h_flex().gap_2().child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(project.name.clone()),
        );
        for (label, tone) in home::activity(project) {
            top = top.child(chip(label, tone, cx));
        }
        if let Some((label, tone)) = home::folder_badge(project) {
            top = top.child(chip(label, tone, cx));
        }
        top = top.child(div().flex_1()).child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .child(home::age(now, project.updated_at)),
        );
        let card = card(SharedString::from(format!("project-{id}")), cx)
            .v_flex()
            .gap_1()
            .child(top)
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(muted)
                    .child(home::short_path(&project.workspace_dir, home_dir)),
            );
        if self.removing.as_ref() == Some(&id) {
            if let Some(question) = &self.removing_folder {
                return card.child(self.render_delete_folder(question, cx));
            }
            // Not a link while the question is open: a stray click must not
            // open the project that is about to be removed.
            return card
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
        let mut actions = div().h_flex().gap_1().child(
            icon_button(
                SharedString::from(format!("star-{id}")),
                if starred { "star-filled" } else { "star" },
                if starred {
                    tr("Stop keeping first")
                } else {
                    tr("Keep first")
                },
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // The card underneath opens the project; this click is not that.
                cx.stop_propagation();
                this.toggle_star(star_id.clone(), !starred, window, cx)
            })),
        );
        let settings_id = id.clone();
        actions = actions.child(
            icon_button(
                SharedString::from(format!("project-settings-{id}")),
                "adjustments",
                tr("Project settings"),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_project_settings(settings_id.clone(), window, cx)
            })),
        );
        let window_id = id.clone();
        actions = actions.child(
            icon_button(
                SharedString::from(format!("project-window-{id}")),
                "dock",
                tr("Open in a new window"),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                crate::open_window_on(this.backend.clone(), Some(window_id.clone()), cx);
            })),
        );
        let (export_id, package_id) = (id.clone(), id.clone());
        actions = actions
            .child(
                icon_button(
                    SharedString::from(format!("project-export-{id}")),
                    "archive-export",
                    tr("Export the project as an archive"),
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.export_project(export_id.clone(), window, cx)
                })),
            )
            .child(
                icon_button(
                    SharedString::from(format!("project-package-{id}")),
                    "folder-move",
                    tr("Export the project as a folder another computer opens in place"),
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
                icon_button(
                    SharedString::from(format!("remove-{id}")),
                    "trash",
                    tr("Remove from Wisp"),
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.ask_remove_project(remove_id.clone(), cx)
                })),
            );
        }
        card.child(
            div()
                .h_flex()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(muted)
                        .child(home::counts(project)),
                )
                .child(actions),
        )
        .cursor_pointer()
        .hover(|style| style.border_color(ring.opacity(0.6)))
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
        let mut card = div()
            .id("new-project")
            .v_flex()
            .gap_3()
            .w(px(520.))
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("New project")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "A folder with this name is made where you choose next, and the project opens in it.",
            )))
            .child(Input::new(&self.new_project_name).small())
            .child(Input::new(&self.new_project_description).h(px(72.)))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("new-project-layout")
                            .label(if layout { tr("On") } else { tr("Off") })
                            .small()
                            .selected(layout)
                            .on_click(cx.listener(|this, _, _, cx| {
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
        card = card.child(
            div()
                .h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("new-project-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::NewProject);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("new-project-create")
                        .label(tr("Choose where and create"))
                        .small()
                        .primary()
                        .disabled(!ready)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_new_project(window, cx)),
                        ),
                ),
        );
        div()
            .id("new-project-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::NewProject);
                    cx.notify();
                }),
            )
            .child(card)
    }

    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ring = cx.theme().ring;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());

        let header = div()
            .h_flex()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .h_flex()
                            .gap(px(7.))
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().text_color(cx.theme().primary).child("Wisp"))
                            .child("Science"),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(tr("Rigorous research, with Wisp at your side.")),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(chip(tr("GPUI preview"), Tone::Neutral, cx))
                    .child(
                        icon_button("home-search", "search", tr("Search everything")).on_click(
                            cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)),
                        ),
                    )
                    .child(
                        icon_button(
                            "home-import",
                            "archive-import",
                            tr("Import a project archive or folder package"),
                        )
                        .disabled(self.pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_project(window, cx)),
                        ),
                    )
                    .child(
                        icon_button(
                            "home-join",
                            "sync",
                            tr("Join a project from another device"),
                        )
                        .disabled(self.pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_join_synced(window, cx)),
                        ),
                    )
                    .child(
                        icon_button(
                            "home-sample",
                            "flask",
                            "Make a sample project to try Wisp on",
                        )
                        .disabled(self.pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.create_sample_project(window, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_button("home-examples", "file-play", tr("Example conversations"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_examples(window, cx)),
                            ),
                    )
                    .child(
                        icon_button("home-calendar", "calendar", tr("Research calendar")).on_click(
                            cx.listener(|this, _, window, cx| this.open_calendar(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-schedules", "clock", tr("Scheduled tasks")).on_click(
                            cx.listener(|this, _, window, cx| this.open_schedules(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-assistant", "bot", tr("Research assistant")).on_click(
                            cx.listener(|this, _, window, cx| this.open_assistant(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-mandates", "gauge", tr("Research mandates")).on_click(
                            cx.listener(|this, _, window, cx| this.open_mandates(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-favorites", "star", tr("Favorites")).on_click(
                            cx.listener(|this, _, window, cx| this.open_favorites(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-settings", "gear", tr("Settings")).on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                    )
                    .child(
                        icon_button("home-docs", "book", tr("Documentation, in your browser"))
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(crate::links::DOCS))),
                    )
                    .child(
                        Button::new("new-project")
                            .icon(glyph("plus"))
                            .label(tr("New project"))
                            .disabled(self.pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_new_project(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("open-project")
                            .icon(glyph("folder-plus"))
                            .label(tr("Open folder"))
                            .primary()
                            .disabled(self.pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_project(window, cx)),
                            ),
                    ),
            );

        let mut projects = div()
            .v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(section(tr("Projects"), cx));
        // Privacy mode hides the chosen projects while recording.
        for project in self
            .projects
            .iter()
            .filter(|project| !home::concealed(&self.privacy, &project.id))
        {
            projects = projects.child(self.project_card(project, now, home_dir.as_deref(), cx));
        }

        let mut recent = div()
            .v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(section(tr("Recent conversations"), cx));
        if self.recent.is_empty() {
            recent = recent.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Conversations you start will appear here.")),
            );
        }
        for row in self
            .recent
            .iter()
            .filter(|row| !home::concealed(&self.privacy, &row.project_id))
        {
            let project = row.project_id.clone();
            let session = row.id.clone();
            let age = home::age(now, row.activity_at);
            let place = if row.project_name.is_empty() {
                age
            } else {
                format!("{} · {age}", row.project_name)
            };
            let mut line = card(SharedString::from(format!("recent-{session}")), cx)
                .h_flex()
                .gap_3()
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(crate::present::one_line(&row.title)))
                        .child(div().truncate().text_xs().text_color(muted).child(place)),
                );
            if let Some((label, tone)) = present::session_status(&row.status) {
                line = line.child(chip(label, tone, cx));
            }
            recent = recent.child(
                line.cursor_pointer()
                    .hover(|style| style.border_color(ring.opacity(0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_session(project.clone(), session.clone(), window, cx)
                    })),
            );
        }

        let mut page = div()
            .v_flex()
            .w_full()
            .max_w(px(1080.))
            .mx_auto()
            .px_8()
            .py_10()
            .gap_8()
            .child(header);
        if !self.settings.configured() {
            page = page.child(
                div()
                    .h_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(cx.theme().radius_lg)
                    .bg(cx.theme().muted)
                    .child(div().flex_1().text_sm().child(tr(
                        "Choose a model before you start: Wisp needs one to answer.",
                    )))
                    .child(
                        Button::new("home-configure-model")
                            .label(tr("Open Settings"))
                            .small()
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
        div().id("home").size_full().overflow_y_scroll().child(
            page.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_8()
                    .child(projects)
                    .child(recent),
            ),
        )
    }
}
