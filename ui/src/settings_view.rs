use crate::app_support::{
    build_conn_json, close_details_ancestor, compose_icon, conn_form_from_row, js_error_text,
    join_tags, new_model_form, profile_to_form, settings_section_label, settings_subpage_label,
    skill_matches_filter, CRED_GROUPS,
};
use crate::bindings::{invoke, invoke_checked};
use crate::dto::*;
use crate::i18n::{localize_backend, set_document_lang, tf, t, Locale};
use crate::text::{dom_value, event_target_checked, event_target_input, format_bytes};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{BTreeSet, HashMap, HashSet};
use wasm_bindgen::JsValue;

fn format_codex_runtime_updated(raw: &str, locale: Locale) -> String {
    let raw = raw.trim();
    if raw.is_empty() { return String::new(); }
    let value = raw.parse::<f64>().ok().map(|number| {
        // Accept both Unix seconds and the millisecond timestamps returned by
        // current App Server builds.
        JsValue::from_f64(if number.abs() < 100_000_000_000.0 { number * 1_000.0 } else { number })
    }).unwrap_or_else(|| JsValue::from_str(raw));
    let date = js_sys::Date::new(&value);
    if date.get_time().is_nan() {
        raw.to_string()
    } else {
        date.to_locale_string(locale.code(), &JsValue::UNDEFINED).into()
    }
}

fn codex_runtime_source_label(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "codex_desktop" | "codex-desktop" | "desktop" => "Codex Desktop".into(),
        "path" => "PATH".into(),
        "windows" => "Windows".into(),
        "wsl" => "WSL".into(),
        _ => raw.trim().to_string(),
    }
}

fn settings_provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        "codex" | "codex_cli" | "codex_local" | "codex-local" => "codex_cli",
        "claude_code" | "claude-code" | "claude" => "claude_code",
        _ => "openai",
    }
}

