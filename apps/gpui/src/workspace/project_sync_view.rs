//! Project sync in the window: the Sync part of project settings (sync
//! now, the device code, choosing a side after a conflict, saving in a
//! cloud-drive folder), the dialog that joins a project from another
//! device, and where projects are synchronized, on the Remote access page.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{ProjectSyncResult, SyncSettings};

use super::{
    Workspace, chip,
    settings_sections::setting,
    surfaces::{self, DialogWidth},
    tone_color,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    present::Tone,
    sync,
};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

impl Workspace {
    pub(super) fn load_project_sync(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_sync = None;
        self.project_sync_note = None;
        self.project_sync_conflict = false;
        let reply = self.backend.project_sync(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.project_form.as_deref() != Some(project.as_str()) {
                    return;
                }
                match result {
                    Ok(info) => this.project_sync = Some(info),
                    Err(error) => this.project_sync_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs a sync of the project in the dialog and says how it went.
    fn run_sync(
        &mut self,
        reply: Reply<ProjectSyncResult>,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_sync_busy = true;
        self.project_sync_note = None;
        self.layers.close(Layer::ConfirmSyncReplace);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.project_sync_busy = false;
                match result {
                    Ok(done) => {
                        this.project_sync_conflict = false;
                        this.project_sync_note = Some(Ok(sync::result_note(&done)));
                        if done.direction == "pull" {
                            // The records changed under the open project.
                            this.load_sessions(window, cx);
                        }
                        this.load_projects(window, cx);
                    }
                    Err(error) => {
                        this.project_sync_conflict = sync::is_conflict(&error);
                        this.project_sync_note = Some(Err(error));
                    }
                }
                if this.project_form.as_deref() == Some(project.as_str()) {
                    this.load_project_sync_keeping_note(project, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_project_sync_keeping_note(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.project_sync(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.project_form.as_deref() == Some(project.as_str()) {
                    if let Ok(info) = result {
                        this.project_sync = Some(info);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_device_code(&mut self, project: String, window: &mut Window, cx: &mut Context<Self>) {
        self.project_sync_note = None;
        let reply = self.backend.project_sync_code(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(code) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(code));
                        this.project_sync_note = Some(Ok(tr(
                            "Device code copied. It holds the project's key: give it only to your own devices.",
                        )
                        .to_owned()));
                    }
                    Err(error) => this.project_sync_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The Sync part of project settings.
    pub(super) fn render_project_sync(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let Some(project) = self.project_form.clone() else {
            return div();
        };
        let mut block = div().v_flex().gap_2().child(
            div()
                .v_flex()
                .gap_1()
                .child(div().text_sm().child(tr("Sync")))
                .child(div().text_xs().text_color(muted).child(tr(
                    "Keeps this project's files and records the same on your devices, encrypted with a key only they have.",
                ))),
        );
        let Some(info) = &self.project_sync else {
            return block.child(div().text_xs().text_color(muted).child(tr("Loading…")));
        };
        let (line, tone) = sync::sync_line(info.as_ref(), now());
        block = block.child(div().text_sm().text_color(tone_color(tone, cx)).child(line));
        let busy = self.project_sync_busy;
        let mut actions = div().h_flex().gap_2().child(
            Button::new("project-sync-now")
                .label(if busy {
                    tr("Synchronizing…")
                } else {
                    tr("Sync now")
                })
                .small()
                .disabled(busy)
                .on_click(cx.listener({
                    let project = project.clone();
                    move |this, _, window, cx| {
                        let reply = this.backend.sync_project(project.clone());
                        this.run_sync(reply, project.clone(), window, cx)
                    }
                })),
        );
        let synced = info.as_ref().is_some_and(|info| info.synced);
        let workspace = info.as_ref().is_some_and(|info| info.kind == "workspace");
        if synced && !workspace {
            actions = actions.child(
                Button::new("project-sync-code")
                    .label(tr("Copy device code"))
                    .small()
                    .ghost()
                    .on_click(cx.listener({
                        let project = project.clone();
                        move |this, _, window, cx| {
                            this.copy_device_code(project.clone(), window, cx)
                        }
                    })),
            );
        }
        if info.is_none() {
            actions = actions.child(
                Button::new("project-sync-folder")
                    .label(tr("Save in its cloud-drive folder"))
                    .small()
                    .ghost()
                    .disabled(busy)
                    .on_click(cx.listener({
                        let project = project.clone();
                        move |this, _, window, cx| {
                            let reply = this.backend.enable_project_folder_sync(project.clone());
                            this.run_sync(reply, project.clone(), window, cx)
                        }
                    })),
            );
        }
        block = block.child(actions);
        if self.project_sync_conflict {
            let asking = self.layers.contains(Layer::ConfirmSyncReplace);
            block = block.child(if asking {
                let use_remote = self.project_sync_replace.unwrap_or(true);
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().text_xs().child(if use_remote {
                        tr("Replace this device's files and records with the other device's copy?")
                    } else {
                        tr("Publish this device's copy over the other device's changes?")
                    }))
                    .child(
                        Button::new("project-sync-replace-yes")
                            .label(if use_remote {
                                tr("Replace")
                            } else {
                                tr("Publish")
                            })
                            .small()
                            .danger()
                            .on_click(cx.listener({
                                let project = project.clone();
                                move |this, _, window, cx| {
                                    let reply = this
                                        .backend
                                        .resolve_project_sync(project.clone(), use_remote);
                                    this.run_sync(reply, project.clone(), window, cx)
                                }
                            })),
                    )
                    .child(
                        Button::new("project-sync-replace-no")
                            .label(tr("Keep"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmSyncReplace);
                                cx.notify();
                            })),
                    )
            } else {
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("project-sync-use-remote")
                            .label(tr("Use the other device's copy"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_sync_replace = Some(true);
                                this.layers.open(Layer::ConfirmSyncReplace);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("project-sync-use-local")
                            .label(tr("Keep this device's copy"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_sync_replace = Some(false);
                                this.layers.open(Layer::ConfirmSyncReplace);
                                cx.notify();
                            })),
                    )
            });
        }
        if let Some(note) = &self.project_sync_note {
            block = block.child(match note {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        block
    }

    pub(super) fn open_join_synced(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.join_note = None;
        self.join_code
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.layers.open(Layer::JoinSyncedProject);
        self.join_code
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Asks where to put the project, then downloads it there.
    fn join_synced(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = self.join_code.read(cx).value().trim().to_owned();
        if code.is_empty() || self.join_busy {
            return;
        }
        self.join_busy = true;
        self.join_note = None;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Put the project in this folder").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match paths.and_then(|mut paths| paths.pop()) {
                    Some(parent) => receive(backend.join_synced_project(code, parent))
                        .await
                        .map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.join_busy = false;
                match result {
                    Ok(Some(project)) => {
                        this.layers.close(Layer::JoinSyncedProject);
                        this.load_projects(window, cx);
                        this.enter_project(project, window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.join_note = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_join_synced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ready = !self.join_code.read(cx).value().trim().is_empty() && !self.join_busy;
        let mut card = surfaces::dialog(
            "join-synced",
            DialogWidth::Narrow,
            tr("Join a project from another device"),
        )
        .gap_3()
        .close_on_click_outside(cx.listener(|this, _, _, cx| {
            this.layers.close(Layer::JoinSyncedProject);
            cx.notify();
        }))
        .child(div().text_xs().text_color(muted).child(tr(
                "Paste the device code copied in the project's settings on the other device, then choose the folder to put the project in.",
            )))
            .child(super::surfaces::input(&self.join_code, cx).small());
        if let Some(error) = &self.join_note {
            card = card.child(chip(crate::i18n::host(error), Tone::Danger, cx));
        }
        card.action(
            Button::new("join-synced-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::JoinSyncedProject);
                    cx.notify();
                })),
        )
        .action(
            Button::new("join-synced-go")
                .label(if self.join_busy {
                    tr("Downloading…")
                } else {
                    tr("Choose a folder and join")
                })
                .small()
                .primary()
                .disabled(!ready)
                .on_click(cx.listener(|this, _, window, cx| this.join_synced(window, cx))),
        )
    }

    pub(super) fn load_sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.sync_settings();
        self.receive_sync_settings(reply, window, cx);
    }

    fn receive_sync_settings(
        &mut self,
        reply: Reply<SyncSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(settings) => {
                        let values = [
                            settings.relay_url.clone(),
                            String::new(),
                            settings.folder.clone(),
                        ];
                        for (field, value) in this.sync_fields.iter().zip(values) {
                            field.update(cx, |input, cx| input.set_value(value, window, cx));
                        }
                        this.sync_backend = settings.backend.clone();
                        this.sync_settings = Some(settings);
                    }
                    Err(error) => this.sync_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_note = None;
        let [relay, token, folder] = self
            .sync_fields
            .each_ref()
            .map(|field| field.read(cx).value().trim().to_string());
        let reply = self.backend.set_sync_settings(
            self.sync_backend.clone(),
            relay,
            (!token.is_empty()).then_some(token),
            folder,
        );
        self.receive_sync_settings(reply, window, cx);
        self.sync_note = Some(Ok(tr("Saved.").to_owned()));
    }

    /// Where projects are synchronized, on the Remote access page.
    pub(super) fn render_sync_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut card = div()
            .v_flex()
            .gap_2()
            .pt_6()
            .child(div().text_sm().child(tr("Project sync")))
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "Where Sync now in a project's settings publishes it: a wisp-relay of your own, or a shared folder such as a network drive. Files and records are encrypted with a key per project before they leave.",
            )));
        let Some(settings) = &self.sync_settings else {
            return card.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let relay = self.sync_backend != "folder";
        card = card.child(setting(
            tr("Synchronize through"),
            tr("Projects already synchronized keep where they were set up."),
            div()
                .h_flex()
                .gap_1()
                .child(
                    Button::new("sync-backend-relay")
                        .label(tr("Relay"))
                        .small()
                        .selected(relay)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sync_backend = "relay".into();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("sync-backend-folder")
                        .label(tr("Shared folder"))
                        .small()
                        .selected(!relay)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sync_backend = "folder".into();
                            cx.notify();
                        })),
                ),
            cx,
        ));
        card = if relay {
            card.child(setting(
                tr("Relay server URL"),
                tr("An https address; http is taken only on a private network."),
                div()
                    .w(px(320.))
                    .child(super::surfaces::input(&self.sync_fields[0], cx).small()),
                cx,
            ))
            .child(setting(
                tr("Relay access token"),
                if settings.has_token {
                    tr("A token is kept; type a new one to replace it.")
                } else {
                    tr("Given by the relay's operator; kept in the system keychain.")
                },
                div()
                    .w(px(220.))
                    .child(super::surfaces::input(&self.sync_fields[1], cx).small()),
                cx,
            ))
        } else {
            card.child(setting(
                tr("Shared folder"),
                tr("An absolute path every device can reach; it must not be inside a project."),
                div()
                    .w(px(320.))
                    .child(super::surfaces::input(&self.sync_fields[2], cx).small()),
                cx,
            ))
        };
        card = card.child(
            div().h_flex().child(
                Button::new("sync-settings-save")
                    .label(tr("Save"))
                    .small()
                    .primary()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.save_sync_settings(window, cx)),
                    ),
            ),
        );
        if let Some(note) = &self.sync_note {
            card = card.child(match note {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        card
    }
}
