//! The side panel's tabs beside Files: the conversation's runs, which can be
//! stopped from here, and the files conversations produced.
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
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
        self.files.close_preview();
        self.layers.open(Layer::FilesPanel);
        self.panel_tab = tab;
        match tab {
            PanelTab::Files => self.ensure_files(window, cx),
            PanelTab::Artifacts => self.load_artifacts(window, cx),
            PanelTab::Memos => self.load_memos(window, cx),
            PanelTab::Memory => self.load_memory(window, cx),
            // Cells show whether their code is saved.
            PanelTab::Highlights | PanelTab::Notebook => self.load_saved(window, cx),
            PanelTab::Runs => {}
        }
        cx.notify();
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

    /// The row of tabs at the top of the side panel.
    pub(super) fn panel_tabs(&self, cx: &mut Context<Self>) -> Div {
        let active = self
            .state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| panel::active_runs(&snapshot.run_cards));
        let mut tabs = div().h_flex().gap_1().flex_1().min_w_0();
        for tab in PanelTab::ALL {
            let selected = self.panel_tab == tab;
            // Only the open tab spells its name; the rest are icons with a
            // tooltip, so more tabs still fit a narrow panel. Runs that are
            // still going show their count either way.
            let label = match (tab, active, selected) {
                (PanelTab::Runs, 1.., true) => Some(format!("Runs · {active}")),
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
            .child(
                icon_button("panel-close", "close", "Close the panel")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_files(window, cx))),
            )
    }

    /// Runs this conversation started: status, and their output on demand.
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
                    "This conversation has not started any runs."
                } else {
                    "Open a conversation to see its runs."
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
                    .label(if expanded { "Hide output" } else { "Output" })
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
            if panel::can_cancel(&run.status) {
                let cancel = id.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("run-cancel-{id}")))
                        .icon(glyph("stop"))
                        .label("Cancel")
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
            if expanded {
                card = card.child(
                    TextView::markdown(
                        SharedString::from(format!("run-output-{}", run.id)),
                        present::run_output(run),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
            list = list.child(card);
        }
        list.into_any_element()
    }

    /// Files conversations produced. One that is a project file opens in the
    /// Files tab.
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
            .child(scope("artifacts-own", "This conversation", false))
            .child(scope("artifacts-all", "Whole project", true))
            .child(div().flex_1())
            .child(
                icon_button("artifacts-reload", "refresh", "Reload")
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
                    "No conversation in this project has produced a file yet."
                } else if self.state.session.is_some() {
                    "This conversation has not produced any files yet."
                } else {
                    "Open a conversation, or look at the whole project."
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
                        "Save this figure to favorites",
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.star_artifact(session.clone(), path.clone(), window, cx)
                    })),
                );
            }
            list = list.child(row);
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
