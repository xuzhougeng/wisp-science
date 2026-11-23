//! The side panel's tabs beside Files: the conversation's runs, which can be
//! stopped from here, and the files conversations produced.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
    surfaces,
};
use crate::{
    assets::icon,
    backend::receive,
    layers::Layer,
    method_search,
    panel::{self, PanelTab},
    present,
};

impl Workspace {
    /// Opens the side panel on `tab`, or switches to it.
    pub(super) fn show_panel_tab(
        &mut self,
        tab: PanelTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::FilePreview);
        self.layers.close(Layer::PanelTabMenu);
        // The panel's preview, not a document shown in the centre.
        if !self.preview_in_centre() {
            self.files.close_preview();
        }
        // What a tab showed in place of its list is not left open, out of
        // sight, for Escape to close.
        if tab != PanelTab::Files {
            self.close_remote_look();
        }
        if tab != PanelTab::Environments {
            self.host_detail = None;
            self.staged_removing = None;
            self.layers.close(Layer::HostDetail);
        }
        self.layers.open(Layer::FilesPanel);
        self.panel_tab = tab;
        self.keep_panel_tab(tab, window, cx);
        match tab {
            PanelTab::Files => {
                self.ensure_files(window, cx);
                // The hosts that can be looked at beside the project.
                self.load_environments(window, cx);
            }
            PanelTab::Artifacts => self.load_artifacts(window, cx),
            PanelTab::Memos => self.load_memos(window, cx),
            PanelTab::Memory => self.load_memory(window, cx),
            // Cells show whether their code is saved.
            PanelTab::Highlights | PanelTab::Notebook => self.load_saved(window, cx),
            PanelTab::Agents => self.load_batches(window, cx),
            PanelTab::Environments => {
                self.load_environments(window, cx);
                self.load_conversation_hosts(window, cx);
            }
            // Read from the transcript as it is.
            PanelTab::Runs | PanelTab::Aside | PanelTab::Provenance => {}
        }
        cx.notify();
    }

    /// Keeps `tab` in the panel's row, remembering it when it had been taken
    /// away: opening a tab, from its row or from elsewhere, adds it back.
    pub(super) fn keep_panel_tab(
        &mut self,
        tab: PanelTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.kept_tabs.contains(&tab) {
            self.kept_tabs = panel::visible(&self.kept_tabs, tab);
            self.save_panel_tabs(window, cx);
        }
    }

    /// Takes `tab` out of the panel's row. Taking the last one away closes
    /// the panel; it opens on that tab again.
    fn take_away_panel_tab(&mut self, tab: PanelTab, window: &mut Window, cx: &mut Context<Self>) {
        let (left, next) = panel::take_away(&self.kept_tabs, self.panel_tab, tab);
        self.kept_tabs = left;
        self.save_panel_tabs(window, cx);
        match next {
            Some(next) if next != self.panel_tab => self.show_panel_tab(next, window, cx),
            Some(_) => {}
            None => {
                if self.layers.contains(Layer::FilesPanel) {
                    self.toggle_files(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// The strip's More menu: the tabs there is no room for on its line,
    /// then taking the open tab away, then the tabs that were taken away,
    /// to add one back. Each is an icon and a word.
    fn panel_tab_menu(
        &self,
        overflow: &[PanelTab],
        missing: &[PanelTab],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let line = cx.theme().border;
        let rule = move || div().h(px(1.)).mx_2().my_1().flex_shrink_0().bg(line);
        let mut menu = div()
            .id("panel-tab-menu")
            .absolute()
            .top(px(surfaces::ICON_BUTTON + 4.))
            .right_0()
            .w(px(220.))
            .max_h(surfaces::menu_room(None, false))
            .overflow_y_scroll()
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(surfaces::surface_border(cx))
            .bg(cx.theme().popover)
            .shadow_md();
        for tab in overflow.iter().copied() {
            menu = menu.child(
                Self::menu_item(
                    SharedString::from(format!("panel-more-{}", tab.key())),
                    tab.icon(),
                    tab.label(),
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.show_panel_tab(tab, window, cx)),
                ),
            );
        }
        if !overflow.is_empty() {
            menu = menu.child(rule());
        }
        let open = self.panel_tab;
        menu = menu.child(
            Self::menu_item(
                SharedString::from(format!("panel-take-away-{}", open.key())),
                "minus",
                tr("Remove this tab"),
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.layers.close(Layer::PanelTabMenu);
                this.take_away_panel_tab(open, window, cx)
            })),
        );
        if !missing.is_empty() {
            menu = menu.child(rule());
        }
        for tab in missing.iter().copied() {
            menu = menu.child(
                Self::menu_item(
                    SharedString::from(format!("panel-add-{}", tab.key())),
                    tab.icon(),
                    tab.label(),
                    cx,
                )
                .child(icon("plus").text_color(cx.theme().muted_foreground))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.show_panel_tab(tab, window, cx)),
                ),
            );
        }
        menu
    }

    /// Lists what the conversation produced, or the whole project.
    pub(super) fn load_artifacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let session = selection.session.clone().filter(|_| !self.artifacts_all);
        // Without a conversation there is nothing of its own to list.
        if session.is_none() && !self.artifacts_all {
            self.artifacts.clear();
            return;
        }
        self.artifacts_request += 1;
        let request = self.artifacts_request;
        let reply = self.backend.artifacts(project, session, String::new(), 200);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Only the latest request for the conversation still open.
                if !this.state.current(&selection) || this.artifacts_request != request {
                    return;
                }
                match result {
                    Ok(artifacts) => this.artifacts = artifacts,
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_artifact_scope(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.artifacts_all != all {
            self.artifacts_all = all;
            self.artifacts.clear();
            self.load_artifacts(window, cx);
            cx.notify();
        }
    }

    /// Asks a run to stop, or with `force` finishes one already stopping
    /// whose stop never came back, as the WebView's Force cancel does.
    pub(super) fn cancel_run(
        &mut self,
        run: String,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        self.error = None;
        let reply = self.backend.cancel_run(project, run, force);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                // The run's new status arrives with the next snapshot.
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks before a run's workspace is deleted, under its card.
    fn ask_clean_run(&mut self, run: String, cx: &mut Context<Self>) {
        self.run_cleaning = Some((run, false));
        self.layers.open(Layer::ConfirmCleanRun);
        cx.notify();
    }

    /// Deletes what a finished run left where it ran. Refused because
    /// outputs were never brought back, the question asks again whether
    /// to lose them.
    fn clean_run(&mut self, run: String, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        self.error = None;
        let reply = self.backend.clean_run(project, run.clone(), force);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.run_cleaning = None;
                        this.layers.close(Layer::ConfirmCleanRun);
                        this.notice = Some(tr("The run's workspace is cleaned.").into());
                    }
                    Err(error) if !force && panel::outputs_at_stake(&error) => {
                        this.run_cleaning = Some((run, true));
                    }
                    Err(error) => {
                        this.run_cleaning = None;
                        this.layers.close(Layer::ConfirmCleanRun);
                        this.error = Some(error);
                    }
                }
                // The run's new state arrives with the next snapshot.
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The question under a run's card before its workspace goes.
    fn clean_question(&self, run: &str, at_stake: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let (asked, run) = (run.to_owned(), run.to_owned());
        surfaces::panel_card(SharedString::from(format!("run-clean-question-{run}")), cx)
            .when(at_stake, |card| card.border_color(cx.theme().danger))
            .child(div().text_xs().child(if at_stake {
                tr("Its outputs were never brought back; cleaning up anyway loses them.")
            } else {
                tr(
                    "Delete this run's workspace where it ran? Its logs are saved into the \
                    project first.",
                )
            }))
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child({
                        let button =
                            Button::new(SharedString::from(format!("run-clean-yes-{run}")))
                                .label(if at_stake {
                                    tr("Clean up anyway")
                                } else {
                                    tr("Clean up")
                                })
                                .xsmall()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.clean_run(asked.clone(), at_stake, window, cx)
                                }));
                        if at_stake {
                            button.danger()
                        } else {
                            button.primary()
                        }
                    })
                    .child(
                        Button::new(SharedString::from(format!("run-clean-no-{run}")))
                            .label(tr("Cancel"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cleaning = None;
                                this.layers.close(Layer::ConfirmCleanRun);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// Forgets the open method search card.
    pub(super) fn close_search(&mut self) {
        self.open_search = None;
        self.search_details = None;
        self.search_busy = false;
        self.search_problem = None;
        self.search_output = None;
    }

    fn toggle_search(&mut self, run: String, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.open_search.as_ref() != Some(&run);
        self.close_search();
        if open {
            self.open_search = Some(run);
            self.load_search(None, window, cx);
        }
        cx.notify();
    }

    /// Reads the open search's card, or applies one of its controls and
    /// reads the card it leaves.
    fn load_search(
        &mut self,
        control: Option<method_search::Control>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(run)) = (selection.project.clone(), self.open_search.clone())
        else {
            return;
        };
        if self.search_busy {
            return;
        }
        self.search_busy = true;
        self.search_problem = None;
        let reply = self.backend.method_search(project, run.clone(), control);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.open_search.as_ref() != Some(&run) {
                    return;
                }
                this.search_busy = false;
                match result {
                    Ok(details) => this.search_details = Some(details),
                    // A refused control leaves the card as it was.
                    Err(error) => this.search_problem = Some(error),
                }
                // The run's new status arrives with the next snapshot.
                if control.is_some() {
                    this.start_poll(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads the open search's card again once its run has moved on.
    pub(super) fn refresh_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(open), Some(details)) = (&self.open_search, &self.search_details) else {
            return;
        };
        let moved = self
            .state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.run_cards.iter().find(|run| &run.id == open))
            .is_some_and(|run| method_search::stale(details, run));
        if moved && !self.search_busy {
            self.load_search(None, window, cx);
        }
    }

    /// Shows one file the search promoted, or hides it when it is showing.
    fn toggle_search_output(
        &mut self,
        version: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .search_output
            .as_ref()
            .is_some_and(|(shown, _)| *shown == version)
        {
            self.search_output = None;
            cx.notify();
            return;
        }
        let selection = self.state.selection();
        let (Some(project), Some(run)) = (selection.project.clone(), self.open_search.clone())
        else {
            return;
        };
        self.search_output = None;
        let reply = self
            .backend
            .method_search_output(project, run.clone(), version.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.current(&selection) && this.open_search.as_ref() == Some(&run) {
                    this.search_output = Some((version, result));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The open search's card: its frozen contract, what can be done with
    /// it, how far it has got, what it kept and what it tried.
    fn render_method_search(
        &self,
        run: &wisp_dto::RunRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let mut card = div().v_flex().gap_2().pt_1();
        if let Some(problem) = &self.search_problem {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(danger)
                    .child(crate::i18n::host(problem)),
            );
        }
        let Some(details) = &self.search_details else {
            if self.search_busy {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Reading the search…")),
                );
            }
            return card;
        };
        let rows = |rows: Vec<(&'static str, String)>| {
            let mut grid = div().v_flex().gap_0p5();
            for (label, value) in rows {
                grid = grid.child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .w(px(104.))
                                .flex_shrink_0()
                                .text_color(muted)
                                .child(label),
                        )
                        .child(div().flex_1().min_w_0().child(value)),
                );
            }
            grid
        };

        // What the researcher approves by starting it.
        card = card
            .child(surfaces::panel_section(tr("Contract"), cx))
            .child(div().text_sm().child(details.spec.objective.clone()))
            .child(rows(method_search::contract(details)));
        let guardrails = method_search::guardrails(details);
        if !guardrails.is_empty() {
            card = card.child(rows(vec![(tr("Guardrails"), guardrails.join(", "))]));
        }
        for constraint in &details.spec.constraints {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("· {constraint}")),
            );
        }
        let mut actions = div().h_flex().gap_1();
        if let Some(control) = method_search::control(&details.run.status) {
            let mut button = Button::new(SharedString::from(format!(
                "search-{}-{}",
                control.key(),
                run.id
            )))
            .icon(glyph(control.icon()))
            .label(control.label())
            .xsmall()
            .disabled(self.search_busy)
            .on_click(
                cx.listener(move |this, _, window, cx| this.load_search(Some(control), window, cx)),
            );
            if control != method_search::Control::Pause {
                button = button.primary();
            }
            actions = actions.child(button);
        }
        actions = actions.child(
            icon_button(
                SharedString::from(format!("search-refresh-{}", run.id)),
                "refresh",
                tr("Read the search again"),
            )
            .xsmall()
            .disabled(self.search_busy)
            .on_click(cx.listener(|this, _, window, cx| this.load_search(None, window, cx))),
        );
        card = card.child(actions);

        // How far it has got.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        card = card
            .child(surfaces::panel_section(tr("Progress"), cx))
            .child(rows(method_search::progress_rows(
                &method_search::progress(&details.run),
                now,
            )));

        // What it kept.
        if !details.outputs.is_empty() {
            let mut files = div().h_flex().flex_wrap().gap_1();
            for output in &details.outputs {
                let version = output.artifact_version_id.clone();
                let shown = self
                    .search_output
                    .as_ref()
                    .is_some_and(|(open, _)| *open == version);
                files = files.child(
                    Button::new(SharedString::from(format!("search-output-{version}")))
                        .icon(glyph("doc"))
                        .label(method_search::output_name(output))
                        .xsmall()
                        .ghost()
                        .selected(shown)
                        .tooltip(output.source_path.clone())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_search_output(version.clone(), window, cx)
                        })),
                );
            }
            card = card
                .child(surfaces::panel_section(tr("Outputs"), cx))
                .child(files);
            if let Some((version, text)) = &self.search_output {
                let path = details
                    .outputs
                    .iter()
                    .find(|output| output.artifact_version_id == *version)
                    .map(|output| output.source_path.as_str())
                    .unwrap_or_default();
                card = card.child(match text {
                    Ok(text) => surfaces::markdown(
                        SharedString::from(format!("search-output-text-{version}")),
                        method_search::output_markdown(path, text),
                        window,
                        cx,
                    )
                    .selectable(true)
                    .into_any_element(),
                    Err(problem) => div()
                        .text_xs()
                        .text_color(danger)
                        .child(crate::i18n::host(problem))
                        .into_any_element(),
                });
            }
        }

        // What it tried, and what it leans on now.
        if !details.candidates.is_empty() {
            card = card.child(surfaces::panel_section(
                crate::i18n::fill(tr("Candidates ({})"), &[&details.candidates.len()]),
                cx,
            ));
            for candidate in &details.candidates {
                let mut line = div()
                    .v_flex()
                    .text_xs()
                    .child(method_search::candidate(candidate));
                if let Some(rationale) = &candidate.rationale {
                    line = line.child(div().text_color(muted).child(rationale.clone()));
                }
                if let Some(error) = &candidate.error {
                    line = line.child(div().text_color(danger).child(error.clone()));
                }
                card = card.child(line);
            }
        }
        if !details.strategies.is_empty() {
            card = card.child(surfaces::panel_section(tr("Strategies"), cx));
            for strategy in &details.strategies {
                card = card.child(div().text_xs().child(method_search::strategy(strategy)));
            }
        }
        card.child(
            div()
                .text_xs()
                .text_color(muted)
                .child(method_search::integrity(details)),
        )
    }

    /// The row of tabs at the top of the side panel.
    /// The side panel's tab strip, on one line: the open tab a chip of its
    /// icon and name, the others icon buttons named by their tooltip, in
    /// the panel's order as far as the width allows; then the rest (the
    /// tabs with no room, taking the open one away, adding one back), its
    /// chevron the strip's last button. The bar's side-panel toggle and
    /// Escape close the panel; the strip has no close cross of its own.
    pub(super) fn panel_tabs(&self, cx: &mut Context<Self>) -> Div {
        let active = self
            .state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| panel::active_runs(&snapshot.run_cards));
        let order = panel::visible(&self.kept_tabs, self.panel_tab);
        let width = self.panel_width.get().unwrap_or(340.);
        let (shown, overflow) = panel::strip(&order, self.panel_tab, panel::strip_capacity(width));
        let mut tabs = div()
            .h_flex()
            .items_center()
            .gap_0p5()
            .flex_1()
            .min_w_0()
            .overflow_hidden();
        for tab in shown {
            let selected = self.panel_tab == tab;
            // Runs that are still going add their count to the open tab.
            let label = match (tab, active) {
                (PanelTab::Runs, 1..) => crate::i18n::fill(tr("Runs · {}"), &[&active]),
                _ => tab.label().to_owned(),
            };
            tabs = tabs.child(surfaces::panel_tab(
                SharedString::from(format!("panel-tab-{}", tab.key())),
                tab.icon(),
                label,
                selected,
                cx.listener(move |this, _, window, cx| this.show_panel_tab(tab, window, cx)),
                cx,
            ));
        }
        let missing = panel::missing(&self.kept_tabs, self.panel_tab);
        let open = self.layers.contains(Layer::PanelTabMenu);
        let mut more = div().relative().flex_none().child(
            surfaces::icon_action(
                "panel-more",
                // The rest of the tabs, not more actions (`more`).
                "chevron-down",
                tr("More tabs"),
                if open {
                    surfaces::IconTone::Ink
                } else {
                    surfaces::IconTone::Faded
                },
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.layers.close(Layer::PanelTabMenu) {
                    this.layers.open(Layer::PanelTabMenu);
                }
                cx.notify();
            })),
        );
        if open {
            more = more.child(deferred(self.panel_tab_menu(&overflow, &missing, cx)));
        }
        // The strip's width, measured, decides how many tabs it shows; a
        // change draws it again.
        let measured = self.panel_width.clone();
        let weak = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, window, cx| {
                let width = Some(f32::from(bounds.size.width));
                if measured.get() != width {
                    measured.set(width);
                    window.defer(cx, move |_, cx| {
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .relative()
            .h_flex()
            .items_center()
            .gap_1()
            .pl_1p5()
            .pr(px(surfaces::RIGHT_INSET))
            .h(px(surfaces::PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(measure)
            .child(tabs)
            .child(more)
    }

    /// Runs this conversation started: status, and their output on demand.
    /// Reads everything a run wrote, to show in place of its last lines.
    fn load_run_log(&mut self, run: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.run_log(project, run.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.run_logs.insert(run, result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_runs_tab(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let runs = self
            .state
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.run_cards.as_slice())
            .unwrap_or_default();
        let mut list = div()
            .id("runs")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            list = list.child(div().px_3().pt_3().child(error_banner(error, cx)));
        }
        if runs.is_empty() {
            list = list.child(surfaces::panel_empty(
                if self.state.session.is_some() {
                    tr("This conversation has not started any runs.")
                } else {
                    tr("Open a conversation to see its runs.")
                },
                cx,
            ));
        }
        // Each run is a record row, as in the conversation: its title, where
        // it ran, its state and time, ask aside and stop at its end. A
        // click opens it to its output and what else can be done with it.
        let mut rows = Vec::new();
        for run in runs {
            let id = run.id.clone();
            let expanded = self.expanded_run.as_ref() == Some(&id);
            let searching = run.kind == "method_search";
            let search_open = searching && self.open_search.as_ref() == Some(&id);
            let asking = self
                .run_cleaning
                .as_ref()
                .is_some_and(|(cleaning, _)| *cleaning == id)
                && self.layers.contains(Layer::ConfirmCleanRun);
            let mut row = self.run_record_row(&id, run, None, cx);
            if panel::can_cancel(&run.status) {
                let cancel = id.clone();
                row = row.action(
                    surfaces::icon_action(
                        SharedString::from(format!("run-cancel-{id}")),
                        "stop",
                        tr("Cancel"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.cancel_run(cancel.clone(), false, window, cx)
                    })),
                );
            } else if panel::can_force_cancel(run) {
                // Its stop never came back: finished here as cancelled.
                let cancel = id.clone();
                row = row.action(
                    surfaces::icon_action(
                        SharedString::from(format!("run-force-cancel-{id}")),
                        "ban",
                        tr("Mark it cancelled without waiting for the host to confirm the stop"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.cancel_run(cancel.clone(), true, window, cx)
                    })),
                );
            }
            // What it opens to: what else can be done with it, its progress
            // and clean-up, and its output.
            let mut below = div().v_flex().gap_2();
            let mut has_below = false;
            // A run moving files says how far it has got, open or not.
            if let Some(progress) = crate::transfers::progress(run) {
                below = below.child(super::transfers_view::meter(&progress, cx));
                has_below = true;
            }
            if asking && let Some((_, at_stake)) = &self.run_cleaning {
                below = below.child(self.clean_question(&id, *at_stake, cx));
                has_below = true;
            }
            if expanded || search_open {
                has_below = true;
                let mut tools = div().h_flex().flex_wrap().gap_1();
                if searching {
                    let wanted = id.clone();
                    tools = tools.child(
                        surfaces::labelled_action(
                            SharedString::from(format!("run-search-{id}")),
                            "search",
                            if search_open {
                                tr("Hide search")
                            } else {
                                tr("Search")
                            },
                            surfaces::IconSize::Inline,
                            if search_open {
                                surfaces::IconTone::Ink
                            } else {
                                surfaces::IconTone::Faded
                            },
                            cx,
                        )
                        .tooltip(tr("The search's contract, progress and candidates"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.toggle_search(wanted.clone(), window, cx)
                            },
                        )),
                    );
                }
                // What a run left on its SSH host can be looked through.
                if panel::can_clean(run) && run.kind == "ssh_direct" {
                    let (reviewed, title) = (id.clone(), run.title.clone());
                    tools = tools.child(
                        surfaces::labelled_action(
                            SharedString::from(format!("run-review-{id}")),
                            "folder",
                            tr("Look through what it left on its host"),
                            surfaces::IconSize::Inline,
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.open_run_review(reviewed.clone(), title.clone(), window, cx)
                            },
                        )),
                    );
                }
                if panel::can_clean(run) {
                    let clean = id.clone();
                    tools =
                        tools.child(
                            surfaces::danger_action(
                                SharedString::from(format!("run-clean-{id}")),
                                "trash",
                                tr("Delete what this run left where it ran"),
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.ask_clean_run(clean.clone(), cx),
                            )),
                        );
                }
                below = below.child(tools);
                if run.cleaned_at.is_some() {
                    below = below.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("Its workspace was cleaned.")),
                    );
                } else if let Some(problem) = &run.cleanup_error {
                    below = below.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(crate::i18n::host(problem)),
                    );
                }
                if search_open {
                    below = below.child(self.render_method_search(run, window, cx));
                }
            }
            if expanded {
                let whole = self.run_logs.get(&run.id);
                let shown = match whole {
                    Some(Ok(log)) => present::whole_log(log),
                    _ => present::run_output(run),
                };
                below = below.child(
                    surfaces::markdown(
                        SharedString::from(format!("run-output-{}", run.id)),
                        shown,
                        window,
                        cx,
                    )
                    .selectable(true),
                );
                if let Some(Err(problem)) = whole {
                    below = below.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(crate::i18n::host(problem)),
                    );
                }
                // The record keeps the last lines; the run's folder has all.
                if present::has_whole_log(run) {
                    let wanted = run.id.clone();
                    below = below.child(
                        div().h_flex().child(
                            Button::new(SharedString::from(format!("run-log-{}", run.id)))
                                .label(if matches!(whole, Some(Ok(_))) {
                                    tr("Read the log again")
                                } else {
                                    tr("Whole log")
                                })
                                .xsmall()
                                .ghost()
                                .tooltip(tr("Read everything the run wrote, from its folder"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.load_run_log(wanted.clone(), window, cx)
                                })),
                        ),
                    );
                }
            }
            let toggle = id.clone();
            let expanded = self.expanded_run.as_deref() == Some(id.as_str());
            rows.push(
                row.expandable(expanded)
                    .below(has_below.then(|| below.into_any_element()))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expanded_run = if this.expanded_run.as_ref() == Some(&toggle) {
                            None
                        } else {
                            Some(toggle.clone())
                        };
                        cx.notify();
                    })),
            );
        }
        if !rows.is_empty() {
            list = list.child(div().px_2().child(surfaces::record_group(rows, cx)));
        }
        list.into_any_element()
    }

    /// Files conversations produced. One that is a project file opens in the
    /// Files tab.
    /// Shows or hides an artifact's versions, reading them the first time.
    fn toggle_artifact_versions(
        &mut self,
        artifact: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.artifact_open.as_deref() == Some(artifact.as_str()) {
            self.artifact_open = None;
            cx.notify();
            return;
        }
        self.artifact_open = Some(artifact.clone());
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.artifact_versions(project, artifact.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(versions) => {
                        this.artifact_versions.insert(artifact, versions);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shows which tool call wrote an artifact, or hides it when it shows.
    fn toggle_artifact_origin(
        &mut self,
        artifact: String,
        session: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .artifact_origin
            .as_ref()
            .is_some_and(|(shown, _)| *shown == artifact)
        {
            self.artifact_origin = None;
            cx.notify();
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.artifact_origin = None;
        let reply = self.backend.file_origin(project, Some(session), path);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.artifact_origin = Some((artifact, result));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Where an artifact came from, under its row.
    fn artifact_origin_block(
        &self,
        found: &Result<Option<wisp_server::desktop::FileOrigin>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut block = div().v_flex().gap_1().pl_9().pr_3().pb_2();
        match found {
            Err(problem) => {
                return block.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(problem)),
                );
            }
            Ok(origin) => {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::files::origin_line(origin.as_ref())),
                );
                if let Some(origin) = origin {
                    let detail = crate::files::origin_detail(origin);
                    if !detail.is_empty() {
                        block = block.child(
                            div().max_h(px(200.)).overflow_hidden().child(
                                surfaces::markdown("artifact-origin-detail", detail, window, cx)
                                    .selectable(true),
                            ),
                        );
                    }
                    if let Some(project) = self.state.project.clone() {
                        let session = origin.session.clone();
                        block = block.child(
                            div().h_flex().child(
                                Button::new("artifact-origin-open")
                                    .label(tr("Open the conversation"))
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_session(
                                            project.clone(),
                                            session.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                        );
                    }
                }
            }
        }
        block
    }

    /// Asks where to save a copy of one version, under the artifact's own
    /// name, and saves it from its snapshot.
    fn save_artifact_copy(
        &mut self,
        version: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&name));
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(path) = place
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                receive(backend.save_artifact_version(project, version, path.clone())).await?;
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

    pub(super) fn render_artifacts_tab(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let scope =
            |id: &'static str, icon_name: &str, label: &'static str, all: bool| {
                surfaces::segment(id, icon_name, label, self.artifacts_all == all, cx)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_artifact_scope(all, window, cx)
                    }))
                    .into_any_element()
            };
        let count = (!self.artifacts.is_empty()).then(|| self.artifacts.len().to_string().into());
        let header = surfaces::panel_header(tr("Artifacts"), count, cx)
            .child(
                surfaces::icon_action(
                    "artifacts-by-folder",
                    "folder",
                    tr("By folder"),
                    if self.artifacts_by_folder {
                        surfaces::IconTone::On
                    } else {
                        surfaces::IconTone::Faded
                    },
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.artifacts_by_folder = !this.artifacts_by_folder;
                    cx.notify();
                })),
            )
            .child(
                surfaces::icon_action(
                    "artifacts-reload",
                    "refresh",
                    tr("Reload"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.load_artifacts(window, cx))),
            );
        // Whose files: the conversation's or the whole project's.
        let scopes =
            div()
                .h_flex()
                .gap_1()
                .px_3()
                .pt_2()
                .flex_shrink_0()
                .child(surfaces::segmented(
                    [
                        scope("artifacts-own", "chat", tr("This conversation"), false),
                        scope("artifacts-all", "notebook", tr("Whole project"), true),
                    ],
                    cx,
                ));
        let mut list = div()
            .id("artifacts")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.artifacts.is_empty() {
            list = list.child(surfaces::panel_empty(
                if self.artifacts_all {
                    tr("No conversation in this project has produced a file yet.")
                } else if self.state.session.is_some() {
                    tr("This conversation has not produced any files yet.")
                } else {
                    tr("Open a conversation, or look at the whole project.")
                },
                cx,
            ));
        }
        let shown = if self.artifacts_by_folder {
            panel::by_folder(&self.artifacts)
        } else {
            (0..self.artifacts.len())
                .map(|index| (None, index))
                .collect()
        };
        for (heading, index) in shown {
            let artifact = &self.artifacts[index];
            if let Some(heading) = heading {
                list = list.child(surfaces::panel_section(heading, cx).px_3().pb_1());
            }
            // A record row: the file's icon, its name, where and when after
            // it; its actions at the end. A click shows it in Files.
            let opened = self.artifact_open.as_deref() == Some(artifact.id.as_str())
                || self
                    .artifact_origin
                    .as_ref()
                    .is_some_and(|(shown, _)| *shown == artifact.id);
            let mut row = surfaces::record_row(
                SharedString::from(format!("artifact-{}", artifact.id)),
                artifact.name.clone(),
            )
            .icon("doc")
            .suffix(panel::artifact_detail(artifact, now, self.artifacts_all))
            .hover(move |style| style.bg(hover));
            if let Some(path) = artifact.path.clone() {
                row = row
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.panel_tab = PanelTab::Files;
                        this.reveal_file(path.clone(), window, cx)
                    }));
            }
            let tone = |on: bool| {
                if on {
                    surfaces::IconTone::On
                } else {
                    surfaces::IconTone::Faded
                }
            };
            // A figure can be kept with the favorites.
            if let Some(path) = artifact
                .path
                .clone()
                .filter(|path| crate::favorites::is_image(path))
            {
                let session = artifact.session_id.clone();
                row = row.action(
                    surfaces::icon_action(
                        ("artifact-star", index),
                        "star",
                        tr("Save this figure to favorites"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.star_artifact(session.clone(), path.clone(), window, cx)
                    })),
                );
            }
            // As the WebView's Open in center on an artifact's tile.
            if let Some(path) = artifact
                .path
                .clone()
                .filter(|_| self.state.session.is_some())
            {
                row = row.action(
                    surfaces::icon_action(
                        ("artifact-centre", index),
                        "external-link",
                        tr("Open in centre"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_in_centre(path.clone(), window, cx)
                    })),
                );
            }
            if let Some(path) = artifact.path.clone() {
                let (id, session) = (artifact.id.clone(), artifact.session_id.clone());
                let asked = self
                    .artifact_origin
                    .as_ref()
                    .is_some_and(|(shown, _)| *shown == artifact.id);
                row = row.action(
                    surfaces::icon_action(
                        ("artifact-origin", index),
                        "timeline",
                        tr("Which tool call wrote it"),
                        tone(asked),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.toggle_artifact_origin(
                            id.clone(),
                            session.clone(),
                            path.clone(),
                            window,
                            cx,
                        )
                    })),
                );
            }
            let id = artifact.id.clone();
            let open = self.artifact_open.as_deref() == Some(artifact.id.as_str());
            row = row.action(
                surfaces::icon_action(
                    ("artifact-versions", index),
                    "history",
                    tr("Its versions, to save a copy of one"),
                    tone(open),
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.toggle_artifact_versions(id.clone(), window, cx)
                })),
            );
            // What the row opened, under it.
            let mut details = div().v_flex().flex_shrink_0();
            if let Some((_, found)) = self
                .artifact_origin
                .as_ref()
                .filter(|(shown, _)| *shown == artifact.id)
            {
                details = details.child(self.artifact_origin_block(found, window, cx));
            }
            if open {
                match self.artifact_versions.get(&artifact.id) {
                    None => {
                        details = details.child(
                            div()
                                .pl_9()
                                .pb_2()
                                .text_xs()
                                .text_color(muted)
                                .child(tr("Reading…")),
                        );
                    }
                    Some(versions) => {
                        for (number, version) in versions.iter().enumerate().rev() {
                            let (version_id, name) = (version.id.clone(), artifact.name.clone());
                            details = details.child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .pl_9()
                                    .pr_3()
                                    .py_1()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(panel::version_line(version, now)),
                                    )
                                    .child(
                                        Button::new(("artifact-save", index * 1_000 + number))
                                            .icon(super::shell::glyph("download"))
                                            .label(tr("Save a copy…"))
                                            .xsmall()
                                            .ghost()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.save_artifact_copy(
                                                    version_id.clone(),
                                                    name.clone(),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    ),
                            );
                        }
                    }
                }
            }
            list = list.child(
                div().px_2().child(
                    row.expandable(opened)
                        .below(opened.then(|| details.into_any_element())),
                ),
            );
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(scopes)
            .child(list)
            .into_any_element()
    }
}
