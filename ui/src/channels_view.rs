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
        <div class="settings-pane">
            <p class="settings-note">{move || t(locale.get(), "channels.desc")}</p>

            <div class="conn-group-label">{move || t(locale.get(), "channels.feishu.title")}</div>
            <div class="settings-form-grid">
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
                        format!("{} — {}", t(locale.get(), "channels.feishu.app_secret"),
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
            <div class="row">
                <label class="toggle">
                    <input type="checkbox" data-testid="feishu-enabled"
                        prop:checked=move || status.get().map(|s| s.feishu_enabled).unwrap_or(false)
                        on:change=move |ev| save_feishu.call(event_target_checked(&ev)) />
                    <span class="toggle-track" aria-hidden="true"></span>
                </label>
                <span class="settings-note">{move || {
                    let s = status.get().unwrap_or_default();
                    let label = state_label(locale.get(), &s.feishu_state);
                    if s.feishu_detail.is_empty() { label } else { format!("{label} · {}", s.feishu_detail) }
                }}</span>
                <button type="button" class="primary" data-testid="feishu-save"
                    on:click=move |_| save_feishu.call(status.get_untracked().map(|s| s.feishu_enabled).unwrap_or(false))>
                    {move || t(locale.get(), "settings.save")}</button>
            </div>
            <p class="settings-note">{move || t(locale.get(), "channels.feishu.hint")}</p>

            <div class="conn-group-label">{move || t(locale.get(), "channels.weixin.title")}</div>
            <div class="row">
                <label class="toggle">
                    <input type="checkbox" data-testid="weixin-enabled"
                        prop:checked=move || status.get().map(|s| s.weixin_enabled).unwrap_or(false)
                        on:change=move |ev| set_weixin_enabled.call(event_target_checked(&ev)) />
                    <span class="toggle-track" aria-hidden="true"></span>
                </label>
                <span class="settings-note">{move || {
                    let s = status.get().unwrap_or_default();
                    if !s.weixin_bound {
                        t(locale.get(), "channels.weixin.not_bound").to_string()
                    } else {
                        let label = state_label(locale.get(), &s.weixin_state);
                        if s.weixin_detail.is_empty() { label } else { format!("{label} · {}", s.weixin_detail) }
                    }
                }}</span>
                {move || {
                    let bound = status.get().map(|s| s.weixin_bound).unwrap_or(false);
                    if bound {
                        view! {
                            <button type="button" data-testid="weixin-unbind"
                                on:click=move |_| unbind.call(())>
                                {move || t(locale.get(), "channels.weixin.unbind")}</button>
                        }.into_view()
                    } else {
                        view! {
                            <button type="button" class="primary" data-testid="weixin-bind"
                                on:click=move |_| start_bind.call(())>
                                {move || t(locale.get(), "channels.weixin.bind")}</button>
                        }.into_view()
                    }
                }}
            </div>
            {move || qr.get().map(|bind| view! {
                <div class="channels-qr" data-testid="weixin-qr">
                    <img src=bind.qr_image alt="WeChat QR" style="width:220px;height:220px;" />
                    <p class="settings-note">{move || t(locale.get(), "channels.weixin.qr_hint")}</p>
                </div>
            })}
            <p class="settings-note">{move || t(locale.get(), "channels.weixin.hint")}</p>

            {move || msg.get().map(|(ok, text)| view! {
                <div class="settings-status" class:ok=move || ok class:fail=move || !ok>{text}</div>
            })}
        </div>
    }
}
