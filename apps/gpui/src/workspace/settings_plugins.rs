//! The Plugins page of Settings: packages of skills installed from a
//! folder and switched on for the open project.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::PluginRow;

use super::{
    Workspace,
    settings_sections::setting,
    shell::glyph,
    surfaces::{self, RecordRow, switch},
};
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

    /// Installs from a folder, or from a `.zip` when `archive`.
    fn install_plugin(&mut self, archive: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.plugin_installing {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: archive,
            directories: !archive,
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

    fn install_plugin_from_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plugin_installing {
            return;
        }
        let [address, sha256] =
            [0, 1].map(|at| self.plugin_address[at].read(cx).value().to_string());
        self.plugin_installing = true;
        let reply = self.backend.install_plugin_from_address(address, sha256);
        self.layers.close(Layer::PluginAddress);
        self.change_plugins(reply, window, cx);
    }

    /// The form for a plugin to download: its address and published
    /// checksum, as setting rows.
    fn plugin_address_form(&self, cx: &mut Context<Self>) -> Div {
        div()
            .v_flex()
            .child(surfaces::settings_note_help(tr("The HTTPS address of a .zip and its SHA-256."),
                tr(
                    "The address of the plugin's .zip, over HTTPS, and the SHA-256 its publisher gives. \
                     It is installed only when the download has that checksum.",
                ),
                cx,
            ))
            .child(setting(
                tr("Address"),
                "",
                div()
                    .w(px(300.))
                    .child(surfaces::input(&self.plugin_address[0], cx).small()),
                cx,
            ))
            .child(setting(
                // The name of a checksum, the same in either language.
                "SHA-256",
                "",
                div()
                    .w(px(300.))
                    .child(surfaces::input(&self.plugin_address[1], cx).small()),
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        Button::new("plugin-address-install")
                            .icon(glyph("download"))
                            .label(tr("Download and install"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.install_plugin_from_address(window, cx)
                            })),
                    )
                    .child(
                        Button::new("plugin-address-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::PluginAddress);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// One plugin as a record row: its name and byline; what it brings
    /// under it; a switch for the open project and Remove last in the danger
    /// colour.
    fn plugin_row(&self, row: &PluginRow, cx: &mut Context<Self>) -> RecordRow {
        let muted = cx.theme().muted_foreground;
        let (id, version) = (row.id.clone(), row.version.clone());
        let asking = self.layers.contains(Layer::ConfirmRemovePlugin)
            && self.plugin_removing.as_deref() == Some(id.as_str());
        let mut about = div().v_flex().gap_1();
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
                    .child(crate::i18n::host(problem)),
            );
        }
        let mut record =
            surfaces::record_row(SharedString::from(format!("plugin-{id}")), row.name.clone())
                .icon("grid")
                .suffix(plugins::byline(row));
        if !row.problems.is_empty() {
            record = record.stamp(surfaces::StampKind::Failed, tr("Failed"));
        }
        if asking {
            about = about.child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new(SharedString::from(format!("plugin-remove-yes-{id}")))
                            .icon(glyph("trash"))
                            .label(tr("Remove"))
                            .xsmall()
                            .danger()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let reply = this.backend.remove_plugin(id.clone(), version.clone());
                                this.change_plugins(reply, window, cx)
                            })),
                    )
                    .child(
                        Button::new("plugin-remove-no")
                            .label(tr("Keep"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemovePlugin);
                                this.plugin_removing = None;
                                cx.notify();
                            })),
                    ),
            );
            return record.below(Some(about.into_any_element()));
        }
        let enabled = row.enabled;
        let project = self.state.project.clone();
        let removing = id.clone();
        record
            .below(Some(about.into_any_element()))
            .action(
                switch(
                    SharedString::from(format!("plugin-toggle-{id}")),
                    enabled,
                    tr("Use this plugin in this project"),
                )
                .disabled(project.is_none())
                .on_change(cx.listener(move |this, _, window, cx| {
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
            .action(
                surfaces::danger_action(
                    SharedString::from(format!("plugin-remove-{removing}")),
                    "trash",
                    tr("Remove"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.plugin_removing = Some(removing.clone());
                    this.layers.open(Layer::ConfirmRemovePlugin);
                    cx.notify();
                })),
            )
    }

    pub(super) fn render_plugin_settings(&self, cx: &mut Context<Self>) -> Div {
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.as_str());
        let by_address = self.layers.contains(Layer::PluginAddress);
        // Installing from a folder is the way most used; a .zip and an
        // address are icon buttons after it.
        let mut page = self
            .settings_page(cx)
            .child(surfaces::settings_note_help(
                tr("Folders of skills in the Claude plugin layout."),
                tr(
                    "A plugin is a folder of skills in the Claude plugin layout. Installing copies \
                     it into the app's data and runs nothing; a project that switches it on has \
                     its skills from the next turn.",
                ),
                cx,
            ))
            .child(surfaces::settings_note(plugins::switch_note(project_name), cx).pt_1())
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .py_3()
                    .child(
                        surfaces::outlined_action(
                            "plugin-install",
                            "folder",
                            if self.plugin_installing {
                                tr("Installing…")
                            } else {
                                tr("Install from a folder…")
                            },
                            cx,
                        )
                        .disabled(self.plugin_installing)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.install_plugin(false, window, cx)
                            }),
                        ),
                    )
                    .child(
                        surfaces::icon_action(
                            "plugin-install-zip",
                            "archive-import",
                            tr("From a .zip…"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(self.plugin_installing)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.install_plugin(true, window, cx)
                            }),
                        ),
                    )
                    .child(
                        surfaces::icon_action(
                            "plugin-install-address",
                            "link",
                            tr("From an address…"),
                            if by_address {
                                surfaces::IconTone::On
                            } else {
                                surfaces::IconTone::Faded
                            },
                            cx,
                        )
                        .disabled(self.plugin_installing)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.layers.close(Layer::PluginAddress) {
                                this.layers.open(Layer::PluginAddress);
                                this.plugin_address[0]
                                    .update(cx, |input, cx| input.focus(window, cx));
                            }
                            cx.notify();
                        })),
                    ),
            );
        if by_address {
            page = page.child(div().pb_3().child(self.plugin_address_form(cx)));
        }
        let Some(rows) = &self.plugins else {
            return page.child(surfaces::settings_note(tr("Loading…"), cx));
        };
        if rows.is_empty() {
            return page.child(surfaces::settings_note(tr("No plugin is installed."), cx));
        }
        let rows: Vec<_> = rows.iter().map(|row| self.plugin_row(row, cx)).collect();
        page.child(surfaces::record_group(rows, cx))
    }
}
