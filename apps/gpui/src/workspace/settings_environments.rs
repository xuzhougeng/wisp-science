//! The Environments page of Settings: the SSH hosts work can run on and
//! where new work goes by default. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::SshHostRow;

use super::{Workspace, shell::glyph};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    settings,
};

impl Workspace {
    pub(super) fn load_environments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.environments();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(view) => this.environments = Some(view),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the environments, then reads them again.
    fn change_environments(
        &mut self,
        reply: Reply<()>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => then(this, window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                this.load_environments(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form on a new host, or on `row` to change it.
    fn edit_environment(
        &mut self,
        row: Option<&SshHostRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = row.cloned().unwrap_or_default();
        self.env_target = Some(row.alias.clone()).filter(|alias| !alias.is_empty());
        self.layers.close(Layer::ConfirmRemoveEnvironment);
        self.layers.open(Layer::EnvironmentForm);
        self.settings_error = None;
        let values = [
            row.alias,
            row.host_name,
            row.user,
            row.port.map(|port| port.to_string()).unwrap_or_default(),
            row.identity_file,
            row.proxy_jump,
            row.notes,
        ];
        for (field, value) in self.env_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.env_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::EnvironmentForm) {
            return;
        }
        let text = |index: usize| self.env_fields[index].read(cx).value().trim().to_string();
        let port = match settings::port(&text(3)) {
            Ok(port) => port,
            Err(problem) => {
                self.settings_error = Some(problem);
                return cx.notify();
            }
        };
        let row = SshHostRow {
            // A host being changed keeps the name it is known by.
            alias: self.env_target.clone().unwrap_or_else(|| text(0)),
            host_name: text(1),
            user: text(2),
            port,
            identity_file: text(4),
            proxy_jump: text(5),
            notes: text(6),
        };
        let reply = self.backend.save_ssh_host(row);
        self.change_environments(
            reply,
            |this, _, _| {
                // Only on success: a refused host keeps the form open.
                this.layers.close(Layer::EnvironmentForm);
                this.env_target = None;
            },
            window,
            cx,
        );
    }

    /// Connects to `alias` now. What it found, or why it failed, is part of
    /// the list that is read again afterwards.
    fn check_environment(&mut self, alias: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.env_checking.insert(alias.clone()) {
            return;
        }
        self.settings_error = None;
        let reply = self.backend.check_environment(alias.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.env_checking.remove(&alias);
                // Only a host that is no longer saved is an error here.
                if let Err(error) = result {
                    this.settings_error = Some(error);
                }
                this.load_environments(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmRemoveEnvironment) {
            return;
        }
        let Some(alias) = self.env_target.clone() else {
            return;
        };
        let reply = self.backend.remove_ssh_host(alias);
        self.change_environments(
            reply,
            |this, _, _| {
                this.layers.close(Layer::ConfirmRemoveEnvironment);
                this.env_target = None;
            },
            window,
            cx,
        );
    }

    fn environment_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let changing = self.env_target.is_some();
        const LABELS: [&str; 7] = [
            "Name: what the host is called here, one word",
            "Address: an IP or a domain. Leave empty to use the entry of that name in your SSH config",
            "User",
            "Port",
            "Key file: where the private key is. Wisp keeps the path, never the key",
            "Jump host, as OpenSSH's ProxyJump takes it",
            "Notes for yourself and the model: what the machine is for",
        ];
        let mut form = div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(540.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if changing {
                        tr("Change the host")
                    } else {
                        tr("Add an SSH host")
                    }),
            );
        for (index, (label, field)) in LABELS.iter().zip(&self.env_fields).enumerate() {
            // The name is what runs and the default refer to; it stays.
            if index == 0 && changing {
                continue;
            }
            form = form
                .child(div().text_xs().text_color(muted).child(tr(label)))
                .child(Input::new(field).small());
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("environment-save")
                        .label(tr("Save"))
                        .small()
                        .primary()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_environment(window, cx)),
                        ),
                )
                .child(
                    Button::new("environment-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::EnvironmentForm);
                            this.env_target = None;
                            cx.notify();
                        })),
                ),
        )
    }

    pub(super) fn render_environment_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(view) = &self.environments else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let place = |id: SharedString, label: String, alias: Option<String>| {
            let chosen = view.default == alias;
            Button::new(id)
                .label(label)
                .small()
                .selected(chosen)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let reply = this.backend.set_default_environment(alias.clone());
                    this.change_environments(reply, |_, _, _| {}, window, cx);
                }))
        };
        let mut places = div().h_flex().flex_wrap().gap_1().child(place(
            "environment-local".into(),
            tr("This computer").into(),
            None,
        ));
        for host in &view.hosts {
            places = places.child(place(
                SharedString::from(format!("environment-default-{}", host.alias)),
                host.alias.clone(),
                Some(host.alias.clone()),
            ));
        }
        let mut page = page
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .pb_4()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("New work runs on")),
                    )
                    .child(div().w(px(540.)).text_xs().text_color(muted).child(tr(
                        "Where the model starts runs unless you or it say otherwise. \
                         Conversation tools such as shell stay on this computer.",
                    )))
                    .child(places),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        div()
                            .v_flex()
                            .w(px(540.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(tr("SSH hosts")),
                            )
                            .child(div().text_xs().text_color(muted).child(tr(
                                "Servers and clusters the model may start runs on. Wisp signs \
                                 in with your SSH keys, agent and config. Check a host once \
                                 it is added: work runs on it only after a check passes.",
                            ))),
                    )
                    .child(
                        Button::new("environment-add")
                            .icon(glyph("plus"))
                            .label(tr("Add"))
                            .small()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_environment(None, window, cx)
                            })),
                    ),
            );
        if self.layers.contains(Layer::EnvironmentForm) {
            page = page.child(div().pt_3().child(self.environment_form(cx)));
        }
        if view.hosts.is_empty() && !self.layers.contains(Layer::EnvironmentForm) {
            page = page.child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("None yet.")),
            );
        }
        for host in &view.hosts {
            let asking = self.layers.contains(Layer::ConfirmRemoveEnvironment)
                && self.env_target.as_deref() == Some(host.alias.as_str());
            let (editing, removing) = (host.clone(), host.alias.clone());
            let mut about = div()
                .v_flex()
                .w(px(540.))
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(host.alias.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(settings::ssh_target(host)),
                );
            if !host.notes.is_empty() {
                about = about.child(div().text_xs().text_color(muted).child(host.notes.clone()));
            }
            let check = view.checks.iter().find(|check| check.alias == host.alias);
            let checking = self.env_checking.contains(&host.alias);
            let failed = !checking && check.is_some_and(|check| check.error.is_some());
            about = about.child(
                div()
                    .text_xs()
                    .text_color(if failed { cx.theme().danger } else { muted })
                    .child(settings::check_line(check, checking, now)),
            );
            if let Some(check) = check.filter(|_| !checking) {
                if let Some(error) = &check.error {
                    about = about.child(div().text_xs().text_color(muted).child(error.clone()));
                }
                for (label, value) in check.found.iter().flat_map(settings::check_facts) {
                    about = about.child(
                        div()
                            .h_flex()
                            .items_start()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_shrink_0()
                                    .text_color(muted)
                                    .child(label),
                            )
                            .child(div().child(value)),
                    );
                }
            }
            let checked = host.alias.clone();
            let actions = if asking {
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Button::new("environment-remove-confirm")
                            .label(tr("Remove"))
                            .xsmall()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.remove_environment(window, cx)
                            })),
                    )
                    .child(
                        Button::new("environment-remove-cancel")
                            .label(tr("Keep"))
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveEnvironment);
                                this.env_target = None;
                                cx.notify();
                            })),
                    )
            } else {
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!(
                            "environment-check-{}",
                            host.alias
                        )))
                        .label(if check.is_some() {
                            tr("Check again")
                        } else {
                            tr("Check")
                        })
                        .xsmall()
                        .disabled(checking)
                        .tooltip(tr("Connect now and see what the host is"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.check_environment(checked.clone(), window, cx)
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "environment-edit-{}",
                            host.alias
                        )))
                        .label(tr("Change"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.edit_environment(Some(&editing), window, cx)
                            },
                        )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "environment-remove-{}",
                            host.alias
                        )))
                        .label(tr("Remove"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.env_target = Some(removing.clone());
                            this.layers.close(Layer::EnvironmentForm);
                            this.layers.open(Layer::ConfirmRemoveEnvironment);
                            cx.notify();
                        })),
                    )
            };
            page = page.child(
                about
                    .child(actions)
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border),
            );
        }
        if !view.config_aliases.is_empty() {
            let all = view.config_aliases.clone();
            let mut found = div().h_flex().flex_wrap().gap_1();
            for alias in &view.config_aliases {
                let one = vec![alias.clone()];
                found = found.child(
                    Button::new(SharedString::from(format!("environment-import-{alias}")))
                        .icon(glyph("plus"))
                        .label(alias.clone())
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let reply = this.backend.import_ssh_hosts(one.clone());
                            this.change_environments(reply, |_, _, _| {}, window, cx);
                        })),
                );
            }
            page = page.child(
                div()
                    .v_flex()
                    .gap_2()
                    .pt_5()
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .v_flex()
                                    .w(px(540.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(tr("In your SSH config")),
                                    )
                                    .child(div().text_xs().text_color(muted).child(tr(
                                        "Hosts named in ~/.ssh/config that are not added yet. \
                                         Added this way, their address, user and key stay in \
                                         that file.",
                                    ))),
                            )
                            .child(
                                Button::new("environment-import-all")
                                    .label(tr("Add all"))
                                    .small()
                                    .flex_shrink_0()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let reply = this.backend.import_ssh_hosts(all.clone());
                                        this.change_environments(reply, |_, _, _| {}, window, cx);
                                    })),
                            ),
                    )
                    .child(found),
            );
        }
        page
    }
}
