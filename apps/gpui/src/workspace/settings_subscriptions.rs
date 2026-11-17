//! Subscriptions on the Models page of Settings: signing in to ChatGPT,
//! ChatGPT through Codex, or SuperGrok, whose models then run on the
//! sign-in instead of an API key. ChatGPT through Codex keeps a pool of
//! accounts, one active, each with a usage meter.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{CodexAccount, SignInStatus, Subscription};

use super::{Workspace, surfaces};
use crate::{backend::receive, codex_pool, layers::Layer, present::Tone, settings};

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
        let accounts = self.backend.codex_accounts();
        cx.spawn_in(window, async move |this, cx| {
            let rows = receive(reply).await;
            let accounts = receive(accounts).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(rows) = rows {
                    this.subscriptions = rows;
                }
                if let Ok(accounts) = accounts {
                    this.codex_usage
                        .retain(|id, _| accounts.iter().any(|account| &account.id == id));
                    this.codex_accounts = accounts;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs ChatGPT (Codex) models on another kept account.
    fn use_codex_account(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let email = self.codex_email(&id);
        let reply = self.backend.switch_codex_account(id);
        self.after_account_change(
            reply,
            crate::i18n::fill(tr("ChatGPT (Codex) models now run on {}."), &[&email]),
            window,
            cx,
        );
    }

    fn remove_codex_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmRemoveCodexAccount);
        let Some(id) = self.codex_removing.take() else {
            return;
        };
        let email = self.codex_email(&id);
        let reply = self.backend.remove_codex_account(id);
        self.after_account_change(
            reply,
            crate::i18n::fill(tr("{} was removed."), &[&email]),
            window,
            cx,
        );
    }

    fn codex_email(&self, id: &str) -> String {
        self.codex_accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.email.clone())
            .filter(|email| !email.is_empty())
            .unwrap_or_else(|| id.to_owned())
    }

    fn after_account_change(
        &mut self,
        reply: crate::backend::Reply<wisp_server::desktop::ModelSettings>,
        said: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(current) => {
                        this.apply_model_settings_from(current, window, cx);
                        this.sign_in_note = Some(Ok(said));
                    }
                    Err(error) => this.sign_in_note = Some(Err(error)),
                }
                this.load_subscriptions(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Brings in the sign-ins Codex CLI and CLIProxyAPI keep here.
    fn import_codex_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.import_codex_accounts();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((found, current)) => {
                        this.apply_model_settings_from(current, window, cx);
                        this.sign_in_note = Some(Ok(crate::i18n::fill(
                            tr("Found {} ChatGPT account(s) on this computer."),
                            &[&found],
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

    /// Asks ChatGPT how much of each account's allowance is used, one
    /// account after another.
    fn check_codex_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.codex_usage_checking {
            return;
        }
        self.codex_usage_checking = true;
        let ids: Vec<String> = self.codex_accounts.iter().map(|a| a.id.clone()).collect();
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            for id in ids {
                let usage = receive(backend.codex_account_usage(id.clone())).await;
                let _ = this.update(cx, |this, cx| {
                    this.codex_usage.insert(id, usage);
                    cx.notify();
                });
            }
            let _ = this.update(cx, |this, cx| {
                this.codex_usage_checking = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn usage_meter(
        &self,
        label: String,
        window: &wisp_server::desktop::UsageWindow,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let used = codex_pool::used(window);
        let fill = match codex_pool::tone(window) {
            Tone::Danger => cx.theme().danger,
            Tone::Attention => cx.theme().warning,
            _ => cx.theme().primary,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() as i64);
        div()
            .h_flex()
            .gap_2()
            .text_xs()
            .child(div().w(px(96.)).text_color(muted).child(label))
            .child(
                div()
                    .w(px(120.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(fill)
                            .w(relative(used as f32 / 100.)),
                    ),
            )
            .child(
                div()
                    .text_color(muted)
                    .child(codex_pool::meter_text(window, now)),
            )
    }

    /// One account of the ChatGPT (Codex) pool as a record row: its
    /// address, its plan, In use as a stamp; Use and Remove as icon
    /// buttons, Remove last in the danger colour. Its usage meters and the
    /// question before removing it open under it.
    fn codex_account_row(
        &self,
        account: &CodexAccount,
        cx: &mut Context<Self>,
    ) -> surfaces::RecordRow {
        let muted = cx.theme().muted_foreground;
        let id = account.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveCodexAccount)
            && self.codex_removing.as_deref() == Some(id.as_str());
        let name = if account.email.is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        let mut row =
            surfaces::record_row(SharedString::from(format!("codex-{id}")), name).icon("user");
        if !account.plan.is_empty() {
            row = row.suffix(codex_pool::plan_label(&account.plan));
        }
        if account.active {
            row = row.stamp(surfaces::StampKind::Running, tr("In use"));
        } else {
            let using = id.clone();
            row = row.action(
                surfaces::icon_action(
                    SharedString::from(format!("codex-use-{id}")),
                    "check",
                    tr("Use"),
                    surfaces::IconTone::Faded,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.use_codex_account(using.clone(), window, cx)
                })),
            );
        }
        let removing = id.clone();
        row = row.action(
            surfaces::danger_action(
                SharedString::from(format!("codex-remove-{id}")),
                "trash",
                tr("Remove"),
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.codex_removing = Some(removing.clone());
                this.layers.open(Layer::ConfirmRemoveCodexAccount);
                cx.notify();
            })),
        );
        let mut below = div().v_flex().gap_1();
        let mut opened = false;
        match self.codex_usage.get(&id) {
            Some(Ok(usage)) => {
                opened = true;
                if usage.limit_reached {
                    below = below.child(div().h_flex().child(surfaces::stamp(
                        surfaces::StampKind::Failed,
                        tr("Limit reached"),
                        cx,
                    )));
                }
                for window in [usage.short, usage.weekly].into_iter().flatten() {
                    below = below.child(self.usage_meter(
                        codex_pool::window_label(window.window_seconds),
                        &window,
                        cx,
                    ));
                }
                if usage.short.is_none() && usage.weekly.is_none() {
                    below = below.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("ChatGPT did not say how much is used.")),
                    );
                }
            }
            Some(Err(error)) => {
                opened = true;
                below = below.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::host(error)),
                );
            }
            None => {}
        }
        if asking {
            opened = true;
            below = below.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().child(tr(
                        "Remove this account? Its sign-in is forgotten on this computer.",
                    )))
                    .child(
                        Button::new("codex-remove-yes")
                            .label(tr("Remove"))
                            .xsmall()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.remove_codex_account(window, cx)
                            })),
                    )
                    .child(
                        Button::new("codex-remove-no")
                            .label(tr("Keep"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveCodexAccount);
                                this.codex_removing = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        row.below(opened.then(|| below.into_any_element()))
    }

    /// The accounts kept for ChatGPT (Codex), under its row: their record
    /// rows, then importing from the Codex CLI and checking their usage as
    /// icon and word.
    fn codex_accounts_block(&self, cx: &mut Context<Self>) -> Div {
        let rows: Vec<_> = self
            .codex_accounts
            .iter()
            .map(|account| self.codex_account_row(account, cx))
            .collect();
        let mut block = div().v_flex().gap_1();
        if !rows.is_empty() {
            block = block.child(surfaces::record_group(rows, cx));
        }
        let mut actions = div().h_flex().gap_1().child(
            surfaces::labelled_action(
                "codex-import",
                "archive-import",
                tr("Import from Codex CLI"),
                surfaces::IconSize::Inline,
                surfaces::IconTone::Faded,
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| this.import_codex_accounts(window, cx))),
        );
        if !self.codex_accounts.is_empty() {
            actions = actions.child(
                surfaces::labelled_action(
                    "codex-usage",
                    "gauge",
                    if self.codex_usage_checking {
                        tr("Checking usage…")
                    } else {
                        tr("Check usage")
                    },
                    surfaces::IconSize::Inline,
                    surfaces::IconTone::Faded,
                    cx,
                )
                .disabled(self.codex_usage_checking)
                .on_click(cx.listener(|this, _, window, cx| this.check_codex_usage(window, cx))),
            );
        }
        block.child(actions)
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

    /// A sign-in under way: a card, since it is acted on in place.
    fn sign_in_panel(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
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
        let mut panel = surfaces::card("sign-in", cx)
            .v_flex()
            .gap_2()
            .p_3()
            .child(div().text_sm().child(crate::i18n::fill(
                tr("Signing in to {}"),
                &[&subscription_name(*subscription)],
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(if failed { cx.theme().danger } else { muted })
                    .child(crate::i18n::host(&message)),
            );
        if !challenge.user_code.is_empty() {
            panel = panel.child(surfaces::read_only(challenge.user_code.clone(), cx));
        }
        let actions = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(
                surfaces::outlined_action(
                    "sign-in-open",
                    "external-link",
                    tr("Open the sign-in page"),
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
            )
            .child(
                Button::new("sign-in-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .ghost()
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
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(surfaces::input(&self.sign_in_redirect, cx).small()),
                        )
                        .child(
                            surfaces::outlined_action(
                                "sign-in-redirect",
                                "check",
                                tr("Use this address"),
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_redirect(window, cx)),
                            ),
                        ),
                );
        }
        Some(panel)
    }

    /// The subscriptions under the models, in the same column: a faded
    /// heading, then one record row each with who is signed in as a stamp;
    /// Sign in is icon and word, signing in with a code and signing out
    /// are icon buttons.
    pub(super) fn render_subscriptions(&self, cx: &mut Context<Self>) -> Div {
        let editable = self
            .settings
            .current()
            .is_none_or(|current| settings::editable(current.source));
        let mut block = div()
            .v_flex()
            .gap_1()
            .child(surfaces::settings_heading(tr("Subscriptions"), cx).pt_0())
            .child(surfaces::settings_note_help(
                tr("Use a model through your subscription instead of an API key."),
                tr(
                    "Use a model through your subscription instead of an API key. The sign-in \
                 is kept in the system keychain and shared with the WebView desktop.",
                ),
                cx,
            ));
        if !editable {
            return block.child(surfaces::settings_note_help(
                tr("The model is set by WISP_* variables."),
                tr(
                    "The model is set by WISP_* variables; subscriptions are signed in to when \
                 it is not.",
                ),
                cx,
            ));
        }
        let mut rows = Vec::new();
        for row in &self.subscriptions {
            let subscription = row.subscription;
            let mut line = surfaces::record_row(
                SharedString::from(format!("subscription-{subscription:?}")),
                subscription_name(subscription),
            )
            .icon("lock");
            line = match &row.account {
                Some(account) => line.stamp(
                    surfaces::StampKind::Success,
                    crate::i18n::fill(tr("Signed in as {}"), &[account]),
                ),
                None => line.suffix(tr("Not signed in")),
            };
            if subscription != Subscription::Xai {
                line = line.action(
                    surfaces::outlined_action(
                        SharedString::from(format!("sign-in-{subscription:?}")),
                        "external-link",
                        tr("Sign in"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_sign_in(subscription, false, window, cx)
                    })),
                );
            }
            if subscription != Subscription::ChatGpt {
                line = line.action(
                    surfaces::icon_action(
                        SharedString::from(format!("sign-in-code-{subscription:?}")),
                        "clipboard",
                        tr("Sign in with a code"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_sign_in(subscription, true, window, cx)
                    })),
                );
            }
            if row.account.is_some() {
                line = line.action(
                    surfaces::icon_action(
                        SharedString::from(format!("sign-out-{subscription:?}")),
                        "arrow-left",
                        tr("Sign out"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sign_out(subscription, window, cx)
                    })),
                );
            }
            if subscription == Subscription::Codex {
                line = line.below(Some(self.codex_accounts_block(cx).into_any_element()));
            }
            rows.push(line);
        }
        if !rows.is_empty() {
            block = block.child(surfaces::record_group(rows, cx));
        }
        if let Some(panel) = self.sign_in_panel(cx) {
            block = block.child(div().pt_2().child(panel));
        }
        if let Some(note) = &self.sign_in_note {
            block = block.child(match note {
                Ok(said) => div()
                    .text_xs()
                    .text_color(cx.theme().success)
                    .child(said.clone()),
                Err(error) => div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(crate::i18n::host(error)),
            });
        }
        block
    }
}
