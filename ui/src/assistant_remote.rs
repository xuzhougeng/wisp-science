//! Independent WeChat access to the research assistant's persistent conversation.
use crate::app_support::compose_icon;
use crate::dto::{AssistantWeixinStatus, WeixinBindStart};
use crate::i18n::{t, Locale};
use crate::research_journey::{call, j};
use leptos::*;
use serde_json::json;
use std::{cell::Cell, rc::Rc, time::Duration};

async fn pause() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 1000);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

#[component]
pub(crate) fn AssistantRemote(locale: RwSignal<Locale>, on_close: Callback<()>) -> impl IntoView {
    let status = create_rw_signal(None::<AssistantWeixinStatus>);
    let error = create_rw_signal(None::<String>);
    let busy = create_rw_signal(false);
    let qr = create_rw_signal(None::<WeixinBindStart>);
    let scan_state = create_rw_signal(String::new());
    let alive = Rc::new(Cell::new(true));
    let refresh = Callback::new(move |_: ()| {
        spawn_local(async move {
            match call::<AssistantWeixinStatus>("assistant_weixin_status", json!({})).await {
                Ok(value) => {
                    let _ = status.try_set(Some(value));
                }
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
        });
    });
    refresh.call(());
    let timer = set_interval_with_handle(move || refresh.call(()), Duration::from_secs(3)).ok();
    on_cleanup({
        let alive = alive.clone();
        move || {
            alive.set(false);
            if let Some(timer) = timer {
                timer.clear();
            }
        }
    });

    let change = Callback::new(move |(command, enabled): (&'static str, bool)| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            if let Err(message) = call::<serde_json::Value>(
                command,
                json!({"destination": "assistant", "enabled": enabled}),
            )
            .await
            {
                let _ = error.try_set(Some(message));
            }
            let _ = busy.try_set(false);
            refresh.call(());
        });
    });
    let bind = Callback::new(move |_: ()| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        scan_state.set("waiting".into());
        let alive = alive.clone();
        spawn_local(async move {
            let result: Result<(), String> = async {
                let binding: WeixinBindStart = call("weixin_bind_start", json!({})).await?;
                if !alive.get() {
                    return Ok(());
                }
                let code = binding.qrcode.clone();
                let _ = qr.try_set(Some(binding));
                for _ in 0..180 {
                    pause().await;
                    if !alive.get() {
                        return Ok(());
                    }
                    let state: String = call(
                        "weixin_bind_poll",
                        json!({"qrcode": code, "destination": "assistant"}),
                    )
                    .await?;
                    if !alive.get() {
                        return Ok(());
                    }
                    let _ = scan_state.try_set(state.clone());
                    match state.as_str() {
                        "confirmed" => {
                            refresh.call(());
                            return Ok(());
                        }
                        "expired" => return Ok(()),
                        _ => {}
                    }
                }
                let _ = scan_state.try_set("expired".into());
                Ok(())
            }
            .await;
            if alive.get() {
                if let Err(message) = result {
                    let _ = error.try_set(Some(message));
                }
                let _ = qr.try_set(None);
                let _ = busy.try_set(false);
            }
        });
    });

    view! {
        <div class="overlay assistant-remote-overlay" on:click=move |_| on_close.call(())>
            <section class="assistant-remote-dialog" role="dialog" aria-modal="true" aria-labelledby="assistant-remote-title"
                data-testid="assistant-remote" on:click=move |event| event.stop_propagation()>
                <header><div><h2 id="assistant-remote-title">{move || j(locale.get(), "Remote access", "远程接入")}</h2>
                    <p>{move || j(locale.get(), "Research assistant · WeChat", "科研助理 · 微信")}</p></div>
                    <button type="button" class="icon-btn" aria-label=move || j(locale.get(), "Close remote access", "关闭远程接入") on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </header>
                <p>{move || j(locale.get(), "Talk to your research assistant from WeChat. The same ongoing conversation manages all visible projects — describe the project and task in your own words.", "在微信里直接与科研助理对话，延续桌面的同一条长期对话。用自然语言说明项目和需求，即可统筹所有可见项目。")}</p>
                <div class="assistant-remote-channel">
                    <div><strong>{move || t(locale.get(), "channels.weixin.title")}</strong>
                        <p role="status">{move || status.get().map(|value| {
                            let key = if !value.bound { "channels.weixin.not_bound" } else { match value.state.as_str() {
                                "running" => "channels.state.running", "connecting" => "channels.state.connecting", "error" => "channels.state.error", _ => "channels.state.stopped",
                            }};
                            t(locale.get(), key).to_string()
                        }).unwrap_or_else(|| j(locale.get(), "Loading…", "正在读取…").into())}</p>
                    </div>
                    <label class="toggle"><input type="checkbox" data-testid="assistant-weixin-enabled"
                        aria-label=move || j(locale.get(), "Enable assistant WeChat", "启用科研助理微信接入")
                        prop:checked=move || status.get().is_some_and(|s| s.enabled)
                        disabled=move || busy.get() || !status.get().is_some_and(|s| s.bound)
                        on:change=move |ev| change.call(("set_weixin_channel", crate::text::event_target_checked(&ev))) />
                        <span class="toggle-track" aria-hidden="true"></span></label>
                </div>
                {move || status.get().filter(|s| !s.detail.is_empty()).map(|s| view! {<p class="assistant-remote-detail">{s.detail}</p>})}
                <div class="assistant-remote-actions">
                    <button type="button" class="primary" disabled=move || busy.get() || status.get().is_none() on:click=move |_| bind.call(())>
                        {move || if status.get().is_some_and(|s| s.bound) { j(locale.get(), "Bind again", "重新扫码绑定") } else { j(locale.get(), "Scan to bind", "扫码绑定") }}</button>
                    {move || status.get().is_some_and(|s| s.bound).then(|| view! {
                        <button type="button" disabled=move || busy.get() on:click=move |_| change.call(("weixin_unbind", false))>{move || j(locale.get(), "Unbind", "解除绑定")}</button>
                    })}
                    <button type="button" disabled=move || busy.get() on:click=move |_| { error.set(None); refresh.call(()); }>{move || j(locale.get(), "Refresh status", "刷新状态")}</button>
                </div>
                {move || qr.get().map(|binding| view! {
                    <div class="assistant-remote-qr"><img src=binding.qr_image alt=move || j(locale.get(), "WeChat binding QR code", "微信绑定二维码") />
                        <p>{move || j(locale.get(), "Scan with the owner's WeChat account and confirm on your phone.", "请用所有者的微信扫码，并在手机上确认。")}</p></div>
                })}
                {move || (scan_state.get() == "expired").then(|| view! {<p role="status">{move || j(locale.get(), "QR code expired. Scan again to retry.", "二维码已过期，请重新扫码绑定。")}</p>})}
                {move || error.get().map(|message| view! {<p class="assistant-remote-error" role="alert">{message}</p>})}
                <footer>{move || j(locale.get(), "This binding and switch are separate from Settings → Remote Access. Only the scanning owner's direct messages are accepted. Keep Wisp running; handle approvals in the desktop assistant or project.", "此处的绑定和开关独立于设置页的远程接入。仅接收扫码所有者的一对一消息。请保持 Wisp 运行；需要审批时，请在桌面助理或对应项目中处理。")}</footer>
            </section>
        </div>
    }
}
