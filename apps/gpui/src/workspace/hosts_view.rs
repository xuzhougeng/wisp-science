//! The Environments tab of the side panel: this computer and the saved SSH
//! hosts, which of them the conversation's work may run on, and for each a
//! check, a terminal, its folders, the interpreters its last check found,
//! the project's runs there and the files the project placed there. A run
//! on a host that is not attached is refused, so attaching one is what
//! lets the model use it.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button},
    surfaces::{self, DialogWidth},
};
use crate::{
    backend::receive,
    layers::Layer,
    panel::PanelTab,
    present::Tone,
    settings::{self, Section},
};
use wisp_server::desktop::{HostRun, StagedFile, StagedState};

/// How many of a host's runs its card lists.
const HOST_RUNS_SHOWN: usize = 20;

/// What one host's card shows under its actions, as the WebView's Runs and
/// Remote files views of a server: nothing until it is read.
pub(super) enum HostDetail {
    Runs {
        alias: String,
        runs: Option<Result<Vec<HostRun>, String>>,
    },
    Placed {
        alias: String,
        files: Option<Result<Vec<StagedFile>, String>>,
        /// A file is being deleted.
        busy: bool,
    },
}

impl HostDetail {
    fn alias(&self) -> &str {
        match self {
            Self::Runs { alias, .. } | Self::Placed { alias, .. } => alias,
        }
    }
}

