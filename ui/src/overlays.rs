use crate::app_support::{compose_icon, refresh_execution_contexts};
use crate::bindings::invoke;
use crate::dto::*;
use crate::i18n::{tf, t, Locale};
use crate::text::{dom_value, event_target_value};
use leptos::*;
use serde_wasm_bindgen::to_value;

#[component]
pub(super) fn AddHostOverlay(
    locale: RwSignal<Locale>,
    show_add_host: RwSignal<bool>,
    host_alias: RwSignal<String>,
    config_aliases: RwSignal<Vec<String>>,
    host_notes: RwSignal<String>,
    host_user: RwSignal<String>,
    host_port: RwSignal<String>,
    host_identity: RwSignal<String>,
    ssh_hosts: RwSignal<Vec<SshHost>>,
    execution_contexts: RwSignal<Vec<ExecutionContext>>,
) -> impl IntoView {
    move || show_add_host.get().then(|| view! {
    <div class="overlay">
        <div class="modal host-modal">
            <h2>{move || t(locale.get(), "hosts.add")}</h2>
            <label class="host-label">{move || t(locale.get(), "hosts.from_config")}</label>
            <select class="host-input" on:change=move |ev| host_alias.set(dom_value(&ev))>
                <option value="">{move || t(locale.get(), "hosts.pick")}</option>
                {move || config_aliases.get().into_iter().map(|a| view! { <option value=a.clone()>{a}</option> }).collect_view()}
            </select>
            <label class="host-label">{move || t(locale.get(), "hosts.or_type")}</label>
            <input id="add-host-alias" class="host-input" autofocus=true prop:value=move || host_alias.get() on:input=move |ev| host_alias.set(event_target_value(&ev)) />
            <label class="host-label">{move || t(locale.get(), "hosts.notes")}</label>
            <textarea class="host-input" prop:value=move || host_notes.get()
                placeholder=move || t(locale.get(), "hosts.notes_ph")
                on:input=move |ev| host_notes.set(event_target_value(&ev))></textarea>
            <details class="host-advanced">
                <summary>{move || t(locale.get(), "hosts.advanced")}</summary>
                <label class="host-label">{move || t(locale.get(), "hosts.user")}</label>
                <input class="host-input" prop:value=move || host_user.get() on:input=move |ev| host_user.set(event_target_value(&ev)) />
                <label class="host-label">{move || t(locale.get(), "hosts.port")}</label>
                <input class="host-input" prop:value=move || host_port.get() on:input=move |ev| host_port.set(event_target_value(&ev)) />
                <label class="host-label">{move || t(locale.get(), "hosts.identity")}</label>
                <input class="host-input" prop:value=move || host_identity.get() on:input=move |ev| host_identity.set(event_target_value(&ev)) />
            </details>
            <div class="row">
                <button type="button" on:click=move |_| show_add_host.set(false)>{move || t(locale.get(), "hosts.cancel")}</button>
                <button type="button" class="primary" disabled=move || host_alias.get().trim().is_empty()
                    on:click=move |_| {
                        let opt = |s: String| { let s = s.trim().to_string(); if s.is_empty() { None } else { Some(s) } };
                        let host = SshHost {
                            alias: host_alias.get().trim().to_string(),
                            user: opt(host_user.get()),
                            port: host_port.get().trim().parse::<u16>().ok(),
                            identity_file: opt(host_identity.get()),
                            notes: opt(host_notes.get()),
                        };
                        let arg = to_value(&serde_json::json!({ "host": host })).unwrap();
                        spawn_local(async move {
                            let v = invoke("add_ssh_host", arg).await;
                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                                ssh_hosts.set(list);
                                refresh_execution_contexts(execution_contexts);
                            }
                        });
                        host_alias.set(String::new()); host_user.set(String::new()); host_port.set(String::new());
                        host_identity.set(String::new()); host_notes.set(String::new());
                        show_add_host.set(false);
                    }>{move || t(locale.get(), "hosts.save")}</button>
            </div>
        </div>
    </div>
}.into_view())
}
#[component]
pub(super) fn CapabilitiesOverlay(
    locale: RwSignal<Locale>,
    show_capabilities: RwSignal<bool>,
    bootstrap: RwSignal<Option<BootstrapStatus>>,
    caps: RwSignal<Option<Capabilities>>,
    busy: RwSignal<bool>,
    start_env_setup: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    move || show_capabilities.get().then(|| view! {
    <div class="overlay">
        <div class="modal modal-wide">
            <div class="fb-head">
                <h2>{move || t(locale.get(), "caps.title")}</h2>
                <button class="icon-btn" on:click=move |_| show_capabilities.set(false)>{compose_icon("close")}</button>
            </div>
            {move || bootstrap.get().map(|b| {
                let loc = locale.get();
                view! {
                <div class="cap-section">
                    <h3>{tf(loc, "caps.runtime", &[("version", &b.app_version)])}</h3>
                    <p class="hint">{tf(loc, "caps.workspace", &[("path", &b.workspace)])}</p>
                    <p class="hint">{{
                        let ready = t(loc, "caps.ready");
                        let missing = t(loc, "caps.missing");
                        tf(loc, "caps.runtime_status", &[
                        ("py", if b.python_ok { &ready } else { &missing }),
                        ("uv", if b.uv_ok { &ready } else { &missing }),
                        ("node", if b.node_ok { &ready } else { &missing }),
                        ("sci", if b.sci_ok { &ready } else { &missing }),
                        ("pixi", if b.pixi_ok { &ready } else { &missing }),
                        ("skills", &b.skills_loaded.to_string()),
                        ("mcp", &b.mcp_catalog.to_string()),
                    ])}}</p>
                    {(!b.errors.is_empty()).then(|| view! {
                        <div class="settings-status fail">
                            {b.errors.join("\n")}
                        </div>
                    })}
                </div>
            }})}
            {move || caps.get().map(|c| view! {
                // ponytail: counts only — detail lists (bio-tool tags, skill list,
                // permissions hint) live in Settings, not this read-only summary.
                <div class="cap-grid">
                    <div class="cap-stat"><span class="cap-num">{c.project.skill_count}</span><span class="cap-label">{move || t(locale.get(), "caps.skills")}</span></div>
                    <div class="cap-stat"><span class="cap-num">{c.mcp_servers.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.mcp_servers")}</span></div>
                    <div class="cap-stat"><span class="cap-num">{c.memory_files.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.memory_files")}</span></div>
                </div>
            })}
            <div class="row">
                <button on:click=move |_| show_capabilities.set(false)>{move || t(locale.get(), "caps.close")}</button>
                {move || bootstrap.get().filter(|b| !b.python_ok || !b.uv_ok || !b.node_ok || !b.sci_ok || !b.pixi_ok).map(|_| view! {
                    <button class="primary" disabled=move || busy.get() on:click=move |ev| start_env_setup.call(ev)>
                        {move || t(locale.get(), "caps.setup_env")}
                    </button>
                })}
            </div>
        </div>
    </div>
}.into_view())
}

#[component]
pub(super) fn OnboardingOverlay(
    locale: RwSignal<Locale>,
    show_onboarding: RwSignal<bool>,
    onboard_step: RwSignal<usize>,
    dismiss_onboard: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    move || show_onboarding.get().then(|| {
    let step = onboard_step.get();
    let loc = locale.get();
    view! {
        <div class="overlay onboard-overlay">
            <div class="modal onboard">
                {match step {
                    0 => view! {
                        <h2>{t(loc, "onboard.welcome.title")}</h2>
                        <p class="hint">{t(loc, "onboard.welcome.body")}</p>
                    }.into_view(),
                    1 => view! {
                        <h2>{t(loc, "onboard.connect.title")}</h2>
                        <p class="hint">{t(loc, "onboard.connect.body")}</p>
                    }.into_view(),
                    _ => view! {
                        <h2>{t(loc, "onboard.features.title")}</h2>
                        <p class="hint">{t(loc, "onboard.features.body")}</p>
                    }.into_view(),
                }}
                <div class="onboard-dots">
                    {(0..3).map(|i| view! {
                        <span class="onboard-dot" class:active=move || onboard_step.get() == i></span>
                    }).collect_view()}
                </div>
                <div class="row">
                    {if step > 0 {
                        view! { <button on:click=move |_| onboard_step.update(|s| *s = s.saturating_sub(1))>{move || t(locale.get(), "onboard.back")}</button> }.into_view()
                    } else { view! { <span></span> }.into_view() }}
                    {if step < 2 {
                        view! { <button class="primary" on:click=move |_| onboard_step.update(|s| *s += 1)>{move || t(locale.get(), "onboard.next")}</button> }.into_view()
                    } else {
                        view! {
                            <button class="primary" on:click=move |ev| dismiss_onboard.call(ev)>{move || t(locale.get(), "onboard.start")}</button>
                        }.into_view()
                    }}
                </div>
            </div>
        </div>
    }.into_view()
})
}
