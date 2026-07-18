use crate::app_support::{
    compose_icon, js_error_text, refresh_execution_contexts, refresh_runtimes, show_toast,
};
use crate::bindings::{invoke_checked, open_external_url};
use crate::dto::*;
use crate::i18n::{localize_backend, t, tf, Locale};
use crate::text::{dom_value, event_target_value, provider_value};
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
    host_auth_method: RwSignal<String>,
    host_password: RwSignal<String>,
    host_has_password: RwSignal<bool>,
    editing_host_alias: RwSignal<Option<String>>,
    ssh_hosts: RwSignal<Vec<SshHost>>,
    execution_contexts: RwSignal<Vec<ExecutionContext>>,
) -> impl IntoView {
    move || {
        show_add_host.get().then(|| view! {
    <div class="overlay">
        <div class="modal host-modal" role="dialog" aria-modal="true">
            <h2>{move || if editing_host_alias.get().is_some() {
                t(locale.get(), "hosts.edit")
            } else {
                t(locale.get(), "hosts.add")
            }}</h2>
            <label class="host-label">{move || t(locale.get(), "hosts.from_config")}</label>
            <select class="host-input" disabled=move || editing_host_alias.get().is_some()
                on:change=move |ev| host_alias.set(dom_value(&ev))>
                <option value="">{move || t(locale.get(), "hosts.pick")}</option>
                {move || config_aliases.get().into_iter().map(|a| view! { <option value=a.clone()>{a}</option> }).collect_view()}
            </select>
            <label class="host-label">{move || t(locale.get(), "hosts.or_type")}</label>
            <input id="add-host-alias" class="host-input" autofocus=true
                disabled=move || editing_host_alias.get().is_some()
                prop:value=move || host_alias.get()
                on:input=move |ev| host_alias.set(event_target_value(&ev)) />
            <label class="host-label" for="host-user">{move || t(locale.get(), "hosts.user")}</label>
            <input id="host-user" class="host-input" prop:value=move || host_user.get()
                placeholder=move || t(locale.get(), "hosts.user_ph")
                on:input=move |ev| host_user.set(event_target_value(&ev)) />
            <label class="host-label">{move || t(locale.get(), "hosts.auth_method")}</label>
            <select class="host-input" prop:value=move || host_auth_method.get()
                on:change=move |ev| host_auth_method.set(dom_value(&ev))>
                <option value="key">{move || t(locale.get(), "hosts.auth_key")}</option>
                <option value="password">{move || t(locale.get(), "hosts.auth_password")}</option>
            </select>
            {move || if host_auth_method.get() == "password" {
                view! {
                    <label class="host-label" for="host-password">{t(locale.get(), "hosts.password")}</label>
                    <input id="host-password" class="host-input" type="password" autocomplete="new-password"
                        prop:value=move || host_password.get()
                        placeholder=move || if host_has_password.get() {
                            t(locale.get(), "hosts.password_keep").to_string()
                        } else {
                            t(locale.get(), "hosts.password_ph").to_string()
                        }
                        on:input=move |ev| host_password.set(event_target_value(&ev)) />
                    <p class="hint">{t(locale.get(), "hosts.password_hint")}</p>
                }.into_view()
            } else {
                view! {
                    <label class="host-label" for="host-identity">{t(locale.get(), "hosts.identity")}</label>
                    <input id="host-identity" class="host-input" prop:value=move || host_identity.get()
                        placeholder=move || t(locale.get(), "hosts.identity_ph")
                        on:input=move |ev| host_identity.set(event_target_value(&ev)) />
                }.into_view()
            }}
            <label class="host-label" for="host-notes">{move || t(locale.get(), "hosts.notes")}</label>
            <textarea id="host-notes" class="host-input" prop:value=move || host_notes.get()
                placeholder=move || t(locale.get(), "hosts.notes_ph")
                on:input=move |ev| host_notes.set(event_target_value(&ev))></textarea>
            <details class="host-advanced">
                <summary>{move || t(locale.get(), "hosts.advanced")}</summary>
                <label class="host-label" for="host-port">{move || t(locale.get(), "hosts.port")}</label>
                <input id="host-port" class="host-input" prop:value=move || host_port.get() on:input=move |ev| host_port.set(event_target_value(&ev)) />
            </details>
            <div class="row">
                <button type="button" on:click=move |_| {
                    editing_host_alias.set(None);
                    show_add_host.set(false);
                }>{move || t(locale.get(), "hosts.cancel")}</button>
                <button type="button" class="primary" disabled=move || {
                    let alias_empty = host_alias.get().trim().is_empty();
                    let password_missing = host_auth_method.get() == "password"
                        && host_password.get().trim().is_empty()
                        && !host_has_password.get();
                    alias_empty || password_missing
                }
                    on:click=move |_| {
                        let opt = |s: String| { let s = s.trim().to_string(); if s.is_empty() { None } else { Some(s) } };
                        let auth = host_auth_method.get();
                        let auth = if auth == "password" { "password" } else { "key" };
                        let password = host_password.get();
                        let host = SshHost {
                            alias: host_alias.get().trim().to_string(),
                            user: opt(host_user.get()),
                            port: host_port.get().trim().parse::<u16>().ok(),
                            identity_file: if auth == "key" { opt(host_identity.get()) } else { None },
                            notes: opt(host_notes.get()),
                            auth_method: Some(auth.into()),
                            has_password: false,
                            password: if auth == "password" { opt(password) } else { None },
                        };
                        let arg = to_value(&serde_json::json!({ "host": host })).unwrap();
                        spawn_local(async move {
                            match invoke_checked("add_ssh_host", arg).await {
                                Ok(v) => {
                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                                        ssh_hosts.set(list);
                                        refresh_execution_contexts(execution_contexts);
                                    }
                                }
                                Err(error) => {
                                    show_toast(&localize_backend(locale.get_untracked(), &js_error_text(error)));
                                }
                            }
                        });
                        host_alias.set(String::new()); host_user.set(String::new()); host_port.set(String::new());
                        host_identity.set(String::new()); host_notes.set(String::new());
                        host_auth_method.set("key".into()); host_password.set(String::new());
                        host_has_password.set(false);
                        editing_host_alias.set(None);
                        show_add_host.set(false);
                    }>{move || if editing_host_alias.get().is_some() {
                        t(locale.get(), "hosts.update")
                    } else {
                        t(locale.get(), "hosts.save")
                    }}</button>
            </div>
        </div>
    </div>
}.into_view())
    }
}

