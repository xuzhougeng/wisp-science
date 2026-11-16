//! Quote / reuse a range of the open conversation's turns, as the WebView
//! has it: the first click on a user message starts the range and a bar
//! above the composer says so, the second click (End quote here) reads the
//! turns and opens a dialog. There they are quoted as they are, or
//! distilled into an experience draft to edit and maybe keep as a memory,
//! and the quote goes in front of the draft of a new conversation or
//! another one in the project, which is then opened to be sent from.
use crate::i18n::{fill, tr};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{
    assets::icon,
    backend::receive,
    layers::Layer,
    reuse::{self, Dialog, Mode, Selection},
};

/// How many conversations the destination list shows at once.
const DESTINATIONS: usize = 50;

impl Workspace {
    /// Whether the open conversation is run by an ACP agent, whose turns
    /// are not saved here to be quoted.
    pub(super) fn reuse_acp(&self) -> bool {
        self.state
            .snapshot
            .as_ref()
            .is_some_and(|s| s.acp_agent_id.is_some())
            || !self.options().acp_agent.is_empty()
    }

    /// The start of the range being chosen in the open conversation.
    pub(super) fn reuse_start(&self) -> Option<&Selection> {
        self.reuse_selection
            .as_ref()
            .filter(|selection| self.state.session.as_ref() == Some(&selection.session))
    }

