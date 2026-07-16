//! Settings → Channels pane: Feishu bot credentials + enable toggle, WeChat
//! iLink QR binding + enable toggle. Self-contained: owns its signals and
//! fetches `channels_status` on mount, so it needs no SettingsViewState
//! plumbing.

use crate::app_support::js_error_text;
use crate::bindings::invoke_checked;
use crate::dto::{ChannelsStatus, WeixinBindStart};
use crate::i18n::{localize_backend, t, Locale};
use crate::text::{event_target_checked, event_target_input};
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsValue;

/// Promise-backed sleep so the QR poll can be a plain async loop.
async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn state_label(locale: Locale, state: &str) -> String {
    match state {
        "running" => t(locale, "channels.state.running"),
        "connecting" => t(locale, "channels.state.connecting"),
        "error" => t(locale, "channels.state.error"),
        _ => t(locale, "channels.state.stopped"),
    }
    .into()
}

fn state_tone(state: &str) -> &'static str {
    match state {
        "running" => "running",
        "connecting" => "connecting",
        "error" => "error",
        _ => "stopped",
    }
}

#[component]
pub(super) fn ChannelsPane(locale: RwSignal<Locale>) -> impl IntoView {
    let status = create_rw_signal(None::<ChannelsStatus>);
    let feishu_app_id = create_rw_signal(String::new());
    let feishu_secret = create_rw_signal(String::new());
    let msg = create_rw_signal(None::<(bool, String)>);
    let qr = create_rw_signal(None::<WeixinBindStart>);
    // Bumped to cancel a stale QR poll loop (new scan, unbind, unmount race).
    let poll_gen = create_rw_signal(0usize);

    let refresh = Callback::new(move |_: ()| {
        spawn_local(async move {
            if let Ok(v) = invoke_checked("channels_status", JsValue::UNDEFINED).await {
                if let Ok(s) = serde_wasm_bindgen::from_value::<ChannelsStatus>(v) {
                    let _ = feishu_app_id.try_set(s.feishu_app_id.clone());
                    let _ = status.try_set(Some(s));
                }
            }
        });
    });
    refresh.call(());

    let save_feishu = Callback::new(move |enabled: bool| {
        let arg = to_value(&serde_json::json!({
            "enabled": enabled,
            "appId": feishu_app_id.get_untracked().trim(),
            "appSecret": feishu_secret.get_untracked(),
        }))
        .unwrap();
        spawn_local(async move {
            match invoke_checked("set_feishu_channel", arg).await {
                Ok(_) => {
                    let _ = feishu_secret.try_set(String::new());
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.saved").into(),
                    )));
                }
                Err(e) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(e)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let set_weixin_enabled = Callback::new(move |enabled: bool| {
        let arg = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
        spawn_local(async move {
            if let Err(e) = invoke_checked("set_weixin_channel", arg).await {
                let _ = msg.try_set(Some((
                    false,
                    localize_backend(locale.get_untracked(), &js_error_text(e)),
                )));
            }
            refresh.call(());
        });
    });

    let start_bind = Callback::new(move |_: ()| {
        let generation = poll_gen.get_untracked() + 1;
        poll_gen.set(generation);
        msg.set(None);
        spawn_local(async move {
            let bind = match invoke_checked("weixin_bind_start", JsValue::UNDEFINED).await {
                Ok(v) => match serde_wasm_bindgen::from_value::<WeixinBindStart>(v) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = msg.try_set(Some((false, e.to_string())));
                        return;
                    }
                },
                Err(e) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(e)),
                    )));
                    return;
                }
            };
            let qrcode = bind.qrcode.clone();
            let _ = qr.try_set(Some(bind));
            loop {
                sleep_ms(2000).await;
                // try_*: the pane may have unmounted mid-poll.
                match poll_gen.try_get_untracked() {
                    Some(current) if current == generation => {}
                    _ => return,
                }
                let arg = to_value(&serde_json::json!({ "qrcode": qrcode })).unwrap();
                match invoke_checked("weixin_bind_poll", arg).await {
                    Ok(v) => {
                        let state: String = serde_wasm_bindgen::from_value(v).unwrap_or_default();
                        match state.as_str() {
                            "confirmed" => {
                                let _ = qr.try_set(None);
                                let _ = msg.try_set(Some((
                                    true,
                                    t(locale.get_untracked(), "channels.weixin.bound").into(),
                                )));
                                refresh.call(());
                                return;
                            }
                            "expired" => {
                                let _ = qr.try_set(None);
                                let _ = msg.try_set(Some((
                                    false,
                                    t(locale.get_untracked(), "channels.weixin.qr_expired").into(),
                                )));
                                return;
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        let _ = qr.try_set(None);
                        let _ = msg.try_set(Some((
                            false,
                            localize_backend(locale.get_untracked(), &js_error_text(e)),
                        )));
                        return;
                    }
                }
            }
        });
    });

    let unbind = Callback::new(move |_: ()| {
        poll_gen.update(|g| *g += 1);
        qr.set(None);
        spawn_local(async move {
            match invoke_checked("weixin_unbind", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.weixin.unbound").into(),
                    )));
                }
                Err(e) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(e)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    view! {
        <div class="settings-pane channels-pane">
            <div class="channels-routing" data-testid="channel-routing-help">
                <div class="channels-routing-mark" aria-hidden="true">"/"</div>
                <div class="channels-routing-copy">
                    <strong>{move || t(locale.get(), "channels.routing.title")}</strong>
                    <p>{move || t(locale.get(), "channels.routing.desc")}</p>
                    <div class="channels-command-list" aria-label="IM slash commands">
                        <code>"/status"</code>
                        <code>"/project"</code>
                        <code>"/session"</code>
                        <code>"/new"</code>
                    </div>
                </div>
            </div>

            {move || msg.get().map(|(ok, text)| view! {
                <div class="settings-status channels-message" class:ok=move || ok class:fail=move || !ok>{text}</div>
            })}

            <section class="channel-card" data-testid="feishu-channel-card">
                <header class="channel-card-head">
                    <div class="channel-brand">
                        <span class="channel-logo channel-logo-feishu" aria-hidden="true">"飞"</span>
                        <div>
                            <h3>{move || t(locale.get(), "channels.feishu.title")}</h3>
                            <p>{move || t(locale.get(), "channels.feishu.subtitle")}</p>
                        </div>
                    </div>
                    <div class="channel-head-actions">
                        <span class=move || {
                            let state = status.get().unwrap_or_default().feishu_state;
                            format!("channel-state channel-state-{}", state_tone(&state))
                        } data-testid="feishu-state">
                            <i aria-hidden="true"></i>
                            {move || {
                                let s = status.get().unwrap_or_default();
                                state_label(locale.get(), &s.feishu_state)
                            }}
                        </span>
                        <label class="toggle channel-toggle">
                            <input type="checkbox" data-testid="feishu-enabled"
                                aria-label=move || t(locale.get(), "channels.feishu.toggle")
                                prop:checked=move || status.get().map(|s| s.feishu_enabled).unwrap_or(false)
                                on:change=move |ev| save_feishu.call(event_target_checked(&ev)) />
                            <span class="toggle-track" aria-hidden="true"></span>
                        </label>
                    </div>
                </header>

                <div class="channel-card-body">
                    <div class="settings-form-grid channel-fields">
                        <label class="span-2">
                            <span>{move || t(locale.get(), "channels.feishu.app_id")}</span>
                            <input type="text" data-testid="feishu-app-id"
                                placeholder="cli_xxxxxxxx"
                                prop:value=move || feishu_app_id.get()
                                on:input=move |ev| feishu_app_id.set(event_target_input(&ev).value()) />
                        </label>
                        <label class="span-2">
                            <span>{move || {
                                let stored = status.get().map(|s| s.feishu_has_secret).unwrap_or(false);
                                format!("{} · {}", t(locale.get(), "channels.feishu.app_secret"),
                                    if stored { t(locale.get(), "cred.stored") } else { t(locale.get(), "cred.not_stored") })
                            }}</span>
                            <input type="password" data-testid="feishu-app-secret"
                                placeholder=move || {
                                    if status.get().map(|s| s.feishu_has_secret).unwrap_or(false) {
                                        t(locale.get(), "settings.stored_key").to_string()
                                    } else { String::new() }
                                }
                                prop:value=move || feishu_secret.get()
                                on:input=move |ev| feishu_secret.set(event_target_input(&ev).value()) />
                        </label>
                    </div>
                    {move || {
                        let detail = status.get().unwrap_or_default().feishu_detail;
                        (!detail.is_empty()).then(|| view! { <p class="channel-detail">{detail}</p> })
                    }}
                    <div class="channel-guide">
                        <span aria-hidden="true">"i"</span>
                        <p>{move || t(locale.get(), "channels.feishu.hint")}</p>
                    </div>
                </div>

                <footer class="channel-card-foot">
                    <span>{move || t(locale.get(), "channels.secret_note")}</span>
                    <button type="button" class="primary" data-testid="feishu-save"
                        on:click=move |_| save_feishu.call(status.get_untracked().map(|s| s.feishu_enabled).unwrap_or(false))>
                        {move || t(locale.get(), "settings.save")}
                    </button>
                </footer>
            </section>

            <section class="channel-card" data-testid="weixin-channel-card">
                <header class="channel-card-head">
                    <div class="channel-brand">
                        <span class="channel-logo channel-logo-weixin" aria-hidden="true">"微"</span>
                        <div>
                            <h3>{move || t(locale.get(), "channels.weixin.title")}</h3>
                            <p>{move || t(locale.get(), "channels.weixin.subtitle")}</p>
                        </div>
                    </div>
                    <div class="channel-head-actions">
                        <span class=move || {
                            let s = status.get().unwrap_or_default();
                            let state = if s.weixin_bound { s.weixin_state.as_str() } else { "stopped" };
                            format!("channel-state channel-state-{}", state_tone(state))
                        } data-testid="weixin-state">
                            <i aria-hidden="true"></i>
                            {move || {
                                let s = status.get().unwrap_or_default();
                                if s.weixin_bound {
                                    state_label(locale.get(), &s.weixin_state)
                                } else {
                                    t(locale.get(), "channels.weixin.not_bound").to_string()
                                }
                            }}
                        </span>
                        <label class="toggle channel-toggle">
                            <input type="checkbox" data-testid="weixin-enabled"
                                aria-label=move || t(locale.get(), "channels.weixin.toggle")
                                prop:disabled=move || !status.get().map(|s| s.weixin_bound).unwrap_or(false)
                                prop:checked=move || status.get().map(|s| s.weixin_enabled).unwrap_or(false)
                                on:change=move |ev| set_weixin_enabled.call(event_target_checked(&ev)) />
                            <span class="toggle-track" aria-hidden="true"></span>
                        </label>
                    </div>
                </header>

                <div class="channel-card-body channel-weixin-body">
                    <div class="channel-bind-row">
                        <div>
                            <strong>{move || {
                                if status.get().map(|s| s.weixin_bound).unwrap_or(false) {
                                    t(locale.get(), "channels.weixin.bound_account")
                                } else {
                                    t(locale.get(), "channels.weixin.scan_title")
                                }
                            }}</strong>
                            <p>{move || {
                                let s = status.get().unwrap_or_default();
                                if !s.weixin_detail.is_empty() {
                                    s.weixin_detail
                                } else {
                                    t(locale.get(), "channels.weixin.hint").to_string()
                                }
                            }}</p>
                        </div>
                        {move || {
                            let bound = status.get().map(|s| s.weixin_bound).unwrap_or(false);
                            if bound {
                                view! {
                                    <button type="button" class="channel-secondary" data-testid="weixin-unbind"
                                        on:click=move |_| unbind.call(())>
                                        {move || t(locale.get(), "channels.weixin.unbind")}
                                    </button>
                                }.into_view()
                            } else {
                                view! {
                                    <button type="button" class="primary" data-testid="weixin-bind"
                                        on:click=move |_| start_bind.call(())>
                                        {move || t(locale.get(), "channels.weixin.bind")}
                                    </button>
                                }.into_view()
                            }
                        }}
                    </div>
                    {move || qr.get().map(|bind| view! {
                        <div class="channels-qr" data-testid="weixin-qr">
                            <div class="channels-qr-frame">
                                <img src=bind.qr_image alt="WeChat QR" />
                            </div>
                            <div>
                                <strong>{move || t(locale.get(), "channels.weixin.qr_title")}</strong>
                                <p>{move || t(locale.get(), "channels.weixin.qr_hint")}</p>
                            </div>
                        </div>
                    })}
                </div>
            </section>
        </div>
    }
}
