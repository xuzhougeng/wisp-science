//! A run's card under the call in the transcript that submitted or waits on
//! it, as the WebView's run card: its state in a word and an icon, where
//! and how long it ran, and what can be done with it now (ask aside, move
//! it to the background, cancel it, look through what it left on its SSH
//! host, open it in the Runs tab). Opening the call shows the run's command,
//! its remote folder and the last of its output. A card is inline, not a
//! layer, so it is not on the Escape stack.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};

use super::{Workspace, chip, shell::icon_button, tone_color};
use crate::{
    assets::icon,
    backend::receive,
    i18n::tr,
    panel::PanelTab,
    present,
    run_cards::{self, Action},
};

impl Workspace {
    /// Works out again which rows draw a run's card, and has the list
    /// measure again each row whose card came, went or changed shape.
    pub(super) fn place_run_cards(&mut self) {
        let runs = self
            .state
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.run_cards.as_slice())
            .unwrap_or_default();
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let placed: std::collections::HashMap<usize, (String, u64)> =
            run_cards::placed(&self.rows, runs, running)
                .into_iter()
                .map(|(index, id)| {
                    let record = runs.iter().find(|run| run.id == id);
                    let backgrounded = record.is_some_and(|run| {
                        run_cards::backgrounded(run, self.backgrounded_runs.contains(&id))
                    });
                    let shape = run_cards::shape(record, backgrounded);
                    (index, (id, shape))
                })
                .collect();
        let changed: Vec<usize> = placed
            .iter()
            .filter(|(index, card)| self.run_card_rows.get(index) != Some(card))
            .map(|(index, _)| *index)
            .chain(
                self.run_card_rows
                    .keys()
                    .filter(|index| !placed.contains_key(index))
                    .copied(),
            )
            .collect();
        self.run_card_rows = placed;
        for index in changed {
            if index < self.rows.len() {
                self.list.splice(index..index + 1, 1);
            }
        }
    }

    /// The card of the run `id` under the tool row at `index`; `open` when
    /// that row is unfolded.
    pub(super) fn run_card(
        &self,
        index: usize,
        id: &str,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let record = self
            .state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.run_cards.iter().find(|run| run.id == id));
        // The state the row was saved with, when the record is not there.
        let status = record.map(|run| run.status.clone()).or_else(|| {
            self.rows
                .get(index)
                .and_then(|row| row.run.as_ref())
                .map(|run| run.status.clone())
        });
        let status = status.unwrap_or_default();
        let (state, tone) = match record {
            Some(run) => present::run_status(run),
            None => present::run_state(&status, None),
        };
        let colour = tone_color(tone, cx);
        let backgrounded = record
            .is_some_and(|run| run_cards::backgrounded(run, self.backgrounded_runs.contains(id)));
        let mut head =
            div()
                .h_flex()
                .gap_2()
                .child(icon(run_cards::status_icon(&status)).text_color(colour))
                .child(
                    div().flex_1().min_w_0().truncate().text_sm().child(
                        record.map_or_else(|| tr("Run").to_owned(), |run| run.title.clone()),
                    ),
                )
                .child(chip(state, tone, cx));
        let actions = record.map_or_else(Vec::new, |run| run_cards::actions(run, backgrounded));
        for action in actions {
            let wanted = id.to_owned();
            let title = record.map(|run| run.title.clone()).unwrap_or_default();
            head = head.child(match action {
                Action::Ask => icon_button(
                    SharedString::from(format!("run-card-ask-{index}")),
                    "chat",
                    "Ask aside about this run, with its live state",
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_aside_about_run(wanted.clone(), title.clone(), window, cx)
                })),
                Action::Background => icon_button(
                    SharedString::from(format!("run-card-background-{index}")),
                    "bell",
                    "Move to background and get a message when it ends",
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.background_run(wanted.clone(), window, cx)
                })),
                Action::Cancel => icon_button(
                    SharedString::from(format!("run-card-cancel-{index}")),
                    "stop",
                    "Cancel run",
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.cancel_run(wanted.clone(), window, cx)
                })),
                Action::Review => icon_button(
                    SharedString::from(format!("run-card-review-{index}")),
                    "folder",
                    "Look through what it left on its host",
                )
                .xsmall()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_run_review(wanted.clone(), title.clone(), window, cx)
                })),
            });
        }
        {
            let wanted = id.to_owned();
            head = head.child(
                Button::new(SharedString::from(format!("show-run-{index}")))
                    .label(tr("Show in Runs"))
                    .xsmall()
                    .ghost()
                    .tooltip(tr(
                        "The run's whole output, log and clean-up in the Runs tab",
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.expanded_run = Some(wanted.clone());
                        this.show_panel_tab(PanelTab::Runs, window, cx);
                    })),
            );
        }
        let mut card = div().v_flex().gap_1().px_3().pb_2().child(head);
        let Some(run) = record else {
            return card.into_any_element();
        };
        card = card.child(
            div()
                .text_xs()
                .text_color(muted)
                .truncate()
                .child(run_cards::meta(run, super::shell::unix_now())),
        );
        if backgrounded {
            card = card.child(
                div()
                    .h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(icon("bell").text_color(cx.theme().info))
                    .child(tr(
                        "Running in the background. This conversation gets a message when it ends.",
                    )),
            );
        }
        // A run moving files says how far it has got.
        if let Some(progress) = crate::transfers::progress(run) {
            card = card.child(super::transfers_view::meter(&progress, cx));
        }
        if let Some(problem) = run
            .last_poll_error
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(crate::i18n::host(problem)),
            );
        }
        if open {
            if let Some(workdir) = &run.remote_workdir {
                card = card.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Remote workdir"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(workdir.clone()),
                        ),
                );
            }
            card = card.child(
                TextView::markdown(
                    SharedString::from(format!("run-card-output-{index}")),
                    present::run_output(run),
                    window,
                    cx,
                )
                .selectable(true),
            );
        }
        card.into_any_element()
    }

    /// Ends a turn's wait on `run` without touching the run; the card says
    /// it goes on in the background until the next snapshot does.
    fn background_run(&mut self, run: String, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        self.error = None;
        self.backgrounded_runs.insert(run.clone());
        self.place_run_cards();
        let chinese = crate::i18n::current() == crate::i18n::Locale::Zh;
        let reply = self.backend.background_run(project, run.clone(), chinese);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.backgrounded_runs.remove(&run);
                    if this.state.current(&selection) {
                        this.error = Some(error);
                    }
                }
                if this.state.current(&selection) {
                    this.place_run_cards();
                    // The turn that waited ends with the next snapshots.
                    this.start_poll(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
