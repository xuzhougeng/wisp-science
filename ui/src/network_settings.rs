use crate::app_support::{compose_icon, js_error_text};
use crate::bindings::invoke_checked;
use crate::dto::{NetworkSettings, Settings};
use crate::i18n::{t, use_locale};
use crate::text::{event_target_input, event_target_value};
use leptos::*;
use wasm_bindgen::JsValue;

fn proxy_value(settings: &NetworkSettings, scope: &str) -> String {
    match scope {
        "model" => settings.model_proxy_url.clone(),
        "mcp" => settings.mcp_proxy_url.clone(),
        _ => settings.command_proxy_url.clone(),
    }
}

fn set_proxy(settings: &mut NetworkSettings, scope: &str, value: String) {
    match scope {
        "model" => settings.model_proxy_url = value,
        "mcp" => settings.mcp_proxy_url = value,
        _ => settings.command_proxy_url = value,
    }
}

#[component]
pub(crate) fn NetworkSettingsView(settings: RwSignal<Settings>) -> impl IntoView {
    let locale = use_locale();
    let saved = create_rw_signal(NetworkSettings::default());
    let draft = create_rw_signal(NetworkSettings::default());
    let loaded = create_rw_signal(false);
    let busy = create_rw_signal(false);
    let mirror_open = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let status = create_rw_signal(false);
    // Custom mode needs to stay selected while its URL is still empty.
    let custom = create_rw_signal(Vec::<String>::new());
    spawn_local(async move {
        let result = invoke_checked("get_network_settings", JsValue::UNDEFINED).await;
        if loaded.try_get_untracked().is_none() {
            return;
        }
        match result {
            Ok(value) => match serde_wasm_bindgen::from_value::<NetworkSettings>(value) {
                Ok(value) => {
                    saved.set(value.clone());
                    draft.set(value);
                    loaded.set(true);
                }
                Err(e) => error.set(Some(e.to_string())),
            },
            Err(e) => error.set(Some(js_error_text(e))),
        }
    });
    crate::window_capture_escape(move || {
        if mirror_open.get_untracked() {
            mirror_open.set(false);
            true
        } else {
            false
        }
    });
    let save = Callback::new(move |scope: &'static str| {
        if busy.get_untracked() || !loaded.get_untracked() {
            return;
        }
        let mut next = saved.get_untracked();
        let edited = draft.get_untracked();
        if scope == "mirrors" {
            next.conda_mirror_url = edited.conda_mirror_url;
            next.pip_index_url = edited.pip_index_url;
            next.ca_bundle_path = edited.ca_bundle_path;
        } else {
            let value = proxy_value(&edited, scope);
            if custom.get_untracked().iter().any(|s| s == scope) && value.trim().is_empty() {
                error.set(Some(
                    t(locale.get_untracked(), "network.address_required").into(),
                ));
                return;
            }
            set_proxy(&mut next, scope, value);
        }
        busy.set(true);
        error.set(None);
        status.set(false);
        spawn_local(async move {
            let args =
                serde_wasm_bindgen::to_value(&serde_json::json!({ "settings": next })).unwrap();
            let result = invoke_checked("set_network_settings", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<NetworkSettings>(value)
                        .map_err(|e| e.to_string())
                });
            if busy.try_get_untracked().is_none() {
                return;
            }
            match result {
                Ok(value) => {
                    settings.update(|s| s.proxy_url = value.model_proxy_url.clone());
                    draft.update(|s| {
                        if scope == "mirrors" {
                            s.conda_mirror_url = value.conda_mirror_url.clone();
                            s.pip_index_url = value.pip_index_url.clone();
                            s.ca_bundle_path = value.ca_bundle_path.clone();
                        } else {
                            set_proxy(s, scope, proxy_value(&value, scope));
                        }
                    });
                    saved.set(value);
                    status.set(true);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    });
    view! {
        <div class="network-settings">
            <div class="settings-head">
                <div class="settings-head-main">
                    {move || if mirror_open.get() {
                        view! {
                            <button class="settings-head-back" type="button" aria-label=move || t(locale.get(), "settings.back")
                                on:click=move |_| mirror_open.set(false)>{compose_icon("chevron-left")}</button>
                            <div class="settings-breadcrumb">
                                <button class="settings-crumb-link" type="button" on:click=move |_| mirror_open.set(false)>{move || t(locale.get(), "settings.nav.network")}</button>
                                <span class="network-crumb-separator">{compose_icon("chevron-right")}</span>
                                <span class="settings-crumb-current">{move || t(locale.get(), "network.mirrors")}</span>
                            </div>
                        }.into_view()
                    } else {
                        view! { <h2>{move || t(locale.get(), "settings.nav.network")}</h2> }.into_view()
                    }}
                </div>
            </div>
            <div class="settings-pane network-pane">
                {move || error.get().map(|message| view! { <div class="settings-status fail" role="alert">{message}</div> })}
                {move || status.get().then(|| view! { <div class="settings-status ok" role="status">{move || t(locale.get(), "network.saved")}</div> })}
                <Show when=move || loaded.get() fallback=move || view! { <p class="settings-field-hint">{t(locale.get(), "network.loading")}</p> }>
                    {move || if mirror_open.get() {
                        view! {
                            <section data-testid="package-mirrors" class="network-section">
                                <h3>{move || t(locale.get(), "network.mirrors")}</h3>
                                <p class="network-description">{move || t(locale.get(), "network.mirrors_hint")}</p>
                                <div class="network-mirror-fields">
                                    <label>{move || t(locale.get(), "network.conda")}
                                        <input data-testid="conda-mirror" placeholder="https://mirror.example.com/anaconda/cloud/conda-forge"
                                            disabled=move || busy.get() prop:value=move || draft.get().conda_mirror_url
                                            on:input=move |ev| { status.set(false); draft.update(|s| s.conda_mirror_url = event_target_input(&ev).value()); } />
                                    </label>
                                    <label>{move || t(locale.get(), "network.pip")}
                                        <input data-testid="pip-index" placeholder="https://mirror.example.com/pypi/simple"
                                            disabled=move || busy.get() prop:value=move || draft.get().pip_index_url
                                            on:input=move |ev| { status.set(false); draft.update(|s| s.pip_index_url = event_target_input(&ev).value()); } />
                                    </label>
                                    <label>{move || t(locale.get(), "network.ca")}
                                        <input data-testid="mirror-ca-bundle" placeholder=move || t(locale.get(), "network.ca_placeholder")
                                            disabled=move || busy.get() prop:value=move || draft.get().ca_bundle_path
                                            on:input=move |ev| { status.set(false); draft.update(|s| s.ca_bundle_path = event_target_input(&ev).value()); } />
                                        <span class="settings-field-hint">{move || t(locale.get(), "network.ca_hint")}</span>
                                    </label>
                                </div>
                                <p class="network-description">{move || t(locale.get(), "network.credentials_hint")}</p>
                                <div class="row settings-footer">
                                    <button type="button" disabled=move || busy.get() on:click=move |_| {
                                        let value = saved.get_untracked();
                                        draft.update(|s| { s.conda_mirror_url = value.conda_mirror_url; s.pip_index_url = value.pip_index_url; s.ca_bundle_path = value.ca_bundle_path; });
                                        mirror_open.set(false);
                                    }>{move || t(locale.get(), "settings.cancel")}</button>
                                    <button type="button" class="primary" data-testid="save-package-mirrors" disabled=move || busy.get() on:click=move |_| save.call("mirrors")>{move || t(locale.get(), "settings.save")}</button>
                                </div>
                            </section>
                        }.into_view()
                    } else {
                        view! {
                            <section class="network-section">
                                <h3>{move || t(locale.get(), "network.connection")}</h3>
                                <p class="network-description">{move || t(locale.get(), "network.connection_hint")}</p>
                                {[ ("model", "network.model", "network.model_hint"), ("mcp", "network.mcp", "network.mcp_hint"), ("command", "network.command", "network.command_hint") ].into_iter().map(|(scope, label, hint)| {
                                    let mode = move || {
                                        let value = proxy_value(&draft.get(), scope);
                                        if value == "none" { "direct" } else if !value.is_empty() || custom.get().iter().any(|s| s == scope) { "custom" } else { "system" }
                                    };
                                    view! {
                                        <div class="network-proxy" data-testid=format!("network-proxy-{scope}")>
                                            <div class="network-proxy-heading">
                                                <label attr:for=format!("network-mode-{scope}")>{move || t(locale.get(), label)}</label>
                                                <span class="network-mode-status">{move || {
                                                    let value = proxy_value(&saved.get(), scope);
                                                    t(locale.get(), if value == "none" { "network.direct" } else if value.is_empty() { "network.system" } else { "network.custom" })
                                                }}</span>
                                            </div>
                                            <div class="network-proxy-controls">
                                                <select id=format!("network-mode-{scope}") data-testid=format!("proxy-mode-{scope}") prop:value=mode disabled=move || busy.get()
                                                    on:change=move |ev| {
                                                        let mode = event_target_value(&ev);
                                                        custom.update(|items| { items.retain(|s| s != scope); if mode == "custom" { items.push(scope.into()); } });
                                                        draft.update(|s| set_proxy(s, scope, if mode == "direct" { "none" } else { "" }.into()));
                                                        status.set(false);
                                                    }>
                                                    <option value="system" prop:selected=move || mode() == "system">{move || t(locale.get(), "network.system")}</option>
                                                    <option value="direct" prop:selected=move || mode() == "direct">{move || t(locale.get(), "network.direct")}</option>
                                                    <option value="custom" prop:selected=move || mode() == "custom">{move || t(locale.get(), "network.custom")}</option>
                                                </select>
                                                <input aria-label=move || format!("{} — {}", t(locale.get(), label), t(locale.get(), "network.address"))
                                                    data-testid=format!("proxy-address-{scope}") placeholder="http://127.0.0.1:7890"
                                                    disabled=move || busy.get() || mode() != "custom"
                                                    prop:value=move || { let value = proxy_value(&draft.get(), scope); if value == "none" { String::new() } else { value } }
                                                    on:input=move |ev| {
                                                        status.set(false);
                                                        custom.update(|items| { if !items.iter().any(|s| s == scope) { items.push(scope.into()); } });
                                                        draft.update(|s| set_proxy(s, scope, event_target_input(&ev).value()));
                                                    } />
                                                <button type="button" class="network-clear" disabled=move || busy.get()
                                                    on:click=move |_| { custom.update(|items| items.retain(|s| s != scope)); draft.update(|s| set_proxy(s, scope, String::new())); status.set(false); }>{move || t(locale.get(), "network.clear")}</button>
                                                <button type="button" data-testid=format!("save-proxy-{scope}") disabled=move || busy.get() on:click=move |_| save.call(scope)>{move || t(locale.get(), "settings.save")}</button>
                                            </div>
                                            <p class="settings-field-hint">{move || t(locale.get(), hint)}</p>
                                        </div>
                                    }
                                }).collect_view()}
                            </section>
                            <section class="network-section network-mirror-summary">
                                <div>
                                    <h3>{move || t(locale.get(), "network.mirrors")}</h3>
                                    <p class="network-description" data-testid="package-mirror-summary">{move || {
                                        let value = saved.get();
                                        if value.conda_mirror_url.is_empty() && value.pip_index_url.is_empty() && value.ca_bundle_path.is_empty() { t(locale.get(), "network.mirrors_empty").to_string() }
                                        else { t(locale.get(), "network.mirrors_configured").to_string() }
                                    }}</p>
                                </div>
                                <button type="button" data-testid="configure-package-mirrors" on:click=move |_| { status.set(false); error.set(None); mirror_open.set(true); }>{move || t(locale.get(), "network.configure")}</button>
                            </section>
                        }.into_view()
                    }}
                </Show>
            </div>
        </div>
    }
}
