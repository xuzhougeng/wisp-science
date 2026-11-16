//! The side panel's Memory tab: one switch, the habits the model is shown in
//! every project, and this project's notes, which it can search. The editor
//! and the delete question are layers inside the panel; the question before
//! Clear all is a dialog over the window.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
    surfaces::{self, DialogWidth, switch},
};
use crate::{
    assets::icon,
    backend::{Reply, receive},
    files::size_label,
    layers::Layer,
    memory::{self, Target},
    panel::PanelTab,
};

impl Workspace {
    /// Reads what is remembered. Only the answer for the project that is
    /// still open, to the latest request, is shown.
    pub(super) fn load_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.memory_request += 1;
        let request = self.memory_request;
        let reply = self.backend.memory(project.clone());
        let tidy = self.backend.memory_tidy();
        self.show_memory_tidy(tidy, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.project.as_ref() != Some(&project) || this.memory_request != request {
                    return;
                }
                match result {
                    Ok(view) => this.memory = Some(view),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows the weekly tidy once `reply` comes; while it runs, looks again
    /// shortly, and reads the habits again once it has applied something.
    fn show_memory_tidy(
        &mut self,
        reply: Reply<wisp_dto::MemoryTidyAutomation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(tidy) => {
                        if tidy.running && this.panel_tab == PanelTab::Memory {
                            cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_secs(3))
                                    .await;
                                let _ = this.update_in(cx, |this, window, cx| {
                                    let reply = this.backend.memory_tidy();
                                    this.show_memory_tidy(reply, window, cx);
                                });
                            })
                            .detach();
                        }
                        this.memory_tidy = Some(tidy);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Applies or dismisses one proposal, then reads the habits again.
    fn settle_tidy(
        &mut self,
        id: String,
        apply: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        let reply = self.backend.settle_memory_tidy(id, apply);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(tidy) => this.memory_tidy = Some(tidy),
                    Err(error) => this.error = Some(error),
                }
                this.load_memory(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The weekly tidy: its switch, how it last went, and what it proposes.
    fn tidy_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut card = div()
            .v_flex()
            .gap_2()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_semibold().child(tr("Tidy habits")))
            .child(div().text_xs().text_color(muted).child(tr(
                "Once a week the model looks over the habits and proposes merging ones that say \
                 the same thing or retiring one a newer one replaces. Nothing changes until you \
                 apply a proposal.",
            )));
        let Some(tidy) = &self.memory_tidy else {
            return card;
        };
        let enabled = tidy.enabled;
        card = card
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        switch("tidy-enabled", enabled, tr("Tidy habits")).on_change(cx.listener(
                            move |this, _, window, cx| {
                                let reply = this.backend.set_memory_tidy(!enabled);
                                this.show_memory_tidy(reply, window, cx);
                            },
                        )),
                    )
                    .child(
                        Button::new("tidy-now")
                            .label(tr("Run now"))
                            .small()
                            .ghost()
                            .disabled(tidy.running)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let reply = this.backend.run_memory_tidy_now();
                                this.show_memory_tidy(reply, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if tidy.error.is_some() && !tidy.running {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .child(memory::tidy_line(tidy, &chrono::Local)),
            );
        for (index, proposal) in tidy.proposals.iter().enumerate() {
            let mut entry = div()
                .v_flex()
                .gap_1()
                .pl_2()
                .border_l_2()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .child(memory::tidy_action(&proposal.action)),
                );
            for habit in &proposal.memories {
                entry = entry.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("• {}", memory::preview(&habit.content, 160))),
                );
            }
            if !proposal.text.is_empty() {
                entry = entry.child(div().text_sm().child(crate::i18n::fill(
                    tr("Becomes: {}"),
                    &[&memory::preview(&proposal.text, 260)],
                )));
            }
            if !proposal.reason.is_empty() {
                entry = entry.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(proposal.reason.clone()),
                );
            }
            let (apply, dismiss) = (proposal.id.clone(), proposal.id.clone());
            entry = entry.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new(("tidy-apply", index))
                            .label(tr("Apply"))
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settle_tidy(apply.clone(), true, window, cx)
                            })),
                    )
                    .child(
                        Button::new(("tidy-dismiss", index))
                            .label(tr("Dismiss"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settle_tidy(dismiss.clone(), false, window, cx)
                            })),
                    ),
            );
            card = card.child(entry);
        }
        card
    }

    /// Runs one change to memory and reads it again. `then` runs only if the
    /// same project is still open.
    fn change_memory<T: 'static>(
        &mut self,
        reply: Reply<T>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = true;
        self.error = None;
        let project = self.state.project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.state.project != project {
                    return;
                }
                match result {
                    Ok(_) => {
                        then(this, window, cx);
                        this.load_memory(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(enabled) = self.memory.as_ref().map(|view| view.enabled) else {
            return;
        };
        let reply = self.backend.set_memory_enabled(!enabled);
        self.change_memory(reply, |_, _, _| {}, window, cx);
    }

    /// Has the model draft what is worth keeping from the turn being
    /// remembered, in place of the text in the editor.
    fn suggest_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (
            Some(project),
            Some(Target::Remember {
                session: Some(session),
                turn,
            }),
        ) = (self.state.project.clone(), self.memory_target.clone())
        else {
            return;
        };
        if self.memory_suggesting {
            return;
        }
        self.memory_suggesting = true;
        self.error = None;
        let reply = self.backend.suggest_memory(project, session.clone(), turn);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.memory_suggesting = false;
                // Only into the editor it was asked from.
                let still = matches!(
                    &this.memory_target,
                    Some(Target::Remember { session: Some(open), .. }) if *open == session
                ) && this.layers.contains(Layer::MemoryEditor);
                if !still {
                    return;
                }
                match result {
                    Ok(draft) => {
                        this.memory_text
                            .update(cx, |input, cx| input.set_value(draft.content, window, cx));
                        this.memory_suggested = Some(draft.scope);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the editor on the memory the model drafted after the latest
    /// turn, with the scope it suggested. Nothing is kept until it is saved.
    pub(super) fn review_memory_proposal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(proposal) = self
            .status
            .as_ref()
            .and_then(|status| status.memory_proposal.clone())
        else {
            return;
        };
        let session = self.state.session.clone();
        self.show_panel_tab(PanelTab::Memory, window, cx);
        self.open_memory_editor(
            Target::Remember {
                session,
                turn: None,
            },
            proposal.content,
            window,
            cx,
        );
        self.memory_suggested = Some(proposal.scope);
        // It is in the editor now; the line above the composer goes.
        self.dismiss_memory_proposal(window, cx);
    }

    /// Puts away the memory drafted after the latest turn.
    pub(super) fn dismiss_memory_proposal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if let Some(status) = &mut self.status {
            status.memory_proposal = None;
        }
        let reply = self.backend.dismiss_memory_proposal(project, session);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    if this.state.current(&selection) {
                        this.error = Some(error);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// The line above the composer about a memory drafted after the latest
    /// turn.
    pub(super) fn render_memory_proposal(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let proposal = self.status.as_ref()?.memory_proposal.as_ref()?;
        Some(
            surfaces::bar("memory-proposal", cx)
                .h_flex()
                .gap_2()
                .child(icon("memory").text_color(cx.theme().muted_foreground))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(memory::proposal_line(proposal)),
                )
                .child(
                    Button::new("memory-proposal-review")
                        .label(tr("Review"))
                        .small()
                        .tooltip(tr("Read and change it before it is kept"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.review_memory_proposal(window, cx)
                        })),
                )
                .child(
                    super::shell::icon_button(
                        "memory-proposal-dismiss",
                        "close",
                        tr("Put this away"),
                    )
                    .xsmall()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.dismiss_memory_proposal(window, cx)),
                    ),
                ),
        )
    }

    /// Opens the editor on `target` with `text` in it.
    fn open_memory_editor(
        &mut self,
        target: Target,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.memory_target = Some(target);
        self.memory_suggested = None;
        self.layers.close(Layer::ConfirmDeleteMemory);
        self.layers.open(Layer::MemoryEditor);
        self.memory_text.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn edit_habit(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let text = id
            .as_deref()
            .and_then(|id| {
                let habits = &self.memory.as_ref()?.habits;
                habits.iter().find(|habit| habit.id == id)
            })
            .map(|habit| habit.content.clone())
            .unwrap_or_default();
        self.open_memory_editor(Target::Habit(id), text, window, cx);
    }

    /// Reads a note in full, then opens the editor on it.
    fn edit_note(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.error = None;
        let reply = self.backend.memory_note(project.clone(), name.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.state.project.as_ref() != Some(&project) {
                    return;
                }
                match result {
                    Ok(text) => this.open_memory_editor(Target::Note(name), text, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Offers the transcript row at `index` for remembering.
    pub(super) fn remember_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.rows.get(index).map(|row| memory::offered(&row.text)) else {
            return;
        };
        let session = self.state.session.clone();
        let turn = crate::controls::turn_of(&self.rows, index);
        self.show_panel_tab(PanelTab::Memory, window, cx);
        self.open_memory_editor(Target::Remember { session, turn }, text, window, cx);
    }

    /// Saves the editor. `everywhere` matters only for remembered text: it
    /// becomes a habit instead of joining the project's note.
    fn save_memory(&mut self, everywhere: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::MemoryEditor) {
            return;
        }
        let (Some(project), Some(target)) =
            (self.state.project.clone(), self.memory_target.clone())
        else {
            return;
        };
        let text = self.memory_text.read(cx).value().to_string();
        // Only on success: a refused entry keeps the editor and its text.
        let close = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.layers.close(Layer::MemoryEditor);
            this.memory_target = None;
            this.focus_composer(window, cx);
        };
        match target {
            Target::Remember { session, .. } => {
                let reply = self.backend.remember(project, session, text, everywhere);
                self.change_memory(reply, close, window, cx);
            }
            Target::Habit(None) => {
                let reply = self.backend.add_habit(text);
                self.change_memory(reply, close, window, cx);
            }
            Target::Habit(Some(id)) => {
                let reply = self.backend.update_habit(id, text);
                self.change_memory(reply, close, window, cx);
            }
            Target::Note(name) => {
                let reply = self.backend.save_memory_note(project, name, text);
                self.change_memory(reply, close, window, cx);
            }
        }
    }

    fn close_memory_layer(&mut self, layer: Layer, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(layer) {
            self.memory_target = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn ask_delete_memory(&mut self, target: Target, cx: &mut Context<Self>) {
        self.memory_target = Some(target);
        self.layers.open(Layer::ConfirmDeleteMemory);
        cx.notify();
    }

    fn delete_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmDeleteMemory) {
            return;
        }
        let (Some(project), Some(target)) =
            (self.state.project.clone(), self.memory_target.clone())
        else {
            return;
        };
        let done = |this: &mut Self, _: &mut Window, _: &mut Context<Self>| {
            this.layers.close(Layer::ConfirmDeleteMemory);
            this.memory_target = None;
        };
        match target {
            Target::Habit(Some(id)) => {
                let reply = self.backend.delete_habit(id);
                self.change_memory(reply, done, window, cx);
            }
            Target::Note(name) => {
                let reply = self.backend.delete_memory_note(project, name);
                self.change_memory(reply, done, window, cx);
            }
            Target::Habit(None) | Target::Remember { .. } => {}
        }
    }

    /// Asks before every note of the project's memory is deleted.
    fn ask_clear_memory(&mut self, cx: &mut Context<Self>) {
        if self
            .memory
            .as_ref()
            .is_some_and(|view| !view.notes.is_empty())
        {
            self.layers.open(Layer::ConfirmClearMemory);
            cx.notify();
        }
    }

    /// Deletes every note of the project's memory, as the WebView's Clear
    /// all does. Habits stay.
    fn clear_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::ConfirmClearMemory) {
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let reply = self.backend.clear_memory_notes(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.state.project.as_ref() != Some(&project) {
                    return;
                }
                this.layers.close(Layer::ConfirmClearMemory);
                match result {
                    Ok(removed) => this.notice = Some(memory::cleared(removed)),
                    Err(error) => this.error = Some(error),
                }
                // Some may be gone even when one could not be deleted.
                this.load_memory(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The question before Clear all: what is deleted, by name, that it
    /// cannot be undone, and that habits stay.
    pub(super) fn render_clear_memory(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let notes = self
            .memory
            .as_ref()
            .map_or(&[][..], |view| view.notes.as_slice());
        let mut names = div().v_flex().gap_0p5();
        for note in notes {
            names = names.child(div().text_xs().text_color(muted).child(format!(
                "{} · {}",
                note.name,
                size_label(note.bytes)
            )));
        }
        surfaces::dialog(
            "clear-memory",
            DialogWidth::Narrow,
            tr("Clear this project's notes?"),
        )
        .child(div().text_sm().child(memory::clear_question(notes.len())))
        .child(names)
        .action(
            Button::new("clear-memory-cancel")
                .label(tr("Cancel"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::ConfirmClearMemory);
                    cx.notify();
                })),
        )
        .action(
            Button::new("clear-memory-confirm")
                .label(tr("Delete all notes"))
                .small()
                .danger()
                .disabled(self.pending || notes.is_empty())
                .on_click(cx.listener(|this, _, window, cx| this.clear_memory(window, cx))),
        )
    }

    fn memory_editor(&self, target: &Target, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let problem = memory::problem(target, &self.memory_text.read(cx).value());
        let ready = problem.is_none() && !self.pending;
        let save =
            |id: &'static str, label: &'static str, everywhere: bool| {
                Button::new(id)
                    .label(label)
                    .small()
                    .disabled(!ready)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_memory(everywhere, window, cx)
                    }))
            };
        let mut buttons = div().h_flex().flex_wrap().gap_2();
        buttons = match target {
            Target::Remember { session, .. } => buttons
                .child(save("memory-save-project", tr("Save for this project"), false).primary())
                .child(save("memory-save-habit", tr("Save as a habit"), true))
                .when(session.is_some(), |buttons| {
                    buttons.child(
                        Button::new("memory-suggest")
                            .label(if self.memory_suggesting {
                                tr("Drafting…")
                            } else {
                                tr("Suggest")
                            })
                            .small()
                            .ghost()
                            .disabled(self.memory_suggesting)
                            .tooltip(tr(
                                "Have the model draft what is worth keeping from this turn",
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.suggest_memory(window, cx)),
                            ),
                    )
                }),
            _ => buttons.child(save("memory-save", tr("Save"), false).primary()),
        };
        let mut editor = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(target.title()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(160.))
                    .child(Input::new(&self.memory_text).h_full()),
            )
            .child(div().text_xs().text_color(muted).child(target.hint()))
            .children(self.memory_suggested.as_deref().map(|scope| {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(memory::suggested_scope(scope))
            }));
        // An empty editor needs no telling; an over-long entry does.
        if let Some(problem) =
            problem.filter(|_| !self.memory_text.read(cx).value().trim().is_empty())
        {
            editor = editor.child(div().text_xs().text_color(cx.theme().danger).child(problem));
        }
        editor.child(
            buttons.child(
                Button::new("memory-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_memory_layer(Layer::MemoryEditor, window, cx)
                    })),
            ),
        )
    }

    /// One habit or note in the list, with its actions or its delete question.
    fn memory_card(
        &self,
        id: SharedString,
        heading: Option<String>,
        detail: String,
        target: Target,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let asking = self.layers.contains(Layer::ConfirmDeleteMemory)
            && self.memory_target.as_ref() == Some(&target);
        let mut card = div()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border);
        if let Some(heading) = heading {
            card = card.child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(heading),
            );
        }
        if !detail.is_empty() {
            card = card.child(div().text_xs().text_color(muted).child(detail));
        }
        if asking {
            let what = match target {
                Target::Note(_) => {
                    tr("Delete this day's note? The file is removed from the project.")
                }
                _ => tr("Delete this habit? It stops applying in every project."),
            };
            return card
                .child(div().text_xs().text_color(cx.theme().danger).child(what))
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(
                            Button::new("memory-delete-confirm")
                                .label(tr("Delete"))
                                .xsmall()
                                .danger()
                                .disabled(self.pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.delete_memory(window, cx)
                                })),
                        )
                        .child(
                            Button::new("memory-delete-cancel")
                                .label(tr("Cancel"))
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_memory_layer(Layer::ConfirmDeleteMemory, window, cx)
                                })),
                        ),
                );
        }
        let (editing, deleting) = (target.clone(), target);
        card.child(
            div()
                .h_flex()
                .justify_end()
                .gap_0p5()
                .child(
                    icon_button(SharedString::from(format!("{id}-edit")), "edit", tr("Edit"))
                        .xsmall()
                        .on_click(
                            cx.listener(move |this, _, window, cx| match editing.clone() {
                                Target::Note(name) => this.edit_note(name, window, cx),
                                Target::Habit(id) => this.edit_habit(id, window, cx),
                                Target::Remember { .. } => {}
                            }),
                        ),
                )
                .child(
                    icon_button(
                        SharedString::from(format!("{id}-delete")),
                        "trash",
                        tr("Delete"),
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_delete_memory(deleting.clone(), cx)
                    })),
                ),
        )
    }

    fn memory_heading(title: &'static str, note: &'static str, button: Button, cx: &App) -> Div {
        div()
            .h_flex()
            .gap_2()
            .pt_2()
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(note),
                    ),
            )
            .child(button.small().flex_shrink_0())
    }

    pub(super) fn render_memory_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(target) = self
            .memory_target
            .as_ref()
            .filter(|_| self.layers.contains(Layer::MemoryEditor))
        {
            let mut editor = div().v_flex().flex_1().min_h_0();
            if let Some(error) = &self.error {
                editor = editor.child(div().p_3().child(error_banner(error, cx)));
            }
            return editor
                .child(self.memory_editor(target, cx))
                .into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("memory")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            list = list.child(error_banner(error, cx));
        }
        let Some(view) = &self.memory else {
            return list
                .child(div().text_sm().text_color(muted).child(tr("Loading…")))
                .into_any_element();
        };
        let enabled = view.enabled;
        list = list.child(
            div()
                .h_flex()
                .gap_2()
                .p_2()
                .rounded(cx.theme().radius)
                .bg(cx.theme().muted)
                .child(icon("memory").text_color(if enabled { cx.theme().primary } else { muted }))
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(if enabled {
                            tr("Memory is on")
                        } else {
                            tr("Memory is off")
                        }))
                        .child(div().text_xs().text_color(muted).child(if enabled {
                            memory::summary(view)
                        } else {
                            tr("The model is not shown habits and cannot search notes. \
                             Nothing is deleted.")
                            .to_owned()
                        })),
                )
                .child(
                    switch("memory-toggle", enabled, tr("Memory"))
                        .disabled(self.pending)
                        .on_change(
                            cx.listener(|this, _, window, cx| this.toggle_memory(window, cx)),
                        ),
                ),
        );

        list = list.child(Self::memory_heading(
            tr("Habits"),
            tr("Shown to the model in every project."),
            Button::new("habit-new")
                .icon(glyph("plus"))
                .label(tr("Add"))
                .on_click(cx.listener(|this, _, window, cx| this.edit_habit(None, window, cx))),
            cx,
        ));
        if view.habits.is_empty() {
            list = list.child(div().text_xs().text_color(muted).child(tr(
                "None yet. For example: \"Report medians with the interquartile range.\"",
            )));
        }
        for habit in &view.habits {
            list = list.child(self.memory_card(
                SharedString::from(format!("habit-{}", habit.id)),
                None,
                memory::preview(&habit.content, 260),
                Target::Habit(Some(habit.id.clone())),
                cx,
            ));
        }

        list = list.child(self.tidy_card(cx));
        let today = view.today.clone();
        list = list.child(Self::memory_heading(
            tr("This project's notes"),
            tr("One per day, in .wisp/memory. The model searches them when it needs to."),
            Button::new("note-new")
                .icon(glyph("note-plus"))
                .label(tr("Today"))
                .tooltip(tr("Open today's note"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.edit_note(today.clone(), window, cx)
                })),
            cx,
        ));
        if view.notes.is_empty() {
            list = list.child(div().text_xs().text_color(muted).child(tr(
                "None yet. Save a finding from a message with the memory button under it.",
            )));
        }
        for note in &view.notes {
            list = list.child(self.memory_card(
                SharedString::from(format!("note-{}", note.name)),
                Some(format!(
                    "{} · {}",
                    memory::note_label(&note.name, &view.today),
                    size_label(note.bytes)
                )),
                memory::preview(&note.preview, 200),
                Target::Note(note.name.clone()),
                cx,
            ));
        }
        // Under the notes, as the WebView has it under its list of them.
        if !view.notes.is_empty() {
            list = list.child(
                div().h_flex().justify_end().child(
                    Button::new("memory-clear-all")
                        .icon(glyph("trash"))
                        .label(tr("Clear all"))
                        .xsmall()
                        .ghost()
                        .disabled(self.pending)
                        .tooltip(tr("Delete all of this project's notes; habits stay"))
                        .on_click(cx.listener(|this, _, _, cx| this.ask_clear_memory(cx))),
                ),
            );
        }
        list.into_any_element()
    }
}
