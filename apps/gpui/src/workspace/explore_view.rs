//! Explorations in the window: starting one from an answer, the bar over a
//! conversation that is the source of a round or one of its candidates,
//! the dialog that shows what keeping one brings in, and the questions
//! before a candidate or the whole round is given up.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{PromotionPreview, TurnRef};

use super::{
    Workspace, chip,
    shell::glyph,
    surfaces::{self, DialogWidth},
    tone_color,
};
use crate::{backend::receive, controls, explore, layers::Layer, present::Tone};

/// What the exploration question is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ExploreQuestion {
    Discard(String),
    Abandon(String),
    FinishByHand(String),
}

impl Workspace {
    /// The current round of the open project, for the bar.
    pub(super) fn load_explorations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.explorations(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.project.as_ref() == Some(&project) {
                    this.explorations = result.unwrap_or_default();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether the open conversation may start an exploration.
    pub(super) fn can_explore(&self) -> bool {
        let Some(session) = self.state.session.as_deref() else {
            return false;
        };
        let state = self
            .state
            .snapshot
            .as_ref()
            .and_then(|s| s.exploration.as_ref());
        explore::can_start(
            state,
            session,
            self.branched.contains_key(session),
            !self.options().acp_agent.is_empty(),
        )
    }

    /// Asks for a name, then starts an exploration from the answer at
    /// `index`.
    pub(super) fn ask_explore_from(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(turn) = controls::turn_of(&self.rows, index) else {
            return;
        };
        let source = self.state.session.clone().unwrap_or_default();
        let name = explore::default_name(explore::candidates(&self.explorations, &source).len());
        self.explore_name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.explore_from = Some(turn);
        self.explore_note = None;
        self.layers.open(Layer::StartExploration);
        self.explore_name
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn start_exploration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project, session)) = self.state.project.clone().zip(self.state.session.clone())
        else {
            return;
        };
        let name = self.explore_name.read(cx).value().trim().to_owned();
        if name.is_empty() || self.explore_busy {
            return;
        }
        self.explore_busy = true;
        let turn: Option<TurnRef> = self.explore_from.clone();
        let reply = self
            .backend
            .start_exploration(project.clone(), session, turn, name);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.explore_busy = false;
                match result {
                    Ok(started) => {
                        this.layers.close(Layer::StartExploration);
                        this.load_explorations(window, cx);
                        this.open_session(project, started.session, window, cx);
                    }
                    Err(error) => this.explore_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_preview(&mut self, exploration: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.explore_note = None;
        self.explore_preview = None;
        self.layers.open(Layer::ExplorationPreview);
        let reply = self
            .backend
            .preview_exploration(project, exploration.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(preview) => this.explore_preview = Some((exploration, preview)),
                    Err(error) => this.explore_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn keep_exploration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some((exploration, preview))) =
            (self.state.project.clone(), self.explore_preview.clone())
        else {
            return;
        };
        self.explore_busy = true;
        let reply = self
            .backend
            .promote_exploration(project.clone(), exploration, preview.guard);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.explore_busy = false;
                match result {
                    Ok(mainline) => {
                        this.layers.close(Layer::ExplorationPreview);
                        this.explore_preview = None;
                        this.load_explorations(window, cx);
                        this.open_session(project, mainline, window, cx);
                    }
                    Err(error) => this.explore_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows the mainline's folder and the exploration's, to copy files
    /// over by hand.
    fn show_folders(&mut self, exploration: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.exploration_folders(project, exploration);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((mainline, exploration)) => {
                        cx.reveal_path(&mainline);
                        cx.reveal_path(&exploration);
                    }
                    Err(error) => this.explore_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn answer_explore_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(question)) =
            (self.state.project.clone(), self.explore_question.take())
        else {
            return;
        };
        self.layers.close(Layer::ConfirmExploration);
        let source = self
            .state
            .snapshot
            .as_ref()
            .and_then(|s| s.exploration.as_ref())
            .and_then(|state| state.round_source_frame_id.clone());
        let reply = match &question {
            ExploreQuestion::Discard(id) => self
                .backend
                .discard_exploration(project.clone(), id.clone()),
            ExploreQuestion::Abandon(source) => self
                .backend
                .abandon_exploration_round(project.clone(), source.clone()),
            ExploreQuestion::FinishByHand(id) => self
                .backend
                .finish_manual_resolution(project.clone(), id.clone()),
        };
        let back = match &question {
            ExploreQuestion::Abandon(source) => Some(source.clone()),
            _ => source,
        };
        self.explore_busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.explore_busy = false;
                match result {
                    Ok(()) => {
                        this.layers.close(Layer::ExplorationPreview);
                        this.explore_preview = None;
                        this.load_explorations(window, cx);
                        // Back to the mainline conversation the round grew from.
                        if let Some(source) = back {
                            this.open_session(project, source, window, cx);
                        }
                    }
                    Err(error) => this.explore_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_explore(&mut self, question: ExploreQuestion, cx: &mut Context<Self>) {
        self.explore_question = Some(question);
        self.layers.open(Layer::ConfirmExploration);
        cx.notify();
    }

    fn explore_question_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.layers.contains(Layer::ConfirmExploration) {
            return None;
        }
        let question = self.explore_question.as_ref()?;
        let (said, yes) = match question {
            ExploreQuestion::Discard(_) => (
                tr("Discard this exploration? Its copy of the files and its conversation go."),
                tr("Discard"),
            ),
            ExploreQuestion::Abandon(_) => (
                tr("Abandon the round? Every exploration goes and the mainline stays as it was."),
                tr("Abandon"),
            ),
            ExploreQuestion::FinishByHand(_) => (
                tr(
                    "Finish by hand? Keep the files you copied; the explorations' conversations and records go.",
                ),
                tr("Finish"),
            ),
        };
        Some(
            div()
                .h_flex()
                .gap_2()
                .child(div().text_xs().child(said))
                .child(
                    Button::new("explore-question-yes")
                        .label(yes)
                        .small()
                        .danger()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.answer_explore_question(window, cx)
                        })),
                )
                .child(
                    Button::new("explore-question-no")
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmExploration);
                            this.explore_question = None;
                            cx.notify();
                        })),
                ),
        )
    }

    /// The bar over a round's source or one of its candidates.
    pub(super) fn render_exploration_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.state.snapshot.as_ref()?.exploration.as_ref()?;
        let session = self.state.session.clone()?;
        let muted = cx.theme().muted_foreground;
        let flask = tone_color(Tone::Active, cx);
        let mut bar = div()
            .v_flex()
            .gap_2()
            .px_4()
            .py_2()
            .flex_shrink_0()
            .bg(flask.opacity(0.06));
        if let Some(summary) = &state.summary {
            let id = summary.exploration.id.clone();
            let source = summary.source_frame_id.clone();
            let busy = self.explore_busy;
            bar = bar
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(glyph("flask").text_color(flask))
                        .child(
                            div()
                                .text_sm()
                                .child(summary.exploration.name.clone()),
                        )
                        .child(chip(
                            explore::status_label(&summary.exploration.status),
                            Tone::Neutral,
                            cx,
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(explore::isolation_label(summary)),
                        ),
                )
                .child(div().text_xs().text_color(muted).child(tr(
                    "An exploration works in its own copy of the project. Effects outside this computer are not rolled back.",
                )))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("explore-view")
                                .label(tr("View changes and keep"))
                                .small()
                                .primary()
                                .disabled(busy)
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _, window, cx| this.open_preview(id.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new("explore-discard")
                                .label(tr("Discard"))
                                .small()
                                .disabled(busy)
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _, _, cx| {
                                        this.ask_explore(ExploreQuestion::Discard(id.clone()), cx)
                                    }
                                })),
                        )
                        .child(
                            Button::new("explore-back")
                                .label(tr("Back to the mainline"))
                                .small()
                                .ghost()
                                .on_click(cx.listener({
                                    let source = source.clone();
                                    move |this, _, window, cx| {
                                        if let Some(project) = this.state.project.clone() {
                                            this.open_session(project, source.clone(), window, cx);
                                        }
                                    }
                                })),
                        )
                        .child(
                            Button::new("explore-abandon")
                                .label(tr("Abandon the round"))
                                .small()
                                .ghost()
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ask_explore(ExploreQuestion::Abandon(source.clone()), cx)
                                })),
                        ),
                );
        } else if state.source_frozen {
            let candidates = explore::candidates(&self.explorations, &session);
            bar = bar.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(glyph("flask").text_color(flask))
                    .child(div().text_sm().child(crate::i18n::fill(
                        tr("Mainline checkpoint · {} exploration(s) in this round"),
                        &[&state.candidate_count],
                    ))),
            );
            // Why it cannot be written to is said once, by the card in the
            // composer's place.
            let mut row = div().flex().flex_wrap().gap_1();
            for candidate in candidates {
                let frame = candidate.exploration.frame_id.clone();
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "explore-open-{}",
                        candidate.exploration.id
                    )))
                    .label(format!(
                        "{} · {}",
                        candidate.exploration.name,
                        explore::status_label(&candidate.exploration.status)
                    ))
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(project) = this.state.project.clone() {
                            this.open_session(project, frame.clone(), window, cx);
                        }
                    })),
                );
            }
            row = row.child(
                Button::new("explore-abandon-source")
                    .label(tr("Abandon the round"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_explore(ExploreQuestion::Abandon(session.clone()), cx)
                    })),
            );
            bar = bar.child(row);
        } else {
            return None;
        }
        if let Some(question) = self.explore_question_row(cx) {
            bar = bar.child(question);
        }
        if !self.layers.contains(Layer::ExplorationPreview) {
            if let Some(Err(error)) = &self.explore_note {
                bar = bar.child(chip(crate::i18n::host(error), Tone::Danger, cx));
            }
        }
        Some(bar)
    }

    fn file_list(
        &self,
        title: &'static str,
        files: &[wisp_server::desktop::FileChange],
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut list = div().v_flex().gap_1().child(div().text_sm().child(title));
        if files.is_empty() {
            return list.child(div().text_xs().text_color(muted).child(tr("None.")));
        }
        for file in files.iter().take(200) {
            list = list.child(
                div()
                    .h_flex()
                    .gap_2()
                    .text_xs()
                    .child(div().w(px(70.)).text_color(muted).child(match file.kind {
                        "added" => tr("added"),
                        "deleted" => tr("deleted"),
                        _ => tr("modified"),
                    }))
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .truncate()
                            .child(file.path.clone()),
                    ),
            );
        }
        list
    }

    pub(super) fn render_exploration_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut card = surfaces::dialog(
            "exploration-preview",
            DialogWidth::Regular,
            tr("Keep this exploration"),
        )
        .gap_3()
        .close_on_click_outside(cx.listener(|this, _, _, cx| {
            this.layers.close(Layer::ExplorationPreview);
            cx.notify();
        }));
        let mut actions = Vec::new();
        match &self.explore_preview {
            None => {
                card = card.child(div().text_sm().text_color(muted).child(tr("Loading…")));
            }
            Some((id, preview)) => {
                let (body, its_actions) = self.preview_body(id, preview, cx);
                card = card.child(body);
                actions = its_actions;
            }
        }
        if let Some(Err(error)) = &self.explore_note {
            card = card.child(chip(crate::i18n::host(error), Tone::Danger, cx));
        }
        card.actions(actions)
    }

    /// What keeping the exploration would change, and the footer's actions:
    /// those for finishing by hand, Close, then Keep at the right end.
    fn preview_body(
        &self,
        id: &str,
        preview: &PromotionPreview,
        cx: &mut Context<Self>,
    ) -> (Div, Vec<Button>) {
        let muted = cx.theme().muted_foreground;
        let mut body = div()
            .v_flex()
            .gap_3()
            .child(self.file_list(tr("This exploration changed"), &preview.files, cx))
            .child(self.file_list(
                tr("The mainline changed since the checkpoint"),
                &preview.mainline_files,
                cx,
            ))
            .child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                tr("Also: {} artifact(s), {} run(s), {} decision(s); {} effect(s) outside this computer, which keeping does not undo."),
                &[&preview.artifacts, &preview.runs, &preview.decisions, &preview.external_effects],
            )));
        for blocker in &preview.blockers {
            body = body.child(chip(crate::i18n::host(blocker), Tone::Attention, cx));
        }
        let busy = self.explore_busy;
        let mut actions = Vec::new();
        if preview.manual_resolution {
            let shown = id.to_owned();
            let finished = id.to_owned();
            actions.push(
                Button::new("exploration-folders")
                    .label(tr("Show both folders"))
                    .small()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.show_folders(shown.clone(), window, cx)
                    })),
            );
            actions.push(
                Button::new("exploration-by-hand")
                    .label(tr("Finish by hand"))
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_explore(ExploreQuestion::FinishByHand(finished.clone()), cx)
                    })),
            );
        }
        actions.push(
            Button::new("exploration-preview-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::ExplorationPreview);
                    cx.notify();
                })),
        );
        actions.push(
            Button::new("exploration-keep")
                .label(if busy {
                    tr("Keeping…")
                } else {
                    tr("Keep it as the mainline")
                })
                .small()
                .primary()
                .disabled(!preview.eligible || busy)
                .on_click(cx.listener(|this, _, window, cx| this.keep_exploration(window, cx))),
        );
        if let Some(question) = self.explore_question_row(cx) {
            body = body.child(question);
        }
        (body, actions)
    }

    pub(super) fn render_start_exploration(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ready = !self.explore_name.read(cx).value().trim().is_empty() && !self.explore_busy;
        let mut card = surfaces::dialog(
            "start-exploration",
            DialogWidth::Narrow,
            tr("Start an exploration"),
        )
        .gap_3()
        .close_on_click_outside(cx.listener(|this, _, _, cx| {
            this.layers.close(Layer::StartExploration);
            cx.notify();
        }))
        .child(div().text_xs().text_color(muted).child(tr(
                "The exploration continues this conversation up to the chosen answer, in its own copy of the project's files as they are now (earlier file versions are not brought back). Keep one later, or discard them.",
            )))
            .child(super::surfaces::input(&self.explore_name, cx).small());
        if let Some(Err(error)) = &self.explore_note {
            card = card.child(chip(crate::i18n::host(error), Tone::Danger, cx));
        }
        card.action(
            Button::new("start-exploration-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::StartExploration);
                    cx.notify();
                })),
        )
        .action(
            Button::new("start-exploration-go")
                .label(if self.explore_busy {
                    tr("Copying the project…")
                } else {
                    tr("Start")
                })
                .small()
                .primary()
                .disabled(!ready)
                .on_click(cx.listener(|this, _, window, cx| this.start_exploration(window, cx))),
        )
    }
}
