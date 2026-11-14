//! The user's own MCP servers, at the top of the Connections page: add,
//! change, test, switch off and remove. The form and the remove question are
//! layers inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{McpServerInput, McpServerRow, McpTransport};

use super::{Workspace, shell::glyph};
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
        let (id, name, http, target) = match row {
            Some(row) => (
                Some(row.server.id.clone()),
                row.server.name.clone(),
                matches!(row.server.transport, McpTransport::Http { .. }),
                settings::transport_line(&row.server.transport),
            ),
            None => (None, String::new(), true, String::new()),
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
        let input = McpServerInput {
            id: self.mcp_editing.clone(),
            name: self.mcp_name.read(cx).value().to_string(),
            transport,
            token: (self.mcp_http && !token.is_empty()).then_some(token),
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

    fn mcp_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let kind = |id: &'static str, label: &'static str, http: bool| {
            Button::new(id)
                .label(label)
                .small()
                .selected(self.mcp_http == http)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mcp_http = http;
                    cx.notify();
                }))
        };
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut form = div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(540.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                if self.mcp_editing.is_some() {
                    tr("Change the server")
                } else {
                    tr("Add an MCP server")
                },
            ))
            .child(label(tr("Name")))
            .child(Input::new(&self.mcp_name).small())
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(kind("mcp-http", tr("Over HTTP"), true))
                    .child(kind("mcp-stdio", tr("A program on this computer"), false)),
            )
            .child(label(if self.mcp_http {
                tr("URL")
            } else {
                tr("Command, with what it is given. Put words with spaces in quotes.")
            }))
            .child(Input::new(&self.mcp_target).small());
        if self.mcp_http {
            form = form
                .child(label(tr(
                    "Token, if the server asks for one. Sent as a bearer token and kept \
                     in the system keychain; leave empty to keep the one that is saved.",
                )))
                .child(Input::new(&self.mcp_token).small());
        } else {
            form = form.child(label(tr(
                "The program runs as you, with your files within its reach. Add only \
                 servers you trust.",
            )));
        }
        form.child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("mcp-save")
                        .label(tr("Save"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.save_mcp(window, cx))),
                )
                .child(
                    Button::new("mcp-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::McpForm);
                            this.mcp_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    fn mcp_row(&self, row: &McpServerRow, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let id = row.server.id.clone();
        let enabled = row.server.enabled;
        let asking = self.layers.contains(Layer::ConfirmRemoveMcp)
            && self.mcp_editing.as_deref() == Some(id.as_str());
        let mut about = div()
            .v_flex()
            .w(px(540.))
            .flex_shrink_0()
            .gap_1()
            .when(!enabled, |text| text.opacity(0.6))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(row.server.name.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(settings::transport_line(&row.server.transport)),
            );
        // The latest test says more than the last turn's failure.
        match (self.mcp_tested.get(&id), &row.error) {
            (Some(None), _) => {
                about = about.child(div().text_xs().text_color(muted).child(tr("Connecting…")));
            }
            (Some(Some(Ok(tools))), _) => {
                about = about.child(div().text_xs().text_color(cx.theme().success).child(
                    if tools.is_empty() {
                        tr("Connected. It offers no tools.").to_owned()
                    } else {
                        format!("Connected: {}", tools.join(", "))
                    },
                ));
            }
            (Some(Some(Err(error))), _) | (None, Some(error)) => {
                about = about.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                );
            }
            (None, None) => {}
        }
        let (testing, editing, removing, toggling) =
            (id.clone(), row.clone(), id.clone(), id.clone());
        let mut actions = div().h_flex().gap_1();
        actions = if asking {
            actions
                .child(
                    Button::new("mcp-remove-confirm")
                        .label(tr("Remove"))
                        .xsmall()
                        .danger()
                        .on_click(cx.listener(|this, _, window, cx| this.remove_mcp(window, cx))),
                )
                .child(
                    Button::new("mcp-remove-cancel")
                        .label(tr("Keep"))
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveMcp);
                            this.mcp_editing = None;
                            cx.notify();
                        })),
                )
        } else {
            actions
                .child(
                    Button::new(SharedString::from(format!("mcp-test-{id}")))
                        .label(tr("Test"))
                        .xsmall()
                        .ghost()
                        .tooltip(tr("Connect now and list its tools"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.test_mcp(testing.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("mcp-edit-{id}")))
                        .label(tr("Change"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_mcp(Some(&editing), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("mcp-remove-{id}")))
                        .label(tr("Remove"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mcp_editing = Some(removing.clone());
                            this.layers.close(Layer::McpForm);
                            this.layers.open(Layer::ConfirmRemoveMcp);
                            cx.notify();
                        })),
                )
        };
        about = about.child(actions);
        div()
            .h_flex()
            .items_start()
            .justify_between()
            .gap_6()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(
                Button::new(SharedString::from(format!("mcp-enabled-{id}")))
                    .label(if enabled { tr("On") } else { tr("Off") })
                    .small()
                    .selected(enabled)
                    .flex_shrink_0()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let reply = this
                            .backend
                            .set_mcp_server_enabled(toggling.clone(), !enabled);
                        this.change_mcp(reply, |_, _, _| {}, window, cx);
                    })),
            )
    }

    /// The servers the user added, above the connectors that ship with Wisp.
    pub(super) fn render_mcp_servers(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut section = div().v_flex().pb_6().child(
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
                                .child(tr("Your MCP servers")),
                        )
                        .child(div().text_xs().text_color(muted).child(tr(
                            "Servers you add give the model their tools in every project. \
                             Each tool asks before it runs.",
                        ))),
                )
                .child(
                    Button::new("mcp-add")
                        .icon(glyph("plus"))
                        .label(tr("Add"))
                        .small()
                        .flex_shrink_0()
                        .disabled(self.layers.contains(Layer::McpForm))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_mcp(None, window, cx)),
                        ),
                ),
        );
        if self.layers.contains(Layer::McpForm) {
            section = section.child(div().pt_3().child(self.mcp_form(cx)));
        }
        match &self.mcp_servers {
            None => {
                section = section.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Loading…")),
                );
            }
            Some(servers) if servers.is_empty() => {
                if !self.layers.contains(Layer::McpForm) {
                    section = section.child(
                        div()
                            .pt_2()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("None yet.")),
                    );
                }
            }
            Some(servers) => {
                for row in servers {
                    section = section.child(self.mcp_row(row, cx));
                }
            }
        }
        section
    }
}
