//! Project file browser beside the transcript: one directory at a time and a
//! read-only preview, both served by the host's confined project file access.
use crate::i18n::tr;
use std::ops::Range;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};

use super::{
    Workspace,
    favorites_view::image_format,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    backend::{Reply, receive},
    files::{self, Naming, PreviewMode},
    layers::Layer,
    panel::PanelTab,
};

impl Workspace {
    pub(super) fn toggle_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::FilesPanel) {
            self.files.close_preview();
            self.restore_focus(window, cx);
        } else {
            self.layers.open(Layer::FilesPanel);
            self.ensure_files(window, cx);
            self.reload_artifacts(window, cx);
        }
        cx.notify();
    }

    /// Lists the folder the panel is pointed at (the project root at first)
    /// once the panel is open and showing the listing.
    pub(super) fn ensure_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.contains(Layer::FilesPanel)
            && self.panel_tab == PanelTab::Files
            && !self.layers.contains(Layer::FilePreview)
            && !self.files.loaded
            && !self.files.loading
        {
            self.load_directory(self.files.path.clone(), window, cx);
        }
    }

    /// Opens a file found outside the panel: shows the panel on the file's
    /// folder with the preview over it.
    pub(super) fn reveal_file(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::FilePreview);
        self.files.close_preview();
        self.files
            .look_in(&files::parent(&path).unwrap_or_default());
        self.layers.open(Layer::FilesPanel);
        self.panel_tab = PanelTab::Files;
        self.open_file(path, window, cx);
    }

    /// Re-lists the open directory, for example after a turn wrote files.
    pub(super) fn reload_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.contains(Layer::FilesPanel) && !self.layers.contains(Layer::FilePreview) {
            self.load_directory(self.files.path.clone(), window, cx);
        }
    }

    /// Opens the name field for a new folder or file, or to rename.
    fn start_naming(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmTrashFile);
        self.file_trashing = None;
        self.file_problem = None;
        self.file_notice = None;
        let start = naming.start().to_owned();
        self.file_naming = Some(naming);
        self.layers.open(Layer::FileName);
        self.file_name_input.update(cx, |input, cx| {
            input.set_value(start, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Runs one change to the project's files, then lists the folder again.
    /// A refusal is shown and leaves whatever was open as it was.
    fn change_files(
        &mut self,
        reply: Reply<String>,
        then: impl FnOnce(&mut Self, String) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_problem = None;
        let project = self.state.project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // The answer is about the project that was open when asked.
                if this.state.project != project {
                    return;
                }
                match result {
                    Ok(path) => then(this, path),
                    Err(error) => this.file_problem = Some(error),
                }
                this.reload_files(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn save_file_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::FileName) {
            return;
        }
        let (Some(project), Some(naming)) = (self.state.project.clone(), self.file_naming.clone())
        else {
            return;
        };
        let name = self.file_name_input.read(cx).value().to_string();
        let folder = self.files.path.clone();
        let reply = match naming {
            Naming::Folder => self
                .backend
                .create_project_entry(project, folder, name, true),
            Naming::File => self
                .backend
                .create_project_entry(project, folder, name, false),
            Naming::Rename(path) => self.backend.rename_project_entry(project, path, name),
        };
        self.change_files(
            reply,
            |this, _| {
                this.layers.close(Layer::FileName);
                this.file_naming = None;
            },
            window,
            cx,
        );
    }

    fn ask_trash_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.layers.close(Layer::FileName);
        self.file_naming = None;
        self.file_problem = None;
        self.file_notice = None;
        self.file_trashing = Some(path);
        self.layers.open(Layer::ConfirmTrashFile);
        cx.notify();
    }

    fn trash_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmTrashFile) {
            return;
        }
        let (Some(project), Some(path)) = (self.state.project.clone(), self.file_trashing.clone())
        else {
            return;
        };
        let reply = self.backend.trash_project_entry(project, path);
        self.change_files(
            reply,
            |this, to| {
                this.layers.close(Layer::ConfirmTrashFile);
                this.file_trashing = None;
                this.file_notice = Some(files::trashed_note(&to));
            },
            window,
            cx,
        );
    }

    fn load_directory(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.files.begin(&path) else {
            return;
        };
        let reply = self.backend.project_directory(ticket.project.clone(), path);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                let applied = match result {
                    Ok(directory) => this.files.accept_directory(&ticket, directory),
                    Err(error) => this.files.fail(&ticket, error),
                };
                if applied {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn open_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.files.begin_preview(&path) else {
            return;
        };
        self.layers.open(Layer::FilePreview);
        self.file_image = None;
        self.file_problem = None;
        // A picture is fetched whole, beside the text preview that tells
        // its size.
        if crate::favorites::is_image(&path) {
            let reply = self
                .backend
                .project_image(ticket.project.clone(), path.clone());
            let wanted = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    // Another file, or none, is being looked at by now.
                    if this.files.preview_path.as_deref() != Some(wanted.as_str()) {
                        return;
                    }
                    match result {
                        Ok((kind, bytes)) => {
                            this.file_image = image_format(Some(&kind)).map(|format| {
                                (
                                    wanted,
                                    std::sync::Arc::new(Image::from_bytes(format, bytes)),
                                )
                            });
                            if this.file_image.is_none() {
                                this.file_problem =
                                    Some(tr("This kind of picture cannot be shown here.").into());
                            }
                        }
                        Err(error) => this.file_problem = Some(error),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let reply = self.backend.project_file(ticket.project.clone(), path);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let applied = match result {
                    Ok(preview) => this.files.accept_preview(&ticket, preview),
                    Err(error) => this.files.fail(&ticket, error),
                };
                if !applied {
                    return;
                }
                if let Some(content) = &this.files.preview {
                    if files::preview_mode(content) == PreviewMode::Text {
                        let text = content.text.clone().unwrap_or_default();
                        this.file_view
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn close_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::FilePreview) {
            self.files.close_preview();
            self.file_image = None;
            self.file_problem = None;
            self.file_origin = None;
            self.restore_focus(window, cx);
            // A file revealed from the palette has no listing under it yet.
            self.ensure_files(window, cx);
            cx.notify();
        }
    }

    fn file_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(entry) = self.files.entries.get(index) else {
            return div().into_any_element();
        };
        let path = files::join(&self.files.path, &entry.name);
        let is_dir = entry.is_dir;
        let hover = cx.theme().list_hover;
        let (name, detail) = if is_dir {
            (format!("{}/", entry.name), tr("Folder").to_string())
        } else {
            (entry.name.clone(), files::size_label(entry.size))
        };
        let (renamed, trashed) = (path.clone(), path.clone());
        div()
            .id(("file-entry", index))
            .h_flex()
            .w_full()
            .h(px(28.))
            .px_3()
            .gap_2()
            .justify_between()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
            .child(
                div()
                    .h_flex()
                    .flex_shrink_0()
                    .child(
                        icon_button(("file-rename", index), "edit", tr("Rename")).on_click(
                            cx.listener(move |this, _, window, cx| {
                                // The row underneath opens the entry.
                                cx.stop_propagation();
                                this.start_naming(Naming::Rename(renamed.clone()), window, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_button(
                            ("file-trash", index),
                            "trash",
                            tr("Move to the project's trash"),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.ask_trash_file(trashed.clone(), cx)
                        })),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // What was said about the last change is about another place now.
                this.file_notice = None;
                this.file_problem = None;
                if is_dir {
                    this.load_directory(path.clone(), window, cx);
                } else {
                    this.open_file(path.clone(), window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_preview(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let message = |text: &'static str| {
            div()
                .p_4()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        if let Some((_, image)) = self
            .file_image
            .as_ref()
            .filter(|(path, _)| self.files.preview_path.as_deref() == Some(path.as_str()))
        {
            return div()
                .flex_1()
                .min_h_0()
                .p_3()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image.clone())
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element();
        }
        let Some(content) = &self.files.preview else {
            return match &self.files.error {
                Some(error) => div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
                    .into_any_element(),
                None => message(tr("Loading preview…")),
            };
        };
        match files::preview_mode(content) {
            PreviewMode::Binary if crate::favorites::is_image(&content.path) => {
                match &self.file_problem {
                    Some(problem) => div()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(problem.clone())
                        .into_any_element(),
                    None => message(tr("Loading the picture…")),
                }
            }
            PreviewMode::Binary => message(tr("This file is not text. Preview is not available.")),
            PreviewMode::Markdown => div()
                .flex_1()
                .min_h_0()
                .p_4()
                .child(
                    TextView::markdown(
                        SharedString::from(format!("file-preview-{}", content.path)),
                        content.text.clone().unwrap_or_default(),
                        window,
                        cx,
                    )
                    .selectable(true)
                    .scrollable(true),
                )
                .into_any_element(),
            // Disabled keeps selection, copy and keyboard scrolling but no edits.
            PreviewMode::Text => div()
                .flex_1()
                .min_h_0()
                .child(Input::new(&self.file_view).disabled(true).h_full())
                .into_any_element(),
        }
    }

    /// Looks up which tool call wrote the file being previewed.
    fn load_file_origin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(path)) =
            (self.state.project.clone(), self.files.preview_path.clone())
        else {
            return;
        };
        let reply = self
            .backend
            .file_origin(project, self.state.session.clone(), path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Another file may be in the preview by now.
                if this.files.preview_path.as_deref() == Some(path.as_str()) {
                    this.file_origin = Some((path, result));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Where the previewed file came from, once it was asked.
    fn render_file_origin(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let (path, found) = self.file_origin.as_ref()?;
        if self.files.preview_path.as_deref() != Some(path.as_str()) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let mut block = div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border);
        match found {
            Err(problem) => {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(problem.clone()),
                );
            }
            Ok(origin) => {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(files::origin_line(origin.as_ref())),
                );
                if let Some(origin) = origin {
                    let detail = files::origin_detail(origin);
                    if !detail.is_empty() {
                        block = block.child(
                            div().max_h(px(220.)).overflow_hidden().child(
                                gpui_component::text::TextView::markdown(
                                    "file-origin-detail",
                                    detail,
                                    window,
                                    cx,
                                )
                                .selectable(true),
                            ),
                        );
                    }
                    if let Some(project) = self.state.project.clone() {
                        let session = origin.session.clone();
                        block = block.child(
                            div().h_flex().child(
                                Button::new("file-origin-open")
                                    .label(tr("Open the conversation"))
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_session(
                                            project.clone(),
                                            session.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                        );
                    }
                }
            }
        }
        Some(block)
    }

    pub(super) fn render_files(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let previewing = self.layers.contains(Layer::FilePreview);
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let panel = div()
            .v_flex()
            .w(px(if previewing { 480. } else { 340. }))
            .min_w(px(240.))
            .h_full()
            .border_l_1()
            .border_color(border);

        if previewing {
            let name = self.files.preview_path.as_deref().unwrap_or_default();
            let name = files::file_name(name).to_string();
            let note = self
                .files
                .preview
                .as_ref()
                .map(files::preview_note)
                .unwrap_or_default();
            return panel
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .p_3()
                        .justify_between()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().text_sm().child(name))
                                .child(div().text_xs().text_color(muted).child(note)),
                        )
                        .child(
                            Button::new("file-preview-origin")
                                .label(tr("Origin"))
                                .small()
                                .ghost()
                                .tooltip(tr(
                                    "Which tool call wrote this file, and in which conversation",
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.load_file_origin(window, cx)
                                })),
                        )
                        .child(
                            Button::new("file-preview-back")
                                .label(tr("Back to files"))
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_preview(window, cx)
                                })),
                        ),
                )
                .children(self.render_file_origin(window, cx))
                .child(self.render_preview(window, cx))
                .into_any_element();
        }

        let panel = panel.child(self.panel_tabs(cx));
        match self.panel_tab {
            PanelTab::Runs => {
                return panel
                    .child(self.render_runs_tab(window, cx))
                    .into_any_element();
            }
            PanelTab::Artifacts => {
                return panel
                    .child(self.render_artifacts_tab(cx))
                    .into_any_element();
            }
            PanelTab::Memos => {
                return panel.child(self.render_memos_tab(cx)).into_any_element();
            }
            PanelTab::Memory => {
                return panel.child(self.render_memory_tab(cx)).into_any_element();
            }
            PanelTab::Highlights => {
                return panel
                    .child(self.render_highlights_tab(cx))
                    .into_any_element();
            }
            PanelTab::Notebook => {
                return panel
                    .child(self.render_notebook_tab(window, cx))
                    .into_any_element();
            }
            PanelTab::Agents => {
                return panel
                    .child(self.render_agents_tab(window, cx))
                    .into_any_element();
            }
            PanelTab::Aside => {
                return panel
                    .child(self.render_aside_tab(window, cx))
                    .into_any_element();
            }
            PanelTab::Files => {}
        }
        let panel = panel.children(self.render_locations(cx));
        if let Some(remote) = self.render_remote_files(cx) {
            return panel.child(remote).into_any_element();
        }
        let location = if self.files.path.is_empty() {
            tr("Project root").to_string()
        } else {
            self.files.path.clone()
        };
        let parent = files::parent(&self.files.path);
        let mut panel = panel.child(
            div()
                .v_flex()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(border)
                .child(div().truncate().text_xs().text_color(muted).child(location))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("files-up")
                                .label(tr("Up"))
                                .small()
                                .disabled(parent.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(parent) = parent.clone() {
                                        this.load_directory(parent, window, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new("files-reload")
                                .label(tr("Reload"))
                                .small()
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.reload_files(window, cx)
                                    }),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                "files-terminal",
                                "terminal",
                                "Open a terminal in this folder",
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.open_terminal(cx))),
                        )
                        .child(
                            icon_button("files-new-folder", "folder-plus", tr("New folder here"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_naming(Naming::Folder, window, cx)
                                })),
                        )
                        .child(
                            icon_button("files-new-file", "note-plus", tr("New file here"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_naming(Naming::File, window, cx)
                                })),
                        ),
                ),
        );
        if let Some(problem) = &self.file_problem {
            panel = panel.child(div().px_3().pt_2().child(error_banner(problem, cx)));
        }
        if let Some(naming) = self
            .file_naming
            .as_ref()
            .filter(|_| self.layers.contains(Layer::FileName))
        {
            panel = panel.child(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_xs().text_color(muted).child(naming.title()))
                    .child(Input::new(&self.file_name_input).small())
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("file-name-save")
                                    .label(tr("Save"))
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_file_name(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("file-name-cancel")
                                    .label(tr("Cancel"))
                                    .small()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.layers.close(Layer::FileName);
                                        this.file_naming = None;
                                        this.file_problem = None;
                                        this.focus_composer(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Some(path) = self
            .file_trashing
            .as_ref()
            .filter(|_| self.layers.contains(Layer::ConfirmTrashFile))
        {
            panel = panel.child(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_sm().child(crate::i18n::fill(tr("Move {} to the project's trash? It is kept in .wisp/trash, not erased."), &[&files::file_name(path)])))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("file-trash-confirm")
                                    .icon(glyph("trash"))
                                    .label(tr("Move to trash"))
                                    .small()
                                    .danger()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.trash_file(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("file-trash-cancel")
                                    .label(tr("Keep"))
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.layers.close(Layer::ConfirmTrashFile);
                                        this.file_trashing = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Some(notice) = &self.file_notice {
            panel = panel.child(
                div()
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(notice.clone()),
            );
        }
        if let Some(error) = &self.files.error {
            panel = panel.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        let hint = if self.files.loading && self.files.entries.is_empty() {
            Some(tr("Loading…"))
        } else if self.files.loaded && self.files.entries.is_empty() {
            Some(tr("This folder is empty."))
        } else {
            None
        };
        if let Some(hint) = hint {
            return panel
                .child(div().p_3().text_sm().text_color(muted).child(hint))
                .into_any_element();
        }
        panel
            .child(
                uniform_list(
                    "file-entries",
                    self.files.entries.len(),
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        range
                            .map(|index| this.file_row(index, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}
