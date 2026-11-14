//! Project file browser beside the transcript: one directory at a time and a
//! read-only preview, both served by the host's confined project file access.
use std::ops::Range;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt, button::Button, input::Input, text::TextView,
};

use super::Workspace;
use crate::{
    backend::receive,
    files::{self, PreviewMode},
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
            (format!("{}/", entry.name), "Folder".to_string())
        } else {
            (entry.name.clone(), files::size_label(entry.size))
        };
        div()
            .id(("file-entry", index))
            .h_flex()
            .w_full()
            .h(px(28.))
            .px_3()
            .gap_3()
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
            .on_click(cx.listener(move |this, _, window, cx| {
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
        let Some(content) = &self.files.preview else {
            return match &self.files.error {
                Some(error) => div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
                    .into_any_element(),
                None => message("Loading preview…"),
            };
        };
        match files::preview_mode(content) {
            PreviewMode::Binary => message("This file is not text. Preview is not available."),
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
                            Button::new("file-preview-back")
                                .label("Back to files")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_preview(window, cx)
                                })),
                        ),
                )
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
            PanelTab::Files => {}
        }
        let location = if self.files.path.is_empty() {
            "Project root".to_string()
        } else {
            self.files.path.clone()
        };
        let parent = files::parent(&self.files.path);
        let mut panel =
            panel.child(
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
                                    .label("Up")
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
                                    .label("Reload")
                                    .small()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload_files(window, cx)
                                    })),
                            ),
                    ),
            );
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
            Some("Loading…")
        } else if self.files.loaded && self.files.entries.is_empty() {
            Some("This folder is empty.")
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
