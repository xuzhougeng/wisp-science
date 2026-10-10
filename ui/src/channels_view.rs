//! Settings → Remote Access pane: a bot list (Feishu, WeChat iLink) with
//! per-bot subpages, mirroring the Connections list → detail pattern. The
//! `open` signal is hoisted so the shared settings breadcrumb can render it;
//! everything else is self-contained and fetched on mount.

use crate::app_support::{copy_text, js_error_text};
use crate::bindings::{invoke_checked, listen};
use crate::dto::{ChannelsStatus, FeishuBindPoll, FeishuBindStart, WeixinBindStart};
use crate::i18n::{localize_backend, t, Locale};
use crate::text::{event_target_checked, event_target_input};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::net::Ipv4Addr;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

const DEFAULT_DEVICE_BRIDGE_PORT: u16 = 18_766;

fn validate_device_fields(bind_ipv4: &str, port: &str) -> Result<(String, u16), &'static str> {
    let address = bind_ipv4
        .trim()
        .parse::<Ipv4Addr>()
        .map_err(|_| "channels.device.invalid_ipv4")?;
    if address.is_unspecified() {
        return Err("channels.device.unspecified_ipv4");
    }
    let port = port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or("channels.device.invalid_port")?;
    Ok((address.to_string(), port))
}

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
        "listening" => t(locale, "channels.device.state.listening"),
        "connecting" => t(locale, "channels.state.connecting"),
        "error" => t(locale, "channels.state.error"),
        _ => t(locale, "channels.state.stopped"),
    }
    .into()
}

fn state_tone(state: &str) -> &'static str {
    match state {
        "running" | "listening" => "running",
        "connecting" => "connecting",
        "error" => "error",
        _ => "stopped",
    }
}