impl Workspace {
    /// Shows a host's runs, or the files the project placed there, under
    /// its card; the same again closes it.
    fn toggle_host_detail(
        &mut self,
        alias: String,
        runs: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.host_detail.as_ref().is_some_and(|detail| {
            detail.alias() == alias && matches!(detail, HostDetail::Runs { .. }) == runs
        });
        self.host_detail = None;
        self.staged_removing = None;
        self.layers.close(Layer::HostDetail);
        if open {
            cx.notify();
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        // One thing under a card at a time.
        self.storage_host = None;
        self.layers.close(Layer::HostStorageForm);
        self.host_detail = Some(if runs {
            HostDetail::Runs {
                alias: alias.clone(),
                runs: None,
            }
        } else {
            HostDetail::Placed {
                alias: alias.clone(),
                files: None,
                busy: false,
            }
        });
        self.layers.open(Layer::HostDetail);
        if runs {
            let reply = self.backend.host_runs(project, alias.clone());
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(HostDetail::Runs { alias: shown, runs }) = &mut this.host_detail
                        && *shown == alias
                    {
                        *runs = Some(result);
                        cx.notify();
                    }
                });
            })
            .detach();
        } else {
            let reply = self.backend.host_staged_files(project, alias.clone());
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    this.show_placed(&alias, result);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Takes the files placed on `alias` that were read, when that host's
    /// list is still the one open.
    fn show_placed(&mut self, alias: &str, result: Result<Vec<StagedFile>, String>) {
        if let Some(HostDetail::Placed {
            alias: shown,
            files,
            busy,
        }) = &mut self.host_detail
            && shown == alias
        {
            *files = Some(result);
            *busy = false;
        }
    }

    /// Asks before a file the project placed on a host is deleted.
    fn ask_remove_staged(&mut self, file: StagedFile, cx: &mut Context<Self>) {
        if !matches!(
            self.host_detail,
            Some(HostDetail::Placed { busy: false, .. })
        ) {
            return;
        }
        self.staged_removing = Some(file);
        self.layers.open(Layer::ConfirmRemoveStaged);
        cx.notify();
    }

    /// Deletes the file asked about from its host, as the WebView's Delete
    /// from server does, and reads the list again.
    fn remove_staged(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(file)) = (self.state.project.clone(), self.staged_removing.take())
        else {
            return;
        };
        self.layers.close(Layer::ConfirmRemoveStaged);
        let Some(HostDetail::Placed { alias, busy, .. }) = &mut self.host_detail else {
            return;
        };
        if *busy {
            return;
        }
        *busy = true;
        let alias = alias.clone();
        let deleted = file.state != StagedState::Replaced;
        let reply =
            self.backend
                .remove_host_staged_file(project.clone(), alias.clone(), file.id.clone());
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            // Whatever went, the list says what is left.
            let (said, listed) = match result {
                Ok(left) => (Ok(()), Ok(left)),
                Err(error) => (
                    Err(error),
                    receive(backend.host_staged_files(project, alias.clone())).await,
                ),
            };
            let _ = this.update(cx, |this, cx| {
                match said {
                    Ok(()) => {
                        this.notice = Some(if deleted {
                            crate::i18n::fill(
                                tr("{} was deleted from {}."),
                                &[&file.remote_path, &alias],
                            )
                        } else {
                            tr("The record of the replaced copy is closed.").into()
                        })
                    }
                    Err(error) => this.error = Some(error),
                }
                this.show_placed(&alias, listed);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The question before a placed file is deleted: the host, the whole
    /// path, that a folder goes with everything in it, and that it cannot
    /// be undone, with the deletion last in the danger tone.
    pub(super) fn render_remove_staged(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let file = self
            .staged_removing
            .as_ref()
            .filter(|_| self.layers.contains(Layer::ConfirmRemoveStaged))?;
        let alias = self.host_detail.as_ref()?.alias().to_owned();
        let replaced = file.state == StagedState::Replaced;
        Some(
            surfaces::dialog(
                "remove-staged",
                DialogWidth::Narrow,
                if replaced {
                    tr("Close the record of this copy?")
                } else {
                    tr("Delete this file from the host?")
                },
            )
            .child(
                div()
                    .text_sm()
                    .child(crate::settings::staged_question(&alias, file)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::fill(
                        tr("On {}: {}"),
                        &[&alias, &file.remote_path],
                    )),
            )
            .action(
                Button::new("remove-staged-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.staged_removing = None;
                        this.layers.close(Layer::ConfirmRemoveStaged);
                        cx.notify();
                    })),
            )
            .action(
                Button::new("remove-staged-confirm")
                    .label(if replaced {
                        tr("Close the record")
                    } else {
                        tr("Delete from the host")
                    })
                    .small()
                    .danger()
                    .on_click(cx.listener(|this, _, window, cx| this.remove_staged(window, cx))),
            ),
        )
    }

    /// What is open under `alias`'s card: its runs, or the files the
    /// project placed there.
    fn host_detail_card(
        &self,
        alias: &str,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let detail = self
            .host_detail
            .as_ref()
            .filter(|detail| detail.alias() == alias && self.layers.contains(Layer::HostDetail))?;
        let muted = cx.theme().muted_foreground;
        let line = |text: String| div().text_xs().text_color(muted).child(text);
        let mut card = surfaces::panel_card("host-detail", cx);
        match detail {
            HostDetail::Runs { runs, .. } => {
                card = card.child(line(
                    tr("The project's runs on this host, newest first.").into(),
                ));
                match runs {
                    None => card = card.child(line(tr("Loading…").into())),
                    Some(Err(error)) => {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(crate::i18n::host(error)),
                        )
                    }
                    Some(Ok(runs)) if runs.is_empty() => {
                        card = card.child(line(tr("No runs recorded yet.").into()))
                    }
                    Some(Ok(runs)) => {
                        for run in runs.iter().take(HOST_RUNS_SHOWN) {
                            let (status, tone) =
                                crate::present::run_state(&run.status, run.exit_code);
                            card = card.child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .child(surfaces::panel_label(run.title.clone()).text_sm())
                                    .child(chip(status, tone, cx))
                                    .child(line(crate::home::age(now, run.created_at))),
                            );
                        }
                        if runs.len() > HOST_RUNS_SHOWN {
                            card = card.child(line(crate::i18n::fill(
                                tr("{} older runs are not listed."),
                                &[&(runs.len() - HOST_RUNS_SHOWN)],
                            )));
                        }
                    }
                }
            }
            HostDetail::Placed { files, busy, .. } => {
                card = card.child(line(
                    tr("Files the project placed on this host. One no run needs can be deleted from it.")
                        .into(),
                ));
                match files {
                    None => card = card.child(line(tr("Loading…").into())),
                    Some(Err(error)) => {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(crate::i18n::host(error)),
                        )
                    }
                    Some(Ok(files)) if files.is_empty() => {
                        card = card.child(line(
                            tr("No files from this project are recorded on this host.").into(),
                        ))
                    }
                    Some(Ok(files)) => {
                        for (index, file) in files.iter().enumerate() {
                            let mut about =
                                vec![crate::settings::staged_state(file.state).to_owned()];
                            let source = crate::settings::staged_source(&file.source);
                            if !source.is_empty() {
                                about.push(source.to_owned());
                            }
                            if let Some(size) = file.size_bytes.filter(|size| *size >= 0) {
                                about.push(crate::files::size_label(size as u64));
                            }
                            if let Some(status) = &file.run_status {
                                about.push(crate::present::run_state(status, None).0);
                            }
                            let mut row = div().h_flex().items_start().gap_2().child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .child(file.remote_path.clone()),
                                    )
                                    .child(line(about.join(" · "))),
                            );
                            if file.state != StagedState::Active {
                                let asked = file.clone();
                                row = row.child(
                                    icon_button(
                                        ("staged-remove", index),
                                        "trash",
                                        "Delete from the host…",
                                    )
                                    .xsmall()
                                    .disabled(*busy)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.ask_remove_staged(asked.clone(), cx)
                                        },
                                    )),
                                );
                            }
                            card = card.child(row);
                        }
                    }
                }
            }
        }
        Some(card)
    }

    /// Reads which hosts the open conversation may run on.
    pub(super) fn load_conversation_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            self.conversation_hosts = None;
            return;
        };
        let reply = self.backend.conversation_hosts(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(hosts) => this.conversation_hosts = Some(hosts),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn attach_host(
        &mut self,
        alias: String,
        attached: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.hosts_busy {
            return;
        }
        self.hosts_busy = true;
        self.error = None;
        let reply = self
            .backend
            .set_conversation_host(project, session, alias, attached);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.hosts_busy = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(hosts) => this.conversation_hosts = Some(hosts),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the storage form on `alias`, filled with what the project
    /// keeps there, or closes it when it is open on that host.
    fn toggle_host_storage(&mut self, alias: String, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .storage_host
            .as_ref()
            .is_some_and(|(open, _)| *open == alias)
        {
            self.storage_host = None;
            self.layers.close(Layer::HostStorageForm);
            cx.notify();
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.error = None;
        // One thing under a card at a time.
        self.host_detail = None;
        self.staged_removing = None;
        self.layers.close(Layer::HostDetail);
        let reply = self.backend.host_storage(project, alias.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(storage) => this.show_host_storage(alias, storage, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_host_storage(
        &mut self,
        alias: String,
        storage: wisp_server::desktop::HostStorage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (field, value) in self.storage_fields.iter().zip([
            storage.data_root,
            storage.workdir_root,
            storage.results_dir,
        ]) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.storage_host = Some((alias, storage.saved));
        self.layers.open(Layer::HostStorageForm);
    }

    fn save_host_storage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some((alias, _))) =
            (self.state.project.clone(), self.storage_host.clone())
        else {
            return;
        };
        let [data_root, workdir_root, results_dir] =
            [0, 1, 2].map(|at| self.storage_fields[at].read(cx).value().to_string());
        self.error = None;
        let reply = self.backend.set_host_storage(
            project,
            alias.clone(),
            wisp_server::desktop::HostStorage {
                data_root,
                workdir_root,
                results_dir,
                saved: true,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(storage) => {
                        this.show_host_storage(alias, storage, window, cx);
                        this.notice =
                            Some(tr("Where the project keeps things there is saved.").into());
                    }
                    // The form stays open with what was typed.
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Where the project keeps things on the host whose form is open.
    fn host_storage_form(&self, saved: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        surfaces::panel_card("host-storage-form", cx)
            .child(div().text_xs().text_color(muted).child(if saved {
                tr("Saved for this project.")
            } else {
                tr("The defaults, until saved for this project.")
            }))
            .child(label(tr(
                "Uploaded data goes to (a whole path, or under ~/)",
            )))
            .child(Input::new(&self.storage_fields[0]).small())
            .child(label(tr("Run folders are made under (in the home folder)")))
            .child(Input::new(&self.storage_fields[1]).small())
            .child(label(tr("Results come back into (in the project)")))
            .child(Input::new(&self.storage_fields[2]).small())
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .pt_1()
                    .child(
                        Button::new("host-storage-save")
                            .label(tr("Save"))
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_host_storage(window, cx)
                            })),
                    )
                    .child(
                        Button::new("host-storage-cancel")
                            .label(tr("Cancel"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.storage_host = None;
                                this.layers.close(Layer::HostStorageForm);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// One place work can run: what it is, how its last check went, and
    /// what can be done with it.
    fn host_card(
        &self,
        index: usize,
        alias: Option<&str>,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let runs_on = self
            .status
            .as_ref()
            .map(|status| status.options.runs_on.as_str())
            .unwrap_or_default();
        let environments = self.environments.as_ref();
        let mut heading = div().h_flex().gap_2().child(surfaces::panel_label(
            alias.map_or_else(|| tr("This computer").to_owned(), str::to_owned),
        ));
        let mut card = surfaces::panel_entry(cx);
        let mut actions = div().h_flex().gap_1();
        match alias {
            None => {
                heading = heading.child(chip(tr("Always"), Tone::Success, cx));
                card = card.child(heading).child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Python, R and the shell run here.")),
                );
                actions = actions
                    .child(
                        Button::new("host-local-terminal")
                            .icon(glyph("terminal"))
                            .label(tr("Terminal"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_terminal_in(None, None, window, cx)
                            })),
                    )
                    .child(
                        Button::new("host-local-files")
                            .icon(glyph("folder"))
                            .label(tr("Files"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.leave_host();
                                this.show_panel_tab(PanelTab::Files, window, cx)
                            })),
                    );
                return card.child(actions);
            }
            Some(alias) => {
                let attached = self
                    .conversation_hosts
                    .as_ref()
                    .is_some_and(|hosts| hosts.iter().any(|host| host == alias));
                if runs_on == alias {
                    heading = heading.child(chip(tr("New work runs here"), Tone::Active, cx));
                }
                heading = heading.child(if attached {
                    chip(tr("Attached"), Tone::Success, cx)
                } else {
                    chip(tr("Not attached"), Tone::Neutral, cx)
                });
                let host = environments
                    .and_then(|view| view.hosts.iter().find(|host| host.alias == alias));
                let address = host
                    .map(|host| match (host.user.as_str(), host.host_name.as_str()) {
                        ("", "") => String::new(),
                        ("", name) => name.to_owned(),
                        (user, "") => format!("{user}@"),
                        (user, name) => format!("{user}@{name}"),
                    })
                    .unwrap_or_default();
                let check = environments
                    .and_then(|view| view.checks.iter().find(|check| check.alias == alias));
                let checking = self.env_checking.contains(alias);
                card = card.child(heading);
                if !address.is_empty() {
                    card = card.child(div().text_xs().text_color(muted).child(address));
                }
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(settings::check_line(check, checking, now)),
                );
                if let Some(caution) = settings::host_caution(check) {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(tr(caution)),
                    );
                }
                // The interpreters its last check found there.
                if let Some(found) = check
                    .and_then(|check| check.found.as_ref())
                    .and_then(settings::host_interpreters)
                {
                    card = card.child(div().text_xs().text_color(muted).child(found));
                }
                let (toggled, checked, terminal, browsed, stored, ran, placed) = (
                    alias.to_owned(),
                    alias.to_owned(),
                    format!("ssh:{alias}"),
                    alias.to_owned(),
                    alias.to_owned(),
                    alias.to_owned(),
                    alias.to_owned(),
                );
                let mut attach = Button::new(("host-attach", index))
                    .label(if attached {
                        tr("Take away")
                    } else {
                        tr("Attach it")
                    })
                    .xsmall()
                    .disabled(self.hosts_busy || self.conversation_hosts.is_none())
                    .tooltip(if attached {
                        tr("The conversation's work no longer runs on this host")
                    } else {
                        tr("Let the conversation's work run on this host")
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.attach_host(toggled.clone(), !attached, window, cx)
                    }));
                if !attached {
                    attach = attach.primary();
                }
                actions = actions
                    .child(attach)
                    .child(
                        icon_button(("host-check", index), "refresh", tr("Check it now"))
                            .xsmall()
                            .disabled(checking)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.check_environment(checked.clone(), window, cx)
                            })),
                    )
                    .child(
                        icon_button(("host-terminal", index), "terminal", tr("A terminal there"))
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_terminal_in(Some(terminal.clone()), None, window, cx)
                            })),
                    )
                    .child(
                        icon_button(
                            ("host-storage", index),
                            "database",
                            tr("Where the project keeps things there"),
                        )
                        .xsmall()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.toggle_host_storage(stored.clone(), window, cx)
                            },
                        )),
                    )
                    .child(
                        icon_button(("host-files", index), "folder", tr("Look at its folders"))
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_panel_tab(PanelTab::Files, window, cx);
                                this.browse_host(browsed.clone(), String::new(), window, cx)
                            })),
                    )
                    .child(
                        icon_button(("host-runs", index), "list", "The project's runs there")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_host_detail(ran.clone(), true, window, cx)
                            })),
                    )
                    .child(
                        icon_button(
                            ("host-placed", index),
                            "archive",
                            "Files the project placed there",
                        )
                        .xsmall()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.toggle_host_detail(placed.clone(), false, window, cx)
                            },
                        )),
                    );
                if let Some(detail) = self.host_detail_card(alias, now, cx) {
                    return card.child(actions).child(detail);
                }
                let storage_open = self
                    .storage_host
                    .as_ref()
                    .filter(|(open, _)| open == alias)
                    .map(|(_, saved)| *saved);
                if let Some(saved) = storage_open
                    && self.layers.contains(Layer::HostStorageForm)
                {
                    return card.child(actions).child(self.host_storage_form(saved, cx));
                }
            }
        }
        card.child(actions)
    }

    pub(super) fn render_hosts_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let mut list = div()
            .id("hosts")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            list = list.child(div().px_3().pt_3().child(error_banner(error, cx)));
        }
        if self.state.session.is_none() {
            return list
                .child(surfaces::panel_empty(
                    tr("Open a conversation to choose where its work may run."),
                    cx,
                ))
                .into_any_element();
        }
        list = list
            .child(
                surfaces::panel_note(
                    tr(
                        "Where this conversation's work may run. This computer always may; an SSH \
                 host must be attached first.",
                    ),
                    cx,
                )
                .pb_1(),
            )
            .child(self.host_card(0, None, now, cx));
        let aliases: Vec<String> = self
            .environments
            .as_ref()
            .map(|view| view.hosts.iter().map(|host| host.alias.clone()).collect())
            .unwrap_or_default();
        if aliases.is_empty() {
            list = list.child(
                surfaces::panel_empty(tr("No SSH host is saved yet."), cx).child(
                    Button::new("hosts-add")
                        .icon(glyph("server"))
                        .label(tr("Add one in Settings"))
                        .xsmall()
                        .outline()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_settings(window, cx);
                            this.show_settings_section(Section::Environments, window, cx)
                        })),
                ),
            );
        }
        for (index, alias) in aliases.iter().enumerate() {
            list = list.child(self.host_card(index + 1, Some(alias), now, cx));
        }
        list.into_any_element()
    }
}
