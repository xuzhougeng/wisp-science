//! Project settings: a dialog for a project's name, its description, and
//! the rules the model is given in it. It is a layer of its own on the
//! Escape stack, opened from the home page or from inside the project.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{ProjectSettings, RunRetention};

use super::{
    Workspace,
    shell::error_banner,
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, home, layers::Layer};

impl Workspace {
    /// Opens the dialog on `project` and fills it with what is saved.
    pub(super) fn open_project_settings(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_form = Some(project.clone());
        self.project_form_error = None;
        self.project_form_ready = false;
        self.layers.open(Layer::ProjectSettings);
        self.load_project_sync(project.clone(), window, cx);
        let reply = self.backend.project_settings(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Closed, or opened on another project, while this was read.
                if this.project_form.as_deref() != Some(project.as_str()) {
                    return;
                }
                match result {
                    Ok(saved) => {
                        let values = [
                            saved.name,
                            saved.description,
                            saved.rules,
                            home::days_text(saved.retention.succeeded),
                            home::days_text(saved.retention.failed),
                            home::days_text(saved.retention.orphaned),
                        ];
                        for (field, value) in this.project_fields.iter().zip(values) {
                            field.update(cx, |input, cx| input.set_value(value, window, cx));
                        }
                        this.project_form_ready = true;
                        this.project_fields[0].update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(error) => this.project_form_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_project_settings(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::ProjectSettings);
        self.project_form = None;
        self.project_form_error = None;
        cx.notify();
    }

    fn save_project_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project_form.clone() else {
            return;
        };
        if !self.project_form_ready {
            return;
        }
        let text = |index: usize| self.project_fields[index].read(cx).value().to_string();
        let days = |index: usize| home::days_typed(&text(index));
        let retention = match (days(3), days(4), days(5)) {
            (Ok(succeeded), Ok(failed), Ok(orphaned)) => RunRetention {
                succeeded,
                failed,
                orphaned,
            },
            (Err(error), ..) | (_, Err(error), _) | (.., Err(error)) => {
                self.project_form_error = Some(error);
                cx.notify();
                return;
            }
        };
        let settings = ProjectSettings {
            name: text(0),
            description: text(1),
            rules: text(2),
            retention,
        };
        self.project_form_error = None;
        let reply = self.backend.set_project_settings(project.clone(), settings);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        if this.project_form.as_deref() == Some(project.as_str()) {
                            this.close_project_settings(cx);
                        }
                        // The name is shown on the home page and in the sidebar.
                        this.load_projects(window, cx);
                        this.load_recent(window, cx);
                    }
                    // A refused form stays open with what was typed.
                    Err(error) => {
                        if this.project_form.as_deref() == Some(project.as_str()) {
                            this.project_form_error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The three cleanup windows side by side, each in days.
    fn render_cleanup_windows(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut row = div().h_flex().gap_3();
        for (index, name) in [
            (3, tr("Succeeded runs")),
            (4, tr("Failed runs")),
            (5, tr("Orphaned files")),
        ] {
            row = row.child(
                div()
                    .v_flex()
                    .flex_1()
                    .gap_1()
                    .child(div().text_xs().child(name))
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(Input::new(&self.project_fields[index]).small())
                            .child(div().text_xs().text_color(muted).child(tr("days"))),
                    ),
            );
        }
        row
    }

    pub(super) fn render_project_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str, note: &'static str| {
            div()
                .v_flex()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(text))
                .child(div().text_xs().text_color(muted).child(note))
        };
        let mut card = surfaces::dialog(
            "project-settings",
            DialogWidth::Regular,
            tr("Project settings"),
        )
        .gap_3()
        .close_on_click_outside(cx.listener(|this, _, _, cx| this.close_project_settings(cx)));
        if let Some(error) = &self.project_form_error {
            card = card.child(error_banner(error, cx));
        }
        card.child(label(
            tr("Name"),
            tr("What the project is called in lists."),
        ))
        .child(Input::new(&self.project_fields[0]).small())
        .child(label(
            tr("Description"),
            tr("For you: what the project is about. The model is not shown it."),
        ))
        .child(Input::new(&self.project_fields[1]).small())
        .child(label(
            tr("Rules for the model"),
            tr(
                "What Wisp should always do or never do in this project, in your own words. \
                 Kept in the project folder as .wisp/WISP.md and read again every turn.",
            ),
        ))
        .child(
            div()
                .h(px(220.))
                .child(Input::new(&self.project_fields[2]).h_full()),
        )
        .child(label(
            tr("Automatic server cleanup"),
            tr(
                "A succeeded run's folder on the server is removed this many days after its \
                 results were brought back, a failed or cancelled run's after the second, and \
                 files on a server that nothing refers to any more after the third. Leave a \
                 field empty to keep everything until you clean up yourself.",
            ),
        ))
        .child(self.render_cleanup_windows(cx))
        .child(self.render_project_sync(cx))
        .action(
            Button::new("project-settings-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.close_project_settings(cx))),
        )
        .action(
            Button::new("project-settings-save")
                .label(tr("Save"))
                .small()
                .primary()
                .disabled(!self.project_form_ready)
                .on_click(
                    cx.listener(|this, _, window, cx| this.save_project_settings(window, cx)),
                ),
        )
    }
}