#[component]
pub(super) fn RuntimeInterpreterOverlay(
    locale: RwSignal<Locale>,
    form: RwSignal<Option<RuntimeInterpreterForm>>,
    execution_contexts: RwSignal<Vec<ExecutionContext>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
) -> impl IntoView {
    let busy = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    // Render the dialog from its open state, not from the whole editable form.
    // Otherwise each input event (including paste) replaces the modal DOM and
    // drops focus.
    let open = create_memo(move |_| form.with(|value| value.is_some()));
    let save = move |_| {
        let Some(current) = form.get_untracked() else {
            return;
        };
        busy.set(true);
        error.set(None);
        let args = to_value(&serde_json::json!({
            "contextId": current.context_id,
            "pythonExecutable": current.python_executable,
            "rscriptExecutable": current.rscript_executable,
        }))
        .unwrap();
        spawn_local(async move {
            match invoke_checked("update_execution_context_interpreters", args).await {
                Ok(_) => {
                    refresh_execution_contexts(execution_contexts);
                    refresh_runtimes(runtimes);
                    form.set(None);
                    show_toast(&t(locale.get_untracked(), "runtime_config.saved"));
                }
                Err(value) => error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(value),
                ))),
            }
            busy.set(false);
        });
    };

    move || {
        open.get().then(|| view! {
                <div class="overlay">
                    <div class="modal runtime-config-modal">
                        <div class="ps-head">
                            <h2>{move || t(locale.get(), "runtime_config.title")}</h2>
                            <button type="button" class="ps-close"
                                title=move || t(locale.get(), "settings.cancel")
                                disabled=move || busy.get()
                                on:click=move |_| form.set(None)>{compose_icon("close")}</button>
                        </div>
                        <p class="runtime-config-hint">{
                            move || {
                                let context = form.with(|value| value.as_ref()
                                    .map(|value| value.context_label.clone())
                                    .unwrap_or_default());
                                tf(locale.get(), "runtime_config.scope", &[("context", &context)])
                            }
                        }</p>
                        <label>
                            {move || t(locale.get(), "runtime_config.python")}
                            <input id="runtime-python-executable" autocomplete="off"
                                placeholder=move || t(locale.get(), "runtime_config.python_placeholder")
                                prop:value=move || form.get().map(|value| value.python_executable).unwrap_or_default()
                                on:input=move |event| form.update(|value| {
                                    if let Some(value) = value {
                                        value.python_executable = event_target_value(&event);
                                    }
                                }) />
                        </label>
                        <label>
                            {move || t(locale.get(), "runtime_config.r")}
                            <input id="runtime-rscript-executable" autocomplete="off"
                                placeholder=move || t(locale.get(), "runtime_config.r_placeholder")
                                prop:value=move || form.get().map(|value| value.rscript_executable).unwrap_or_default()
                                on:input=move |event| form.update(|value| {
                                    if let Some(value) = value {
                                        value.rscript_executable = event_target_value(&event);
                                    }
                                }) />
                        </label>
                        <p class="runtime-config-hint">{move || t(locale.get(), "runtime_config.hint")}</p>
                        {move || error.get().map(|message| view! {
                            <div class="settings-status fail">{message}</div>
                        })}
                        <div class="row">
                            <button type="button" disabled=move || busy.get()
                                on:click=move |_| form.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" disabled=move || busy.get()
                                on:click=save>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                </div>
            })
    }
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
    move || {
        show_capabilities.get().then(|| view! {
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
                {move || bootstrap.get().filter(|b| !b.python_initializing && (!b.python_ok || !b.uv_ok || !b.node_ok || !b.sci_ok || !b.pixi_ok)).map(|_| view! {
                    <button class="primary" disabled=move || busy.get() on:click=move |ev| start_env_setup.call(ev)>
                        {move || t(locale.get(), "caps.setup_env")}
                    </button>
                })}
            </div>
        </div>
    </div>
}.into_view())
    }
}

