use crate::app_support::{
    build_conn_json, close_details_ancestor, compose_icon, conn_form_from_row, js_error_text,
    join_tags, new_model_form, profile_to_form, settings_section_label, settings_subpage_label,
    skill_matches_filter, CRED_GROUPS,
};
use crate::bindings::{invoke, invoke_checked};
use crate::dto::*;
use crate::i18n::{localize_backend, set_document_lang, tf, t, Locale};
use crate::text::{
    dom_value, event_target_checked, event_target_input, format_bytes, provider_defaults,
    provider_value,
};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{BTreeSet, HashMap, HashSet};
use wasm_bindgen::JsValue;

#[derive(Clone, Copy)]
pub(super) struct SettingsViewState {
    pub(super) locale: RwSignal<Locale>,
    pub(super) show_settings: RwSignal<bool>,
    pub(super) settings_section: RwSignal<String>,
    pub(super) open_conn_key: RwSignal<Option<String>>,
    pub(super) connectors: RwSignal<Option<ConnectorsView>>,
    pub(super) model_form: RwSignal<Option<ModelForm>>,
    pub(super) conn_form: RwSignal<Option<ConnForm>>,
    pub(super) memory_selected: RwSignal<Option<String>>,
    pub(super) specialist_form: RwSignal<Option<Specialist>>,
    pub(super) settings: RwSignal<Settings>,
    pub(super) bootstrap: RwSignal<Option<BootstrapStatus>>,
    pub(super) settings_message: RwSignal<Option<(bool, String)>>,
    pub(super) settings_busy: RwSignal<bool>,
    pub(super) model_form_open: Memo<bool>,
    pub(super) model_form_key: RwSignal<String>,
    pub(super) models: RwSignal<Vec<ModelProfile>>,
    pub(super) model_form_msg: RwSignal<Option<(bool, String)>>,
    pub(super) specialists: RwSignal<Vec<Specialist>>,
    pub(super) specialist_form_open: Memo<bool>,
    pub(super) memory_view: RwSignal<Option<MemoryView>>,
    pub(super) memory_editor: RwSignal<String>,
    pub(super) memory_msg: RwSignal<Option<(bool, String)>>,
    pub(super) skills_list: RwSignal<Vec<SkillRow>>,
    pub(super) skill_filter_tag: RwSignal<String>,
    pub(super) skills_search: RwSignal<String>,
    pub(super) skills_msg: RwSignal<Option<(bool, String)>>,
    pub(super) cred_status: RwSignal<HashMap<String, bool>>,
    pub(super) cred_inputs: RwSignal<HashMap<String, String>>,
    pub(super) cred_msg: RwSignal<Option<(bool, String)>>,
    pub(super) approval_grants: RwSignal<Vec<ApprovalGrantRow>>,
    pub(super) conns_view: RwSignal<Option<ConnView>>,
    pub(super) conn_form_open: Memo<bool>,
    pub(super) conn_form_kind: Memo<String>,
    pub(super) conn_test_msg: RwSignal<Option<(bool, String)>>,
    pub(super) custom_conn_tools: RwSignal<HashMap<String, Vec<ConnectorTool>>>,
    pub(super) custom_conn_tools_loading: RwSignal<HashSet<String>>,
    pub(super) custom_conn_tool_errors: RwSignal<HashMap<String, String>>,
}