    /// Quote / reuse on the user message at row `index`: starts the range,
    /// or ends it there and reads the turns for the dialog.
    pub(super) fn reuse_turn(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        if self.reuse_loading || !reuse::offered(&self.rows, index, self.reuse_acp(), running) {
            return;
        }
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let Some(selection) = self.reuse_start().cloned() else {
            self.reuse_selection = Selection::begin(&session, &self.rows, index);
            if self.reuse_selection.is_some() {
                self.layers.open(Layer::ReuseRange);
            }
            cx.notify();
            return;
        };
        let Some((first, last)) = selection.range(&self.rows, index) else {
            // The start is no longer in the transcript: begin again here.
            self.reuse_selection = Selection::begin(&session, &self.rows, index);
            cx.notify();
            return;
        };
        self.reuse_loading = true;
        self.notice = None;
        let reply = self.backend.render_session_excerpt(
            project,
            session.clone(),
            first.clone(),
            last.clone(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Cancelled, or another conversation opened, meanwhile.
                if !this.reuse_loading || this.reuse_start().is_none_or(|s| s.session != session) {
                    return;
                }
                this.reuse_loading = false;
                match result {
                    Ok(excerpt) => {
                        this.reuse = Some(Dialog::open(excerpt, first, last));
                        this.reuse_error = None;
                        for field in &this.reuse_fields {
                            field.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        this.layers.open(Layer::SessionReuse);
                    }
                    Err(error) => {
                        this.notice = Some(fill(
                            tr("Could not quote these turns: {}"),
                            &[&crate::i18n::host(&error)],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Gives up the range, and the dialog over it.
    pub(super) fn cancel_reuse(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::ReuseRange);
        self.reuse_selection = None;
        self.reuse_loading = false;
        self.reuse = None;
        self.reuse_error = None;
        cx.notify();
    }

    /// Closes the dialog; the range stays chosen, as in the WebView.
    pub(super) fn close_reuse(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::SessionReuse) {
            self.reuse = None;
            self.reuse_error = None;
            cx.notify();
        }
    }

    fn reuse_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if let Some(dialog) = self.reuse.as_mut().filter(|dialog| !dialog.busy) {
            dialog.mode = mode;
            cx.notify();
        }
    }

    /// Has the conversation's model distill the turns, with the focus as
    /// it reads now; the draft replaces what the field holds.
    fn generate_experience(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let focus = self.reuse_fields[0].read(cx).value().trim().to_owned();
        let Some(dialog) = self.reuse.as_mut() else {
            return;
        };
        let Some(ticket) = dialog.generate() else {
            return;
        };
        self.reuse_error = None;
        let session = dialog.excerpt.session_id.clone();
        let reply = self.backend.draft_session_experience(
            project,
            session.clone(),
            dialog.first.clone(),
            dialog.last.clone(),
            (!focus.is_empty()).then_some(focus),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(dialog) = this
                    .reuse
                    .as_mut()
                    .filter(|dialog| dialog.excerpt.session_id == session)
                else {
                    return;
                };
                if !dialog.generated(ticket) {
                    return;
                }
                match result {
                    Ok(experience) => {
                        dialog.shortened = experience.truncated;
                        dialog.saved = false;
                        this.reuse_fields[1].update(cx, |input, cx| {
                            input.set_value(experience.content, window, cx)
                        });
                    }
                    // The field keeps what it had.
                    Err(error) => this.reuse_error = Some(crate::i18n::host(&error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps the draft as it reads now, with where it came from: in this
    /// project's memory, or as a habit for every project.
    fn save_experience(&mut self, everywhere: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let draft = self.reuse_fields[1].read(cx).value().to_string();
        let Some(dialog) = self.reuse.as_mut() else {
            return;
        };
        if !dialog.can_save(&draft) {
            return;
        }
        dialog.busy = true;
        self.reuse_error = None;
        let session = dialog.excerpt.session_id.clone();
        let text = reuse::memory_text(&dialog.excerpt, &draft);
        let reply = self
            .backend
            .remember(project, Some(session.clone()), text, everywhere);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(dialog) = this
                    .reuse
                    .as_mut()
                    .filter(|dialog| dialog.excerpt.session_id == session)
                {
                    dialog.busy = false;
                    match &result {
                        // Unless it was changed while it was being kept.
                        Ok(()) => {
                            dialog.saved =
                                this.reuse_fields[1].read(cx).value().to_string() == draft
                        }
                        Err(error) => this.reuse_error = Some(crate::i18n::host(error)),
                    }
                }
                if result.is_ok() {
                    this.notice = Some(
                        if everywhere {
                            tr("Global memory saved. It will be used from the next turn.")
                        } else {
                            tr("Memory saved.")
                        }
                        .to_owned(),
                    );
                    this.load_memory(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Puts the quote in front of the destination's draft and opens it
    /// there: a conversation chosen in the project, or a new one made for
    /// it. Nothing is sent; the user adds a request and sends.
    fn attach_reuse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let draft = self.reuse_fields[1].read(cx).value().to_string();
        let Some(dialog) = self.reuse.as_mut() else {
            return;
        };
        if !dialog.can_attach(&draft) {
            return;
        }
        let Some(quote) = reuse::quote(dialog.mode, &dialog.excerpt, &draft) else {
            return;
        };
        match dialog.destination.clone() {
            Some(target) => self.deliver_reuse(target, quote, window, cx),
            None => {
                dialog.busy = true;
                self.reuse_error = None;
                let selection = self.state.selection();
                let reply = self.backend.create_conversation(project);
                cx.spawn_in(window, async move |this, cx| {
                    let result = receive(reply).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        if let Some(dialog) = this.reuse.as_mut() {
                            dialog.busy = false;
                        }
                        // The conversation the quote came from was left.
                        if !this.state.current(&selection) || this.reuse.is_none() {
                            return;
                        }
                        match result {
                            Ok(id) => {
                                this.deliver_reuse(id, quote, window, cx);
                                this.load_sessions(window, cx);
                            }
                            Err(error) => this.reuse_error = Some(crate::i18n::host(&error)),
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            }
        }
    }

    fn deliver_reuse(
        &mut self,
        target: String,
        quote: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_reuse(cx);
        // Leaving the source keeps its draft, and opening the destination
        // brings back what was being written there.
        self.select_session(target, window, cx);
        let filled = reuse::with_quote(&self.composer.read(cx).value(), &quote);
        self.state.save_draft(filled.clone());
        self.fill_composer(filled, window, cx);
        cx.notify();
    }

    /// The bar above the composer while the range is being chosen.
    pub(super) fn render_reuse_range(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let selection = self.reuse_start()?;
        let muted = cx.theme().muted_foreground;
        let body = if self.reuse_loading {
            tr("Preparing the selected turns…").to_owned()
        } else {
            match selection.number(&self.rows, self.before.is_none()) {
                Some(turn) => fill(
                    tr(
                        "Quoting starts at turn {}. Click “End quote here” on that turn or a later one; Esc cancels.",
                    ),
                    &[&turn],
                ),
                None => fill(
                    tr(
                        "Quoting starts at “{}”. Click “End quote here” on that turn or a later one; Esc cancels.",
                    ),
                    &[&crate::present::one_line(&selection.start.text)],
                ),
            }
        };
        Some(
            surfaces::bar("reuse-range", cx)
                .h_flex()
                .gap_2()
                .child(icon("list").text_color(muted))
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(tr("Choose the last turn to quote")),
                        )
                        .child(div().text_xs().text_color(muted).line_clamp(2).child(body)),
                )
                .child(
                    Button::new("reuse-range-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_reuse(cx))),
                ),
        )
    }

    /// The turns or the experience, where the quote goes, and Add to
    /// conversation. A click beside it does not throw a draft away: only
    /// Cancel and Escape leave it.
    pub(super) fn render_session_reuse(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let card = surfaces::dialog(
            "session-reuse",
            DialogWidth::Regular,
            tr("Reuse selected turns"),
        )
        .gap_3();
        let Some(dialog) = &self.reuse else {
            return card;
        };
        let draft = self.reuse_fields[1].read(cx).value().to_string();
        let idle = !dialog.busy && !dialog.generating;
        let mode = |id: &'static str, glyph_name: &'static str, text: &'static str, which: Mode| {
            Button::new(id)
                .icon(glyph(glyph_name))
                .label(text)
                .small()
                .selected(dialog.mode == which)
                .disabled(dialog.busy)
                .on_click(cx.listener(move |this, _, _, cx| this.reuse_mode(which, cx)))
        };
        let mut card = card
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(reuse::source_label(&dialog.excerpt)),
            )
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(mode(
                        "reuse-original",
                        "copy",
                        tr("Original turns"),
                        Mode::Original,
                    ))
                    .child(mode(
                        "reuse-experience",
                        "sparkles",
                        tr("Distill experience"),
                        Mode::Experience,
                    )),
            );
        card = match dialog.mode {
            Mode::Original => card
                .child(
                    div()
                        .id("reuse-preview")
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(dialog.excerpt.text.clone()),
                )
                .when(dialog.excerpt.truncated, |card| {
                    card.child(label(tr("The quoted excerpt was shortened to fit.")))
                }),
            Mode::Experience => card
                .child(label(tr(
                    "Extract when this approach applies, its steps, checks and limits. Review the draft before reusing or saving it.",
                )))
                .child(label(tr("What worked well? (optional)")))
                .child(Input::new(&self.reuse_fields[0]).small().disabled(!idle))
                .child(
                    div().h_flex().child(
                        Button::new("reuse-generate")
                            .icon(glyph("sparkles"))
                            .label(if dialog.generating {
                                tr("Generating experience…")
                            } else if draft.trim().is_empty() {
                                tr("Generate experience")
                            } else {
                                tr("Regenerate experience")
                            })
                            .small()
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.generate_experience(window, cx)
                            })),
                    ),
                )
                .child(label(tr("Editable experience draft")))
                .child(
                    div()
                        .h(px(220.))
                        .child(Input::new(&self.reuse_fields[1]).h_full().disabled(!idle)),
                )
                .when(dialog.shortened, |card| {
                    card.child(label(tr(
                        "Some selected evidence was shortened. Check that the operation you want to reuse is covered.",
                    )))
                })
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_2()
                        .child(if dialog.saved {
                            Button::new("reuse-saved")
                                .icon(glyph("check"))
                                .label(tr("Saved"))
                                .small()
                                .ghost()
                                .disabled(true)
                        } else {
                            Button::new("reuse-save-project")
                                .label(tr("Save for this project"))
                                .small()
                                .disabled(!dialog.can_save(&draft))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_experience(false, window, cx)
                                }))
                        })
                        .when(!dialog.saved, |row| {
                            row.child(
                                Button::new("reuse-save-habit")
                                    .label(tr("Save as a habit"))
                                    .small()
                                    .disabled(!dialog.can_save(&draft))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_experience(true, window, cx)
                                    })),
                            )
                        }),
                ),
        };
        // Where the quote goes.
        let query = self.reuse_fields[2].read(cx).value().to_string();
        let source = dialog.excerpt.session_id.as_str();
        let project = self.state.project.as_deref().unwrap_or_default();
        let found = reuse::destinations(
            &self.sessions,
            project,
            source,
            |id| self.dispatched.contains_key(id),
            &query,
        );
        let mut list = div()
            .id("reuse-destinations")
            .v_flex()
            .gap_0p5()
            .max_h(px(200.))
            .overflow_y_scroll();
        let new_chat = (None, tr("New conversation").to_owned());
        let rows = std::iter::once(new_chat).chain(found.iter().take(DESTINATIONS).map(|row| {
            let title = crate::present::one_line(&row.title);
            let title = if title.trim().is_empty() {
                tr("Untitled conversation").to_owned()
            } else {
                title
            };
            (Some(row.id.clone()), title)
        }));
        for (at, (id, title)) in rows.enumerate() {
            let chosen = dialog.destination == id;
            list = list.child(
                div()
                    .id(("reuse-destination", at))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(glyph(if chosen { "circle-check" } else { "circle" }).small())
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(dialog) = this.reuse.as_mut().filter(|d| !d.busy) {
                            dialog.destination = id.clone();
                        }
                        cx.notify();
                    })),
            );
        }
        if found.len() > DESTINATIONS {
            list = list.child(label(tr("Type more of a title to find the others.")));
        }
        card = card
            .child(label(tr("Destination")))
            .child(Input::new(&self.reuse_fields[2]).small())
            .child(list)
            .child(label(tr(
                "The quote is added to the destination’s draft. Add your request and send when ready.",
            )));
        if let Some(error) = &self.reuse_error {
            card = card.child(error_banner(error, cx));
        }
        card.action(
            Button::new("reuse-cancel")
                .label(tr("Cancel"))
                .small()
                .ghost()
                .disabled(dialog.busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_reuse(cx))),
        )
        .action(
            Button::new("reuse-attach")
                .icon(glyph("conversation-move"))
                .label(tr("Add to conversation"))
                .small()
                .primary()
                .disabled(!dialog.can_attach(&draft))
                .on_click(cx.listener(|this, _, window, cx| this.attach_reuse(window, cx))),
        )
    }
}
