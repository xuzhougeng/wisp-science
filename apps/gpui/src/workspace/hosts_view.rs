//! The Environments tab of the side panel: this computer and the saved SSH
//! hosts, which of them the conversation's work may run on, and for each a
//! check, a terminal and its folders. A run on a host that is not attached
//! is refused, so attaching one is what lets the model use it.
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
};
use crate::{
    backend::receive,
    layers::Layer,
    panel::PanelTab,
    present::Tone,
    settings::{self, Section},
};

impl Workspace {
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
    fn host_storage_form(&self, saved: bool, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        div()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
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
        let mut heading = div().h_flex().gap_2().child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(alias.map_or_else(|| tr("This computer").to_owned(), str::to_owned)),
        );
        let mut card = div()
            .v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border);
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
                                this.remote = None;
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
                let (toggled, checked, terminal, browsed, stored) = (
                    alias.to_owned(),
                    alias.to_owned(),
                    format!("ssh:{alias}"),
                    alias.to_owned(),
                    alias.to_owned(),
                );
                let mut attach = Button::new(("host-attach", index))
                    .label(if attached {
                        tr("Take away")
                    } else {
                        tr("Attach")
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
                    );
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
        let muted = cx.theme().muted_foreground;
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
            list = list.child(div().p_3().child(error_banner(error, cx)));
        }
        if self.state.session.is_none() {
            return list
                .child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(muted)
                        .child(tr("Open a conversation to choose where its work may run.")),
                )
                .into_any_element();
        }
        list = list
            .child(div().px_3().pt_2().text_xs().text_color(muted).child(tr(
                "Where this conversation's work may run. This computer always may; an SSH \
                 host must be attached first.",
            )))
            .child(self.host_card(0, None, now, cx));
        let aliases: Vec<String> = self
            .environments
            .as_ref()
            .map(|view| view.hosts.iter().map(|host| host.alias.clone()).collect())
            .unwrap_or_default();
        if aliases.is_empty() {
            list = list.child(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(tr("No SSH host is saved yet.")),
                    )
                    .child(
                        div().h_flex().child(
                            Button::new("hosts-add")
                                .icon(glyph("server"))
                                .label(tr("Add one in Settings"))
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_settings(window, cx);
                                    this.show_settings_section(Section::Environments, window, cx)
                                })),
                        ),
                    ),
            );
        }
        for (index, alias) in aliases.iter().enumerate() {
            list = list.child(self.host_card(index + 1, Some(alias), now, cx));
        }
        list.into_any_element()
    }
}