fn settings_provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match settings_provider_value(provider) {
        "anthropic" => ("https://api.anthropic.com", "claude-sonnet-5"),
        "openai_responses" => ("https://api.openai.com/v1", "gpt-5.5"),
        // Local runners discover their model from the selected executable; an
        // API URL/model placeholder here would falsely imply it is being used.
        "codex_cli" | "claude_code" => ("", ""),
        _ => ("https://api.deepseek.com", "deepseek-v4-pro"),
    }
}

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
    pub(super) codex_runtime: RwSignal<Option<RuntimeSnapshot>>,
    pub(super) codex_runtime_error: RwSignal<Option<String>>,
    pub(super) codex_runtime_loading: RwSignal<bool>,
    pub(super) codex_settings_action_loading: RwSignal<bool>,
    pub(super) codex_profile_overrides: RwSignal<CodexModeOverrides>,
    pub(super) codex_preview_normal: RwSignal<Option<ResolvedTurnConfig>>,
    pub(super) codex_preview_plan: RwSignal<Option<ResolvedTurnConfig>>,
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
    refresh_codex_runtime: Callback<()>,
    preview_codex_configs: Callback<()>,
    save_codex_profile: Callback<()>,
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
        codex_runtime,
        codex_runtime_error,
        codex_runtime_loading,
        codex_settings_action_loading,
        codex_profile_overrides,
        codex_preview_normal,
        codex_preview_plan,
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
                                                        let (api_url, model) = settings_provider_defaults(&p);
                                                        o.provider = settings_provider_value(&p).into();
                                                        o.api_url = api_url.into();
                                                        o.model = model.into();
                                                    });
                                                }
                                                prop:value=move || model_form.get().map(|f| settings_provider_value(&f.provider).to_string()).unwrap_or_else(|| "openai".into())>
                                                <option value="openai">{move || t(locale.get(), "settings.provider.openai")}</option>
                                                <option value="openai_responses">{move || t(locale.get(), "settings.provider.openai_responses")}</option>
                                                <option value="anthropic">{move || t(locale.get(), "settings.provider.anthropic")}</option>
                                                <option value="codex_cli">{move || t(locale.get(), "settings.provider.codex")}</option>
                                                <option value="claude_code">{move || t(locale.get(), "settings.provider.claude_code")}</option>
                                            </select>
                                        </label>
                                        {move || model_form.get().filter(|form| matches!(settings_provider_value(&form.provider), "codex_cli" | "claude_code")).map(|form| {
                                            let is_codex = settings_provider_value(&form.provider) == "codex_cli";
                                            view! {
                                                <div class="span-2 local-runner-fields">
                                                    <label>
                                                        {move || t(locale.get(), if is_codex { "codex.runner.command" } else { "claude.runner.command" })}
                                                        <input prop:value=if is_codex { form.runner_command.clone() } else { form.runner_claude_command.clone() }
                                                            placeholder=if is_codex { "codex" } else { "claude" }
                                                            on:input=move |event| {
                                                                let value = event_target_input(&event).value();
                                                                model_form.update(|form| if let Some(form) = form {
                                                                    if is_codex { form.runner_command = value.clone(); } else { form.runner_claude_command = value.clone(); }
                                                                });
                                                            } />
                                                    </label>
                                                    <label>
                                                        {move || t(locale.get(), "codex.runner.profile")}
                                                        <input prop:value=form.runner_profile.clone()
                                                            placeholder=move || t(locale.get(), "codex.runner.profile_ph")
                                                            on:input=move |event| model_form.update(|form| if let Some(form) = form { form.runner_profile = event_target_input(&event).value(); }) />
                                                    </label>
                                                    <label class="settings-check span-2">
                                                        <input type="checkbox" prop:checked=form.runner_persistent
                                                            on:change=move |event| model_form.update(|form| if let Some(form) = form { form.runner_persistent = event_target_checked(&event); }) />
                                                        <span>{move || t(locale.get(), "codex.runner.persistent")}</span>
                                                    </label>
                                                </div>
                                            }
                                        })}
                                        <label class="span-2">{move || t(locale.get(), "settings.api_url")}
                                            <input prop:value=move || model_form.get().map(|f| f.api_url.clone()).unwrap_or_default()
                                                prop:disabled=move || model_form.get().is_some_and(|form| matches!(settings_provider_value(&form.provider), "codex_cli" | "claude_code"))
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.api_url = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(), "settings.label")}
                                            <input prop:value=move || model_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.label_ph")
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.label = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(), "settings.model")}
                                            <input prop:value=move || model_form.get().map(|f| f.model.clone()).unwrap_or_default()
                                                prop:disabled=move || model_form.get().is_some_and(|form| matches!(settings_provider_value(&form.provider), "codex_cli" | "claude_code"))
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
                                                <option value="max">"max"</option>
                                                <option value="ultra">"ultra"</option>
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
                        <div class="settings-pane settings-pane-list codex-settings-pane">
                            <CodexRuntimeSettings
                                locale=locale
                                runtime=codex_runtime
                                runtime_error=codex_runtime_error
                                runtime_loading=codex_runtime_loading
                                action_loading=codex_settings_action_loading
                                overrides=codex_profile_overrides
                                preview_normal=codex_preview_normal
                                preview_plan=codex_preview_plan
                                on_refresh=refresh_codex_runtime
                                on_preview=preview_codex_configs
                                on_save=save_codex_profile
                            />
                            <div class="settings-section-divider"></div>
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
                                                    let mut form = profile_to_form(&edit);
                                                    form.runner_command = edit.runner_command.clone();
                                                    form.runner_profile = edit.runner_profile.clone();
                                                    form.runner_sandbox = edit.runner_sandbox.clone();
                                                    form.runner_web_search_mode = edit.runner_web_search_mode.clone();
                                                    form.runner_claude_command = edit.runner_claude_command.clone();
                                                    form.runner_persistent = edit.runner_persistent;
                                                    form.normal_model = edit.normal_model.clone();
                                                    form.normal_reasoning_effort = edit.normal_reasoning_effort.clone();
                                                    form.plan_model = edit.plan_model.clone();
                                                    form.plan_reasoning_effort = edit.plan_reasoning_effort.clone();
                                                    form.service_tier = edit.service_tier.clone();
                                                    form.personality = edit.personality.clone();
                                                    form.reasoning_summary = edit.reasoning_summary.clone();
                                                    form.verbosity = edit.verbosity.clone();
                                                    model_form.set(Some(form));
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
                            <input class="settings-search" type="text" inputmode="search"
                                autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
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

