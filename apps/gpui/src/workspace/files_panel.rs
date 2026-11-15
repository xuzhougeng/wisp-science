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
    /// What can be done with the previewed file outside the preview: its
    /// path copied, the file attached to the message, shown in the file
    /// manager or opened with the system's own application.
    fn file_actions(&self, cx: &mut Context<Self>) -> Option<Div> {
        let relative = self.files.preview_path.clone()?;
        let root = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| std::path::PathBuf::from(&project.workspace_dir))?;
        let path = files::on_disk(&root, &relative)?;
        let (copied, shown, opened) = (path.clone(), path.clone(), path);
        let mut actions = div().h_flex().flex_shrink_0().child(
            icon_button("file-copy-path", "copy", "Copy the file's path").on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        copied.to_string_lossy().into_owned(),
                    ));
                    this.file_notice = Some(tr("Path copied.").to_owned());
                    cx.notify();
                },
            )),
        );
        // Attaching and keeping need a conversation to belong to.
        if self.state.session.is_some() {
            let kept = relative.clone();
            actions = actions
                .child(
                    icon_button("file-attach", "attach", "Attach to the message").on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.attach(crate::compose::Chip::File(relative.clone()), cx);
                            this.file_notice = Some(tr("Attached to the message.").to_owned());
                        }),
                    ),
                )
                .child(
                    icon_button(
                        "file-keep-artifact",
                        "layers",
                        "Keep as an artifact of this conversation",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.keep_as_artifact(kept.clone(), window, cx)
                    })),
                );
        }
        Some(
            actions
                .child(
                    icon_button("file-reveal", "folder", "Show in the file manager")
                        .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&shown))),
                )
                .child(
                    icon_button(
                        "file-open-system",
                        "external-link",
                        "Open with the system's application",
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&opened))),
                ),
        )
    }

    /// Keeps the previewed file as an artifact of the open conversation.
    fn keep_as_artifact(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let reply = self.backend.register_artifact(project, session, path);
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::backend::receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        // An unchanged file is the same version as before,
                        // listed under the conversation that first kept it.
                        this.file_notice = Some(
                            tr("Kept as an artifact. If it is unchanged since it was first kept, it stays under that conversation.")
                                .to_owned(),
                        );
                        this.reload_artifacts(window, cx);
                    }
                    Err(error) => this.file_notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Looks for files named like what is typed in the Files panel, after
    /// a pause so fast typing costs one search.
    pub(super) fn search_files_by_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.file_search.read(cx).value().to_string();
        let Some((ticket, query)) = self.files.search.begin(&typed) else {
            cx.notify();
            return;
        };
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(150))
                .await;
            let current = this
                .update(cx, |this, _| this.files.search.query == query)
                .unwrap_or(false);
            if !current {
                return;
            }
            let found = receive(backend.find_files(Some(project), query, files::NAME_HITS)).await;
            let _ = this.update(cx, |this, cx| {
                match found {
                    Ok(found) => {
                        let paths = found.into_iter().map(|hit| hit.path).collect();
                        this.files.search.accept(ticket, paths);
                    }
                    Err(error) => this.files.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// One file found by name: its name, and the folder it is in.
    fn file_hit_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(path) = self
            .files
            .search
            .hits
            .as_ref()
            .and_then(|hits| hits.get(index))
            .cloned()
        else {
            return div().into_any_element();
        };
        let folder = files::parent(&path).unwrap_or_default();
        let hover = cx.theme().list_hover;
        div()
            .id(("file-hit", index))
            .h_flex()
            .w_full()
            .h(px(28.))
            .px_3()
            .gap_2()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(180.))
                    .truncate()
                    .text_sm()
                    .child(files::file_name(&path).to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if folder.is_empty() {
                        tr("Project root").to_owned()
                    } else {
                        folder
                    }),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.reveal_file(path.clone(), window, cx)),
            )
            .into_any_element()
    }

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
        self.file_document = None;
        self.file_pages = None;
        self.file_sequences = None;
        self.file_structure = None;
        self.file_problem = None;
        self.file_as_text = false;
        // A PDF's first pages are drawn, off the window's thread.
        if crate::pdf_pages::is_pdf(&path) {
            let reply = self
                .backend
                .project_pdf(ticket.project.clone(), path.clone());
            let wanted = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let drawn = match receive(reply).await {
                    Ok(bytes) => {
                        cx.background_executor()
                            .spawn(async move {
                                crate::pdf_pages::draw(
                                    bytes,
                                    crate::pdf_pages::SHOWN,
                                    crate::pdf_pages::SCALE,
                                )
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                let _ = this.update(cx, |this, cx| {
                    if this.files.preview_path.as_deref() != Some(wanted.as_str()) {
                        return;
                    }
                    let pages = drawn.map(|drawn| {
                        let note = crate::pdf_pages::note(&drawn);
                        let images = drawn
                            .pages
                            .into_iter()
                            .map(|png| {
                                let ratio = crate::pdf_pages::png_size(&png)
                                    .map_or(1.414, |(width, height)| {
                                        height as f32 / width.max(1) as f32
                                    });
                                (
                                    std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                                    ratio,
                                )
                            })
                            .collect();
                        (images, note)
                    });
                    this.file_pages = Some((wanted, pages));
                    cx.notify();
                });
            })
            .detach();
        }
        // A document is turned into text by the host, beside the preview
        // that tells its size.
        if files::is_document(&path) {
            let reply = self
                .backend
                .document_preview(ticket.project.clone(), path.clone());
            let wanted = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    if this.files.preview_path.as_deref() != Some(wanted.as_str()) {
                        return;
                    }
                    let read = match result {
                        Ok(Some(text)) => Ok(text),
                        Ok(None) => {
                            Err(tr("This file is not text. Preview is not available.").into())
                        }
                        Err(error) => Err(error),
                    };
                    this.file_document = Some((wanted, read));
                    cx.notify();
                });
            })
            .detach();
        }
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
                        // An alignment or FASTA file is read once, not at
                        // every frame.
                        this.file_sequences = crate::sequences::parse(&content.path, &text)
                            .map(|read| (content.path.clone(), std::sync::Arc::new(read)));
                        this.sequence_page = 0;
                        this.file_structure = crate::molecules::parse(&content.path, &text)
                            .map(|read| (content.path.clone(), std::sync::Arc::new(read)));
                        this.structure_turn = (0.0, 0.0);
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
            self.file_as_text = false;
            // A note about the previewed file is not about the list.
            self.file_notice = None;
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
                    .child(crate::i18n::host(error))
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
                        .child(crate::i18n::host(problem))
                        .into_any_element(),
                    None => message(tr("Loading the picture…")),
                }
            }
            PreviewMode::Binary
                if crate::pdf_pages::is_pdf(&content.path) && !self.file_as_text =>
            {
                self.render_pdf_pages(&content.path, cx)
            }
            PreviewMode::Binary if files::is_document(&content.path) => {
                match self
                    .file_document
                    .as_ref()
                    .filter(|(path, _)| *path == content.path)
                {
                    None => message(tr("Turning the document into text…")),
                    Some((_, Err(problem))) => div()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(problem))
                        .into_any_element(),
                    Some((path, Ok(text))) => div()
                        .v_flex()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .px_4()
                                .pt_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if crate::pdf_pages::is_pdf(path) {
                                            tr("The PDF's text and tables, to read and copy.")
                                        } else {
                                            tr("The document's text and tables, to read; its layout and pictures are not shown. Open it with the system's application for the original.")
                                        }),
                                )
                                .when(crate::pdf_pages::is_pdf(path), |bar| {
                                    bar.child(
                                        Button::new("file-pdf-pages")
                                            .label(tr("Show the pages"))
                                            .xsmall()
                                            .ghost()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.file_as_text = false;
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .child(
                            div().flex_1().min_h_0().p_4().child(
                                TextView::markdown(
                                    SharedString::from(format!("document-{path}")),
                                    text.clone(),
                                    window,
                                    cx,
                                )
                                .selectable(true)
                                .scrollable(true),
                            ),
                        )
                        .into_any_element(),
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
            PreviewMode::Text => {
                let text = content.text.as_deref().unwrap_or_default();
                if let Some((_, read)) = self
                    .file_sequences
                    .as_ref()
                    .filter(|(path, _)| *path == content.path)
                {
                    return self.render_sequences(read.clone(), cx);
                }
                if let Some((_, read)) = self
                    .file_structure
                    .as_ref()
                    .filter(|(path, _)| *path == content.path)
                {
                    return self.render_structure(read.clone(), cx);
                }
                if files::html_page(content) {
                    return self.render_html(&content.path, text, window, cx);
                }
                if let Some(book) = files::notebook(&content.path, text) {
                    return self.render_notebook(&content.path, book, window, cx);
                }
                match files::table(&content.path, text) {
                    Some(table) => self.render_table(&table, cx),
                    None => self.render_text_preview(),
                }
            }
        }
    }

    /// Verbatim text. Disabled keeps selection, copy and keyboard scrolling
    /// but no edits.
    fn render_text_preview(&self) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .child(Input::new(&self.file_view).disabled(true).h_full())
            .into_any_element()
    }

    /// An HTML file as a page, the way a document is laid out, with a
    /// switch to its source. Nothing in it runs or is fetched: scripts and
    /// styles are left out, and the app has no network client for pictures.
    fn render_html(
        &self,
        path: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let switch = Button::new("file-html-switch")
            .label(if self.file_as_text {
                tr("Show the page")
            } else {
                tr("Show the source")
            })
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
                this.file_as_text = !this.file_as_text;
                cx.notify();
            }));
        let bar = div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(
                        "The page's text, headings, lists, tables and links; its scripts do not run and its styles and pictures are not loaded.",
                    )),
            )
            .child(switch);
        let body = if self.file_as_text {
            self.render_text_preview()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .p_4()
                .child(
                    TextView::html(
                        SharedString::from(format!("file-html-{path}")),
                        text.to_owned(),
                        window,
                        cx,
                    )
                    .selectable(true)
                    .scrollable(true),
                )
                .into_any_element()
        };
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(bar)
            .child(body)
            .into_any_element()
    }

    /// A molecule as its atoms and bonds, or a structure as the trace of
    /// its chains, drawn flat and turned by dragging; with a switch to its
    /// text.
    fn render_structure(
        &self,
        read: std::sync::Arc<crate::molecules::Structure>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::molecules;
        let muted = cx.theme().muted_foreground;
        let switch = Button::new("file-structure-switch")
            .label(if self.file_as_text {
                tr("Show the drawing")
            } else {
                tr("Show as text")
            })
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
                this.file_as_text = !this.file_as_text;
                cx.notify();
            }));
        if self.file_as_text {
            return div()
                .v_flex()
                .flex_1()
                .min_h_0()
                .child(div().h_flex().px_3().py_1().child(switch))
                .child(self.render_text_preview())
                .into_any_element();
        }
        let (yaw, pitch) = self.structure_turn;
        let bond_colour = cx.theme().foreground.opacity(0.7);
        let dot = cx.theme().foreground;
        let drawing = read.clone();
        let shown = read.clone();
        let picture = canvas(
            move |bounds, _, _| {
                molecules::project(
                    &drawing,
                    yaw,
                    pitch,
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                )
            },
            move |bounds, points, window, _| {
                let at = |index: usize| {
                    let (x, y, _) = points[index];
                    point(bounds.origin.x + px(x), bounds.origin.y + px(y))
                };
                // Bonds of one colour go in one path: a chain each when
                // traced, all together for a small molecule.
                let mut groups: Vec<(String, PathBuilder)> = Vec::new();
                // The inner line of an aromatic bond, drawn lighter.
                let mut aromatic = PathBuilder::stroke(px(1.));
                let mut any_aromatic = false;
                for (bond, &(first, second)) in shown.bonds.iter().enumerate() {
                    let chain = if shown.trace {
                        shown.atoms[first].chain.clone()
                    } else {
                        String::new()
                    };
                    let index = match groups.iter().position(|(kept, _)| *kept == chain) {
                        Some(index) => index,
                        None => {
                            groups.push((
                                chain,
                                PathBuilder::stroke(px(if shown.trace { 2. } else { 1.5 })),
                            ));
                            groups.len() - 1
                        }
                    };
                    let path = &mut groups[index].1;
                    let (from, to) = (at(first), at(second));
                    let order = match shown.trace {
                        true => 1,
                        false => shown.orders.get(bond).copied().unwrap_or(1),
                    };
                    // Across the bond, for the lines of a double or triple
                    // one.
                    let (dx, dy) = (f32::from(to.x - from.x), f32::from(to.y - from.y));
                    let length = (dx * dx + dy * dy).sqrt().max(0.001);
                    let (nx, ny) = (-dy / length, dx / length);
                    let offsets: &[f32] = match order {
                        2 => &[-1.8, 1.8],
                        3 => &[-2.8, 0.0, 2.8],
                        _ => &[0.0],
                    };
                    for offset in offsets {
                        path.move_to(point(from.x + px(nx * offset), from.y + px(ny * offset)));
                        path.line_to(point(to.x + px(nx * offset), to.y + px(ny * offset)));
                    }
                    if order == 4 {
                        any_aromatic = true;
                        let along = |part: f32| {
                            point(
                                from.x + px(dx * part + nx * 3.0),
                                from.y + px(dy * part + ny * 3.0),
                            )
                        };
                        aromatic.move_to(along(0.2));
                        aromatic.line_to(along(0.8));
                    }
                }
                if any_aromatic {
                    if let Ok(path) = aromatic.build() {
                        window.paint_path(path, bond_colour.opacity(0.5));
                    }
                }
                for (number, (_, path)) in groups.into_iter().enumerate() {
                    let colour = if shown.trace {
                        hsla((number as f32 * 0.17) % 1.0, 0.6, 0.5, 1.0)
                    } else {
                        bond_colour
                    };
                    if let Ok(path) = path.build() {
                        window.paint_path(path, colour);
                    }
                }
                if shown.trace {
                    return;
                }
                // The far atoms first, so the near ones cover them.
                let mut order: Vec<usize> = (0..shown.atoms.len()).collect();
                order.sort_by(|a, b| points[*a].2.total_cmp(&points[*b].2));
                for index in order {
                    let atom = &shown.atoms[index];
                    let (colour, radius) = match molecules::element_colour(&atom.element) {
                        Some((hue, lightness)) => (hsla(hue, 0.7, lightness, 1.0), 4.),
                        None => (dot, 2.),
                    };
                    let centre = at(index);
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(centre.x - px(radius), centre.y - px(radius)),
                                size(px(radius * 2.), px(radius * 2.)),
                            ),
                            colour,
                        )
                        .corner_radii(px(radius)),
                    );
                }
            },
        )
        .size_full();
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(molecules::note(&read)),
                    )
                    .child(
                        Button::new("file-structure-reset")
                            .label(tr("Face on"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.structure_turn = (0.0, 0.0);
                                cx.notify();
                            })),
                    )
                    .child(switch),
            )
            .child(
                div()
                    .id("file-structure")
                    .flex_1()
                    .min_h_0()
                    .m_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .cursor_grab()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.structure_drag = Some(event.position);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            this.structure_drag = None;
                            return;
                        }
                        if let Some(last) = this.structure_drag {
                            let (yaw, pitch) = this.structure_turn;
                            this.structure_turn = (
                                yaw + f32::from(event.position.x - last.x) * 0.01,
                                (pitch + f32::from(event.position.y - last.y) * 0.01)
                                    .clamp(-1.5, 1.5),
                            );
                            cx.notify();
                        }
                        this.structure_drag = Some(event.position);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, _| this.structure_drag = None),
                    )
                    .child(picture),
            )
            .into_any_element()
    }

    /// An alignment or FASTA file as rows of residues coloured by kind,
    /// a hundred columns at a time, with a switch to its text.
    fn render_sequences(
        &self,
        read: std::sync::Arc<crate::sequences::Alignment>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::sequences::{self, Residue};
        const PAGE: usize = 100;
        let muted = cx.theme().muted_foreground;
        let switch = Button::new("file-sequences-switch")
            .label(if self.file_as_text {
                tr("Show as sequences")
            } else {
                tr("Show as text")
            })
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
                this.file_as_text = !this.file_as_text;
                cx.notify();
            }));
        if self.file_as_text {
            return div()
                .v_flex()
                .flex_1()
                .min_h_0()
                .child(div().h_flex().px_3().py_1().child(switch))
                .child(self.render_text_preview())
                .into_any_element();
        }
        let shown = read.rows.iter().map(String::len).max().unwrap_or(0);
        let pages = shown.div_ceil(PAGE).max(1);
        let page = self.sequence_page.min(pages - 1);
        let (from, to) = (page * PAGE, ((page + 1) * PAGE).min(shown));
        // Translucent, so the letters read on light and dark alike.
        let colour = |kind: Residue| -> Option<Hsla> {
            let hue = match kind {
                Residue::Hydrophobic | Residue::Cytosine => 0.6,
                Residue::Aromatic => 0.5,
                Residue::Positive | Residue::Thymine => 0.0,
                Residue::Negative => 0.83,
                Residue::Polar | Residue::Adenine => 0.33,
                Residue::Glycine | Residue::Guanine => 0.08,
                Residue::Proline => 0.15,
                Residue::Cysteine => 0.92,
                Residue::Gap | Residue::Other => return None,
            };
            Some(hsla(hue, 0.65, 0.55, 0.35))
        };
        let mut bar = div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(muted)
                    .child(sequences::note(&read)),
            )
            .child(switch);
        if pages > 1 {
            bar = bar
                .child(
                    icon_button("file-sequences-back", "chevron-left", "Earlier columns")
                        .disabled(page == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sequence_page = this.sequence_page.saturating_sub(1);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::fill(tr("Columns {}–{}"), &[&(from + 1), &to])),
                )
                .child(
                    icon_button("file-sequences-next", "chevron-right", "Later columns")
                        .disabled(page + 1 >= pages)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sequence_page = (page + 1).min(pages - 1);
                            cx.notify();
                        })),
                );
        }
        let mut names = div().v_flex().flex_shrink_0().w(px(120.)).pr_2();
        let mut rows = div().v_flex();
        for (name, row) in read.names.iter().zip(&read.rows) {
            names = names.child(
                div()
                    .h(px(18.))
                    .truncate()
                    .text_color(muted)
                    .child(name.clone()),
            );
            let part = row
                .get(from.min(row.len())..to.min(row.len()))
                .unwrap_or_default()
                .to_owned();
            let highlights: Vec<_> = sequences::runs(&part, read.nucleic)
                .into_iter()
                .filter_map(|(range, kind)| {
                    colour(kind).map(|background| {
                        (
                            range,
                            HighlightStyle {
                                background_color: Some(background),
                                ..Default::default()
                            },
                        )
                    })
                })
                .collect();
            rows = rows.child(
                div()
                    .h(px(18.))
                    .whitespace_nowrap()
                    .child(StyledText::new(part).with_highlights(highlights)),
            );
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(bar)
            .child(
                div()
                    .id("file-sequences")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .px_3()
                    .py_2()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(div().h_flex().items_start().child(names).child(rows)),
            )
            .into_any_element()
    }

    /// A PDF's first pages, drawn, with a switch to its text.
    fn render_pdf_pages(&self, path: &str, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let switch = Button::new("file-pdf-switch")
            .label(tr("Show the text"))
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
                this.file_as_text = true;
                cx.notify();
            }));
        let pages = self
            .file_pages
            .as_ref()
            .filter(|(shown, _)| shown == path)
            .map(|(_, pages)| pages);
        let (images, note) = match pages {
            None => {
                return div()
                    .p_4()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Drawing the pages…"))
                    .into_any_element();
            }
            Some(Err(problem)) => {
                return div()
                    .v_flex()
                    .gap_2()
                    .p_4()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(crate::i18n::host(problem)),
                    )
                    .child(div().child(switch))
                    .into_any_element();
            }
            Some(Ok(drawn)) => drawn,
        };
        let mut list = div()
            .id("file-pdf-pages")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .p_3()
            .overflow_y_scroll();
        for (index, (image, ratio)) in images.iter().enumerate() {
            list = list.child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        // As wide as the panel and as tall as the page is
                        // for that width: padding in a fraction of the width.
                        div()
                            .w_full()
                            .relative()
                            .pb(relative(*ratio))
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                img(image.clone())
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(crate::i18n::fill(tr("Page {}"), &[&(index + 1)])),
                    ),
            );
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(note.clone()),
                    )
                    .child(switch),
            )
            .child(list)
            .into_any_element()
    }

    /// A notebook as its cells and outputs, with a switch to its text.
    fn render_notebook(
        &self,
        path: &str,
        book: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bar = div().h_flex().px_3().py_1().child(
            Button::new("file-notebook-switch")
                .label(if self.file_as_text {
                    tr("Show as a notebook")
                } else {
                    tr("Show as text")
                })
                .xsmall()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.file_as_text = !this.file_as_text;
                    cx.notify();
                })),
        );
        let body = if self.file_as_text {
            self.render_text_preview()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .p_4()
                .child(
                    TextView::markdown(
                        SharedString::from(format!("notebook-{path}")),
                        book,
                        window,
                        cx,
                    )
                    .selectable(true)
                    .scrollable(true),
                )
                .into_any_element()
        };
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(bar)
            .child(body)
            .into_any_element()
    }

    /// A delimited file as a grid, with a switch to its text.
    fn render_table(&self, table: &files::Table, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut bar = div().h_flex().gap_3().px_3().py_1().child(
            Button::new("file-table-switch")
                .label(if self.file_as_text {
                    tr("Show as a table")
                } else {
                    tr("Show as text")
                })
                .xsmall()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.file_as_text = !this.file_as_text;
                    cx.notify();
                })),
        );
        if self.file_as_text {
            return div()
                .v_flex()
                .flex_1()
                .min_h_0()
                .child(bar)
                .child(self.render_text_preview())
                .into_any_element();
        }
        if let Some(note) = files::table_note(table) {
            bar = bar.child(div().text_xs().text_color(muted).child(note));
        }
        let line = |cells: &[String], head: bool| {
            let mut row = div().h_flex().border_b_1().border_color(border);
            for cell in cells {
                let mut cell = div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .truncate()
                    .text_xs()
                    .child(cell.lines().next().unwrap_or_default().to_owned());
                if head {
                    cell = cell.font_weight(FontWeight::SEMIBOLD);
                }
                row = row.child(cell);
            }
            row
        };
        let mut grid = div()
            .id("file-table")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .font_family(cx.theme().mono_font_family.clone())
            .child(line(&table.header, true));
        for row in &table.rows {
            grid = grid.child(line(row, false));
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(bar)
            .child(grid)
            .into_any_element()
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
                        .child(crate::i18n::host(problem)),
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
                        .children(self.file_actions(cx))
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
                // What an action on the file just did.
                .children(self.file_notice.as_ref().map(|notice| {
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(muted)
                        .child(notice.clone())
                }))
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
                        .child(
                            Button::new("files-sort")
                                .icon(glyph("sort"))
                                .label(self.files.sort.label())
                                .small()
                                .ghost()
                                .tooltip(tr("Change the order"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let next = this.files.sort.next();
                                    this.files.set_sort(next);
                                    this.save_files_sort(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                "files-terminal",
                                "terminal",
                                "Open a terminal in this folder",
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.open_terminal_here(window, cx),
                            )),
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
                )
                .child(Input::new(&self.file_search).small().cleanable(true)),
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
                    .child(crate::i18n::host(error)),
            );
        }
        // While a name is looked for, what was found stands in for the folder.
        if self.files.search.active() {
            let hits = self.files.search.hits.as_ref().map_or(0, Vec::len);
            let hint = match &self.files.search.hits {
                None => Some(tr("Searching…")),
                Some(found) if found.is_empty() => {
                    Some(tr("No file in this project has that name."))
                }
                Some(_) => None,
            };
            if let Some(hint) = hint {
                return panel
                    .child(div().p_3().text_sm().text_color(muted).child(hint))
                    .into_any_element();
            }
            return panel
                .child(
                    uniform_list(
                        "file-hits",
                        hits,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            range
                                .map(|index| this.file_hit_row(index, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .into_any_element();
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
