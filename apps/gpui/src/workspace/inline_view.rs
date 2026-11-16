//! Pictures under the answers that point at them: reading them from the
//! project and drawing them.
use std::sync::Arc;

use gpui::{prelude::*, *};

use super::{Workspace, favorites_view::image_format};
use crate::{backend::receive, i18n::tr, inline};

impl Workspace {
    /// Reads the pictures the loaded answers point at and that are not
    /// known yet. Each is asked for once; one that cannot be read is
    /// simply not shown.
    pub(super) fn load_inline_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let wanted: Vec<(usize, String)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, item)| item.role == "assistant")
            .flat_map(|(index, item)| {
                inline::pictures(&item.text)
                    .into_iter()
                    .chain(inline::written(
                        &self.rows,
                        index,
                        &self.recorded_pictures(index),
                    ))
                    .map(move |path| (index, path))
            })
            .filter(|(_, path)| !self.inline_images.contains_key(path))
            .collect();
        for (_, path) in wanted {
            if self.inline_images.len() >= inline::KEPT || self.inline_images.contains_key(&path) {
                continue;
            }
            self.inline_images.insert(path.clone(), None);
            let reply = self.backend.project_image(project.clone(), path.clone());
            let project = project.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    if this.state.project.as_deref() != Some(project.as_str()) {
                        return;
                    }
                    let Ok((kind, bytes)) = result else {
                        return;
                    };
                    let Some(format) = image_format(Some(&kind)) else {
                        return;
                    };
                    this.inline_images.insert(
                        path.clone(),
                        Some(Arc::new(Image::from_bytes(format, bytes))),
                    );
                    // The rows that show it are taller now.
                    for (index, item) in this.rows.iter().enumerate() {
                        let shows = item.role == "assistant"
                            && (inline::pictures(&item.text).contains(&path)
                                || inline::written(
                                    &this.rows,
                                    index,
                                    &this.recorded_pictures(index),
                                )
                                .contains(&path));
                        if shows {
                            this.list.splice(index..index + 1, 1);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The pictures the turn an answer closes wrote, or recorded as its
    /// artifacts, and the answer does not show, under it, as the WebView
    /// shows a turn's files.
    pub(super) fn written_pictures(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        self.pictures(
            inline::written(&self.rows, index, &self.recorded_pictures(index)),
            cx,
        )
    }

    /// The pictures an answer points at that have been read, in order.
    /// Clicking one opens it in the Files tab, larger.
    pub(super) fn inline_pictures(&self, text: &str, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        self.pictures(inline::pictures(text), cx)
    }

    fn pictures(&self, paths: Vec<String>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        paths
            .into_iter()
            .filter_map(|path| {
                let image = self.inline_images.get(&path)?.clone()?;
                let opened = path.clone();
                Some(
                    div()
                        .id(SharedString::from(format!("inline-picture-{path}")))
                        .py_1()
                        .cursor_pointer()
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new(tr("Open it in Files"))
                                .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reveal_file(opened.clone(), window, cx)
                        }))
                        .child(
                            img(image)
                                .max_w(px(640.))
                                .max_h(px(420.))
                                .object_fit(ObjectFit::Contain),
                        ),
                )
            })
            .collect()
    }
}