fn codex_select_value(value: &Option<String>, allowed: &[String]) -> String {
    match value.as_deref() {
        None | Some("") => "__inherit__".into(),
        Some(value) if allowed.iter().any(|item| item == value) => value.into(),
        Some(_) => "__custom__".into(),
    }
}

fn codex_efforts(
    snapshot: &RuntimeSnapshot,
    selected: Option<&str>,
    inherited_effective_model: Option<&str>,
) -> Vec<String> {
    if let Some(model) = selected.filter(|value| !value.is_empty()) {
        if let Some(info) = snapshot.models.iter().find(|item| item.id == model) {
            return info.supported_reasoning_efforts.clone();
        }
        return vec!["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
            .into_iter().map(str::to_string).collect();
    }
    // For an inherited model, prefer the effort list declared by the effective
    // catalog model.  A custom model may still use these common values or the
    // free-form input; the app server remains the final validator.
    let effective = inherited_effective_model.unwrap_or("");
    snapshot.models.iter().find(|item| item.id == effective)
        .map(|item| item.supported_reasoning_efforts.clone())
        .unwrap_or_else(|| vec!["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
        .into_iter().map(str::to_string).collect())
}

#[component]
fn CodexModeEditor(
    locale: RwSignal<Locale>,
    mode: &'static str,
    runtime: RwSignal<Option<RuntimeSnapshot>>,
    overrides: RwSignal<CodexModeOverrides>,
    preview: RwSignal<Option<ResolvedTurnConfig>>,
) -> impl IntoView {
    let is_plan = mode == "plan";
    let compatibility_notice = create_rw_signal(None::<String>);
    let custom_model_editing = create_rw_signal(false);
    let custom_effort_editing = create_rw_signal(false);
    let title_key = if is_plan { "codex.mode.plan" } else { "codex.mode.normal" };
    let model_options = create_memo(move |_| runtime.get().map(|snapshot| {
        snapshot.models.into_iter().map(|model| model.id).collect::<Vec<_>>()
    }).unwrap_or_default());
    let effort_options = create_memo(move |_| {
        let Some(snapshot) = runtime.get() else { return Vec::new(); };
        let layer = overrides.get();
        let selected = if is_plan { layer.plan.model.as_deref() } else { layer.normal.model.as_deref() };
        let inherited_model = preview.get().map(|config| config.effective_model().to_string());
        codex_efforts(&snapshot, selected, inherited_model.as_deref())
    });
    let custom_model_visible = create_memo(move |_| {
        let layer = overrides.get();
        let selected = if is_plan { layer.plan.model } else { layer.normal.model };
        let allowed = model_options.get();
        custom_model_editing.get()
            || matches!(selected.as_deref(), Some(value) if value.is_empty() || !allowed.iter().any(|item| item == value))
    });
    let custom_effort_visible = create_memo(move |_| {
        let layer = overrides.get();
        let selected = if is_plan { layer.plan.effort } else { layer.normal.effort };
        let allowed = effort_options.get();
        custom_effort_editing.get()
            || matches!(selected.as_deref(), Some(value) if value.is_empty() || !allowed.iter().any(|item| item == value))
    });

    view! {
        <section class="codex-mode-card" data-mode=mode>
            <div class="codex-mode-head">
                <div>
                    <h4>{move || t(locale.get(), title_key)}</h4>
                    <p>{move || t(locale.get(), if is_plan { "codex.mode.plan_hint" } else { "codex.mode.normal_hint" })}</p>
                </div>
                {is_plan.then(|| view! { <span class="codex-lock-badge">{move || t(locale.get(), "codex.plan.read_only")}</span> })}
            </div>
            <div class="codex-mode-fields">
                <label>
                    <span>{move || t(locale.get(), "codex.model")}</span>
                    <select data-testid=format!("codex-{mode}-model")
                        prop:value=move || {
                            if custom_model_editing.get() { return "__custom__".into(); }
                            let layer = overrides.get();
                            let value = if is_plan { &layer.plan.model } else { &layer.normal.model };
                            codex_select_value(value, &model_options.get())
                        }
                        on:change=move |event| {
                            let value = dom_value(&event);
                            custom_model_editing.set(value == "__custom__");
                            let selected = match value.as_str() {
                                "__inherit__" => None,
                                "__custom__" => Some(String::new()),
                                _ => Some(value.clone()),
                            };
                            let inherited_model = preview.get_untracked().map(|config| config.effective_model().to_string());
                            let supported = runtime.get_untracked().map(|snapshot| {
                                codex_efforts(&snapshot, selected.as_deref(), inherited_model.as_deref())
                            }).unwrap_or_default();
                            let mut reset_effort = false;
                            overrides.update(|layer| {
                                let target = if is_plan { &mut layer.plan } else { &mut layer.normal };
                                target.model = selected;
                                if target.effort.as_ref().is_some_and(|effort| !supported.iter().any(|item| item == effort)) {
                                    target.effort = None;
                                    reset_effort = true;
                                }
                            });
                            compatibility_notice.set(reset_effort.then(|| t(locale.get_untracked(), "codex.effort_reset").into()));
                        }>
                        <option value="__inherit__">{move || t(locale.get(), "codex.inherit")}</option>
                        {move || runtime.get().map(|snapshot| snapshot.models.into_iter().map(|model| {
                            let id = model.id.clone();
                            view! { <option value=id.clone()>{format!("{} · {}", model.label(), id)}</option> }
                        }).collect_view())}
                        <option value="__custom__">{move || t(locale.get(), "codex.custom")}</option>
                    </select>
                </label>
                <label>
                    <span>{move || t(locale.get(), "codex.effort")}</span>
                    <select data-testid=format!("codex-{mode}-effort")
                        prop:value=move || {
                            if custom_effort_editing.get() { return "__custom__".into(); }
                            let layer = overrides.get();
                            let value = if is_plan { &layer.plan.effort } else { &layer.normal.effort };
                            codex_select_value(value, &effort_options.get())
                        }
                        on:change=move |event| {
                            let value = dom_value(&event);
                            custom_effort_editing.set(value == "__custom__");
                            let selected = match value.as_str() {
                                "__inherit__" => None,
                                "__custom__" => Some(String::new()),
                                _ => Some(value),
                            };
                            overrides.update(|layer| {
                                let target = if is_plan { &mut layer.plan } else { &mut layer.normal };
                                target.effort = selected;
                            });
                        }>
                        <option value="__inherit__">{move || t(locale.get(), "codex.inherit")}</option>
                        {move || effort_options.get().into_iter().map(|effort| view! {
                            <option value=effort.clone()>{effort}</option>
                        }).collect_view()}
                        <option value="__custom__">{move || t(locale.get(), "codex.custom")}</option>
                    </select>
                </label>
                {move || custom_model_visible.get().then(|| view! {
                        <label class="codex-custom-input">
                            <span>{move || t(locale.get(), "codex.custom_model")}</span>
                            <input data-testid=format!("codex-{mode}-custom-model")
                                prop:value=move || {
                                    let layer = overrides.get();
                                    if is_plan { layer.plan.model } else { layer.normal.model }.unwrap_or_default()
                                }
                                placeholder=move || t(locale.get(), "codex.custom_model_ph")
                                on:input=move |event| {
                                    let value = event_target_input(&event).value();
                                    overrides.update(|layer| {
                                        let target = if is_plan { &mut layer.plan } else { &mut layer.normal };
                                        target.model = Some(value.clone());
                                    });
                                } />
                        </label>
                    })}
                {move || custom_effort_visible.get().then(|| view! {
                        <label class="codex-custom-input">
                            <span>{move || t(locale.get(), "codex.custom_effort")}</span>
                            <input data-testid=format!("codex-{mode}-custom-effort")
                                prop:value=move || {
                                    let layer = overrides.get();
                                    if is_plan { layer.plan.effort } else { layer.normal.effort }.unwrap_or_default()
                                }
                                placeholder=move || t(locale.get(), "codex.custom_effort_ph")
                                on:input=move |event| {
                                    let value = event_target_input(&event).value();
                                    overrides.update(|layer| {
                                        let target = if is_plan { &mut layer.plan } else { &mut layer.normal };
                                        target.effort = Some(value.clone());
                                    });
                                } />
                        </label>
                    })}
                {move || compatibility_notice.get().map(|notice| view! {
                    <div class="codex-inline-notice">{notice}</div>
                })}
            </div>
        </section>
    }
}

#[component]
fn CodexPreviewCard(
    locale: RwSignal<Locale>,
    title: &'static str,
    config: RwSignal<Option<ResolvedTurnConfig>>,
) -> impl IntoView {
    view! {
        <div class="codex-preview-card">
            <h5>{move || t(locale.get(), title)}</h5>
            {move || if let Some(config) = config.get() {
                let requested_model = config.requested_model().to_string();
                let effective_model = config.effective_model().to_string();
                let requested_effort = config.requested_effort().to_string();
                let effective_effort = config.effective_effort().to_string();
                let rerouted = !requested_model.is_empty() && !effective_model.is_empty() && requested_model != effective_model;
                let model_source = config.sources.get("model").cloned().unwrap_or_default();
                let sandbox_detail = if config.sandbox_policy.is_null() {
                    config.sandbox.clone()
                } else {
                    config.sandbox_policy.to_string()
                };
                let details = [
                    ("service tier", config.service_tier.clone(), "service_tier"),
                    ("personality", config.personality.clone(), "personality"),
                    ("reasoning summary", config.summary.clone(), "summary"),
                    ("verbosity", config.verbosity.clone(), "verbosity"),
                    ("web search", config.web_search.clone(), "web_search"),
                ];
                view! {
                    <dl>
                        <dt>{t(locale.get(), "codex.model")}</dt>
                        <dd class:rerouted=rerouted>{if rerouted { format!("{requested_model} → {effective_model}") } else { effective_model }}</dd>
                        <dt>{t(locale.get(), "codex.effort")}</dt>
                        <dd>{if requested_effort != effective_effort && !requested_effort.is_empty() { format!("{requested_effort} → {effective_effort}") } else { effective_effort }}</dd>
                        <dt>{t(locale.get(), "codex.sandbox")}</dt><dd>{sandbox_detail}</dd>
                        {(!model_source.is_empty()).then(|| view! { <dt>{t(locale.get(), "codex.source")}</dt><dd>{model_source}</dd> })}
                        {details.into_iter().filter(|(_, value, _)| !value.is_empty()).map(|(label, value, key)| {
                            let source = config.effective_sources.get(key)
                                .or_else(|| config.sources.get(key)).cloned().unwrap_or_default();
                            view! {
                                <dt>{label}</dt><dd>{if source.is_empty() { value } else { format!("{value} · {source}") }}</dd>
                            }
                        }).collect_view()}
                        {(!config.runtime_path.is_empty()).then(|| view! { <dt>"runtime"</dt><dd><code>{config.runtime_path.clone()}</code></dd> })}
                        {(!config.runtime_version.is_empty()).then(|| view! { <dt>"version"</dt><dd>{config.runtime_version.clone()}</dd> })}
                        {(!config.codex_home.is_empty()).then(|| view! { <dt>"CODEX_HOME"</dt><dd><code>{config.codex_home.clone()}</code></dd> })}
                        <dt>{t(locale.get(), "codex.generation")}</dt><dd>{config.config_version}</dd>
                    </dl>
                    {(!config.warnings.is_empty()).then(|| view! {
                        <ul class="codex-warning-list">{config.warnings.into_iter().map(|warning| view! { <li>{warning}</li> }).collect_view()}</ul>
                    })}
                    {(!config.validation_errors.is_empty()).then(|| view! {
                        <ul class="codex-warning-list">{config.validation_errors.into_iter().map(|error| view! { <li>{error}</li> }).collect_view()}</ul>
                    })}
                }.into_view()
            } else {
                view! { <p class="codex-preview-empty">{t(locale.get(), "codex.preview_empty")}</p> }.into_view()
            }}
        </div>
    }
}

#[component]
fn CodexRuntimeSettings(
    locale: RwSignal<Locale>,
    runtime: RwSignal<Option<RuntimeSnapshot>>,
    runtime_error: RwSignal<Option<String>>,
    runtime_loading: RwSignal<bool>,
    action_loading: RwSignal<bool>,
    overrides: RwSignal<CodexModeOverrides>,
    preview_normal: RwSignal<Option<ResolvedTurnConfig>>,
    preview_plan: RwSignal<Option<ResolvedTurnConfig>>,
    on_refresh: Callback<()>,
    on_preview: Callback<()>,
    on_save: Callback<()>,
) -> impl IntoView {
    let select_optional = |value: String| (!value.is_empty() && value != "__inherit__").then_some(value);
    let selected_models = create_memo(move |_| {
        let Some(snapshot) = runtime.get() else { return Vec::new(); };
        let profile_layer = overrides.get();
        let normal_model = profile_layer.normal.model
            .filter(|value| !value.trim().is_empty())
            .or_else(|| preview_normal.get().map(|config| config.effective_model().to_string()))
            .unwrap_or_default();
        let plan_model = profile_layer.plan.model
            .filter(|value| !value.trim().is_empty())
            .or_else(|| preview_plan.get().map(|config| config.effective_model().to_string()))
            .unwrap_or_default();
        [normal_model, plan_model].into_iter()
            .filter_map(|id| snapshot.models.iter().find(|model| model.id == id).cloned())
            .collect::<Vec<_>>()
    });
    let personality_supported = create_memo(move |_| {
        runtime.get().is_some_and(|snapshot| snapshot.provider_capabilities.personality)
            && selected_models.get().iter().all(|model| model.supports_personality)
    });
    let service_tiers = create_memo(move |_| {
        let models = selected_models.get();
        let mut tiers = models.first().map(|model| model.service_tiers.clone()).unwrap_or_default();
        for model in models.iter().skip(1) {
            tiers.retain(|tier| model.service_tiers.contains(tier));
        }
        tiers.sort();
        tiers.dedup();
        tiers
    });
    let service_tier_supported = create_memo(move |_| {
        runtime.get().is_some_and(|snapshot| snapshot.provider_capabilities.service_tier)
            && !service_tiers.get().is_empty()
    });
    view! {
        <section class="codex-runtime-settings" data-testid="codex-runtime-settings">
            <div class="codex-runtime-titlebar">
                <div>
                    <h3>{move || t(locale.get(), "codex.runtime.title")}</h3>
                    <p>{move || t(locale.get(), "codex.runtime.subtitle")}</p>
                </div>
                <button type="button" class="settings-add-btn codex-runtime-refresh" disabled=move || runtime_loading.get() on:click=move |_| on_refresh.call(())>
                    {move || t(locale.get(), if runtime_loading.get() { "codex.runtime.refreshing" } else { "codex.runtime.refresh" })}
                </button>
            </div>
            {move || runtime_error.get().map(|message| view! {
                <div class="settings-status fail codex-runtime-error">{message}</div>
            })}
            {move || if let Some(snapshot) = runtime.get() {
                let capabilities = snapshot.provider_capabilities.clone();
                let runtime_source = [snapshot.runtime.source.clone(), snapshot.runtime.context.clone()]
                    .into_iter().filter(|value| !value.trim().is_empty())
                    .map(|value| codex_runtime_source_label(&value)).collect::<Vec<_>>().join(" · ");
                let path = snapshot.executable_path().to_string();
                let version = snapshot.version().to_string();
                let home = snapshot.codex_home().to_string();
                let updated = format_codex_runtime_updated(&snapshot.refreshed_at, locale.get());
                let warnings = snapshot.warnings.clone();
                view! {
                    <details class="codex-runtime-card">
                        <summary class="codex-runtime-summary">
                            <span class="codex-runtime-summary-main">
                                <strong>{if version.is_empty() { t(locale.get(), "codex.runtime.title").into() } else { version.clone() }}</strong>
                                <span>{runtime_source.clone()}</span>
                            </span>
                            <span class="codex-capability-list">
                                {[
                                    (t(locale.get(), "codex.capability.app_server"), capabilities.app_server),
                                    (t(locale.get(), "codex.capability.native_plan"), capabilities.native_plan),
                                    (t(locale.get(), "codex.capability.images"), capabilities.image_input),
                                    (t(locale.get(), "codex.capability.personality"), capabilities.personality),
                                ].into_iter().map(|(label, supported)| view! {
                                    <span class="codex-capability" class:supported=supported class:unsupported=!supported>
                                        <span class="codex-capability-dot"></span>{label}
                                    </span>
                                }).collect_view()}
                            </span>
                            <span class="settings-list-chevron codex-runtime-chevron">"›"</span>
                        </summary>
                        <div class="codex-runtime-details-body">
                            <div class="codex-runtime-meta">
                                <div><span>{t(locale.get(), "codex.runtime.path")}</span><code title=path.clone()>{path}</code></div>
                                <div><span>{t(locale.get(), "codex.runtime.version")}</span><strong>{version}</strong></div>
                                <div><span>"CODEX_HOME"</span><code title=home.clone()>{home}</code></div>
                                <div><span>{t(locale.get(), "codex.runtime.source")}</span><strong>{runtime_source}</strong></div>
                                {(!updated.is_empty()).then(|| view! { <div><span>{t(locale.get(), "codex.runtime.updated")}</span><strong>{updated}</strong></div> })}
                            </div>
                            {(!warnings.is_empty()).then(|| view! {
                                <ul class="codex-warning-list">{warnings.into_iter().map(|warning| view! { <li>{warning}</li> }).collect_view()}</ul>
                            })}
                        </div>
                    </details>
                    <div class="codex-mode-grid">
                        <CodexModeEditor locale=locale mode="normal" runtime=runtime overrides=overrides preview=preview_normal />
                        <CodexModeEditor locale=locale mode="plan" runtime=runtime overrides=overrides preview=preview_plan />
                    </div>
                    <section class="codex-common-card">
                        <div class="codex-mode-head">
                            <div><h4>{t(locale.get(), "codex.common.title")}</h4><p>{t(locale.get(), "codex.common.hint")}</p></div>
                        </div>
                        <div class="codex-common-grid">
                            <label><span>{t(locale.get(), "codex.service_tier")}</span>
                                <select disabled=move || !service_tier_supported.get()
                                    title=move || (!service_tier_supported.get()).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().service_tier.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.service_tier=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option>
                                    {move || service_tiers.get().into_iter().map(|tier| view! { <option value=tier.clone()>{tier}</option> }).collect_view()}
                                </select>
                            </label>
                            <label><span>{t(locale.get(), "codex.personality")}</span>
                                <select disabled=move || !personality_supported.get()
                                    title=move || (!personality_supported.get()).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().personality.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.personality=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option><option value="none">"none"</option><option value="friendly">"friendly"</option><option value="pragmatic">"pragmatic"</option>
                                </select>
                            </label>
                            <label><span>{t(locale.get(), "codex.summary")}</span>
                                <select disabled=!capabilities.reasoning_summary
                                    title=(!capabilities.reasoning_summary).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().summary.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.summary=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option><option value="auto">"auto"</option><option value="concise">"concise"</option><option value="detailed">"detailed"</option><option value="none">"none"</option>
                                </select>
                            </label>
                            <label><span>{t(locale.get(), "codex.verbosity")}</span>
                                <select disabled=!capabilities.verbosity
                                    title=(!capabilities.verbosity).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().verbosity.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.verbosity=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option><option value="low">"low"</option><option value="medium">"medium"</option><option value="high">"high"</option>
                                </select>
                            </label>
                            <label><span>{t(locale.get(), "codex.web_search")}</span>
                                <select disabled=!capabilities.web_search
                                    title=(!capabilities.web_search).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().web_search.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.web_search=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option><option value="disabled">"disabled"</option><option value="cached">"cached"</option><option value="indexed">"indexed"</option><option value="live">"live"</option>
                                </select>
                            </label>
                            <label><span>{t(locale.get(), "codex.sandbox")}</span>
                                <select disabled=!capabilities.sandbox
                                    title=(!capabilities.sandbox).then(|| t(locale.get(), "codex.unsupported")).unwrap_or_default()
                                    prop:value=move || overrides.get().sandbox.unwrap_or_else(|| "__inherit__".into())
                                    on:change=move |event| { let value=dom_value(&event); overrides.update(|item| item.sandbox=select_optional(value)); }>
                                    <option value="__inherit__">{t(locale.get(), "codex.inherit")}</option><option value="read-only">"read-only"</option><option value="workspace-write">"workspace-write"</option><option value="danger-full-access">"danger-full-access"</option>
                                </select>
                            </label>
                        </div>
                    </section>
                    <section class="codex-preview-section">
                        <div class="codex-preview-head"><h4>{t(locale.get(), "codex.preview.title")}</h4><p>{t(locale.get(), "codex.preview.hint")}</p></div>
                        <div class="codex-preview-grid">
                            <CodexPreviewCard locale=locale title="codex.mode.normal" config=preview_normal />
                            <CodexPreviewCard locale=locale title="codex.mode.plan" config=preview_plan />
                        </div>
                    </section>
                    <div class="row codex-runtime-actions">
                        <button type="button" disabled=move || runtime_loading.get() || action_loading.get() on:click=move |_| on_preview.call(())>{t(locale.get(), "codex.preview.button")}</button>
                        <button type="button" class="primary" disabled=move || runtime_loading.get() || action_loading.get() on:click=move |_| on_save.call(())>{t(locale.get(), "codex.save_profile")}</button>
                    </div>
                }.into_view()
            } else if runtime_loading.get() {
                view! {
                    <div class="codex-runtime-empty codex-runtime-loading-card" aria-live="polite">
                        <strong>{t(locale.get(), "codex.runtime.loading_short")}</strong>
                        <p>{t(locale.get(), "codex.runtime.subtitle")}</p>
                    </div>
                }.into_view()
            } else {
                view! {
                    <div class="codex-runtime-empty">
                        <strong>{t(locale.get(), "codex.runtime.unavailable")}</strong>
                        <p>{t(locale.get(), "codex.runtime.unavailable_hint")}</p>
                    </div>
                    <div class="codex-inline-notice">{t(locale.get(), "codex.compat_profile_hint")}</div>
                    <div class="codex-mode-grid codex-fallback-grid">
                        <section class="codex-mode-card" data-mode="normal">
                            <div class="codex-mode-head"><div><h4>{t(locale.get(), "codex.mode.normal")}</h4><p>{t(locale.get(), "codex.mode.normal_hint")}</p></div></div>
                            <div class="codex-mode-fields">
                                <label><span>{t(locale.get(), "codex.model")}</span>
                                    <input data-testid="codex-fallback-normal-model" prop:value=move || overrides.get().normal.model.unwrap_or_default()
                                        placeholder=move || t(locale.get(), "codex.inherit")
                                        on:input=move |event| {
                                            let value=event_target_input(&event).value();
                                            overrides.update(|item| item.normal.model=(!value.trim().is_empty()).then_some(value));
                                        } />
                                </label>
                                <label><span>{t(locale.get(), "codex.effort")}</span>
                                    <input data-testid="codex-fallback-normal-effort" prop:value=move || overrides.get().normal.effort.unwrap_or_default()
                                        placeholder=move || t(locale.get(), "codex.inherit")
                                        on:input=move |event| {
                                            let value=event_target_input(&event).value();
                                            overrides.update(|item| item.normal.effort=(!value.trim().is_empty()).then_some(value));
                                        } />
                                </label>
                            </div>
                        </section>
                        <section class="codex-mode-card" data-mode="plan">
                            <div class="codex-mode-head"><div><h4>{t(locale.get(), "codex.mode.plan")}</h4><p>{t(locale.get(), "codex.mode.plan_hint")}</p></div><span class="codex-lock-badge">{t(locale.get(), "codex.plan.read_only")}</span></div>
                            <div class="codex-mode-fields">
                                <label><span>{t(locale.get(), "codex.model")}</span>
                                    <input data-testid="codex-fallback-plan-model" prop:value=move || overrides.get().plan.model.unwrap_or_default()
                                        placeholder=move || t(locale.get(), "codex.inherit")
                                        on:input=move |event| {
                                            let value=event_target_input(&event).value();
                                            overrides.update(|item| item.plan.model=(!value.trim().is_empty()).then_some(value));
                                        } />
                                </label>
                                <label><span>{t(locale.get(), "codex.effort")}</span>
                                    <input data-testid="codex-fallback-plan-effort" prop:value=move || overrides.get().plan.effort.unwrap_or_default()
                                        placeholder=move || t(locale.get(), "codex.inherit")
                                        on:input=move |event| {
                                            let value=event_target_input(&event).value();
                                            overrides.update(|item| item.plan.effort=(!value.trim().is_empty()).then_some(value));
                                        } />
                                </label>
                            </div>
                        </section>
                    </div>
                    <div class="row codex-runtime-actions">
                        <button type="button" class="primary" disabled=move || action_loading.get() on:click=move |_| on_save.call(())>{t(locale.get(), "codex.save_profile")}</button>
                    </div>
                }.into_view()
            }}
        </section>
    }
}
