//! The Chat apps page of Settings: WeChat and Feishu as ways to talk to
//! Wisp from a phone. Binding shows a QR code to scan; messages from the
//! owner then run in the project chats go to, and the answer comes back.
use std::time::Duration;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{FeishuBindPoll, FeishuStatus, QrMatrix, WeixinScan, WeixinStatus};

use super::{Workspace, chip, settings_sections::setting, tone_color};
use crate::{
    backend::{Reply, receive},
    channels,
    layers::Layer,
    present::Tone,
};

/// How long one module of a QR code is drawn.
const MODULE: f32 = 4.;

/// A QR code drawn as squares, with the light margin scanners need.
fn qr_code(matrix: &QrMatrix) -> Div {
    let mut rows = div().v_flex().p(px(MODULE * 4.)).bg(gpui::white());
    for row in matrix.dark.chunks(matrix.width.max(1)) {
        let mut line = div().flex();
        for dark in row {
            line = line.child(
                div()
                    .size(px(MODULE))
                    .when(*dark, |module| module.bg(gpui::black())),
            );
        }
        rows = rows.child(line);
    }
    rows
}

impl Workspace {
    pub(super) fn load_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.weixin_status();
        self.receive_weixin(reply, window, cx);
        let reply = self.backend.feishu_status();
        self.receive_feishu(reply, true, window, cx);
        self.load_lab(window, cx);
    }

    /// Takes the host's answer about Feishu. `fill` writes the App ID and
    /// the service into the form and empties the secret field.
    fn receive_feishu(
        &mut self,
        reply: Reply<FeishuStatus>,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(status) => {
                        if fill {
                            let app_id = status.app_id.clone();
                            this.feishu_fields[0]
                                .update(cx, |input, cx| input.set_value(app_id, window, cx));
                            this.feishu_fields[1]
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            let owner = status.owner.clone();
                            this.feishu_fields[2]
                                .update(cx, |input, cx| input.set_value(owner, window, cx));
                            this.feishu_lark = status.international;
                        }
                        this.feishu = Some(status);
                    }
                    // A refused form changes nothing and keeps what was typed.
                    Err(error) => this.feishu_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Looks at the bot again a few times while it connects.
    fn watch_feishu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            for wait in [800, 2_000, 5_000] {
                cx.background_executor()
                    .timer(Duration::from_millis(wait))
                    .await;
                let result = receive(backend.feishu_status()).await;
                let open = this
                    .update(cx, |this, cx| {
                        if let Ok(status) = result {
                            this.feishu = Some(status);
                        }
                        cx.notify();
                        this.settings_section == crate::settings::Section::Channels
                    })
                    .unwrap_or(false);
                if !open {
                    return;
                }
            }
        })
        .detach();
    }

    fn save_feishu(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.feishu_note = None;
        let [app_id, secret, _] = self
            .feishu_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let secret = secret.trim().to_owned();
        let reply = self.backend.set_feishu(
            enabled,
            self.feishu_lark,
            app_id,
            (!secret.is_empty()).then_some(secret),
        );
        self.receive_feishu(reply, true, window, cx);
        self.watch_feishu(window, cx);
    }

    fn save_feishu_owner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.feishu_note = None;
        let owner = self.feishu_fields[2].read(cx).value().trim().to_owned();
        let reply = self.backend.set_feishu_owner(owner);
        self.receive_feishu(reply, true, window, cx);
    }

    fn answer_feishu_pending(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.feishu_note = None;
        let reply = if confirm {
            self.backend.confirm_feishu_owner()
        } else {
            self.backend.reject_feishu_owner()
        };
        self.receive_feishu(reply, true, window, cx);
    }

    /// Gets a QR code that makes the bot's app in Feishu (or Lark) and
    /// follows the scan as often as Feishu allows, until it ends or the
    /// page is left.
    fn bind_feishu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.feishu_note = None;
        self.cancel_feishu_bind(cx);
        let bind = self.feishu_bind_id;
        let backend = self.backend.clone();
        let lark = self.feishu_lark;
        cx.spawn_in(window, async move |this, cx| {
            let started = match receive(backend.start_feishu_bind(lark)).await {
                Ok(started) => started,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.feishu_note = Some(Err(error));
                        cx.notify();
                    });
                    return;
                }
            };
            let flow = started.flow.clone();
            let _ = this.update(cx, |this, cx| {
                if this.feishu_bind_id == bind {
                    this.feishu_qr = Some(started);
                }
                cx.notify();
            });
            let mut wait = 1_000u64;
            for _ in 0..600 {
                cx.background_executor()
                    .timer(Duration::from_millis(wait.clamp(500, 10_000)))
                    .await;
                let current = this
                    .update(cx, |this, cx| {
                        let shown = this.layers.contains(Layer::Settings)
                            && this.settings_section == crate::settings::Section::Channels;
                        if !shown && this.feishu_bind_id == bind {
                            this.cancel_feishu_bind(cx);
                        }
                        shown && this.feishu_bind_id == bind && this.feishu_qr.is_some()
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                let result = receive(backend.poll_feishu_bind(flow.clone())).await;
                let next = this
                    .update_in(cx, |this, window, cx| {
                        if this.feishu_bind_id != bind {
                            return None;
                        }
                        let next = match result {
                            Ok(FeishuBindPoll::Pending { retry_after_ms }) => Some(retry_after_ms),
                            Ok(FeishuBindPoll::Confirmed { .. }) => {
                                this.feishu_qr = None;
                                this.feishu_note = Some(Ok(tr(
                                    "The Feishu app is made and you are its owner. Turn it on to answer messages.",
                                )
                                .to_owned()));
                                this.load_channels(window, cx);
                                None
                            }
                            Ok(FeishuBindPoll::Denied) => {
                                this.feishu_qr = None;
                                this.feishu_note =
                                    Some(Err(tr("The scan was refused in Feishu.").to_owned()));
                                None
                            }
                            Ok(FeishuBindPoll::Expired) => {
                                this.feishu_qr = None;
                                this.feishu_note =
                                    Some(Err(tr("The code expired; get a new one.").to_owned()));
                                None
                            }
                            Err(error) => {
                                this.feishu_qr = None;
                                this.feishu_note = Some(Err(error));
                                None
                            }
                        };
                        cx.notify();
                        next
                    })
                    .ok()
                    .flatten();
                match next {
                    Some(after) => wait = after,
                    None => return,
                }
            }
        })
        .detach();
    }

    pub(super) fn cancel_feishu_bind(&mut self, cx: &mut Context<Self>) {
        self.feishu_bind_id += 1;
        if let Some(started) = self.feishu_qr.take() {
            let reply = self.backend.cancel_feishu_bind(started.flow);
            cx.background_spawn(async move {
                let _ = receive(reply).await;
            })
            .detach();
        }
        cx.notify();
    }

    fn unbind_feishu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmUnbindFeishu);
        self.feishu_note = None;
        let reply = self.backend.unbind_feishu();
        self.receive_feishu(reply, true, window, cx);
    }

    fn feishu_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let Some(status) = &self.feishu else {
            return div().text_sm().text_color(muted).child(tr("Loading…"));
        };
        let (line, tone) = channels::feishu_line(status);
        let enabled = status.enabled;
        let lark = self.feishu_lark;
        let [app_id, secret, owner] = self
            .feishu_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let changed = channels::feishu_changed(status, &app_id, &secret, lark);
        let mut card = div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("Feishu")),
                    )
                    .child(div().text_sm().text_color(tone_color(tone, cx)).child(line)),
            )
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "A bot of your own Feishu (or Lark) app over Feishu's long connection, so no public address is needed. Scanning makes the app, or enter one you made. Only its owner is answered. The App Secret is kept in the system keychain and shared with the WebView desktop.",
            )))
            .child(setting(
                tr("Answer messages"),
                tr("On, the bot runs while the app is open."),
                Button::new("feishu-enabled")
                    .label(if enabled { tr("On") } else { tr("Off") })
                    .small()
                    .selected(enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_feishu(!enabled, window, cx)
                    })),
                cx,
            ))
            .child(setting(
                tr("Service"),
                tr("Feishu in China, or Lark elsewhere."),
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Button::new("feishu-service-feishu")
                            .label(tr("Feishu"))
                            .small()
                            .selected(!lark)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.feishu_lark = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("feishu-service-lark")
                            .label("Lark")
                            .small()
                            .selected(lark)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.feishu_lark = true;
                                cx.notify();
                            })),
                    ),
                cx,
            ))
            .child(setting(
                "App ID",
                tr("The app's ID from the Feishu developer console."),
                div()
                    .w(px(320.))
                    .child(Input::new(&self.feishu_fields[0]).small()),
                cx,
            ))
            .child(setting(
                "App Secret",
                if status.has_secret {
                    tr("A secret is kept; type a new one to replace it.")
                } else {
                    tr("No secret is kept yet.")
                },
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&self.feishu_fields[1]).small()),
                    )
                    .child(
                        Button::new("feishu-save")
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .disabled(!changed)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_feishu(enabled, window, cx)
                            })),
                    ),
                cx,
            ))
            .child(setting(
                tr("Owner"),
                tr("The open_id of the one person the bot answers; empty answers nobody."),
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&self.feishu_fields[2]).small()),
                    )
                    .child(
                        Button::new("feishu-owner-save")
                            .label(tr("Save"))
                            .small()
                            .disabled(owner.trim() == status.owner)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_feishu_owner(window, cx)
                            })),
                    ),
                cx,
            ));
        if !status.pending_owner.is_empty() {
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().text_xs().child(crate::i18n::fill(
                        tr("{} wrote to the bot and waits to be its owner."),
                        &[&status.pending_owner],
                    )))
                    .child(
                        Button::new("feishu-pending-yes")
                            .label(tr("Make owner"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_feishu_pending(true, window, cx)
                            })),
                    )
                    .child(
                        Button::new("feishu-pending-no")
                            .label(tr("Refuse"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_feishu_pending(false, window, cx)
                            })),
                    ),
            );
        }
        let asking = self.layers.contains(Layer::ConfirmUnbindFeishu);
        let mut actions = div().h_flex().gap_2();
        if self.feishu_qr.is_none() {
            actions = actions.child(
                Button::new("feishu-bind")
                    .label(tr("Make the app by scanning"))
                    .small()
                    .when(!status.bound, |button| button.primary())
                    .on_click(cx.listener(|this, _, window, cx| this.bind_feishu(window, cx))),
            );
        }
        if status.bound && !asking {
            actions = actions.child(
                Button::new("feishu-unbind")
                    .label(tr("Unbind"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.layers.open(Layer::ConfirmUnbindFeishu);
                        cx.notify();
                    })),
            );
        }
        card = card.child(actions);
        if asking {
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().text_xs().child(tr(
                        "Unbind Feishu? The bot stops, and its App Secret and owner are forgotten on this computer.",
                    )))
                    .child(
                        Button::new("feishu-unbind-yes")
                            .label(tr("Unbind"))
                            .small()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.unbind_feishu(window, cx)
                            })),
                    )
                    .child(
                        Button::new("feishu-unbind-no")
                            .label(tr("Keep"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmUnbindFeishu);
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(started) = &self.feishu_qr {
            card = card.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_4()
                    .child(qr_code(&started.matrix))
                    .child(
                        div()
                            .v_flex()
                            .gap_2()
                            .child(div().text_sm().child(tr(
                                "Scan this code with Feishu on your phone and confirm: the bot's app is made and you become its owner.",
                            )))
                            .child(
                                Button::new("feishu-bind-cancel")
                                    .label(tr("Cancel"))
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_feishu_bind(cx)),
                                    ),
                            ),
                    ),
            );
        }
        if let Some(note) = &self.feishu_note {
            card = card.child(match note {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        card
    }

    fn receive_weixin(
        &mut self,
        reply: Reply<WeixinStatus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.weixin = Some(status),
                    Err(error) => this.weixin_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Looks at the bot again a few times while it connects.
    fn watch_weixin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            for wait in [800, 2_000, 5_000] {
                cx.background_executor()
                    .timer(Duration::from_millis(wait))
                    .await;
                let result = receive(backend.weixin_status()).await;
                let open = this
                    .update(cx, |this, cx| {
                        if let Ok(status) = result {
                            this.weixin = Some(status);
                        }
                        cx.notify();
                        this.settings_section == crate::settings::Section::Channels
                    })
                    .unwrap_or(false);
                if !open {
                    return;
                }
            }
        })
        .detach();
    }

    fn set_weixin_enabled(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.weixin_note = None;
        let reply = self.backend.set_weixin_enabled(enabled);
        self.receive_weixin(reply, window, cx);
        self.watch_weixin(window, cx);
    }

    /// Gets a QR code and follows its scan every two seconds until it is
    /// confirmed, expires, or the panel is closed.
    fn bind_weixin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.weixin_note = None;
        self.weixin_scan = None;
        self.weixin_bind += 1;
        let bind = self.weixin_bind;
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let qr = match receive(backend.start_weixin_bind()).await {
                Ok(qr) => qr,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.weixin_note = Some(Err(error));
                        cx.notify();
                    });
                    return;
                }
            };
            let code = qr.qrcode.clone();
            let _ = this.update(cx, |this, cx| {
                if this.weixin_bind == bind {
                    this.weixin_qr = Some(qr);
                }
                cx.notify();
            });
            // A code lasts a few minutes; this stops well after.
            for _ in 0..240 {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                // Leaving the page puts the code away.
                let current = this
                    .update(cx, |this, cx| {
                        let shown = this.layers.contains(Layer::Settings)
                            && this.settings_section == crate::settings::Section::Channels;
                        if !shown && this.weixin_bind == bind {
                            this.cancel_weixin_bind(cx);
                        }
                        shown && this.weixin_bind == bind && this.weixin_qr.is_some()
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                let result = receive(backend.poll_weixin_bind(code.clone())).await;
                let go_on = this
                    .update_in(cx, |this, window, cx| {
                        if this.weixin_bind != bind {
                            return false;
                        }
                        let go_on = match result {
                            Ok(scan) => {
                                this.weixin_scan = Some(scan);
                                if scan == WeixinScan::Confirmed {
                                    this.weixin_qr = None;
                                    this.weixin_note = Some(Ok(tr(
                                        "WeChat is bound. Turn it on to answer messages.",
                                    )
                                    .to_owned()));
                                    this.load_channels(window, cx);
                                }
                                channels::scanning(Some(scan))
                            }
                            Err(error) => {
                                this.weixin_qr = None;
                                this.weixin_note = Some(Err(error));
                                false
                            }
                        };
                        cx.notify();
                        go_on
                    })
                    .unwrap_or(false);
                if !go_on {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn cancel_weixin_bind(&mut self, cx: &mut Context<Self>) {
        self.weixin_bind += 1;
        self.weixin_qr = None;
        self.weixin_scan = None;
        cx.notify();
    }

    fn unbind_weixin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmUnbindWeixin);
        self.weixin_note = None;
        let reply = self.backend.unbind_weixin();
        self.receive_weixin(reply, window, cx);
    }

    fn weixin_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let Some(status) = &self.weixin else {
            return div().text_sm().text_color(muted).child(tr("Loading…"));
        };
        let (line, tone) = channels::weixin_line(status);
        let enabled = status.enabled;
        let mut card = div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("WeChat")),
                    )
                    .child(div().text_sm().text_color(tone_color(tone, cx)).child(line)),
            )
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "A WeChat bot through Tencent's iLink service. Only the person who scanned \
                 the code is answered, one to one. The bot's token is kept in the system \
                 keychain and shared with the WebView desktop.",
            )));
        if status.bound {
            card = card.child(setting(
                tr("Answer messages"),
                tr("On, the bot runs while the app is open."),
                Button::new("weixin-enabled")
                    .label(if enabled { tr("On") } else { tr("Off") })
                    .small()
                    .selected(enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_weixin_enabled(!enabled, window, cx)
                    })),
                cx,
            ));
        }
        let asking = self.layers.contains(Layer::ConfirmUnbindWeixin);
        let mut actions = div().h_flex().gap_2();
        if self.weixin_qr.is_none() {
            actions = actions.child(
                Button::new("weixin-bind")
                    .label(if status.bound {
                        tr("Bind again")
                    } else {
                        tr("Bind with a QR code")
                    })
                    .small()
                    .when(!status.bound, |button| button.primary())
                    .on_click(cx.listener(|this, _, window, cx| this.bind_weixin(window, cx))),
            );
        }
        if status.bound && !asking {
            actions = actions.child(
                Button::new("weixin-unbind")
                    .label(tr("Unbind"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.layers.open(Layer::ConfirmUnbindWeixin);
                        cx.notify();
                    })),
            );
        }
        card = card.child(actions);
        if asking {
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().text_xs().child(tr(
                        "Unbind WeChat? The bot stops and its token is forgotten on this computer.",
                    )))
                    .child(
                        Button::new("weixin-unbind-yes")
                            .label(tr("Unbind"))
                            .small()
                            .danger()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.unbind_weixin(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("weixin-unbind-no")
                            .label(tr("Keep"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmUnbindWeixin);
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(qr) = &self.weixin_qr {
            card = card.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_4()
                    .child(qr_code(&qr.matrix))
                    .child(
                        div()
                            .v_flex()
                            .gap_2()
                            .child(div().text_sm().child(channels::scan_note(self.weixin_scan)))
                            .child(
                                Button::new("weixin-bind-cancel")
                                    .label(tr("Cancel"))
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_weixin_bind(cx)),
                                    ),
                            ),
                    ),
            );
        } else if self.weixin_scan == Some(WeixinScan::Expired) {
            card = card.child(chip(
                channels::scan_note(self.weixin_scan),
                Tone::Attention,
                cx,
            ));
        }
        if let Some(note) = &self.weixin_note {
            card = card.child(match note {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        card
    }

    pub(super) fn render_channel_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut commands = div().v_flex().gap_1();
        for (command, what) in channels::COMMANDS {
            commands = commands.child(
                div()
                    .h_flex()
                    .gap_3()
                    .text_xs()
                    .child(
                        div()
                            .w(px(88.))
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(command),
                    )
                    .child(div().text_color(muted).child(tr(what))),
            );
        }
        self.settings_page(cx)
            .child(div().w(px(640.)).text_xs().text_color(muted).child(tr(
                "Talk to Wisp from a chat app. A message runs in the project chats go to and \
                 continues its latest conversation there, which the desktop shows as usual; \
                 the answer comes back in the chat. A turn started from a chat always asks \
                 before changing anything, and the approval is sent to the chat with a code.",
            )))
            .child(div().pt_4().child(self.weixin_card(cx)))
            .child(div().pt_6().child(self.feishu_card(cx)))
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .pt_6()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Wisp Lab"),
                    )
                    .child(self.lab_card(cx)),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .pt_6()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr("Commands a chat can send")),
                    )
                    .child(commands),
            )
    }
}
