use crate::app_support::{
    build_conn_json, close_details_ancestor, compose_icon, conn_form_from_row,
    context_capability_summary, focus_element_soon, join_tags, js_error_text, new_acp_form,
    new_model_form, profile_to_form, reviewer_backend_key, reviewer_backend_label,
    reviewer_missing_acp_profile_id, set_reviewer_backend, settings_section_label,
    settings_subpage_label, skill_matches_filter, CRED_GROUPS,
};
use crate::bindings::{invoke, invoke_checked, is_windows};
use crate::dto::*;
use crate::i18n::{localize_backend, set_document_lang, t, tf, Locale};
use crate::text::{
    dom_value, event_target_checked, event_target_input, event_target_value, format_bytes,
};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{BTreeSet, HashMap, HashSet};
use wasm_bindgen::JsValue;

fn settings_provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        _ => "openai",
    }
}

fn settings_provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match settings_provider_value(provider) {
        "anthropic" => ("https://api.anthropic.com", "claude-sonnet-5"),
        "openai_responses" => ("https://api.openai.com/v1", "gpt-5.5"),
        _ => ("https://api.deepseek.com", "deepseek-v4-pro"),
    }
}

fn appearance_palette_options(dark: bool) -> [(&'static str, &'static str); 5] {
    if dark {
        [
            ("charcoal", "Wisp Charcoal"),
            ("codex", "Codex"),
            ("github", "GitHub Dark"),
            ("catppuccin", "Catppuccin Mocha"),
            ("gruvbox", "Gruvbox"),
        ]
    } else {
        [
            ("paper", "Wisp Paper"),
            ("codex", "Codex"),
            ("github", "GitHub"),
            ("catppuccin", "Catppuccin Latte"),
            ("everforest", "Everforest"),
        ]
    }
}

fn appearance_palette_meta(
    dark: bool,
    palette: &str,
) -> (&'static str, &'static str, &'static str) {
    match (dark, palette) {
        (false, "codex") => ("#2563EB", "#F4F6F8", "#172033"),
        (false, "github") => ("#0969DA", "#F6F8FA", "#1F2328"),
        (false, "catppuccin") => ("#8839EF", "#EFF1F5", "#4C4F69"),
        (false, "everforest") => ("#3A8F6B", "#F4F0D9", "#2F383E"),
        (true, "codex") => ("#7C8CFF", "#202123", "#F3F4F6"),
        (true, "github") => ("#58A6FF", "#0D1117", "#F0F6FC"),
        (true, "catppuccin") => ("#CBA6F7", "#1E1E2E", "#CDD6F4"),
        (true, "gruvbox") => ("#D79921", "#282828", "#EBDBB2"),
        (true, _) => ("#2DA898", "#171614", "#EBE8E2"),
        _ => ("#0D9488", "#FAF9F6", "#141413"),
    }
}

#[derive(Clone, Copy)]
pub(super) struct SettingsViewState {
    pub(super) locale: RwSignal<Locale>,
    pub(super) theme_mode: RwSignal<String>,
    pub(super) light_palette: RwSignal<String>,
    pub(super) dark_palette: RwSignal<String>,
    pub(super) ui_font_size: RwSignal<u16>,
    pub(super) code_font_size: RwSignal<u16>,
    pub(super) show_settings: RwSignal<bool>,
    pub(super) settings_section: RwSignal<String>,
    pub(super) open_conn_key: RwSignal<Option<String>>,
    pub(super) channels_open: RwSignal<Option<String>>,
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
    pub(super) show_acp_agents: RwSignal<bool>,
    pub(super) acp_agents: RwSignal<Vec<AcpAgentProfile>>,
    pub(super) active_acp_agent_id: RwSignal<Option<String>>,
    pub(super) acp_form: RwSignal<Option<AcpAgentProfile>>,
    pub(super) acp_form_msg: RwSignal<Option<(bool, String)>>,
    pub(super) acp_infos: RwSignal<HashMap<String, AcpAgentInfo>>,
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
    pub(super) custom_credentials: RwSignal<Vec<CustomCredentialStatus>>,
    pub(super) cred_msg: RwSignal<Option<(bool, String)>>,
    pub(super) approval_grants: RwSignal<Vec<ApprovalGrantRow>>,
    pub(super) conns_view: RwSignal<Option<ConnView>>,
    pub(super) conn_form_open: Memo<bool>,
    pub(super) conn_form_kind: Memo<String>,
    pub(super) conn_test_msg: RwSignal<Option<(bool, String)>>,
    pub(super) custom_conn_tools: RwSignal<HashMap<String, Vec<ConnectorTool>>>,
    pub(super) custom_conn_tools_loading: RwSignal<HashSet<String>>,
    pub(super) custom_conn_tool_errors: RwSignal<HashMap<String, String>>,
    pub(super) pet_status: RwSignal<PetStatus>,
    pub(super) ssh_hosts: RwSignal<Vec<SshHost>>,
    pub(super) execution_contexts: RwSignal<Vec<ExecutionContext>>,
    pub(super) runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    pub(super) probing_context_id: RwSignal<Option<String>>,
}