/// Where each provider issues API keys — used by the onboarding "Get an API key" link.
fn provider_key_url(provider: &str) -> &'static str {
    match provider_value(provider) {
        "anthropic" => "https://console.anthropic.com/settings/keys",
        "openai_responses" => "https://platform.openai.com/api-keys",
        _ => "https://platform.deepseek.com/api_keys",
    }
}

#[component]
pub(super) fn OnboardingOverlay(
    locale: RwSignal<Locale>,
    show_onboarding: RwSignal<bool>,
    onboard_step: RwSignal<usize>,
    onboard_provider: RwSignal<String>,
    onboard_key: RwSignal<String>,
    save_onboard_key: Callback<()>,
    dismiss_onboard: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    move || {
        show_onboarding.get().then(|| {
    let step = onboard_step.get();
    let loc = locale.get();
    view! {
        <div class="overlay onboard-overlay">
            <div class="modal onboard">
                {match step {
                    0 => view! {
                        <h2>{t(loc, "onboard.apikey.title")}</h2>
                        <p class="hint">{t(loc, "onboard.apikey.body")}</p>
                        <div class="onboard-form">
                            <label>{t(loc, "settings.provider")}
                                <select prop:value=move || provider_value(&onboard_provider.get()).to_string()
                                    on:change=move |ev| onboard_provider.set(provider_value(&dom_value(&ev)).into())>
                                    <option value="openai">{t(loc, "settings.provider.openai")}</option>
                                    <option value="openai_responses">{t(loc, "settings.provider.openai_responses")}</option>
                                    <option value="anthropic">{t(loc, "settings.provider.anthropic")}</option>
                                </select>
                            </label>
                            <label>{t(loc, "settings.api_key")}
                                <input type="password" autocomplete="new-password"
                                    prop:value=move || onboard_key.get()
                                    on:input=move |ev| onboard_key.set(event_target_value(&ev)) />
                            </label>
                            <button type="button" class="linklike onboard-getkey"
                                on:click=move |_| open_external_url(provider_key_url(&onboard_provider.get()).into())>
                                {t(loc, "onboard.apikey.get_key")}
                            </button>
                        </div>
                    }.into_view(),
                    1 => view! {
                        <h2>{t(loc, "onboard.welcome.title")}</h2>
                        <p class="hint">{t(loc, "onboard.welcome.body")}</p>
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
                        view! { <button class="primary" on:click=move |_| {
                            if step == 0 { save_onboard_key.call(()); }
                            onboard_step.update(|s| *s += 1);
                        }>{move || t(locale.get(), "onboard.next")}</button> }.into_view()
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
}
