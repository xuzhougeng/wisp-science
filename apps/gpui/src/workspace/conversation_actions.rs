//! Rename, pin, group, move, archive and delete for the open conversation.
//! What the bar turns into or opens is a layer in the window's Escape stack.
use crate::i18n::tr;
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
    controls,
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
        self.notice = None;
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

    /// Puts the request to turn this conversation into a skill in the
    /// composer. It is sent like any message, so it can be changed first.
    pub(super) fn ask_for_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.session.is_none() {
            return;
        }
        let text = crate::compose::with_skill_prompt(&self.composer.read(cx).value());
        self.state.save_draft(text.clone());
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Asks where to save the conversation and writes it there as
    /// Markdown. The file is the user's: it is put only where they say.
    fn export_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((project, session)), Some(title)) = (
            self.current_ids(),
            self.current_row().map(|row| row.title.clone()),
        ) else {
            return;
        };
        self.error = None;
        self.notice = None;
        let folder = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&crate::organise::export_name(&title)));
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(path) = place
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                else {
                    // The dialog was dismissed: nothing is written.
                    return Ok(None);
                };
                let text = receive(backend.export_conversation(project, session)).await?;
                std::fs::write(&path, text).map_err(|error| {
                    crate::i18n::fill(tr("Could not save {}: {}"), &[&path.display(), &error])
                })?;
                Ok::<_, String>(Some(path))
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(path)) => {
                        this.notice =
                            Some(crate::i18n::fill(tr("Saved to {}"), &[&path.display()]));
                    }
                    Ok(None) => {}
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
        self.layers.switch(Layer::Rename, &Layer::CONVERSATION_BAR);
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
        self.layers
            .switch(Layer::ConfirmDelete, &Layer::CONVERSATION_BAR);
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
                this.tabs.close(&session, None);
                this.state.forget_draft(&session);
                this.chips.remove(&session);
                this.queue.forget(&session);
                this.leave_conversation(window, cx);
            },
            window,
            cx,
        );
    }

    /// Asks before cutting the conversation back to the user message the
    /// row at `index` belongs to.
    pub(super) fn ask_rewind(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(turn) = controls::turn_of(&self.rows, index) else {
            return;
        };
        self.rewind_turn = Some(turn);
        self.layers
            .switch(Layer::ConfirmRewind, &Layer::CONVERSATION_BAR);
        cx.notify();
    }

    fn rewind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmRewind) {
            return;
        }
        let (Some((project, session)), Some(turn)) = (self.current_ids(), self.rewind_turn.clone())
        else {
            return;
        };
        self.pending = true;
        self.error = None;
        self.notice = None;
        let selection = self.state.selection();
        let reply = self.backend.rewind_conversation(project, session, turn);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // Answered either way; a refusal is not worth asking again.
                this.layers.close(Layer::ConfirmRewind);
                this.rewind_turn = None;
                match result {
                    Ok(text) if this.state.current(&selection) => {
                        this.reload_conversation(window, cx);
                        this.state.save_draft(text.clone());
                        this.composer.update(cx, |input, cx| {
                            input.set_value(text, window, cx);
                            input.focus(window, cx);
                        });
                        this.refresh_lists(window, cx);
                    }
                    Ok(_) => this.refresh_lists(window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Reads the open conversation afresh after its saved rows changed.
    fn reload_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (project, session) = (self.state.project.clone(), self.state.session.clone());
        // A new selection generation: answers to earlier requests are stale.
        self.state.select(project, session);
        self.clear_transcript();
        self.start_poll(window, cx);
        self.load_status(window, cx);
    }

    /// Starts a branch that holds the conversation through the turn the row
    /// at `index` belongs to, and opens it.
    pub(super) fn branch_from(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        let Some(turn) = controls::turn_of(&self.rows, index) else {
            return;
        };
        self.pending = true;
        self.error = None;
        self.notice = None;
        let reply = self
            .backend
            .branch_conversation(project.clone(), session, Some(turn));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(branch) => {
                        if this.state.project.as_ref() == Some(&project) {
                            this.open_session(project, branch, window, cx);
                            this.notice = Some(
                                tr("This is a branch: a copy to take in another direction. \
                                 The conversation it came from is unchanged.")
                                .into(),
                            );
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

    /// Closes the open conversation; the next list load selects another.
    pub(super) fn leave_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            // With the side panel open the actions take a second line
            // rather than run off the edge.
            .flex_wrap()
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
                            .label(tr("Save title"))
                            .small()
                            .primary()
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_rename(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("rename-cancel")
                            .label(tr("Cancel"))
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
                        .child(tr(
                            "Delete this conversation and its messages permanently? \
                             Project files it wrote are kept.",
                        )),
                )
                .child(
                    Button::new("delete-confirm")
                        .label(tr("Delete permanently"))
                        .small()
                        .danger()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                )
                .child(
                    Button::new("delete-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_layer(Layer::ConfirmDelete, window, cx)
                        })),
                ),
            );
        }
        if self.layers.contains(Layer::ConfirmRewind) {
            return Some(
                bar.child(div().flex_1().min_w_0().text_sm().child(tr(
                    "Go back to this message? It and everything after it leave the \
                     conversation, and its text returns to the composer to be \
                     changed. Files already written stay as they are.",
                )))
                .child(
                    Button::new("rewind-confirm")
                        .label(tr("Go back and edit"))
                        .small()
                        .primary()
                        .disabled(busy || running)
                        .on_click(cx.listener(|this, _, window, cx| this.rewind(window, cx))),
                )
                .child(
                    Button::new("rewind-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.rewind_turn = None;
                            this.cancel_layer(Layer::ConfirmRewind, window, cx)
                        })),
                ),
            );
        }
        let pinned = self.pinned.contains(&row.id);
        let archived = row.shelved;
        Some(
            bar.child(
                Button::new("conversation-rename")
                    .label(tr("Rename"))
                    .small()
                    .ghost()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.begin_rename(window, cx))),
            )
            .child(
                Button::new("conversation-pin")
                    .label(if pinned { tr("Unpin") } else { tr("Pin") })
                    .small()
                    .ghost()
                    .disabled(busy || archived)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_pin(window, cx))),
            )
            .child(self.group_button(busy || archived, cx))
            .child(self.transfer_button(busy || running, cx))
            .child(
                Button::new("conversation-export")
                    .label(tr("Export"))
                    .small()
                    .ghost()
                    .tooltip(tr("Save the whole conversation as a Markdown file"))
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.export_conversation(window, cx)),
                    ),
            )
            .child(
                Button::new("conversation-trajectory")
                    .label(tr("Trajectory"))
                    .small()
                    .ghost()
                    .tooltip(tr("Every step of this conversation, with what it took"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_trajectory(window, cx))),
            )
            .child(
                Button::new("conversation-review")
                    .label(tr("Review"))
                    .small()
                    .ghost()
                    .tooltip(tr(
                        "Have the model check this conversation's claims against its tool output",
                    ))
                    .disabled(running)
                    .on_click(cx.listener(|this, _, window, cx| this.open_review(window, cx))),
            )
            .child(
                Button::new("conversation-research-archive")
                    .label(tr("Research archive"))
                    .small()
                    .ghost()
                    .tooltip(tr(
                        "Write this conversation's findings up as a report and freeze it",
                    ))
                    .disabled(running)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_research_archive(window, cx)),
                    ),
            )
            .child(
                Button::new("conversation-skill")
                    .label(tr("Save as skill"))
                    .small()
                    .ghost()
                    .tooltip(tr(
                        "Ask Wisp to distill this conversation into a reusable skill",
                    ))
                    .disabled(archived)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_for_skill(window, cx))),
            )
            .child(
                Button::new("conversation-archive")
                    .label(if archived {
                        tr("Unarchive")
                    } else {
                        tr("Archive")
                    })
                    .small()
                    .ghost()
                    // The host refuses to archive or delete a running turn.
                    .disabled(busy || (running && !archived))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_archive(window, cx))),
            )
            .child(
                Button::new("conversation-delete")
                    .label(tr("Delete"))
                    .small()
                    .ghost()
                    .disabled(busy || running)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
            ),
        )
    }
}
