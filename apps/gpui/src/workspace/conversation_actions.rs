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

use super::{Workspace, surfaces};
use crate::{
    backend::{Reply, receive},
    controls,
    layers::Layer,
};

impl Workspace {
    pub(super) fn current_row(&self) -> Option<&SessionSearchInfo> {
        self.known.get(self.state.session.as_ref()?)
    }

    pub(super) fn current_ids(&self) -> Option<(String, String)> {
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
        self.fill_composer(text, window, cx);
        cx.notify();
    }

    /// Asks where to save the conversation and writes it there as
    /// Markdown. The file is the user's: it is put only where they say.
    /// Saves what the conversation sends the model as a JSON file the
    /// researcher picks.
    pub(super) fn export_debug_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project, session)) = self.current_ids() else {
            return;
        };
        self.error = None;
        self.notice = None;
        let reply = self.backend.debug_request(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let prepared = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match prepared {
                    Ok((name, json)) => this.save_debug_request(name, json, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_debug_request(
        &mut self,
        name: String,
        json: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(path) = place
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                std::fs::write(&path, json).map_err(|error| {
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
    }

    pub(super) fn export_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let Some(title) = self
            .current_row()
            .map(|row| crate::present::one_line(&row.title))
        else {
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

    fn rewind(&mut self, undo_files: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        let reply = self
            .backend
            .rewind_or_undo(project, session, turn, undo_files);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // Answered either way; a refusal is not worth asking again.
                this.layers.close(Layer::ConfirmRewind);
                this.rewind_turn = None;
                match result {
                    Ok((text, files)) if this.state.current(&selection) => {
                        this.notice = files.as_ref().and_then(crate::compose::undo_note);
                        this.reload_conversation(window, cx);
                        this.state.save_draft(text.clone());
                        this.fill_composer(text, window, cx);
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
        self.load_reply_artifacts(window, cx);
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

    /// The row under the header that one of the conversation's actions
    /// opened: the title being renamed, or a question before it is deleted
    /// or cut back.
    pub(super) fn render_conversation_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.current_row()?;
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
                bar.child(div().flex_1().min_w_0().text_sm().child(
                    // Only the latest turn's files can be put back.
                    if self.rewind_turn.as_ref().is_some_and(|turn| turn.from_end == 0) {
                        tr("Go back to this message? It and everything after it leave the \
                            conversation, and its text returns to the composer to be \
                            changed. Files it wrote stay as they are unless you undo them too.")
                    } else {
                        tr("Go back to this message? It and everything after it leave the \
                            conversation, and its text returns to the composer to be \
                            changed. Files already written stay as they are.")
                    },
                ))
                .child(
                    Button::new("rewind-confirm")
                        .label(tr("Go back and edit"))
                        .small()
                        .primary()
                        .disabled(busy || running)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.rewind(false, window, cx)),
                        ),
                )
                // Only the latest turn's files can be put back.
                .children(
                    self.rewind_turn
                        .as_ref()
                        .is_some_and(|turn| turn.from_end == 0)
                        .then(|| {
                            Button::new("rewind-undo-files")
                                .label(tr("Go back and undo its files"))
                                .small()
                                .disabled(busy || running)
                                .tooltip(tr(
                                    "Also puts back the files this turn edited and moves the ones it created to the project's trash",
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.rewind(true, window, cx)
                                }))
                        }),
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
        // The actions themselves are a menu under the header's More button.
        None
    }

    /// Opens or closes the menu of the open conversation's actions.
    pub(super) fn toggle_conversation_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ConversationMenu) {
            self.focus_composer(window, cx);
        } else {
            self.layers.open(Layer::ConversationMenu);
        }
        cx.notify();
    }

    /// What can be done with the open conversation, as a menu under the
    /// More button at the right end of the window's bar: first finding
    /// one's way in it (the outline, a refresh), then its name and place,
    /// copies of it, looking back over it, and putting it away, in groups
    /// with a hairline between. Each is an icon and a word. Choosing one
    /// closes the menu first; Group and Move open beside it. An action that
    /// cannot be taken now is dimmed and inert.
    pub(super) fn conversation_menu(&self, cx: &mut Context<Self>) -> Div {
        /// One action of the menu.
        fn action(
            id: &'static str,
            icon: &'static str,
            label: &'static str,
            enabled: bool,
            run: fn(&mut Workspace, &mut Window, &mut Context<Workspace>),
            cx: &mut Context<Workspace>,
        ) -> Stateful<Div> {
            let row = Workspace::menu_item(id, icon, label, cx);
            if !enabled {
                return row.opacity(0.5).cursor_default();
            }
            row.on_click(cx.listener(move |this, _, window, cx| {
                this.layers.close(Layer::ConversationMenu);
                run(this, window, cx);
                cx.notify();
            }))
        }

        let line = cx.theme().border;
        let danger = cx.theme().danger;
        let rule = move || div().h(px(1.)).mx_2().my_1().flex_shrink_0().bg(line);
        let menu = div()
            .absolute()
            .top(px(surfaces::ICON_BUTTON + 4.))
            .right_0()
            .w(px(240.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(line)
            .bg(cx.theme().popover)
            .shadow_md()
            // Finding one's way in it.
            .child(action(
                "conversation-outline",
                "list",
                tr("Outline: what you asked, in order"),
                self.state.session.is_some(),
                |this, _, cx| {
                    this.layers.open(Layer::Outline);
                    cx.notify();
                },
                cx,
            ))
            .child(action(
                "refresh",
                "refresh",
                tr("Refresh"),
                !self.pending,
                |this, window, cx| {
                    this.error = None;
                    this.refresh_lists(window, cx);
                    this.start_poll(window, cx);
                    this.reload_files(window, cx);
                },
                cx,
            ));
        let Some(row) = self.current_row() else {
            return menu;
        };
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let busy = self.pending;
        let pinned = self.pinned.contains(&row.id);
        let archived = row.shelved;
        let title = crate::present::one_line(&row.title);
        let branch = self.open_is_branch();
        let offered = self.offered();
        menu.child(rule())
            // Its name and its place.
            .child(action(
                "conversation-rename",
                "edit",
                tr("Rename"),
                !busy,
                |this, window, cx| this.begin_rename(window, cx),
                cx,
            ))
            .child(
                Self::menu_item("conversation-copy-title", "copy", tr("Copy title"), cx).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.layers.close(Layer::ConversationMenu);
                        cx.write_to_clipboard(ClipboardItem::new_string(title.clone()));
                        this.notice = Some(tr("Title copied.").into());
                        cx.notify();
                    }),
                ),
            )
            .child(action(
                "conversation-pin",
                "pin",
                if pinned { tr("Unpin") } else { tr("Pin") },
                !(busy || archived),
                |this, window, cx| this.toggle_pin(window, cx),
                cx,
            ))
            .child(self.group_button(busy || archived, cx))
            .child(self.transfer_button(busy || running, cx))
            .child(rule())
            // Copies of it to keep or hand on.
            .child(action(
                "conversation-export",
                "archive-export",
                tr("Export"),
                !busy,
                |this, window, cx| this.export_conversation(window, cx),
                cx,
            ))
            .child(action(
                "conversation-share-page",
                "share",
                tr("Share as HTML"),
                !busy,
                |this, window, cx| this.share_conversation_page(window, cx),
                cx,
            ))
            .child(action(
                "conversation-debug-request",
                "code",
                tr("Debug request"),
                !busy,
                |this, window, cx| this.export_debug_request(window, cx),
                cx,
            ))
            .child(rule())
            // Looking back over it.
            .child(action(
                "conversation-trajectory",
                "activity-orbit",
                tr("Trajectory"),
                true,
                |this, window, cx| this.open_trajectory(window, cx),
                cx,
            ))
            // A frozen branch is not reviewed: the host refuses it.
            .child(action(
                "conversation-review",
                "review",
                tr("Review"),
                !running && offered.review,
                |this, window, cx| this.open_review(window, cx),
                cx,
            ))
            .when(branch, |menu| {
                menu.child(action(
                    "conversation-merge-branch",
                    "branch",
                    tr("Bring back"),
                    !running,
                    |this, window, cx| this.open_branch_merge(window, cx),
                    cx,
                ))
            })
            .child(action(
                "conversation-research-archive",
                "archive",
                tr("Research archive"),
                !running,
                |this, window, cx| this.open_research_archive(window, cx),
                cx,
            ))
            .child(action(
                "conversation-skill",
                "skill",
                tr("Save as skill"),
                !archived,
                |this, window, cx| this.ask_for_skill(window, cx),
                cx,
            ))
            .child(rule())
            // Putting it away. The host refuses to archive or delete a
            // running turn.
            .child(action(
                "conversation-archive",
                "inbox",
                if archived {
                    tr("Unarchive")
                } else {
                    tr("Archive")
                },
                !(busy || (running && !archived)),
                |this, window, cx| this.toggle_archive(window, cx),
                cx,
            ))
            .child(
                action(
                    "conversation-delete",
                    "trash",
                    tr("Delete"),
                    !(busy || running),
                    |this, _, cx| this.confirm_delete(cx),
                    cx,
                )
                .text_color(danger),
            )
    }
}
