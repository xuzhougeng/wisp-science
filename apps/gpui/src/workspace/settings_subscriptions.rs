//! Subscriptions on the Models page of Settings: signing in to ChatGPT,
//! ChatGPT through Codex, or SuperGrok, whose models then run on the
//! sign-in instead of an API key.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{SignInStatus, Subscription};

use super::{Workspace, chip, shell::glyph};
use crate::{backend::receive, present::Tone, settings};

/// What a subscription is called on the page.
pub(super) fn subscription_name(subscription: Subscription) -> &'static str {
    match subscription {
        Subscription::ChatGpt => "ChatGPT",
        Subscription::Codex => "ChatGPT (Codex)",
        Subscription::Xai => "SuperGrok",
    }
}

impl Workspace {
    pub(super) fn load_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.subscriptions();
        cx.spawn_in(window, async move |this, cx| {
            let rows = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(rows) = rows {
                    this.subscriptions = rows;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_sign_in(
        &mut self,
        subscription: Subscription,
        device: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_sign_in(cx);
        self.sign_in_note = None;
        let reply = self
            .backend
            .start_subscription_sign_in(subscription, device);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(challenge) => {
                        // In this computer's browser the page opens by itself.
                        if challenge.user_code.is_empty() {
                            cx.open_url(&challenge.url);
                        }
                        this.sign_in_redirect
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        let id = challenge.id.clone();
                        this.sign_in = Some((subscription, challenge));
                        this.sign_in_state = None;
                        this.follow_sign_in(id, window, cx);
                    }
                    Err(error) => this.sign_in_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads where the sign-in stands every second until it ends, and
    /// keeps it (adding a model) once it is done.
    fn follow_sign_in(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let state = receive(backend.subscription_sign_in(id.clone())).await;
                let next = this
                    .update_in(cx, |this, window, cx| {
                        let current = this.sign_in.as_ref().map(|(_, c)| c.id.clone());
                        if current.as_deref() != Some(id.as_str()) {
                            return false;
                        }
                        match state {
                            Ok(state) => {
                                let status = state.status;
                                this.sign_in_state = Some(state);
                                cx.notify();
                                match status {
                                    SignInStatus::Waiting => true,
                                    SignInStatus::Done => {
                                        this.keep_sign_in(id.clone(), window, cx);
                                        false
                                    }
                                    SignInStatus::Failed => false,
                                }
                            }
                            Err(error) => {
                                this.sign_in = None;
                                this.sign_in_note = Some(Err(error));
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !next {
                    break;
                }
            }
        })
        .detach();
    }

    fn keep_sign_in(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let account = self
            .sign_in_state
            .as_ref()
            .map(|state| state.account.clone())
            .unwrap_or_default();
        let reply = self.backend.save_subscription_sign_in(id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sign_in = None;
                this.sign_in_state = None;
                match result {
                    Ok(current) => {
                        this.apply_model_settings_from(current, window, cx);
                        this.sign_in_note = Some(Ok(crate::i18n::fill(
                            tr("Signed in as {}. Its model is in the list."),
                            &[&account],
                        )));
                    }
                    Err(error) => this.sign_in_note = Some(Err(error)),
                }
                this.load_subscriptions(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn submit_redirect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, challenge)) = &self.sign_in else {
            return;
        };
        let redirect = self.sign_in_redirect.read(cx).value().trim().to_owned();
        if redirect.is_empty() {
            return;
        }
        let reply = self
            .backend
            .submit_subscription_redirect(challenge.id.clone(), redirect);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(state) => this.sign_in_state = Some(state),
                    Err(error) => this.sign_in_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some((_, challenge)) = self.sign_in.take() {
            let reply = self.backend.cancel_subscription_sign_in(challenge.id);
            cx.background_spawn(async move {
                let _ = receive(reply).await;
            })
            .detach();
        }
        self.sign_in_state = None;
        cx.notify();
    }

    fn sign_out(
        &mut self,
        subscription: Subscription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.sign_out_subscription(subscription);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(current) => {
                        this.apply_model_settings_from(current, window, cx);
                        this.sign_in_note = None;
                    }
                    Err(error) => this.sign_in_note = Some(Err(error)),
                }
                this.load_subscriptions(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn sign_in_panel(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (subscription, challenge) = self.sign_in.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let message = self
            .sign_in_state
            .as_ref()
            .map(|state| state.message.clone())
            .unwrap_or_else(|| challenge.message.clone());
        let failed = self
            .sign_in_state
            .as_ref()
            .is_some_and(|state| state.status == SignInStatus::Failed);
        let url = challenge.url.clone();
        let mut panel = div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(crate::i18n::fill(
                        tr("Signing in to {}"),
                        &[&subscription_name(*subscription)],
                    )),
            )
            .child(if failed {
                chip(crate::i18n::host(&message), Tone::Danger, cx)
            } else {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::i18n::host(&message))
            });
        if !challenge.user_code.is_empty() {
            panel = panel.child(
                div()
                    .text_xl()
                    .font_family("Menlo")
                    .font_weight(FontWeight::MEDIUM)
                    .child(challenge.user_code.clone()),
            );
        }
        let mut actions = div().h_flex().gap_2().child(
            Button::new("sign-in-open")
                .icon(glyph("external-link"))
                .label(tr("Open the sign-in page"))
                .small()
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
        );
        actions = actions.child(
            Button::new("sign-in-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))),
        );
        panel = panel.child(actions);
        // In the browser the answer comes to this computer; when it cannot,
        // the address the browser ended on is pasted here.
        if challenge.user_code.is_empty() {
            panel = panel
                .child(div().text_xs().text_color(muted).child(tr(
                    "If the browser cannot reach this computer, paste the address it ended on:",
                )))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.sign_in_redirect).small()),
                        )
                        .child(
                            Button::new("sign-in-redirect")
                                .label(tr("Use this address"))
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_redirect(window, cx)
                                })),
                        ),
                );
        }
        Some(panel)
    }

    /// The subscriptions, each with who is signed in and a way to sign in
    /// or out, under the list of models.
    pub(super) fn render_subscriptions(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let editable = self
            .settings
            .current()
            .is_none_or(|current| settings::editable(current.source));
        let mut block = div()
            .v_flex()
            .gap_2()
            .pt_6()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Subscriptions")),
            )
            .child(div().text_xs().text_color(muted).child(tr(
                "Use a model through your subscription instead of an API key. The sign-in \
                 is kept in the system keychain and shared with the WebView desktop.",
            )));
        if !editable {
            return block.child(div().text_xs().text_color(muted).child(tr(
                "The model is set by WISP_* variables; subscriptions are signed in to when \
                 it is not.",
            )));
        }
        for row in &self.subscriptions {
            let subscription = row.subscription;
            let mut line = div().v_flex().gap_1().py_1().child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().text_sm().child(subscription_name(subscription)))
                    .child(match &row.account {
                        Some(account) => chip(
                            crate::i18n::fill(tr("Signed in as {}"), &[account]),
                            Tone::Success,
                            cx,
                        ),
                        None => chip(tr("Not signed in"), Tone::Neutral, cx),
                    }),
            );
            let mut buttons = div().h_flex().gap_1();
            if subscription != Subscription::Xai {
                buttons = buttons.child(
                    Button::new(SharedString::from(format!("sign-in-{subscription:?}")))
                        .label(tr("Sign in"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_sign_in(subscription, false, window, cx)
                        })),
                );
            }
            if subscription != Subscription::ChatGpt {
                buttons = buttons.child(
                    Button::new(SharedString::from(format!("sign-in-code-{subscription:?}")))
                        .label(tr("Sign in with a code"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_sign_in(subscription, true, window, cx)
                        })),
                );
            }
            if row.account.is_some() {
                buttons = buttons.child(
                    Button::new(SharedString::from(format!("sign-out-{subscription:?}")))
                        .label(tr("Sign out"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sign_out(subscription, window, cx)
                        })),
                );
            }
            line = line.child(buttons);
            block = block.child(line);
        }
        if let Some(panel) = self.sign_in_panel(cx) {
            block = block.child(panel);
        }
        if let Some(note) = &self.sign_in_note {
            block = block.child(match note {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        block
    }
}
