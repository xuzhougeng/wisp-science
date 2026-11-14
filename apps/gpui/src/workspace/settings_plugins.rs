//! The Plugins page of Settings: packages of skills installed from a
//! folder and switched on for the open project.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::PluginRow;

use super::Workspace;
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    plugins,
};

impl Workspace {
    pub(super) fn load_plugins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.plugins(self.state.project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(rows) => this.plugins = Some(rows),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the plugins, then reads the list again.
    fn change_plugins<T: Send + 'static>(
        &mut self,
        reply: Reply<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.plugin_installing = false;
                this.layers.close(Layer::ConfirmRemovePlugin);
                this.plugin_removing = None;
                match result {
                    Ok(_) => this.load_plugins(window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn install_plugin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plugin_installing {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Install").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(picked))) = picker.await else {
                return;
            };
            let Some(folder) = picked.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.plugin_installing = true;
                let reply = this.backend.install_plugin(folder);
                this.change_plugins(reply, window, cx);
            });
        })
        .detach();
    }

    fn plugin_row(&self, row: &PluginRow, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let (id, version) = (row.id.clone(), row.version.clone());
        let asking = self.layers.contains(Layer::ConfirmRemovePlugin)
            && self.plugin_removing.as_deref() == Some(id.as_str());
        let mut about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(plugins::byline(row)),
            );
        if !row.description.is_empty() {
            about = about.child(div().text_xs().child(row.description.clone()));
        }
        about = about.child(
            div()
                .text_xs()
                .text_color(muted)
                .child(plugins::skills_line(row)),
        );
        if let Some(servers) = plugins::servers_line(row) {
            about = about.child(div().text_xs().text_color(muted).child(servers));
        }
        for problem in &row.problems {
            about = about.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("plugin-remove-yes-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let reply = this.backend.remove_plugin(id.clone(), version.clone());
                            this.change_plugins(reply, window, cx)
                        })),
                )
                .child(
                    Button::new("plugin-remove-no")
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemovePlugin);
                            this.plugin_removing = None;
                            cx.notify();
                        })),
                );
        } else {
            let enabled = row.enabled;
            let project = self.state.project.clone();
            let removing = id.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("plugin-toggle-{id}")))
                        .label(if enabled { tr("On") } else { tr("Off") })
                        .small()
                        .selected(enabled)
                        .disabled(project.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let Some(project) = project.clone() else {
                                return;
                            };
                            let reply = this.backend.set_plugin_enabled(
                                project,
                                id.clone(),
                                version.clone(),
                                !enabled,
                            );
                            this.change_plugins(reply, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("plugin-remove-{removing}")))
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.plugin_removing = Some(removing.clone());
                            this.layers.open(Layer::ConfirmRemovePlugin);
                            cx.notify();
                        })),
                );
        }
        div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(actions)
    }

    pub(super) fn render_plugin_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.as_str());
        let mut page = self
            .settings_page(cx)
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "A plugin is a folder of skills in the Claude plugin layout. Installing copies \
                 it into the app's data and runs nothing; a project that switches it on has \
                 its skills from the next turn.",
            )))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .child(
                        Button::new("plugin-install")
                            .label(if self.plugin_installing {
                                tr("Installing…")
                            } else {
                                tr("Install from a folder…")
                            })
                            .small()
                            .disabled(self.plugin_installing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.install_plugin(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(plugins::switch_note(project_name)),
                    ),
            );
        let Some(rows) = &self.plugins else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        if rows.is_empty() {
            page = page.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("No plugin is installed.")),
            );
        }
        for row in rows {
            page = page.child(self.plugin_row(row, cx));
        }
        page
    }
}
