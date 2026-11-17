//! The user's own MCP servers, at the top of the Connections page: add,
//! change, test, switch off and remove. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    tooltip::Tooltip,
};
use wisp_server::desktop::{McpServerInput, McpServerRow, McpTransport};

use super::{
    Workspace,
    settings_sections::setting,
    shell::glyph,
    surfaces::{self, RecordRow, switch},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    settings,
};

impl Workspace {
    pub(super) fn load_mcp_servers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.mcp_servers();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(servers) => this.mcp_servers = Some(servers),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the servers, then lists them again.
    fn change_mcp<T: 'static>(
        &mut self,
        reply: Reply<T>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => then(this, window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                this.load_mcp_servers(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form on a new server, or on `row` to change it.
    fn edit_mcp(
        &mut self,
        row: Option<&McpServerRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let route = row.map(|row| row.server.proxy.clone()).unwrap_or_default();
        let (id, name, http, target, pairs) = match row {
            Some(row) => {
                let http = matches!(row.server.transport, McpTransport::Http { .. });
                let pairs = match http {
                    true => settings::pairs_text(&row.server.headers, ':'),
                    false => settings::pairs_text(&row.server.env, '='),
                };
                (
                    Some(row.server.id.clone()),
                    row.server.name.clone(),
                    http,
                    settings::transport_line(&row.server.transport),
                    pairs,
                )
            }
            None => (None, String::new(), true, String::new(), String::new()),
        };
        self.mcp_editing = id;
        self.mcp_http = http;
        self.layers.close(Layer::ConfirmRemoveMcp);
        self.layers.open(Layer::McpForm);
        self.settings_error = None;
        for (field, value) in [
            (&self.mcp_name, name),
            (&self.mcp_target, target),
            (&self.mcp_token, String::new()),
            (&self.mcp_pairs, pairs),
            (&self.mcp_route, route),
        ] {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.mcp_name
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_mcp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::McpForm) {
            return;
        }
        let target = self.mcp_target.read(cx).value().trim().to_string();
        let transport = if self.mcp_http {
            McpTransport::Http { url: target }
        } else {
            let mut words = settings::split_command(&target).into_iter();
            McpTransport::Stdio {
                command: words.next().unwrap_or_default(),
                args: words.collect(),
            }
        };
        // An empty token field leaves a saved token alone.
        let token = self.mcp_token.read(cx).value().trim().to_string();
        let separator = if self.mcp_http { ':' } else { '=' };
        let pairs = match settings::parse_pairs(&self.mcp_pairs.read(cx).value(), separator) {
            Ok(pairs) => pairs,
            Err(problem) => {
                self.settings_error = Some(problem);
                return cx.notify();
            }
        };
        let (env, headers) = match self.mcp_http {
            true => (Some(Vec::new()), Some(pairs)),
            false => (Some(pairs), Some(Vec::new())),
        };
        let input = McpServerInput {
            id: self.mcp_editing.clone(),
            name: self.mcp_name.read(cx).value().to_string(),
            transport,
            token: (self.mcp_http && !token.is_empty()).then_some(token),
            env,
            headers,
            // A program has no route of its own.
            proxy: Some(match self.mcp_http {
                true => self.mcp_route.read(cx).value().to_string(),
                false => String::new(),
            }),
        };
        let reply = self.backend.save_mcp_server(input);
        self.change_mcp(
            reply,
            |this, window, cx| {
                // Only on success: a refused server keeps the form open.
                this.layers.close(Layer::McpForm);
                this.mcp_editing = None;
                this.mcp_token
                    .update(cx, |input, cx| input.set_value("", window, cx));
            },
            window,
            cx,
        );
    }

    fn test_mcp(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.mcp_tested.insert(id.clone(), None);
        let reply = self.backend.test_mcp_server(id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.mcp_tested.insert(id, Some(result));
                this.load_mcp_servers(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_mcp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmRemoveMcp) {
            return;
        }
        let Some(id) = self.mcp_editing.clone() else {
            return;
        };
        let reply = self.backend.delete_mcp_server(id.clone());
        self.change_mcp(
            reply,
            move |this, _, _| {
                this.layers.close(Layer::ConfirmRemoveMcp);
                this.mcp_editing = None;
                this.mcp_tested.remove(&id);
            },
            window,
            cx,
        );
    }

    /// The server being added or changed, under the list: one setting row
    /// for each field. What a field takes in full is its tooltip.
    fn mcp_form(&self, cx: &mut Context<Self>) -> Div {
        let kind = |id: &'static str, icon: &'static str, label: &'static str, http: bool| {
            surfaces::segment(id, icon, label, self.mcp_http == http, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mcp_http = http;
                    cx.notify();
                }))
                .into_any_element()
        };
        let field = |id: &'static str,
                     input: &Entity<gpui_component::input::InputState>,
                     tip: &'static str,
                     tall: bool| {
            div()
                .id(id)
                .w(px(300.))
                .when(tall, |field| field.h(px(72.)))
                .when(!tip.is_empty(), |field| {
                    field.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                })
                .child(if tall {
                    surfaces::input(input, cx).h_full()
                } else {
                    surfaces::input(input, cx).small()
                })
        };
        let mut form = div()
            .v_flex()
            .child(surfaces::settings_heading(
                if self.mcp_editing.is_some() {
                    tr("Change the server")
                } else {
                    tr("Add an MCP server")
                },
                cx,
            ))
            .child(setting(
                tr("Name"),
                "",
                field("mcp-name-field", &self.mcp_name, "", false),
                cx,
            ))
            .child(setting(
                tr("How it connects"),
                "",
                surfaces::segmented(
                    [
                        kind("mcp-http", "link", tr("Over HTTP"), true),
                        kind(
                            "mcp-stdio",
                            "computer",
                            tr("A program on this computer"),
                            false,
                        ),
                    ],
                    cx,
                ),
                cx,
            ));
        if self.mcp_http {
            form = form
                .child(setting(
                    tr("URL"),
                    "",
                    field("mcp-target-field", &self.mcp_target, "", false),
                    cx,
                ))
                .child(setting(
                    tr("Token"),
                    tr("If the server asks for one; kept in the system keychain."),
                    field(
                        "mcp-token-field",
                        &self.mcp_token,
                        tr(
                            "Token, if the server asks for one. Sent as a bearer token and kept \
                             in the system keychain; leave empty to keep the one that is saved.",
                        ),
                        false,
                    ),
                    cx,
                ))
                .child(setting(
                    tr("Headers"),
                    tr("One per line as Name: value."),
                    field(
                        "mcp-headers-field",
                        &self.mcp_pairs,
                        tr(
                            "Headers sent with every request, one per line as Name: value, kept in \
                             the system keychain. A name with no value keeps the one that is saved.",
                        ),
                        true,
                    ),
                    cx,
                ))
                .child(setting(
                    tr("Route"),
                    tr("Empty follows the connector route in Network."),
                    field(
                        "mcp-route-field",
                        &self.mcp_route,
                        tr(
                            "Route: a proxy for this server alone, or none to connect directly. Empty \
                             follows the connector route in Settings, Network.",
                        ),
                        false,
                    ),
                    cx,
                ));
        } else {
            form = form
                .child(setting(
                    tr("Command"),
                    tr("Command, with what it is given. Put words with spaces in quotes."),
                    field("mcp-target-field", &self.mcp_target, "", false),
                    cx,
                ))
                .child(setting(
                    tr("Environment variables"),
                    tr("One per line as NAME=value."),
                    field(
                        "mcp-env-field",
                        &self.mcp_pairs,
                        tr(
                            "Environment variables it is started with, one per line as NAME=value, \
                             kept in the system keychain. A name with no value keeps the one that \
                             is saved.",
                        ),
                        true,
                    ),
                    cx,
                ))
                .child(
                    surfaces::settings_note_help(
                        tr("The program runs as you. Add only servers you trust."),
                        tr(
                            "The program runs as you, with your files within its reach. Add only \
                             servers you trust.",
                        ),
                        cx,
                    )
                    .pt_2(),
                );
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_3()
                .child(
                    surfaces::outlined_action("mcp-save", "save", tr("Save"), cx)
                        .on_click(cx.listener(|this, _, window, cx| this.save_mcp(window, cx))),
                )
                .child(
                    Button::new("mcp-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::McpForm);
                            this.mcp_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    /// One server as a record row: its name, how it is reached, the latest
    /// test as a stamp; a switch, Test outlined, Change, and Remove last in
    /// the danger colour. What the test found is under it.
    fn mcp_row(&self, row: &McpServerRow, cx: &mut Context<Self>) -> RecordRow {
        let muted = cx.theme().muted_foreground;
        let id = row.server.id.clone();
        let enabled = row.server.enabled;
        let asking = self.layers.contains(Layer::ConfirmRemoveMcp)
            && self.mcp_editing.as_deref() == Some(id.as_str());
        let mut record = surfaces::record_row(
            SharedString::from(format!("mcp-{id}")),
            row.server.name.clone(),
        )
        .icon("link")
        .suffix(settings::transport_line(&row.server.transport))
        .when(!enabled, |record| record.opacity(0.6));
        let mut about = div().v_flex().gap_1();
        let mut said = false;
        // Brought over without a value the keychain would not give.
        if !row.unset.is_empty() {
            said = true;
            about = about.child(div().text_xs().text_color(cx.theme().warning).child(
                crate::i18n::fill(
                    tr("Enter again before it is sent: {}"),
                    &[&row.unset.join(", ")],
                ),
            ));
        }
        // The latest test says more than the last turn's failure.
        match (self.mcp_tested.get(&id), &row.error) {
            (Some(None), _) => {
                record = record.stamp(surfaces::StampKind::Running, tr("Connecting…"));
            }
            (Some(Some(Ok(tools))), _) => {
                record = record.stamp(surfaces::StampKind::Success, tr("Ready"));
                said = true;
                about = about.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(if tools.is_empty() {
                            tr("Connected. It offers no tools.").to_owned()
                        } else {
                            crate::i18n::fill(tr("Connected: {}"), &[&tools.join(", ")])
                        }),
                );
            }
            (Some(Some(Err(error))), _) | (None, Some(error)) => {
                record = record.stamp(surfaces::StampKind::Failed, tr("Failed"));
                said = true;
                about = about.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(error)),
                );
            }
            (None, None) => {}
        }
        if asking {
            said = true;
            about = about.child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("mcp-remove-confirm")
                            .icon(glyph("trash"))
                            .label(tr("Remove"))
                            .xsmall()
                            .danger()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.remove_mcp(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("mcp-remove-cancel")
                            .label(tr("Keep"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveMcp);
                                this.mcp_editing = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let (testing, editing, removing, toggling) =
            (id.clone(), row.clone(), id.clone(), id.clone());
        record
            .below(said.then(|| about.into_any_element()))
            .action(
                switch(
                    SharedString::from(format!("mcp-enabled-{id}")),
                    enabled,
                    tr("Use this server"),
                )
                .on_change(cx.listener(move |this, _, window, cx| {
                    let reply = this
                        .backend
                        .set_mcp_server_enabled(toggling.clone(), !enabled);
                    this.change_mcp(reply, |_, _, _| {}, window, cx);
                })),
            )
            .action(
                surfaces::outlined_action(
                    SharedString::from(format!("mcp-test-{id}")),
                    "refresh",
                    tr("Test"),
                    cx,
                )
                .tooltip(tr("Connect now and list its tools"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.test_mcp(testing.clone(), window, cx)
                })),
            )
            .action(
                surfaces::icon_action(
                    SharedString::from(format!("mcp-edit-{id}")),
                    "edit",
                    tr("Change"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.edit_mcp(Some(&editing), window, cx)
                    }),
                ),
            )
            .action(
                surfaces::danger_action(
                    SharedString::from(format!("mcp-remove-{id}")),
                    "trash",
                    tr("Remove"),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mcp_editing = Some(removing.clone());
                    this.layers.close(Layer::McpForm);
                    this.layers.open(Layer::ConfirmRemoveMcp);
                    cx.notify();
                })),
            )
    }

