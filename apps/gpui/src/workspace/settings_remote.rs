//! The Remote access page of Settings: opening this desktop's projects in
//! a browser through a relay of your own.
use std::time::Duration;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::RemoteAccess;

use super::{Workspace, settings_sections::setting, surfaces::switch, tone_color};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    remote,
};

impl Workspace {
    pub(super) fn load_remote_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.remote_access();
        self.receive_remote_access(reply, true, window, cx);
    }

    /// Takes the host's answer. `fill` writes the relay's address into its
    /// field and empties the token field; a plain look at the state leaves
    /// what is being typed alone.
    fn receive_remote_access(
        &mut self,
        reply: Reply<RemoteAccess>,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(access) => {
                        if fill {
                            let url = access.relay_url.clone();
                            this.remote_fields[0]
                                .update(cx, |input, cx| input.set_value(url, window, cx));
                            this.remote_fields[1]
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        this.remote_access = Some(access);
                    }
                    // A refused form changes nothing and keeps what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Looks at the tunnel again a few times, while it connects.
    fn watch_remote_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remote_watch += 1;
        let watch = self.remote_watch;
        cx.spawn_in(window, async move |this, cx| {
            for wait in [600, 1_500, 3_000, 6_000] {
                cx.background_executor()
                    .timer(Duration::from_millis(wait))
                    .await;
                let Ok(reply) = this.update(cx, |this, _| {
                    (this.remote_watch == watch).then(|| this.backend.remote_access())
                }) else {
                    return;
                };
                let Some(reply) = reply else {
                    return;
                };
                let result = receive(reply).await;
                let _ = this.update(cx, |this, cx| {
                    if let Ok(access) = result {
                        this.remote_access = Some(access);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn save_remote_access(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        self.layers.close(Layer::ConfirmResetRemoteCode);
        let [url, token] = self
            .remote_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let token = token.trim().to_owned();
        let reply =
            self.backend
                .set_remote_access(enabled, url, (!token.is_empty()).then_some(token));
        self.receive_remote_access(reply, true, window, cx);
        self.watch_remote_access(window, cx);
        cx.notify();
    }

    fn reset_remote_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        self.layers.close(Layer::ConfirmResetRemoteCode);
        let reply = self.backend.reset_remote_access_code();
        self.receive_remote_access(reply, false, window, cx);
        self.watch_remote_access(window, cx);
        cx.notify();
    }

    pub(super) fn render_remote_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(access) = &self.remote_access else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let enabled = access.enabled;
        let [url, token] = self
            .remote_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let (line, tone) = remote::status(access);
        let copy = |id: &'static str, label: &'static str, text: String| {
            Button::new(id)
                .label(label)
                .small()
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }))
        };
        let mut page = page
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "Open this desktop's projects in a phone or a browser. Wisp dials out to \
                 your self-hosted wisp-relay; no inbound port is opened, and the relay only \
                 forwards encrypted frames. Anyone with the link or the code can use this \
                 computer's projects, so keep them to yourself.",
            )))
            .child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(tone_color(tone, cx))
                    .child(line),
            )
            .child(setting(
                tr("Remote web access"),
                tr("On, the tunnel is dialled when the app starts and kept up while it runs."),
                switch("remote-enabled", enabled, tr("Remote web access")).on_change(cx.listener(
                    move |this, _, window, cx| this.save_remote_access(!enabled, window, cx),
                )),
                cx,
            ))
            .child(setting(
                tr("Relay server URL"),
                tr("An https address; http is taken only for this computer."),
                div()
                    .w(px(320.))
                    .child(Input::new(&self.remote_fields[0]).small()),
                cx,
            ))
            .child(setting(
                tr("Relay access token"),
                remote::token_note(access),
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&self.remote_fields[1]).small()),
                    )
                    .child(
                        Button::new("remote-save")
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .disabled(!remote::changed(access, &url, &token))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_remote_access(enabled, window, cx)
                            })),
                    ),
                cx,
            ));
        if let (Some(link), Some(code)) = (&access.link, &access.code) {
            let asking = self.layers.contains(Layer::ConfirmResetRemoteCode);
            // One line: wrapping put each button on a line of its own,
            // past the bottom of the row.
            let mut actions = div()
                .h_flex()
                .flex_shrink_0()
                .gap_2()
                .justify_end()
                .child(copy("remote-copy-link", tr("Copy link"), link.clone()))
                .child(copy("remote-copy-code", tr("Copy code"), code.clone()));
            if asking {
                actions = actions
                    .child(
                        Button::new("remote-reset-yes")
                            .label(tr("Reset code"))
                            .small()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reset_remote_code(window, cx)
                            })),
                    )
                    .child(
                        Button::new("remote-reset-no")
                            .label(tr("Keep"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmResetRemoteCode);
                                cx.notify();
                            })),
                    );
            } else {
                actions = actions.child(
                    Button::new("remote-reset")
                        .label(tr("Reset code"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.open(Layer::ConfirmResetRemoteCode);
                            cx.notify();
                        })),
                );
            }
            page = page.child(setting(
                tr("Connection code"),
                if asking {
                    tr("A new code stops every old link and drops the browsers connected now.")
                } else {
                    tr("Open the link in a browser, or type the code on the relay's page.")
                },
                div()
                    .v_flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(code.clone()),
                    )
                    .child(actions),
                cx,
            ));
            // For a phone: the same link to scan. As secret as the link, so
            // it is shown only when asked for.
            let shown = self.remote_qr_shown;
            let mut scan = div().v_flex().items_end().gap_2().child(
                Button::new("remote-qr")
                    .label(if shown {
                        tr("Hide the QR code")
                    } else {
                        tr("Show the QR code")
                    })
                    .small()
                    .selected(shown)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.remote_qr_shown = !this.remote_qr_shown;
                        cx.notify();
                    })),
            );
            if let (true, Some(matrix)) = (shown, &access.qr) {
                scan = scan.child(super::settings_channels::qr_code(matrix));
            }
            page = page.child(setting(
                tr("Open on a phone"),
                tr("Scan the link with the phone's camera instead of typing it."),
                scan,
                cx,
            ));
        }
        page.child(
            div()
                .w(px(640.))
                .pt_3()
                .text_xs()
                .text_color(muted)
                .child(tr(
                    "A browser can list projects and conversations, read and send messages, stop a \
             turn and answer an approval once. It cannot change settings, and what it starts \
             asks before writing files or running commands.",
                )),
        )
        .child(self.render_sync_settings(cx))
        .child(self.render_device_bridge(cx))
    }
}