#[component]
pub(super) fn ChannelsPane(
    locale: RwSignal<Locale>,
    open: RwSignal<Option<String>>,
    /// Opens a project and leaves Settings, after the lab's knowledge base
    /// was downloaded.
    on_open_project: Callback<String>,
    /// Reloads the project sync settings after the lab filled them in.
    on_sync_settings_changed: Callback<()>,
) -> impl IntoView {
    let status = create_rw_signal(None::<ChannelsStatus>);
    let feishu_app_id = create_rw_signal(String::new());
    let feishu_secret = create_rw_signal(String::new());
    let feishu_owner_id = create_rw_signal(String::new());
    let feishu_international = create_rw_signal(false);
    let feishu_qr = create_rw_signal(None::<FeishuBindStart>);
    let feishu_bind_state = create_rw_signal(String::new());
    let feishu_poll_gen = create_rw_signal(0usize);
    let msg = create_rw_signal(None::<(bool, String)>);
    let qr = create_rw_signal(None::<WeixinBindStart>);
    let device_bind_ipv4 = create_rw_signal(String::new());
    let device_port = create_rw_signal(DEFAULT_DEVICE_BRIDGE_PORT.to_string());
    // Bumped to cancel a stale QR poll loop (new scan, unbind, unmount race).
    let poll_gen = create_rw_signal(0usize);
    let remote_url = create_rw_signal(String::new());
    let remote_token = create_rw_signal(String::new());
    // Status events fire on every browser connect; load the URL field once.
    let remote_loaded = create_rw_signal(false);

    let refresh = Callback::new(move |_: ()| {
        spawn_local(async move {
            if let Ok(v) = invoke_checked("channels_status", JsValue::UNDEFINED).await {
                if let Ok(s) = serde_wasm_bindgen::from_value::<ChannelsStatus>(v) {
                    let _ = feishu_app_id.try_set(s.feishu_app_id.clone());
                    let _ = feishu_international.try_set(s.feishu_international);
                    let _ = device_bind_ipv4.try_set(s.device.bind_ipv4.clone());
                    let _ = device_port.try_set(s.device.port.to_string());
                    if remote_loaded.try_get_untracked() == Some(false) {
                        let _ = remote_url.try_set(s.remote.relay_url.clone());
                        let _ = remote_loaded.try_set(true);
                    }
                    let _ = status.try_set(Some(s));
                }
            }
        });
    });
    refresh.call(());
    {
        let refresh = refresh;
        spawn_local(async move {
            let callback = Closure::wrap(Box::new(move |_event: JsValue| {
                refresh.call(());
            }) as Box<dyn FnMut(JsValue)>);
            let _ = listen("channels-updated", callback.as_ref().unchecked_ref()).await;
            callback.forget();
        });
    }

    let save_feishu = Callback::new(move |enabled: bool| {
        let arg = to_value(&serde_json::json!({
            "enabled": enabled,
            "international": feishu_international.get_untracked(),
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

    let start_feishu_bind = Callback::new(move |_: ()| {
        let generation = feishu_poll_gen.get_untracked() + 1;
        feishu_poll_gen.set(generation);
        let previous_flow_id = feishu_qr.get_untracked().map(|bind| bind.flow_id);
        feishu_qr.set(None);
        feishu_bind_state.set("requesting".into());
        msg.set(None);
        spawn_local(async move {
            if let Some(flow_id) = previous_flow_id {
                let cancel_arg = to_value(&serde_json::json!({ "flowId": flow_id })).unwrap();
                let _ = invoke_checked("feishu_bind_cancel", cancel_arg).await;
            }
            let arg = to_value(&serde_json::json!({
                "international": feishu_international.get_untracked(),
            }))
            .unwrap();
            let bind = match invoke_checked("feishu_bind_start", arg).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<FeishuBindStart>(value) {
                    Ok(bind) => bind,
                    Err(error) => {
                        let _ = feishu_bind_state.try_set("error".into());
                        let _ = msg.try_set(Some((false, error.to_string())));
                        return;
                    }
                },
                Err(error) => {
                    let _ = feishu_bind_state.try_set("error".into());
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                    return;
                }
            };
            let flow_id = bind.flow_id.clone();
            let _ = feishu_qr.try_set(Some(bind));
            let _ = feishu_bind_state.try_set("waiting".into());
            let mut retry_after_ms = 0_u64;
            loop {
                if retry_after_ms > 0 {
                    sleep_ms(retry_after_ms.min(i32::MAX as u64) as i32).await;
                }
                match feishu_poll_gen.try_get_untracked() {
                    Some(current) if current == generation => {}
                    _ => return,
                }
                let arg = to_value(&serde_json::json!({ "flowId": flow_id })).unwrap();
                let poll = match invoke_checked("feishu_bind_poll", arg).await {
                    Ok(value) => match serde_wasm_bindgen::from_value::<FeishuBindPoll>(value) {
                        Ok(poll) => poll,
                        Err(error) => {
                            let _ = feishu_bind_state.try_set("error".into());
                            let _ = msg.try_set(Some((false, error.to_string())));
                            return;
                        }
                    },
                    Err(error) => {
                        let _ = feishu_bind_state.try_set("error".into());
                        let _ = msg.try_set(Some((
                            false,
                            localize_backend(locale.get_untracked(), &js_error_text(error)),
                        )));
                        return;
                    }
                };
                match poll.state.as_str() {
                    "confirmed" => {
                        let _ = feishu_app_id.try_set(poll.app_id);
                        let _ = feishu_qr.try_set(None);
                        let _ = feishu_bind_state.try_set("confirmed".into());
                        let _ = msg.try_set(Some((
                            true,
                            t(locale.get_untracked(), "channels.feishu.bound").into(),
                        )));
                        refresh.call(());
                        return;
                    }
                    "denied" => {
                        let _ = feishu_bind_state.try_set("denied".into());
                        let _ = msg.try_set(Some((
                            false,
                            t(locale.get_untracked(), "channels.feishu.denied").into(),
                        )));
                        return;
                    }
                    "expired" => {
                        let _ = feishu_bind_state.try_set("expired".into());
                        let _ = msg.try_set(Some((
                            false,
                            t(locale.get_untracked(), "channels.feishu.qr_expired").into(),
                        )));
                        return;
                    }
                    _ => retry_after_ms = poll.retry_after_ms.max(500),
                }
            }
        });
    });

    let cancel_feishu_bind = Callback::new(move |_: ()| {
        feishu_poll_gen.update(|generation| *generation += 1);
        let flow_id = feishu_qr.get_untracked().map(|bind| bind.flow_id);
        feishu_qr.set(None);
        feishu_bind_state.set(String::new());
        if let Some(flow_id) = flow_id {
            spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "flowId": flow_id })).unwrap();
                let _ = invoke_checked("feishu_bind_cancel", arg).await;
            });
        }
    });

    let unbind_feishu = Callback::new(move |_: ()| {
        feishu_poll_gen.update(|generation| *generation += 1);
        feishu_qr.set(None);
        spawn_local(async move {
            match invoke_checked("feishu_unbind", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = feishu_app_id.try_set(String::new());
                    let _ = feishu_secret.try_set(String::new());
                    let _ = feishu_owner_id.try_set(String::new());
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.feishu.unbound").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let save_feishu_owner = Callback::new(move |_: ()| {
        let arg = to_value(&serde_json::json!({
            "openId": feishu_owner_id.get_untracked().trim(),
        }))
        .unwrap();
        spawn_local(async move {
            match invoke_checked("set_feishu_owner", arg).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.feishu.owner_saved").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let clear_feishu_owner = Callback::new(move |_: ()| {
        let arg = to_value(&serde_json::json!({ "openId": "" })).unwrap();
        spawn_local(async move {
            match invoke_checked("set_feishu_owner", arg).await {
                Ok(_) => {
                    let _ = feishu_owner_id.try_set(String::new());
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.feishu.owner_cleared").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let confirm_feishu_pending = Callback::new(move |_: ()| {
        spawn_local(async move {
            match invoke_checked("confirm_feishu_pending_owner", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.feishu.owner_saved").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let reject_feishu_pending = Callback::new(move |_: ()| {
        spawn_local(async move {
            if let Err(error) =
                invoke_checked("reject_feishu_pending_owner", JsValue::UNDEFINED).await
            {
                let _ = msg.try_set(Some((
                    false,
                    localize_backend(locale.get_untracked(), &js_error_text(error)),
                )));
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

    let save_device = Callback::new(move |enabled: bool| {
        let (bind_ipv4, port) = match validate_device_fields(
            &device_bind_ipv4.get_untracked(),
            &device_port.get_untracked(),
        ) {
            Ok(fields) => fields,
            Err(key) => {
                msg.set(Some((false, t(locale.get_untracked(), key).into())));
                return;
            }
        };
        let arg = to_value(&serde_json::json!({
            "enabled": enabled,
            "mode": "lan",
            "bindIpv4": bind_ipv4,
            "port": port,
        }))
        .unwrap();
        spawn_local(async move {
            match invoke_checked("set_device_bridge", arg).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.device.saved").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let save_remote = Callback::new(move |enabled: bool| {
        let arg = to_value(&serde_json::json!({
            "enabled": enabled,
            "relayUrl": remote_url.get_untracked().trim(),
            "relayToken": remote_token.get_untracked(),
        }))
        .unwrap();
        spawn_local(async move {
            match invoke_checked("set_remote_access", arg).await {
                Ok(_) => {
                    let _ = remote_token.try_set(String::new());
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.saved").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let reset_remote_code = Callback::new(move |_: ()| {
        spawn_local(async move {
            match invoke_checked("reset_remote_access_code", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.remote.reset_done").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let copy_remote = move |text: Option<String>| {
        if let Some(text) = text {
            copy_text(text);
            msg.set(Some((true, t(locale.get_untracked(), "channels.remote.copied").into())));
        }
    };

    let rotate_device_token = Callback::new(move |_: ()| {
        spawn_local(async move {
            match invoke_checked("rotate_device_bridge_token", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.device.token_rotated").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let copy_device_token = Callback::new(move |_: ()| {
        spawn_local(async move {
            match invoke_checked("get_device_bridge_token", JsValue::UNDEFINED).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<String>(value) {
                    Ok(token) => {
                        copy_text(token);
                        let _ = msg.try_set(Some((
                            true,
                            t(locale.get_untracked(), "channels.device.token_copied").into(),
                        )));
                    }
                    Err(error) => {
                        let _ = msg.try_set(Some((false, error.to_string())));
                    }
                },
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
        });
    });

    let revoke_device_token = Callback::new(move |_: ()| {
        spawn_local(async move {
            match invoke_checked("revoke_device_bridge_token", JsValue::UNDEFINED).await {
                Ok(_) => {
                    let _ = msg.try_set(Some((
                        true,
                        t(locale.get_untracked(), "channels.device.token_revoked").into(),
                    )));
                }
                Err(error) => {
                    let _ = msg.try_set(Some((
                        false,
                        localize_backend(locale.get_untracked(), &js_error_text(error)),
                    )));
                }
            }
            refresh.call(());
        });
    });

    let msg_view =
        move || {
            msg.get().map(|(ok, text)| view! {
            <div class="settings-status" class:ok=move || ok class:fail=move || !ok>{text}</div>
        })
        };
    let feishu_badge = move || {
        let s = status.get().unwrap_or_default();
        let state = if s.feishu_bound {
            s.feishu_state.clone()
        } else {
            "stopped".into()
        };
        view! {
            <span class=format!("badge channel-state-{}", state_tone(&state)) data-testid="feishu-state">
                {if s.feishu_bound {
                    state_label(locale.get(), &state)
                } else {
                    t(locale.get(), "channels.feishu.not_bound").to_string()
                }}
            </span>
        }
    };
    let weixin_badge = move || {
        let s = status.get().unwrap_or_default();
        let state = if s.weixin_bound {
            s.weixin_state.clone()
        } else {
            "stopped".into()
        };
        view! {
            <span class=format!("badge channel-state-{}", state_tone(&state)) data-testid="weixin-state">
                {if s.weixin_bound {
                    state_label(locale.get(), &state)
                } else {
                    t(locale.get(), "channels.weixin.not_bound").to_string()
                }}
            </span>
        }
    };
    let device_badge = move || {
        let s = status.get().unwrap_or_default().device;
        let state = if s.enabled { s.state } else { "stopped".into() };
        view! {
            <span class=format!("badge channel-state-{}", state_tone(&state)) data-testid="sticks3-state">
                {state_label(locale.get(), &state)}
            </span>
        }
    };
    let remote_badge = move || {
        let s = status.get().unwrap_or_default().remote;
        let state = if s.enabled { s.state } else { "stopped".into() };
        view! {
            <span class=format!("badge channel-state-{}", state_tone(&state)) data-testid="remote-state">
                {state_label(locale.get(), &state)}
            </span>
        }
    };
    let feishu_region = move || {
        let s = status.get().unwrap_or_default();
        format!(
            "{} · {}",
            s.feishu_app_id,
            if s.feishu_international {
                t(locale.get(), "channels.feishu.region_lark")
            } else {
                t(locale.get(), "channels.feishu.region_cn")
            }
        )
    };

    move || {
        match open.get().as_deref() {
        Some("feishu") => view! {
            <div class="settings-pane settings-pane-subpage" data-testid="feishu-channel-card">
                {msg_view}
                <div class="channel-bind-row">
                    <div>
                        <strong>{move || {
                            if status.get().map(|s| s.feishu_bound).unwrap_or(false) {
                                t(locale.get(), "channels.feishu.bound_app")
                            } else {
                                t(locale.get(), "channels.feishu.scan_title")
                            }
                        }}</strong>
                        <p>{move || {
                            if status.get().map(|s| s.feishu_bound).unwrap_or(false) {
                                feishu_region()
                            } else {
                                t(locale.get(), "channels.feishu.scan_hint").to_string()
                            }
                        }}</p>
                    </div>
                    <div class="row channel-bind-actions">
                        {move || {
                            if feishu_qr.get().is_some() {
                                view! {
                                    <button type="button" data-testid="feishu-bind-cancel"
                                        on:click=move |_| cancel_feishu_bind.call(())>
                                        {move || t(locale.get(), "settings.cancel")}
                                    </button>
                                }.into_view()
                            } else {
                                view! {
                                    <button type="button" class="primary" data-testid="feishu-bind"
                                        on:click=move |_| start_feishu_bind.call(())>
                                        {move || {
                                            if status.get().map(|s| s.feishu_bound).unwrap_or(false) {
                                                t(locale.get(), "channels.feishu.recreate")
                                            } else {
                                                t(locale.get(), "channels.feishu.bind")
                                            }
                                        }}
                                    </button>
                                }.into_view()
                            }
                        }}
                        {move || status.get().map(|s| s.feishu_bound).unwrap_or(false).then(|| view! {
                            <button type="button" data-testid="feishu-unbind"
                                on:click=move |_| unbind_feishu.call(())>
                                {move || t(locale.get(), "channels.feishu.unbind")}
                            </button>
                        })}
                    </div>
                </div>

                {move || feishu_qr.get().map(|bind| {
                    let terminal = matches!(feishu_bind_state.get().as_str(), "expired" | "denied" | "error");
                    let qr_image = bind.qr_image.clone();
                    let expires_in_seconds = bind.expires_in_seconds;
                    view! {
                        <div class="channels-qr" class:terminal=terminal data-testid="feishu-qr">
                            <div class="channels-qr-frame">
                                <img src=qr_image alt="Feishu app registration QR" />
                            </div>
                            <div class="channels-qr-copy">
                                <strong>{move || match feishu_bind_state.get().as_str() {
                                    "expired" => t(locale.get(), "channels.feishu.qr_expired"),
                                    "denied" => t(locale.get(), "channels.feishu.denied"),
                                    "error" => t(locale.get(), "channels.state.error"),
                                    _ => t(locale.get(), "channels.feishu.qr_title"),
                                }}</strong>
                                <p>{move || {
                                    if terminal {
                                        t(locale.get(), "channels.feishu.retry_hint").to_string()
                                    } else {
                                        format!(
                                            "{} · {} {}",
                                            t(locale.get(), "channels.feishu.qr_hint"),
                                            expires_in_seconds / 60,
                                            t(locale.get(), "channels.feishu.minutes")
                                        )
                                    }
                                }}</p>
                                {terminal.then(|| view! {
                                    <div class="row channels-qr-retry">
                                        <button type="button" data-testid="feishu-bind-retry"
                                            on:click=move |_| start_feishu_bind.call(())>
                                            {move || t(locale.get(), "channels.feishu.retry")}
                                        </button>
                                    </div>
                                })}
                            </div>
                        </div>
                    }
                })}

                <div class="channel-divider">
                    <span>{move || t(locale.get(), "channels.feishu.manual")}</span>
                </div>
                <div class="settings-form-grid">
                    <label class="span-2 settings-check">
                        <input type="checkbox" data-testid="feishu-international"
                            prop:disabled=move || feishu_qr.get().is_some()
                            prop:checked=move || feishu_international.get()
                            on:change=move |ev| feishu_international.set(event_target_checked(&ev)) />
                        <span>{move || t(locale.get(), "channels.feishu.international")}</span>
                    </label>
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
                <div class="channel-divider">
                    <span>{move || t(locale.get(), "channels.feishu.owner_title")}</span>
                </div>
                {move || {
                    let pending = status.get().unwrap_or_default().feishu_pending_owner_open_id;
                    let loc = locale.get();
                    (!pending.is_empty()).then(move || {
                        let pending_label = pending.clone();
                        view! {
                            <div class="channel-bind-row" data-testid="feishu-pending-owner">
                                <div>
                                    <strong>{t(loc, "channels.feishu.pending_title")}</strong>
                                    <p>{format!("{} ({pending_label})", t(loc, "channels.feishu.pending_hint"))}</p>
                                </div>
                                <div class="row channel-bind-actions">
                                    <button type="button" class="primary" data-testid="feishu-pending-confirm"
                                        on:click=move |_| confirm_feishu_pending.call(())>
                                        {t(loc, "channels.feishu.pending_confirm")}
                                    </button>
                                    <button type="button" data-testid="feishu-pending-reject"
                                        on:click=move |_| reject_feishu_pending.call(())>
                                        {t(loc, "channels.feishu.pending_reject")}
                                    </button>
                                </div>
                            </div>
                        }
                    })
                }}
                <div class="channel-bind-row">
                    <div>
                        <strong data-testid="feishu-owner-status">{move || {
                            let owner = status.get().unwrap_or_default().feishu_owner_open_id;
                            if owner.is_empty() {
                                t(locale.get(), "channels.feishu.owner_not_bound").to_string()
                            } else {
                                format!("{} · {owner}", t(locale.get(), "channels.feishu.owner_bound"))
                            }
                        }}</strong>
                        <p>{move || t(locale.get(), "channels.feishu.owner_hint")}</p>
                    </div>
                    {move || status.get().map(|s| !s.feishu_owner_open_id.is_empty()).unwrap_or(false).then(|| view! {
                        <button type="button" data-testid="feishu-owner-clear"
                            on:click=move |_| clear_feishu_owner.call(())>
                            {move || t(locale.get(), "channels.feishu.owner_clear")}
                        </button>
                    })}
                </div>
                <div class="settings-form-grid">
                    <label class="span-2">
                        <span>{move || t(locale.get(), "channels.feishu.owner_open_id")}</span>
                        <input type="text" data-testid="feishu-owner-id"
                            placeholder="ou_xxxxxxxx"
                            prop:value=move || feishu_owner_id.get()
                            on:input=move |ev| feishu_owner_id.set(event_target_input(&ev).value()) />
                    </label>
                </div>
                <div class="row" style="justify-content:flex-start;">
                    <button type="button" data-testid="feishu-owner-save"
                        on:click=move |_| save_feishu_owner.call(())>
                        {move || t(locale.get(), "channels.feishu.owner_save")}
                    </button>
                </div>
                {move || {
                    let detail = status.get().unwrap_or_default().feishu_detail;
                    (!detail.is_empty()).then(|| view! { <p class="settings-field-hint">{detail}</p> })
                }}
                <p class="settings-field-hint">{move || t(locale.get(), "channels.feishu.hint")}</p>
                <div class="row settings-footer">
                    <span class="settings-footer-note">{move || t(locale.get(), "channels.secret_note")}</span>
                    <button type="button" class="primary" data-testid="feishu-save"
                        on:click=move |_| save_feishu.call(status.get_untracked().map(|s| s.feishu_enabled).unwrap_or(false))>
                        {move || t(locale.get(), "settings.save")}
                    </button>
                </div>
            </div>
        }.into_view(),
        Some("weixin") => view! {
            <div class="settings-pane settings-pane-subpage" data-testid="weixin-channel-card">
                {msg_view}
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
                    <div class="row channel-bind-actions">
                        {move || {
                            let bound = status.get().map(|s| s.weixin_bound).unwrap_or(false);
                            if bound {
                                view! {
                                    <button type="button" data-testid="weixin-unbind"
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
                </div>
                {move || qr.get().map(|bind| view! {
                    <div class="channels-qr" data-testid="weixin-qr">
                        <div class="channels-qr-frame">
                            <img src=bind.qr_image alt="WeChat QR" />
                        </div>
                        <div class="channels-qr-copy">
                            <strong>{move || t(locale.get(), "channels.weixin.qr_title")}</strong>
                            <p>{move || t(locale.get(), "channels.weixin.qr_hint")}</p>
                        </div>
                    </div>
                })}
            </div>
        }.into_view(),
        Some("sticks3") => view! {
            <div class="settings-pane settings-pane-subpage" data-testid="sticks3-channel-card">
                {msg_view}
                <div class="channel-bind-row">
                    <div>
                        <strong>{move || t(locale.get(), "channels.device.heading")}</strong>
                        <p>{move || t(locale.get(), "channels.device.hint")}</p>
                    </div>
                    <span class=move || {
                        let state = status.get().unwrap_or_default().device.state;
                        format!("badge channel-state-{}", state_tone(&state))
                    } data-testid="sticks3-detail-state">
                        {move || state_label(locale.get(), &status.get().unwrap_or_default().device.state)}
                    </span>
                </div>

                <div class="device-mode-grid" role="radiogroup" aria-label=move || t(locale.get(), "channels.device.mode")>
                    <label class="device-mode-option selected">
                        <input type="radio" name="device-bridge-mode" value="lan"
                            prop:checked=move || status.get().unwrap_or_default().device.mode != "relay" />
                        <span>
                            <strong>{move || t(locale.get(), "channels.device.mode_lan")}</strong>
                            <small>{move || t(locale.get(), "channels.device.mode_lan_hint")}</small>
                        </span>
                    </label>
                    <label class="device-mode-option disabled">
                        <input type="radio" name="device-bridge-mode" value="relay" disabled=true
                            prop:checked=move || status.get().unwrap_or_default().device.mode == "relay" />
                        <span>
                            <strong>
                                {move || t(locale.get(), "channels.device.mode_relay")}
                                " · "
                                {move || t(locale.get(), "channels.device.planned")}
                            </strong>
                            <small>{move || t(locale.get(), "channels.device.mode_relay_hint")}</small>
                        </span>
                    </label>
                </div>

                <div class="settings-form-grid">
                    <label class="span-2 settings-check">
                        <input type="checkbox" data-testid="sticks3-enabled-detail"
                            prop:checked=move || status.get().unwrap_or_default().device.enabled
                            on:change=move |ev| save_device.call(event_target_checked(&ev)) />
                        <span>{move || t(locale.get(), "channels.device.toggle")}</span>
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "channels.device.bind_ipv4")}</span>
                        <input type="text" inputmode="decimal" data-testid="sticks3-bind-ipv4"
                            placeholder="10.10.87.103"
                            prop:value=move || device_bind_ipv4.get()
                            on:input=move |ev| device_bind_ipv4.set(event_target_input(&ev).value()) />
                    </label>
                    <label>
                        <span>{move || t(locale.get(), "channels.device.port")}</span>
                        <input type="number" min="1" max="65535" data-testid="sticks3-port"
                            prop:value=move || device_port.get()
                            on:input=move |ev| device_port.set(event_target_input(&ev).value()) />
                    </label>
                </div>
                {move || {
                    let device = status.get().unwrap_or_default().device;
                    device.url.map(|url| view! {
                        <p class="settings-field-hint" data-testid="sticks3-listening-url">
                            {format!("{}: {url}", t(locale.get(), "channels.device.current_url"))}
                        </p>
                    })
                }}
                {move || {
                    let detail = status.get().unwrap_or_default().device.detail;
                    (!detail.is_empty()).then(|| view! {
                        <p class="settings-field-error" data-testid="sticks3-runtime-error">{detail}</p>
                    })
                }}
                <p class="settings-field-hint">{move || t(locale.get(), "channels.device.bind_hint")}</p>

                <div class="device-token-row">
                    <div>
                        <strong>{move || t(locale.get(), "channels.device.token")}</strong>
                        <p>{move || {
                            if status.get().unwrap_or_default().device.has_token {
                                t(locale.get(), "channels.device.token_stored")
                            } else {
                                t(locale.get(), "channels.device.token_missing")
                            }
                        }}</p>
                    </div>
                    <div class="row">
                        <button type="button" data-testid="sticks3-rotate-token"
                            on:click=move |_| rotate_device_token.call(())>
                            {move || {
                                if status.get().unwrap_or_default().device.has_token {
                                    t(locale.get(), "channels.device.rotate_token")
                                } else {
                                    t(locale.get(), "channels.device.generate_token")
                                }
                            }}
                        </button>
                        <button type="button" data-testid="sticks3-copy-token"
                            prop:disabled=move || !status.get().unwrap_or_default().device.has_token
                            on:click=move |_| copy_device_token.call(())>
                            {move || t(locale.get(), "channels.device.copy_token")}
                        </button>
                        <button type="button" class="danger" data-testid="sticks3-revoke-token"
                            prop:disabled=move || !status.get().unwrap_or_default().device.has_token
                            on:click=move |_| revoke_device_token.call(())>
                            {move || t(locale.get(), "channels.device.revoke_token")}
                        </button>
                    </div>
                </div>

                <p class="settings-note device-security-note">{move || t(locale.get(), "channels.device.security")}</p>
                <div class="row settings-footer">
                    <span class="settings-footer-note">{move || t(locale.get(), "channels.secret_note")}</span>
                    <button type="button" class="primary" data-testid="sticks3-save"
                        on:click=move |_| save_device.call(status.get_untracked().unwrap_or_default().device.enabled)>
                        {move || t(locale.get(), "settings.save")}
                    </button>
                </div>
            </div>
        }.into_view(),
        Some("remote") => view! {
            <div class="settings-pane settings-pane-subpage" data-testid="remote-channel-card">
                {msg_view}
                <div class="channel-bind-row">
                    <div>
                        <strong>{move || t(locale.get(), "channels.remote.heading")}</strong>
                        <p>{move || t(locale.get(), "channels.remote.hint")}</p>
                    </div>
                    <span class=move || {
                        let s = status.get().unwrap_or_default().remote;
                        let state = if s.enabled { s.state } else { "stopped".into() };
                        format!("badge channel-state-{}", state_tone(&state))
                    } data-testid="remote-detail-state">
                        {move || {
                            let s = status.get().unwrap_or_default().remote;
                            state_label(locale.get(), if s.enabled { &s.state } else { "stopped" })
                        }}
                    </span>
                </div>

                <div class="settings-form-grid">
                    <label class="span-2 settings-check">
                        <input type="checkbox" data-testid="remote-enabled-detail"
                            prop:checked=move || status.get().unwrap_or_default().remote.enabled
                            on:change=move |ev| save_remote.call(event_target_checked(&ev)) />
                        <span>{move || t(locale.get(), "channels.remote.toggle")}</span>
                    </label>
                    <label class="span-2">
                        <span>{move || t(locale.get(), "channels.remote.relay_url")}</span>
                        <input type="url" data-testid="remote-relay-url"
                            placeholder="https://relay.example.com"
                            prop:value=move || remote_url.get()
                            on:input=move |ev| remote_url.set(event_target_input(&ev).value()) />
                    </label>
                    <label class="span-2">
                        <span>{move || t(locale.get(), "channels.remote.token")}</span>
                        <input type="password" data-testid="remote-relay-token"
                            prop:value=move || remote_token.get()
                            placeholder=move || if status.get().unwrap_or_default().remote.has_token {
                                t(locale.get(), "settings.key_stored")
                            } else {
                                t(locale.get(), "channels.remote.token_placeholder")
                            }
                            on:input=move |ev| remote_token.set(event_target_input(&ev).value()) />
                    </label>
                </div>
                {move || {
                    let s = status.get().unwrap_or_default().remote;
                    (s.enabled && !s.detail.is_empty()).then(|| view! {
                        <p class="settings-field-error" data-testid="remote-runtime-error">
                            {localize_backend(locale.get(), &s.detail)}
                        </p>
                    })
                }}
                {move || {
                    let s = status.get().unwrap_or_default().remote;
                    s.code.clone().map(|code| {
                        let link = s.link.clone();
                        let shown_link = link.clone().unwrap_or_default();
                        let copy_link = link.clone();
                        view! {
                            <div class="device-token-row">
                                <div>
                                    <strong>{move || t(locale.get(), "channels.remote.code")}</strong>
                                    <p><code data-testid="remote-code">{code.clone()}</code></p>
                                    <p class="settings-field-hint" data-testid="remote-link">{shown_link}</p>
                                </div>
                                <div class="row">
                                    <button type="button" data-testid="remote-copy-link"
                                        prop:disabled=link.is_none()
                                        on:click=move |_| copy_remote(copy_link.clone())>
                                        {move || t(locale.get(), "channels.remote.copy_link")}
                                    </button>
                                    <button type="button" data-testid="remote-copy-code"
                                        on:click=move |_| copy_remote(Some(code.clone()))>
                                        {move || t(locale.get(), "channels.remote.copy_code")}
                                    </button>
                                    <button type="button" class="danger" data-testid="remote-reset-code"
                                        on:click=move |_| reset_remote_code.call(())>
                                        {move || t(locale.get(), "channels.remote.reset")}
                                    </button>
                                </div>
                            </div>
                        }
                    })
                }}
                {move || {
                    let s = status.get().unwrap_or_default().remote;
                    (s.enabled && s.state == "running").then(|| view! {
                        <p class="settings-field-hint" data-testid="remote-clients">
                            {t(locale.get(), "channels.remote.clients").replace("{n}", &s.clients.to_string())}
                        </p>
                    })
                }}
                <p class="settings-note">{move || t(locale.get(), "channels.remote.security")}</p>
                <div class="row settings-footer">
                    <span class="settings-footer-note">{move || t(locale.get(), "channels.secret_note")}</span>
                    <button type="button" class="primary" data-testid="remote-save"
                        on:click=move |_| save_remote.call(status.get_untracked().unwrap_or_default().remote.enabled)>
                        {move || t(locale.get(), "settings.save")}
                    </button>
                </div>
            </div>
        }.into_view(),
        Some("lab") => view! {
            <crate::lab_view::LabPane locale=locale on_open_project=on_open_project
                on_sync_settings_changed=on_sync_settings_changed/>
        }.into_view(),
        Some(_) => view! { <div></div> }.into_view(),
        None => view! {
            <div class="settings-card channels-overview" data-testid="channels-overview">
                <h3>{move || t(locale.get(), "channels.messages_devices")}</h3>
                <p class="settings-note">{move || t(locale.get(), "channels.desc")}</p>

                {msg_view}
                <div class="settings-list">
                    <div class="settings-list-row settings-list-row-link" data-testid="feishu-channel-row"
                        on:click=move |_| open.set(Some("feishu".into()))>
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {move || t(locale.get(), "channels.feishu.title")}
                                " "
                                {feishu_badge}
                            </span>
                            <span class="settings-list-sub">{move || {
                                let s = status.get().unwrap_or_default();
                                if !s.feishu_pending_owner_open_id.is_empty() {
                                    t(locale.get(), "channels.feishu.pending_title").to_string()
                                } else if s.feishu_bound && s.feishu_owner_open_id.is_empty() {
                                    t(locale.get(), "channels.feishu.owner_missing").to_string()
                                } else if s.feishu_bound {
                                    feishu_region()
                                } else {
                                    t(locale.get(), "channels.feishu.subtitle").to_string()
                                }
                            }}</span>
                        </div>
                        <div class="settings-list-actions">
                            <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                <input type="checkbox" data-testid="feishu-enabled"
                                    aria-label=move || t(locale.get(), "channels.feishu.toggle")
                                    prop:disabled=move || !status.get().map(|s| s.feishu_bound).unwrap_or(false)
                                    prop:checked=move || status.get().map(|s| s.feishu_enabled).unwrap_or(false)
                                    on:change=move |ev| save_feishu.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                        </div>
                    </div>
                    <div class="settings-list-row settings-list-row-link" data-testid="weixin-channel-row"
                        on:click=move |_| open.set(Some("weixin".into()))>
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {move || t(locale.get(), "channels.weixin.title")}
                                " "
                                {weixin_badge}
                            </span>
                            <span class="settings-list-sub">{move || {
                                let s = status.get().unwrap_or_default();
                                if s.weixin_bound && !s.weixin_detail.is_empty() {
                                    s.weixin_detail
                                } else {
                                    t(locale.get(), "channels.weixin.subtitle").to_string()
                                }
                            }}</span>
                        </div>
                        <div class="settings-list-actions">
                            <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                <input type="checkbox" data-testid="weixin-enabled"
                                    aria-label=move || t(locale.get(), "channels.weixin.toggle")
                                    prop:disabled=move || !status.get().map(|s| s.weixin_bound).unwrap_or(false)
                                    prop:checked=move || status.get().map(|s| s.weixin_enabled).unwrap_or(false)
                                    on:change=move |ev| set_weixin_enabled.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                        </div>
                    </div>
                    <div class="settings-list-row settings-list-row-link" data-testid="sticks3-channel-row"
                        on:click=move |_| open.set(Some("sticks3".into()))>
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {move || t(locale.get(), "channels.device.title")}
                                " "
                                {device_badge}
                            </span>
                            <span class="settings-list-sub">{move || {
                                let device = status.get().unwrap_or_default().device;
                                device.url.unwrap_or_else(|| {
                                    if device.bind_ipv4.is_empty() {
                                        t(locale.get(), "channels.device.subtitle").to_string()
                                    } else {
                                        format!("{}:{}", device.bind_ipv4, device.port)
                                    }
                                })
                            }}</span>
                        </div>
                        <div class="settings-list-actions">
                            <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                <input type="checkbox" data-testid="sticks3-enabled"
                                    aria-label=move || t(locale.get(), "channels.device.toggle")
                                    prop:disabled=move || status.get().unwrap_or_default().device.bind_ipv4.is_empty()
                                    prop:checked=move || status.get().unwrap_or_default().device.enabled
                                    on:change=move |ev| save_device.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                        </div>
                    </div>
                    <div class="settings-list-row settings-list-row-link" data-testid="remote-channel-row"
                        on:click=move |_| open.set(Some("remote".into()))>
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {move || t(locale.get(), "channels.remote.title")}
                                " "
                                {remote_badge}
                            </span>
                            <span class="settings-list-sub">{move || {
                                let remote = status.get().unwrap_or_default().remote;
                                if remote.enabled && !remote.relay_url.is_empty() {
                                    remote.relay_url
                                } else {
                                    t(locale.get(), "channels.remote.subtitle").to_string()
                                }
                            }}</span>
                        </div>
                        <div class="settings-list-actions">
                            <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                <input type="checkbox" data-testid="remote-enabled"
                                    aria-label=move || t(locale.get(), "channels.remote.toggle")
                                    prop:disabled=move || {
                                        let remote = status.get().unwrap_or_default().remote;
                                        remote.relay_url.is_empty() || !remote.has_token
                                    }
                                    prop:checked=move || status.get().unwrap_or_default().remote.enabled
                                    on:change=move |ev| save_remote.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                        </div>
                    </div>
                    <div class="settings-list-row settings-list-row-link" data-testid="lab-channel-row"
                        on:click=move |_| open.set(Some("lab".into()))>
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {move || t(locale.get(), "channels.lab.title")}
                            </span>
                            <span class="settings-list-sub">
                                {move || t(locale.get(), "channels.lab.subtitle")}
                            </span>
                        </div>
                        <div class="settings-list-actions">
                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                        </div>
                    </div>
                </div>
                <p class="settings-note" data-testid="channel-routing-help">
                    {move || t(locale.get(), "channels.routing.desc")}
                    <span class="channels-command-list" aria-label="IM slash commands">
                        <code>"/status"</code>
                        <code>"/project"</code>
                        <code>"/session"</code>
                        <code>"/new"</code>
                    </span>
                </p>
            </div>
        }.into_view(),
    }
    }
}