    /// The servers the user added, above the connectors that ship with Wisp:
    /// record rows, the form under them.
    pub(super) fn render_mcp_servers(&self, cx: &mut Context<Self>) -> Div {
        let form_open = self.layers.contains(Layer::McpForm);
        let mut section = div()
            .v_flex()
            .pb_6()
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(surfaces::settings_heading(tr("Your MCP servers"), cx)),
                    )
                    .child(
                        surfaces::outlined_action("mcp-add", "plus", tr("Add"), cx)
                            .tooltip(tr("Add an MCP server"))
                            .disabled(form_open)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_mcp(None, window, cx)),
                            ),
                    ),
            )
            .child(
                surfaces::settings_note_help(
                    tr("Servers you add give the model their tools in every project."),
                    tr(
                        "Servers you add give the model their tools in every project. \
                         Each tool asks before it runs.",
                    ),
                    cx,
                )
                .pb_2(),
            );
        match &self.mcp_servers {
            None => {
                section = section.child(surfaces::settings_note(tr("Loading…"), cx));
            }
            Some(servers) if servers.is_empty() => {
                if !form_open {
                    section = section.child(surfaces::settings_note(tr("None yet."), cx));
                }
            }
            Some(servers) => {
                let rows: Vec<_> = servers.iter().map(|row| self.mcp_row(row, cx)).collect();
                section = section.child(surfaces::record_group(rows, cx));
            }
        }
        if form_open {
            section = section.child(self.mcp_form(cx));
        }
        section
    }
}