/// The StickS3 desk display: a small device on the local network that
/// shows what the agent is doing and the pet, and can open a conversation.
impl Workspace {
    pub(super) fn load_device_bridge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.device_bridge();
        self.receive_device(reply, true, window, cx);
    }

    fn receive_device(
        &mut self,
        reply: Reply<wisp_server::desktop::DeviceBridge>,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(bridge) => {
                        if fill {
                            let (address, port) =
                                (bridge.bind_ipv4.clone(), bridge.port.to_string());
                            this.device_fields[0]
                                .update(cx, |input, cx| input.set_value(address, window, cx));
                            this.device_fields[1]
                                .update(cx, |input, cx| input.set_value(port, window, cx));
                        }
                        if !bridge.has_token {
                            this.device_token = None;
                        }
                        this.device = Some(bridge);
                    }
                    Err(error) => this.device_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_device_bridge(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.device_note = None;
        let [address, port] = self
            .device_fields
            .each_ref()
            .map(|field| field.read(cx).value().trim().to_string());
        let Ok(port) = port.parse::<u32>() else {
            self.device_note = Some(Err(
                tr("Device Bridge port must be between 1 and 65535.").into()
            ));
            cx.notify();
            return;
        };
        let reply = self.backend.set_device_bridge(enabled, address, port);
        self.receive_device(reply, true, window, cx);
    }

    fn show_device_token(&mut self, rotate: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.device_note = None;
        let reply = if rotate {
            self.backend.rotate_device_bridge_token()
        } else {
            self.backend.device_bridge_token()
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(token) => this.device_token = Some(token),
                    Err(error) => this.device_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_device_bridge(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut card = div()
            .v_flex()
            .gap_2()
            .pt_6()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("StickS3 desk display")),
            )
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "A small device on your local network that shows what the agent is doing and \
                 the pet, and can open the conversation that needs you. It reads only with its \
                 token, and nothing it sends reaches an agent or a tool. Experimental.",
            )));
        let Some(bridge) = &self.device else {
            return card.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let (line, tone) = remote::device_line(bridge);
        let enabled = bridge.enabled;
        card = card
            .child(div().text_sm().text_color(tone_color(tone, cx)).child(line))
            .child(setting(
                tr("Listen for the device"),
                tr("On, the address below is listened on while the app runs."),
                switch("device-enabled", enabled, tr("Listen for the device")).on_change(
                    cx.listener(move |this, _, window, cx| {
                        this.save_device_bridge(!enabled, window, cx)
                    }),
                ),
                cx,
            ))
            .child(setting(
                tr("Address and port"),
                tr("One IPv4 address of this computer on the network the device is on."),
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(160.))
                            .child(Input::new(&self.device_fields[0]).small()),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .child(Input::new(&self.device_fields[1]).small()),
                    )
                    .child(
                        Button::new("device-save")
                            .label(tr("Save"))
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_device_bridge(enabled, window, cx)
                            })),
                    ),
                cx,
            ));
        if bridge.has_token {
            let mut token = div().h_flex().gap_2();
            match &self.device_token {
                Some(shown) => {
                    let copied = shown.clone();
                    token = token
                        .child(
                            div()
                                .text_xs()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(shown.clone()),
                        )
                        .child(
                            Button::new("device-token-copy")
                                .label(tr("Copy"))
                                .small()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                })),
                        );
                }
                None => {
                    token = token.child(
                        Button::new("device-token-show")
                            .label(tr("Show token"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_device_token(false, window, cx)
                            })),
                    );
                }
            }
            token = token.child(
                Button::new("device-token-new")
                    .label(tr("New token"))
                    .small()
                    .ghost()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.show_device_token(true, window, cx)),
                    ),
            );
            card = card.child(setting(
                tr("Device token"),
                tr("Enter it on the device. A new one locks the old one out."),
                token,
                cx,
            ));
        }
        if let Some(note) = &self.device_note {
            card = card.child(match note {
                Ok(said) => super::chip(said.clone(), crate::present::Tone::Success, cx),
                Err(error) => {
                    super::chip(crate::i18n::host(error), crate::present::Tone::Danger, cx)
                }
            });
        }
        card
    }
}
