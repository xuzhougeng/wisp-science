//! The side panel's tabs beside Files: the conversation's runs, which can be
//! stopped from here, and the files conversations produced.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    text::TextView,
};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    assets::icon,
    backend::receive,
    layers::Layer,
    method_search,
    panel::{self, PanelTab},
    present::{self, Tone},
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
        self.files.close_preview();
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

    /// The tabs that were taken away, to add one back: a menu under the
    /// row's add button.
    fn panel_tab_menu(&self, missing: &[PanelTab], cx: &mut Context<Self>) -> Div {
        let mut menu = div()
            .absolute()
            .top(px(30.))
            .right_0()
            .w(px(200.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md();
        for tab in missing.iter().copied() {
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("panel-add-{}", tab.key())),
                    tab.label(),
                    "",
                    false,
                    cx,
                )
                .child(icon(tab.icon()).text_color(cx.theme().muted_foreground))
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

    fn cancel_run(&mut self, run: String, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        self.error = None;
        let reply = self.backend.cancel_run(project, run);
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
        let heading = |text: String| div().text_xs().font_semibold().child(text);

        // What the researcher approves by starting it.
        card = card
            .child(heading(tr("Contract").into()))
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
        card =
            card.child(heading(tr("Progress").into()))
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
            card = card.child(heading(tr("Outputs").into())).child(files);
            if let Some((version, text)) = &self.search_output {
                let path = details
                    .outputs
                    .iter()
                    .find(|output| output.artifact_version_id == *version)
                    .map(|output| output.source_path.as_str())
                    .unwrap_or_default();
                card = card.child(match text {
                    Ok(text) => TextView::markdown(
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
            card = card.child(heading(crate::i18n::fill(
                tr("Candidates ({})"),
                &[&details.candidates.len()],
            )));
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
            card = card.child(heading(tr("Strategies").into()));
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
    pub(super) fn panel_tabs(&self, cx: &mut Context<Self>) -> Div {
        let active = self
            .state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| panel::active_runs(&snapshot.run_cards));
        let mut tabs = div().h_flex().gap_1().flex_1().min_w_0().overflow_hidden();
        for tab in panel::visible(&self.kept_tabs, self.panel_tab) {
            let selected = self.panel_tab == tab;
            // Only the open tab spells its name; the rest are icons with a
            // tooltip, so more tabs still fit a narrow panel. Runs that are
            // still going show their count either way.
            let label = match (tab, active, selected) {
                (PanelTab::Runs, 1.., true) => Some(crate::i18n::fill(tr("Runs · {}"), &[&active])),
                (PanelTab::Runs, 1.., false) => Some(active.to_string()),
                (_, _, true) => Some(tab.label().to_owned()),
                _ => None,
            };
            let mut button = Button::new(SharedString::from(format!("panel-tab-{}", tab.label())))
                .icon(glyph(tab.icon()))
                .small()
                .ghost()
                .selected(selected)
                .tooltip(tab.label())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.show_panel_tab(tab, window, cx)),
                );
            if let Some(label) = label {
                button = button.label(label);
            }
            tabs = tabs.child(button);
            // The open tab can be taken out of the row.
            if selected {
                tabs = tabs.child(
                    icon_button(
                        SharedString::from(format!("panel-take-away-{}", tab.key())),
                        "minus",
                        tr("Take this tab away"),
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.take_away_panel_tab(tab, window, cx)
                    })),
                );
            }
        }
        let missing = panel::missing(&self.kept_tabs, self.panel_tab);
        let adding = self.layers.contains(Layer::PanelTabMenu);
        let mut add = div().relative().flex_shrink_0();
        if !missing.is_empty() {
            add = add.child(
                icon_button("panel-add-tab", "plus", tr("Add a tab"))
                    .selected(adding)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.layers.close(Layer::PanelTabMenu) {
                            this.layers.open(Layer::PanelTabMenu);
                        }
                        cx.notify();
                    })),
            );
            if adding {
                add = add.child(deferred(self.panel_tab_menu(&missing, cx)));
            }
        }
        div()
            .h_flex()
            .gap_2()
            .px_2()
            .h(px(44.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(tabs)
            .child(add)
            .child(
                icon_button("panel-close", "close", tr("Close the panel"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_files(window, cx))),
            )
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
            list = list.child(div().p_3().child(error_banner(error, cx)));
        }
        if runs.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(muted).child(
                if self.state.session.is_some() {
                    tr("This conversation has not started any runs.")
                } else {
                    tr("Open a conversation to see its runs.")
                },
            ));
        }
        for run in runs {
            let id = run.id.clone();
            let expanded = self.expanded_run.as_ref() == Some(&id);
            let (status, tone) = present::run_status(run);
            let toggle = id.clone();
            let mut actions = div().h_flex().gap_1().child(
                Button::new(SharedString::from(format!("run-{id}")))
                    .label(if expanded {
                        tr("Hide output")
                    } else {
                        tr("Output")
                    })
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expanded_run = if this.expanded_run.as_ref() == Some(&toggle) {
                            None
                        } else {
                            Some(toggle.clone())
                        };
                        cx.notify();
                    })),
            );
            let searching = run.kind == "method_search";
            if searching {
                let open = self.open_search.as_ref() == Some(&id);
                let wanted = id.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("run-search-{id}")))
                        .label(if open {
                            tr("Hide search")
                        } else {
                            tr("Search")
                        })
                        .xsmall()
                        .ghost()
                        .selected(open)
                        .tooltip(tr("The search's contract, progress and candidates"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_search(wanted.clone(), window, cx)
                        })),
                );
            }
            {
                let (asked, title) = (id.clone(), run.title.clone());
                actions = actions.child(
                    Button::new(SharedString::from(format!("run-ask-{id}")))
                        .label(tr("Ask"))
                        .xsmall()
                        .ghost()
                        .tooltip(tr("Ask aside about this run, with its live state"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_aside_about_run(asked.clone(), title.clone(), window, cx)
                        })),
                );
            }
            if panel::can_cancel(&run.status) {
                let cancel = id.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("run-cancel-{id}")))
                        .icon(glyph("stop"))
                        .label(tr("Cancel"))
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.cancel_run(cancel.clone(), window, cx)
                        })),
                );
            }
            let mut card = div()
                .v_flex()
                .gap_1()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().text_sm().child(run.title.clone()))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(chip(run.kind.clone(), Tone::Neutral, cx))
                        .child(chip(status, tone, cx))
                        .child(div().flex_1())
                        .child(actions),
                );
            if searching && self.open_search.as_ref() == Some(&id) {
                card = card.child(self.render_method_search(run, window, cx));
            }
            if expanded {
                let whole = self.run_logs.get(&run.id);
                let shown = match whole {
                    Some(Ok(log)) => present::whole_log(log),
                    _ => present::run_output(run),
                };
                card = card.child(
                    TextView::markdown(
                        SharedString::from(format!("run-output-{}", run.id)),
                        shown,
                        window,
                        cx,
                    )
                    .selectable(true),
                );
                if let Some(Err(problem)) = whole {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(crate::i18n::host(problem)),
                    );
                }
                // The record keeps the last lines; the run's folder has all.
                if present::has_whole_log(run) {
                    let wanted = run.id.clone();
                    card = card.child(
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
            list = list.child(card);
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

    pub(super) fn render_artifacts_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let scope =
            |id: &'static str, label: &'static str, all: bool| {
                Button::new(id)
                    .label(label)
                    .xsmall()
                    .ghost()
                    .selected(self.artifacts_all == all)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_artifact_scope(all, window, cx)
                    }))
            };
        let header = div()
            .h_flex()
            .gap_1()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .child(scope("artifacts-own", tr("This conversation"), false))
            .child(scope("artifacts-all", tr("Whole project"), true))
            .child(div().flex_1())
            .child(
                icon_button("artifacts-reload", "refresh", tr("Reload"))
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| this.load_artifacts(window, cx))),
            );
        let mut list = div()
            .id("artifacts")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.artifacts.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(muted).child(
                if self.artifacts_all {
                    tr("No conversation in this project has produced a file yet.")
                } else if self.state.session.is_some() {
                    tr("This conversation has not produced any files yet.")
                } else {
                    tr("Open a conversation, or look at the whole project.")
                },
            ));
        }
        for (index, artifact) in self.artifacts.iter().enumerate() {
            let mut name =
                div()
                    .id(SharedString::from(format!("artifact-{}", artifact.id)))
                    .h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(icon("doc").text_color(muted))
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_sm().child(artifact.name.clone()))
                            .child(
                                div().truncate().text_xs().text_color(muted).child(
                                    panel::artifact_detail(artifact, now, self.artifacts_all),
                                ),
                            ),
                    );
            if let Some(path) = artifact.path.clone() {
                name = name
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.panel_tab = PanelTab::Files;
                        this.reveal_file(path.clone(), window, cx)
                    }));
            }
            let mut row = div()
                .h_flex()
                .gap_1()
                .px_3()
                .py_2()
                .hover(|style| style.bg(hover))
                .child(name);
            // The star is beside the name, not inside it, so pressing it
            // does not also open the file.
            if let Some(path) = artifact
                .path
                .clone()
                .filter(|path| crate::favorites::is_image(path))
            {
                let session = artifact.session_id.clone();
                row = row.child(
                    icon_button(
                        ("artifact-star", index),
                        "star",
                        tr("Save this figure to favorites"),
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.star_artifact(session.clone(), path.clone(), window, cx)
                    })),
                );
            }
            let id = artifact.id.clone();
            let open = self.artifact_open.as_deref() == Some(artifact.id.as_str());
            row = row.child(
                icon_button(
                    ("artifact-versions", index),
                    "history",
                    tr("Its versions, to save a copy of one"),
                )
                .xsmall()
                .selected(open)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_artifact_versions(id.clone(), window, cx)
                })),
            );
            list = list.child(row);
            if open {
                match self.artifact_versions.get(&artifact.id) {
                    None => {
                        list = list.child(
                            div()
                                .px_6()
                                .pb_2()
                                .text_xs()
                                .text_color(muted)
                                .child(tr("Reading…")),
                        );
                    }
                    Some(versions) => {
                        for (number, version) in versions.iter().enumerate().rev() {
                            let (version_id, name) = (version.id.clone(), artifact.name.clone());
                            list = list.child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .pl(px(36.))
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
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(list)
            .into_any_element()
    }
}
