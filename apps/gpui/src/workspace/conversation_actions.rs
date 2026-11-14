//! Rename, pin, archive and delete for the open conversation. Rename and the
//! delete confirmation are layers in the window's Escape stack.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::SessionSearchInfo;

use super::Workspace;
use crate::{
    backend::{Reply, receive},
    layers::Layer,
};

impl Workspace {
    pub(super) fn current_row(&self) -> Option<&SessionSearchInfo> {
        self.known.get(self.state.session.as_ref()?)
    }

    fn current_ids(&self) -> Option<(String, String)> {
        Some((self.state.project.clone()?, self.state.session.clone()?))
    }

    /// Runs one management call, then refreshes every list it can affect.
    /// `then` runs only if the same conversation is still open.
    fn manage(
        &mut self,
        reply: Reply<()>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = true;
        self.error = None;
        let selection = self.state.selection();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        if this.state.current(&selection) {
                            then(this, window, cx);
                        }
                        this.refresh_lists(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.current_row().map(|row| row.title.clone()) else {
            return;
        };
        self.layers.close(Layer::ConfirmDelete);
        self.layers.open(Layer::Rename);
        self.rename.update(cx, |input, cx| {
            input.set_value(title, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::Rename) {
            return;
        }
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        let title = self.rename.read(cx).value().trim().to_string();
        let reply = self
            .backend
            .rename_conversation(project, session.clone(), title.clone());
        self.manage(
            reply,
            move |this, window, cx| {
                if let Some(row) = this.known.get_mut(&session) {
                    row.title = title;
                }
                // Only on success: a rejected title keeps the editor open.
                this.layers.close(Layer::Rename);
                this.focus_composer(window, cx);
            },
            window,
            cx,
        );
    }

    fn toggle_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        let pinned = !self.pinned.contains(&session);
        let reply = self.backend.pin_conversation(project, session, pinned);
        self.manage(reply, |_, _, _| {}, window, cx);
    }

    fn toggle_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        let archive = !self.current_row().is_some_and(|row| row.shelved);
        let reply = self
            .backend
            .archive_conversation(project, session.clone(), archive);
        self.manage(
            reply,
            move |this, window, cx| {
                if let Some(row) = this.known.get_mut(&session) {
                    row.shelved = archive;
                }
                if archive {
                    // It has left everyday navigation; move on to what remains.
                    this.leave_conversation(window, cx);
                }
            },
            window,
            cx,
        );
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::Rename);
        self.layers.open(Layer::ConfirmDelete);
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmDelete) {
            return;
        }
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        let reply = self.backend.delete_conversation(project, session.clone());
        self.manage(
            reply,
            move |this, window, cx| {
                this.known.remove(&session);
                this.state.forget_draft(&session);
                this.chips.remove(&session);
                this.leave_conversation(window, cx);
            },
            window,
            cx,
        );
    }

    /// Closes the open conversation; the next list load selects another.
    fn leave_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose(self.state.project.clone(), None);
        self.clear_transcript();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn cancel_layer(&mut self, layer: Layer, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(layer) {
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// The row under the header: the actions, or the layer one of them opened.
    pub(super) fn render_conversation_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let row = self.current_row()?;
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let busy = self.pending;
        let bar = div()
            .h_flex()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border);
        if self.layers.contains(Layer::Rename) {
            return Some(
                bar.child(div().flex_1().min_w_0().child(Input::new(&self.rename)))
                    .child(
                        Button::new("rename-save")
                            .label("Save title")
                            .small()
                            .primary()
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_rename(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("rename-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_layer(Layer::Rename, window, cx)
                            })),
                    ),
            );
        }
        if self.layers.contains(Layer::ConfirmDelete) {
            return Some(
                bar.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(
                            "Delete this conversation and its messages permanently? \
                             Project files it wrote are kept.",
                        ),
                )
                .child(
                    Button::new("delete-confirm")
                        .label("Delete permanently")
                        .small()
                        .danger()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                )
                .child(
                    Button::new("delete-cancel")
                        .label("Cancel")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_layer(Layer::ConfirmDelete, window, cx)
                        })),
                ),
            );
        }
        let pinned = self.pinned.contains(&row.id);
        let archived = row.shelved;
        Some(
            bar.child(
                Button::new("conversation-rename")
                    .label("Rename")
                    .small()
                    .ghost()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.begin_rename(window, cx))),
            )
            .child(
                Button::new("conversation-pin")
                    .label(if pinned { "Unpin" } else { "Pin" })
                    .small()
                    .ghost()
                    .disabled(busy || archived)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_pin(window, cx))),
            )
            .child(
                Button::new("conversation-archive")
                    .label(if archived { "Unarchive" } else { "Archive" })
                    .small()
                    .ghost()
                    // The host refuses to archive or delete a running turn.
                    .disabled(busy || (running && !archived))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_archive(window, cx))),
            )
            .child(
                Button::new("conversation-delete")
                    .label("Delete")
                    .small()
                    .ghost()
                    .disabled(busy || running)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
            ),
        )
    }
}