#[component]
pub(super) fn SettingsView(
    state: SettingsViewState,
    open_project: Callback<String>,
    go_settings_section: Callback<String>,
    close_settings_subpage: Callback<()>,
    check_updates: Callback<web_sys::MouseEvent>,
    save_settings: Callback<web_sys::MouseEvent>,
    save_model_form: Callback<web_sys::MouseEvent>,
    save_specialist_form: Callback<web_sys::MouseEvent>,
    test_reviewer_form: Callback<web_sys::MouseEvent>,
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
    open_add_host: Callback<()>,
    edit_ssh_host: Callback<String>,
    import_ssh_hosts: Callback<()>,
    import_wsl_contexts: Callback<()>,
    remove_ssh_host: Callback<String>,
    probe_compute_resource: Callback<String>,
) -> impl IntoView {
    let SettingsViewState {
        locale,
        theme_mode,
        light_palette,
        dark_palette,
        ui_font_size,
        code_font_size,
        show_settings,
        settings_section,
        open_conn_key,
        channels_open,
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
        show_acp_agents,
        acp_agents,
        active_acp_agent_id,
        acp_form,
        acp_form_msg,
        acp_infos,
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
        custom_credentials,
        cred_msg,
        approval_grants,
        conns_view,
        conn_form_open,
        conn_form_kind,
        conn_test_msg,
        custom_conn_tools,
        custom_conn_tools_loading,
        custom_conn_tool_errors,
        pet_status,
        ssh_hosts,
        execution_contexts,
        runtime_interpreter_form,
        probing_context_id,
    } = state;
    let acp_form_open = create_memo(move |_| acp_form.get().is_some());
    let model_delete_confirm = create_rw_signal(None::<(String, String)>);
    let joining = create_rw_signal(false);
    let join_code = create_rw_signal(String::new());
    let join_busy = create_rw_signal(false);
    let join_error = create_rw_signal(None::<String>);
    let oauth_authorizing = create_rw_signal(false);
    let custom_cred_name = create_rw_signal(String::new());
    let custom_cred_env = create_rw_signal(String::new());
    let custom_cred_value = create_rw_signal(String::new());
    let custom_cred_busy = create_rw_signal(false);
    create_effect(move |_| {
        if joining.get() {
            focus_element_soon("sync-device-code");
        }
    });
    let choose_sync_folder = move |_| {
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(path) = serde_wasm_bindgen::from_value::<String>(value) {
                settings.update(|current| current.sync_folder = path);
            }
        });
    };
    let choose_pet_directory = move |_| {
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(path) = serde_wasm_bindgen::from_value::<String>(value) {
                settings.update(|current| current.pet_directory = path);
            }
        });
    };
    let open_sync_guide = move |_| {
        let page = if locale.get_untracked() == Locale::Zh {
            "project-sync.zh-CN.md"
        } else {
            "project-sync.md"
        };
        crate::bindings::open_external_url(format!(
            "https://github.com/xuzhougeng/wisp-science/blob/main/docs/{page}"
        ));
    };
    let join_project = move |_| {
        let code = join_code.get();
        if code.trim().is_empty() || join_busy.get_untracked() {
            return;
        }
        join_busy.set(true);
        join_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "code": code })).unwrap();
            match invoke_checked("join_synced_project", args).await {
                Ok(value) => {
                    if let Ok(Some(project)) =
                        serde_wasm_bindgen::from_value::<Option<ProjectSummary>>(value)
                    {
                        joining.set(false);
                        join_code.set(String::new());
                        show_settings.set(false);
                        open_project.call(project.id);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    join_error.set(Some(message));
                }
            }
            join_busy.set(false);
        });
    };

    move || {
        show_settings.get().then(|| view! {
        <div class="settings-page">
            <div class="settings-nav">
                <button type="button" class="settings-app-back settings-head-close"
                    on:click=move |_| show_settings.set(false)>
                    {compose_icon("chevron-left")}
                    <span>{move || t(locale.get(), "settings.back_to_app")}</span>
                </button>
                <div class="settings-nav-title">{move || t(locale.get(), "settings.title")}</div>
                <div class="settings-nav-group">
                    <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.workspace")}</span>
                    <button class:active=move || settings_section.get()=="general"
                        on:click=move |_| go_settings_section.call("general".into())>
                        {move || t(locale.get(), "settings.nav.general")}</button>
                    <button class:active=move || settings_section.get()=="appearance"
                        on:click=move |_| go_settings_section.call("appearance".into())>
                        {move || t(locale.get(), "settings.nav.appearance")}</button>
                    <button class:active=move || settings_section.get()=="pet"
                        on:click=move |_| go_settings_section.call("pet".into())>
                        {move || t(locale.get(), "settings.nav.pet")}</button>
                    <button class:active=move || settings_section.get()=="credentials"
                        on:click=move |_| go_settings_section.call("credentials".into())>
                        {move || t(locale.get(), "settings.nav.credentials")}</button>
                    <button class:active=move || settings_section.get()=="permissions"
                        on:click=move |_| go_settings_section.call("permissions".into())>
                        {move || t(locale.get(), "settings.nav.permissions")}</button>
                    <button class:active=move || settings_section.get()=="environments"
                        on:click=move |_| go_settings_section.call("environments".into())>
                        {move || t(locale.get(), "settings.nav.environments")}</button>
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
                    <button class:active=move || settings_section.get()=="channels"
                        on:click=move |_| go_settings_section.call("channels".into())>
                        {move || t(locale.get(), "settings.nav.channels")}</button>
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
                        acp_form.get().as_ref(),
                        channels_open.get().as_deref(),
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
                        <label class="span-2">{move || t(locale.get(), "settings.max_iter")}
                            <input data-testid="max-iter" type="number" min="0" step="1"
                                on:input=move |ev| settings.update(|s| {
                                    if let Ok(value) = event_target_input(&ev).value().parse() {
                                        s.max_iter = value;
                                    }
                                })
                                prop:value=move || settings.get().max_iter.to_string() />
                            <span class="settings-field-hint">{move || t(locale.get(), "settings.max_iter_hint")}</span>
                        </label>
                        <div class="span-2 settings-sync-block">
                            <h3>{move || t(locale.get(), "settings.sync.title")}</h3>
                            <p class="settings-field-hint">{move || t(locale.get(), "settings.sync.hint")}</p>
                            <label>{move || t(locale.get(), "settings.sync.backend")}
                                <select data-testid="sync-backend"
                                    prop:value=move || settings.get().sync_backend
                                    on:change=move |ev| settings.update(|current| current.sync_backend = dom_value(&ev))>
                                    <option value="relay">{move || t(locale.get(), "settings.sync.relay")}</option>
                                    <option value="folder">{move || t(locale.get(), "settings.sync.folder")}</option>
                                </select>
                            </label>
                            {move || if settings.get().sync_backend == "folder" {
                                view! {
                                    <label>{move || t(locale.get(), "settings.sync.folder_path")}
                                        <div class="settings-path-row">
                                            <input class="settings-path-input" data-testid="sync-folder"
                                                prop:value=move || settings.get().sync_folder
                                                on:input=move |ev| settings.update(|current| current.sync_folder = event_target_input(&ev).value()) />
                                            <button type="button" on:click=choose_sync_folder>{move || t(locale.get(), "projects.choose_dir")}</button>
                                        </div>
                                        <span class="settings-field-hint">{move || t(locale.get(), "settings.sync.folder_hint")}</span>
                                    </label>
                                }.into_view()
                            } else {
                                view! {
                                    <label>{move || t(locale.get(), "settings.sync.relay_url")}
                                        <input data-testid="sync-relay-url" type="url"
                                            prop:value=move || settings.get().sync_relay_url
                                            placeholder="https://sync.example.com"
                                            on:input=move |ev| settings.update(|current| current.sync_relay_url = event_target_input(&ev).value()) />
                                    </label>
                                    <label>{move || t(locale.get(), "settings.sync.relay_token")}
                                        <input data-testid="sync-relay-token" type="password"
                                            prop:value=move || settings.get().sync_relay_token
                                            placeholder=move || if settings.get().has_sync_relay_token {
                                                t(locale.get(), "settings.key_stored")
                                            } else {
                                                t(locale.get(), "settings.sync.token_placeholder")
                                            }
                                            on:input=move |ev| settings.update(|current| current.sync_relay_token = event_target_input(&ev).value()) />
                                        <span class="settings-field-hint">{move || t(locale.get(), "settings.sync.relay_hint")}</span>
                                    </label>
                                }.into_view()
                            }}
                            <p class="settings-field-hint">
                                {move || t(locale.get(), "settings.sync.join_hint")}
                            </p>
                            <div class="row settings-sync-actions">
                                <button type="button" on:click=open_sync_guide>
                                    {compose_icon("doc")}
                                    <span>{move || t(locale.get(), "projects.sync.guide")}</span>
                                </button>
                                <button type="button" class="primary"
                                    on:click=move |_| {
                                        join_error.set(None);
                                        joining.set(true);
                                    }>
                                    {compose_icon("link")}
                                    <span>{move || t(locale.get(), "projects.sync.join")}</span>
                                </button>
                            </div>
                        </div>
                        </div>
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status"
                                class:ok=move || ok
                                class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                                <span class="settings-version">{concat!("wisp-science v", env!("CARGO_PKG_VERSION"))}</span>
                                <button type="button" disabled=move || settings_busy.get() on:click=move |ev| check_updates.call(ev)>{move || t(locale.get(), "settings.check_updates")}</button>
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "environments").then(|| view! {
                    <div class="settings-pane settings-pane-list environment-settings-pane">
                        <p class="settings-note">{move || t(locale.get(), "environments.hint")}</p>
                        <div class="settings-toolbar environment-settings-actions">
                            <button type="button" class="primary" on:click=move |_| open_add_host.call(())>
                                {compose_icon("plus")}
                                <span>{move || t(locale.get(), "hosts.add")}</span>
                            </button>
                            <span></span>
                            <button type="button" on:click=move |_| import_ssh_hosts.call(())>
                                {move || t(locale.get(), "hosts.import")}
                            </button>
                            {is_windows().then(|| view! {
                                <button type="button" on:click=move |_| import_wsl_contexts.call(())>
                                    {move || t(locale.get(), "contexts.import_wsl")}
                                </button>
                            })}
                        </div>
                        <div class="settings-list environment-settings-list">
                            {move || {
                                let contexts = execution_contexts.get();
                                let hosts = ssh_hosts.get();
                                if contexts.is_empty() {
                                    return view! { <div class="settings-list-empty">{t(locale.get(), "environments.empty")}</div> }.into_view();
                                }
                                contexts.into_iter().map(|context| {
                                    let context_id = context.id.clone();
                                    let title = if context.kind == "local" {
                                        t(locale.get(), "compute.local").to_string()
                                    } else if context.label.trim().is_empty() {
                                        context.id.clone()
                                    } else {
                                        context.label.clone()
                                    };
                                    let connection = context.id.strip_prefix("ssh:")
                                        .and_then(|alias| hosts.iter().find(|host| host.alias == alias))
                                        .map(|host| match (&host.user, host.port) {
                                            (Some(user), Some(port)) => format!("{user}@{}:{port}", host.alias),
                                            (Some(user), None) => format!("{user}@{}", host.alias),
                                            (None, Some(port)) => format!("{}:{port}", host.alias),
                                            (None, None) => host.alias.clone(),
                                        })
                                        .unwrap_or_else(|| context.id.clone());
                                    let capability_summary = format!(" · {}", context_capability_summary(&context));
                                    let config_context = context.clone();
                                    let probe_id = context_id.clone();
                                    let probe_busy_id = context_id.clone();
                                    let probe_label_id = context_id.clone();
                                    let probe_status_id = context_id.clone();
                                    let is_ssh = context.kind == "ssh";
                                    let ssh_alias = context.id.strip_prefix("ssh:").map(str::to_string);
                                    let edit_alias = ssh_alias.clone();
                                    let remove_alias = ssh_alias;
                                    view! {
                                        <div class="settings-list-row environment-settings-row" data-context-id=context_id>
                                            <span class="environment-server-icon">
                                                {compose_icon("server")}
                                            </span>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{title}</span>
                                                <span class="settings-list-sub">
                                                    {connection}
                                                    {capability_summary}
                                                </span>
                                                {move || (probing_context_id.get().as_deref() == Some(probe_status_id.as_str())).then(|| view! {
                                                    <span class="environment-probe-feedback" role="status">
                                                        <span class="environment-probe-spinner" aria-hidden="true"></span>
                                                        {if is_ssh {
                                                            t(locale.get(), "contexts.probing_ssh")
                                                        } else {
                                                            t(locale.get(), "contexts.probing_local")
                                                        }}
                                                    </span>
                                                })}
                                            </div>
                                            <div class="settings-list-actions">
                                                {edit_alias.map(|alias| view! {
                                                    <button type="button" class="environment-edit"
                                                        title=move || t(locale.get(), "environments.edit")
                                                        aria-label=move || t(locale.get(), "environments.edit")
                                                        on:click=move |_| edit_ssh_host.call(alias.clone())>
                                                        {t(locale.get(), "environments.edit")}
                                                    </button>
                                                })}
                                                <button type="button" class="environment-runtime-config"
                                                    title=move || t(locale.get(), "contexts.configure_interpreters")
                                                    aria-label=move || t(locale.get(), "contexts.configure_interpreters")
                                                    on:click=move |_| runtime_interpreter_form.set(Some(
                                                        RuntimeInterpreterForm::from_context(&config_context)
                                                    ))>
                                                    {t(locale.get(), "runtime.configure")}
                                                </button>
                                                <button type="button" class="environment-probe"
                                                    disabled=move || probing_context_id.get().is_some()
                                                    aria-busy=move || if probing_context_id.get().as_deref() == Some(probe_busy_id.as_str()) { "true" } else { "false" }
                                                    on:click=move |_| probe_compute_resource.call(probe_id.clone())>
                                                    {move || if probing_context_id.get().as_deref() == Some(probe_label_id.as_str()) {
                                                        t(locale.get(), "contexts.probing")
                                                    } else {
                                                        t(locale.get(), "contexts.probe")
                                                    }}
                                                </button>
                                                <span class="environment-remove-slot">
                                                    {remove_alias.map(|alias| view! {
                                                        <button type="button" class="settings-list-remove"
                                                            title=move || t(locale.get(), "environments.remove")
                                                            aria-label=move || t(locale.get(), "environments.remove")
                                                            on:click=move |_| remove_ssh_host.call(alias.clone())>
                                                            {compose_icon("close")}
                                                        </button>
                                                    })}
                                                </span>
                                            </div>
                                        </div>
                                    }.into_view()
                                }).collect_view()
                            }}
                        </div>
                    </div>
                }.into_view())}
                {move || joining.get().then(|| view! {
                    <div class="overlay project-sync-join-overlay"
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Escape" {
                                ev.prevent_default();
                                ev.stop_propagation();
                                joining.set(false);
                            }
                        }>
                        <div class="modal project-sync-join-modal" role="dialog"
                            aria-modal="true"
                            aria-label=move || t(locale.get(), "projects.sync.join_title")
                            aria-describedby="sync-join-hint">
                            <div class="ps-head">
                                <h2>{move || t(locale.get(), "projects.sync.join_title")}</h2>
                                <button type="button" class="ps-close"
                                    title=move || t(locale.get(), "projects.cancel")
                                    aria-label=move || t(locale.get(), "projects.cancel")
                                    on:click=move |_| joining.set(false)>{compose_icon("close")}</button>
                            </div>
                            <p id="sync-join-hint" class="project-sync-join-hint">
                                {move || t(locale.get(), "projects.sync.join_hint")}
                            </p>
                            <div class="project-sync-code-head">
                                <label for="sync-device-code">
                                    {move || t(locale.get(), "projects.sync.code_label")}
                                </label>
                                <button type="button" class="project-sync-guide" on:click=open_sync_guide>
                                    {compose_icon("doc")}
                                    <span>{move || t(locale.get(), "projects.sync.guide")}</span>
                                </button>
                            </div>
                            <textarea id="sync-device-code" data-testid="sync-device-code" rows="5"
                                autofocus=true autocomplete="off" spellcheck="false"
                                placeholder=move || t(locale.get(), "projects.sync.code_placeholder")
                                prop:value=move || join_code.get()
                                on:input=move |ev| join_code.set(event_target_value(&ev))></textarea>
                            {move || join_error.get().map(|message| view! {
                                <div class="settings-status fail" role="alert">{message}</div>
                            })}
                            <div class="row project-sync-join-actions">
                                <button type="button" disabled=move || join_busy.get()
                                    on:click=move |_| joining.set(false)>
                                    {move || t(locale.get(), "projects.cancel")}</button>
                                <button type="button" class="primary"
                                    disabled=move || join_busy.get() || join_code.get().trim().is_empty()
                                    on:click=join_project>{move || t(locale.get(), "projects.sync.join_action")}</button>
                            </div>
                        </div>
                    </div>
                })}
                {move || (settings_section.get() == "appearance").then(|| view! {
                    <div class="settings-pane settings-appearance-pane">
                        <section class="appearance-theme-section">
                            <h3>{move || t(locale.get(), "appearance.theme")}</h3>
                            <div class="theme-mode-grid" role="radiogroup"
                                aria-label=move || t(locale.get(), "appearance.theme")>
                                {[
                                    ("system", "appearance.system", "theme-preview-system"),
                                    ("light", "appearance.light", "theme-preview-light"),
                                    ("dark", "appearance.dark", "theme-preview-dark"),
                                ].into_iter().map(|(mode, label_key, preview_class)| view! {
                                    <button type="button"
                                        class="theme-mode-card"
                                        class:active=move || theme_mode.get() == mode
                                        aria-pressed=move || theme_mode.get() == mode
                                        data-testid=format!("theme-mode-{mode}")
                                        on:click=move |_| theme_mode.set(mode.into())>
                                        <span class=format!("theme-mode-preview {preview_class}") aria-hidden="true">
                                            <span class="theme-preview-window">
                                                <span class="theme-preview-sidebar"></span>
                                                <span class="theme-preview-content">
                                                    <i></i><i></i><i></i>
                                                </span>
                                            </span>
                                        </span>
                                        <span>{move || t(locale.get(), label_key)}</span>
                                    </button>
                                }).collect_view()}
                            </div>
                        </section>
                        <div class="appearance-diff-preview" aria-hidden="true">
                            <div class="appearance-diff-column is-removed">
                                <div><b>"1"</b><code><em>"const"</em> " themePreview: "<i>"ThemeConfig"</i>" = {"</code></div>
                                <div><b>"2"</b><code>"  surface: "<span>"\"sidebar\""</span>","</code></div>
                                <div><b>"3"</b><code>"  accent: "<span>"\"#2563eb\""</span>","</code></div>
                                <div><b>"4"</b><code>"  contrast: "<strong>"42"</strong>","</code></div>
                                <div><b>"5"</b><code>"};"</code></div>
                            </div>
                            <div class="appearance-diff-column is-added">
                                <div><b>"1"</b><code><em>"const"</em> " themePreview: "<i>"ThemeConfig"</i>" = {"</code></div>
                                <div><b>"2"</b><code>"  surface: "<span>"\"sidebar-elevated\""</span>","</code></div>
                                <div><b>"3"</b><code>"  accent: "<span>"\"#0ea5e9\""</span>","</code></div>
                                <div><b>"4"</b><code>"  contrast: "<strong>"68"</strong>","</code></div>
                                <div><b>"5"</b><code>"};"</code></div>
                            </div>
                        </div>
                        {move || {
                            let dark = theme_mode.get() == "dark";
                            let palette = if dark { dark_palette.get() } else { light_palette.get() };
                            let (accent, background, foreground) = appearance_palette_meta(dark, &palette);
                            let accent_ink = if dark && palette == "gruvbox" { "#1D2021" } else { "#FFFFFF" };
                            let background_ink = if dark { "#FFFFFF" } else { "#1F2328" };
                            let foreground_ink = if dark { "#1F2328" } else { "#FFFFFF" };
                            let options = appearance_palette_options(dark);
                            view! {
                                <section class="appearance-config-card">
                                    <div class="appearance-config-head">
                                        <strong>{t(locale.get(), if dark { "appearance.dark_theme" } else { "appearance.light_theme" })}</strong>
                                        <select data-testid="appearance-palette-select"
                                            aria-label=t(locale.get(), "appearance.palette")
                                            on:change=move |ev| {
                                                let value = dom_value(&ev);
                                                if dark { dark_palette.set(value); } else { light_palette.set(value); }
                                            }>
                                            {options.into_iter().map(|(value, name)| view! {
                                                <option value=value
                                                    prop:selected=move || if dark {
                                                        dark_palette.get() == value
                                                    } else {
                                                        light_palette.get() == value
                                                    }>{name}</option>
                                            }).collect_view()}
                                        </select>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.accent")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{accent};--appearance-ink:{accent_ink}")><i></i>{accent}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.background")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{background};--appearance-ink:{background_ink}")><i></i>{background}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.foreground")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{foreground};--appearance-ink:{foreground_ink}")><i></i>{foreground}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.ui_font_size")}</strong>
                                            <span>{t(locale.get(), "appearance.ui_font_size_hint")}</span>
                                        </div>
                                        <label class="font-size-control">
                                            <input type="range" min="12" max="18" step="1"
                                                aria-label=t(locale.get(), "appearance.ui_font_size")
                                                prop:value=move || ui_font_size.get().to_string()
                                                on:input=move |ev| ui_font_size.set(event_target_value(&ev).parse().unwrap_or(14)) />
                                            <output>{move || format!("{} px", ui_font_size.get())}</output>
                                        </label>
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.code_font_size")}</strong>
                                            <span>{t(locale.get(), "appearance.code_font_size_hint")}</span>
                                        </div>
                                        <label class="font-size-control">
                                            <input type="range" min="10" max="18" step="1"
                                                aria-label=t(locale.get(), "appearance.code_font_size")
                                                prop:value=move || code_font_size.get().to_string()
                                                on:input=move |ev| code_font_size.set(event_target_value(&ev).parse().unwrap_or(12)) />
                                            <output>{move || format!("{} px", code_font_size.get())}</output>
                                        </label>
                                    </div>
                                </section>
                            }
                        }}
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "models").then(|| {
                    if acp_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage acp-agents-pane" data-testid="acp-agents-settings">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "models.acp_label")}
                                            <input data-testid="acp-agent-label"
                                                prop:value=move || acp_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o { o.label = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "models.acp_command")}
                                            <input data-testid="acp-agent-command"
                                                prop:value=move || acp_form.get().map(|f| f.command.clone()).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o { o.command = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "models.acp_args")}
                                            <textarea data-testid="acp-agent-args" rows="5"
                                                prop:value=move || acp_form.get().map(|f| f.args.join("\n")).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o {
                                                    o.args = event_target_value(&ev).split('\n').map(|arg| arg.to_string()).collect();
                                                })></textarea></label>
                                    </div>
                                    <span class="hint">{move || t(locale.get(), "models.acp_subpage_hint")}</span>
                                    {move || acp_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| {
                                            acp_form.set(None);
                                            acp_form_msg.set(None);
                                        }>{move || t(locale.get(), "settings.cancel")}</button>
                                        <button type="button" class="primary" data-testid="save-acp-agent" disabled=move || settings_busy.get()
                                            on:click=move |_| {
                                                let Some(mut profile) = acp_form.get() else { return; };
                                                profile.label = profile.label.trim().to_string();
                                                profile.command = profile.command.trim().to_string();
                                                if profile.label.is_empty() || profile.command.is_empty() {
                                                    acp_form_msg.set(Some((false, t(locale.get(), "models.acp_required").to_string())));
                                                    return;
                                                }
                                                let saved = t(locale.get(), "models.acp_saved").to_string();
                                                spawn_local(async move {
                                                    settings_busy.set(true);
                                                    let arg = to_value(&serde_json::json!({ "profile": profile })).unwrap();
                                                    match invoke_checked("save_acp_agent", arg).await {
                                                        Ok(value) => match serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(value) {
                                                            Ok(list) => {
                                                                acp_agents.set(list);
                                                                acp_form.set(None);
                                                                acp_form_msg.set(Some((true, saved)));
                                                                show_acp_agents.set(true);
                                                            }
                                                            Err(error) => {
                                                                acp_form_msg.set(Some((false, error.to_string())));
                                                            }
                                                        },
                                                        Err(error) => {
                                                            acp_form_msg.set(Some((false, js_error_text(error))));
                                                        }
                                                    }
                                                    settings_busy.set(false);
                                                });
                                            }>{move || t(locale.get(), "models.acp_save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else if model_form_open.get() {
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
                                                >
                                                <option value="openai"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "openai")>
                                                    {move || t(locale.get(), "settings.provider.openai")}
                                                </option>
                                                <option value="openai_responses"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "openai_responses")>
                                                    {move || t(locale.get(), "settings.provider.openai_responses")}
                                                </option>
                                                <option value="anthropic"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "anthropic")>
                                                    {move || t(locale.get(), "settings.provider.anthropic")}
                                                </option>
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
                                                >
                                                <option value="default"
                                                    prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort.is_empty())>
                                                    {move || t(locale.get(), "settings.reasoning_effort.default")}
                                                </option>
                                                <option value="none" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "none")>"none"</option>
                                                <option value="minimal" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "minimal")>"minimal"</option>
                                                <option value="low" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "low")>"low"</option>
                                                <option value="medium" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "medium")>"medium"</option>
                                                <option value="high" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "high")>"high"</option>
                                                <option value="xhigh" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "xhigh")>"xhigh"</option>
                                                <option value="max" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "max")>"max"</option>
                                                <option value="ultra" prop:selected=move || model_form.get().is_some_and(|f| f.reasoning_effort == "ultra")>"ultra"</option>
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
                        <div class="settings-pane settings-pane-list model-settings-pane">
                            <div class="settings-toolbar settings-toolbar-end model-category-toolbar">
                                <div class="settings-category-tabs" role="tablist" aria-label="Model categories">
                                    <button type="button" role="tab" class="settings-category-tab"
                                        class:active=move || !show_acp_agents.get()
                                        aria-selected=move || (!show_acp_agents.get()).to_string()
                                        data-testid="models-category-http"
                                        on:click=move |_| show_acp_agents.set(false)>
                                        {move || {
                                            let n = models.get().len();
                                            format!("{} ({n})", t(locale.get(), "models.category.http"))
                                        }}
                                    </button>
                                    <button type="button" role="tab" class="settings-category-tab"
                                        class:active=move || show_acp_agents.get()
                                        aria-selected=move || show_acp_agents.get().to_string()
                                        data-testid="open-acp-agents-from-settings"
                                        on:click=move |_| show_acp_agents.set(true)>
                                        {move || {
                                            let n = acp_agents.get().len();
                                            format!("{} ({n})", t(locale.get(), "models.acp_open"))
                                        }}
                                    </button>
                                </div>
                                <div class="settings-toolbar-actions">
                                    {move || if show_acp_agents.get() {
                                        view! {
                                            <button type="button" class="settings-add-btn" data-testid="add-acp-agent-settings" on:click=move |_| {
                                                show_acp_agents.set(true);
                                                acp_form.set(Some(new_acp_form()));
                                                acp_form_msg.set(None);
                                            }>{move || t(locale.get(), "models.add_acp")}</button>
                                        }.into_view()
                                    } else {
                                        view! {
                                            <button type="button" class="settings-add-btn" on:click=move |_| {
                                                show_acp_agents.set(false);
                                                model_form.set(Some(new_model_form()));
                                                model_form_key.set(String::new());
                                                model_form_msg.set(None);
                                            }>{move || t(locale.get(), "models.add")}</button>
                                        }.into_view()
                                    }}
                                </div>
                            </div>
                            {move || if show_acp_agents.get() {
                                view! {
                                    <div class="acp-agents-pane" data-testid="acp-agents-settings">
                                        <p class="hint">{move || t(locale.get(), "models.acp_subpage_hint")}</p>
                                        {move || acp_form_msg.get().map(|(ok, text)| view! {
                                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                        })}
                                        <div class="settings-list" data-testid="acp-agents-list">
                                            <For each=move || acp_agents.get() key=|agent| agent.id.clone() let:agent>
                                                {
                                                    let edit = agent.clone();
                                                    let id_for_test = agent.id.clone();
                                                    let id_for_delete = agent.id.clone();
                                                    let is_active = active_acp_agent_id.get().as_deref() == Some(agent.id.as_str());
                                                    view! {
                                                        <div class="settings-list-row settings-list-row-link"
                                                            data-testid="acp-agent-row"
                                                            class:settings-list-row-active=is_active
                                                            on:click=move |_| {
                                                                acp_form.set(Some(edit.clone()));
                                                                acp_form_msg.set(None);
                                                            }>
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">
                                                                    {agent.label.clone()}
                                                                    {is_active.then(|| view! { <span class="settings-active-mark" title="active">" ✓"</span> })}
                                                                </span>
                                                                <span class="settings-list-sub">{agent.command.clone()}</span>
                                                            </div>
                                                            <div class="settings-list-actions">
                                                                {is_active.then(|| view! {
                                                                    <span class="settings-active-mark" title="active">"✓"</span>
                                                                })}
                                                                <button class="settings-list-use" type="button" data-testid="test-acp-agent"
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id_for_test.clone();
                                                                        spawn_local(async move {
                                                                            settings_busy.set(true);
                                                                            let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                                            match invoke_checked("test_acp_agent", args).await {
                                                                                Ok(value) => match serde_wasm_bindgen::from_value::<AcpAgentInfo>(value) {
                                                                                    Ok(info) => {
                                                                                        acp_infos.update(|infos| {
                                                                                            infos.insert(id, info);
                                                                                        });
                                                                                        acp_form_msg.set(None);
                                                                                    }
                                                                                    Err(error) => acp_form_msg.set(Some((false, error.to_string()))),
                                                                                },
                                                                                Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                                            }
                                                                            settings_busy.set(false);
                                                                        });
                                                                    }>{move || t(locale.get(), "models.acp_test")}</button>
                                                                <button class="settings-list-remove" type="button" title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id_for_delete.clone();
                                                                        spawn_local(async move {
                                                                            settings_busy.set(true);
                                                                            let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                                            match invoke_checked("remove_acp_agent", args).await {
                                                                                Ok(value) => {
                                                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(value) {
                                                                                        acp_agents.set(list);
                                                                                        acp_infos.update(|infos| {
                                                                                            infos.remove(&id);
                                                                                        });
                                                                                        if active_acp_agent_id.get().as_deref() == Some(id.as_str()) {
                                                                                            active_acp_agent_id.set(None);
                                                                                        }
                                                                                    }
                                                                                }
                                                                                Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                                            }
                                                                            settings_busy.set(false);
                                                                        });
                                                                    }>{compose_icon("close")}</button>
                                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                            </div>
                                                            {move || {
                                                                let id = agent.id.clone();
                                                                acp_infos.get().get(&id).cloned().map(|info| {
                                                                    // "Codex 1.1.2 · ACP v1": the agent's own version first, so the
                                                                    // protocol version is not mistaken for it (#200).
                                                                    let mut version_label = format!("ACP v{}", info.protocol_version);
                                                                    if let Some(implementation) = info.implementation.as_ref() {
                                                                        let name = implementation.get("title").and_then(serde_json::Value::as_str)
                                                                            .or_else(|| implementation.get("name").and_then(serde_json::Value::as_str));
                                                                        if let Some(name) = name {
                                                                            let version = implementation.get("version").and_then(serde_json::Value::as_str).unwrap_or("");
                                                                            version_label = if version.is_empty() {
                                                                                format!("{name} · {version_label}")
                                                                            } else {
                                                                                format!("{name} {version} · {version_label}")
                                                                            };
                                                                        }
                                                                    }
                                                                    let methods = info.auth_methods;
                                                                    view! {
                                                                        <div class="acp-agent-info" data-testid="acp-agent-info" on:click=|ev| ev.stop_propagation()>
                                                                            <span>{version_label}</span>
                                                                            {methods.into_iter().map(|method| {
                                                                                let id = id.clone();
                                                                                let method_id = method.id.clone();
                                                                                view! {
                                                                                    <button type="button" data-testid="authenticate-acp-agent" title=method.description.clone().unwrap_or_default()
                                                                                        on:click=move |ev| {
                                                                                            ev.stop_propagation();
                                                                                            let id = id.clone();
                                                                                            let method_id = method_id.clone();
                                                                                            spawn_local(async move {
                                                                                                let args = to_value(&serde_json::json!({ "id": id, "methodId": method_id })).unwrap();
                                                                                                match invoke_checked("authenticate_acp_agent", args).await {
                                                                                                    Ok(_) => acp_form_msg.set(Some((true, t(Locale::detect_browser(), "models.acp_auth_ok").into()))),
                                                                                                    Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                                                                }
                                                                                            });
                                                                                        }>{method.name.clone()}</button>
                                                                                }
                                                                            }).collect_view()}
                                                                        </div>
                                                                    }
                                                                })
                                                            }}
                                                        </div>
                                                    }
                                                }
                                            </For>
                                        </div>
                                        {move || acp_agents.get().is_empty().then(|| view! {
                                            <p class="model-empty-hint">{move || t(locale.get(), "models.empty_acp")}</p>
                                        })}
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                    <p class="hint" data-testid="acp-models-list-hint">{move || t(locale.get(), "models.acp_hint")}</p>
                                    <div class="settings-list">
                                        <For each=move || models.get() key=|m| (m.id.clone(), m.active) let:m>
                                            {
                                                let pick_id = m.id.clone();
                                                let del_id = m.id.clone();
                                                let del_label = m.label.clone();
                                                let edit = m.clone();
                                                let is_active = m.active;
                                                let can_delete = models.get().len() > 1;
                                                let show_sub = !m.model.is_empty() && m.model != m.label;
                                                view! {
                                                    <div class="settings-list-row settings-list-row-link"
                                                        class:settings-list-row-active=is_active
                                                        on:click=move |_| {
                                                            let form = profile_to_form(&edit);
                                                            show_acp_agents.set(false);
                                                            model_form.set(Some(form));
                                                            model_form_key.set(String::new());
                                                            model_form_msg.set(None);
                                                        }>
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">
                                                                {m.label.clone()}
                                                                {m.use_for_vision.then(|| view! { <span class="settings-cap-badge" title="vision">"vision"</span> })}
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
                                                                        model_delete_confirm.set(Some((id.clone(), del_label.clone())));
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
                                }.into_view()
                            }}
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "pet").then(|| view! {
                    <div class="settings-pane pet-settings-pane">
                        <div class="pet-settings-hero">
                            <div class="pet-settings-preview" class:empty=move || pet_status.get().asset.is_none()>
                                {move || pet_status.get().asset.map(|asset| {
                                    let style = format!("background-image:url('{}')", asset.spritesheet_data_url);
                                    view! { <span class="pet-settings-sprite" style=style aria-hidden="true"></span> }
                                })}
                            </div>
                            <div class="pet-settings-copy">
                                <h3>{move || pet_status.get().asset.map(|asset| asset.display_name).unwrap_or_else(|| t(locale.get(), "pet.not_configured").into())}</h3>
                                <p>{move || pet_status.get().asset.map(|asset| asset.description).filter(|text| !text.is_empty()).unwrap_or_else(|| t(locale.get(), "pet.description").into())}</p>
                                {move || pet_status.get().asset.map(|asset| view! {
                                    <div class="pet-settings-meta">
                                        <code>{asset.id}</code>
                                        <span>{format!("v{}", asset.sprite_version_number)}</span>
                                        <code title=pet_status.get().directory>{pet_status.get().directory}</code>
                                    </div>
                                })}
                            </div>
                        </div>
                        <div class="appearance-config-card pet-config-card">
                            <div class="appearance-config-row">
                                <div>
                                    <strong>{move || t(locale.get(), "pet.enabled")}</strong>
                                    <span>{move || t(locale.get(), "pet.enabled_hint")}</span>
                                </div>
                                <label class="toggle">
                                    <input type="checkbox" data-testid="pet-enabled"
                                        prop:checked=move || settings.get().pet_enabled
                                        on:change=move |ev| settings.update(|current| current.pet_enabled = event_target_checked(&ev)) />
                                    <span class="toggle-track" aria-hidden="true"></span>
                                </label>
                            </div>
                            <div class="pet-directory-row">
                                <label>{move || t(locale.get(), "pet.directory")}
                                    <div class="settings-path-row">
                                        <input class="settings-path-input" data-testid="pet-directory"
                                            prop:value=move || settings.get().pet_directory
                                            placeholder=move || t(locale.get(), "pet.directory_placeholder")
                                            on:input=move |ev| settings.update(|current| current.pet_directory = event_target_input(&ev).value()) />
                                        <button type="button" data-testid="pet-choose" on:click=choose_pet_directory>
                                            {move || t(locale.get(), "projects.choose_dir")}
                                        </button>
                                    </div>
                                    <span class="settings-field-hint">{move || t(locale.get(), "pet.directory_hint")}</span>
                                </label>
                            </div>
                        </div>
                        {move || pet_status.get().error.map(|error| view! {
                            <div class="settings-status fail">{error}</div>
                        })}
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
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
                                                data-testid="reviewer-backend-select"
                                                on:change=move |ev| specialist_form.update(|o| if let Some(o)=o {
                                                    let value = dom_value(&ev);
                                                    if o.id == "reviewer" {
                                                        set_reviewer_backend(o, &value);
                                                    } else {
                                                        o.model_id = value;
                                                    }
                                                })>
                                                {move || if specialist_form.get().is_some_and(|f| f.id == "reviewer") {
                                                    view! {
                                                        <option value="http:"
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| reviewer_backend_key(&f) == "http:")>
                                                            {t(locale.get(), "composer.reviewer.default_http")}
                                                        </option>
                                                        <option value="follow_session"
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| reviewer_backend_key(&f) == "follow_session")>
                                                            {t(locale.get(), "composer.reviewer.follow_session")}
                                                        </option>
                                                    }.into_view()
                                                } else {
                                                    view! {
                                                        <option value=""
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| f.model_id.is_empty())>
                                                            {t(locale.get(), "specialists.model.follow")}
                                                        </option>
                                                    }.into_view()
                                                }}
                                                {move || specialist_form.get()
                                                    .filter(|f| f.id == "reviewer")
                                                    .and_then(|reviewer| reviewer_missing_acp_profile_id(
                                                        &reviewer,
                                                        &acp_agents.get(),
                                                    ))
                                                    .map(|profile_id| {
                                                        let value = format!("acp:{profile_id}");
                                                        let label = format!(
                                                            "{} · {profile_id}",
                                                            t(locale.get(), "composer.reviewer.missing_acp"),
                                                        );
                                                        view! {
                                                            <option value=value prop:selected=true disabled=true
                                                                data-testid="reviewer-missing-acp-option">
                                                                {label}
                                                            </option>
                                                        }
                                                    })}
                                                {move || models.get().into_iter().map(|m| {
                                                    let value = if specialist_form.get().is_some_and(|f| f.id == "reviewer") {
                                                        format!("http:{}", m.id)
                                                    } else {
                                                        m.id.clone()
                                                    };
                                                    let selected_value = value.clone();
                                                    view! {
                                                        <option value=value prop:selected=move || specialist_form.get().is_some_and(|f| {
                                                            if f.id == "reviewer" {
                                                                reviewer_backend_key(&f) == selected_value
                                                            } else {
                                                                f.model_id == selected_value
                                                            }
                                                        })>{m.label.clone()}</option>
                                                    }
                                                }).collect_view()}
                                                {move || specialist_form.get().is_some_and(|f| f.id == "reviewer").then(|| view! {
                                                    <optgroup label="ACP Agents">
                                                        {acp_agents.get().into_iter().map(|agent| {
                                                            let value = format!("acp:{}", agent.id);
                                                            let selected_value = value.clone();
                                                            view! {
                                                                <option value=value prop:selected=move || specialist_form.get().is_some_and(|f| {
                                                                    reviewer_backend_key(&f) == selected_value
                                                                })>{format!("{} · ACP", agent.label)}</option>
                                                            }
                                                        }).collect_view()}
                                                    </optgroup>
                                                })}
                                            </select>
                                        </label>
                                        {move || specialist_form.get().filter(|f| f.id == "reviewer").map(|reviewer| {
                                            let backend = reviewer_backend_label(
                                                &reviewer,
                                                &models.get(),
                                                &acp_agents.get(),
                                                &t(locale.get(), "composer.reviewer.follow_session"),
                                                &t(locale.get(), "composer.reviewer.missing_acp"),
                                            ).unwrap_or_else(|| t(locale.get(), "composer.reviewer.default_http"));
                                            view! {
                                                <span class="hint span-2" data-testid="reviewer-selected-backend">
                                                    {tf(locale.get(), "specialists.reviewer.selected_backend", &[("backend", &backend)])}
                                                </span>
                                                <span class="hint span-2">{move || t(locale.get(), "specialists.reviewer.test_hint")}</span>
                                            }
                                        })}
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
                                        {move || specialist_form.get().is_some_and(|f| f.id == "reviewer").then(|| view! {
                                            <button type="button" data-testid="test-reviewer-backend"
                                                disabled=move || settings_busy.get()
                                                on:click=move |ev| test_reviewer_form.call(ev)>
                                                {move || t(locale.get(), "specialists.reviewer.test")}
                                            </button>
                                        })}
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
                                            review_backend: None,
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
                                let skills = skills_list.get();
                                let visible = skills.iter().filter(|s| {
                                    skill_matches_filter(s, &tag, &q)
                                }).count();
                                let enabled = skills.iter().filter(|s| s.enabled).count();
                                tf(locale.get(), "skills.summary", &[
                                    ("visible", &visible.to_string()),
                                    ("enabled", &enabled.to_string()),
                                    ("total", &skills.len().to_string()),
                                ])
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
                            <button class:active=move || skill_filter_tag.get() == "__enabled"
                                on:click=move |_| skill_filter_tag.set("__enabled".into())>
                                {move || t(locale.get(), "skills.enabled")}
                            </button>
                            <button class:active=move || skill_filter_tag.get() == "__disabled"
                                on:click=move |_| skill_filter_tag.set("__disabled".into())>
                                {move || t(locale.get(), "skills.disabled")}
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
                        <p class="settings-note">{move || t(locale.get(), "settings.auto_saved_new_session")}</p>
                        {move || skills_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        {move || {
                            let q = skills_search.get().trim().to_lowercase();
                            let tag = skill_filter_tag.get();
                            (!skills_list.get().iter().any(|s| skill_matches_filter(s, &tag, &q))).then(|| view! {
                                <p class="skill-filter-empty">{move || t(locale.get(), "skills.empty")}</p>
                            })
                        }}
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
                                    let tags_input_text = tags_text.clone();
                                    let tags_cb = save_skill_tags.clone();
                                    view! {
                                        <div class="settings-list-row" data-skill-name=s.name.clone()>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{s.name.clone()}</span>
                                                {(!s.description.is_empty() && s.description != ">").then(|| {
                                                    let desc = s.description.clone();
                                                    view! { <span class="settings-list-sub">{desc}</span> }
                                                })}
                                                <details class="skill-tags-editor">
                                                    <summary>
                                                        <span>{move || t(locale.get(), "skills.edit_tags")}</span>
                                                        <span class="skill-tags-summary">{tags_text}</span>
                                                    </summary>
                                                    <input class="skill-tags-input"
                                                        prop:value=tags_input_text
                                                        prop:placeholder=move || t(locale.get(), "skills.tags_placeholder")
                                                        on:change=move |ev| tags_cb.call((name_tags.clone(), event_target_value(&ev))) />
                                                </details>
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
                        <div class="conn-group-label">{move || t(locale.get(), "cred.custom.name")}</div>
                        <p class="settings-note">{move || t(locale.get(), "cred.custom.hint")}</p>
                        <For
                            each=move || custom_credentials.get()
                            key=|credential| credential.id.clone()
                            let:credential
                        >
                            {
                                let id = credential.id.clone();
                                let status_id = id.clone();
                                let clear_id = id.clone();
                                let input_id = id.clone();
                                let edit_id = id.clone();
                                let remove_id = id.clone();
                                let initial_present = credential.present;
                                view! {
                                    <div class="custom-credential-card" data-custom-credential=credential.env_var.clone()>
                                        <div class="custom-credential-head">
                                            <div class="custom-credential-meta">
                                                <strong>{credential.name.clone()}</strong>
                                                <code>{credential.env_var.clone()}</code>
                                                <span>{move || if cred_status.get().get(&status_id).copied().unwrap_or(initial_present) {
                                                    t(locale.get(), "cred.stored")
                                                } else {
                                                    t(locale.get(), "cred.not_stored")
                                                }}</span>
                                            </div>
                                            <div class="custom-credential-actions">
                                                {move || cred_status.get().get(&clear_id).copied().unwrap_or(initial_present).then(|| {
                                                    let id = clear_id.clone();
                                                    view! {
                                                        <button type="button" class="linklike" on:click=move |_| {
                                                            let id = id.clone();
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "id": id.clone(), "value": "" })).unwrap();
                                                                match invoke_checked("set_credential", arg).await {
                                                                    Ok(_) => {
                                                                        cred_inputs.update(|values| { values.remove(&id); });
                                                                        cred_status.update(|status| { status.insert(id, false); });
                                                                        cred_msg.set(Some((true, t(locale.get(), "cred.cleared").into())));
                                                                    }
                                                                    Err(error) => cred_msg.set(Some((false,
                                                                        localize_backend(locale.get(), &js_error_text(error))))),
                                                                }
                                                            });
                                                        }>{move || t(locale.get(), "cred.clear")}</button>
                                                    }
                                                })}
                                                <button type="button" class="linklike danger" on:click=move |_| {
                                                    let id = remove_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                        match invoke_checked("remove_custom_credential", arg).await {
                                                            Ok(_) => {
                                                                custom_credentials.update(|items| items.retain(|item| item.id != id));
                                                                cred_inputs.update(|values| { values.remove(&id); });
                                                                cred_status.update(|status| { status.remove(&id); });
                                                                cred_msg.set(Some((true, t(locale.get(), "cred.custom.removed").into())));
                                                            }
                                                            Err(error) => cred_msg.set(Some((false,
                                                                localize_backend(locale.get(), &js_error_text(error))))),
                                                        }
                                                    });
                                                }>{move || t(locale.get(), "specialists.remove")}</button>
                                            </div>
                                        </div>
                                        <input type="password"
                                            placeholder=move || if cred_status.get().get(&input_id).copied().unwrap_or(initial_present) {
                                                t(locale.get(), "settings.stored_key").to_string()
                                            } else {
                                                t(locale.get(), "cred.custom.value_placeholder").to_string()
                                            }
                                            prop:value=move || cred_inputs.get().get(&id).cloned().unwrap_or_default()
                                            on:input=move |event| {
                                                let value = event_target_input(&event).value();
                                                cred_inputs.update(|values| { values.insert(edit_id.clone(), value); });
                                            } />
                                    </div>
                                }
                            }
                        </For>
                        <div class="settings-sync-block custom-credential-add">
                            <h3>{move || t(locale.get(), "cred.custom.add")}</h3>
                            <div class="settings-form-grid">
                                <label>
                                    <span>{move || t(locale.get(), "cred.custom.service_name")}</span>
                                    <input type="text"
                                        placeholder=move || t(locale.get(), "cred.custom.service_placeholder")
                                        prop:value=move || custom_cred_name.get()
                                        on:input=move |event| custom_cred_name.set(event_target_input(&event).value()) />
                                </label>
                                <label>
                                    <span>{move || t(locale.get(), "cred.custom.env_var")}</span>
                                    <input type="text" class="mono"
                                        placeholder="METASO_API_KEY"
                                        prop:value=move || custom_cred_env.get()
                                        on:input=move |event| custom_cred_env.set(event_target_input(&event).value()) />
                                </label>
                                <label class="span-2">
                                    <span>{move || t(locale.get(), "cred.custom.value")}</span>
                                    <input type="password"
                                        placeholder=move || t(locale.get(), "cred.custom.value_placeholder")
                                        prop:value=move || custom_cred_value.get()
                                        on:input=move |event| custom_cred_value.set(event_target_input(&event).value()) />
                                </label>
                            </div>
                            <p class="settings-field-hint">{move || t(locale.get(), "cred.custom.env_hint")}</p>
                            <div class="row">
                                <button type="button" class="settings-add-btn"
                                    disabled=move || custom_cred_busy.get()
                                        || custom_cred_name.get().trim().is_empty()
                                        || custom_cred_env.get().trim().is_empty()
                                        || custom_cred_value.get().trim().is_empty()
                                    on:click=move |_| {
                                        if custom_cred_busy.get_untracked() { return; }
                                        let name = custom_cred_name.get_untracked();
                                        let env_var = custom_cred_env.get_untracked();
                                        let value = custom_cred_value.get_untracked();
                                        custom_cred_busy.set(true);
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({
                                                "name": name,
                                                "envVar": env_var,
                                                "value": value,
                                            })).unwrap();
                                            match invoke_checked("add_custom_credential", arg).await {
                                                Ok(value) => match serde_wasm_bindgen::from_value::<CustomCredentialStatus>(value) {
                                                    Ok(credential) => {
                                                        cred_status.update(|status| {
                                                            status.insert(credential.id.clone(), credential.present);
                                                        });
                                                        custom_credentials.update(|items| items.push(credential));
                                                        custom_cred_name.set(String::new());
                                                        custom_cred_env.set(String::new());
                                                        custom_cred_value.set(String::new());
                                                        cred_msg.set(Some((true, t(locale.get(), "cred.custom.added").into())));
                                                    }
                                                    Err(error) => cred_msg.set(Some((false, error.to_string()))),
                                                },
                                                Err(error) => cred_msg.set(Some((false,
                                                    localize_backend(locale.get(), &js_error_text(error))))),
                                            }
                                            custom_cred_busy.set(false);
                                        });
                                    }>{move || if custom_cred_busy.get() {
                                        t(locale.get(), "cred.custom.adding")
                                    } else {
                                        t(locale.get(), "cred.custom.add")
                                    }}</button>
                            </div>
                        </div>
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
                {move || (settings_section.get() == "channels").then(|| view! {
                    <crate::channels_view::ChannelsPane locale=locale open=channels_open/>
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
                                            disabled=move || oauth_authorizing.get()
                                            on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.name = event_target_input(&ev).value(); }) /></label>
                                    <label>{move || t(locale.get(),"conn.kind")}
                                        <select prop:value=move || conn_form.get().map(|f| f.kind.clone()).unwrap_or_else(|| "stdio".into())
                                            disabled=move || oauth_authorizing.get()
                                            on:change=move |ev| {
                                                let kind = dom_value(&ev);
                                                conn_form.update(|form| if let Some(form) = form {
                                                    form.kind = kind;
                                                });
                                            }>
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
                                                disabled=move || oauth_authorizing.get()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.url = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(),"conn.auth")}
                                            <select prop:value=move || conn_form.get().map(|f| f.auth.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| "none".into())
                                                disabled=move || oauth_authorizing.get()
                                                on:change=move |ev| {
                                                    let auth = dom_value(&ev);
                                                    conn_form.update(|form| if let Some(form) = form { form.auth = auth; });
                                                }>
                                                <option value="none">{move || t(locale.get(),"conn.auth.none")}</option>
                                                <option value="oauth">{move || t(locale.get(),"conn.auth.oauth")}</option>
                                            </select>
                                        </label>
                                        <label>{move || t(locale.get(),"conn.headers")}
                                            <input placeholder=move || if conn_form.get().is_some_and(|form| form.auth == "oauth") {
                                                    "X-Custom-Header: value"
                                                } else {
                                                    "Authorization: Bearer token"
                                                }
                                                prop:value=move || conn_form.get().map(|f| f.headers.clone()).unwrap_or_default()
                                                disabled=move || oauth_authorizing.get()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.headers = event_target_input(&ev).value(); }) /></label>
                                    })}
                                    {move || (conn_form_kind.get() == "http"
                                        && conn_form.get().is_some_and(|form| form.auth == "oauth")).then(|| view!{
                                        <p class="settings-note">{move || t(locale.get(), "conn.oauth.desc")}</p>
                                    })}
                                    {move || conn_test_msg.get().map(|(ok,msg)| view!{
                                        <div class="settings-status" class:ok=ok class:fail=move||!ok>{msg}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" disabled=move || oauth_authorizing.get()
                                            on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                            spawn_local(async move {
                                                let oauth = f.kind == "http" && f.auth == "oauth";
                                                if oauth {
                                                    oauth_authorizing.set(true);
                                                    conn_test_msg.set(Some((true, t(locale.get(), "conn.oauth.waiting").into())));
                                                }
                                                let conn = build_conn_json(&f, false);
                                                let command = if oauth {
                                                    "test_oauth_mcp_connection"
                                                } else {
                                                    "test_mcp_connection"
                                                };
                                                match invoke_checked(command, to_value(&serde_json::json!({"conn": conn})).unwrap()).await {
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
                                                if oauth {
                                                    oauth_authorizing.set(false);
                                                }
                                            });
                                        }>{move || t(locale.get(),"conn.test")}</button>
                                        <button type="button"
                                            on:click=move |_| {
                                                if oauth_authorizing.get() {
                                                    spawn_local(async move {
                                                        let _ = invoke_checked("cancel_oauth_authorization", JsValue::UNDEFINED).await;
                                                    });
                                                }
                                                oauth_authorizing.set(false);
                                                close_settings_subpage.call(());
                                            }>{move || t(locale.get(),"settings.cancel")}</button>
                                        <button type="button" class="primary" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                            spawn_local(async move {
                                                if f.kind == "http" && f.auth == "oauth" {
                                                    oauth_authorizing.set(true);
                                                    conn_test_msg.set(Some((true, t(locale.get(), "conn.oauth.waiting").into())));
                                                    let conn = build_conn_json(&f, true);
                                                    let args = to_value(&serde_json::json!({ "conn": conn })).unwrap();
                                                    match invoke_checked("authorize_http_connection", args).await {
                                                        Ok(_) => {
                                                            conn_form.set(None);
                                                            conn_test_msg.set(None);
                                                            refresh_conns.call(());
                                                        }
                                                        Err(error) => {
                                                            conn_test_msg.set(Some((false, js_error_text(error))));
                                                        }
                                                    }
                                                    oauth_authorizing.set(false);
                                                    return;
                                                }
                                                let editing = f.id.is_some();
                                                let conn = build_conn_json(&f, true);
                                                let cmd = if editing { "update_mcp_connection" } else { "add_mcp_connection" };
                                                if invoke_checked(cmd, to_value(&serde_json::json!({"conn": conn})).unwrap()).await.is_ok() {
                                                    conn_form.set(None); conn_test_msg.set(None); refresh_conns.call(());
                                                }
                                            });
                                        } disabled=move || oauth_authorizing.get()>
                                            {move || t(locale.get(), "settings.save")}
                                        </button>
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
                                        let service = c.subtitle.clone();
                                        let enabled = c.enabled;
                                        let transport = c.transport.clone();
                                        let auth = c.auth.clone();
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
                                            {is_custom.then(|| view! {
                                                <div class="settings-list">
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.service")}</span>
                                                            <span class="settings-list-sub">{service}</span>
                                                        </div>
                                                    </div>
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.status")}</span>
                                                            <span class="settings-list-sub">{move || t(locale.get(), if enabled {
                                                                "conn.status.enabled"
                                                            } else {
                                                                "conn.status.disabled"
                                                            })}</span>
                                                        </div>
                                                    </div>
                                                    {(transport == "http").then(|| view! {
                                                        <div class="settings-list-row">
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">{move || t(locale.get(), "conn.auth")}</span>
                                                                <span class="settings-list-sub">{move || t(locale.get(), if auth == "oauth" {
                                                                    "conn.auth.oauth"
                                                                } else {
                                                                    "conn.auth.none"
                                                                })}</span>
                                                            </div>
                                                        </div>
                                                    })}
                                                </div>
                                            })}
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
                                    let auth_badge = match &c.transport {
                                        ConnTransport::Http { auth, .. } if auth == "oauth" => Some("OAuth"),
                                        _ => None,
                                    };
                                    let enabled = c.enabled;
                                    view! {
                                        <div class="settings-list-row settings-list-row-link"
                                            on:click=move |_| {
                                                open_conn_key.set(Some(id_open.clone()));
                                                load_custom_conn_tools.call(row_open.clone());
                                            }>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">
                                                    {c.name.clone()}
                                                    " "
                                                    <span class="badge">{kind_badge}</span>
                                                    {auth_badge.map(|auth| view! { <span class="badge">{auth}</span> })}
                                                </span>
                                                <span class="settings-list-sub">
                                                    {match &c.transport {
                                                        ConnTransport::Stdio { command, .. } => command.clone(),
                                                        ConnTransport::Http { url, .. } => url.clone(),
                                                    }}
                                                </span>
                                                <span class="settings-list-sub">
                                                    {move || t(locale.get(), if enabled {
                                                        "conn.status.enabled"
                                                    } else {
                                                        "conn.status.disabled"
                                                    })}
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
            {move || model_delete_confirm.get().map(|(id, label)| {
                let remove_id = id.clone();
                view! {
                    <div class="overlay" data-testid="model-delete-confirm">
                        <div class="modal confirm-modal">
                            <h2>{move || t(locale.get(), "confirm.title")}</h2>
                            <div class="hint">{move || tf(
                                locale.get(),
                                "models.remove_confirm",
                                &[("model", &label)],
                            )}</div>
                            <div class="row">
                                <button on:click=move |_| model_delete_confirm.set(None)>
                                    {move || t(locale.get(), "settings.cancel")}
                                </button>
                                <button class="primary" on:click=move |_| {
                                    model_delete_confirm.set(None);
                                    let id = remove_id.clone();
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                        if let Ok(value) = invoke_checked("remove_model", arg).await {
                                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(value) {
                                                models.set(list);
                                            }
                                        }
                                    });
                                }>{move || t(locale.get(), "models.remove")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
}.into_view())
    }
}
