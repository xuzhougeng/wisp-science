//! The Environments page of Settings: the SSH hosts work can run on and
//! where new work goes by default. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::InputState,
};
use wisp_server::desktop::SshHostRow;

use super::{
    Workspace,
    settings_sections::{explained_setting, setting},
    shell::glyph,
    surfaces,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    settings, toolchain,
};

impl Workspace {
    pub(super) fn load_environments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_toolchain(window, cx);
        self.tool_paths_saved = false;
        let reply = self.backend.tool_paths();
        self.show_tool_paths(reply, window, cx);
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

    /// Shows the paths the host kept, each in a field with what the search
    /// path finds as its hint.
    fn show_tool_paths(
        &mut self,
        reply: Reply<Vec<wisp_server::desktop::ToolPath>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(paths) => {
                        this.tool_paths = paths
                            .into_iter()
                            .map(|path| {
                                let hint = path
                                    .found
                                    .clone()
                                    .unwrap_or_else(|| tr("Not found on the search path").into());
                                let value = path.saved.clone();
                                let field = cx.new(|cx| {
                                    let mut input = InputState::new(window, cx).placeholder(hint);
                                    input.set_value(value, window, cx);
                                    input
                                });
                                (path, field)
                            })
                            .collect();
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_tool_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self
            .tool_paths
            .iter()
            .map(|(path, field)| (path.key.to_owned(), field.read(cx).value().to_string()))
            .collect();
        self.settings_error = None;
        let reply = self.backend.set_tool_paths(paths);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.tool_paths_saved = true;
                        // Shown again as kept, trimmed.
                        let reply = this.backend.tool_paths();
                        this.show_tool_paths(reply, window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The local environment's executables: a path for each, or empty to
    /// look on the search path, as the WebView's Environments page keeps
    /// them.
    /// One setting row for each tool: its name, and the path to use in a
    /// white field whose hint is where the search path finds it.
    fn tool_paths_card(&self, cx: &mut Context<Self>) -> Div {
        let mut card = div()
            .v_flex()
            .child(surfaces::settings_heading(tr("Where the tools are"), cx))
            .child(surfaces::settings_note_help(tr("Leave a path empty to use the one on the search path."),
                tr(
                    "The full path of each program on this computer. Empty looks for it on the \
                     search path, shown as the hint. A path is checked to be a file; nothing is run.",
                ),
                cx,
            ));
        for (path, field) in &self.tool_paths {
            // What the search path finds is the field's hint, said once.
            card = card.child(setting(
                path.label,
                "",
                div().w(px(300.)).child(surfaces::input(field, cx).small()),
                cx,
            ));
        }
        card.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .pt_3()
                .child(
                    surfaces::outlined_action("tool-paths-save", "save", tr("Save paths"), cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_tool_paths(window, cx)),
                        ),
                )
                .children(
                    self.tool_paths_saved
                        .then(|| surfaces::stamp(surfaces::StampKind::Success, tr("Saved"), cx)),
                ),
        )
    }

    /// Reads the managed tools' state, and keeps reading it every half
    /// second while an install runs and the page is open.
    fn follow_toolchain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.toolchain_following {
            return;
        }
        self.toolchain_following = true;
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let result = receive(backend.toolchain()).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let running = match result {
                            Ok(status) => {
                                let running = toolchain::running(&status);
                                this.toolchain = Some(status);
                                running
                            }
                            Err(error) => {
                                this.settings_error = Some(error);
                                false
                            }
                        };
                        cx.notify();
                        let open = (this.layers.contains(Layer::Settings)
                            && this.settings_section == settings::Section::Environments)
                            || this.layers.contains(Layer::Onboarding);
                        let keep = running && open;
                        if !keep {
                            this.toolchain_following = false;
                        }
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
            }
        })
        .detach();
    }

    pub(super) fn install_toolchain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tools = self
            .toolchain_choice
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        let reply = self.backend.install_toolchain(tools);
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(status) => {
                        this.toolchain = Some(status);
                        this.follow_toolchain(window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel_toolchain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.cancel_toolchain_install();
        cx.spawn_in(window, async move |this, cx| {
            let _ = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| this.follow_toolchain(window, cx));
        })
        .detach();
    }

    /// The card that installs Python, R and Node into Wisp's own folder.
    /// Python, R and Node.js installed into Wisp's own folder: checkable
    /// chips and one Install button, the page's one filled button. The
    /// longer account of what installing does is the button's tooltip.
    fn toolchain_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let card = div()
            .v_flex()
            .gap_2()
            .child(surfaces::settings_heading(tr("Tools on this computer"), cx));
        let Some(status) = &self.toolchain else {
            return card.child(surfaces::settings_note(tr("Loading…"), cx));
        };
        let mut card = card.child(surfaces::settings_note(
            toolchain::installed_line(status),
            cx,
        ));
        if toolchain::running(status) {
            card =
                card.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(toolchain::progress_line(status)),
                        )
                        .child(
                            surfaces::outlined_action("toolchain-cancel", "stop", tr("Stop"), cx)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_toolchain(window, cx)
                                })),
                        ),
                );
        } else {
            let mut choices = div().h_flex().flex_wrap().gap_1().flex_1().min_w_0();
            for id in toolchain::CHOICES {
                let on = self.toolchain_choice.contains(&id);
                choices = choices.child(
                    surfaces::choice_chip(
                        SharedString::from(format!("toolchain-choice-{id}")),
                        on,
                        cx,
                    )
                    .child(surfaces::sized_icon(
                        if on { "check-square" } else { "square" },
                        surfaces::IconSize::Inline,
                        if on {
                            surfaces::IconTone::On
                        } else {
                            surfaces::IconTone::Faded
                        },
                        cx,
                    ))
                    .child(toolchain::tool_name(id))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(at) = this.toolchain_choice.iter().position(|c| *c == id) {
                            this.toolchain_choice.remove(at);
                        } else {
                            this.toolchain_choice.push(id);
                        }
                        cx.notify();
                    })),
                );
            }
            card = card.child(
                div().h_flex().items_center().gap_2().child(choices).child(
                    Button::new("toolchain-install")
                        .icon(glyph("download"))
                        .label(tr("Install"))
                        .small()
                        .primary()
                        .tooltip(tr(
                            "Wisp can install Python, R and Node.js into its own data folder: no \
                             administrator rights, and your shell and PATH are left alone. Downloads \
                             follow the Code requests route and mirrors in Network.",
                        ))
                        .disabled(self.toolchain_choice.is_empty())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.install_toolchain(window, cx)),
                        ),
                ),
            );
            if let Some((text, tone)) = toolchain::outcome(status) {
                let ink = surfaces::StampKind::from_tone(tone)
                    .map(|kind| surfaces::stamp_color(kind, cx))
                    .unwrap_or(muted);
                card = card.child(div().text_xs().text_color(ink).child(text));
            }
        }
        if !status.log.is_empty() {
            // What the installer said last, as facts on the paper.
            let mut log = div()
                .id("toolchain-log")
                .v_flex()
                .max_h(px(140.))
                .overflow_y_scroll()
                .py_1()
                .border_l_1()
                .border_color(cx.theme().border)
                .pl_2();
            for line in status.log.iter().rev().take(8).rev() {
                log = log.child(surfaces::fact(line.clone(), cx).truncate());
            }
            card = card.child(log);
        }
        card.child(surfaces::fact(
            crate::i18n::fill(tr("Folder: {}"), &[&status.root]),
            cx,
        ))
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
        // A saved password is never shown; typing one replaces it.
        self.env_password_sign_in = row.password_sign_in;
        self.env_has_password = row.has_password;
        self.env_password.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(
                if row.has_password {
                    tr("Saved; type a new one to change it")
                } else {
                    tr("The password for this host")
                },
                window,
                cx,
            );
        });
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
        let typed = self.env_password.read(cx).value().to_string();
        let row = SshHostRow {
            // A host being changed keeps the name it is known by.
            alias: self.env_target.clone().unwrap_or_else(|| text(0)),
            host_name: text(1),
            user: text(2),
            port,
            identity_file: text(4),
            proxy_jump: text(5),
            notes: text(6),
            password_sign_in: self.env_password_sign_in,
            has_password: false,
            password: (!typed.is_empty()).then_some(typed),
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
    pub(super) fn check_environment(
        &mut self,
        alias: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// The host being added or changed, under the list in the same column:
    /// one setting row for each field.
    fn environment_form(&self, cx: &mut Context<Self>) -> Div {
        let changing = self.env_target.is_some();
        const FIELDS: [(&str, &str); 7] = [
            ("Name", "What the host is called here, one word."),
            (
                "Address",
                "An IP or a domain; empty uses your SSH config's entry.",
            ),
            ("User", ""),
            ("Port", ""),
            (
                "Key file",
                "Where the private key is. Wisp keeps the path, never the key.",
            ),
            ("Jump host", "As OpenSSH's ProxyJump takes it."),
            ("Notes", "What the machine is for, for you and the model."),
        ];
        let mut form = div().v_flex().child(surfaces::settings_heading(
            if changing {
                tr("Change the host")
            } else {
                tr("Add an SSH host")
            },
            cx,
        ));
        for (index, ((name, note), field)) in FIELDS.iter().zip(&self.env_fields).enumerate() {
            // The name is what runs and the default refer to; it stays.
            if index == 0 && changing {
                continue;
            }
            // A host that signs in by password has no key file.
            if index == 4 && self.env_password_sign_in {
                continue;
            }
            let note = if note.is_empty() { "" } else { tr(*note) };
            form = form.child(setting(
                tr(*name),
                note,
                div().w(px(300.)).child(surfaces::input(field, cx).small()),
                cx,
            ));
        }
        // How it signs in: with a key, or with a password kept in the
        // system's keyring.
        let sign_in = surfaces::segmented(
            [(false, tr("A key")), (true, tr("A password"))]
                .into_iter()
                .enumerate()
                .map(|(number, (password, label))| {
                    surfaces::plain_segment(
                        SharedString::from(format!("environment-sign-in-{number}")),
                        label,
                        self.env_password_sign_in == password,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.env_password_sign_in = password;
                        cx.notify();
                    }))
                    .into_any_element()
                }),
            cx,
        );
        form = form.child(setting(tr("Sign in with"), "", sign_in, cx));
        if self.env_password_sign_in {
            form = form.child(setting(
                tr("Password"),
                tr("Kept in the system's keyring, never in Wisp's files."),
                div()
                    .id("environment-password")
                    .w(px(300.))
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(tr(
                            "Password: kept in the system's keyring, never in Wisp's files. It \
                             answers the host's prompt for runs, checks and terminals.",
                        ))
                        .build(window, cx)
                    })
                    .child(surfaces::input(&self.env_password, cx).small()),
                cx,
            ));
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_3()
                .child(
                    surfaces::outlined_action("environment-save", "save", tr("Save"), cx).on_click(
                        cx.listener(|this, _, window, cx| this.save_environment(window, cx)),
                    ),
                )
                .child(
                    Button::new("environment-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::EnvironmentForm);
                            this.env_target = None;
                            cx.notify();
                        })),
                ),
        )
    }

    /// One host as a record row: its name, its address as a fact, the last
    /// check as a stamp; Check outlined, a terminal and Change as icon
    /// buttons, Remove last in the danger colour. What the check found is
    /// under the row.
    fn environment_row(
        &self,
        host: &SshHostRow,
        view: &wisp_server::desktop::EnvironmentsView,
        now: i64,
        cx: &mut Context<Self>,
    ) -> surfaces::RecordRow {
        let muted = cx.theme().muted_foreground;
        let asking = self.layers.contains(Layer::ConfirmRemoveEnvironment)
            && self.env_target.as_deref() == Some(host.alias.as_str());
        let (editing, removing) = (host.clone(), host.alias.clone());
        let check = view.checks.iter().find(|check| check.alias == host.alias);
        let checking = self.env_checking.contains(&host.alias);
        let failed = !checking && check.is_some_and(|check| check.error.is_some());
        let mut row = surfaces::record_row(
            SharedString::from(format!("environment-host-{}", host.alias)),
            host.alias.clone(),
        )
        .icon("server")
        .suffix(settings::ssh_target(host));
        if checking {
            row = row.stamp(surfaces::StampKind::Running, tr("Checking…"));
        } else if failed {
            row = row.stamp(surfaces::StampKind::Failed, tr("Failed"));
        } else if check.is_some() {
            row = row.stamp(surfaces::StampKind::Success, tr("Ready"));
        } else {
            // Not checked yet: a stamp, the reason under the pointer.
            row = row
                .stamp(surfaces::StampKind::Waiting, tr("Not checked"))
                .stamp_tooltip(settings::check_line(None, false, now));
        }
        // What the row opens to: the notes, the last check and its facts.
        let mut about = div().v_flex().gap_1().pt_1();
        let mut opened = false;
        if !host.notes.is_empty() {
            opened = true;
            about = about.child(div().text_xs().text_color(muted).child(host.notes.clone()));
        }
        // While it checks, and before its first check, the stamp says so.
        if let Some(check) = check.filter(|_| !checking) {
            opened = true;
            about = about.child(
                div()
                    .text_xs()
                    .text_color(if failed { cx.theme().danger } else { muted })
                    .child(settings::check_line(Some(check), false, now)),
            );
            if let Some(error) = &check.error {
                about = about.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::host(error)),
                );
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
                        .child(surfaces::fact(value, cx).text_color(cx.theme().foreground)),
                );
            }
        }
        if asking {
            opened = true;
            about = about.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(div().flex_1())
                    .child(
                        Button::new("environment-remove-confirm")
                            .icon(glyph("trash"))
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
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveEnvironment);
                                this.env_target = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let checked = host.alias.clone();
        let context = format!("ssh:{}", host.alias);
        row.below(opened.then(|| about.into_any_element()))
            .action(
                surfaces::outlined_action(
                    SharedString::from(format!("environment-check-{}", host.alias)),
                    "refresh",
                    if check.is_some() {
                        tr("Check again")
                    } else {
                        tr("Check")
                    },
                    cx,
                )
                .tooltip(tr("Connect now and see what the host is"))
                .disabled(checking)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.check_environment(checked.clone(), window, cx)
                })),
            )
            .action(
                surfaces::icon_action(
                    SharedString::from(format!("environment-terminal-{}", host.alias)),
                    "terminal",
                    tr("Open a terminal on this host in the system's terminal"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_host_terminal(context.clone(), None, window, cx)
                })),
            )
            .action(
                surfaces::icon_action(
                    SharedString::from(format!("environment-edit-{}", host.alias)),
                    "edit",
                    tr("Change"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.edit_environment(Some(&editing), window, cx)
                })),
            )
            .action(
                surfaces::danger_action(
                    SharedString::from(format!("environment-remove-{}", host.alias)),
                    "trash",
                    tr("Remove"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.env_target = Some(removing.clone());
                    this.layers.close(Layer::EnvironmentForm);
                    this.layers.open(Layer::ConfirmRemoveEnvironment);
                    cx.notify();
                })),
            )
    }

    pub(super) fn render_environment_settings(&self, cx: &mut Context<Self>) -> Div {
        let page = self.settings_page(cx);
        let Some(view) = &self.environments else {
            return page.child(surfaces::settings_note(tr("Loading…"), cx));
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        // Where new runs start: one of this computer and the hosts.
        let place = |id: SharedString, label: String, alias: Option<String>| {
            let chosen = view.default == alias;
            let icon = settings::runs_on_icon(alias.as_deref().unwrap_or(""), None);
            surfaces::segment(id, icon, label, chosen, cx)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let reply = this.backend.set_default_environment(alias.clone());
                    this.change_environments(reply, |_, _, _| {}, window, cx);
                }))
                .into_any_element()
        };
        let mut places = vec![place(
            "environment-local".into(),
            tr("This computer").into(),
            None,
        )];
        for host in &view.hosts {
            places.push(place(
                SharedString::from(format!("environment-default-{}", host.alias)),
                host.alias.clone(),
                Some(host.alias.clone()),
            ));
        }
        let mut page = page.child(self.toolchain_card(cx)).child(
            div().pt_3().child(explained_setting(
                tr("New work runs on"),
                "",
                tr(
                    "Where the model starts runs unless you or it say otherwise. \
                    Conversation tools such as shell stay on this computer.",
                ),
                div()
                    .h_flex()
                    .flex_wrap()
                    .justify_end()
                    .child(surfaces::segmented(places, cx)),
                cx,
            )),
        );
        page = page
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(surfaces::settings_heading(tr("SSH hosts"), cx)),
                    )
                    .child(
                        surfaces::outlined_action("environment-add", "plus", tr("Add"), cx)
                            .tooltip(tr("Add an SSH host"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_environment(None, window, cx)
                            })),
                    ),
            )
            .child(
                surfaces::settings_note_help(
                    tr("Servers and clusters the model may start runs on."),
                    tr(
                        "Servers and clusters the model may start runs on. Wisp signs \
                        in with your SSH keys, agent and config. Check a host once \
                        it is added: work runs on it only after a check passes.",
                    ),
                    cx,
                )
                .pb_2(),
            );
        if view.hosts.is_empty() && !self.layers.contains(Layer::EnvironmentForm) {
            page = page.child(surfaces::settings_note(tr("None yet."), cx));
        }
        let rows: Vec<_> = view
            .hosts
            .iter()
            .map(|host| self.environment_row(host, view, now, cx))
            .collect();
        if !rows.is_empty() {
            page = page.child(surfaces::record_group(rows, cx));
        }
        if self.layers.contains(Layer::EnvironmentForm) {
            page = page.child(self.environment_form(cx));
        }
        if !view.config_aliases.is_empty() {
            let all = view.config_aliases.clone();
            let mut found = div().h_flex().flex_wrap().gap_1();
            for alias in &view.config_aliases {
                let one = vec![alias.clone()];
                found = found.child(
                    surfaces::choice_chip(
                        SharedString::from(format!("environment-import-{alias}")),
                        false,
                        cx,
                    )
                    .child(surfaces::sized_icon(
                        "plus",
                        surfaces::IconSize::Inline,
                        surfaces::IconTone::Faded,
                        cx,
                    ))
                    .child(alias.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let reply = this.backend.import_ssh_hosts(one.clone());
                        this.change_environments(reply, |_, _, _| {}, window, cx);
                    })),
                );
            }
            page = page
                .child(
                    div()
                        .h_flex()
                        .items_end()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(surfaces::settings_heading(tr("In your SSH config"), cx)),
                        )
                        .child(
                            surfaces::outlined_action(
                                "environment-import-all",
                                "download",
                                tr("Add all"),
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    let reply = this.backend.import_ssh_hosts(all.clone());
                                    this.change_environments(reply, |_, _, _| {}, window, cx);
                                },
                            )),
                        ),
                )
                .child(
                    surfaces::settings_note_help(
                        tr("Hosts in ~/.ssh/config that are not added yet."),
                        tr("Hosts named in ~/.ssh/config that are not added yet. \
                            Added this way, their address, user and key stay in \
                            that file."),
                        cx,
                    )
                    .pb_2(),
                )
                .child(found);
        }
        page.child(self.tool_paths_card(cx))
    }
}