#[component]
pub(super) fn SettingsView(
    state: SettingsViewState,
    go_settings_section: Callback<String>,
    close_settings_subpage: Callback<()>,
    check_updates: Callback<web_sys::MouseEvent>,
    save_settings: Callback<web_sys::MouseEvent>,
    save_model_form: Callback<web_sys::MouseEvent>,
    save_specialist_form: Callback<web_sys::MouseEvent>,
    validate_model_form: Callback<web_sys::MouseEvent>,
    start_specialist_chat: Callback<web_sys::MouseEvent>,
    refresh_conns: Callback<()>,
    refresh_skills: Callback<()>,
    refresh_approval_grants: Callback<()>,
    load_memory_file: Callback<String>,
    load_custom_conn_tools: Callback<ConnRow>,
    save_skill_tags: Callback<(String, String)>,
    set_visible_skills_enabled: Callback<bool>,
    install_skill_from: Callback<String>,
    remove_specialist: Callback<String>,
) -> impl IntoView {
    let SettingsViewState {
        locale,
        show_settings,
        settings_section,
        open_conn_key,
        connectors,
        model_form,
        conn_form,
        memory_selected,
        specialist_form,
        settings,
        bootstrap,
        settings_message,
        settings_busy,
        model_form_open,
        model_form_key,
        models,
        model_form_msg,
        specialists,
        specialist_form_open,
        memory_view,
        memory_editor,
        memory_msg,
        skills_list,
        skill_filter_tag,
        skills_search,
        skills_msg,
        cred_status,
        cred_inputs,
        cred_msg,
        approval_grants,
        conns_view,
        conn_form_open,
        conn_form_kind,
        conn_test_msg,
        custom_conn_tools,
        custom_conn_tools_loading,
        custom_conn_tool_errors,
    } = state;

move || show_settings.get().then(|| view! {
    <div class="overlay">
        <div class="modal settings-modal">
            <div class="settings-nav">
                <div class="settings-nav-group">
                    <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.workspace")}</span>
                    <button class:active=move || settings_section.get()=="general"
                        on:click=move |_| go_settings_section.call("general".into())>
                        {move || t(locale.get(), "settings.nav.general")}</button>
                    <button class:active=move || settings_section.get()=="credentials"
                        on:click=move |_| go_settings_section.call("credentials".into())>
                        {move || t(locale.get(), "settings.nav.credentials")}</button>
                    <button class:active=move || settings_section.get()=="permissions"
                        on:click=move |_| go_settings_section.call("permissions".into())>
                        {move || t(locale.get(), "settings.nav.permissions")}</button>
                </div>
                <div class="settings-nav-group">
                    <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.capabilities")}</span>
                    <button class:active=move || settings_section.get()=="models"
                        on:click=move |_| go_settings_section.call("models".into())>
                        {move || t(locale.get(), "settings.nav.models")}</button>
                    <button class:active=move || settings_section.get()=="specialists"
                        on:click=move |_| go_settings_section.call("specialists".into())>
                        {move || t(locale.get(), "settings.nav.specialists")}</button>
                    <button class:active=move || settings_section.get()=="memory"
                        on:click=move |_| go_settings_section.call("memory".into())>
                        {move || t(locale.get(), "settings.nav.memory")}</button>
                    <button class:active=move || settings_section.get()=="skills"
                        on:click=move |_| go_settings_section.call("skills".into())>
                        {move || t(locale.get(), "settings.nav.skills")}</button>
                    <button class:active=move || settings_section.get()=="connections"
                        on:click=move |_| go_settings_section.call("connections".into())>
                        {move || t(locale.get(), "settings.nav.connections")}</button>
                </div>
            </div>
            <div class="settings-content">
                {move || {
                    let sec = settings_section.get();
                    let loc = locale.get();
                    let parent = settings_section_label(loc, &sec);
                    let open_conn_name = open_conn_key.get().and_then(|k| {
                        connectors.get().and_then(|v| {
                            v.connectors.into_iter().find(|c| c.key == k).map(|c| c.name)
                        })
                    });
                    let sub = settings_subpage_label(
                        loc,
                        &sec,
                        model_form.get().as_ref(),
                        conn_form.get().as_ref(),
                        open_conn_name.as_deref(),
                        memory_selected.get().as_deref(),
                        specialist_form.get().as_ref(),
                    );
                    view! {
                        <div class="settings-head">
                            <div class="settings-head-main">
                                {sub.is_some().then(|| view! {
                                    <button type="button" class="settings-head-back"
                                        title=move || t(locale.get(), "settings.back")
                                        on:click=move |_| close_settings_subpage.call(())>{compose_icon("chevron-left")}</button>
                                })}
                                {move || if let Some(child) = sub.clone() {
                                    view! {
                                        <div class="settings-breadcrumb">
                                            <button type="button" class="settings-crumb-link"
                                                on:click=move |_| close_settings_subpage.call(())>{parent.clone()}</button>
                                            <span class="settings-crumb-sep">"›"</span>
                                            <span class="settings-crumb-current">{child}</span>
                                        </div>
                                    }.into_view()
                                } else {
                                    view! { <h2>{parent.clone()}</h2> }.into_view()
                                }}
                            </div>
                            <button type="button" class="settings-head-close icon-btn"
                                title=move || t(locale.get(), "settings.cancel")
                                on:click=move |_| show_settings.set(false)>{compose_icon("close")}</button>
                        </div>
                    }
                }}
                {move || (settings_section.get() == "general").then(|| view! {
                    <div class="settings-pane">
                        <div class="settings-form-grid">
                        <label class="span-2">{move || t(locale.get(), "settings.language")}
                            <select
                                on:change=move|ev| {
                                    let code = dom_value(&ev);
                                    let loc = Locale::from_code(&code);
                                    locale.set(loc);
                                    set_document_lang(loc);
                                    settings.update(|s| s.locale = code);
                                }
                                prop:value=move || locale.get().code().to_string()>
                                <option value="en">{move || t(locale.get(), "settings.language.en")}</option>
                                <option value="zh">{move || t(locale.get(), "settings.language.zh")}</option>
                            </select>
                        </label>
                        <label class="span-2">{move || t(locale.get(), "settings.workspace_dir")}
                            <input class="settings-path-input" on:input=move|ev| settings.update(|s| {
                                    s.workspace_dir = event_target_input(&ev).value();
                                })
                                prop:value={move || settings.get().workspace_dir}
                                placeholder=move || bootstrap.get().map(|b| b.workspace).unwrap_or_default() />
                        </label>
                        </div>
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status"
                                class:ok=move || ok
                                class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                                <button type="button" disabled=move || settings_busy.get() on:click=move |ev| check_updates.call(ev)>{move || t(locale.get(), "settings.check_updates")}</button>
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "models").then(|| {
                    if model_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "settings.provider")}
                                            <select data-testid="settings-provider"
                                                on:change=move|ev| {
                                                    let p = dom_value(&ev);
                                                    model_form.update(|o| if let Some(o)=o {
                                                        let (api_url, model) = provider_defaults(&p);
                                                        o.provider = provider_value(&p).into();
                                                        o.api_url = api_url.into();
                                                        o.model = model.into();
                                                    });
                                                }
                                                prop:value=move || model_form.get().map(|f| provider_value(&f.provider).to_string()).unwrap_or_else(|| "openai".into())>
                                                <option value="openai">{move || t(locale.get(), "settings.provider.openai")}</option>
                                                <option value="openai_responses">{move || t(locale.get(), "settings.provider.openai_responses")}</option>
                                                <option value="anthropic">{move || t(locale.get(), "settings.provider.anthropic")}</option>
                                            </select>
                                        </label>
                                        <label class="span-2">{move || t(locale.get(), "settings.api_url")}
                                            <input prop:value=move || model_form.get().map(|f| f.api_url.clone()).unwrap_or_default()
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.api_url = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(), "settings.label")}
                                            <input prop:value=move || model_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.label_ph")
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.label = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(), "settings.model")}
                                            <input prop:value=move || model_form.get().map(|f| f.model.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.model_ph")
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.model = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(), "settings.max_tokens")}
                                            <input type="number" min="16" step="1"
                                                on:input=move|ev| model_form.update(|o| if let Some(o)=o {
                                                    o.max_tokens = dom_value(&ev).parse().unwrap_or(0);
                                                })
                                                prop:value=move || model_form.get().map(|f| f.max_tokens.to_string()).unwrap_or_else(|| "8192".into()) />
                                        </label>
                                        <label>{move || t(locale.get(), "settings.reasoning_effort")}
                                            <select
                                                on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                    let v = dom_value(&ev);
                                                    o.reasoning_effort = if v == "default" { String::new() } else { v };
                                                })
                                                prop:value=move || {
                                                    let v = model_form.get().map(|f| f.reasoning_effort).unwrap_or_default();
                                                    if v.is_empty() { "default".to_string() } else { v }
                                                }>
                                                <option value="default">{move || t(locale.get(), "settings.reasoning_effort.default")}</option>
                                                <option value="none">"none"</option>
                                                <option value="minimal">"minimal"</option>
                                                <option value="low">"low"</option>
                                                <option value="medium">"medium"</option>
                                                <option value="high">"high"</option>
                                                <option value="xhigh">"xhigh"</option>
                                            </select>
                                        </label>
                                        <div class="span-2 settings-form-grid">
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || model_form.get().map(|f| f.supports_vision).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.supports_vision = event_target_checked(&ev);
                                                        if !o.supports_vision {
                                                            o.use_for_vision = false;
                                                        }
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.supports_vision")}</span>
                                            </label>
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || model_form.get().map(|f| f.use_for_vision).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.use_for_vision = event_target_checked(&ev);
                                                        if o.use_for_vision {
                                                            o.supports_vision = true;
                                                        }
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.use_for_vision")}</span>
                                            </label>
                                            <span class="hint span-2">{move || t(locale.get(), "settings.vision_hint")}</span>
                                        </div>
                                        <label class="span-2">{move || t(locale.get(), "settings.api_key")}
                                            <input type="password" prop:value=move || model_form_key.get()
                                                placeholder=move || {
                                                    let Some(id) = model_form.get().and_then(|f| f.id) else { return String::new(); };
                                                    if models.get().iter().any(|m| m.id == id && m.has_api_key) {
                                                        t(locale.get(), "settings.stored_key").to_string()
                                                    } else {
                                                        String::new()
                                                    }
                                                }
                                                autocomplete="new-password"
                                                on:input=move |ev| model_form_key.set(event_target_input(&ev).value()) /></label>
                                    </div>
                                    <span class="hint">{move || t(locale.get(), "settings.tip")}</span>
                                    {move || model_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                            <button type="button" disabled=move || settings_busy.get() on:click=move |ev| validate_model_form.call(ev)>{move || t(locale.get(), "settings.validate")}</button>
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(), "settings.cancel")}</button>
                                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_model_form.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-list">
                            <div class="settings-toolbar settings-toolbar-end">
                                <span class="settings-filter">{move || {
                                    let n = models.get().len();
                                    format!("{} ({n})", t(locale.get(), "settings.nav.models"))
                                }}</span>
                                <button type="button" class="settings-add-btn" on:click=move |_| {
                                    model_form.set(Some(new_model_form()));
                                    model_form_key.set(String::new());
                                    model_form_msg.set(None);
                                }>{move || t(locale.get(), "models.add")}</button>
                            </div>
                            <div class="settings-list">
                                <For each=move || models.get() key=|m| m.id.clone() let:m>
                                    {
                                        let pick_id = m.id.clone();
                                        let del_id = m.id.clone();
                                        let edit = m.clone();
                                        let is_active = m.active;
                                        let can_delete = models.get().len() > 1;
                                        let show_sub = !m.model.is_empty() && m.model != m.label;
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                class:settings-list-row-active=is_active
                                                on:click=move |_| {
                                                    model_form.set(Some(profile_to_form(&edit)));
                                                    model_form_key.set(String::new());
                                                    model_form_msg.set(None);
                                                }>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">
                                                        {m.label.clone()}
                                                        {m.use_for_vision.then(|| view! { <span class="settings-active-mark" title="vision">" vision"</span> })}
                                                    </span>
                                                    {show_sub.then(|| view! {
                                                        <span class="settings-list-sub">{m.model.clone()}</span>
                                                    })}
                                                </div>
                                                <div class="settings-list-actions">
                                                    {is_active.then(|| view! {
                                                        <span class="settings-active-mark" title="active">"✓"</span>
                                                    })}
                                                    {(can_delete && !is_active).then(|| { let id = del_id.clone(); view! {
                                                        <button class="settings-list-remove" type="button" title=move || t(locale.get(), "models.remove")
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                let id = id.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                    if let Ok(v) = invoke_checked("remove_model", arg).await {
                                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                            models.set(list);
                                                                        }
                                                                    }
                                                                });
                                                            }>{compose_icon("close")}</button>
                                                    }})}
                                                    {(!is_active).then(|| { let id = pick_id.clone(); view! {
                                                        <button class="settings-list-use" type="button"
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                let id = id.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                    if let Ok(v) = invoke_checked("set_active_model", arg).await {
                                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                            models.set(list);
                                                                        }
                                                                    }
                                                                });
                                                            }>{move || t(locale.get(), "models.use")}</button>
                                                    }})}
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                            {move || models.get().is_empty().then(|| view! {
                                <p class="model-empty-hint">{move || t(locale.get(), "models.empty")}</p>
                            })}
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "specialists").then(|| {
                    if specialist_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "specialists.name")}
                                            <input prop:value=move || specialist_form.get().map(|f| f.name.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.name = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "specialists.description")}
                                            <textarea prop:value=move || specialist_form.get().map(|f| f.description.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.description = event_target_value(&ev); })></textarea></label>
                                        <label class="span-2">{move || t(locale.get(), "specialists.instructions")}
                                            <textarea rows="6"
                                                prop:disabled=move || specialist_form.get().map(|f| f.builtin).unwrap_or(false)
                                                prop:value=move || specialist_form.get().map(|f| f.instructions.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.instructions = event_target_value(&ev); })></textarea></label>
                                        {move || specialist_form.get().filter(|f| f.builtin).map(|_| view! {
                                            <span class="hint span-2">{move || t(locale.get(), "specialists.builtin_locked")}</span>
                                        })}
                                        {move || specialist_form.get().filter(|f| !f.builtin).map(|_| view! {
                                            <span class="hint span-2">{move || t(locale.get(), "specialists.instructions.hint")}</span>
                                        })}
                                        <label class="span-2">{move || t(locale.get(), "specialists.model")}
                                            <select
                                                on:change=move |ev| specialist_form.update(|o| if let Some(o)=o { o.model_id = dom_value(&ev); })
                                                prop:value=move || specialist_form.get().map(|f| f.model_id.clone()).unwrap_or_default()>
                                                <option value="">{move || t(locale.get(), "specialists.model.follow")}</option>
                                                {move || models.get().into_iter().map(|m| {
                                                    view! { <option value=m.id.clone()>{m.label.clone()}</option> }
                                                }).collect_view()}
                                            </select>
                                        </label>
                                        <div class="span-2 settings-form-grid">
                                            <span class="span-2">{move || t(locale.get(), "specialists.skills")}</span>
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || specialist_form.get().map(|f| f.skills.is_none()).unwrap_or(true)
                                                    on:change=move |ev| specialist_form.update(|o| if let Some(o)=o {
                                                        o.skills = if event_target_checked(&ev) { None } else { Some(vec![]) };
                                                    }) />
                                                <span>{move || t(locale.get(), "specialists.inherit")}</span>
                                            </label>
                                            {move || specialist_form.get().filter(|f| f.skills.is_some()).map(|_| view! {
                                                <span class="hint span-2">{move || t(locale.get(), "specialists.skills.whitelist_hint")}</span>
                                            })}
                                            {move || {
                                                let whitelist = specialist_form.get().and_then(|f| f.skills);
                                                whitelist.map(|list| {
                                                    let list = std::rc::Rc::new(list);
                                                    view! {
                                                        <div class="span-2 settings-form-grid">
                                                            {move || skills_list.get().into_iter().map(|s| {
                                                                let name = s.name.clone();
                                                                let name_checked = name.clone();
                                                                let checked = list.contains(&name);
                                                                view! {
                                                                    <label class="settings-check">
                                                                        <input type="checkbox"
                                                                            prop:checked=checked
                                                                            on:change=move |ev| {
                                                                                let on = event_target_checked(&ev);
                                                                                let name = name_checked.clone();
                                                                                specialist_form.update(|o| if let Some(o) = o {
                                                                                    let mut cur = o.skills.clone().unwrap_or_default();
                                                                                    if on {
                                                                                        if !cur.contains(&name) { cur.push(name); }
                                                                                    } else {
                                                                                        cur.retain(|n| n != &name);
                                                                                    }
                                                                                    o.skills = Some(cur);
                                                                                });
                                                                            } />
                                                                        <span>{name}</span>
                                                                    </label>
                                                                }
                                                            }).collect_view()}
                                                        </div>
                                                    }
                                                })
                                            }}
                                        </div>
                                    </div>
                                    {move || model_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(), "settings.cancel")}</button>
                                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_specialist_form.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-list">
                            <div class="settings-toolbar settings-toolbar-end">
                                <span class="settings-filter">{move || {
                                    let n = specialists.get().len();
                                    format!("{} ({n})", t(locale.get(), "settings.nav.specialists"))
                                }}</span>
                                <details class="settings-add-menu">
                                    <summary>{move || t(locale.get(), "specialists.add")}</summary>
                                    <button type="button" on:click=move |ev| {
                                        close_details_ancestor(&ev);
                                        model_form_msg.set(None);
                                        specialist_form.set(Some(Specialist {
                                            id: String::new(),
                                            name: String::new(),
                                            icon: "review".into(),
                                            color: "clay".into(),
                                            description: String::new(),
                                            instructions: String::new(),
                                            model_id: String::new(),
                                            skills: None,
                                            connectors: None,
                                            builtin: false,
                                        }));
                                    }>{move || t(locale.get(), "specialists.add.scratch")}</button>
                                        <button type="button" on:click=move |ev| start_specialist_chat.call(ev)>
                                        {move || t(locale.get(), "specialists.add.chat")}
                                    </button>
                                </details>
                            </div>
                            <div class="conn-group-label">{move || t(locale.get(), "specialists.builtin")}</div>
                            <div class="settings-list">
                                <For each=move || { specialists.get().into_iter().filter(|s| s.builtin).collect::<Vec<_>>() } key=|s| s.id.clone() let:s>
                                    {
                                        let edit = s.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| {
                                                    model_form_msg.set(None);
                                                    specialist_form.set(Some(edit.clone()));
                                                }>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{s.name.clone()}</span>
                                                    {(!s.description.is_empty()).then(|| view! {
                                                        <span class="settings-list-sub">{s.description.clone()}</span>
                                                    })}
                                                </div>
                                                <div class="settings-list-actions">
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                            <div class="conn-group-label">{move || t(locale.get(), "specialists.custom")}</div>
                            <div class="settings-list">
                                <For each=move || { specialists.get().into_iter().filter(|s| !s.builtin).collect::<Vec<_>>() } key=|s| s.id.clone() let:s>
                                    {
                                        let edit = s.clone();
                                        let del_id = s.id.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| {
                                                    model_form_msg.set(None);
                                                    specialist_form.set(Some(edit.clone()));
                                                }>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{s.name.clone()}</span>
                                                    {(!s.description.is_empty()).then(|| view! {
                                                        <span class="settings-list-sub">{s.description.clone()}</span>
                                                    })}
                                                </div>
                                                <div class="settings-list-actions">
                                                    {(!s.builtin).then(|| { let id = del_id.clone(); view! {
                                                        <button class="settings-list-remove" type="button" title=move || t(locale.get(), "specialists.remove")
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                remove_specialist.call(id.clone());
                                                            }>{compose_icon("close")}</button>
                                                    }})}
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "memory").then(|| {
                    if memory_selected.get().is_some() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                {move || memory_selected.get().map(|name| {
                                    let name_del = name.clone();
                                    let name_save = name.clone();
                                    view! {
                                        <div class="memory-editor-inner memory-editor-page">
                                            <textarea class="memory-editor-text" prop:value=move || memory_editor.get()
                                                on:input=move |ev| memory_editor.set(event_target_value(&ev))></textarea>
                                            {move || memory_msg.get().map(|(ok, text)| view! {
                                                <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                            })}
                                            <div class="row settings-footer">
                                                <button type="button" class="memory-delete-btn"
                                                    on:click=move |_| {
                                                        let n = name_del.clone();
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "name": n })).unwrap();
                                                            if let Ok(files) = invoke_checked("delete_memory_file", arg).await {
                                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(files) {
                                                                    memory_view.update(|o| if let Some(o)=o { o.files = list; });
                                                                    close_settings_subpage.call(());
                                                                }
                                                            }
                                                        });
                                                    }>{move || t(locale.get(), "memory.delete")}</button>
                                                <button type="button" class="primary" on:click=move |_| {
                                                    let n = name_save.clone();
                                                    let content = memory_editor.get();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "name": n, "content": content })).unwrap();
                                                        match invoke_checked("write_memory_file", arg).await {
                                                            Ok(v) => {
                                                                if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                                    memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                                }
                                                                memory_msg.set(Some((true, t(locale.get(), "memory.save").into())));
                                                            }
                                                            Err(e) => memory_msg.set(Some((false, js_error_text(e)))),
                                                        }
                                                    });
                                                }>{move || t(locale.get(), "memory.save")}</button>
                                            </div>
                                        </div>
                                    }
                                })}
                            </div>
                        }.into_view()
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-memory">
                            <div class="settings-toolbar settings-toolbar-end memory-toolbar">
                                <span class="settings-filter">{move || {
                                    let n = memory_view.get().map(|v| v.files.len()).unwrap_or(0);
                                    format!("{} ({n})", t(locale.get(), "memory.notes"))
                                }}</span>
                                <div class="memory-toolbar-actions">
                                    <label class="toggle" title=move || t(locale.get(), "settings.nav.memory")>
                                        <input type="checkbox" prop:checked=move || memory_view.get().map(|v| v.enabled).unwrap_or(true)
                                            on:change=move |ev| {
                                                let on = event_target_checked(&ev);
                                                spawn_local(async move {
                                                    let arg = to_value(&serde_json::json!({ "enabled": on })).unwrap();
                                                    if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                        if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                            memory_view.set(Some(view));
                                                        }
                                                    }
                                                });
                                            } />
                                        <span class="toggle-track" aria-hidden="true"></span>
                                    </label>
                                    <button type="button" class="settings-add-btn" on:click=move |_| {
                                        if let Some(today) = memory_view.get().map(|v| v.today_file) {
                                            load_memory_file.call(today);
                                        }
                                    }>{move || t(locale.get(), "memory.add")}</button>
                                    <button type="button" class="memory-clear-btn" on:click=move |_| {
                                        spawn_local(async move {
                                            let v = invoke("clear_memory", JsValue::UNDEFINED).await;
                                            if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                memory_selected.set(None);
                                                memory_editor.set(String::new());
                                            }
                                        });
                                    }>{move || t(locale.get(), "memory.clear_all")}</button>
                                </div>
                            </div>
                            {move || {
                                let off = memory_view.get().map(|v| !v.enabled).unwrap_or(false);
                                off.then(|| view! {
                                <div class="memory-off-banner">
                                    <span>{move || t(locale.get(), "memory.off_banner")}</span>
                                    <button type="button" class="settings-add-btn" on:click=move |_| {
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({ "enabled": true })).unwrap();
                                            if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                    memory_view.set(Some(view));
                                                }
                                            }
                                        });
                                    }>{move || t(locale.get(), "memory.turn_on")}</button>
                                </div>
                                })
                            }}
                            <div class="settings-list memory-file-list">
                                <For each=move || memory_view.get().map(|v| v.files).unwrap_or_default() key=|f| f.name.clone() let:f>
                                    {
                                        let pick = f.name.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| load_memory_file.call(pick.clone())>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{f.name.clone()}</span>
                                                    <span class="settings-list-sub">{format_bytes(f.bytes)}</span>
                                                </div>
                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                            {move || memory_view.get().map(|v| v.files.is_empty().then(|| view! {
                                <p class="memory-empty">{move || t(locale.get(), "memory.empty")}</p>
                            })).into_view()}
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "skills").then(|| view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar">
                            <span class="settings-filter">{move || {
                                let q = skills_search.get().trim().to_lowercase();
                                let tag = skill_filter_tag.get();
                                let n = skills_list.get().iter().filter(|s| {
                                    skill_matches_filter(s, &tag, &q)
                                }).count();
                                format!("{} ({n})", t(locale.get(), "skills.all"))
                            }}</span>
                            <input class="settings-search" type="search"
                                placeholder=move || t(locale.get(), "skills.search_ph")
                                prop:value=move || skills_search.get()
                                on:input=move |ev| skills_search.set(event_target_input(&ev).value()) />
                            <button type="button" on:click=move |_| set_visible_skills_enabled.call(true)>
                                {move || t(locale.get(), "skills.enable_visible")}
                            </button>
                            <button type="button" on:click=move |_| set_visible_skills_enabled.call(false)>
                                {move || t(locale.get(), "skills.disable_visible")}
                            </button>
                            <details class="settings-add-menu">
                                <summary>{move || t(locale.get(), "skills.add")}</summary>
                                <button type="button" on:click=move |_| {
                                    spawn_local(async move {
                                        let picked = invoke("pick_skill_source", JsValue::UNDEFINED).await;
                                        if let Some(path) = picked.as_string() {
                                            install_skill_from.call(path);
                                        }
                                    });
                                }>{move || t(locale.get(), "skills.add_file")}</button>
                                <button type="button" on:click=move |_| {
                                    spawn_local(async move {
                                        let picked = invoke("pick_directory", JsValue::UNDEFINED).await;
                                        if let Some(path) = picked.as_string() {
                                            install_skill_from.call(path);
                                        }
                                    });
                                }>{move || t(locale.get(), "skills.add_folder")}</button>
                            </details>
                        </div>
                        <div class="skill-tags-filter">
                            <button class:active=move || skill_filter_tag.get().is_empty()
                                on:click=move |_| skill_filter_tag.set(String::new())>
                                {move || t(locale.get(), "skills.all")}
                            </button>
                            <button class:active=move || skill_filter_tag.get() == "__untagged"
                                on:click=move |_| skill_filter_tag.set("__untagged".into())>
                                {move || t(locale.get(), "skills.untagged")}
                            </button>
                            {move || {
                                let tags = skills_list.get().iter()
                                    .flat_map(|s| s.tags.iter().cloned())
                                    .collect::<BTreeSet<_>>()
                                    .into_iter()
                                    .collect::<Vec<_>>();
                                tags.into_iter().map(|tag| {
                                    let active_tag = tag.clone();
                                    let set_tag = tag.clone();
                                    view! {
                                        <button class:active=move || skill_filter_tag.get() == active_tag
                                            on:click=move |_| skill_filter_tag.set(set_tag.clone())>
                                            {tag}
                                        </button>
                                    }
                                }).collect_view()
                            }}
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                        {move || skills_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="settings-list">
                            <For each=move || {
                                let q = skills_search.get().trim().to_lowercase();
                                let tag = skill_filter_tag.get();
                                skills_list.get().into_iter().filter(|s| {
                                    skill_matches_filter(s, &tag, &q)
                                }).collect::<Vec<_>>()
                            } key=|s| format!("{}:{}:{}", s.name, s.enabled, join_tags(&s.tags)) let:s>
                                {
                                    let name_toggle = s.name.clone();
                                    let name_remove = s.name.clone();
                                    let name_tags = s.name.clone();
                                    let enabled = s.enabled;
                                    let builtin = s.builtin;
                                    let tags_text = join_tags(&s.tags);
                                    let tags_cb = save_skill_tags.clone();
                                    view! {
                                        <div class="settings-list-row" data-skill-name=s.name.clone()>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{s.name.clone()}</span>
                                                {(!s.description.is_empty() && s.description != ">").then(|| {
                                                    let desc = s.description.clone();
                                                    view! { <span class="settings-list-sub">{desc}</span> }
                                                })}
                                                <input class="skill-tags-input"
                                                    prop:value=tags_text
                                                    prop:placeholder=move || t(locale.get(), "skills.tags_placeholder")
                                                    on:change=move |ev| tags_cb.call((name_tags.clone(), event_target_value(&ev))) />
                                            </div>
                                            <div class="settings-list-actions">
                                                {(!builtin).then(|| { let n = name_remove.clone(); view! {
                                                    <button class="settings-list-remove" type="button" title="remove" on:click=move |_| {
                                                        let n = n.clone();
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "name": n })).unwrap();
                                                            let _ = invoke_checked("remove_skill", arg).await;
                                                            refresh_skills.call(());
                                                        });
                                                    }>{compose_icon("close")}</button>
                                                }})}
                                                <label class="toggle">
                                                    <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                        let n = name_toggle.clone();
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "name": n, "enabled": on })).unwrap();
                                                            let _ = invoke_checked("set_skill_enabled", arg).await;
                                                            refresh_skills.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "credentials").then(|| view! {
                    <div class="settings-pane">
                        <p class="settings-note">{move || t(locale.get(), "cred.desc")}</p>
                        {CRED_GROUPS.iter().map(|g| view! {
                            <div class="conn-group-label">{move || t(locale.get(), g.name_key)}</div>
                            <div class="settings-form-grid">
                                {g.fields.iter().map(|f| {
                                    let id = f.id;
                                    let stored = move || cred_status.get().get(id).copied().unwrap_or(false);
                                    view! {
                                        <label class="span-2">
                                            <span class="cred-field-head">
                                                <span>{move || format!("{} — {}", t(locale.get(), f.label_key),
                                                    if stored() { t(locale.get(), "cred.stored") } else { t(locale.get(), "cred.not_stored") })}</span>
                                                {move || stored().then(|| view! {
                                                    <button type="button" class="linklike" on:click=move |_| {
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "id": id, "value": "" })).unwrap();
                                                            match invoke_checked("set_credential", arg).await {
                                                                Ok(_) => {
                                                                    cred_inputs.update(|m| { m.remove(id); });
                                                                    cred_status.update(|m| { m.insert(id.into(), false); });
                                                                    cred_msg.set(Some((true, t(locale.get(), "cred.cleared").into())));
                                                                }
                                                                Err(e) => cred_msg.set(Some((false, localize_backend(locale.get(), &js_error_text(e))))),
                                                            }
                                                        });
                                                    }>{move || t(locale.get(), "cred.clear")}</button>
                                                })}
                                            </span>
                                            <input type=if f.secret { "password" } else { "text" }
                                                placeholder=move || if stored() { t(locale.get(), "settings.stored_key").to_string() } else { String::new() }
                                                prop:value=move || cred_inputs.get().get(id).cloned().unwrap_or_default()
                                                on:input=move |ev| { let v = event_target_input(&ev).value(); cred_inputs.update(|m| { m.insert(id.into(), v); }); } />
                                        </label>
                                    }
                                }).collect_view()}
                            </div>
                            <p class="settings-note">{move || t(locale.get(), g.hint_key)}</p>
                        }).collect_view()}
                        {move || cred_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=move || ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                            <button type="button" class="primary" on:click=move |_| {
                                // Save every field that was edited (non-empty input); blank inputs
                                // leave a stored key untouched (placeholder communicates this).
                                let edits: Vec<(String, String)> = cred_inputs.get().into_iter()
                                    .filter(|(_, v)| !v.trim().is_empty()).collect();
                                if edits.is_empty() { return; }
                                spawn_local(async move {
                                    let mut ok_all = true;
                                    for (id, value) in edits {
                                        let arg = to_value(&serde_json::json!({ "id": id, "value": value })).unwrap();
                                        if let Err(e) = invoke_checked("set_credential", arg).await {
                                            ok_all = false;
                                            cred_msg.set(Some((false, localize_backend(locale.get(), &js_error_text(e)))));
                                            break;
                                        }
                                    }
                                    if ok_all {
                                        cred_inputs.set(std::collections::HashMap::new());
                                        cred_msg.set(Some((true, t(locale.get(), "cred.saved").into())));
                                    }
                                    let v = invoke("credential_status", JsValue::UNDEFINED).await;
                                    if let Ok(pairs) = serde_wasm_bindgen::from_value::<Vec<(String, bool)>>(v) {
                                        cred_status.set(pairs.into_iter().collect());
                                    }
                                });
                            }>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "permissions").then(|| view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar settings-toolbar-end">
                            <span class="settings-filter">{move || {
                                format!("{} ({})", t(locale.get(), "settings.nav.permissions"), approval_grants.get().len())
                            }}</span>
                            <button type="button" class="settings-add-btn"
                                disabled=move || approval_grants.get().is_empty()
                                on:click=move |_| {
                                    spawn_local(async move {
                                        let _ = invoke_checked("revoke_all_approval_grants", JsValue::UNDEFINED).await;
                                        refresh_approval_grants.call(());
                                    });
                                }>{move || t(locale.get(), "permissions.revoke_all")}</button>
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "permissions.note")}</p>
                        {move || approval_grants.get().is_empty().then(|| view! {
                            <div class="settings-status">{move || t(locale.get(), "permissions.empty")}</div>
                        })}
                        <div class="settings-list">
                            {move || approval_grants.get().into_iter().map(|row| {
                                let scope_label = match row.scope.as_str() {
                                    "session" => "permissions.scope.session",
                                    "project" => "permissions.scope.project",
                                    "global" => "permissions.scope.global",
                                    _ => "approval.scope.once",
                                };
                                let subtitle = format!("{} - {}", row.kind, row.target);
                                let scope = row.scope.clone();
                                let kind = row.kind.clone();
                                let target = row.target.clone();
                                let session_id = row.session_id.clone();
                                let project_id = row.project_id.clone();
                                view! {
                                    <div class="settings-list-row">
                                        <div class="settings-list-main">
                                            <span class="settings-list-title">{row.label}</span>
                                            <span class="settings-list-sub">{subtitle}</span>
                                        </div>
                                        <div class="settings-list-actions">
                                            <span class="badge">{move || t(locale.get(), scope_label)}</span>
                                            <button class="settings-list-remove" type="button"
                                                title=move || t(locale.get(), "permissions.revoke")
                                                on:click=move |_| {
                                                    let scope = scope.clone();
                                                    let kind = kind.clone();
                                                    let target = target.clone();
                                                    let session_id = session_id.clone();
                                                    let project_id = project_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({
                                                            "scope": scope,
                                                            "kind": kind,
                                                            "target": target,
                                                            "sessionId": session_id,
                                                            "projectId": project_id,
                                                        })).unwrap();
                                                        let _ = invoke_checked("revoke_approval_grant", arg).await;
                                                        refresh_approval_grants.call(());
                                                    });
                                                }>{compose_icon("close")}</button>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "connections").then(|| {
                    if conn_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form">
                                    <label>{move || t(locale.get(),"conn.name")}
                                        <input prop:value=move || conn_form.get().map(|f| f.name.clone()).unwrap_or_default()
                                            on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.name = event_target_input(&ev).value(); }) /></label>
                                    <label>{move || t(locale.get(),"conn.kind")}
                                        <select prop:value=move || conn_form.get().map(|f| f.kind.clone()).unwrap_or_else(|| "stdio".into())
                                            on:change=move |ev| { let k = dom_value(&ev); conn_form.update(|o| if let Some(o)=o { o.kind = k; }); }>
                                            <option value="stdio">{move || t(locale.get(),"conn.kind.stdio")}</option>
                                            <option value="http">{move || t(locale.get(),"conn.kind.http")}</option>
                                        </select></label>
                                    {move || (conn_form_kind.get() == "stdio").then(|| view!{
                                        <label>{move || t(locale.get(),"conn.command")}
                                            <input prop:value=move || conn_form.get().map(|f| f.command.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.command = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(),"conn.args")}
                                            <input placeholder="arg1 arg2" prop:value=move || conn_form.get().map(|f| f.args.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.args = event_target_input(&ev).value(); }) /></label>
                                    })}
                                    {move || (conn_form_kind.get() == "http").then(|| view!{
                                        <label>{move || t(locale.get(),"conn.url")}
                                            <input placeholder="https://host/mcp" prop:value=move || conn_form.get().map(|f| f.url.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.url = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(),"conn.headers")}
                                            <input placeholder="Authorization: Bearer xxx" prop:value=move || conn_form.get().map(|f| f.headers.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.headers = event_target_input(&ev).value(); }) /></label>
                                    })}
                                    {move || conn_test_msg.get().map(|(ok,msg)| view!{
                                        <div class="settings-status" class:ok=ok class:fail=move||!ok>{msg}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                            spawn_local(async move {
                                                let conn = build_conn_json(&f, false);
                                                match invoke_checked("test_mcp_connection", to_value(&serde_json::json!({"conn": conn})).unwrap()).await {
                                                    Ok(v) => match serde_wasm_bindgen::from_value::<Vec<ConnectorTool>>(v) {
                                                        Ok(tools) => {
                                                            let n = tools.len();
                                                            if let Some(id) = f.id.clone() {
                                                                custom_conn_tools.update(|m| { m.insert(id, tools); });
                                                            }
                                                            conn_test_msg.set(Some((true, format!("OK — {n} tools"))));
                                                        }
                                                        Err(e) => conn_test_msg.set(Some((false, e.to_string()))),
                                                    },
                                                    Err(e) => conn_test_msg.set(Some((false, js_error_text(e)))),
                                                }
                                            });
                                        }>{move || t(locale.get(),"conn.test")}</button>
                                        <button type="button" on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(),"settings.cancel")}</button>
                                        <button type="button" class="primary" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                            spawn_local(async move {
                                                let editing = f.id.is_some();
                                                let conn = build_conn_json(&f, true);
                                                let cmd = if editing { "update_mcp_connection" } else { "add_mcp_connection" };
                                                if invoke_checked(cmd, to_value(&serde_json::json!({"conn": conn})).unwrap()).await.is_ok() {
                                                    conn_form.set(None); conn_test_msg.set(None); refresh_conns.call(());
                                                }
                                            });
                                        }>{move || t(locale.get(),"settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else if open_conn_key.get().is_some() {
                        // Level 2 — connector detail. Bundled connectors have static approval controls;
                        // custom MCP tools are discovered on demand.
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                                {move || {
                                    let key = open_conn_key.get();
                                    let conn = key.and_then(|k| connectors.get().and_then(|v| v.connectors.into_iter().find(|c| c.key == k)));
                                    conn.map(|c| {
                                        let is_custom = c.kind == "custom";
                                        let skip_on = c.skip_approvals;
                                        let key_skip = c.key.clone();
                                        let tools = if is_custom {
                                            custom_conn_tools.get().get(&c.key).cloned().unwrap_or_default()
                                        } else {
                                            c.tools.clone()
                                        };
                                        let loading = is_custom && custom_conn_tools_loading.get().contains(&c.key);
                                        let error = if is_custom {
                                            custom_conn_tool_errors.get().get(&c.key).cloned()
                                        } else {
                                            None
                                        };
                                        let has_error = error.is_some();
                                        view! {
                                            {(!is_custom).then(|| view! {
                                                <div class="settings-list">
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.skip_approvals")}</span>
                                                            <span class="settings-list-sub">{move || t(locale.get(), "conn.skip_approvals.desc")}</span>
                                                        </div>
                                                        <label class="toggle">
                                                            <input type="checkbox" prop:checked=skip_on on:change=move |ev| {
                                                                let key = key_skip.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_connector_skip_approvals", arg).await;
                                                                    refresh_conns.call(());
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                    </div>
                                                </div>
                                            })}
                                            <div class="conn-group-label">{move || t(locale.get(), "conn.tools")}</div>
                                            {loading.then(|| view! {
                                                <div class="settings-status">{move || t(locale.get(), "conn.tools_loading")}</div>
                                            })}
                                            {error.map(|msg| view! {
                                                <div class="settings-status fail">{move || tf(locale.get(), "conn.tools_failed", &[("msg", &msg)])}</div>
                                            })}
                                            {(!loading && !has_error && tools.is_empty()).then(|| view! {
                                                <div class="settings-status">{move || t(locale.get(), "conn.no_tools")}</div>
                                            })}
                                            <div class="settings-list">
                                                {tools.iter().map(|tool| {
                                                    let name = tool.name.clone();
                                                    let mode = tool.mode.clone();
                                                    let desc = tool.description.clone();
                                                    let seg = |m: &'static str, glyph: &'static str, key: &'static str| {
                                                        let name2 = name.clone();
                                                        let active = mode.as_str() == m;
                                                        view! {
                                                            <button type="button" class=format!("approval-btn approval-{m}") class:active=active
                                                                disabled=skip_on
                                                                title=move || t(locale.get(), key)
                                                                on:click=move |_| {
                                                                    let name = name2.clone();
                                                                    spawn_local(async move {
                                                                        let arg = to_value(&serde_json::json!({ "tool": name, "mode": m })).unwrap();
                                                                        let _ = invoke_checked("set_tool_approval", arg).await;
                                                                        refresh_conns.call(());
                                                                    });
                                                                }>{glyph}</button>
                                                        }
                                                    };
                                                    view! {
                                                        <div class="settings-list-row">
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">{tool.name.clone()}</span>
                                                                {(!desc.is_empty()).then(|| view! {
                                                                    <span class="settings-list-sub">{desc.clone()}</span>
                                                                })}
                                                            </div>
                                                            {(!is_custom).then(|| view! {
                                                                <div class="approval-seg" class:disabled=skip_on>
                                                                    {seg("allow", "✓", "conn.approval.allow")}
                                                                    {seg("ask", "?", "conn.approval.ask")}
                                                                    {seg("deny", "✕", "conn.approval.deny")}
                                                                </div>
                                                            })}
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }
                                    })
                                }}
                            </div>
                        }.into_view()
                    } else {
                        view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar settings-toolbar-end">
                            <span class="settings-filter">{move || {
                                let nb = connectors.get().map(|v| v.connectors.iter().filter(|c| c.kind == "bundled").count()).unwrap_or(0);
                                let nc = conns_view.get().map(|v| v.connections.len()).unwrap_or(0);
                                format!("{} ({})", t(locale.get(), "settings.nav.connections"), nb + nc)
                            }}</span>
                            <button type="button" class="settings-add-btn" on:click=move |_| {
                                conn_form.set(Some(ConnForm { kind: "stdio".into(), enabled: true, ..Default::default() }));
                                conn_test_msg.set(None);
                            }>{move || t(locale.get(), "conn.add")}</button>
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                        <div class="settings-list">
                            <div class="settings-list-row">
                                <div class="settings-list-main">
                                    <span class="settings-list-title">{move || t(locale.get(), "conn.scope")}</span>
                                    <span class="settings-list-sub">{move || {
                                        let cur = connectors.get().map(|v| v.scope).unwrap_or_else(|| "ask".into());
                                        t(locale.get(), match cur.as_str() {
                                            "full" => "conn.scope.full.desc",
                                            "auto" => "conn.scope.auto.desc",
                                            _ => "conn.scope.ask.desc",
                                        })
                                    }}</span>
                                </div>
                                <div class="approval-seg">
                                    {["ask", "auto", "full"].into_iter().map(|val| {
                                        let label_key = match val {
                                            "full" => "conn.scope.full",
                                            "auto" => "conn.scope.auto",
                                            _ => "conn.scope.ask",
                                        };
                                        let active = move || connectors.get().map(|v| v.scope).unwrap_or_else(|| "ask".into()) == val;
                                        view! {
                                            <button type="button" class=format!("approval-btn scope-seg scope-{val}") class:active=active
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "scope": val })).unwrap();
                                                        let _ = invoke_checked("set_approval_scope", arg).await;
                                                        refresh_conns.call(());
                                                    });
                                                }>{move || t(locale.get(), label_key)}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        </div>
                        <div class="conn-group-label">{move || t(locale.get(), "conn.featured")}</div>
                        <div class="settings-list">
                            <For each=move || connectors.get().map(|v| v.connectors.into_iter().filter(|c| c.kind == "bundled").collect::<Vec<_>>()).unwrap_or_default() key=|c| c.key.clone() let:c>
                                {
                                    let key_open = c.key.clone();
                                    let key_toggle = c.key.clone();
                                    let n_tools = c.tools.len();
                                    let enabled = c.enabled;
                                    view! {
                                        <div class="settings-list-row settings-list-row-link"
                                            on:click=move |_| open_conn_key.set(Some(key_open.clone()))>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{c.name.clone()}</span>
                                                <span class="settings-list-sub">{move || tf(locale.get(), "conn.tools_count", &[("n", &n_tools.to_string())])}</span>
                                            </div>
                                            <div class="settings-list-actions">
                                                <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                    <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                        let key = key_toggle.clone();
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                            let _ = invoke_checked("set_connector_enabled", arg).await;
                                                            refresh_conns.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                        {move || conns_view.get().map(|v| v.connections.len()).unwrap_or(0).gt(&0).then(|| view! {
                            <div class="conn-group-label">{move || t(locale.get(), "conn.custom")}</div>
                        })}
                        <div class="settings-list">
                            <For each=move || conns_view.get().map(|v| v.connections).unwrap_or_default() key=|c| c.id.clone() let:c>
                                {
                                    let id_del = c.id.clone();
                                    let id_toggle = c.id.clone();
                                    let id_open = c.id.clone();
                                    let row_open = c.clone();
                                    let row_edit = c.clone();
                                    let kind_badge = match &c.transport {
                                        ConnTransport::Stdio { .. } => "stdio",
                                        ConnTransport::Http { .. } => "http",
                                    };
                                    view! {
                                        <div class="settings-list-row settings-list-row-link"
                                            on:click=move |_| {
                                                open_conn_key.set(Some(id_open.clone()));
                                                load_custom_conn_tools.call(row_open.clone());
                                            }>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{c.name.clone()} <span class="badge">{kind_badge}</span></span>
                                                <span class="settings-list-sub">
                                                    {match &c.transport {
                                                        ConnTransport::Stdio { command, .. } => command.clone(),
                                                        ConnTransport::Http { url, .. } => url.clone(),
                                                    }}
                                                </span>
                                            </div>
                                            <div class="settings-list-actions">
                                                <button class="settings-list-edit" type="button"
                                                    title=move || t(locale.get(), "conn.edit")
                                                    aria-label=move || t(locale.get(), "conn.edit")
                                                    on:click=move |ev| {
                                                        ev.stop_propagation();
                                                        conn_form.set(Some(conn_form_from_row(&row_edit)));
                                                        conn_test_msg.set(None);
                                                    }>{compose_icon("edit")}</button>
                                                <button class="settings-list-remove" type="button" title="remove" on:click=move |ev| {
                                                    ev.stop_propagation();
                                                    let id = id_del.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                        let _ = invoke_checked("delete_mcp_connection", arg).await;
                                                        refresh_conns.call(());
                                                    });
                                                }>{compose_icon("close")}</button>
                                                <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                    <input type="checkbox" prop:checked=c.enabled on:change=move |ev| {
                                                        let id = id_toggle.clone();
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "id": id, "enabled": on })).unwrap();
                                                            let _ = invoke_checked("set_mcp_connection_enabled", arg).await;
                                                            refresh_conns.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                    </div>
                        }.into_view()
                    }
                })}
            </div>
        </div>
    </div>
}.into_view())
}
