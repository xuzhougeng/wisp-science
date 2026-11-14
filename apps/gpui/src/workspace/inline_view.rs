//! Pictures under the answers that point at them: reading them from the
//! project and drawing them.
use std::sync::Arc;

use gpui::{prelude::*, *};

use super::{Workspace, favorites_view::image_format};
use crate::{backend::receive, inline};

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
                        if item.role == "assistant" && inline::pictures(&item.text).contains(&path)
                        {
                            this.list.splice(index..index + 1, 1);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The pictures an answer points at that have been read, in order.
    pub(super) fn inline_pictures(&self, text: &str) -> Vec<Div> {
        inline::pictures(text)
            .into_iter()
            .filter_map(|path| {
                let image = self.inline_images.get(&path)?.clone()?;
                Some(
                    div().py_1().child(
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
