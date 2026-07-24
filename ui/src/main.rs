mod agent_workflows;
mod bindings;
mod channels_view;
mod context_menu;
mod dto;
mod i18n;
mod library;
mod notebook;
mod overlays;
mod pet;
mod project_landing;
mod settings_view;
mod sidebar;
mod text;
mod window_titlebar;

use agent_workflows::{
    agent_workflows_panel, refresh_agent_resources, refresh_agent_workflows, AgentPanelState,
};
use bindings::{
    attach_chat_autoscroll, clear_selection, close_mcp_app, force_chat_bottom, invoke,
    invoke_checked, invoke_timeout, is_mac, is_windows, jump_chat_to_user, listen,
    listen_native_file_drop, mount_mcp_app, mount_terminal, native_drop_in_composer,
    open_external_url, park_mcp_app, pasted_image_count, preserve_chat_prepend_position,
    preview_selection, schedule_chat_follow, set_saved_marks, set_terminal_active,
    unmount_terminal, CHAT_SCROLLER_ID, CHAT_THREAD_ID,
};
use context_menu::{ContextMenuPortal, CtxMenu};
use dto::*;
use futures_channel::oneshot;
use i18n::{
    empty_subtitle, empty_title, localize_backend, set_document_lang, t, tab_count, tf, use_locale,
    Locale, EMPTY_SUBTITLE_COUNT, EMPTY_TITLE_COUNT,
};
use leptos::{ev, window_event_listener, *};
use library::{refresh_library, HighlightsPane, LibraryScreen};
use notebook::{collect_notebook_cells, NotebookCache, NotebookView};
use overlays::{AddHostOverlay, CapabilitiesOverlay, OnboardingOverlay, RuntimeInterpreterOverlay};
use pet::{PetDesktop, PetOverlay};
use project_landing::{ProjectLanding, ProjectLandingState};
use serde_wasm_bindgen::to_value;
use settings_view::{DeleteConfirm, SettingsView, SettingsViewState};
use sidebar::{Sidebar, SidebarState};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use text::{
    dom_value, event_target_checked, event_target_value, file_kind, format_bytes,
    format_duration_ms, group_artifact_indices, ime_composing, join_path, md_to_html,
    opens_in_system_browser, parent_path, provider_defaults, provider_value, runtime_language,
    tool_card_label, unique_dom_id, user_message_presentation,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use window_titlebar::WindowTitlebar;

/// Stable substring of the backend's missing-key error (`src-tauri` `send_message`),
/// used to turn that failure into an actionable "open Settings" prompt.
const NO_API_KEY_MARK: &str = "No API key set";
const HOME_SEARCH_PROJECT_LIMIT: usize = 6;
const HOME_SEARCH_ARTIFACT_LIMIT: usize = 8;
const HOME_SEARCH_SESSION_LIMIT: usize = 6;
const TRANSCRIPT_RENDER_TURNS: usize = 40;
const TRANSCRIPT_WINDOW_STEP: usize = 20;
const CENTER_PANE_MIN_WIDTH: f64 = 360.0;
const RIGHT_PANE_MIN_WIDTH: f64 = 320.0;
const RIGHT_PANE_MAX_WIDTH: f64 = 900.0;
const PANE_RESIZER_WIDTH: f64 = 5.0;
const SIDEBAR_RESIZER_WIDTH: f64 = 10.0;
const THEME_STORAGE_KEY: &str = "wisp-theme";
const SIDE_CHAT_SCROLLER_ID: &str = "side-chat-scroller";
/// Reserved `acp_config_menu_open` key for the session-mode dropdown, kept
/// distinct from any agent-supplied config option id.
const ACP_MODE_MENU: &str = "__acp_session_mode";

fn mcp_app_title(payload: &serde_json::Value) -> String {
    payload
        .pointer("/tool/title")
        .or_else(|| payload.pointer("/tool/annotations/title"))
        .or_else(|| payload.pointer("/tool/name"))
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("MCP App")
        .to_string()
}

fn mcp_app_instance_id(
    frame_id: &str,
    presentation_id: &str,
    payload: &serde_json::Value,
) -> String {
    let identity = (!presentation_id.is_empty())
        .then_some(presentation_id)
        .or_else(|| {
            payload
                .pointer("/resource/uri")
                .or_else(|| payload.pointer("/tool/name"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("app");
    format!("mcp-app:{frame_id}:{identity}")
}

#[component]
fn McpAppPreview(instance_id: String, payload_json: String) -> impl IntoView {
    let dom_id = unique_dom_id("center-mcp-app");
    {
        let mount_id = instance_id.clone();
        let mount_dom_id = dom_id.clone();
        let mount_payload = payload_json.clone();
        create_effect(move |_| {
            let _ = mount_mcp_app(&mount_id, &mount_dom_id, &mount_payload);
        });
    }
    {
        let parked_id = instance_id.clone();
        on_cleanup(move || park_mcp_app(&parked_id));
    }
    view! {
        <div class="center-mcp-app" id=dom_id data-mcp-app-id=instance_id></div>
    }
}

fn session_highlight_count(session: Option<String>, items: &[LibraryItem]) -> usize {
    let Some(session) = session else { return 0 };
    items
        .iter()
        .filter(|item| item.kind == "text" && item.source_session_id == session)
        .count()
}

fn max_right_pane_width(sidebar_open: bool, sidebar_width: f64) -> f64 {
    let viewport_width = web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .unwrap_or(RIGHT_PANE_MAX_WIDTH + CENTER_PANE_MIN_WIDTH + SIDEBAR_W_DEFAULT);
    let sidebar_space = if sidebar_open {
        sidebar_width + SIDEBAR_RESIZER_WIDTH
    } else {
        0.0
    };
    let available = viewport_width - sidebar_space - CENTER_PANE_MIN_WIDTH - PANE_RESIZER_WIDTH;
    available.clamp(RIGHT_PANE_MIN_WIDTH, RIGHT_PANE_MAX_WIDTH)
}

#[component]
fn CenterRuntimeConsole(path: String, consoles: RwSignal<RuntimeConsoles>) -> impl IntoView {
    let locale = use_locale();
    let log_path = path.clone();
    let clear_path = path;
    let log = create_memo(move |_| consoles.get().get(&log_path).cloned().unwrap_or_default());
    let output_ref = create_node_ref::<html::Pre>();

    // Follow appended output with ordinary positive scrollTop. The old
    // column-reverse trick made WebKit's scrollbar direction and selection
    // behavior backwards, especially once a whole script filled the console.
    create_effect(move |_| {
        let _ = log.get();
        if let Some(output) = output_ref.get() {
            request_animation_frame(move || output.set_scroll_top(output.scroll_height()));
        }
    });

    view! {
        <div class="center-file-console">
            <div class="center-file-console-head">
                <span>{move || t(locale.get(), "runtime.console")}</span>
                <div class="spacer"></div>
                <button type="button" class="center-file-btn"
                    title=move || t(locale.get(), "runtime.console_clear")
                    aria-label=move || t(locale.get(), "runtime.console_clear")
                    on:click=move |_| consoles.update(|logs| {
                        logs.remove(&clear_path);
                    })>{compose_icon("close")}</button>
            </div>
            <pre node_ref=output_ref class:empty=move || log.get().is_empty()>{move || {
                let text = log.get();
                if text.is_empty() {
                    t(locale.get(), "runtime.console_empty").into()
                } else {
                    text
                }
            }}</pre>
        </div>
    }
}

#[component]
fn CenterRuntimeEnvironment(
    project_id: String,
    context_id: String,
    context_label: String,
    language: String,
    locale: RwSignal<Locale>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    selection_popup: RwSignal<Option<(String, Option<String>, i32, i32)>>,
) -> impl IntoView {
    let state_key = runtime_binding_state_key(&project_id, &context_id, &language);
    let status_project = project_id.clone();
    let status_context = context_id.clone();
    let status_language = language.clone();
    let status = create_memo(move |_| {
        runtimes
            .get()
            .into_iter()
            .find(|runtime| {
                runtime.key.project_id == status_project
                    && runtime.key.context_id == status_context
                    && runtime.key.language == status_language
            })
            .map(|runtime| runtime.status)
            .unwrap_or_else(|| "missing".into())
    });
    let language_label = language_display(&language).to_string();
    let aria_language_label = language_label.clone();
    let title_language_label = language_label.clone();
    let loading_key = state_key.clone();
    let content_key = state_key.clone();
    let refresh_key = state_key;
    let refresh_project = project_id;
    let refresh_context = context_id;
    let refresh_language = language;

    view! {
        <aside class="center-runtime-environment" aria-label=move || {
            tf(locale.get(), "runtime.environment_title", &[("language", &aria_language_label)])
        }>
            <div class="center-runtime-environment-head">
                <div>
                    <h3>{move || tf(locale.get(), "runtime.environment_title", &[("language", &title_language_label)])}</h3>
                    <span>{context_label}</span>
                </div>
                <span class=move || format!("runtime-status {}", status.get())>
                    {move || runtime_status_label(locale.get(), &status.get())}
                </span>
                <button type="button" class="runtime-environment-refresh"
                    title=move || t(locale.get(), "runtime.inspect_objects")
                    aria-label=move || t(locale.get(), "runtime.inspect_objects")
                    disabled=move || status.get() != "ready" || states.with(|states| {
                        states.get(&loading_key).is_some_and(|state| state.loading)
                    })
                    on:click=move |_| inspect_runtime_objects(
                        refresh_key.clone(),
                        refresh_project.clone(),
                        refresh_context.clone(),
                        refresh_language.clone(),
                        locale,
                        states,
                        runtimes,
                    )>{compose_icon("sync")}</button>
            </div>
            <div class="center-runtime-environment-table-head" aria-hidden="true">
                <span>{move || t(locale.get(), "runtime.object_name")}</span>
                <span>{move || t(locale.get(), "runtime.object_type")}</span>
                <span>{move || t(locale.get(), "runtime.object_value")}</span>
                <span>{move || t(locale.get(), "runtime.object_size")}</span>
            </div>
            <div class="center-runtime-environment-body">
                {move || {
                    let state = states.with(|states| {
                        states.get(&content_key).cloned().unwrap_or_default()
                    });
                    if state.loading && state.snapshot.is_none() {
                        return view! {
                            <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_loading")}</div>
                        }.into_view();
                    }
                    if let Some(error) = state.error {
                        return view! { <div class="context-error">{error}</div> }.into_view();
                    }
                    let Some(snapshot) = state.snapshot else {
                        let key = if status.get() == "ready" {
                            "runtime.objects_hint"
                        } else {
                            "runtime.environment_unavailable"
                        };
                        return view! {
                            <div class="runtime-environment-empty">{t(locale.get(), key)}</div>
                        }.into_view();
                    };
                    if snapshot.objects.is_empty() {
                        return view! {
                            <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_empty")}</div>
                        }.into_view();
                    }
                    let shown = snapshot.objects.len();
                    let total = snapshot.total_count;
                    view! {
                        <div class="center-runtime-environment-rows">
                            {snapshot.objects.into_iter().map(|object| {
                                let size = object.size_bytes.map(format_bytes).unwrap_or_else(|| "—".into());
                                let summary = if object.summary.is_empty() { "—".into() } else { object.summary };
                                let quote = runtime_object_quote(
                                    &language_label, &object.name, &object.type_name, &summary, &size,
                                );
                                view! {
                                    <div class="center-runtime-environment-row" role="button" tabindex="0"
                                        title=move || t(locale.get(), "runtime.quote_object")
                                        on:click=move |event: web_sys::MouseEvent| selection_popup.set(Some((
                                            quote.clone(), None, event.client_x(), event.client_y(),
                                        )))>
                                        <span class="runtime-object-name" title=object.name.clone()>{object.name}</span>
                                        <span class="runtime-object-type" title=object.type_name.clone()>{object.type_name}</span>
                                        <span class="runtime-object-value" title=summary.clone()>{summary}</span>
                                        <span class="runtime-object-size">{size}</span>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                        {(shown < total).then(|| view! {
                            <div class="runtime-objects-limit">{
                                tf(locale.get(), "runtime.objects_showing", &[
                                    ("shown", &shown.to_string()),
                                    ("total", &total.to_string()),
                                ])
                            }</div>
                        })}
                    }.into_view()
                }}
            </div>
        </aside>
    }
}

#[derive(Default)]
struct ProjectOpenGate {
    held: bool,
    waiters: VecDeque<oneshot::Sender<()>>,
}

struct ProjectOpenPermit(Rc<RefCell<ProjectOpenGate>>);

impl Drop for ProjectOpenPermit {
    fn drop(&mut self) {
        let next = self.0.borrow_mut().waiters.pop_front();
        if let Some(next) = next {
            let _ = next.send(());
        } else {
            self.0.borrow_mut().held = false;
        }
    }
}

async fn acquire_project_open_gate(gate: Rc<RefCell<ProjectOpenGate>>) -> ProjectOpenPermit {
    let receiver = {
        let mut state = gate.borrow_mut();
        if state.held {
            let (sender, receiver) = oneshot::channel();
            state.waiters.push_back(sender);
            Some(receiver)
        } else {
            state.held = true;
            None
        }
    };
    if let Some(receiver) = receiver {
        let _ = receiver.await;
    }
    ProjectOpenPermit(gate)
}

fn project_transition_is_current(
    epoch: &Rc<Cell<u64>>,
    target: &Rc<RefCell<Option<String>>>,
    request_epoch: u64,
    project_id: &str,
) -> bool {
    epoch.get() == request_epoch && target.borrow().as_deref() == Some(project_id)
}

fn acp_value_text(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        value => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn upsert_acp_tool(items: &mut Vec<ChatItem>, payload: &serde_json::Value) {
    let Some(call_id) = payload
        .get("toolCallId")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let index = items
        .iter()
        .position(|item| matches!(item, ChatItem::AcpTool { call_id: id, .. } if id == call_id));
    if let Some(index) = index {
        if let ChatItem::AcpTool {
            title,
            kind,
            status,
            content,
            locations,
            ..
        } = &mut items[index]
        {
            if let Some(value) = payload.get("title").and_then(serde_json::Value::as_str) {
                *title = value.into();
            }
            if let Some(value) = payload.get("kind").and_then(serde_json::Value::as_str) {
                *kind = value.into();
            }
            if let Some(value) = payload.get("status").and_then(serde_json::Value::as_str) {
                *status = value.into();
            }
            if payload.get("content").is_some() {
                *content = acp_value_text(payload.get("content"));
            }
            if payload.get("locations").is_some() {
                *locations = acp_value_text(payload.get("locations"));
            }
        }
    } else {
        let row = ChatItem::AcpTool {
            call_id: call_id.into(),
            title: payload
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("ACP tool")
                .into(),
            kind: payload
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .into(),
            status: payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("pending")
                .into(),
            content: acp_value_text(payload.get("content")),
            locations: acp_value_text(payload.get("locations")),
        };
        let index = process_item_insert_index(items);
        items.insert(index, row);
    }
}

fn acp_plan_text(payload: &serde_json::Value) -> String {
    payload
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let status = entry
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("pending");
                    let mark = if status == "completed" { "x" } else { " " };
                    let content = entry
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    format!("- [{mark}] {content}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn acp_select_options(option: &serde_json::Value) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for row in option
        .get("options")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(value) = row.get("value").and_then(serde_json::Value::as_str) {
            result.push((
                value.into(),
                row.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(value)
                    .into(),
            ));
        } else if let Some(options) = row.get("options").and_then(serde_json::Value::as_array) {
            for choice in options {
                if let Some(value) = choice.get("value").and_then(serde_json::Value::as_str) {
                    result.push((
                        value.into(),
                        choice
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(value)
                            .into(),
                    ));
                }
            }
        }
    }
    result
}

fn remove_optimistic_send_rows(rows: &mut Vec<ChatItem>, display_message: &str) {
    let Some(index) = rows.iter().rposition(|item| {
        matches!(item, ChatItem::User(value) if value == display_message)
            || matches!(item, ChatItem::QueuedUser { text, .. } if text == display_message)
    }) else {
        return;
    };
    if matches!(rows.get(index), Some(ChatItem::QueuedUser { .. })) {
        rows.remove(index);
        return;
    }
    if matches!(rows.get(index + 1), Some(ChatItem::Assistant { text, .. }) if text.is_empty()) {
        rows.drain(index..=index + 1);
    }
}

fn mark_optimistic_send_failed(rows: &mut Vec<ChatItem>, display_message: &str, error: &str) {
    let Some(index) = rows.iter().rposition(|item| {
        matches!(item, ChatItem::User(value) if value == display_message)
            || matches!(item, ChatItem::QueuedUser { text, .. } if text == display_message)
    }) else {
        return;
    };
    if matches!(rows.get(index), Some(ChatItem::QueuedUser { .. })) {
        rows[index] = ChatItem::User(display_message.to_string());
        rows.insert(
            index + 1,
            ChatItem::Assistant {
                text: format!("Error: {error}"),
                model: None,
                resources: Vec::new(),
            },
        );
        return;
    }
    if let Some(ChatItem::Assistant { text, .. }) = rows.get_mut(index + 1) {
        if text.is_empty() {
            *text = format!("Error: {error}");
        }
    }
}

fn split_turn_started_error(error: &str) -> (bool, &str) {
    error
        .strip_prefix("[turn-started] ")
        .map_or((false, error), |message| (true, message))
}

mod app_support;
use app_support::*;

fn terminal_element_id(session_id: &str) -> String {
    format!("terminal-session-{session_id}")
}

fn terminal_tab_id(session_id: &str) -> String {
    format!("terminal-tab-{session_id}")
}

#[component]
fn TerminalHost(session_id: String, active_terminal_id: RwSignal<Option<String>>) -> impl IntoView {
    let element_id = terminal_element_id(&session_id);
    let labelled_by = terminal_tab_id(&session_id);
    let host_ref = create_node_ref::<html::Div>();
    let mount_element_id = element_id.clone();
    let mount_session_id = session_id.clone();
    let active_session_id = session_id.clone();
    let class_session_id = session_id.clone();

    create_effect(move |_| {
        if host_ref.get().is_none() {
            return;
        }
        mount_terminal(&mount_element_id, &mount_session_id);
        set_terminal_active(
            &mount_element_id,
            active_terminal_id.get().as_deref() == Some(active_session_id.as_str()),
        );
    });

    let cleanup_element_id = element_id.clone();
    on_cleanup(move || unmount_terminal(&cleanup_element_id));

    view! {
        <div
            id=element_id
            node_ref=host_ref
            class="terminal-dock-frame"
            class:active=move || active_terminal_id.get().as_deref() == Some(class_session_id.as_str())
            data-terminal-session=session_id
            role="tabpanel"
            aria-labelledby=labelled_by
        ></div>
    }
}

#[component]
fn App() -> impl IntoView {
    let locale = create_rw_signal(Locale::detect_browser());
    provide_context(locale.read_only());
    let theme_mode = create_rw_signal(load_theme_mode());
    create_effect(move |_| apply_theme_mode(&theme_mode.get()));
    let light_palette = create_rw_signal(load_light_palette());
    let dark_palette = create_rw_signal(load_dark_palette());
    create_effect(move |_| apply_palette_modes(&light_palette.get(), &dark_palette.get()));
    let ui_font_size = create_rw_signal(load_ui_font_size());
    let code_font_size = create_rw_signal(load_code_font_size());
    create_effect(move |_| apply_font_sizes(ui_font_size.get(), code_font_size.get()));
    let selection_popup_enabled = create_rw_signal(load_selection_popup_enabled());
    create_effect(move |_| save_selection_popup_enabled(selection_popup_enabled.get()));
    let send_with_modifier = create_rw_signal(load_send_with_modifier());
    create_effect(move |_| save_send_with_modifier(send_with_modifier.get()));

    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    // Disclosure choices belong to the session/step identity, not to a render
    // instance. Content fingerprints intentionally remount changed rows while
    // streaming, so keeping this state here preserves explicit user choices.
    let step_disclosure_state = create_rw_signal::<HashMap<String, bool>>(HashMap::new());
    let empty_title_idx = create_rw_signal(
        (js_sys::Math::random() * EMPTY_TITLE_COUNT as f64).floor() as usize % EMPTY_TITLE_COUNT,
    );
    let empty_subtitle_idx = create_rw_signal(
        (js_sys::Math::random() * EMPTY_SUBTITLE_COUNT as f64).floor() as usize
            % EMPTY_SUBTITLE_COUNT,
    );
    create_effect(move |_| {
        if items.get().is_empty() {
            empty_title_idx.set(
                (js_sys::Math::random() * EMPTY_TITLE_COUNT as f64).floor() as usize
                    % EMPTY_TITLE_COUNT,
            );
            empty_subtitle_idx.set(
                (js_sys::Math::random() * EMPTY_SUBTITLE_COUNT as f64).floor() as usize
                    % EMPTY_SUBTITLE_COUNT,
            );
        }
    });
    let input = create_rw_signal(String::new());
    let attachments = create_rw_signal::<Vec<ComposerAttachment>>(vec![]);
    let uploading = create_rw_signal(false);
    let drag_over = create_rw_signal(false);
    // Per-session streaming state. `running` is the set of session ids with an
    // in-flight turn; `transcripts` caches the live transcript of background
    // (non-active) sessions so switching to them shows streaming progress.
    let running = create_rw_signal::<HashSet<String>>(HashSet::new());
    let approval_pending = create_rw_signal::<HashSet<String>>(HashSet::new());
    let pet_activity = create_rw_signal((String::from("idle"), 0_u64));
    let pending_turns = create_rw_signal::<HashMap<String, usize>>(HashMap::new());
    let transcripts = create_rw_signal::<HashMap<String, Vec<ChatItem>>>(HashMap::new());
    let transcript_pages = create_rw_signal::<HashMap<String, TranscriptPageState>>(HashMap::new());
    let conversation_outlines =
        create_rw_signal::<HashMap<String, Vec<SessionOutlineItem>>>(HashMap::new());
    let conversation_outline_open = create_rw_signal(false);
    let conversation_outline_selected = create_rw_signal::<Option<usize>>(None);
    let busy = create_rw_signal(false);
    // Interrupting a running turn (especially a language runtime) is not instant, so
    // keep track of the session whose Stop click is waiting for the backend.
    let stopping_session = create_rw_signal::<Option<String>>(None);
    let show_settings = create_rw_signal(false);
    let settings_section = create_rw_signal(String::from("general"));
    let skills_list = create_rw_signal(Vec::<SkillRow>::new());
    let skills_search = create_rw_signal(String::new());
    let skills_msg = create_rw_signal(None::<(bool, String)>);
    let plugins_list = create_rw_signal(Vec::<PluginRow>::new());
    let plugins_msg = create_rw_signal(None::<(bool, String)>);
    let model_form = create_rw_signal(None::<ModelForm>);
    let model_form_key = create_rw_signal(String::new());
    let model_form_msg = create_rw_signal(None::<(bool, String)>);
    let specialists = create_rw_signal::<Vec<Specialist>>(vec![]);
    let specialist_form = create_rw_signal::<Option<Specialist>>(None);
    let specialist_form_open = create_memo(move |_| specialist_form.get().is_some());
    let memory_view = create_rw_signal(None::<MemoryView>);
    let memory_selected = create_rw_signal(None::<String>);
    let memory_editor = create_rw_signal(String::new());
    let memory_msg = create_rw_signal(None::<(bool, String)>);
    let conns_view = create_rw_signal(None::<ConnView>);
    let connectors = create_rw_signal(None::<ConnectorsView>);
    let approval_grants = create_rw_signal(Vec::<ApprovalGrantRow>::new());
    let custom_conn_tools = create_rw_signal(HashMap::<String, Vec<ConnectorTool>>::new());
    let custom_conn_tools_loading = create_rw_signal(HashSet::<String>::new());
    let custom_conn_tool_errors = create_rw_signal(HashMap::<String, String>::new());
    let open_conn_key = create_rw_signal(None::<String>);
    let channels_open = create_rw_signal(None::<String>);
    let conn_form = create_rw_signal(None::<ConnForm>);
    let conn_test_msg = create_rw_signal(None::<(bool, String)>);
    // Service credentials (Settings → Credentials, #115). `cred_status` maps a
    // credential id -> whether a value is stored; `cred_inputs` holds the
    // in-progress edit per id; one shared status message.
    let cred_status = create_rw_signal(std::collections::HashMap::<String, bool>::new());
    let cred_inputs = create_rw_signal(std::collections::HashMap::<String, String>::new());
    let custom_credentials = create_rw_signal(Vec::<CustomCredentialStatus>::new());
    let cred_msg = create_rw_signal(None::<(bool, String)>);
    // Gate the settings sub-form panes on whether a form is open — NOT on its
    // contents. A closure that reads the whole form signal re-runs on every
    // keystroke (each `on:input` calls `.update`), rebuilding the inputs and
    // dropping focus after each character (#62). A memo only notifies when the
    // Some/None state flips, so the inputs stay mounted while editing.
    let model_form_open = create_memo(move |_| model_form.get().is_some());
    let conn_form_open = create_memo(move |_| conn_form.get().is_some());
    // Same reason, one level deeper: the connection form swaps stdio/http fields
    // on `kind`; track just `kind` so editing command/url doesn't rebuild them.
    let conn_form_kind = create_memo(move |_| conn_form.get().map(|f| f.kind).unwrap_or_default());
    let settings = create_rw_signal(Settings::default());
    let pet_status = create_rw_signal(PetStatus::default());
    // Configured model profiles + the composer's bottom-right picker state.
    let models = create_rw_signal::<Vec<ModelProfile>>(vec![]);
    let active_session = create_rw_signal::<Option<String>>(None);
    let conversation_outline = create_memo(move |_| {
        let Some(id) = active_session.get() else {
            return Vec::new();
        };
        let persisted = conversation_outlines
            .with(|outlines| outlines.get(&id).cloned())
            .unwrap_or_default();
        let user_offset = transcript_pages
            .with(|pages| pages.get(&id).copied())
            .map_or(0, |page| page.user_offset);
        items.with(|rows| merge_conversation_outline(&persisted, rows, user_offset))
    });
    create_effect(move |_| {
        let _ = active_session.get();
        conversation_outline_open.set(false);
        conversation_outline_selected.set(None);
    });
    let session_model_ids = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    let acp_agents = create_rw_signal::<Vec<AcpAgentProfile>>(vec![]);
    let active_acp_agent_id = create_rw_signal::<Option<String>>(None);
    // An ACP Agent can only bind an empty frame. When the picker creates that
    // frame on demand, retain the intended selection while the async binding
    // lookup still (correctly) reports None before the first prompt.
    let provisional_acp_selection = create_rw_signal::<Option<(String, String)>>(None);
    let show_acp_agents = create_rw_signal(false);
    let acp_form = create_rw_signal::<Option<AcpAgentProfile>>(None);
    let acp_form_msg = create_rw_signal::<Option<(bool, String)>>(None);
    let acp_infos = create_rw_signal::<HashMap<String, AcpAgentInfo>>(HashMap::new());
    let acp_session_configs =
        create_rw_signal::<HashMap<String, Vec<serde_json::Value>>>(HashMap::new());
    let acp_session_modes = create_rw_signal::<HashMap<String, serde_json::Value>>(HashMap::new());
    let acp_config_menu_open = create_rw_signal::<Option<String>>(None);
    let show_projects = create_rw_signal(true); // app lands on the Projects screen
    let show_library = create_rw_signal(false);
    let library_items = create_rw_signal::<Vec<LibraryItem>>(vec![]);
    let refresh_library_items = Callback::new(move |_: ()| refresh_library(library_items));
    refresh_library_items.call(());
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    let demo_mode = create_rw_signal(false); // true = the synthetic "Example project" is open
    let project_open_error = create_rw_signal(None::<String>);
    let app_shell_entering = create_rw_signal(false);
    let project_transition_epoch = Rc::new(Cell::new(0u64));
    let project_transition_target = Rc::new(RefCell::new(None::<String>));
    let project_open_gate = Rc::new(RefCell::new(ProjectOpenGate::default()));
    let model_menu_open = create_rw_signal(false);
    let model_switch_confirm = create_rw_signal::<Option<(String, String)>>(None);
    let status = create_rw_signal(String::new());
    let switch_http_model = Callback::new(move |(id, dont_ask_again): (String, bool)| {
        provisional_acp_selection.set(None);
        active_acp_agent_id.set(None);
        let session_id = active_session.get_untracked();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "id": id.clone(),
                "sessionId": session_id.clone(),
            }))
            .unwrap();
            match invoke_checked("set_active_model", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                    if let Some(session_id) = session_id {
                        session_model_ids.update(|models| {
                            models.insert(session_id, id);
                        });
                    }
                    if dont_ask_again {
                        disable_model_switch_warning();
                    }
                }
                Err(err) => {
                    web_sys::console::warn_1(&format!("set_active_model failed: {:?}", err).into());
                }
            }
        });
    });
    let send_mode_menu_open = create_rw_signal(false);
    // Queue (#433): monotonic key for optimistic queued bubbles, shared with the
    // backend queue item so edit/cancel/cut-in target the same row.
    let queue_seq = create_rw_signal(0u64);
    let side_chat_input = create_rw_signal(String::new());
    let side_chat_items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    let side_chat_busy = create_rw_signal(false);
    let side_chat_model_menu_open = create_rw_signal(false);
    // Side chat routes through this ACP Agent when set; None = the active model.
    let side_chat_acp_agent = create_rw_signal::<Option<String>>(None);
    let settings_busy = create_rw_signal(false);
    // Owned here so the window-level Escape stack can close the confirm before
    // it falls through to closing the whole settings page.
    let delete_confirm = create_rw_signal(None::<DeleteConfirm>);
    let plugin_install_open = create_rw_signal(false);
    let settings_message = create_rw_signal::<Option<(bool, String)>>(None);
    let update_check_busy = create_rw_signal(false);
    let update_check_modal = create_rw_signal::<Option<UpdateCheckModal>>(None);
    // Newer release found by the silent auto-check → sidebar prompt card.
    let update_banner = create_rw_signal::<Option<AvailableUpdate>>(None);
    // "不再提醒更新" opt-out; loaded on startup, mirrored by the settings toggle.
    let update_check_enabled = create_rw_signal(true);
    // Set when a send fails because no API key is configured, so the status bar
    // can offer a one-click jump to Settings instead of a dead-end message.
    let needs_api_key = create_rw_signal(false);
    let refresh_models = move || {
        spawn_local(async move {
            let v = invoke("list_models", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                models.set(list);
            }
            let v = invoke("list_acp_agents", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(v) {
                acp_agents.set(list);
            }
        })
    };
    // Tauri's native drag/drop event contains absolute paths (including
    // directories). Keep those paths as references; unlike the browser File
    // picker they must not be copied through `upload_file` first.
    let native_drop_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let inside = native_drop_in_composer(payload.clone());
        let value =
            serde_wasm_bindgen::from_value::<serde_json::Value>(payload).unwrap_or_default();
        let kind = value
            .get("kind")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(kind.as_str(), "enter" | "over" | "hover" | "hovered") {
            drag_over.set(inside);
            return;
        }
        if matches!(kind.as_str(), "leave" | "cancel" | "cancelled") {
            drag_over.set(false);
            return;
        }
        if !matches!(kind.as_str(), "drop" | "dropped") {
            return;
        }
        drag_over.set(false);
        if !inside {
            return;
        }
        let paths = value
            .get("paths")
            .and_then(|item| item.as_array())
            .cloned()
            .unwrap_or_default();
        for path in paths
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
        {
            let _ = attach_ready_path(attachments, path);
        }
        if active_acp_agent_id.get_untracked().is_none() {
            status.set(t(locale.get_untracked(), "composer.native_path_api_hint").into());
        }
    }) as Box<dyn FnMut(JsValue)>);
    let native_drop_js = native_drop_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    std::mem::forget(native_drop_cb);
    spawn_local(async move {
        let _ = listen_native_file_drop(&native_drop_js).await;
    });
    let refresh_specialists = move || {
        spawn_local(async move {
            let v = invoke("list_specialists", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                specialists.set(list);
            }
        })
    };
    // Per-session specialist (persona) picker, gated to before the first message.
    let session_specialist = create_rw_signal::<Option<Specialist>>(None);
    let demos = create_rw_signal::<Vec<DemoInfo>>(vec![]);
    let command_palette_open = create_rw_signal(false);
    let action_palette_open = create_rw_signal(false);
    // Top-nav project switcher dropdown + Project Settings modal.
    let show_proj_menu = create_rw_signal(false);
    let proj_list = create_rw_signal::<Vec<ProjectSummary>>(vec![]);
    let show_proj_settings = create_rw_signal(false);
    let proj_settings = create_rw_signal(ProjectSettings::default());
    let proj_settings_busy = create_rw_signal(false);

    // Session history (left sidebar).
    let sessions = create_rw_signal::<Vec<SessionInfo>>(vec![]);
    let session_history_cursor = create_rw_signal::<Option<SessionCursor>>(None);
    let session_history_loading = create_rw_signal(false);
    let refresh_session_history =
        move || refresh_sessions(sessions, pending_turns, running, session_history_cursor);
    let folders = create_rw_signal::<Vec<FolderInfo>>(vec![]);
    let collapsed_folders = create_rw_signal::<HashSet<String>>(HashSet::new());
    let drag_session = create_rw_signal::<Option<String>>(None);
    let drop_target = create_rw_signal::<Option<String>>(None);
    let session_execution_contexts = create_rw_signal::<HashSet<String>>(HashSet::new());
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
            let Ok(value) = invoke_checked("get_session_model", args).await else {
                return;
            };
            let Some(model_id) = value.as_string() else {
                return;
            };
            if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                session_model_ids.update(|models| {
                    models.insert(session_id, model_id);
                });
            }
        });
    });
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            session_execution_contexts.set(HashSet::new());
            return;
        };
        session_execution_contexts.set(HashSet::new());
        refresh_session_execution_contexts(session_execution_contexts, active_session, session_id);
    });
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            active_acp_agent_id.set(None);
            provisional_acp_selection.set(None);
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "frameId": session_id.clone() })).unwrap();
            let Ok(value) = invoke_checked("get_acp_session_agent", args).await else {
                return;
            };
            let Ok(agent_id) = serde_wasm_bindgen::from_value::<Option<String>>(value) else {
                return;
            };
            if active_session.get_untracked().as_deref() != Some(session_id.as_str()) {
                return;
            }
            let next = acp_agent_selection_after_fetch(
                agent_id,
                &session_id,
                &pending_turns.get_untracked(),
                &running.get_untracked(),
                provisional_acp_selection.get_untracked().as_ref(),
            );
            let Some(mut next) = next else {
                return;
            };
            // A fetch started before the first ACP bind can still return None after
            // send_message finishes. Confirm before clearing a live selection.
            if next.is_none() && active_acp_agent_id.get_untracked().is_some() {
                let args = to_value(&serde_json::json!({ "frameId": session_id.clone() })).unwrap();
                let Ok(value) = invoke_checked("get_acp_session_agent", args).await else {
                    return;
                };
                let Ok(confirmed) = serde_wasm_bindgen::from_value::<Option<String>>(value) else {
                    return;
                };
                if active_session.get_untracked().as_deref() != Some(session_id.as_str()) {
                    return;
                }
                next = confirmed;
            }
            active_acp_agent_id.set(next);
        });
    });
    refresh_session_history();
    refresh_folders(folders);

    // `busy` is "the active session is currently streaming" — derived from the
    // per-session `running` set so it stays correct when the user switches
    // conversations or a background turn finishes.
    create_effect(move |_| {
        let r = running.get();
        let b = active_session
            .get()
            .map(|id| r.contains(&id))
            .unwrap_or(false);
        busy.set(b);
    });

    // Refresh the session's specialist whenever the active session changes
    // (including on load and on "no session").
    create_effect(move |_| {
        let Some(sid) = active_session.get() else {
            session_specialist.set(None);
            return;
        };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
            let v = invoke("get_session_specialist", arg).await;
            if active_session.get_untracked().as_deref() == Some(sid.as_str()) {
                session_specialist.set(
                    serde_wasm_bindgen::from_value::<Option<Specialist>>(v)
                        .ok()
                        .flatten(),
                );
            }
        });
    });
    let pick_specialist = move |id: String| {
        let Some(sid) = active_session.get() else {
            return;
        };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid, "id": id })).unwrap();
            if invoke_checked("set_session_specialist", arg).await.is_ok() {
                let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
                let v = invoke("get_session_specialist", arg).await;
                if active_session.get_untracked().as_deref() == Some(sid.as_str()) {
                    session_specialist.set(
                        serde_wasm_bindgen::from_value::<Option<Specialist>>(v)
                            .ok()
                            .flatten(),
                    );
                }
            }
        });
    };

    // Three-pane layout state (mirrors web-dist: sidebar / conversation / right pane).
    let show_sidebar = create_rw_signal(true);
    let sidebar_w = create_rw_signal(load_sidebar_w());
    let sidebar_dragging = create_rw_signal(false);
    let sidebar_drag_start_x = create_rw_signal(0.0_f64);
    let sidebar_drag_start_w = create_rw_signal(0.0_f64);
    let show_right = create_rw_signal(false);
    let right_w = create_rw_signal(400.0_f64);
    let dragging = create_rw_signal(false);
    let drag_start_x = create_rw_signal(0.0_f64);
    let drag_start_w = create_rw_signal(0.0_f64);
    let composer_h = create_rw_signal(load_composer_h());
    let composer_h_custom = create_rw_signal(composer_h_custom());
    let composer_dragging = create_rw_signal(false);
    let composer_drag_start_y = create_rw_signal(0.0_f64);
    let composer_drag_start_h = create_rw_signal(0.0_f64);
    let terminal_sessions = create_rw_signal::<Vec<TerminalSessionSummary>>(vec![]);
    let active_terminal_id = create_rw_signal(None::<String>);
    let terminal_panel_open = create_rw_signal(false);
    let terminal_add_menu_open = create_rw_signal(false);
    let terminal_h = create_rw_signal(320.0_f64);
    let terminal_dragging = create_rw_signal(false);
    let terminal_drag_start_y = create_rw_signal(0.0_f64);
    let terminal_drag_start_h = create_rw_signal(0.0_f64);

    // Artifacts and notebook cells are projections of the active transcript.
    let proto_cache = Rc::new(RefCell::new(ProtoCache::new()));
    let artifacts_all = create_memo(move |_| {
        items.with(|list| collect_artifacts(list, locale.get(), &mut proto_cache.borrow_mut()))
    });
    // File-backed artifacts are scraped from chat text, so a file that was
    // renamed or overwritten still lingers and 404s on click (#41). Ask the
    // backend which referenced files are gone and drop them from the list.
    let missing_paths = create_rw_signal(std::collections::HashSet::<String>::new());
    create_effect(move |_| {
        let paths: Vec<String> = artifacts_all
            .get()
            .iter()
            .filter_map(|a| match &a.data {
                PreviewData::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        if paths.is_empty() {
            missing_paths.set(std::collections::HashSet::new());
            return;
        }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "paths": paths })).unwrap();
            let v = invoke("missing_files", arg).await;
            if let Ok(m) = serde_wasm_bindgen::from_value::<Vec<String>>(v) {
                missing_paths.set(m.into_iter().collect());
            }
        });
    });
    let artifacts = create_memo(move |_| {
        let miss = missing_paths.get();
        let root = project_info
            .get()
            .map(|project| project.root)
            .unwrap_or_default();
        current_artifacts(&artifacts_all.get(), &root, &miss)
    });
    let notebook_cache = Rc::new(RefCell::new(NotebookCache::new()));
    let notebook_cells = create_memo(move |_| {
        items.with(|list| collect_notebook_cells(list, &mut notebook_cache.borrow_mut()))
    });
    let sel_artifact = create_rw_signal(0usize);
    let show_art_preview = create_rw_signal(false);
    let modal_artifact = create_rw_signal(None::<ModalArtifact>); // (path, name, kind)
    let artifact_menu = create_rw_signal(None::<(usize, i32, i32)>); // (open tile idx, cursor x, y) — fixed-positioned so the `.rp-tiles` overflow doesn't clip it
    let collapsed_art_groups = create_rw_signal::<HashSet<String>>(HashSet::new());
    let rp_grid = create_rw_signal(false); // false = detailed/list, true = tiled/grid; shared by Artifacts + Files
    let right_tab = create_rw_signal(RightTab::Artifacts);
    let open_right_tabs = create_rw_signal(DEFAULT_RIGHT_TABS.to_vec());
    let right_tab_add_menu_open = create_rw_signal(false);
    let rp_tab_drag = create_rw_signal(None::<RightTab>);
    let rp_tab_drop = create_rw_signal(None::<RightTab>);
    create_effect(move |_| {
        side_chat_items.with(|items| items.len());
        if !show_right.get() || right_tab.get() != RightTab::SideChat {
            return;
        }
        request_animation_frame(|| {
            let Some(scroller) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(SIDE_CHAT_SCROLLER_ID))
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            else {
                return;
            };
            scroller.set_scroll_top(scroller.scroll_height());
        });
    });
    create_effect(move |_| {
        if show_right.get() {
            let _ = right_tab.get();
            let _ = open_right_tabs.get();
            scroll_active_right_tab_into_view();
        }
    });
    let agent_panel = AgentPanelState::new(active_session);
    refresh_agent_resources(agent_panel, specialists);
    let file_source = create_rw_signal("local".to_string());
    let file_query = create_rw_signal(String::new());
    let file_cwd = create_rw_signal(".".to_string());
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let file_search_hits = create_rw_signal::<Vec<FileSearchHit>>(vec![]);
    let remote_file_cwd = create_rw_signal("~".to_string());
    let remote_file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let remote_file_loading = create_rw_signal(false);
    let remote_file_error = create_rw_signal::<Option<String>>(None);
    let center_files = create_rw_signal::<Vec<CenterFileTab>>(vec![]);
    let center_file = create_rw_signal::<Option<String>>(None);
    // Live MCP Apps use the same center-tab surface as files, but their HTML,
    // tool input, and result stay in a separate instance map so in-memory tab
    // snapshots do not repeatedly clone multi-megabyte payloads. The backend
    // persists the presentation event once so a reopened session can restore it.
    let mcp_apps = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    // Successful edit/write tool calls bump the matching tab's revision. The
    // preview subtree is keyed by this value and re-reads the saved file.
    let center_file_revisions = create_rw_signal::<HashMap<String, u64>>(HashMap::new());
    let center_file_open = create_memo(move |_| center_file.get().is_some());
    // Split view: keep the main conversation beside the open document instead of
    // hiding it. Same session, same history — only the layout moves.
    let center_split = create_rw_signal(false);
    let center_split_on = create_memo(move |_| center_split.get() && center_file_open.get());
    // Runtime binding for R/Python previews: file path -> execution context id.
    // The language comes from the extension, so the context is the whole binding.
    // In-memory on purpose — a runtime dies with the app, so a binding that
    // outlived one would point at a process that no longer exists.
    let center_runtime_binding = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    let center_console = create_rw_signal::<RuntimeConsoles>(RuntimeConsoles::new());
    let center_run_busy = create_rw_signal::<Option<String>>(None);
    let center_runtime_panel = create_rw_signal(false);
    // Runtime inspection belongs to the active source tab, so a newly selected
    // file starts with its full preview until the user asks for the panel.
    create_effect(move |_| {
        let _ = center_file.get();
        center_runtime_panel.set(false);
    });
    let center_tabs_by_session =
        create_rw_signal::<HashMap<String, (Vec<CenterFileTab>, Option<String>)>>(HashMap::new());
    let previous_center_session = Rc::new(RefCell::new(None::<String>));
    create_effect(move |_| {
        let current_session = active_session.get();
        let mut previous_session = previous_center_session.borrow_mut();
        if *previous_session == current_session {
            return;
        }

        if let Some(session_id) = previous_session.as_ref() {
            center_tabs_by_session.update(|states| {
                states.insert(
                    session_id.clone(),
                    (center_files.get_untracked(), center_file.get_untracked()),
                );
            });
        }

        let restored = current_session.as_ref().and_then(|session_id| {
            center_tabs_by_session.with_untracked(|states| states.get(session_id).cloned())
        });
        let (files, selected) = restored.unwrap_or_default();
        center_files.set(files);
        center_file.set(selected);
        *previous_session = current_session;
    });
    // Side chat is per-session, same as the center tabs above: stash the
    // outgoing session's Q&A and restore the incoming session's, so switching
    // sessions no longer leaves the previous session's side chat on screen.
    let side_chat_by_session = create_rw_signal::<HashMap<String, Vec<ChatItem>>>(HashMap::new());
    let previous_side_chat_session = Rc::new(RefCell::new(None::<String>));
    create_effect(move |_| {
        let current_session = active_session.get();
        let mut previous_session = previous_side_chat_session.borrow_mut();
        if *previous_session == current_session {
            return;
        }
        if let Some(session_id) = previous_session.as_ref() {
            side_chat_by_session.update(|states| {
                states.insert(session_id.clone(), side_chat_items.get_untracked());
            });
        }
        let restored = current_session.as_ref().and_then(|session_id| {
            side_chat_by_session.with_untracked(|states| states.get(session_id).cloned())
        });
        side_chat_items.set(restored.unwrap_or_default());
        side_chat_input.set(String::new());
        side_chat_model_menu_open.set(false);
        // ponytail: busy is a global flag, so we clear it on switch to drop a
        // stale spinner. Trade-off: returning to a session whose request is
        // still in flight won't re-show its spinner. Make busy per-session if
        // that ever matters.
        side_chat_busy.set(false);
        *previous_session = current_session;
    });
    // Dedicated project windows use the same guarded transition as every
    // interactive project-open path. The callback is built after `load_session`.
    let dedicated_project_id = url_project_param();
    let show_capabilities = create_rw_signal(false);
    let skill_filter_tag = create_rw_signal(String::new());
    let caps = create_rw_signal::<Option<Capabilities>>(None);
    let bootstrap = create_rw_signal::<Option<BootstrapStatus>>(None);
    let show_onboarding = create_rw_signal(false);
    let onboard_step = create_rw_signal(0usize);
    let onboard_provider = create_rw_signal("openai".to_string());
    let onboard_key = create_rw_signal(String::new());

    create_effect(move |_| {
        if file_source.get() != "local" {
            file_search_hits.set(vec![]);
            return;
        }
        let q = file_query.get();
        if q.trim().is_empty() {
            file_search_hits.set(vec![]);
            return;
        }
        refresh_file_search(file_query, file_search_hits);
    });

    let open_resource = Callback::new(move |(path, name, kind): ModalArtifact| {
        if opens_in_modal(&kind) {
            modal_artifact.set(Some((path, name, kind)));
            return;
        }
        let tab = CenterFileTab::new(path.clone(), name, kind);
        center_files.update(|files| {
            if !files.iter().any(|file| file.path == path) {
                files.push(tab.clone());
            }
        });
        center_file.set(Some(path));
        show_projects.set(false);
    });

    let on_artifact_select = Callback::new(move |idx: usize| {
        let arts = artifacts.get();
        if let Some(a) = arts.get(idx) {
            if let PreviewData::File { path, kind } = &a.data {
                open_resource.call((path.clone(), a.name.clone(), kind.clone()));
            } else {
                ensure_right_tab(RightTab::Artifacts, show_right, open_right_tabs, right_tab);
                sel_artifact.set(idx);
                show_art_preview.set(true);
            }
        }
    });

    let on_file_link = Callback::new(move |resource: ModalArtifact| {
        open_resource.call(resource);
    });

    // Inline @ artifact, # session, and / skill pickers all share one cursor
    // model and one chip list. Uploads remain separate because they have async
    // progress/error state; selected catalog items are already durable records.
    let composer_references = create_rw_signal::<Vec<ComposerReferenceChip>>(vec![]);
    // Quoted selections retain their source path. The persisted message still
    // carries ordinary text, but the agent now knows which workspace file a
    // "change this" request must edit.
    let composer_quotes = create_rw_signal::<Vec<ComposerQuote>>(vec![]);
    // Floating action popup over a text selection: (text, source file path, x, y).
    // The source path is Some only when the selection is inside a file preview —
    // it gates the "annotate" action and names the review sidecar.
    let selection_popup = create_rw_signal::<Option<(String, Option<String>, i32, i32)>>(None);
    let picker_mode = create_rw_signal(None::<ComposerPickerMode>);
    let picker_query = create_rw_signal(String::new());
    let picker_index = create_rw_signal(0usize);
    let picker_artifacts = create_rw_signal(Vec::<ArtifactInfo>::new());
    let picker_sessions = create_rw_signal(Vec::<SessionSearchInfo>::new());
    // Declared up here (not with the other context-view signals) so the
    // composer @ menu can offer servers and runtimes alongside artifacts.
    let execution_contexts = create_rw_signal::<Vec<ExecutionContext>>(vec![]);
    create_effect(move |_| {
        let Some(mode) = picker_mode.get() else {
            return;
        };
        let query = picker_query.get();
        match mode {
            ComposerPickerMode::Artifact => spawn_local(async move {
                let arg = to_value(
                    &serde_json::json!({ "query": query, "limit": 40, "allProjects": true }),
                )
                .unwrap();
                let v = invoke("search_artifacts", arg).await;
                if picker_mode.get_untracked() == Some(mode)
                    && picker_query.get_untracked() == query
                {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(v) {
                        picker_artifacts.set(rows);
                    }
                }
            }),
            ComposerPickerMode::Session => spawn_local(async move {
                let needs_project = project_info.get_untracked().is_none();
                let arg = to_value(&serde_json::json!({ "query": query, "limit": 40 })).unwrap();
                let v = invoke("search_sessions", arg).await;
                if picker_mode.get_untracked() == Some(mode)
                    && picker_query.get_untracked() == query
                {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(v) {
                        picker_sessions.set(rows);
                    }
                }
                if needs_project {
                    let value = invoke("get_project_info", JsValue::UNDEFINED).await;
                    if picker_mode.get_untracked() == Some(mode)
                        && picker_query.get_untracked() == query
                    {
                        if let Ok(project) = serde_wasm_bindgen::from_value::<ProjectInfo>(value) {
                            project_info.set(Some(project));
                        }
                    }
                }
            }),
            ComposerPickerMode::Skill if skills_list.get_untracked().is_empty() => {
                spawn_local(async move {
                    let v = invoke("list_skills", JsValue::UNDEFINED).await;
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                        skills_list.set(rows);
                    }
                })
            }
            ComposerPickerMode::Skill => {}
        }
    });
    let picker_items = create_memo(move |_| {
        let query = picker_query.get().to_lowercase();
        match picker_mode.get() {
            Some(ComposerPickerMode::Artifact) => {
                let current_session = active_session.get();
                let current_project = project_info.get().map(|p| p.id);
                let mut rows = picker_artifacts.get();
                rows.sort_by_key(|a| {
                    (
                        if a.session_id.as_deref() == current_session.as_deref() {
                            0
                        } else if a.project_id.as_deref() == current_project.as_deref() {
                            1
                        } else {
                            2
                        },
                        std::cmp::Reverse(a.ts),
                    )
                });
                let mut items: Vec<_> =
                    rows.into_iter().map(ComposerPickerItem::Artifact).collect();
                items.extend(mention_compute_entries(&query, &execution_contexts.get()));
                items
            }
            Some(ComposerPickerMode::Session) => {
                let current_project = project_info.get().map(|p| p.id);
                let mut rows: Vec<_> = picker_sessions
                    .get()
                    .into_iter()
                    .filter(|s| active_session.get().as_deref() != Some(s.id.as_str()))
                    .collect();
                rows.sort_by_key(|s| {
                    (
                        current_project.as_deref() != Some(s.project_id.as_str()),
                        std::cmp::Reverse(s.activity_at),
                    )
                });
                let mut items: Vec<_> = rows.into_iter().map(ComposerPickerItem::Session).collect();
                if let Some(project) = project_info.get() {
                    if query.is_empty()
                        || "project".contains(&query)
                        || project.name.to_lowercase().contains(&query)
                    {
                        items.insert(
                            0,
                            ComposerPickerItem::Project {
                                id: project.id,
                                name: project.name,
                            },
                        );
                    }
                }
                items
            }
            Some(ComposerPickerMode::Skill) => {
                let mut rows: Vec<_> = skills_list
                    .get()
                    .into_iter()
                    .filter(|s| {
                        s.enabled
                            && (s.name.to_lowercase().contains(&query)
                                || s.description.to_lowercase().contains(&query)
                                || s.tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
                    })
                    .collect();
                rows.sort_by_key(|s| (!s.builtin, s.name.clone()));
                rows.into_iter().map(ComposerPickerItem::Skill).collect()
            }
            None => vec![],
        }
    });
    let select_picker_item = Callback::new(move |i: usize| {
        let Some(item) = picker_items.get().get(i).cloned() else {
            return;
        };
        let reference = match item {
            ComposerPickerItem::Artifact(a) => ComposerReferenceChip::Artifact {
                id: a.id,
                name: a.name,
            },
            ComposerPickerItem::Session(s) => ComposerReferenceChip::Session {
                id: s.id,
                title: s.title,
                project_name: s.project_name,
            },
            ComposerPickerItem::Project { id, name } => ComposerReferenceChip::Project { id, name },
            ComposerPickerItem::Skill(s) => ComposerReferenceChip::Skill { name: s.name },
            ComposerPickerItem::Context { id, label } => {
                ComposerReferenceChip::Context { id, label }
            }
            ComposerPickerItem::Runtime {
                context_id,
                context_label,
                language,
            } => ComposerReferenceChip::Runtime {
                context_id,
                context_label,
                language,
            },
        };
        input.update(|s| {
            if let Some((at, _, _)) = active_composer_trigger(s) {
                s.truncate(at);
            }
        });
        composer_references.update(|items| {
            if !items.iter().any(|item| item.key() == reference.key()) {
                items.push(reference);
            }
        });
        picker_mode.set(None);
        focus_composer();
    });

    let refresh_pet = Callback::new(move |_: ()| {
        spawn_local(async move {
            let value = invoke("get_pet", JsValue::UNDEFINED).await;
            if let Ok(status) = serde_wasm_bindgen::from_value::<PetStatus>(value) {
                pet_status.set(status);
            }
        });
    });
    refresh_pet.call(());

    spawn_local(async move {
        let v = invoke("get_project_info", JsValue::UNDEFINED).await;
        if show_projects.get_untracked() {
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                project_info.set(Some(p));
            }
        }
        let v = invoke("get_settings", JsValue::UNDEFINED).await;
        if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
            let loc = Locale::from_code(&cfg.locale);
            locale.set(loc);
            set_document_lang(loc);
        }
        let v = invoke("get_onboarding_state", JsValue::UNDEFINED).await;
        if let Ok(s) = serde_wasm_bindgen::from_value::<OnboardingState>(v) {
            if s.show {
                show_onboarding.set(true);
            }
        }
        let b = invoke("get_bootstrap_status", JsValue::UNDEFINED).await;
        if let Ok(st) = serde_wasm_bindgen::from_value::<BootstrapStatus>(b) {
            bootstrap.set(Some(st));
        }
        refresh_models();
    });

    // Silent startup update check: respect the "不再提醒更新" opt-out, and only
    // surface the sidebar prompt when a newer release exists. Never pops a modal.
    spawn_local(async move {
        let enabled = invoke("get_update_check_enabled", JsValue::UNDEFINED)
            .await
            .as_bool()
            .unwrap_or(true);
        update_check_enabled.set(enabled);
        if !enabled {
            return;
        }
        if let Ok(update) = serde_wasm_bindgen::from_value::<UpdateCheck>(
            invoke("check_for_updates", JsValue::UNDEFINED).await,
        ) {
            if update.update_available {
                update_banner.set(Some(AvailableUpdate {
                    version: update.latest_version,
                    notes: update.notes,
                    release_url: update.release_url,
                }));
            }
        }
    });

    // The native shell publishes the result of its one-time Python setup after
    // the UI is already interactive. Keep the capabilities view in sync without
    // polling or delaying the first window.
    {
        let bootstrap_js = Closure::<dyn Fn(JsValue)>::new(move |event: JsValue| {
            if let Ok(payload) = js_sys::Reflect::get(&event, &JsValue::from_str("payload")) {
                if let Ok(status) = serde_wasm_bindgen::from_value::<BootstrapStatus>(payload) {
                    bootstrap.set(Some(status));
                }
            }
        });
        let bootstrap_fn = bootstrap_js
            .as_ref()
            .unchecked_ref::<js_sys::Function>()
            .clone();
        bootstrap_js.forget();
        spawn_local(async move {
            let _ = listen("bootstrap-status", &bootstrap_fn).await;
        });
    }

    create_effect(move |_| {
        attach_chat_autoscroll();
    });
    create_effect(move |_| {
        let _ = items.get();
        schedule_chat_follow();
    });

    // Wire the agent event stream once. Every event carries the session frame
    // id; route transcript mutations to `items` (active session) or the
    // `transcripts` cache (background session) so parallel conversations don't
    // interleave in the view.
    let items_cb = items;
    let active_cb = active_session;
    let transcripts_cb = transcripts;
    let running_cb = running;
    let pending_cb = pending_turns;
    let approval_cb = approval_pending;
    // Desktop notification for task status (#327). The backend drops it while
    // any app window is focused or when disabled in settings, so callers just
    // fire on every done/error/approval event.
    let notify_desktop = move |frame_id: &str, kind: &str, detail: &str| {
        let loc = locale.get_untracked();
        let title = t(loc, &format!("notify.{kind}"));
        let session = sessions
            .get_untracked()
            .iter()
            .find(|s| s.id == frame_id)
            .map(|s| s.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| t(loc, "sidebar.untitled"));
        let body = if detail.is_empty() {
            session
        } else {
            format!("{session} · {detail}")
        };
        let session_id = frame_id.to_string();
        spawn_local(async move {
            let arg = to_value(
                &serde_json::json!({ "title": title, "body": body, "sessionId": session_id }),
            )
            .unwrap();
            let _ = invoke("notify_user", arg).await;
        });
    };
    let pet_activity_cb = pet_activity;
    let status_cb = status;
    let locale_cb = locale;
    let models_cb = models;
    let session_models_cb = session_model_ids;
    let center_file_revisions_cb = center_file_revisions;
    let center_files_cb = center_files;
    let center_file_cb = center_file;
    let center_split_cb = center_split;
    let show_right_cb = show_right;
    let mcp_apps_cb = mcp_apps;
    let show_mcp_app = Callback::new(
        move |(frame_id, presentation_id, payload, replace): (
            String,
            String,
            serde_json::Value,
            bool,
        )| {
            let instance_id = mcp_app_instance_id(&frame_id, &presentation_id, &payload);
            if !replace && mcp_apps_cb.with_untracked(|apps| apps.contains_key(&instance_id)) {
                return;
            }
            let Ok(payload_json) = serde_json::to_string(&payload) else {
                return;
            };
            let tab = CenterFileTab::new(
                instance_id.clone(),
                mcp_app_title(&payload),
                "mcp_app".into(),
            );
            mcp_apps_cb.update(|apps| {
                apps.insert(instance_id.clone(), payload_json);
            });
            center_files_cb.update(|files| {
                if !files.iter().any(|file| file.path == instance_id) {
                    files.push(tab);
                }
            });
            center_file_cb.set(Some(instance_id));
            center_split_cb.set(true);
            show_right_cb.set(false);
        },
    );
    let project_info_cb = project_info;
    // Streaming deltas are buffered and flushed on a timer (~20 fps) instead of
    // being applied per token; see the "Streaming delta batching" block above.
    let delta_buf: DeltaBuf = Rc::new(RefCell::new(HashMap::new()));
    let flush_scheduled = Rc::new(Cell::new(false));
    let cb_buf = delta_buf.clone();
    let cb_scheduled = flush_scheduled.clone();
    let cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let ev: AgentEvent = match serde_wasm_bindgen::from_value(payload) {
            Ok(e) => e,
            Err(err) => {
                web_sys::console::log_1(&format!("agent event decode error: {err:?}").into());
                return;
            }
        };
        // Ordered, non-delta events (tool calls, results, done…) must observe
        // every delta buffered before them, so drain the buffer first.
        let flush_now = || {
            flush_delta_buf(
                &cb_buf,
                active_cb,
                items_cb,
                transcripts_cb,
                models_cb,
                session_models_cb,
            )
        };
        let queue = |fid: String, d: PendingDelta| {
            queue_delta(&cb_buf, fid, d);
            schedule_delta_flush(
                &cb_buf,
                &cb_scheduled,
                active_cb,
                items_cb,
                transcripts_cb,
                models_cb,
                session_models_cb,
            );
        };
        let set_pet_activity = |frame_id: &str, state: &str| {
            if active_cb.get_untracked().as_deref() == Some(frame_id) {
                pet_activity_cb.update(|activity| {
                    activity.0 = state.to_string();
                    activity.1 = activity.1.wrapping_add(1);
                });
            }
        };
        match ev {
            AgentEvent::User { frame_id, text } => {
                set_pet_activity(&frame_id, "running");
                flush_now();
                let model = session_model_label(
                    &models_cb.get_untracked(),
                    &session_models_cb.get_untracked(),
                    Some(&frame_id),
                );
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    start_user_turn(v, text, model.clone());
                })
            }
            AgentEvent::MessageBoundary { .. } => {}
            AgentEvent::Resources {
                frame_id,
                resources,
                ..
            } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    if let Some(ChatItem::Assistant {
                        resources: current, ..
                    }) = items
                        .iter_mut()
                        .rev()
                        .find(|item| matches!(item, ChatItem::Assistant { .. }))
                    {
                        *current = resources;
                    }
                });
            }
            AgentEvent::Text { frame_id, delta } => {
                set_pet_activity(&frame_id, "running");
                queue(frame_id, PendingDelta::Text(delta));
            }
            AgentEvent::Reasoning { frame_id, delta } => {
                set_pet_activity(&frame_id, "running");
                queue(frame_id, PendingDelta::Reasoning(delta));
            }
            AgentEvent::ToolCall {
                frame_id,
                name,
                preview,
            } => {
                set_pet_activity(&frame_id, "review");
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let idx = process_item_insert_index(v);
                    v.insert(
                        idx,
                        ChatItem::Tool {
                            name,
                            ok: None,
                            input: preview,
                            output: String::new(),
                            started_at_ms: Some(now_ms()),
                            duration_ms: None,
                        },
                    );
                })
            }
            AgentEvent::ToolResult {
                frame_id,
                name,
                ok,
                content,
                duration_ms: event_ms,
            } => {
                set_pet_activity(&frame_id, if ok { "running" } else { "failed" });
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let queue_start = process_item_insert_index(v);
                    let idx = v[..queue_start].iter().rposition(
                        |c| matches!(c, ChatItem::Tool { name: n, ok: None, .. } if n == &name),
                    );
                    if let Some(i) = idx {
                        if let ChatItem::Tool {
                            ok: o,
                            output,
                            started_at_ms,
                            duration_ms,
                            ..
                        } = &mut v[i]
                        {
                            *o = Some(ok);
                            *output = content.clone();
                            finalize_tool_duration(started_at_ms, duration_ms, event_ms);
                        }
                    } else {
                        let dur = if event_ms > 0 { Some(event_ms) } else { None };
                        v.insert(
                            queue_start,
                            ChatItem::Tool {
                                name: name.clone(),
                                ok: Some(ok),
                                input: String::new(),
                                output: content.clone(),
                                started_at_ms: None,
                                duration_ms: dur,
                            },
                        );
                    }
                    if name == "attempt_completion" && ok {
                        promote_assistant_text(v, &content);
                    }
                })
            }
            AgentEvent::ToolPresentation {
                frame_id,
                presentation_id,
                presentation_kind,
                payload,
            } => {
                if presentation_kind == "mcp_app"
                    && active_cb.get_untracked().as_deref() == Some(frame_id.as_str())
                {
                    show_mcp_app.call((frame_id, presentation_id, payload, true));
                }
            }
            AgentEvent::Usage {
                frame_id,
                input,
                output,
                reasoning,
                cached,
                ctx_tokens,
                max_context,
                ..
            } => {
                // One usage row per reply: each round's usage (one API call)
                // is folded into the turn's row, which floats to the tail so
                // it never splits the coalesced tool-steps panel.
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    upsert_turn_usage(v, input, output, reasoning, cached);
                });
                // Status bar reflects only the active session's usage.
                if active_cb.get().as_deref() == Some(&frame_id) {
                    let pct = if max_context > 0 {
                        ctx_tokens * 100 / max_context
                    } else {
                        0
                    };
                    let loc = locale_cb.get();
                    status_cb.set(tf(
                        loc,
                        "status.usage",
                        &[
                            ("in", &fmt_tokens(input)),
                            ("out", &fmt_tokens(output)),
                            ("pct", &pct.to_string()),
                        ],
                    ));
                }
            }
            AgentEvent::Compaction {
                frame_id,
                before,
                after,
                ..
            } => {
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(tf(
                        locale_cb.get(),
                        "status.compact",
                        &[
                            ("before", &before.to_string()),
                            ("after", &after.to_string()),
                        ],
                    ));
                }
            }
            AgentEvent::ContextWarning {
                frame_id,
                ctx_tokens,
                max_context,
            } => {
                if active_cb.get().as_deref() == Some(&frame_id) {
                    let pct = if max_context > 0 {
                        ctx_tokens * 100 / max_context
                    } else {
                        0
                    };
                    status_cb.set(tf(
                        locale_cb.get(),
                        "status.ctx_warning",
                        &[("pct", &pct.to_string())],
                    ));
                }
            }
            AgentEvent::Stdout { frame_id, chunk } => {
                set_pet_activity(&frame_id, "running");
                queue(frame_id, PendingDelta::Stdout(chunk));
            }
            AgentEvent::Done {
                frame_id,
                stop_reason: _,
            } => {
                flush_now();
                notify_desktop(&frame_id, "done", "");
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    strip_approval_pending(items);
                });
                approval_cb.update(|s| {
                    s.remove(&frame_id);
                });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                set_pet_activity(&frame_id, "jumping");
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                refresh_session_history();
            }
            AgentEvent::Error { frame_id, message } => {
                flush_now();
                notify_desktop(&frame_id, "error", &message);
                let model = session_model_label(
                    &models_cb.get_untracked(),
                    &session_models_cb.get_untracked(),
                    Some(&frame_id),
                );
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    strip_approval_pending(v);
                    v.push(ChatItem::Assistant {
                        text: format!("Error: {message}"),
                        model,
                        resources: Vec::new(),
                    });
                });
                approval_cb.update(|s| {
                    s.remove(&frame_id);
                });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                set_pet_activity(&frame_id, "failed");
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
            }
            AgentEvent::DelegationCompleted {
                frame_id,
                workflow_id,
                status: completion_status,
                result,
                auto_resume,
            } => {
                flush_now();
                let succeeded = completion_status == "succeeded";
                let loc = locale_cb.get();
                let label = if auto_resume {
                    t(loc, "agents.background.completed_resuming")
                } else {
                    t(loc, "agents.background.completed")
                };
                notify_desktop(&frame_id, if succeeded { "done" } else { "error" }, &label);
                let workflow_label = workflow_id.chars().take(8).collect::<String>();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let index = trailing_queue_start(items);
                    items.insert(
                        index,
                        ChatItem::Tool {
                            name: "delegate_tasks".into(),
                            ok: Some(succeeded),
                            input: format!("{label} · {workflow_label}"),
                            output: result,
                            started_at_ms: None,
                            duration_ms: None,
                        },
                    );
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(label);
                }
                refresh_session_history();
            }
            AgentEvent::ReviewStarted { frame_id } => {
                set_pet_activity(&frame_id, "review");
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let index = trailing_queue_start(v);
                    v.insert(
                        index,
                        ChatItem::ReviewTransition {
                            phase: ReviewTransitionPhase::Reviewing,
                            model: None,
                        },
                    );
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.reviewing"));
                }
            }
            AgentEvent::ReviewFailed { frame_id, message } => {
                set_pet_activity(&frame_id, "failed");
                flush_now();
                let loc = locale_cb.get();
                let text = tf(
                    loc,
                    "status.review_failed",
                    &[("msg", &localize_backend(loc, &message))],
                );
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    v.push(ChatItem::Assistant {
                        text: text.clone(),
                        model: None,
                        resources: Vec::new(),
                    });
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(text);
                }
            }
            AgentEvent::CorrectionStarted { frame_id, model } => {
                set_pet_activity(&frame_id, "running");
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let index = trailing_queue_start(v);
                    v.insert(
                        index,
                        ChatItem::ReviewTransition {
                            phase: ReviewTransitionPhase::Correcting,
                            model: (!model.is_empty()).then_some(model.clone()),
                        },
                    );
                    v.insert(
                        index + 1,
                        ChatItem::Assistant {
                            text: String::new(),
                            model: (!model.is_empty()).then_some(model),
                            resources: Vec::new(),
                        },
                    );
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.correcting"));
                }
            }
            AgentEvent::Review { frame_id, report } => {
                set_pet_activity(&frame_id, "review");
                flush_now();
                let passed = report.review_status == "passed"
                    || (report.review_status.is_empty()
                        && report.findings.is_empty()
                        && report.coverage_gaps.is_empty());
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    upsert_review(v, report);
                    if passed {
                        let index = trailing_queue_start(v);
                        v.insert(
                            index,
                            ChatItem::ReviewTransition {
                                phase: ReviewTransitionPhase::Passed,
                                model: None,
                            },
                        );
                    }
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.review_done"));
                }
            }
            AgentEvent::Diff { .. } => {}
            AgentEvent::FileChanged { path, .. } => {
                let root = project_info_cb.get_untracked().map(|project| project.root);
                center_file_revisions_cb.update(|revisions| {
                    for key in file_change_refresh_keys(&path, root.as_deref()) {
                        let revision = revisions.entry(key).or_default();
                        *revision = revision.wrapping_add(1);
                    }
                });
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    let agent_js = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(cb);
    // wasm-bindgen only runs an async extern's JS body when the returned
    // future is polled, so we must await `listen` (not fire-and-forget it).
    spawn_local(async move {
        let _ = listen("agent", &agent_js).await;
    });

    // Confirm handler: render an inline approval card in the session thread
    // (not a global modal — see README inline tool-approval card).
    let confirm_active = active_session;
    let confirm_items = items;
    let confirm_transcripts = transcripts;
    let confirm_pending = approval_pending;
    let confirm_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(v) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let fid = v
                .get("frame_id")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            if msg.is_empty() || fid.is_empty() {
                return;
            }
            let mut tool = v
                .get("tool")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let mut preview = v
                .get("preview")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if tool.is_empty() {
                if let Some(rest) = msg.strip_prefix("Run tool '") {
                    if let Some((t, _)) = rest.split_once("'?") {
                        tool = t.to_string();
                    }
                } else if msg.starts_with("Dangerous command detected") {
                    tool = "shell".into();
                }
            }
            notify_desktop(&fid, "attention", &tool);
            route_items(
                confirm_active,
                confirm_items,
                confirm_transcripts,
                &fid,
                |v| {
                    strip_approval_pending(v);
                    if preview.is_empty() {
                        preview = last_tool_input(v, &tool);
                    }
                    v.push(ChatItem::ApprovalPending {
                        tool,
                        preview,
                        message: msg,
                    });
                },
            );
            confirm_pending.update(|s| {
                s.insert(fid);
            });
            force_chat_bottom();
        }
    }) as Box<dyn FnMut(JsValue)>);
    let confirm_js = confirm_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    std::mem::forget(confirm_cb);
    spawn_local(async move {
        let _ = listen("confirm-request", &confirm_js).await;
    });
    let acp_permission_items = items;
    let acp_permission_active = active_session;
    let acp_permission_transcripts = transcripts;
    let acp_permission_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(request) = serde_wasm_bindgen::from_value::<AcpPermissionRequest>(payload) else {
            return;
        };
        let tool = request
            .tool_call
            .get("title")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                request
                    .tool_call
                    .get("name")
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("ACP tool request")
            .to_string();
        notify_desktop(&request.frame_id, "attention", &tool);
        approval_pending.update(|s| {
            s.insert(request.frame_id.clone());
        });
        route_items(
            acp_permission_active,
            acp_permission_items,
            acp_permission_transcripts,
            &request.frame_id,
            |items| {
                items.push(ChatItem::AcpPermission {
                    request_id: request.request_id,
                    tool,
                    options: request.options,
                });
            },
        );
    }) as Box<dyn FnMut(JsValue)>);
    let acp_permission_js: js_sys::Function = acp_permission_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    acp_permission_cb.forget();
    spawn_local(async move {
        let _ = listen("permission-request", &acp_permission_js).await;
    });

    let acp_update_buf = delta_buf.clone();
    let acp_update_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(update) = serde_wasm_bindgen::from_value::<AcpSessionUpdate>(payload) else {
            return;
        };
        // ACP tool updates arrive on a second event channel. Drain assistant
        // deltas first so commentary → reasoning → action keeps wire order.
        flush_delta_buf(
            &acp_update_buf,
            active_session,
            items,
            transcripts,
            models,
            session_model_ids,
        );
        match update.kind.as_str() {
            "ToolCall" | "ToolCallUpdate" => route_items(
                active_session,
                items,
                transcripts,
                &update.frame_id,
                |rows| {
                    upsert_acp_tool(rows, &update.payload);
                },
            ),
            "Plan" => {
                let text = acp_plan_text(&update.payload);
                route_items(
                    active_session,
                    items,
                    transcripts,
                    &update.frame_id,
                    |rows| {
                        let card = PlanCard { text };
                        if let Some(index) = rows
                            .iter()
                            .rposition(|row| matches!(row, ChatItem::Plan(_)))
                        {
                            rows[index] = ChatItem::Plan(card);
                        } else {
                            let index = trailing_queue_start(rows);
                            rows.insert(index, ChatItem::Plan(card));
                        }
                    },
                );
            }
            "ConfigOptions" => {
                if let Some(options) = update
                    .payload
                    .get("configOptions")
                    .and_then(serde_json::Value::as_array)
                {
                    acp_session_configs.update(|all| {
                        all.insert(update.frame_id, options.clone());
                    });
                }
            }
            "CurrentMode" => {
                // A CurrentModeUpdate only carries `currentModeId`; merge it into
                // the existing state so the `availableModes` captured from the
                // initial SessionModeState (and needed by the mode picker) survive.
                acp_session_modes.update(|all| {
                    let merged = merge_current_mode(all.get(&update.frame_id), update.payload);
                    all.insert(update.frame_id, merged);
                });
            }
            "Usage" => {
                if active_session.get_untracked().as_deref() == Some(update.frame_id.as_str()) {
                    let used = update
                        .payload
                        .get("used")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let size = update
                        .payload
                        .get("size")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    status.set(if size == 0 {
                        format!("ACP context: {used} tokens")
                    } else {
                        format!("ACP context: {used} / {size} tokens")
                    });
                }
            }
            "SessionInfo" => {
                if active_session.get_untracked().as_deref() == Some(update.frame_id.as_str()) {
                    if let Some(title) = update
                        .payload
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                    {
                        status.set(title.into());
                    }
                }
            }
            "AvailableCommands" => {
                if active_session.get_untracked().as_deref() == Some(update.frame_id.as_str()) {
                    status.set("ACP commands updated".into());
                }
            }
            _ => {}
        }
    }) as Box<dyn FnMut(JsValue)>);
    let acp_update_js: js_sys::Function = acp_update_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    acp_update_cb.forget();
    spawn_local(async move {
        let _ = listen("acp-session-update", &acp_update_js).await;
    });

    let acp_state_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(state) = serde_wasm_bindgen::from_value::<AcpSessionState>(payload) else {
            return;
        };
        if let Some(options) = state.config_options {
            acp_session_configs.update(|all| {
                all.insert(state.frame_id.clone(), options);
            });
        }
        if let Some(modes) = state.modes {
            acp_session_modes.update(|all| {
                all.insert(state.frame_id, modes);
            });
        }
    }) as Box<dyn FnMut(JsValue)>);
    let acp_state_js: js_sys::Function = acp_state_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    acp_state_cb.forget();
    spawn_local(async move {
        let _ = listen("acp-session-state", &acp_state_js).await;
    });

    let acp_resolved_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(resolved) = serde_wasm_bindgen::from_value::<AcpPermissionResolved>(payload) else {
            return;
        };
        approval_pending.update(|s| {
            s.remove(&resolved.frame_id);
        });
        route_items(
            active_session,
            items,
            transcripts,
            &resolved.frame_id,
            |rows| {
                rows.retain(|row| !matches!(row, ChatItem::AcpPermission { request_id, .. } if request_id == &resolved.request_id));
            },
        );
    }) as Box<dyn FnMut(JsValue)>);
    let acp_resolved_js: js_sys::Function = acp_resolved_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    acp_resolved_cb.forget();
    spawn_local(async move {
        let _ = listen("permission-resolved", &acp_resolved_js).await;
    });

    let stop = move |_| {
        if stopping_session.get().is_some() {
            return;
        }
        // Stop only the active session's turn; background conversations keep running.
        let sid = active_session.get();
        stopping_session.set(sid.clone());
        spawn_local(async move {
            let arg = to_value(&tauri_args::stop_agent(&sid)).unwrap();
            let _ = invoke("stop_agent", arg).await;
        });
    };

    let send = Callback::new(move |action: ComposerSendAction| {
        let message = input.get();
        let saved_attachments = attachments.get();
        let refs = composer_references.get();
        let quotes = composer_quotes.get();
        let paths = attachment_paths(&saved_attachments);
        let display_message = message_with_composer_context(&message, &paths, &refs, &quotes);
        let reference_args = refs
            .iter()
            .map(ComposerReferenceChip::arg)
            .collect::<Vec<_>>();
        // An @-referenced server turns itself on for the session backend-side;
        // re-read the enabled set afterwards so the sidebar toggles agree.
        let touches_contexts = reference_args.iter().any(|reference| {
            matches!(
                reference,
                ComposerReferenceArg::Context { .. } | ComposerReferenceArg::Runtime { .. }
            )
        });
        if message.trim().is_empty()
            && paths.is_empty()
            && reference_args.is_empty()
            && quotes.is_empty()
        {
            return;
        }
        if active_acp_agent_id.get().is_some() && action == ComposerSendAction::BranchNew {
            status.set("ACP protocol v1 does not support branching a bound session.".into());
            return;
        }
        let active = active_session.get();
        let branch = action == ComposerSendAction::BranchNew;
        let queued = !branch && active.as_ref().is_some_and(|id| running.get().contains(id));
        // Queue (#433): a plain send into a busy session parks behind the
        // running turn — editable/cancellable until the driver runs it — instead
        // of a dialog. Cut-in / interrupt-replace are explicit dropdown choices.
        if queued && action == ComposerSendAction::Normal {
            let Some(session) = active.clone() else { return };
            let qid = queue_seq.get() + 1;
            queue_seq.set(qid);
            input.set(String::new());
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            picker_mode.set(None);
            route_items(active_session, items, transcripts, &session, |rows| {
                rows.push(ChatItem::QueuedUser {
                    id: qid,
                    text: display_message.clone(),
                });
            });
            force_chat_bottom();
            let enqueue_msg = display_message.clone();
            spawn_local(async move {
                let args = to_value(&EnqueueTurnArgs {
                    session_id: session.clone(),
                    id: qid,
                    message: enqueue_msg.clone(),
                    attachments: paths,
                    references: reference_args,
                })
                .unwrap();
                if invoke_checked("enqueue_turn", args).await.is_err() {
                    route_items(active_session, items, transcripts, &session, |rows| {
                        remove_optimistic_send_rows(rows, &enqueue_msg);
                    });
                    status.set(t(locale.get(), "status.send_failed").into());
                }
            });
            return;
        }
        let agent_id = active_acp_agent_id.get();
        let turn_model = if let Some(id) = agent_id.as_ref() {
            acp_agents
                .get()
                .into_iter()
                .find(|agent| &agent.id == id)
                .map(|agent| agent.label)
                .or_else(|| Some("ACP Agent".into()))
        } else {
            session_model_label(&models.get(), &session_model_ids.get(), active.as_deref())
        };
        input.set(String::new());
        attachments.set(vec![]);
        composer_references.set(vec![]);
        composer_quotes.set(vec![]);
        picker_mode.set(None);
        spawn_local(async move {
            let id = if branch {
                let args = to_value(&tauri_args::branch_session(
                    &active,
                    Some(message.trim()),
                    None,
                ))
                .unwrap();
                match invoke("branch_session", args).await.as_string() {
                    Some(id) => id,
                    None => {
                        input.set(message);
                        attachments.set(saved_attachments);
                        composer_references.set(refs);
                        composer_quotes.set(quotes);
                        status.set(t(locale.get(), "status.send_failed").into());
                        return;
                    }
                }
            } else if let Some(id) = active {
                id
            } else {
                match invoke("new_session", JsValue::UNDEFINED).await.as_string() {
                    Some(id) => id,
                    None => {
                        input.set(message);
                        attachments.set(saved_attachments);
                        composer_references.set(refs);
                        composer_quotes.set(quotes);
                        status.set(t(locale.get(), "status.send_failed").into());
                        return;
                    }
                }
            };
            // Mark the turn pending before touching active_session so the
            // session→ACP lookup effect does not clear a just-selected agent
            // while send_message is still binding the session.
            begin_pending_turn(pending_turns, running, &id);
            if active_session.get_untracked().as_deref() != Some(id.as_str()) {
                active_session.set(Some(id.clone()));
            }
            transcript_pages.update(|pages| {
                pages.entry(id.clone()).or_default().window_user_start = usize::MAX;
            });
            route_items(active_session, items, transcripts, &id, |rows| {
                if queued {
                    // Cut-in (#433): a direct guide-append from the dropdown folds
                    // into the running turn immediately, so it carries no queue id
                    // (id 0 = transient, no edit/cancel controls).
                    rows.push(ChatItem::QueuedUser {
                        id: 0,
                        text: display_message.clone(),
                    });
                } else {
                    rows.push(ChatItem::User(display_message.clone()));
                    rows.push(ChatItem::Assistant {
                        text: String::new(),
                        model: turn_model.clone(),
                        resources: Vec::new(),
                    });
                }
            });
            force_chat_bottom();
            // Await the stop before send_message so the running turn is already
            // flagged for cancellation; send_message then blocks on the session's
            // workflow lock and starts as soon as the old turn aborts. Firing the
            // stop concurrently could cancel the new turn instead.
            if action == ComposerSendAction::InterruptReplace {
                let arg = to_value(&tauri_args::stop_agent(&Some(id.clone()))).unwrap();
                let _ = invoke("stop_agent", arg).await;
            }
            // Persist/emit the same display text the optimistic bubble uses
            // (including "Uploaded files: …"). Sending the bare composer body
            // makes AgentEvent::User mismatch the optimistic row and append a
            // duplicate; after a session switch only the persisted body remains.
            let args = to_value(&SendMessageArgs {
                session_id: Some(id.clone()),
                message: display_message.clone(),
                attachments: paths,
                references: reference_args,
                resume: false,
                acp_agent_id: agent_id.clone(),
                guide: (action == ComposerSendAction::GuideAppend).then_some(true),
                replace: (action == ComposerSendAction::InterruptReplace).then_some(true),
            })
            .unwrap();
            match invoke_checked("send_message", args).await {
                Ok(_) => {
                    if let Some(agent_id) = agent_id {
                        active_acp_agent_id.set(Some(agent_id));
                    }
                    if touches_contexts {
                        refresh_session_execution_contexts(
                            session_execution_contexts,
                            active_session,
                            id.clone(),
                        );
                    }
                    refresh_session_history();
                }
                Err(error) => {
                    let raw = js_error_text(error);
                    let (started, message_text) = split_turn_started_error(&raw);
                    route_items(active_session, items, transcripts, &id, |rows| {
                        if started {
                            mark_optimistic_send_failed(rows, &display_message, message_text);
                        } else {
                            remove_optimistic_send_rows(rows, &display_message);
                        }
                    });
                    if !started {
                        if input.get_untracked().is_empty() {
                            input.set(message);
                        }
                        if attachments.get_untracked().is_empty() {
                            attachments.set(saved_attachments);
                        }
                        if composer_references.get_untracked().is_empty() {
                            composer_references.set(refs);
                        }
                        if composer_quotes.get_untracked().is_empty() {
                            composer_quotes.set(quotes);
                        }
                    }
                    if raw.contains(NO_API_KEY_MARK) {
                        needs_api_key.set(true);
                    }
                    status.set(tf(
                        locale.get(),
                        "status.send_failed",
                        &[("msg", &localize_backend(locale.get(), message_text))],
                    ));
                }
            }
            finish_pending_turn(pending_turns, running, &id);
        });
    });
    let send_side_chat = move |question: String| {
        let question = question.trim().to_string();
        if question.is_empty() || side_chat_busy.get() {
            return;
        }
        ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab);
        side_chat_input.set(String::new());
        side_chat_items.update(|v| v.push(ChatItem::User(question.clone())));
        side_chat_busy.set(true);
        let sid = active_session.get();
        let acp_agent = side_chat_acp_agent.get();
        let model = match acp_agent.as_ref() {
            Some(id) => acp_agents
                .get()
                .into_iter()
                .find(|agent| &agent.id == id)
                .map(|agent| agent.label)
                .or_else(|| Some("ACP Agent".into())),
            None => active_model_label(&models.get()),
        };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "sessionId": sid.clone(),
                "question": question,
                "acpAgentId": acp_agent,
            }))
            .unwrap();
            let reply = match invoke_checked("side_chat", arg).await {
                Ok(v) => ChatItem::Assistant {
                    text: v.as_string().unwrap_or_default(),
                    model: model.clone(),
                    resources: Vec::new(),
                },
                Err(err) => ChatItem::Assistant {
                    text: format!(
                        "Error: {}",
                        localize_backend(locale.get(), &js_error_text(err))
                    ),
                    model: model.clone(),
                    resources: Vec::new(),
                },
            };
            // The user may have switched sessions while this was in flight.
            // Deliver the answer to the session it was asked about, not whatever
            // side chat is on screen now.
            if active_session.get_untracked() == sid {
                side_chat_items.update(|items| items.push(reply));
                side_chat_busy.set(false);
            } else if let Some(id) = sid {
                side_chat_by_session.update(|states| {
                    states.entry(id).or_default().push(reply);
                });
            }
        });
    };

    let on_send = move |ev: web_sys::KeyboardEvent| {
        // While an IME is composing (e.g. Chinese pinyin), Enter confirms the
        // candidate, so let the IME handle every key and never send/navigate
        // mid-composition (#108; keyCode-229 quirk in ime_composing).
        if ime_composing(&ev) {
            return;
        }
        if picker_mode.get().is_some() {
            match ev.key().as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    let n = picker_items.get().len().max(1);
                    let next = (picker_index.get() + 1) % n;
                    picker_index.set(next);
                    scroll_picker_item(".mention-item", next);
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    let n = picker_items.get().len().max(1);
                    let next = (picker_index.get() + n - 1) % n;
                    picker_index.set(next);
                    scroll_picker_item(".mention-item", next);
                }
                "Enter" | "Tab" => {
                    ev.prevent_default();
                    select_picker_item.call(picker_index.get());
                }
                "Escape" => {
                    ev.prevent_default();
                    picker_mode.set(None);
                }
                _ => {}
            }
            return;
        }
        if ev.key() == "Enter"
            && !ev.shift_key()
            && (!send_with_modifier.get_untracked() || ev.ctrl_key() || ev.meta_key())
        {
            ev.prevent_default();
            send.call(ComposerSendAction::Normal);
        }
    };

    let edit_message = move |ui_index: usize| {
        if busy.get() {
            return;
        }
        let list = items.get();
        let Some(user_idx) = user_message_index(&list, ui_index) else {
            return;
        };
        let Some(ChatItem::User(text)) = list.get(ui_index) else {
            return;
        };
        let draft = composer_text_from_user_message(text);
        let sid = active_session.get();
        let user_idx = user_idx
            + sid
                .as_deref()
                .and_then(|id| transcript_pages.with(|pages| pages.get(id).copied()))
                .map_or(0, |page| page.user_offset);
        if let Some(id) = sid.as_deref() {
            conversation_outlines.update(|outlines| {
                if let Some(outline) = outlines.get_mut(id) {
                    outline.retain(|entry| entry.user_index < user_idx);
                }
            });
        }
        items.set(list.into_iter().take(ui_index).collect());
        input.set(draft);
        focus_composer();
        spawn_local(async move {
            let arg = to_value(&tauri_args::rewind_session(&sid, user_idx)).unwrap();
            let _ = invoke("rewind_session", arg).await;
        });
    };
    let branch_message = {
        let locale = locale;
        let status = status;
        let active_session = active_session;
        let items = items;
        let input = input;
        let attachments = attachments;
        let composer_references = composer_references;
        let transcripts = transcripts;
        move |ui_index: usize| {
            let list = items.get();
            let Some(user_idx) = user_message_index(&list, ui_index) else {
                return;
            };
            let Some(ChatItem::User(text)) = list.get(ui_index) else {
                return;
            };
            let sid = active_session.get();
            if sid.as_deref().is_none_or(str::is_empty) {
                return;
            }
            let user_idx = user_idx
                + sid
                    .as_deref()
                    .and_then(|id| transcript_pages.with(|pages| pages.get(id).copied()))
                    .map_or(0, |page| page.user_offset);
            let prefix_items = list.iter().take(ui_index).cloned().collect::<Vec<_>>();
            let draft = composer_text_from_user_message(text);
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            spawn_local(async move {
                let arg = to_value(&tauri_args::branch_session(
                    &sid,
                    Some(draft.as_str()),
                    Some(user_idx),
                ))
                .unwrap();
                let Some(id) = invoke("branch_session", arg).await.as_string() else {
                    let loc = locale.get();
                    status.set(t(loc, "status.send_failed").into());
                    return;
                };
                let loaded = invoke(
                    "load_session",
                    to_value(&serde_json::json!({ "id": id.clone() })).unwrap(),
                )
                .await;
                let (branch_items, page_state) =
                    match serde_wasm_bindgen::from_value::<LoadedSessionPage>(loaded) {
                        Ok(page) => {
                            conversation_outlines.update(|outlines| {
                                outlines.insert(id.clone(), page.outline.clone());
                            });
                            (
                                page.items
                                    .into_iter()
                                    .map(LoadedItem::into_chat)
                                    .collect::<Vec<_>>(),
                                Some(TranscriptPageState {
                                    next_before_seq: page.next_before_seq,
                                    user_offset: page.user_offset,
                                    loading: false,
                                    window_user_start: usize::MAX,
                                }),
                            )
                        }
                        Err(_) => (prefix_items, None),
                    };
                if let Some(old) = sid {
                    transcripts.update(|m| {
                        m.insert(old, list.clone());
                        m.insert(id.clone(), branch_items.clone());
                    });
                }
                if let Some(page_state) = page_state {
                    transcript_pages.update(|pages| {
                        pages.insert(id.clone(), page_state);
                    });
                }
                items.set(branch_items);
                input.set(draft);
                active_session.set(Some(id));
                refresh_session_history();
                focus_composer();
            });
        }
    };

    // Queue (#433): edit / cancel / cut-in a parked follow-up from its bubble.
    let on_queue = Callback::new(move |op: QueueOp| {
        let sid = active_session.get_untracked().unwrap_or_default();
        if sid.is_empty() {
            return;
        }
        let (id, action, message): (u64, &'static str, Option<String>) = match op {
            QueueOp::Cancel(id) => {
                route_items(active_session, items, transcripts, &sid, |rows| {
                    rows.retain(
                        |it| !matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id),
                    );
                });
                (id, "cancel", None)
            }
            // The bubble stays; it promotes to a User row when the running turn
            // folds it in and emits the matching User event.
            QueueOp::CutIn(id) => (id, "cutin", None),
            QueueOp::Save(id, text) => {
                route_items(active_session, items, transcripts, &sid, |rows| {
                    if let Some(ChatItem::QueuedUser { text: slot, .. }) = rows
                        .iter_mut()
                        .find(|it| matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id))
                    {
                        *slot = text.clone();
                    }
                });
                (id, "edit", Some(text))
            }
            // Reorder (#433): swap with the neighbouring queued row locally, then
            // mirror it server-side. Queued rows sit contiguously at the tail, so
            // a neighbour that is not a QueuedUser means we are at an end → no-op.
            QueueOp::MoveUp(id) | QueueOp::MoveDown(id) => {
                let up = matches!(op, QueueOp::MoveUp(_));
                route_items(active_session, items, transcripts, &sid, |rows| {
                    let Some(i) = rows
                        .iter()
                        .position(|it| matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id))
                    else {
                        return;
                    };
                    let target = if up {
                        i.checked_sub(1)
                    } else {
                        (i + 1 < rows.len()).then_some(i + 1)
                    };
                    if let Some(j) = target {
                        if matches!(rows.get(j), Some(ChatItem::QueuedUser { .. })) {
                            rows.swap(i, j);
                        }
                    }
                });
                (id, if up { "move_up" } else { "move_down" }, None)
            }
        };
        spawn_local(async move {
            let args = to_value(&QueuedTurnActionArgs {
                session_id: sid,
                id,
                action,
                message,
            })
            .unwrap();
            let _ = invoke("queued_turn_action", args).await;
        });
    });

    let resume_turn = {
        let locale = locale;
        let status = status;
        let running = running;
        let busy = busy;
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let stopping_session = stopping_session;
        let pending_turns = pending_turns;
        let models = models;
        let needs_api_key = needs_api_key;
        move |error_idx: usize| {
            if busy.get() {
                return;
            }
            let Some(id) = active_session.get() else {
                return;
            };
            if active_acp_agent_id.get().is_some() {
                status.set("ACP protocol v1 cannot replay a Wisp transcript.".into());
                return;
            }
            let model = session_model_label(&models.get(), &session_model_ids.get(), Some(&id));
            items.update(|v| {
                strip_error_at(v, error_idx);
                ensure_streaming_assistant(v, model.clone());
            });
            force_chat_bottom();
            begin_pending_turn(pending_turns, running, &id);
            spawn_local(async move {
                let arg = to_value(&SendMessageArgs {
                    session_id: Some(id.clone()),
                    message: String::new(),
                    attachments: vec![],
                    references: vec![],
                    resume: true,
                    acp_agent_id: None,
                    guide: None,
                    replace: None,
                })
                .unwrap();
                match invoke_checked("send_message", arg).await {
                    Ok(_) => {
                        finish_pending_turn(pending_turns, running, &id);
                        if stopping_session.get().as_deref() == Some(&id) {
                            stopping_session.set(None);
                        }
                        let is_active = active_session.get().as_deref() == Some(&id);
                        let stranded = if is_active {
                            items.with(|v| {
                                v.iter()
                                    .any(|c| matches!(c, ChatItem::Tool { ok: None, .. }))
                            })
                        } else {
                            transcripts.with(|m| {
                                m.get(&id).map_or(false, |v| {
                                    v.iter()
                                        .any(|c| matches!(c, ChatItem::Tool { ok: None, .. }))
                                })
                            })
                        };
                        if stranded {
                            let v = invoke(
                                "load_session",
                                to_value(&serde_json::json!({ "id": id })).unwrap(),
                            )
                            .await;
                            if let Ok(page) = serde_wasm_bindgen::from_value::<LoadedSessionPage>(v)
                            {
                                conversation_outlines.update(|outlines| {
                                    outlines.insert(id.clone(), page.outline.clone());
                                });
                                let chats: Vec<ChatItem> =
                                    page.items.into_iter().map(LoadedItem::into_chat).collect();
                                transcript_pages.update(|pages| {
                                    pages.insert(
                                        id.clone(),
                                        TranscriptPageState {
                                            next_before_seq: page.next_before_seq,
                                            user_offset: page.user_offset,
                                            loading: false,
                                            window_user_start: usize::MAX,
                                        },
                                    );
                                });
                                transcripts.update(|m| {
                                    m.insert(id.clone(), chats.clone());
                                });
                                if active_session.get().as_deref() == Some(&id) {
                                    items.set(chats);
                                    force_chat_bottom();
                                }
                            }
                        }
                        refresh_session_history();
                    }
                    Err(err) => {
                        let loc = locale.get();
                        let raw = js_error_text(err);
                        if raw.contains(NO_API_KEY_MARK) {
                            needs_api_key.set(true);
                        }
                        status.set(tf(
                            loc,
                            "status.send_failed",
                            &[("msg", &localize_backend(loc, &raw))],
                        ));
                        finish_pending_turn(pending_turns, running, &id);
                        if stopping_session.get().as_deref() == Some(&id) {
                            stopping_session.set(None);
                        }
                    }
                }
            });
        }
    };

    let pick_files = move |_| {
        if uploading.get() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(doc) = window.document() else {
            return;
        };
        let Some(el) = doc.get_element_by_id("composer-file-input") else {
            return;
        };
        let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.click());
    };

    let on_files_selected = move |_ev: web_sys::Event| {
        if uploading.get() {
            return;
        }
        upload_from_input(attachments, uploading, "composer-file-input");
    };

    let on_drag_over = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        if !uploading.get() {
            drag_over.set(true);
        }
    };

    let on_drag_leave = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(false);
    };

    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(false);
        if uploading.get() {
            return;
        }
        if let Some(dt) = ev.data_transfer() {
            if let Some(files) = dt.files() {
                queue_uploads(attachments, uploading, files.into());
            }
        }
    };

    let on_paste = move |ev: web_sys::Event| {
        if uploading.get() {
            return;
        }
        let event: JsValue = ev.clone().into();
        let count = pasted_image_count(event.clone());
        if count == 0 {
            return;
        }
        ev.prevent_default();
        upload_from_paste(attachments, uploading, event, count);
    };

    let composer_blocked = move || uploading.get();

    let run_update_check = Rc::new(move || {
        if update_check_busy.get() {
            update_check_modal.set(Some(UpdateCheckModal::Checking));
            return;
        }
        let checking = t(locale.get(), "status.checking_updates").to_string();
        update_check_busy.set(true);
        update_check_modal.set(Some(UpdateCheckModal::Checking));
        settings_message.set(Some((true, checking.clone())));
        status.set(checking);
        let msg = settings_message;
        let busy = update_check_busy;
        let loc = locale;
        let modal = update_check_modal;
        let status_msg = status;
        spawn_local(async move {
            match invoke_checked("check_for_updates", JsValue::UNDEFINED).await {
                Ok(v) => match serde_wasm_bindgen::from_value::<UpdateCheck>(v) {
                    Ok(update) if update.update_available => {
                        let text = tf(
                            loc.get(),
                            "status.update_available",
                            &[("version", &update.latest_version)],
                        );
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text);
                        modal.set(Some(UpdateCheckModal::Available {
                            version: update.latest_version,
                            notes: update.notes,
                            release_url: update.release_url,
                        }));
                    }
                    Ok(update) => {
                        let text = tf(
                            loc.get(),
                            "status.up_to_date",
                            &[("version", &update.current_version)],
                        );
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text);
                        modal.set(Some(UpdateCheckModal::UpToDate {
                            version: update.current_version,
                        }));
                    }
                    Err(_) => {
                        let text = t(loc.get(), "status.update_check_complete").to_string();
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text.clone());
                        modal.set(Some(UpdateCheckModal::Failed { message: text }));
                    }
                },
                Err(err) => {
                    let text = localize_backend(loc.get(), &js_error_text(err));
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text.clone());
                    modal.set(Some(UpdateCheckModal::Failed { message: text }));
                }
            }
            busy.set(false);
        });
    });
    let check_updates = {
        let run_update_check = run_update_check.clone();
        move |_| run_update_check()
    };

    let refresh_skills = move || {
        spawn_local(async move {
            let v = invoke("list_skills", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                skills_list.set(rows);
            }
        });
    };

    let install_skill_from = move |path: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
            match invoke_checked("install_skill", arg).await {
                Ok(_) => {
                    skills_msg.set(None);
                    refresh_skills();
                }
                Err(err) => {
                    skills_msg.set(Some((
                        false,
                        localize_backend(locale.get(), &js_error_text(err)),
                    )));
                }
            }
        });
    };

    let refresh_plugins = move || {
        spawn_local(async move {
            let value = invoke("list_plugins", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<PluginRow>>(value) {
                plugins_list.set(rows);
            }
        });
    };

    let install_plugin_from =
        Callback::new(move |(path, expected_sha256): (String, Option<String>)| {
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "srcPath": path,
                    "expectedSha256": expected_sha256,
                }))
                .unwrap();
                match invoke_checked("install_plugin", args).await {
                    Ok(_) => {
                        plugins_msg.set(Some((true, t(locale.get(), "plugins.installed").into())));
                        plugin_install_open.set(false);
                        refresh_plugins();
                    }
                    Err(error) => {
                        plugins_msg.set(Some((
                            false,
                            localize_backend(locale.get(), &js_error_text(error)),
                        )));
                        refresh_plugins();
                    }
                }
            });
        });
    let install_plugin_url =
        Callback::new(move |(source_url, expected_sha256): (String, String)| {
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "sourceUrl": source_url,
                    "expectedSha256": expected_sha256,
                }))
                .unwrap();
                match invoke_checked("install_plugin_url", args).await {
                    Ok(_) => {
                        plugins_msg.set(Some((true, t(locale.get(), "plugins.installed").into())));
                        plugin_install_open.set(false);
                        refresh_plugins();
                    }
                    Err(error) => {
                        plugins_msg.set(Some((
                            false,
                            localize_backend(locale.get(), &js_error_text(error)),
                        )));
                        refresh_plugins();
                    }
                }
            });
        });
    let set_plugin_enabled =
        Callback::new(move |(id, version, enabled): (String, String, bool)| {
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "pluginId": id,
                    "version": version,
                    "enabled": enabled,
                }))
                .unwrap();
                match invoke_checked("set_plugin_enabled", args).await {
                    Ok(_) => {
                        plugins_msg.set(None);
                        refresh_plugins();
                        refresh_skills();
                    }
                    Err(error) => {
                        plugins_msg.set(Some((
                            false,
                            localize_backend(locale.get(), &js_error_text(error)),
                        )));
                        refresh_plugins();
                    }
                }
            });
        });
    let remove_plugin = Callback::new(move |(id, version): (String, String)| {
        spawn_local(async move {
            let args =
                to_value(&serde_json::json!({ "pluginId": id, "version": version })).unwrap();
            match invoke_checked("remove_plugin", args).await {
                Ok(_) => {
                    plugins_msg.set(None);
                    refresh_plugins();
                    refresh_skills();
                }
                Err(error) => plugins_msg.set(Some((
                    false,
                    localize_backend(locale.get(), &js_error_text(error)),
                ))),
            }
        });
    });

    let refresh_conns = move || {
        spawn_local(async move {
            let v = invoke("list_mcp_connections", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnView>(v) {
                conns_view.set(Some(view));
            }
            let c = invoke("list_connectors", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnectorsView>(c) {
                connectors.set(Some(view));
            }
        });
    };

    let refresh_approval_grants = move || {
        spawn_local(async move {
            let v = invoke("list_approval_grants", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ApprovalGrantRow>>(v) {
                approval_grants.set(rows);
            }
        });
    };

    let load_custom_conn_tools = move |row: ConnRow| {
        let id = row.id.clone();
        custom_conn_tools_loading.update(|s| {
            s.insert(id.clone());
        });
        custom_conn_tool_errors.update(|m| {
            m.remove(&id);
        });
        spawn_local(async move {
            let conn = build_conn_json(&conn_form_from_row(&row), false);
            let out = invoke_checked(
                "test_mcp_connection",
                to_value(&serde_json::json!({ "conn": conn })).unwrap(),
            )
            .await;
            match out.and_then(|v| {
                serde_wasm_bindgen::from_value::<Vec<ConnectorTool>>(v)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
            }) {
                Ok(tools) => custom_conn_tools.update(|m| {
                    m.insert(id.clone(), tools);
                }),
                Err(err) => custom_conn_tool_errors.update(|m| {
                    m.insert(id.clone(), js_error_text(err));
                }),
            }
            custom_conn_tools_loading.update(|s| {
                s.remove(&id);
            });
        });
    };

    let refresh_memory = move || {
        spawn_local(async move {
            let v = invoke("get_memory_view", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                memory_view.set(Some(view));
            }
        });
    };

    let refresh_credentials = move || {
        spawn_local(async move {
            let v = invoke("credential_status", JsValue::UNDEFINED).await;
            if let Ok(pairs) = serde_wasm_bindgen::from_value::<Vec<(String, bool)>>(v) {
                cred_status.set(pairs.into_iter().collect());
            }
            let v = invoke("list_custom_credentials", JsValue::UNDEFINED).await;
            if let Ok(credentials) =
                serde_wasm_bindgen::from_value::<Vec<CustomCredentialStatus>>(v)
            {
                custom_credentials.set(credentials);
            }
        });
    };

    let load_memory_file = move |name: String| {
        memory_selected.set(Some(name.clone()));
        memory_msg.set(None);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "name": name })).unwrap();
            let v = invoke("read_memory_file", arg).await;
            memory_editor.set(v.as_string().unwrap_or_default());
        });
    };

    let close_settings_subpage = move || {
        model_form.set(None);
        model_form_key.set(String::new());
        model_form_msg.set(None);
        acp_form.set(None);
        acp_form_msg.set(None);
        specialist_form.set(None);
        conn_form.set(None);
        open_conn_key.set(None);
        channels_open.set(None);
        conn_test_msg.set(None);
        memory_selected.set(None);
        memory_editor.set(String::new());
        memory_msg.set(None);
        skills_msg.set(None);
        plugins_msg.set(None);
    };

    let go_settings_section = move |sec: &str| {
        close_settings_subpage();
        settings_section.set(sec.into());
        match sec {
            "models" => refresh_models(),
            "specialists" => refresh_specialists(),
            "memory" => refresh_memory(),
            "skills" => {
                refresh_skills();
            }
            "plugins" => refresh_plugins(),
            "connections" => refresh_conns(),
            "credentials" => refresh_credentials(),
            "permissions" => refresh_approval_grants(),
            _ => {}
        }
    };

    let open_settings_fn = move |section: Option<String>| {
        show_settings.set(true);
        settings_message.set(None);
        needs_api_key.set(false);
        close_settings_subpage();
        if let Some(sec) = section {
            settings_section.set(sec);
        }
        let s = settings;
        let msg = settings_message;
        let loc = locale;
        refresh_skills();
        refresh_plugins();
        refresh_conns();
        refresh_models();
        refresh_specialists();
        refresh_memory();
        refresh_credentials();
        refresh_approval_grants();
        spawn_local(async move {
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                let mut cfg = normalized_settings(cfg);
                // Keep the live locale authoritative: reloading the settings form
                // must not clobber an unsaved language change (#431). Sync the form
                // field to the live signal instead of the other way around.
                cfg.locale = loc.get_untracked().code().into();
                s.set(cfg);
            } else {
                msg.set(Some((
                    false,
                    t(loc.get(), "status.failed_load_settings").into(),
                )));
            }
        });
    };
    let open_settings = move |_| open_settings_fn(None);

    let save_settings = move |_| {
        if settings_busy.get() {
            return;
        }
        let mut cfg = normalized_settings(settings.get());
        cfg.locale = locale.get().code().into();
        let s = settings;
        let show = show_settings;
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        let loc = locale;
        let refresh_pet = refresh_pet;
        busy.set(true);
        let saving = t(loc.get(), "status.saving_settings").to_string();
        msg.set(Some((true, saving.clone())));
        status_msg.set(saving);
        spawn_local(async move {
            let settings_result = invoke_checked(
                "set_settings",
                to_value(&serde_json::json!({ "settings": cfg.clone() })).unwrap(),
            )
            .await;
            if let Err(err) = settings_result {
                let l = loc.get();
                let text = tf(
                    l,
                    "status.save_failed",
                    &[("msg", &localize_backend(l, &js_error_text(err)))],
                );
                msg.set(Some((false, text.clone())));
                status_msg.set(text);
                busy.set(false);
                return;
            }
            if !cfg.sync_relay_token.trim().is_empty() {
                cfg.has_sync_relay_token = true;
                cfg.sync_relay_token.clear();
            }
            busy.set(false);
            show.set(false);
            status_msg.set(t(loc.get(), "status.settings_saved").into());
            s.set(cfg);
            refresh_pet.call(());
        });
    };

    let save_model_form = move |_| {
        if settings_busy.get() {
            return;
        }
        let Some(form) = model_form.get() else {
            return;
        };
        let loc = locale.get();
        let key = model_form_key.get();
        let has_key = form
            .id
            .as_ref()
            .and_then(|id| {
                models
                    .get()
                    .iter()
                    .find(|m| &m.id == id)
                    .map(|m| m.has_api_key)
            })
            .unwrap_or(false);
        let cfg = model_form_to_settings(&form, has_key && key.is_empty());
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            let text = tf(loc, "status.save_failed", &[("msg", &err)]);
            model_form_msg.set(Some((false, text)));
            return;
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        let provider = provider_value(&form.provider);
        let profile = serde_json::json!({
            "id": form.id.clone().unwrap_or_default(),
            "label": form.label.trim(),
            "provider": provider,
            "api_url": form.api_url.trim(),
            "model": form.model.trim(),
            "max_tokens": form.max_tokens,
            "context_window": form.context_window,
            "reasoning_effort": form.reasoning_effort.trim(),
            "supports_vision": form.supports_vision,
            "use_for_vision": form.use_for_vision,
        });
        let key_arg = if key.is_empty() { None } else { Some(key) };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "profile": profile,
                "key": key_arg,
                "useForVision": form.use_for_vision,
            }))
            .unwrap();
            match invoke_checked("save_model", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                    let v = invoke("get_settings", JsValue::UNDEFINED).await;
                    if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                        settings.set(normalized_settings(cfg));
                    }
                    model_form.set(None);
                    model_form_key.set(String::new());
                    model_form_msg.set(Some((true, t(loc, "status.settings_saved").into())));
                }
                Err(err) => {
                    model_form_msg.set(Some((false, localize_backend(loc, &js_error_text(err)))));
                }
            }
            settings_busy.set(false);
        });
    };

    let validate_model_form = move |_| {
        if settings_busy.get() {
            return;
        }
        let Some(form) = model_form.get() else {
            return;
        };
        let loc = locale.get();
        let key = model_form_key.get();
        let has_key = models
            .get()
            .iter()
            .find(|m| Some(m.id.as_str()) == form.id.as_deref())
            .map(|m| m.has_api_key)
            .unwrap_or(false);
        let cfg = model_form_to_settings(&form, has_key);
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            model_form_msg.set(Some((
                false,
                tf(loc, "status.validation_failed", &[("msg", &err)]),
            )));
            return;
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.validating").into())));
        spawn_local(async move {
            let res = invoke_timeout(
                "validate_settings",
                to_value(&serde_json::json!({
                    "settings": cfg,
                    "key": key,
                    "profileId": form.id.clone(),
                }))
                .unwrap(),
                35_000,
            )
            .await;
            match res {
                Ok(v) => {
                    let raw = v
                        .as_string()
                        .unwrap_or_else(|| t(loc, "status.validation_succeeded").into());
                    model_form_msg.set(Some((true, localize_backend(loc, &raw))));
                }
                Err(err) => {
                    model_form_msg.set(Some((
                        false,
                        tf(
                            loc,
                            "status.validation_failed",
                            &[("msg", &localize_backend(loc, &js_error_text(err)))],
                        ),
                    )));
                }
            }
            settings_busy.set(false);
        });
    };

    let test_reviewer_form = move |_| {
        let Some(spec) = specialist_form.get() else {
            return;
        };
        if spec.id != "reviewer" || settings_busy.get() {
            return;
        }
        let loc = locale.get();
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "specialists.reviewer.testing").into())));
        spawn_local(async move {
            let result = invoke_timeout(
                "test_reviewer_backend",
                to_value(&serde_json::json!({ "reviewer": spec })).unwrap(),
                120_000,
            )
            .await;
            match result {
                Ok(value) => {
                    match serde_wasm_bindgen::from_value::<ReviewerBackendTestResult>(value) {
                        Ok(result) => {
                            let backend = match result.backend.as_str() {
                                "acp_agent" => "ACP",
                                "http_model" => "HTTP",
                                other => other,
                            };
                            let headline = tf(
                                loc,
                                "specialists.reviewer.test_ok",
                                &[
                                    ("backend", backend),
                                    ("model", &result.model),
                                    ("status", &result.status),
                                ],
                            );
                            model_form_msg.set(Some((
                                true,
                                if result.summary.trim().is_empty() {
                                    headline
                                } else {
                                    format!("{headline} {}", result.summary.trim())
                                },
                            )));
                        }
                        Err(error) => model_form_msg.set(Some((false, error.to_string()))),
                    }
                }
                Err(error) => model_form_msg.set(Some((
                    false,
                    tf(
                        loc,
                        "specialists.reviewer.test_failed",
                        &[("msg", &localize_backend(loc, &js_error_text(error)))],
                    ),
                ))),
            }
            settings_busy.set(false);
        });
    };

    let save_specialist_form = move |_| {
        let Some(spec) = specialist_form.get() else {
            return;
        };
        let loc = locale.get();
        if spec.name.trim().is_empty() {
            model_form_msg.set(Some((false, t(loc, "specialists.name_required").into())));
            return;
        }
        let saved_id = spec.id.clone();
        let keep_open = saved_id == "reviewer";
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "spec": spec })).unwrap();
            match invoke_checked("save_specialist_cmd", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                    Ok(value) => {
                        let saved = value.iter().find(|item| item.id == saved_id).cloned();
                        specialists.set(value);
                        if keep_open {
                            specialist_form.set(saved);
                            model_form_msg.set(Some((true, t(loc, "specialists.saved").into())));
                        } else {
                            specialist_form.set(None);
                            settings_message.set(Some((true, t(loc, "specialists.saved").into())));
                        }
                    }
                    Err(error) => model_form_msg.set(Some((false, error.to_string()))),
                },
                Err(error) => model_form_msg.set(Some((false, js_error_text(error)))),
            }
            settings_busy.set(false);
        });
    };

    let remove_specialist_fn = move |id: String| {
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id })).unwrap();
            match invoke_checked("remove_specialist", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                    Ok(value) => specialists.set(value),
                    Err(error) => settings_message.set(Some((false, error.to_string()))),
                },
                Err(error) => settings_message.set(Some((false, js_error_text(error)))),
            }
        });
    };

    let new_session = move |_| {
        demo_mode.set(false); // starting a fresh chat leaves the demo view
                              // Stash the current transcript under its id so a running turn keeps
                              // streaming into the cache, then create a fresh frame and show it.
                              // We do NOT cancel any running turn — parallel conversations keep going.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| {
                m.insert(old, items.get());
            });
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let v = invoke("new_session", JsValue::UNDEFINED).await;
            // Guard the malformed-response case: a `None` id would blank the active
            // session and strand the user on an empty, unusable view (#15). The old
            // transcript is already stashed above, so bailing keeps it reachable.
            let Some(id) = v.as_string() else {
                status.set(t(locale.get(), "status.send_failed").into());
                return;
            };
            active_session.set(Some(id));
            items.set(vec![]);
            refresh_session_history();
            focus_composer();
        });
    };

    let start_env_setup = {
        let items = items;
        let running = running;
        let status = status;
        let locale = locale;
        let show_capabilities = show_capabilities;
        let active_session = active_session;
        let sel_artifact = sel_artifact;
        let right_tab = right_tab;
        let models = models;
        move |_| {
            if busy.get() {
                return;
            }
            show_capabilities.set(false);
            attachments.set(vec![]);
            sel_artifact.set(0);
            right_tab.set(RightTab::Artifacts);
            let text: String = t(locale.get(), "caps.env_setup_prompt").into();
            let turn_model = active_model_label(&models.get());
            items.set(vec![
                ChatItem::User(text.clone()),
                ChatItem::Assistant {
                    text: String::new(),
                    model: turn_model,
                    resources: Vec::new(),
                },
            ]);
            force_chat_bottom();
            spawn_local(async move {
                // Fresh frame for the setup turn; route events to it.
                let v = invoke("new_session", JsValue::UNDEFINED).await;
                let id = v.as_string().unwrap_or_default();
                if id.is_empty() {
                    let loc = locale.get();
                    status.set(t(loc, "status.send_failed").into());
                    return;
                }
                active_session.set(Some(id.clone()));
                running.update(|r| {
                    r.insert(id.clone());
                });
                refresh_session_history();
                let arg = to_value(&SendMessageArgs {
                    session_id: Some(id.clone()),
                    message: text,
                    attachments: vec![],
                    references: vec![],
                    resume: false,
                    acp_agent_id: None,
                    guide: None,
                    replace: None,
                })
                .unwrap();
                match invoke_checked("send_message", arg).await {
                    // The awaited command resolving is the reliable turn-complete
                    // signal; clear `running` here so a dropped `Done` broadcast
                    // can't pin the session on "运行中" (#34).
                    Ok(_) => {
                        running.update(|r| {
                            r.remove(&id);
                        });
                        refresh_session_history();
                    }
                    Err(err) => {
                        let loc = locale.get();
                        let raw = js_error_text(err);
                        if raw.contains(NO_API_KEY_MARK) {
                            needs_api_key.set(true);
                        }
                        status.set(tf(
                            loc,
                            "status.send_failed",
                            &[("msg", &localize_backend(loc, &raw))],
                        ));
                        running.update(|r| {
                            r.clear();
                        });
                    }
                }
            });
        }
    };

    let use_plugin = Callback::new(
        move |(plugin_id, version, display_name, skill_names, enabled): (String, String, String, Vec<String>, bool)| {
            let prompt = tf(
                locale.get(),
                if skill_names.is_empty() {
                    "plugins.start_prompt"
                } else {
                    "plugins.start_prompt_guided"
                },
                &[("name", &display_name)],
            );
            let skill_references = skill_names
                .into_iter()
                .map(|name| ComposerReferenceArg::Skill { name })
                .collect();
            let turn_model = active_model_label(&models.get());
            if let Some(old) = active_session.get() {
                transcripts.update(|cache| {
                    cache.insert(old, items.get());
                });
            }
            spawn_local(async move {
                if !enabled {
                    let args = to_value(&serde_json::json!({
                        "pluginId": plugin_id,
                        "version": version,
                        "enabled": true,
                    }))
                    .unwrap();
                    if let Err(error) = invoke_checked("set_plugin_enabled", args).await {
                        plugins_msg.set(Some((
                            false,
                            localize_backend(locale.get(), &js_error_text(error)),
                        )));
                        refresh_plugins();
                        return;
                    }
                    refresh_plugins();
                    refresh_skills();
                }

                let Some(session_id) = invoke("new_session", JsValue::UNDEFINED).await.as_string()
                else {
                    status.set(t(locale.get(), "status.send_failed").into());
                    return;
                };
                demo_mode.set(false);
                show_settings.set(false);
                attachments.set(vec![]);
                sel_artifact.set(0);
                right_tab.set(RightTab::Artifacts);
                active_session.set(Some(session_id.clone()));
                items.set(vec![
                    ChatItem::User(prompt.clone()),
                    ChatItem::Assistant {
                        text: String::new(),
                        model: turn_model,
                        resources: Vec::new(),
                    },
                ]);
                running.update(|sessions| {
                    sessions.insert(session_id.clone());
                });
                refresh_session_history();
                force_chat_bottom();

                let args = to_value(&SendMessageArgs {
                    session_id: Some(session_id.clone()),
                    message: prompt,
                    attachments: vec![],
                    references: skill_references,
                    resume: false,
                    acp_agent_id: None,
                    guide: None,
                    replace: None,
                })
                .unwrap();
                match invoke_checked("send_message", args).await {
                    Ok(_) => {
                        running.update(|sessions| {
                            sessions.remove(&session_id);
                        });
                        refresh_session_history();
                    }
                    Err(error) => {
                        let loc = locale.get();
                        let raw = js_error_text(error);
                        if raw.contains(NO_API_KEY_MARK) {
                            needs_api_key.set(true);
                        }
                        status.set(tf(
                            loc,
                            "status.send_failed",
                            &[("msg", &localize_backend(loc, &raw))],
                        ));
                        running.update(|sessions| {
                            sessions.remove(&session_id);
                        });
                    }
                }
            });
        },
    );

    let load_session = Callback::new(move |id: String| {
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        // Stash the transcript we're leaving under its id.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| {
                m.insert(old, items.get());
            });
        }
        let is_running = running.get().contains(&id);
        active_session.set(Some(id.clone()));
        if is_running {
            // Mid-stream: render the cached transcript immediately, but still
            // reconcile the separately persisted Plan claim/status. This keeps
            // session switching and restart semantics identical.
            items.set(transcripts.with(|m| m.get(&id).cloned().unwrap_or_default()));
            transcript_pages.update(|pages| {
                pages.entry(id.clone()).or_default().window_user_start = usize::MAX;
            });
            force_chat_bottom();
            // Still retarget the backend's viewed-session marker so uploads
            // attach here (#194). Not `load_session`: that would overwrite the
            // running turn's persisted seq with the DB snapshot.
            spawn_local(async move {
                let _ = invoke(
                    "set_viewed_session",
                    to_value(&serde_json::json!({ "id": id })).unwrap(),
                )
                .await;
            });
            return;
        }
        // Idle session: load from DB and overwrite any stale cache entry.
        spawn_local(async move {
            let v = invoke(
                "load_session",
                to_value(&serde_json::json!({ "id": id.clone() })).unwrap(),
            )
            .await;
            if let Ok(page) = serde_wasm_bindgen::from_value::<LoadedSessionPage>(v) {
                let presentations = page.presentations.clone();
                conversation_outlines.update(|outlines| {
                    outlines.insert(id.clone(), page.outline.clone());
                });
                let chats: Vec<ChatItem> =
                    page.items.into_iter().map(LoadedItem::into_chat).collect();
                transcript_pages.update(|pages| {
                    pages.insert(
                        id.clone(),
                        TranscriptPageState {
                            next_before_seq: page.next_before_seq,
                            user_offset: page.user_offset,
                            loading: false,
                            window_user_start: usize::MAX,
                        },
                    );
                });
                transcripts.update(|m| {
                    m.insert(id.clone(), chats.clone());
                });
                // Only repaint the view if we're still on this session — a rapid
                // switch could have moved on while the load was in flight, and an
                // unguarded set would clobber the newer view with stale rows (#53).
                if active_session.get().as_deref() == Some(&id) {
                    items.set(chats);
                    for presentation in presentations {
                        if presentation.presentation_kind == "mcp_app" {
                            show_mcp_app.call((
                                id.clone(),
                                presentation.presentation_id,
                                presentation.payload,
                                false,
                            ));
                        }
                    }
                    force_chat_bottom();
                }
            }
        });
    });
    let refresh_agent_sessions = Callback::new(move |_: ()| refresh_session_history());

    // Take-over from the Agents panel: load_session flips the right pane to
    // Artifacts for the generic session-switch path, which made the panel (and
    // the workflow being inspected) vanish mid-click (#442).
    let takeover_session = Callback::new(move |id: String| {
        load_session.call(id);
        right_tab.set(RightTab::Agents);
    });

    // "Ask model to review" in the Agents panel drops the serialized workflow
    // config into the composer so the user can send it to the current chat.
    let agent_config_to_chat = Callback::new(move |text: String| {
        input.set(text);
        focus_composer();
    });

    let load_earlier_messages = Callback::new(move |_: ()| {
        let Some(id) = active_session.get_untracked() else {
            return;
        };
        if running.with_untracked(|sessions| sessions.contains(&id)) {
            return;
        }
        let Some(cursor) = transcript_pages.with_untracked(|pages| {
            pages
                .get(&id)
                .and_then(|page| (!page.loading).then_some(page.next_before_seq).flatten())
        }) else {
            return;
        };
        transcript_pages.update(|pages| {
            if let Some(page) = pages.get_mut(&id) {
                page.loading = true;
            }
        });
        spawn_local(async move {
            let value = invoke(
                "load_session",
                to_value(&serde_json::json!({
                    "id": id.clone(),
                    "beforeSeq": cursor,
                }))
                .unwrap(),
            )
            .await;
            let Ok(page) = serde_wasm_bindgen::from_value::<LoadedSessionPage>(value) else {
                transcript_pages.update(|pages| {
                    if let Some(page) = pages.get_mut(&id) {
                        page.loading = false;
                    }
                });
                return;
            };
            let older = page
                .items
                .into_iter()
                .map(LoadedItem::into_chat)
                .collect::<Vec<_>>();
            transcripts.update(|saved| {
                let current = saved.entry(id.clone()).or_default();
                current.splice(0..0, older.iter().cloned());
            });
            transcript_pages.update(|pages| {
                pages.insert(
                    id.clone(),
                    TranscriptPageState {
                        next_before_seq: page.next_before_seq,
                        user_offset: page.user_offset,
                        loading: false,
                        window_user_start: 0,
                    },
                );
            });
            if active_session.get_untracked().as_deref() == Some(id.as_str()) {
                preserve_chat_prepend_position();
                items.update(|current| {
                    current.splice(0..0, older);
                });
            }
        });
    });

    let show_earlier_loaded = Callback::new(move |_: ()| {
        let Some(id) = active_session.get_untracked() else {
            return;
        };
        let requested = transcript_pages.with_untracked(|pages| {
            pages
                .get(&id)
                .map_or(usize::MAX, |page| page.window_user_start)
        });
        let (_, start, _) = items.with_untracked(|rows| {
            transcript_render_window(rows, requested, TRANSCRIPT_RENDER_TURNS)
        });
        transcript_pages.update(|pages| {
            pages.entry(id).or_default().window_user_start =
                start.saturating_sub(TRANSCRIPT_WINDOW_STEP);
        });
    });

    let show_newer_loaded = Callback::new(move |_: ()| {
        let Some(id) = active_session.get_untracked() else {
            return;
        };
        let requested = transcript_pages.with_untracked(|pages| {
            pages
                .get(&id)
                .map_or(usize::MAX, |page| page.window_user_start)
        });
        let (_, start, total) = items.with_untracked(|rows| {
            transcript_render_window(rows, requested, TRANSCRIPT_RENDER_TURNS)
        });
        let latest_start = total.saturating_sub(TRANSCRIPT_RENDER_TURNS);
        let next = start.saturating_add(TRANSCRIPT_WINDOW_STEP);
        transcript_pages.update(|pages| {
            pages.entry(id).or_default().window_user_start = if next >= latest_start {
                usize::MAX
            } else {
                next
            };
        });
    });

    let jump_to_conversation_outline =
        Callback::new(move |(target, before_seq): (usize, Option<i64>)| {
            let Some(id) = active_session.get_untracked() else {
                return;
            };
            let user_offset = transcript_pages
                .with_untracked(|pages| pages.get(&id).copied())
                .map_or(0, |page| page.user_offset);
            if conversation_outline_target_is_loaded(
                &items.get_untracked(),
                user_offset,
                target,
            ) {
                transcript_pages.update(|pages| {
                    pages.entry(id).or_default().window_user_start =
                        target.saturating_sub(user_offset);
                });
                conversation_outline_selected.set(Some(target));
                jump_chat_to_user(target);
                return;
            }
            if busy.get_untracked() {
                return;
            }
            conversation_outline_selected.set(Some(target));
            spawn_local(async move {
                let value = invoke(
                    "load_session",
                    to_value(&serde_json::json!({
                        "id": id.clone(),
                        "beforeSeq": before_seq,
                    }))
                    .unwrap(),
                )
                .await;
                let Ok(page) = serde_wasm_bindgen::from_value::<LoadedSessionPage>(value) else {
                    return;
                };
                let target_local = target.saturating_sub(page.user_offset);
                let chats = page
                    .items
                    .into_iter()
                    .map(LoadedItem::into_chat)
                    .collect::<Vec<_>>();
                let loaded_turns = chats
                    .iter()
                    .filter(|item| {
                        matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. })
                    })
                    .count();
                if target < page.user_offset || target_local >= loaded_turns {
                    return;
                }
                if !page.outline.is_empty() {
                    conversation_outlines.update(|outlines| {
                        outlines.insert(id.clone(), page.outline);
                    });
                }
                transcript_pages.update(|pages| {
                    pages.insert(
                        id.clone(),
                        TranscriptPageState {
                            next_before_seq: page.next_before_seq,
                            user_offset: page.user_offset,
                            loading: false,
                            window_user_start: target_local,
                        },
                    );
                });
                transcripts.update(|saved| {
                    saved.insert(id.clone(), chats.clone());
                });
                if active_session.get_untracked().as_deref() == Some(id.as_str()) {
                    items.set(chats);
                    jump_chat_to_user(target);
                }
            });
        });

    let load_demo = move |info: DemoInfo| {
        let id = info.id.clone();
        let items = items;
        // Demos are read-only transcripts; they don't stream, so we don't touch
        // `running`. We do stash the current chat so returning to it is possible.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| {
                m.insert(old, items.get());
            });
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        active_session.set(None);
        spawn_local(async move {
            // Fresh session so the demo doesn't mix into a real conversation.
            let _ = invoke("new_session", JsValue::UNDEFINED).await;
            let v = invoke(
                "load_demo",
                to_value(&serde_json::json!({ "id": id })).unwrap(),
            )
            .await;
            if let Ok(demo) = serde_wasm_bindgen::from_value::<Demo>(v) {
                let mut view = vec![ChatItem::User(demo.request.clone())];
                if let Some(t) = &demo.thinking {
                    if !t.is_empty() {
                        view.push(ChatItem::Reasoning(t.clone()));
                    }
                }
                view.push(ChatItem::Assistant {
                    text: demo.response.clone(),
                    model: None,
                    resources: Vec::new(),
                });
                items.set(view);
                force_chat_bottom();
                status_cb.set(tf(locale.get(), "status.demo", &[("title", &demo.title)]));
            }
        });
    };

    let respond_confirm = {
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let approval_pending = approval_pending;
        Callback::new(
            move |(sid, approved, feedback, scope): (String, bool, Option<String>, String)| {
                route_items(
                    active_session,
                    items,
                    transcripts,
                    &sid,
                    strip_approval_pending,
                );
                approval_pending.update(|s| {
                    s.remove(&sid);
                });
                let arg = to_value(&tauri_args::confirm_response(
                    &sid,
                    approved,
                    feedback.as_deref(),
                    Some(&scope),
                ))
                .unwrap();
                spawn_local(async move {
                    let _ = invoke("confirm_response", arg).await;
                });
            },
        )
    };

    let on_sidebar_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        sidebar_dragging.set(true);
        sidebar_drag_start_x.set(ev.client_x() as f64);
        sidebar_drag_start_w.set(sidebar_w.get());
    };
    let on_sidebar_resize_move = move |ev: web_sys::MouseEvent| {
        if sidebar_dragging.get() {
            let dx = ev.client_x() as f64 - sidebar_drag_start_x.get();
            sidebar_w.set((sidebar_drag_start_w.get() + dx).clamp(SIDEBAR_W_MIN, SIDEBAR_W_MAX));
        }
    };
    let on_sidebar_resize_end = move |_| {
        if sidebar_dragging.get() {
            save_sidebar_w(sidebar_w.get());
            sidebar_dragging.set(false);
        }
    };

    let on_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        dragging.set(true);
        drag_start_x.set(ev.client_x() as f64);
        drag_start_w.set(right_w.get());
    };
    let on_resize_move = move |ev: web_sys::MouseEvent| {
        if dragging.get() {
            let dx = drag_start_x.get() - ev.client_x() as f64;
            let max_width = max_right_pane_width(show_sidebar.get(), sidebar_w.get());
            right_w.set((drag_start_w.get() + dx).clamp(RIGHT_PANE_MIN_WIDTH, max_width));
        }
    };

    let on_composer_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        composer_dragging.set(true);
        composer_drag_start_y.set(ev.client_y() as f64);
        composer_drag_start_h.set(composer_h.get());
    };
    let on_composer_resize_move = move |ev: web_sys::MouseEvent| {
        if composer_dragging.get() {
            let dy = composer_drag_start_y.get() - ev.client_y() as f64;
            composer_h
                .set((composer_drag_start_h.get() + dy).clamp(COMPOSER_H_MIN, COMPOSER_H_MAX));
            composer_h_custom.set(true);
        }
    };
    let on_composer_resize_end = move |_| {
        if composer_dragging.get() {
            composer_dragging.set(false);
            save_composer_h(composer_h.get());
            schedule_chat_follow();
        }
    };

    let on_terminal_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        terminal_dragging.set(true);
        terminal_drag_start_y.set(ev.client_y() as f64);
        terminal_drag_start_h.set(terminal_h.get());
    };
    let on_terminal_resize_move = move |ev: web_sys::MouseEvent| {
        if terminal_dragging.get() {
            let dy = terminal_drag_start_y.get() - ev.client_y() as f64;
            let max_h = web_sys::window()
                .and_then(|window| window.inner_height().ok())
                .and_then(|height| height.as_f64())
                .map(|height| (height - 180.0).max(220.0))
                .unwrap_or(720.0);
            terminal_h.set((terminal_drag_start_h.get() + dy).clamp(150.0, max_h));
        }
    };

    let open_files = move |_| {
        ensure_right_tab(RightTab::File, show_right, open_right_tabs, right_tab);
        refresh_active_file_dir(
            file_source,
            file_cwd,
            file_entries,
            remote_file_cwd,
            remote_file_entries,
            remote_file_loading,
            remote_file_error,
        );
    };

    let open_capabilities = move |_| {
        show_capabilities.set(true);
        refresh_capabilities(caps);
    };

    let start_specialist_chat = Callback::new(move |ev: web_sys::MouseEvent| {
        close_details_ancestor(&ev);
        show_settings.set(false);
        let loc = locale.get();
        let prompt = t(loc, "specialists.chat_prompt").to_string();
        spawn_local(async move {
            let v = invoke("new_session", JsValue::UNDEFINED).await;
            let Some(id) = v.as_string() else {
                status.set(t(loc, "status.send_failed").into());
                return;
            };
            active_session.set(Some(id.clone()));
            items.set(vec![]);
            refresh_session_history();
            let arg = to_value(&SendMessageArgs {
                session_id: Some(id.clone()),
                message: prompt,
                attachments: vec![],
                references: vec![],
                resume: false,
                acp_agent_id: None,
                guide: None,
                replace: None,
            })
            .unwrap();
            begin_pending_turn(pending_turns, running, &id);
            match invoke_checked("send_message", arg).await {
                Ok(_) => refresh_session_history(),
                Err(err) => {
                    let raw = js_error_text(err);
                    if raw.contains(NO_API_KEY_MARK) {
                        needs_api_key.set(true);
                    }
                    status.set(tf(
                        loc,
                        "status.send_failed",
                        &[("msg", &localize_backend(loc, &raw))],
                    ));
                }
            }
            finish_pending_turn(pending_turns, running, &id);
        });
    });

    let save_skill_tags = Callback::new(move |(name, raw): (String, String)| {
        let tags = split_tags(&raw);
        spawn_local(async move {
            let _ = invoke_checked(
                "set_skill_tags",
                to_value(&serde_json::json!({ "name": name, "tags": tags })).unwrap(),
            )
            .await;
            refresh_skills();
        });
    });

    let set_visible_skills_enabled = Callback::new(move |enabled: bool| {
        let tag = skill_filter_tag.get();
        let query = skills_search.get();
        let names = skills_list
            .get()
            .into_iter()
            .filter(|s| !s.managed && skill_matches_filter(s, &tag, &query))
            .map(|s| s.name)
            .collect::<Vec<_>>();
        if names.is_empty() {
            return;
        }
        let names_for_update = names.clone();
        skills_list.update(|list| {
            for skill in list {
                if names_for_update.contains(&skill.name) {
                    skill.enabled = enabled;
                }
            }
        });
        spawn_local(async move {
            let _ = invoke_checked(
                "set_skills_enabled",
                to_value(&serde_json::json!({ "names": names, "enabled": enabled })).unwrap(),
            )
            .await;
            refresh_skills();
        });
    });

    let dismiss_onboarding = Callback::new(move |_| {
        show_onboarding.set(false);
        spawn_local(async move {
            let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await;
        });
    });
    let dismiss_onboard = move |_| dismiss_onboarding.call(());

    // Onboarding step 0: save the entered key as a new model (DeepSeek defaults),
    // reusing the same `save_model` command as Settings. Blank key = skip.
    let save_onboard_key = Callback::new(move |_| {
        let key = onboard_key.get();
        if key.trim().is_empty() {
            return;
        }
        let provider = provider_value(&onboard_provider.get()).to_string();
        let (api_url, model) = provider_defaults(&provider);
        let profile = serde_json::json!({
            "id": "",
            "label": "",
            "provider": provider,
            "api_url": api_url,
            "model": model,
            "max_tokens": 8192,
            "reasoning_effort": "",
            "supports_vision": false,
            "use_for_vision": false,
        });
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "profile": profile,
                "key": Some(key),
                "useForVision": false,
            }))
            .unwrap();
            if let Ok(v) = invoke_checked("save_model", arg).await {
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                    models.set(list);
                }
            }
            onboard_key.set(String::new());
        });
    });

    let ctx_menu = create_rw_signal::<Option<CtxMenu>>(None);
    let rename_session_target = create_rw_signal::<Option<(String, String)>>(None);
    let rename_session_input = create_rw_signal(String::new());
    let session_transfer = create_rw_signal::<Option<SessionTransfer>>(None);
    let session_transfer_busy = create_rw_signal(false);
    let session_transfer_error = create_rw_signal::<Option<String>>(None);
    let folder_modal = create_rw_signal::<Option<FolderModal>>(None);
    let folder_modal_input = create_rw_signal(String::new());
    let file_entry_modal = create_rw_signal::<Option<FileEntryModal>>(None);
    let file_entry_input = create_rw_signal(String::new());
    let file_entry_busy = create_rw_signal(false);
    let file_entry_error = create_rw_signal::<Option<String>>(None);
    let ui_confirm = create_rw_signal::<Option<UiConfirm>>(None);
    let compose_menu_open = create_rw_signal(false);
    let agent_menu_open = create_rw_signal(false);
    let reviewer_model_menu_open = create_rw_signal(false);
    let compute_menu_open = create_rw_signal(false);
    let compute_search = create_rw_signal(String::new());
    let specialist_menu_open = create_rw_signal(false);
    let auto_review_enabled = create_rw_signal(false);
    let delegation_enabled = create_rw_signal(false);
    let delegation_setting_busy = create_rw_signal(false);
    let agent_completion = create_rw_signal(AgentCompletionSettings::default());
    let agent_completion_busy = create_rw_signal(false);
    create_effect(move |_| {
        delegation_enabled.set(false);
        delegation_setting_busy.set(false);
        agent_completion.set(AgentCompletionSettings::default());
        agent_completion_busy.set(false);
        let Some(session_id) = active_session.get() else {
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
            let enabled = invoke_checked("get_session_delegation_enabled", args.clone())
                .await
                .ok()
                .and_then(|value| value.as_bool());
            let completion = invoke_checked("get_session_agent_completion", args)
                .await
                .ok()
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<AgentCompletionSettings>(value).ok()
                });
            if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                delegation_enabled.set(enabled.unwrap_or(false));
                agent_completion.set(completion.unwrap_or_default());
            }
        });
    });
    let save_agent_completion = Callback::new(move |next: AgentCompletionSettings| {
        let previous = agent_completion.get_untracked();
        let Some(session_id) = active_session.get_untracked() else {
            return;
        };
        agent_completion.set(next);
        agent_completion_busy.set(true);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sessionId": session_id.clone(),
                "policy": next.policy,
                "autoResume": next.auto_resume,
            }))
            .unwrap();
            let saved = invoke_checked("set_session_agent_completion", args)
                .await
                .ok()
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<AgentCompletionSettings>(value).ok()
                });
            if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                agent_completion.set(saved.unwrap_or(previous));
                agent_completion_busy.set(false);
            }
        });
    });
    spawn_local(async move {
        let value = invoke("get_auto_review_enabled", JsValue::UNDEFINED).await;
        if let Some(enabled) = value.as_bool() {
            auto_review_enabled.set(enabled);
        }
    });
    let ssh_hosts = create_rw_signal::<Vec<SshHost>>(vec![]);
    let selected_context_id = create_rw_signal::<Option<String>>(None);
    let probing_context_id = create_rw_signal::<Option<String>>(None);
    let context_details_modal = create_rw_signal::<Option<(String, ContextModalKind)>>(None);
    let runtime_interpreter_form = create_rw_signal(None::<RuntimeInterpreterForm>);
    let runtime_environment = create_rw_signal(None::<RuntimeSlot>);
    let runtime_environment_pinned = create_rw_signal(false);
    let runtime_environment_position = create_rw_signal((16, 16));
    let runtime_infos = create_rw_signal::<Vec<RuntimeInfo>>(vec![]);
    let runtime_object_states =
        create_rw_signal::<HashMap<String, RuntimeObjectState>>(HashMap::new());
    let run_records = create_rw_signal::<Vec<RunRecord>>(vec![]);
    let show_add_host = create_rw_signal(false);
    let host_alias = create_rw_signal(String::new());
    let host_hostname = create_rw_signal(String::new());
    let host_user = create_rw_signal(String::new());
    let host_port = create_rw_signal(String::new());
    let host_identity = create_rw_signal(String::new());
    let host_notes = create_rw_signal(String::new());
    let host_auth_method = create_rw_signal(String::from("key"));
    let host_password = create_rw_signal(String::new());
    let host_has_password = create_rw_signal(false);
    let editing_host_alias = create_rw_signal::<Option<String>>(None);
    let ssh_connectivity_modal = create_rw_signal::<Option<SshConnectivityModal>>(None);
    let ssh_connectivity_busy = create_rw_signal(false);

    let open_add_host_form = Callback::new(move |_: ()| {
        editing_host_alias.set(None);
        host_alias.set(String::new());
        host_hostname.set(String::new());
        host_user.set(String::new());
        host_port.set(String::new());
        host_identity.set(String::new());
        host_notes.set(String::new());
        host_auth_method.set("key".into());
        host_password.set(String::new());
        host_has_password.set(false);
        show_add_host.set(true);
    });
    let edit_ssh_host = Callback::new(move |alias: String| {
        let existing = ssh_hosts
            .get_untracked()
            .into_iter()
            .find(|host| host.alias == alias);
        host_alias.set(alias.clone());
        host_hostname.set(
            existing
                .as_ref()
                .and_then(|host| host.host_name.clone())
                .unwrap_or_default(),
        );
        host_user.set(
            existing
                .as_ref()
                .and_then(|host| host.user.clone())
                .unwrap_or_default(),
        );
        host_port.set(
            existing
                .as_ref()
                .and_then(|host| host.port)
                .map(|port| port.to_string())
                .unwrap_or_default(),
        );
        host_identity.set(
            existing
                .as_ref()
                .and_then(|host| host.identity_file.clone())
                .unwrap_or_default(),
        );
        host_notes.set(
            existing
                .as_ref()
                .and_then(|host| host.notes.clone())
                .unwrap_or_default(),
        );
        let auth_method = existing
            .as_ref()
            .and_then(|host| host.auth_method.clone())
            .unwrap_or_else(|| "key".into());
        host_auth_method.set(if auth_method == "password" {
            "password".into()
        } else {
            "key".into()
        });
        host_password.set(String::new());
        host_has_password.set(
            existing
                .as_ref()
                .map(|host| host.has_password)
                .unwrap_or(false),
        );
        editing_host_alias.set(Some(alias));
        ssh_connectivity_modal.set(None);
        ssh_connectivity_busy.set(false);
        open_settings_fn(Some("environments".into()));
        show_add_host.set(true);
    });

    let apply_session_compute_resource =
        Callback::new(move |(context_id, enabled): (String, bool)| {
            spawn_local(async move {
                let (session_id, created) = match active_session.get_untracked() {
                    Some(session_id) => (session_id, false),
                    None => match invoke_checked("new_session", JsValue::UNDEFINED)
                        .await
                        .ok()
                        .and_then(|value| value.as_string())
                    {
                        Some(session_id) => (session_id, true),
                        None => {
                            show_toast(&t(locale.get_untracked(), "status.send_failed"));
                            return;
                        }
                    },
                };
                let args = to_value(&serde_json::json!({
                    "sessionId": session_id.clone(),
                    "contextId": context_id,
                    "enabled": enabled,
                }))
                .unwrap();
                match invoke_checked("set_session_execution_context_enabled", args).await {
                    Ok(value) => {
                        let Ok(ids) = serde_wasm_bindgen::from_value::<Vec<String>>(value) else {
                            return;
                        };
                        if created && active_session.get_untracked().is_none() {
                            active_session.set(Some(session_id.clone()));
                            items.set(vec![]);
                            refresh_session_history();
                        }
                        if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                            session_execution_contexts.set(ids.into_iter().collect());
                        }
                    }
                    Err(error) => {
                        let message =
                            localize_backend(locale.get_untracked(), &js_error_text(error));
                        show_toast(&message);
                    }
                }
            });
        });

    let toggle_session_compute_resource =
        Callback::new(move |(context_id, enabled): (String, bool)| {
            if enabled {
                if let Some(ctx) = execution_contexts
                    .get_untracked()
                    .into_iter()
                    .find(|ctx| ctx.id == context_id)
                {
                    if let Some(detail) = ssh_connectivity_gap(&ctx) {
                        let label = if ctx.label.trim().is_empty() {
                            ctx.id.clone()
                        } else {
                            ctx.label.clone()
                        };
                        ssh_connectivity_modal.set(Some(SshConnectivityModal::from_gap(
                            context_id, label, detail, true,
                        )));
                        return;
                    }
                } else if context_id.starts_with("ssh:") {
                    // Context row may not be loaded yet — still require an explicit probe.
                    ssh_connectivity_modal.set(Some(SshConnectivityModal::need_confirm(
                        context_id.clone(),
                        context_id.clone(),
                        "not probed yet".into(),
                        true,
                    )));
                    return;
                }
            }
            apply_session_compute_resource.call((context_id, enabled));
        });

    let open_terminal_for_context = Callback::new(move |context_id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
            match invoke_checked("open_terminal", arg).await {
                Ok(value) => {
                    match serde_wasm_bindgen::from_value::<TerminalSessionSummary>(value) {
                        Ok(session) => {
                            let session_id = session.id.clone();
                            terminal_sessions.update(|sessions| {
                                if let Some(existing) =
                                    sessions.iter_mut().find(|item| item.id == session_id)
                                {
                                    *existing = session;
                                } else {
                                    sessions.push(session);
                                }
                            });
                            active_terminal_id.set(Some(session_id));
                            terminal_panel_open.set(true);
                            terminal_add_menu_open.set(false);
                        }
                        Err(error) => show_toast(&error.to_string()),
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    show_toast(&message);
                }
            }
        });
    });

    // Load persisted hosts once at startup.
    {
        let ssh_hosts = ssh_hosts;
        spawn_local(async move {
            let v = invoke("list_ssh_hosts", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                ssh_hosts.set(list);
            }
        });
    }
    refresh_execution_contexts(execution_contexts);
    // Auto-register installed WSL distributions so they show up as checkable
    // rows in the compute menu. No-op on non-Windows and (via a registry guard
    // in the backend) on Windows machines without WSL, so it never spawns
    // wsl.exe where there is nothing to detect.
    spawn_local(async move {
        let _ = invoke("import_wsl_contexts", JsValue::UNDEFINED).await;
        refresh_execution_contexts(execution_contexts);
    });
    refresh_runtimes(runtime_infos);
    refresh_runs(run_records, locale);
    {
        let ticks = Cell::new(0_u8);
        let refresh = Closure::wrap(Box::new(move || {
            let tick = (ticks.get() + 1) % 5;
            ticks.set(tick);
            let transfer_active = run_records.get_untracked().iter().any(|run| {
                matches!(run.status.as_str(), "submitted" | "running" | "cancelling")
                    && !run.progress_json.is_empty()
                    && run.progress_json != "{}"
            });
            if tick == 0 || busy.get_untracked() || transfer_active {
                refresh_runs(run_records, locale);
            }
        }) as Box<dyn FnMut()>);
        let _ = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    1_000,
                )
                .ok()
        });
        refresh.forget();
    }
    {
        let refresh = Closure::wrap(Box::new(move || {
            if show_right.get_untracked() && right_tab.get_untracked() == RightTab::Hosts {
                refresh_runtimes(runtime_infos);
            }
        }) as Box<dyn FnMut()>);
        let _ = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    1_000,
                )
                .ok()
        });
        refresh.forget();
    }
    {
        let refresh = Closure::wrap(Box::new(move || {
            if show_right.get_untracked() && right_tab.get_untracked() == RightTab::Agents {
                refresh_agent_workflows(agent_panel);
            }
        }) as Box<dyn FnMut()>);
        let _ = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    1_000,
                )
                .ok()
        });
        refresh.forget();
    }
    // Cross-project "needs you" inbox (#423): sessions across every project
    // that are waiting on the user, surfaced from any window's topbar.
    let inbox_open = create_rw_signal(false);
    let inbox_sessions = create_rw_signal::<Vec<SessionSearchInfo>>(vec![]);
    let refresh_inbox = move || {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "query": "", "limit": 50 })).unwrap();
            let v = invoke("search_sessions", arg).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(v) {
                inbox_sessions.set(
                    rows.into_iter()
                        .filter(|s| s.status == "needs_you")
                        .collect(),
                );
            }
        });
    };
    refresh_inbox();
    // Close on any click that bubbles to the window; the bell and the dropdown
    // stop propagation (same pattern as the titlebar menus — a fixed backdrop
    // would be clipped to the topbar, whose backdrop-filter contains it).
    window_event_listener(ev::click, move |_| {
        if inbox_open.get_untracked() {
            inbox_open.set(false);
        }
    });
    {
        // ponytail: 20s poll; switch to pushed session events if latency matters.
        let refresh = Closure::wrap(Box::new(refresh_inbox) as Box<dyn FnMut()>);
        let _ = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    20_000,
                )
                .ok()
        });
        refresh.forget();
    }
    create_effect(move |_| {
        if rename_session_target.get().is_some() {
            focus_and_select_soon("rename-session-input");
        }
    });
    create_effect(move |_| {
        if folder_modal.get().is_some() {
            focus_and_select_soon("folder-modal-input");
        }
    });
    create_effect(move |_| {
        if file_entry_modal.get().is_some() {
            focus_and_select_soon("file-entry-modal-input");
        }
    });
    create_effect(move |_| {
        if show_add_host.get() {
            focus_and_select_soon("add-host-alias");
        }
    });
    // Re-underline saved excerpts whenever the transcript or library changes.
    create_effect(move |_| {
        let _ = items.get();
        let texts = match active_session.get() {
            Some(session) => library_items
                .get()
                .iter()
                .filter(|item| item.kind == "text" && item.source_session_id == session)
                .map(|item| item.code.clone())
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        set_saved_marks(&serde_json::to_string(&texts).unwrap_or_default());
    });
    let open_session = load_session.clone();
    let on_ctx_pick = {
        let open_session = open_session.clone();
        let sessions = sessions;
        let rename_session_target = rename_session_target;
        let rename_session_input = rename_session_input;
        let session_transfer = session_transfer;
        let session_transfer_error = session_transfer_error;
        let project_info = project_info;
        let proj_list = proj_list;
        let folder_modal = folder_modal;
        let folder_modal_input = folder_modal_input;
        let ui_confirm = ui_confirm;
        let active_session = active_session;
        let artifacts = artifacts;
        let attachments = attachments;
        Callback::new(move |(action, payload): (String, String)| {
            if action == "quoteSelection" {
                let (source, text) = payload
                    .split_once('\u{1e}')
                    .unwrap_or(("", payload.as_str()));
                let source = (!source.is_empty()).then(|| source.to_string());
                composer_quotes.update(|items| {
                    items.push(ComposerQuote::from_selection(text, source.clone()))
                });
                clear_selection();
                if source.as_deref() == center_file.get_untracked().as_deref() {
                    center_split.set(true);
                    show_right.set(false);
                }
                focus_composer();
                return;
            }
            if action == "explainSelection" {
                let question = message_with_quotes(
                    &t(locale.get(), "selection.explain_prompt"),
                    &[ComposerQuote::plain(payload)],
                );
                clear_selection();
                send_side_chat(question);
                return;
            }
            if action == "downloadFile" {
                download_artifact(payload);
                return;
            }
            if action == "revealInFileManager" {
                reveal_in_file_manager(payload);
                return;
            }
            if action == "copyImage" {
                spawn_local(async move {
                    if context_menu::copy_image(&payload).await {
                        show_copy_toast();
                    }
                });
                return;
            }
            if action == "attachWorkspaceFile" {
                let _ = attach_ready_path(attachments, payload);
                focus_composer();
                return;
            }
            if action == "openWorkspaceFileCenter" {
                let tab = CenterFileTab::from_path(payload.clone());
                center_files.update(|files| {
                    if !files.iter().any(|file| file.path == payload) {
                        files.push(tab.clone());
                    }
                });
                center_file.set(Some(payload));
                return;
            }
            if action == "closeCenterCurrent" {
                if payload.starts_with("mcp-app:") {
                    close_mcp_app(&payload);
                    mcp_apps.update(|apps| {
                        apps.remove(&payload);
                    });
                }
                center_files.update(|files| files.retain(|file| file.path != payload));
                if center_file.get_untracked().as_ref() == Some(&payload) {
                    center_file.set(None);
                }
                return;
            }
            if action == "closeCenterRight" {
                let removed_apps = center_files.with_untracked(|files| {
                    files
                        .iter()
                        .position(|file| file.path == payload)
                        .map(|index| {
                            files[index + 1..]
                                .iter()
                                .filter(|file| file.path.starts_with("mcp-app:"))
                                .map(|file| file.path.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                });
                for instance_id in &removed_apps {
                    close_mcp_app(instance_id);
                }
                mcp_apps.update(|apps| {
                    for instance_id in &removed_apps {
                        apps.remove(instance_id);
                    }
                });
                center_files.update(|files| {
                    if let Some(index) = files.iter().position(|file| file.path == payload) {
                        files.truncate(index + 1);
                    }
                });
                if !center_files
                    .get_untracked()
                    .iter()
                    .any(|file| Some(&file.path) == center_file.get_untracked().as_ref())
                {
                    center_file.set(Some(payload));
                }
                return;
            }
            if action == "closeCenterAll" {
                let removed_apps = center_files.with_untracked(|files| {
                    files
                        .iter()
                        .filter(|file| file.path.starts_with("mcp-app:"))
                        .map(|file| file.path.clone())
                        .collect::<Vec<_>>()
                });
                for instance_id in &removed_apps {
                    close_mcp_app(instance_id);
                }
                mcp_apps.update(|apps| {
                    for instance_id in &removed_apps {
                        apps.remove(instance_id);
                    }
                });
                center_files.set(vec![]);
                center_file.set(None);
                return;
            }
            if action == "exportSession" {
                let session_id = if payload.is_empty() {
                    let Some(id) = active_session.get() else {
                        return;
                    };
                    id
                } else {
                    payload.clone()
                };
                let is_active = active_session.get().as_deref() == Some(session_id.as_str());
                let artifact_paths = if is_active {
                    artifacts
                        .get()
                        .into_iter()
                        .filter_map(|a| match a.data {
                            PreviewData::File { path, .. } => Some(path),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({
                        "sessionId": session_id,
                        "artifactPaths": artifact_paths,
                    }))
                    .unwrap();
                    let _ = invoke("export_session", arg).await;
                });
                return;
            }
            if action == "exportDebugRequest" {
                let session_id = if payload.is_empty() {
                    let Some(id) = active_session.get() else {
                        return;
                    };
                    id
                } else {
                    payload.clone()
                };
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
                    let _ = invoke("export_debug_request", arg).await;
                });
                return;
            }
            if let Some(act) = context_menu::folder_action(&action, &payload) {
                match act {
                    context_menu::FolderAction::Rename { id, name } => {
                        folder_modal_input.set(name);
                        folder_modal.set(Some(FolderModal::Rename(id)));
                    }
                    context_menu::FolderAction::Delete(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteFolder(id)));
                    }
                }
                return;
            }
            if let Some(act) = context_menu::workspace_entry_action(&action, &payload) {
                match act {
                    context_menu::WorkspaceEntryAction::Rename { path, is_dir } => {
                        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
                        file_entry_input.set(name);
                        file_entry_error.set(None);
                        file_entry_modal.set(Some(FileEntryModal::Rename { path, is_dir }));
                    }
                    context_menu::WorkspaceEntryAction::Delete { path, is_dir } => {
                        ui_confirm.set(Some(UiConfirm::DeleteFileEntry { path, is_dir }));
                    }
                }
                return;
            }
            if let Some(act) = context_menu::session_action(&action, &payload) {
                match act {
                    context_menu::SessionAction::Open(id) => open_session.call(id),
                    context_menu::SessionAction::Rename { id, title } => {
                        rename_session_input.set(title.clone());
                        rename_session_target.set(Some((id, title)));
                    }
                    context_menu::SessionAction::Move { id, folder_id } => {
                        spawn_local(async move {
                            let arg =
                                to_value(&serde_json::json!({ "id": id, "folderId": folder_id }))
                                    .unwrap();
                            if invoke_checked("move_session", arg).await.is_ok() {
                                refresh_session_history();
                            }
                        });
                    }
                    context_menu::SessionAction::Transfer { id, mode } => {
                        let title = sessions
                            .get()
                            .into_iter()
                            .find(|session| session.id == id)
                            .map(|session| session.title)
                            .unwrap_or_else(|| t(locale.get(), "sidebar.untitled").into());
                        session_transfer_error.set(None);
                        session_transfer.set(Some(SessionTransfer {
                            id,
                            title,
                            mode,
                            target_project_id: String::new(),
                        }));
                        let active_project_id = project_info
                            .get()
                            .map(|project| project.id)
                            .unwrap_or_default();
                        spawn_local(async move {
                            let value = invoke("list_projects", JsValue::UNDEFINED).await;
                            if let Ok(list) =
                                serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(value)
                            {
                                let default_target = list
                                    .iter()
                                    .find(|project| project.id != active_project_id)
                                    .map(|project| project.id.clone())
                                    .unwrap_or_default();
                                proj_list.set(list);
                                session_transfer.update(|transfer| {
                                    if let Some(transfer) = transfer {
                                        if transfer.target_project_id.is_empty() {
                                            transfer.target_project_id = default_target;
                                        }
                                    }
                                });
                            }
                        });
                    }
                    context_menu::SessionAction::SetPinned { id, pinned } => {
                        spawn_local(async move {
                            let arg =
                                to_value(&serde_json::json!({ "id": id, "pinned": pinned })).unwrap();
                            if invoke_checked("set_session_pinned", arg).await.is_ok() {
                                refresh_session_history();
                            }
                        });
                    }
                    context_menu::SessionAction::Delete(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteSession(id)));
                    }
                }
            }
            context_menu::run_action(&action, &payload, copy_text);
        })
    };
    let on_context_menu = move |ev: web_sys::MouseEvent| {
        let loc = locale.get();
        let center = center_file.get_untracked();
        if let Some(menu) =
            context_menu::build(&ev, loc, active_session.get().is_some(), center.as_deref())
        {
            if !menu.items.is_empty() {
                ev.prevent_default();
                // The context menu supersedes the selection popup — never
                // show both at once.
                selection_popup.set(None);
                ctx_menu.set(Some(menu));
                return;
            }
        }
        ctx_menu.set(None);
        if !context_menu::dev_mode() {
            ev.prevent_default();
        }
    };

    // Escape stack: topmost overlay → menus → drag cancel → right pane →
    // approval reject last. Composer @-mention and plan-feedback collapse
    // preventDefault locally so they win before this handler runs.
    // ProjectsScreen owns create/delete/search Escape while `show_projects`,
    // but app-level overlays (settings, artifact modal, onboarding) still
    // close here — they can sit on top of the projects landing.
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.key() != "Escape" || ev.default_prevented() || ime_composing(ev) {
            return;
        }
        if update_check_modal.get().is_some() {
            ev.prevent_default();
            update_check_modal.set(None);
            return;
        }
        if action_palette_open.get() {
            ev.prevent_default();
            action_palette_open.set(false);
            return;
        }
        if command_palette_open.get() {
            ev.prevent_default();
            command_palette_open.set(false);
            return;
        }

        // Overlays that can appear over the projects landing (must run before
        // the show_projects early-return below).
        if show_add_host.get() {
            ev.prevent_default();
            show_add_host.set(false);
            editing_host_alias.set(None);
            return;
        }
        if ssh_connectivity_modal.get().is_some() && !ssh_connectivity_busy.get() {
            ev.prevent_default();
            ssh_connectivity_modal.set(None);
            return;
        }
        if runtime_interpreter_form.get().is_some() {
            ev.prevent_default();
            runtime_interpreter_form.set(None);
            return;
        }
        if context_details_modal.get().is_some() {
            ev.prevent_default();
            context_details_modal.set(None);
            return;
        }
        if plugin_install_open.get() {
            ev.prevent_default();
            plugin_install_open.set(false);
            return;
        }
        // Confirm dialog sits on top of settings — close it first, not the page.
        if delete_confirm.get().is_some() {
            ev.prevent_default();
            delete_confirm.set(None);
            return;
        }
        if show_settings.get() && !settings_busy.get() {
            ev.prevent_default();
            show_settings.set(false);
            return;
        }
        if modal_artifact.get().is_some() {
            ev.prevent_default();
            modal_artifact.set(None);
            return;
        }
        if show_onboarding.get() {
            ev.prevent_default();
            if onboard_step.get() > 0 {
                onboard_step.update(|s| *s = s.saturating_sub(1));
            } else {
                dismiss_onboarding.call(());
            }
            return;
        }

        if show_projects.get() {
            return;
        }

        // --- overlays (most interrupting first) ---
        if ui_confirm.get().is_some() {
            ev.prevent_default();
            ui_confirm.set(None);
            return;
        }
        if rename_session_target.get().is_some() {
            ev.prevent_default();
            rename_session_target.set(None);
            return;
        }
        if session_transfer.get().is_some() && !session_transfer_busy.get() {
            ev.prevent_default();
            session_transfer.set(None);
            session_transfer_error.set(None);
            return;
        }
        if folder_modal.get().is_some() {
            ev.prevent_default();
            folder_modal.set(None);
            return;
        }
        if file_entry_modal.get().is_some() && !file_entry_busy.get() {
            ev.prevent_default();
            file_entry_modal.set(None);
            file_entry_error.set(None);
            return;
        }
        if show_proj_settings.get() && !proj_settings_busy.get() {
            ev.prevent_default();
            show_proj_settings.set(false);
            return;
        }
        if show_capabilities.get() {
            ev.prevent_default();
            show_capabilities.set(false);
            return;
        }

        // --- menus / popovers ---
        if ctx_menu.get().is_some() {
            ev.prevent_default();
            ctx_menu.set(None);
            return;
        }
        if artifact_menu.get().is_some() {
            ev.prevent_default();
            artifact_menu.set(None);
            return;
        }
        if show_proj_menu.get() {
            ev.prevent_default();
            show_proj_menu.set(false);
            return;
        }
        if compose_menu_open.get() {
            ev.prevent_default();
            compose_menu_open.set(false);
            return;
        }
        if agent_menu_open.get() {
            ev.prevent_default();
            agent_menu_open.set(false);
            reviewer_model_menu_open.set(false);
            compute_menu_open.set(false);
            specialist_menu_open.set(false);
            return;
        }
        if model_menu_open.get() {
            ev.prevent_default();
            model_menu_open.set(false);
            return;
        }
        if acp_config_menu_open.get().is_some() {
            ev.prevent_default();
            acp_config_menu_open.set(None);
            return;
        }
        if send_mode_menu_open.get() {
            ev.prevent_default();
            send_mode_menu_open.set(false);
            return;
        }
        if right_tab_add_menu_open.get() {
            ev.prevent_default();
            right_tab_add_menu_open.set(false);
            return;
        }
        if side_chat_model_menu_open.get() {
            ev.prevent_default();
            side_chat_model_menu_open.set(false);
            return;
        }
        if runtime_environment_pinned.get() {
            ev.prevent_default();
            runtime_environment.set(None);
            runtime_environment_pinned.set(false);
            return;
        }

        // --- drag cancel ---
        if dragging.get() {
            ev.prevent_default();
            dragging.set(false);
            return;
        }
        if composer_dragging.get() {
            ev.prevent_default();
            composer_dragging.set(false);
            return;
        }

        // --- right pane ---
        // Close regardless of focus: mention/skill pickers already preventDefault
        // Escape locally, so they still win when open.
        if show_right.get() {
            ev.prevent_default();
            show_right.set(false);
            return;
        }

        // --- approval reject last ---
        if active_session.get().is_some_and(|_sid| {
            items
                .get()
                .iter()
                .any(|i| matches!(i, ChatItem::ApprovalPending { .. }))
        }) {
            ev.prevent_default();
            if let Some(sid) = active_session.get() {
                respond_confirm.call((sid, false, None, "once".into()));
            }
        }
    });

    // External links (http/https/mailto/tel) must open in the system browser,
    // never navigate the app's own webview away from the UI (no way back —
    // issue #97). Chat markdown routes clicks through `handle_md_click`, which
    // stop_propagation's before the event reaches here; markdown rendered
    // elsewhere (file preview, right pane, review) has no per-element handler,
    // so this window-level catch covers every render path.
    window_event_listener(ev::click, move |ev| {
        use wasm_bindgen::JsCast;
        if ev.default_prevented() {
            return;
        }
        let mut el = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        while let Some(n) = el {
            if n.tag_name().eq_ignore_ascii_case("a") {
                if let Some(href) = n.get_attribute("href") {
                    if opens_in_system_browser(&href) {
                        ev.prevent_default();
                        open_external_url(href);
                    }
                }
                return;
            }
            el = n.parent_element();
        }
    });

    // Selecting text inside any file preview (tagged `data-file-path`) raises the
    // same quote popup the chat uses, plus an "annotate" action. Runs on window
    // so it covers the center preview, the artifact modal, and the right pane
    // uniformly. Fires after the chat's own element handler during bubbling, so
    // it only clears/replaces a *preview* popup (source == Some) and never stomps
    // a chat selection popup.
    window_event_listener(ev::mouseup, move |ev| {
        use wasm_bindgen::JsCast;
        // Primary button only — right-click has its own context menu — and
        // gated on the "selection quick actions" setting.
        if ev.button() != 0 || !selection_popup_enabled.get_untracked() {
            return;
        }
        // Clicking a popup button is itself a mouseup — ignore it so it can't
        // re-capture the selection and race the button's own click handler.
        let in_popup = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|el| el.closest(".selection-popup").ok().flatten())
            .is_some();
        if in_popup {
            return;
        }
        let json = preview_selection();
        if json.is_empty() {
            if matches!(selection_popup.get_untracked(), Some((_, Some(_), _, _))) {
                selection_popup.set(None);
            }
            return;
        }
        if let Ok(sel) = serde_json::from_str::<PreviewSelection>(&json) {
            if !sel.text.trim().is_empty() {
                selection_popup.set(Some((sel.text, Some(sel.path), sel.x, sel.y)));
            }
        }
    });

    // A cropped image region fires this only after the user chooses one of the
    // preview popup actions. The jump action also exits either preview surface.
    window_event_listener_untyped("wisp:region-attach", move |ev| {
        use wasm_bindgen::JsCast;
        let Some(detail) = ev
            .dyn_ref::<web_sys::CustomEvent>()
            .and_then(|ce| serde_wasm_bindgen::from_value::<RegionAttach>(ce.detail()).ok())
        else {
            return;
        };
        attach_ready_path(attachments, detail.path);
        if detail.jump_to_chat {
            modal_artifact.set(None);
            center_file.set(None);
            focus_composer();
        }
    });

    // Dismiss the selection popup on any press outside it: starting a new
    // selection, clicking the composer, or clicking elsewhere in the app.
    window_event_listener(ev::mousedown, move |ev| {
        use wasm_bindgen::JsCast;
        if selection_popup.get_untracked().is_none() {
            return;
        }
        let inside = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|el| el.closest(".selection-popup").ok().flatten())
            .is_some();
        if !inside {
            selection_popup.set(None);
        }
    });

    // --- Top-nav project switcher + Project Settings ---
    // Every project-open entry point shares one epoch/target guard and one
    // serialized gate. A rapid A -> B switch can therefore never let A's late
    // response load a session, refresh lists, or publish project metadata after
    // B has become the requested target.
    let open_project_transition = {
        let transition_epoch = project_transition_epoch.clone();
        let transition_target = project_transition_target.clone();
        let open_gate = project_open_gate.clone();
        let load_session = load_session.clone();
        let app_shell_entering = app_shell_entering;
        Callback::new(move |(project_id, session_id): (String, Option<String>)| {
            let request_epoch = transition_epoch.get().wrapping_add(1);
            transition_epoch.set(request_epoch);
            *transition_target.borrow_mut() = Some(project_id.clone());

            project_open_error.set(None);
            status.set(String::new());
            show_proj_menu.set(false);
            demo_mode.set(false);
            // Stash the transcript we're leaving, like every other switch path —
            // dropping it made running sessions "roll back" on return (#194).
            if let Some(old) = active_session.get() {
                transcripts.update(|m| {
                    m.insert(old, items.get());
                });
            }
            items.set(vec![]);
            active_session.set(None);
            collapsed_folders.set(HashSet::new());
            project_info.set(None);
            app_shell_entering.set(true);
            {
                let transition_epoch = transition_epoch.clone();
                let app_shell_entering = app_shell_entering;
                set_timeout(
                    move || {
                        if transition_epoch.get() == request_epoch {
                            app_shell_entering.set(false);
                        }
                    },
                    std::time::Duration::from_millis(520),
                );
            }
            show_projects.set(false);

            let transition_epoch = transition_epoch.clone();
            let transition_target = transition_target.clone();
            let open_gate = open_gate.clone();
            let load_session = load_session.clone();
            spawn_local(async move {
                let _permit = acquire_project_open_gate(open_gate).await;
                if !project_transition_is_current(
                    &transition_epoch,
                    &transition_target,
                    request_epoch,
                    &project_id,
                ) {
                    return;
                }

                let args = to_value(&serde_json::json!({ "id": project_id.clone() })).unwrap();
                let open_result = invoke_checked("open_project", args).await;
                if !project_transition_is_current(
                    &transition_epoch,
                    &transition_target,
                    request_epoch,
                    &project_id,
                ) {
                    return;
                }

                let project_result = match open_result {
                    Ok(_) => invoke_checked("get_project_info", JsValue::UNDEFINED).await,
                    Err(error) => Err(error),
                };
                if !project_transition_is_current(
                    &transition_epoch,
                    &transition_target,
                    request_epoch,
                    &project_id,
                ) {
                    return;
                }

                let result = project_result
                    .map_err(js_error_text)
                    .and_then(|value| {
                        serde_wasm_bindgen::from_value::<ProjectInfo>(value)
                            .map_err(|_| "The project returned invalid metadata.".to_string())
                    })
                    .and_then(|project| {
                        if project.id == project_id {
                            Ok(project)
                        } else {
                            Err(format!(
                                "The project response did not match the requested project ({project_id})."
                            ))
                        }
                    });

                let project = match result {
                    Ok(project) => project,
                    Err(raw_error) => {
                        let loc = locale.get_untracked();
                        let detail = localize_backend(loc, &raw_error);
                        let message = tf(loc, "projects.open_failed", &[("msg", &detail)]);
                        project_open_error.set(Some(message.clone()));
                        status.set(message);
                        project_info.set(None);
                        *transition_target.borrow_mut() = None;
                        show_projects.set(true);
                        return;
                    }
                };

                // No await occurs after this final guard, so a newer transition
                // cannot interleave before these current-project actions run.
                if !project_transition_is_current(
                    &transition_epoch,
                    &transition_target,
                    request_epoch,
                    &project_id,
                ) {
                    return;
                }
                project_info.set(Some(project));
                if let Some(session_id) = session_id {
                    load_session.call(session_id);
                }
                refresh_session_history();
                refresh_folders(folders);
            });
        })
    };
    // Sent by the pet (to "main") and by `open_project_window` targeting a
    // session in an already-open project window (#423).
    let event_open_project = open_project_transition;
    let open_session_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(target) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) else {
            return;
        };
        let Some(project_id) = target.get("projectId").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(session_id) = target.get("sessionId").and_then(serde_json::Value::as_str) else {
            return;
        };
        event_open_project.call((project_id.to_string(), Some(session_id.to_string())));
    }) as Box<dyn FnMut(JsValue)>);
    let open_session_js = open_session_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    open_session_cb.forget();
    spawn_local(async move {
        let _ = listen("open-session", &open_session_js).await;
    });
    // Cross-project opens from inside a workspace go to the target project's
    // own window (#423). The landing (and a window with no project yet) keeps
    // repointing the current window — it's the "enter a project here" surface.
    let opens_in_project_window = move |project_id: &str| -> bool {
        !show_projects.get_untracked()
            && matches!(project_info.get_untracked(), Some(p) if p.id != project_id)
    };
    // Switch the active project inline (same guarded flow as the Projects screen).
    let switch_project = {
        let open_project_transition = open_project_transition;
        Callback::new(move |id: String| {
            if opens_in_project_window(&id) {
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                    let _ = invoke("open_project_window", arg).await;
                });
                return;
            }
            open_project_transition.call((id, None));
        })
    };
    // Dedicated project window (#52): enter through the same serialized,
    // target-validated transition instead of maintaining a second startup path.
    // `&session=` (#423) drops the window straight into the requested session.
    if let Some(project_id) = dedicated_project_id {
        open_project_transition.call((project_id, url_session_param()));
    }
    let toggle_proj_menu = move |_| {
        let opening = !show_proj_menu.get();
        show_proj_menu.set(opening);
        if opening {
            spawn_local(async move {
                let v = invoke("list_projects", JsValue::UNDEFINED).await;
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                    proj_list.set(list);
                }
            });
        }
    };
    let open_proj_settings = move |_| {
        show_proj_menu.set(false);
        spawn_local(async move {
            let v = invoke("get_project_settings", JsValue::UNDEFINED).await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<ProjectSettings>(v) {
                proj_settings.set(s);
                show_proj_settings.set(true);
            }
        });
    };
    let save_proj_settings = move |_| {
        if proj_settings_busy.get() {
            return;
        }
        let form = proj_settings.get();
        if form.name.trim().is_empty() {
            return;
        }
        proj_settings_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": form.name, "description": form.description, "agentContext": form.agent_context,
            })).unwrap();
            let res = invoke_checked("update_project", arg).await;
            proj_settings_busy.set(false);
            if res.is_ok() {
                show_proj_settings.set(false);
                let v = invoke("get_project_info", JsValue::UNDEFINED).await;
                if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                    project_info.set(Some(p));
                }
            }
        });
    };

    let move_session_to = {
        Callback::new(move |(session_id, folder_id): (String, Option<String>)| {
            spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "id": session_id, "folderId": folder_id }))
                    .unwrap();
                if invoke_checked("move_session", arg).await.is_ok() {
                    refresh_session_history();
                }
            });
        })
    };

    let new_folder = move |_| {
        folder_modal_input.set(String::new());
        folder_modal.set(Some(FolderModal::Create));
    };

    let save_folder_modal = {
        let folders = folders;
        move |mode: FolderModal| {
            let name = folder_modal_input.get().trim().to_string();
            if name.is_empty() {
                return;
            }
            folder_modal.set(None);
            match mode {
                FolderModal::Create => spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "name": name })).unwrap();
                    if invoke_checked("create_folder", arg).await.is_ok() {
                        refresh_folders(folders);
                    }
                }),
                FolderModal::Rename(id) => spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": id, "name": name })).unwrap();
                    if invoke_checked("rename_folder", arg).await.is_ok() {
                        refresh_folders(folders);
                    }
                }),
            }
        }
    };

    let save_file_entry_modal = Callback::new(move |mode: FileEntryModal| {
        if file_entry_busy.get_untracked() {
            return;
        }
        let name = file_entry_input.get_untracked().trim().to_string();
        if name.is_empty()
            || matches!(name.as_str(), "." | "..")
            || name.contains(['/', '\\', '\0'])
        {
            file_entry_error.set(Some(
                t(locale.get_untracked(), "files.invalid_name").to_string(),
            ));
            return;
        }

        let (command, args, rename) = match &mode {
            FileEntryModal::CreateFile => {
                let path = join_path(&file_cwd.get_untracked(), &name);
                ("create_file", serde_json::json!({ "path": path }), None)
            }
            FileEntryModal::CreateDirectory => {
                let path = join_path(&file_cwd.get_untracked(), &name);
                (
                    "create_directory",
                    serde_json::json!({ "path": path }),
                    None,
                )
            }
            FileEntryModal::Rename { path, is_dir } => {
                let new_path = join_path(&parent_path(path), &name);
                (
                    "rename_entry",
                    serde_json::json!({ "path": path, "newPath": new_path }),
                    Some((path.clone(), new_path, *is_dir)),
                )
            }
        };

        file_entry_busy.set(true);
        file_entry_error.set(None);
        spawn_local(async move {
            let result = invoke_checked(command, to_value(&args).unwrap()).await;
            file_entry_busy.set(false);
            match result {
                Ok(_) => {
                    if let Some((old_path, new_path, is_dir)) = rename {
                        let old_prefix = format!("{old_path}/");
                        center_files.update(|files| {
                            for file in files.iter_mut() {
                                let renamed = if file.path == old_path {
                                    Some(new_path.clone())
                                } else if is_dir {
                                    file.path
                                        .strip_prefix(&old_prefix)
                                        .map(|suffix| format!("{new_path}/{suffix}"))
                                } else {
                                    None
                                };
                                if let Some(path) = renamed {
                                    *file = CenterFileTab::from_path(path);
                                }
                            }
                        });
                        center_file.update(|active| {
                            let Some(path) = active.as_ref() else {
                                return;
                            };
                            if path == &old_path {
                                *active = Some(new_path.clone());
                            } else if is_dir {
                                if let Some(suffix) = path.strip_prefix(&old_prefix) {
                                    *active = Some(format!("{new_path}/{suffix}"));
                                }
                            }
                        });
                    }
                    file_entry_modal.set(None);
                    file_entry_input.set(String::new());
                    refresh_dir(file_cwd, file_entries);
                    if !file_query.get_untracked().trim().is_empty() {
                        refresh_file_search(file_query, file_search_hits);
                    }
                }
                Err(error) => file_entry_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ))),
            }
        });
    });

    let save_session_transfer = move |_| {
        let Some(transfer) = session_transfer.get() else {
            return;
        };
        if transfer.target_project_id.is_empty() || session_transfer_busy.get() {
            return;
        }
        let target_name = proj_list
            .get()
            .into_iter()
            .find(|project| project.id == transfer.target_project_id)
            .map(|project| project.name)
            .unwrap_or_else(|| transfer.target_project_id.clone());
        session_transfer_busy.set(true);
        session_transfer_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "id": transfer.id,
                "targetProjectId": transfer.target_project_id,
                "mode": transfer.mode.as_str(),
            }))
            .unwrap();
            match invoke_checked("transfer_session_to_project", args).await {
                Ok(_) => {
                    if transfer.mode == SessionTransferMode::Move {
                        transcripts.update(|saved| {
                            saved.remove(&transfer.id);
                        });
                        running.update(|ids| {
                            ids.remove(&transfer.id);
                        });
                        pending_turns.update(|turns| {
                            turns.remove(&transfer.id);
                        });
                        if active_session.get().as_deref() == Some(transfer.id.as_str()) {
                            active_session.set(None);
                            items.set(vec![]);
                        }
                    }
                    refresh_session_history();
                    let message_key = if transfer.mode == SessionTransferMode::Copy {
                        "session.copy_success"
                    } else {
                        "session.move_success"
                    };
                    status.set(tf(locale.get(), message_key, &[("project", &target_name)]));
                    session_transfer.set(None);
                }
                Err(error) => {
                    session_transfer_error
                        .set(Some(localize_backend(locale.get(), &js_error_text(error))));
                }
            }
            session_transfer_busy.set(false);
        });
    };

    let palette_open_session = {
        let open_project_transition = open_project_transition;
        Callback::new(move |(project_id, session_id): (String, String)| {
            if opens_in_project_window(&project_id) {
                spawn_local(async move {
                    let arg =
                        to_value(&serde_json::json!({ "id": project_id, "session": session_id }))
                            .unwrap();
                    let _ = invoke("open_project_window", arg).await;
                });
                return;
            }
            open_project_transition.call((project_id, Some(session_id)));
        })
    };
    let command_palette_open_project = {
        let open_project_transition = open_project_transition;
        Callback::new(move |(project_id, new_window): (String, bool)| {
            if new_window {
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": project_id })).unwrap();
                    let _ = invoke("open_project_window", arg).await;
                });
            } else {
                open_project_transition.call((project_id, None));
            }
        })
    };
    let command_palette_open_session = {
        let open_project_transition = open_project_transition;
        Callback::new(
            move |(project_id, session_id, new_window): (String, String, bool)| {
                if new_window {
                    spawn_local(async move {
                        let arg = to_value(
                            &serde_json::json!({ "id": project_id, "session": session_id }),
                        )
                        .unwrap();
                        let _ = invoke("open_project_window", arg).await;
                    });
                } else {
                    open_project_transition.call((project_id, Some(session_id)));
                }
            },
        )
    };
    let palette_open_artifact =
        Callback::new(move |(path, name, kind): (String, String, String)| {
            modal_artifact.set(Some((path, name, kind)));
        });
    let palette_new_session = Callback::new(move |_: ()| {
        demo_mode.set(false);
        if let Some(old) = active_session.get() {
            transcripts.update(|m| {
                m.insert(old, items.get());
            });
        }
        attachments.set(vec![]);
        composer_references.set(vec![]);
        composer_quotes.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let Some(id) = invoke("new_session", JsValue::UNDEFINED).await.as_string() else {
                status.set(t(locale.get(), "status.send_failed").into());
                return;
            };
            active_session.set(Some(id));
            items.set(vec![]);
            refresh_session_history();
            focus_composer();
        });
    });
    let palette_project_settings = Callback::new(move |_: ()| {
        spawn_local(async move {
            let v = invoke("get_project_settings", JsValue::UNDEFINED).await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<ProjectSettings>(v) {
                proj_settings.set(s);
                show_proj_settings.set(true);
            }
        });
    });
    let palette_manage_skills = Callback::new(move |_: ()| {
        show_settings.set(true);
        settings_section.set("skills".into());
        spawn_local(async move {
            let v = invoke("list_skills", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                skills_list.set(rows);
            }
        });
    });
    let export_current_project = Callback::new(move |_: ()| {
        if show_projects.get_untracked() || demo_mode.get_untracked() {
            return;
        }
        let Some(id) = project_info.get_untracked().map(|project| project.id) else {
            return;
        };
        project_open_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id })).unwrap();
            if let Err(error) = invoke_checked("export_project", args).await {
                let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                status.set(message.clone());
                project_open_error.set(Some(message));
            }
        });
    });
    let palette_attach = Callback::new(move |reference: ComposerReferenceChip| {
        if !composer_references
            .get()
            .iter()
            .any(|item| item.key() == reference.key())
        {
            composer_references.update(|items| items.push(reference));
        }
    });
    let palette_action = {
        let new_session = palette_new_session.clone();
        let project_settings = palette_project_settings.clone();
        let manage_skills = palette_manage_skills.clone();
        let run_update_check = run_update_check.clone();
        let export_current_project = export_current_project.clone();
        Callback::new(move |action: &'static str| match action {
            "new" => new_session.call(()),
            "search" => command_palette_open.set(true),
            "commands" => action_palette_open.set(true),
            "projects" => show_projects.set(true),
            "settings" => {
                show_settings.set(true);
                settings_section.set("models".into());
            }
            "project-settings" => project_settings.call(()),
            "export-current-project" => export_current_project.call(()),
            "skills" => manage_skills.call(()),
            "check-updates" => run_update_check(),
            "docs" => open_external_url("https://github.com/xuzhougeng/wisp-science#readme".into()),
            "star-us" => open_external_url("https://github.com/xuzhougeng/wisp-science".into()),
            "issues" => {
                open_external_url("https://github.com/xuzhougeng/wisp-science/issues".into())
            }
            "toggle-sidebar" => show_sidebar.update(|show| *show = !*show),
            "artifacts" => {
                ensure_right_tab(RightTab::Artifacts, show_right, open_right_tabs, right_tab)
            }
            "notebook" => {
                ensure_right_tab(RightTab::Notebook, show_right, open_right_tabs, right_tab)
            }
            "files" => {
                ensure_right_tab(RightTab::File, show_right, open_right_tabs, right_tab);
                refresh_active_file_dir(
                    file_source,
                    file_cwd,
                    file_entries,
                    remote_file_cwd,
                    remote_file_entries,
                    remote_file_loading,
                    remote_file_error,
                );
            }
            "provenance" => {
                ensure_right_tab(RightTab::Provenance, show_right, open_right_tabs, right_tab)
            }
            "contexts" => {
                ensure_right_tab(RightTab::Hosts, show_right, open_right_tabs, right_tab);
                refresh_execution_contexts(execution_contexts);
                refresh_runtimes(runtime_infos);
                refresh_runs(run_records, locale);
            }
            "side-chat" => {
                ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab)
            }
            "close-panel" => show_right.set(false),
            "theme-light" => theme_mode.set("light".into()),
            "theme-dark" => theme_mode.set("dark".into()),
            "theme-system" => theme_mode.set("system".into()),
            _ => {}
        })
    };
    {
        let palette_action = palette_action.clone();
        let run_update_check = run_update_check.clone();
        let native_menu_cb = Closure::wrap(Box::new(move |payload: JsValue| {
            let Some(action) = payload.as_string() else {
                return;
            };
            match action.as_str() {
                "check-updates" => run_update_check(),
                "docs" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science#readme".into())
                }
                "star-us" => open_external_url("https://github.com/xuzhougeng/wisp-science".into()),
                "issues" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science/issues".into())
                }
                other => {
                    if let Some(action) = match other {
                        "new" => Some("new"),
                        "search" => Some("search"),
                        "commands" => Some("commands"),
                        "projects" => Some("projects"),
                        "settings" => Some("settings"),
                        "project-settings" => Some("project-settings"),
                        "export-current-project" => Some("export-current-project"),
                        "skills" => Some("skills"),
                        "toggle-sidebar" => Some("toggle-sidebar"),
                        "artifacts" => Some("artifacts"),
                        "notebook" => Some("notebook"),
                        "files" => Some("files"),
                        "provenance" => Some("provenance"),
                        "contexts" => Some("contexts"),
                        "side-chat" => Some("side-chat"),
                        "close-panel" => Some("close-panel"),
                        "theme-light" => Some("theme-light"),
                        "theme-dark" => Some("theme-dark"),
                        "theme-system" => Some("theme-system"),
                        _ => None,
                    } {
                        palette_action.call(action);
                    }
                }
            }
        }) as Box<dyn FnMut(JsValue)>);
        let native_menu_js = native_menu_cb
            .as_ref()
            .unchecked_ref::<js_sys::Function>()
            .clone();
        native_menu_cb.forget();
        spawn_local(async move {
            let _ = listen("native-menu-action", &native_menu_js).await;
        });
    }
    let palette_project_id = Signal::derive(move || project_info.get().map(|p| p.id));
    let has_current_project = Signal::derive(move || {
        project_info.get().is_some() && !show_projects.get() && !demo_mode.get()
    });
    let shortcut_action = palette_action.clone();
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ime_composing(ev) || !(ev.ctrl_key() || ev.meta_key()) {
            return;
        }
        let key = ev.key().to_lowercase();
        match key.as_str() {
            "p" => {
                ev.prevent_default();
                command_palette_open.set(false);
                action_palette_open.update(|open| *open = !*open);
            }
            "k" => {
                ev.prevent_default();
                action_palette_open.set(false);
                command_palette_open.update(|open| *open = !*open);
            }
            "n" => {
                ev.prevent_default();
                shortcut_action.call("new");
            }
            "b" => {
                ev.prevent_default();
                shortcut_action.call("toggle-sidebar");
            }
            "," => {
                ev.prevent_default();
                shortcut_action.call("settings");
            }
            _ => {}
        }
    });
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.default_prevented()
            || ime_composing(ev)
            || ev.alt_key()
            || ev.ctrl_key()
            || ev.meta_key()
            || ev.shift_key()
            || keyboard_event_targets_text_entry(ev)
        {
            return;
        }
        let Some((path, _, kind)) = modal_artifact.get() else {
            return;
        };
        let (prev_artifact, next_artifact) =
            modal_image_nav_targets(&artifacts.get(), &path, &kind);
        match ev.key().as_str() {
            "ArrowLeft" => {
                let Some((path, name, kind)) = prev_artifact else {
                    return;
                };
                ev.prevent_default();
                modal_artifact.set(Some((path, name, kind)));
            }
            "ArrowRight" => {
                let Some((path, name, kind)) = next_artifact else {
                    return;
                };
                ev.prevent_default();
                modal_artifact.set(Some((path, name, kind)));
            }
            _ => {}
        }
    });

    view! {
        {is_windows().then(|| view! {
            <WindowTitlebar locale=locale has_current_project=has_current_project
                on_action=palette_action.clone() />
        })}
        <ActionPalette open=action_palette_open on_action=palette_action />
        <CommandPalette open=command_palette_open current_project_id=palette_project_id
            on_open_project=command_palette_open_project on_open_session=command_palette_open_session on_open_artifact=palette_open_artifact
            on_command=palette_action
            on_new_session=palette_new_session on_project_settings=palette_project_settings
            on_manage_skills=palette_manage_skills on_attach=palette_attach />
        <ProjectLanding
            state=ProjectLandingState {
                show_projects, demo_mode, items, active_session, project_open_error,
                demos, modal_artifact, locale, running, approval_pending,
                command_palette_open,
            }
            open_project=switch_project
            open_project_session=palette_open_session
            open_settings=Callback::new(move |section: Option<String>| open_settings_fn(section))
            open_library=Callback::new(move |_| show_library.set(true))
        />
        {move || show_library.get().then(|| view! {
            <LibraryScreen
                locale=locale.read_only()
                items=library_items.read_only()
                on_close=Callback::new(move |_| show_library.set(false))
                on_open_source=palette_open_session
                on_changed=refresh_library_items
            />
        })}
        {move || ssh_connectivity_modal.get().map(|modal| {
            let host = modal.label.clone();
            let raw_detail = modal.detail.clone();
            let context_id = modal.context_id.clone();
            let enable_after = modal.enable_after_probe;
            let failed = modal.phase == SshCheckPhase::Failed;
            let fail_kind = classify_ssh_failure(&raw_detail);
            let loc = locale.get();
            let detail = localize_backend(loc, &raw_detail);
            let title = if failed {
                match fail_kind {
                    SshFailKind::ProbeOutput => t(loc, "ssh_check.probe_output_title"),
                    SshFailKind::PasswordAuth => t(loc, "ssh_check.password_title"),
                    SshFailKind::KeyAuth => t(loc, "ssh_check.key_title"),
                    _ => t(loc, "ssh_check.fail_title"),
                }
            } else {
                t(loc, "ssh_check.title")
            };
            let body = if failed {
                let key = match fail_kind {
                    SshFailKind::ProbeOutput => "ssh_check.probe_output_body",
                    SshFailKind::PasswordAuth => "ssh_check.password_body",
                    SshFailKind::KeyAuth => "ssh_check.key_body",
                    _ => "ssh_check.fail_body",
                };
                tf(loc, key, &[("host", &host)])
            } else {
                tf(loc, "ssh_check.body", &[("host", &host)])
            };
            let detail_line = tf(loc, "ssh_check.detail", &[("detail", &detail)]);
            let cause_keys = ssh_fail_cause_keys(fail_kind);
            let host_for_probe = host.clone();
            let run_probe = Rc::new({
                let context_id = context_id.clone();
                move || {
                    let context_id = context_id.clone();
                    let host_for_probe = host_for_probe.clone();
                    ssh_connectivity_busy.set(true);
                    spawn_local(async move {
                        let arg =
                            to_value(&serde_json::json!({ "contextId": context_id.clone() }))
                                .unwrap();
                        match invoke_checked("probe_execution_context", arg).await {
                            Ok(value) => {
                                show_probe_stopped_toast(&value, locale);
                                refresh_execution_contexts(execution_contexts);
                                let Ok(updated) =
                                    serde_wasm_bindgen::from_value::<ExecutionContext>(value)
                                else {
                                    ssh_connectivity_busy.set(false);
                                    return;
                                };
                                if ssh_context_known_good(&updated) {
                                    if enable_after {
                                        apply_session_compute_resource
                                            .call((context_id.clone(), true));
                                        show_toast(&t(
                                            locale.get_untracked(),
                                            "ssh_check.enabled",
                                        ));
                                    } else {
                                        show_toast(&t(
                                            locale.get_untracked(),
                                            "ssh_check.probed_ok",
                                        ));
                                    }
                                    if file_source.get_untracked() == context_id {
                                        refresh_remote_dir(
                                            context_id.clone(),
                                            remote_file_cwd,
                                            remote_file_entries,
                                            remote_file_loading,
                                            remote_file_error,
                                            file_source,
                                        );
                                    }
                                    ssh_connectivity_modal.set(None);
                                } else {
                                    // Stop probing loop: switch to diagnosis + fix.
                                    let detail = ssh_connectivity_gap(&updated)
                                        .unwrap_or_else(|| "probe failed".into());
                                    let label = if updated.label.trim().is_empty() {
                                        updated.id.clone()
                                    } else {
                                        updated.label.clone()
                                    };
                                    ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                        context_id.clone(),
                                        label,
                                        detail,
                                        enable_after,
                                    )));
                                    show_warning_toast(&t(
                                        locale.get_untracked(),
                                        "ssh_check.still_failed",
                                    ));
                                }
                            }
                            Err(error) => {
                                let message = localize_backend(
                                    locale.get_untracked(),
                                    &js_error_text(error),
                                );
                                show_toast(&message);
                                ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                    context_id.clone(),
                                    host_for_probe,
                                    message,
                                    enable_after,
                                )));
                            }
                        }
                        ssh_connectivity_busy.set(false);
                    });
                }
            });
            let open_edit_host = Rc::new({
                let context_id = context_id.clone();
                move || {
                    let alias = context_id
                        .strip_prefix("ssh:")
                        .unwrap_or(context_id.as_str())
                        .to_string();
                    edit_ssh_host.call(alias);
                }
            });
            view! {
                <div class="overlay" data-testid="ssh-connectivity-modal">
                    <div class="modal confirm-modal update-check-modal ssh-check-modal"
                        class:ssh-check-failed=failed role="dialog" aria-modal="true">
                        <h2>{title}</h2>
                        <div class="hint ssh-check-scroll">
                            <p>{body}</p>
                            <p class="ssh-check-error">{detail_line}</p>
                            {failed.then(|| view! {
                                <div class="ssh-check-causes" data-testid="ssh-check-causes">
                                    <div class="ssh-check-causes-title">
                                        {t(loc, "ssh_check.causes_title")}
                                    </div>
                                    <ul>
                                        {cause_keys.iter().map(|key| view! {
                                            <li>{t(loc, key)}</li>
                                        }).collect_view()}
                                    </ul>
                                </div>
                            })}
                            {(!failed).then(|| view! {
                                <p>{t(loc, "ssh_check.hint")}</p>
                            })}
                        </div>
                        <div class="row ssh-check-actions">
                            <button
                                type="button"
                                prop:disabled=move || ssh_connectivity_busy.get()
                                on:click=move |_| {
                                    ssh_connectivity_modal.set(None);
                                    ssh_connectivity_busy.set(false);
                                }
                            >
                                {t(loc, "ssh_check.cancel")}
                            </button>
                            {if failed {
                                let edit = open_edit_host.clone();
                                let reprobe = run_probe.clone();
                                view! {
                                    <button
                                        type="button"
                                        data-testid="ssh-connectivity-settings"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| {
                                            ssh_connectivity_modal.set(None);
                                            open_settings_fn(Some("environments".into()));
                                        }
                                    >
                                        {t(loc, "ssh_check.jump")}
                                    </button>
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="ssh-connectivity-fix-host"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| edit()
                                    >
                                        {t(loc, "ssh_check.fix_host")}
                                    </button>
                                    <button
                                        type="button"
                                        data-testid="ssh-connectivity-reprobe"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| reprobe()
                                    >
                                        {move || if ssh_connectivity_busy.get() {
                                            t(locale.get(), "ssh_check.probing")
                                        } else {
                                            t(locale.get(), "ssh_check.reprobe_after_fix")
                                        }}
                                    </button>
                                }.into_view()
                            } else {
                                let probe = run_probe.clone();
                                view! {
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="ssh-connectivity-probe"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| probe()
                                    >
                                        {move || if ssh_connectivity_busy.get() {
                                            t(locale.get(), "ssh_check.probing")
                                        } else {
                                            t(locale.get(), "ssh_check.probe")
                                        }}
                                    </button>
                                }.into_view()
                            }}
                        </div>
                    </div>
                </div>
            }
            .into_view()
        })}
        {move || update_check_modal.get().map(|modal| match modal {
            UpdateCheckModal::Checking => view! {
                <div class="overlay">
                    <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                        <h2>{move || t(locale.get(), "update_modal.checking_title")}</h2>
                        <div class="hint">{move || t(locale.get(), "update_modal.checking_body")}</div>
                    </div>
                </div>
            }
            .into_view(),
            UpdateCheckModal::Available { version, notes, release_url } => {
                let body = tf(locale.get(), "update_modal.available_body", &[("version", &version)]);
                let notes_html = (!notes.trim().is_empty()).then(|| md_to_html(&notes));
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.available_title")}</h2>
                            <div class="hint">{body}</div>
                            {notes_html.map(|html| view! {
                                <div class="update-notes md markdown" inner_html=html></div>
                            })}
                            <div class="row">
                                <button
                                    type="button"
                                    class="update-modal-dismiss"
                                    data-testid="update-check-dismiss"
                                    on:click=move |_| {
                                        update_check_enabled.set(false);
                                        update_banner.set(None);
                                        update_check_modal.set(None);
                                        spawn_local(async {
                                            let arg = to_value(&serde_json::json!({ "enabled": false })).unwrap_or(JsValue::NULL);
                                            let _ = invoke("set_update_check_enabled", arg).await;
                                        });
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.never")}
                                </button>
                                <button
                                    type="button"
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.later")}
                                </button>
                                <button
                                    type="button"
                                    class="primary"
                                    data-testid="update-check-open-releases"
                                    on:click=move |_| {
                                        open_external_url(release_url.clone());
                                        update_check_modal.set(None);
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.open_releases")}
                                </button>
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::UpToDate { version } => {
                let body = tf(locale.get(), "update_modal.up_to_date_body", &[("version", &version)]);
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.up_to_date_title")}</h2>
                            <div class="hint">{body}</div>
                            <div class="row">
                                <button
                                    type="button"
                                    class="primary"
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.ok")}
                                </button>
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::Failed { message } => view! {
                <div class="overlay">
                    <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                        <h2>{move || t(locale.get(), "update_modal.failed_title")}</h2>
                        <div class="hint">{message}</div>
                        <div class="row">
                            <button
                                type="button"
                                class="primary"
                                on:click=move |_| update_check_modal.set(None)
                            >
                                {move || t(locale.get(), "update_modal.ok")}
                            </button>
                        </div>
                    </div>
                </div>
            }
            .into_view(),
        })}
        <div class="app"
            class:app-entering=move || app_shell_entering.get()
            class:app-hidden=move || show_projects.get() && !show_settings.get() && modal_artifact.get().is_none()
            on:contextmenu=on_context_menu>
        <Sidebar
            state=SidebarState {
                locale, show_sidebar, sidebar_w, show_proj_menu, show_projects, demo_mode, project_info, proj_list,
                sessions, folders, drag_session, drop_target, active_session, running,
                attention: approval_pending,
                rename_session_input, rename_session_target, collapsed_folders, folder_modal_input,
                folder_modal, demos, session_history_cursor, session_history_loading,
                update_banner,
            }
            open_update=Callback::new(move |_| {
                if let Some(u) = update_banner.get() {
                    update_check_modal.set(Some(UpdateCheckModal::Available {
                        version: u.version,
                        notes: u.notes,
                        release_url: u.release_url,
                    }));
                }
            })
            toggle_proj_menu=Callback::new(toggle_proj_menu)
            open_proj_settings=Callback::new(open_proj_settings)
            switch_project=switch_project
            new_session=Callback::new(new_session)
            new_folder=Callback::new(new_folder)
            open_files=Callback::new(open_files)
            open_library=Callback::new(move |_| show_library.set(true))
            load_demo=Callback::new(load_demo)
            load_session=load_session
            load_older_sessions=Callback::new(move |_| load_older_sessions(
                sessions,
                pending_turns,
                running,
                session_history_cursor,
                session_history_loading,
            ))
            move_session_to=move_session_to
            open_session_actions=Callback::new(move |(ev, id, title, pinned): (web_sys::MouseEvent, String, String, bool)| {
                ctx_menu.set(Some(context_menu::session_menu(
                    ev.client_x() as f64,
                    ev.client_y() as f64,
                    &id,
                    &title,
                    pinned,
                    locale.get(),
                )));
            })
            open_folder_actions=Callback::new(move |(ev, id, name): (web_sys::MouseEvent, String, String)| {
                ctx_menu.set(Some(context_menu::folder_menu(
                    ev.client_x() as f64,
                    ev.client_y() as f64,
                    &id,
                    &name,
                    locale.get(),
                )));
            })
            open_capabilities=Callback::new(open_capabilities)
            open_settings=Callback::new(open_settings)
            on_sidebar_resize_start=Callback::new(on_sidebar_resize_start)
        />

        <div class="workspace-area">
        <div class="workspace-main">
        <main class="center" class:split=move || center_split_on.get()>
            <div class="topbar">
                {move || (!show_sidebar.get()).then(|| view! {
                    <button class="icon-btn" title=move || t(locale.get(), "sidebar.show") on:click=move |_| show_sidebar.set(true)>{compose_icon("chevron")}</button>
                })}
                <div class="center-tabs" role="tablist">
                    <button type="button" class="center-tab" class:active=move || center_file.get().is_none()
                        on:click=move |_| center_file.set(None)>
                        <span class="center-tab-label">{move || {
                            let loc = locale.get();
                            if let Some(id) = active_session.get() {
                                if let Some(s) = sessions.get().iter().find(|s| s.id == id) {
                                    let clean = user_message_presentation(&s.title).body;
                                    let title = clean.trim();
                                    if !title.is_empty() { return clean; }
                                }
                            }
                            items.get().iter().find_map(|i| match i {
                                ChatItem::User(msg) => {
                                    let clean = user_message_presentation(msg).body;
                                    let t = clean.trim();
                                    if t.is_empty() { None }
                                    else if t.chars().count() > 48 {
                                        Some(format!("{}…", t.chars().take(48).collect::<String>()))
                                    } else { Some(t.to_string()) }
                                }
                                _ => None,
                            }).unwrap_or_else(|| i18n::t(loc, "center.new_session").into())
                        }}</span>
                    </button>
                    <For
                        each=move || center_files.get()
                        key=|file| file.path.clone()
                        children=move |file| {
                            let path = file.path;
                            let select_path = path.clone();
                            let close_path = path.clone();
                            let label = file.name;
                            view! {
                                <div class="center-tab-wrap">
                                    <button type="button" class="center-tab" class:active=move || center_file.get().as_ref() == Some(&path)
                                        title=path.clone() data-center-path=path.clone()
                                        on:click=move |_| center_file.set(Some(select_path.clone()))>
                                        <span class="center-tab-label">{label}</span>
                                    </button>
                                    <button type="button" class="center-tab-close"
                                        aria-label=move || t(locale.get(), "center.close_tab")
                                        on:click=move |ev| {
                                            ev.stop_propagation();
                                            let was_active = center_file.get_untracked().as_ref() == Some(&close_path);
                                            if close_path.starts_with("mcp-app:") {
                                                close_mcp_app(&close_path);
                                                mcp_apps.update(|apps| { apps.remove(&close_path); });
                                            }
                                            center_files.update(|files| files.retain(|file| file.path != close_path));
                                            if was_active { center_file.set(None); }
                                        }>{compose_icon("close")}</button>
                                </div>
                            }
                        }
                    />
                </div>
                {move || session_specialist.get().map(|s| view! { <span class="session-specialist">{s.name}</span> })}
                {move || if needs_api_key.get() {
                    view! {
                        <span class="hint hint-action">
                            {move || t(locale.get(), "err.no_api_key")}" "
                            <button type="button" class="link-inline" on:click=move |_| open_settings_fn(Some("models".into()))>
                                {move || t(locale.get(), "status.open_settings")}
                            </button>
                        </span>
                    }.into_view()
                } else {
                    view! { <span class="hint">{move || status.get()}</span> }.into_view()
                }}
                <div class="spacer"></div>
                <div class="inbox-wrap">
                    <button class="icon-btn"
                        class:active=move || inbox_open.get()
                        title=move || {
                            let n = inbox_sessions.get().len().to_string();
                            tf(locale.get(), "sess_status.needs_you_n", &[("n", &n)])
                        }
                        on:click=move |ev| {
                            ev.stop_propagation();
                            let opening = !inbox_open.get_untracked();
                            if opening { refresh_inbox(); }
                            inbox_open.set(opening);
                        }>
                        {compose_icon("bell")}
                        {move || {
                            let n = inbox_sessions.get().len();
                            (n > 0).then(|| view! { <span class="inbox-badge">{n}</span> })
                        }}
                    </button>
                    {move || inbox_open.get().then(|| view! {
                        <div class="inbox-drop" on:click=|ev| ev.stop_propagation()>
                            <div class="inbox-title">{move || t(locale.get(), "sess_status.needs_you")}</div>
                            {move || {
                                let rows = inbox_sessions.get();
                                if rows.is_empty() {
                                    view! { <div class="inbox-empty">{move || t(locale.get(), "inbox.empty")}</div> }.into_view()
                                } else {
                                    rows.into_iter().map(|s| {
                                        let project_id = s.project_id.clone();
                                        let session_id = s.id.clone();
                                        let title = user_message_presentation(&s.title).body;
                                        view! {
                                            <button type="button" class="inbox-item"
                                                on:click=move |_| {
                                                    inbox_open.set(false);
                                                    palette_open_session.call((project_id.clone(), session_id.clone()));
                                                }>
                                                <span class="inbox-item-project">{s.project_name.clone()}</span>
                                                <span class="inbox-item-title">{title}</span>
                                            </button>
                                        }
                                    }).collect_view()
                                }
                            }}
                        </div>
                    })}
                </div>
                <button class="icon-btn" title=move || t(locale.get(), "contexts.open_terminal")
                    class:active=move || terminal_panel_open.get()
                    on:click=move |_| {
                        if terminal_sessions.get_untracked().is_empty() {
                            open_terminal_for_context.call("local".into());
                        } else {
                            let should_open = !terminal_panel_open.get_untracked();
                            if should_open && active_terminal_id.get_untracked().is_none() {
                                if let Some(session) = terminal_sessions.get_untracked().first() {
                                    active_terminal_id.set(Some(session.id.clone()));
                                }
                            }
                            terminal_add_menu_open.set(false);
                            terminal_panel_open.set(should_open);
                        }
                    }>{compose_icon("terminal")}</button>
                <button class="icon-btn" title=move || t(locale.get(), "center.toggle_panel")
                    class:active=move || show_right.get()
                    on:click=move |_| {
                        show_right.update(|open| {
                            if *open {
                                *open = false;
                            } else {
                                if open_right_tabs.get_untracked().is_empty() {
                                    open_right_tabs.set(DEFAULT_RIGHT_TABS.to_vec());
                                    right_tab.set(RightTab::Artifacts);
                                }
                                *open = true;
                            }
                        });
                    }><span class="gi panel"></span></button>
            </div>

            {move || center_file.get().and_then(|path| {
                center_files.get().into_iter().find(|file| file.path == path)
            }).map(|file| {
                let path = file.path.clone();
                let revision = center_file_revisions.with(|revisions| {
                    revisions.get(&path).copied().unwrap_or_default()
                });
                // Including the revision in the preview identity disposes the
                // old async loader and mounts a fresh read after FileChanged.
                let dom_id = format!("center-file-{}-{revision}", file.path);
                let kind = file.kind.clone();
                let label = file.name.clone();
                let is_mcp_app = kind == "mcp_app";
                // R/Python scripts bind to a persistent runtime and can be run in
                // it. Immutable artifact tabs have no workspace path to run from,
                // and remote previews have no local file for the runtime to read.
                let run_language = (!path.starts_with("artifact:")
                    && !path.starts_with("artifact-version:")
                    && remote_file_path(&path).is_none())
                    .then(|| runtime_language(&path))
                    .flatten();
                let console_file = path.clone();
                view! {
                    <div
                        class=if run_language.is_some() {
                            "center-file-preview center-file-runtime-preview"
                        } else {
                            "center-file-preview"
                        }
                        class:runtime-panel-open=move || run_language.is_some() && center_runtime_panel.get()
                        class:center-mcp-app-preview=is_mcp_app
                        data-file-revision=revision
                        data-preview-kind=kind.clone()
                        data-file-path=path.clone()>
                        <div class="center-file-head">
                            <span>{if is_mcp_app { label } else { path.clone() }}</span>
                            <div class="spacer"></div>
                            // Bind this script to a runtime. Whole-file execution
                            // and direct editing deliberately stay out of this
                            // AI-first preview; selected code can still be run.
                            {run_language.map(|language| {
                                let bind_path = path.clone();
                                let options = create_memo(move |_| {
                                    runtime_binding_options(&execution_contexts.get(), language)
                                });
                                // None = no context can host this language, so
                                // there is nothing to inspect or run selections in.
                                let bound = create_memo({
                                    let path = path.clone();
                                    move |_| {
                                        let stored = center_runtime_binding.get().get(&path).cloned();
                                        resolve_runtime_binding(&options.get(), stored.as_deref())
                                    }
                                });
                                view! {
                                  {move || bound.get().map(|bound_id| {
                                    let bind_path = bind_path.clone();
                                    let inspect_context = bound_id.clone();
                                    view! {
                                    <select class="center-file-runtime"
                                        title=move || t(locale.get(), "runtime.bind")
                                        aria-label=move || t(locale.get(), "runtime.bind")
                                        // dom_value, not event_target_value: the
                                        // latter only casts input/textarea and
                                        // reads a <select> as "".
                                        on:change=move |ev| {
                                            let context_id = dom_value(&ev);
                                            center_runtime_binding.update(|bindings| {
                                                bindings.insert(bind_path.clone(), context_id.clone());
                                            });
                                            if center_runtime_panel.get_untracked() {
                                                if let Some(project) = project_info.get_untracked() {
                                                    let ready = runtime_infos.get_untracked().iter().any(|runtime| {
                                                        runtime.key.project_id == project.id
                                                            && runtime.key.context_id == context_id
                                                            && runtime.key.language == language
                                                            && runtime.status == "ready"
                                                    });
                                                    if ready {
                                                        inspect_runtime_objects(
                                                            runtime_binding_state_key(&project.id, &context_id, language),
                                                            project.id,
                                                            context_id,
                                                            language.to_string(),
                                                            locale,
                                                            runtime_object_states,
                                                            runtime_infos,
                                                        );
                                                    }
                                                }
                                            }
                                        }>
                                        {options.get().into_iter().map(|(id, label)| {
                                            let selected = bound_id == id;
                                            view! {
                                                <option value=id selected=selected>
                                                    {format!("{} · {label}", language_display(language))}
                                                </option>
                                            }
                                        }).collect_view()}
                                    </select>
                                    <button type="button" class="center-file-btn" data-runtime-panel=""
                                        class:primary=move || center_runtime_panel.get()
                                        title=move || t(locale.get(), "runtime.toggle_panel")
                                        aria-label=move || t(locale.get(), "runtime.toggle_panel")
                                        on:click=move |_| {
                                            let opening = !center_runtime_panel.get_untracked();
                                            center_runtime_panel.set(opening);
                                            if !opening {
                                                return;
                                            }
                                            let Some(project) = project_info.get_untracked() else {
                                                return;
                                            };
                                            let ready = runtime_infos.get_untracked().iter().any(|runtime| {
                                                runtime.key.project_id == project.id
                                                    && runtime.key.context_id == inspect_context
                                                    && runtime.key.language == language
                                                    && runtime.status == "ready"
                                            });
                                            if ready {
                                                inspect_runtime_objects(
                                                    runtime_binding_state_key(&project.id, &inspect_context, language),
                                                    project.id,
                                                    inspect_context.clone(),
                                                    language.to_string(),
                                                    locale,
                                                    runtime_object_states,
                                                    runtime_infos,
                                                );
                                            }
                                        }>{compose_icon("runtime-panel")}</button>
                                    }
                                  })}
                                }
                            })}
                            // Split the center: document left, the main conversation
                            // right. Collapses the right pane so the two share its width.
                            <button type="button" class="center-file-btn" data-center-split=""
                                class:primary=move || center_split.get()
                                title=move || t(locale.get(), "center.split")
                                on:click=move |_| {
                                    center_split.update(|on| *on = !*on);
                                    if center_split.get_untracked() { show_right.set(false); }
                                }>{compose_icon("split")}</button>
                        </div>
                        {if is_mcp_app {
                            mcp_apps.get().get(&path).cloned().map(|payload_json| view! {
                                <McpAppPreview instance_id=path.clone() payload_json=payload_json />
                            }).into_view()
                        } else {
                            view! {
                                <WorkspaceFilePreview dom_id=dom_id.clone() path=path.clone() kind=kind.clone() />
                            }.into_view()
                        }}
                        {run_language.map(|language| {
                            let inspector_path = path.clone();
                            let inspector_options = create_memo(move |_| {
                                runtime_binding_options(&execution_contexts.get(), language)
                            });
                            let inspector_bound = create_memo(move |_| {
                                let stored = center_runtime_binding.get().get(&inspector_path).cloned();
                                resolve_runtime_binding(&inspector_options.get(), stored.as_deref())
                            });
                            move || center_runtime_panel.get().then(|| {
                                inspector_bound.get().and_then(|context_id| {
                                    let project = project_info.get()?;
                                    let context_label = inspector_options.get().into_iter()
                                        .find(|(id, _)| id == &context_id)
                                        .map(|(_, label)| label)
                                        .unwrap_or_else(|| context_id.clone());
                                    Some(view! {
                                        <CenterRuntimeEnvironment
                                            project_id=project.id
                                            context_id=context_id
                                            context_label=context_label
                                            language=language.to_string()
                                            locale=locale
                                            states=runtime_object_states
                                            runtimes=runtime_infos
                                            selection_popup=selection_popup
                                        />
                                    })
                                })
                            })
                        })}
                        {run_language.map(|_| {
                            let console_file = console_file.clone();
                            move || center_runtime_panel.get().then(|| view! {
                                <CenterRuntimeConsole path=console_file.clone() consoles=center_console />
                            })
                        })}
                    </div>
                }
            })}
            {move || selection_popup.get().map(|(text, source, x, y)| {
                let quote = text.clone();
                let quote_source = source.clone();
                let quote_source_for_click = quote_source.clone();
                let explain = text.clone();
                let annotate_text = text.clone();
                let annotate_source = source.clone();
                // Only chat-transcript selections (no source path) can be saved
                // as a highlight; file-preview selections have their own actions.
                let star_text = source.is_none().then(|| text.clone());
                // Run the selection in the file's bound runtime — the RStudio
                // reflex. Only for R/Python sources, where a runtime exists.
                let run_selection = source.as_deref()
                    .and_then(runtime_language)
                    .map(|language| (source.clone().unwrap_or_default(), language, text));
                view! {
                    <div class="selection-popup" style=format!("left:{x}px;top:{y}px")>
                        {star_text.map(|text| view! {
                            <button type="button" class="selection-popup-btn"
                                on:click=move |_| {
                                    let Some(session_id) = active_session.get_untracked() else { return; };
                                    let text = text.clone();
                                    selection_popup.set(None);
                                    clear_selection();
                                    spawn_local(async move {
                                        let args = to_value(&serde_json::json!({
                                            "sessionId": session_id, "text": text,
                                        })).unwrap();
                                        if invoke_checked("star_library_text", args).await.is_ok() {
                                            refresh_library_items.call(());
                                            ensure_right_tab(RightTab::Highlights, show_right, open_right_tabs, right_tab);
                                        }
                                    });
                                }>
                                {compose_icon("star")}
                                <span>{t(locale.get(), "selection.highlight")}</span>
                            </button>
                        })}
                        {run_selection.map(|(path, language, code)| {
                            let run_ctx = RuntimeRunCtx {
                                consoles: center_console,
                                busy: center_run_busy,
                                runtimes: runtime_infos,
                                project: project_info,
                                object_states: runtime_object_states,
                                inspector_open: center_runtime_panel,
                                locale,
                            };
                            view! {
                                <button type="button" class="selection-popup-btn"
                                    on:click=move |_| {
                                        let options = runtime_binding_options(
                                            &execution_contexts.get_untracked(), language,
                                        );
                                        let stored = center_runtime_binding.get_untracked()
                                            .get(&path).cloned();
                                        let Some(context_id) = resolve_runtime_binding(
                                            &options, stored.as_deref(),
                                        ) else { return; };
                                        selection_popup.set(None);
                                        clear_selection();
                                        run_in_runtime(
                                            path.clone(),
                                            context_id,
                                            language.to_string(),
                                            code.clone(),
                                            locale.get_untracked(),
                                            run_ctx,
                                        );
                                    }>
                                    {compose_icon("play")}
                                    <span>{t(locale.get(), "selection.run")}</span>
                                </button>
                            }
                        })}
                        <button type="button" class="selection-popup-btn"
                            on:click=move |_| {
                                composer_quotes.update(|items| items.push(
                                    ComposerQuote::from_selection(
                                        quote.clone(),
                                        quote_source_for_click.clone(),
                                    )
                                ));
                                selection_popup.set(None);
                                clear_selection();
                                if quote_source_for_click.as_ref() == center_file.get_untracked().as_ref() {
                                    center_split.set(true);
                                    show_right.set(false);
                                }
                                focus_composer();
                            }>
                            {compose_icon("plus")}
                            <span>{move || if quote_source.as_ref() == center_file.get().as_ref() {
                                t(locale.get(), "selection.ask_ai")
                            } else {
                                t(locale.get(), "selection.add_to_chat")
                            }}</span>
                        </button>
                        <button type="button" class="selection-popup-btn"
                            on:click=move |_| {
                                let question = message_with_quotes(
                                    &t(locale.get(), "selection.explain_prompt"),
                                    &[ComposerQuote::plain(explain.clone())],
                                );
                                selection_popup.set(None);
                                clear_selection();
                                send_side_chat(question);
                            }>
                            {compose_icon("chat")}
                            <span>{t(locale.get(), "selection.explain")}</span>
                        </button>
                        // Annotate → append the passage to reviews/<file>.md, which the
                        // agent reads back with its ordinary tools. Only offered when the
                        // selection came from a file preview (source path known).
                        {annotate_source.map(|src| {
                            let quote = annotate_text.clone();
                            view! {
                                <button type="button" class="selection-popup-btn"
                                    on:click=move |_| {
                                        let quote = quote.clone();
                                        let src = src.clone();
                                        let loc = locale.get();
                                        selection_popup.set(None);
                                        clear_selection();
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({
                                                "sourcePath": src, "quote": quote,
                                            })).unwrap();
                                            match invoke_checked("append_review_note", arg).await {
                                                Ok(v) => {
                                                    let path = v.as_string().unwrap_or_default();
                                                    status.set(tf(loc, "selection.annotated", &[("path", &path)]));
                                                }
                                                Err(e) => status.set(localize_backend(loc, &js_error_text(e))),
                                            }
                                        });
                                    }>
                                    {compose_icon("doc")}
                                    <span>{t(locale.get(), "selection.annotate")}</span>
                                </button>
                            }
                        })}
                    </div>
                }
            })}
            <div class="chat-stage" class:center-hidden=move || center_file_open.get() && !center_split.get()>
            <div class="chat" id=CHAT_SCROLLER_ID
                on:mouseup=move |ev| {
                    // Primary button only: a right-click mouseup would re-raise
                    // the popup on top of the context menu. Also honors the
                    // "selection quick actions" setting.
                    if ev.button() != 0 {
                        return;
                    }
                    let popup = selection_popup_enabled
                        .get_untracked()
                        .then(context_menu::selection_text)
                        .flatten()
                        .map(|text| (text, None, ev.client_x(), ev.client_y()));
                    selection_popup.set(popup);
                }
                on:scroll=move |_| {
                    if selection_popup.get_untracked().is_some() {
                        selection_popup.set(None);
                    }
                }>
                <div class="thread" id=CHAT_THREAD_ID>
                    {move || active_session.get().and_then(|id| {
                        transcript_pages.get().get(&id).copied().and_then(|page| {
                            let (_, window_start, _) = items.with(|rows| {
                                transcript_render_window(
                                    rows,
                                    page.window_user_start,
                                    TRANSCRIPT_RENDER_TURNS,
                                )
                            });
                            if window_start > 0 {
                                Some(view! {
                                    <div class="transcript-page-control">
                                        <button
                                            type="button"
                                            class="transcript-load-older"
                                            on:click=move |_| show_earlier_loaded.call(())
                                        >
                                            {t(locale.get(), "transcript.show_earlier")}
                                        </button>
                                    </div>
                                })
                            } else {
                                page.next_before_seq.map(|_| {
                                let loading = page.loading;
                                view! {
                                    <div class="transcript-page-control">
                                        <button
                                            type="button"
                                            class="transcript-load-older"
                                            disabled=loading
                                            on:click=move |_| load_earlier_messages.call(())
                                        >
                                            {t(
                                                locale.get(),
                                                if loading {
                                                    "transcript.loading_older"
                                                } else {
                                                    "transcript.load_older"
                                                },
                                            )}
                                        </button>
                                    </div>
                                }
                                })
                            }
                        })
                    })}
                    {move || items.with(|l| l.is_empty()).then(|| view! {
                        <div class="empty">
                            <span class="empty-logo"></span>
                            <h1>{move || empty_title(locale.get(), empty_title_idx.get())}</h1>
                            <p>{move || empty_subtitle(locale.get(), empty_subtitle_idx.get())}</p>
                        </div>
                    })}
                    // Keyed rows (#65): the key is a content fingerprint, so a
                    // streaming delta rebuilds only the message it touched, not
                    // the whole thread (which froze long conversations).
                    <For
                        each=move || {
                            use std::hash::{Hash, Hasher};
                            let arts_fp = artifacts.with(|a| artifacts_fingerprint(a));
                            let busy_now = busy.get();
                            let requested_start = if busy_now {
                                usize::MAX
                            } else {
                                active_session.get().and_then(|id| {
                                    transcript_pages
                                        .get()
                                        .get(&id)
                                        .map(|page| page.window_user_start)
                                }).unwrap_or(usize::MAX)
                            };
                            // `with` avoids deep-cloning every message per flush;
                            // only rows being built clone their item below.
                            items.with(|list| {
                            // Queued user turns live after the active turn and
                            // must not make its process group look historical.
                            let last = trailing_queue_start(list).saturating_sub(1);
                            // Keep process layers separate while the turn runs;
                            // once complete, fold commentary + reasoning + tools
                            // into one activity summary before the final answer.
                            let mut rows: Vec<(usize, u64, ThreadRow)> = Vec::new();
                            let (window, _, _) = transcript_render_window(
                                list,
                                requested_start,
                                TRANSCRIPT_RENDER_TURNS,
                            );
                            let mut i = window.start;
                            while i < window.end {
                                if renders_nothing(&list[i]) { i += 1; continue; }
                                if let Some(end) = completed_activity_end(list, i, busy_now) {
                                    let start = i;
                                    let mut run: Vec<(usize, ChatItem)> = Vec::new();
                                    for j in i..end {
                                        if is_turn_activity_at(list, j) {
                                            run.push((j, list[j].clone()));
                                        }
                                    }
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    for (idx, it) in &run { (idx, it.fingerprint()).hash(&mut h); }
                                    true.hash(&mut h);
                                    let items_only = run.into_iter().map(|(_, item)| item).collect();
                                    rows.push((start, h.finish(), ThreadRow::Activity { items: items_only }));
                                    i = end;
                                } else if is_tool_activity(&list[i]) {
                                    let start = i;
                                    let mut run: Vec<(usize, ChatItem)> = Vec::new();
                                    let mut j = i;
                                    while j < window.end {
                                        if renders_nothing(&list[j]) { j += 1; continue; }
                                        if is_tool_activity(&list[j]) { run.push((j, list[j].clone())); j += 1; }
                                        else { break; }
                                    }
                                    // Usage is metadata for the whole reply, not
                                    // a boundary that closes the live step run.
                                    let live = busy_now && (j > last || list[j..=last].iter().all(|item| {
                                        renders_nothing(item) || matches!(item, ChatItem::Usage { .. })
                                    }));
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    for (idx, it) in &run { (idx, it.fingerprint()).hash(&mut h); }
                                    live.hash(&mut h);
                                    let items_only: Vec<ChatItem> = run.into_iter().map(|(_, c)| c).collect();
                                    rows.push((start, h.finish(), ThreadRow::Steps { items: items_only, live }));
                                    i = j;
                                } else {
                                    let commentary = is_commentary_at(list, i);
                                    // A text row can become commentary when the
                                    // next tool event arrives. Keep streaming
                                    // assistant rows lightweight so replacing
                                    // one cannot strand async markdown effects
                                    // under a disposed Leptos owner.
                                    let compact_assistant = commentary
                                        || (busy_now
                                            && matches!(&list[i], ChatItem::Assistant { .. }));
                                    let mut fp = list[i].fingerprint();
                                    // Assistant markdown embeds artifact chips (index + label).
                                    if matches!(&list[i], ChatItem::Assistant { .. }) { fp ^= arts_fp; }
                                    fp ^= (commentary as u64) << 63;
                                    fp ^= (compact_assistant as u64) << 62;
                                    rows.push((i, fp, ThreadRow::Item {
                                        i,
                                        item: list[i].clone(),
                                        commentary,
                                        compact_assistant,
                                    }));
                                    i += 1;
                                }
                            }
                            rows
                            })
                        }
                        key=|(start, fp, _)| (*start, *fp)
                        children=move |(start, _, row)| {
                            match row {
                                ThreadRow::Item {
                                    i,
                                    item,
                                    commentary,
                                    compact_assistant,
                                } => {
                                    let arts = artifacts.get_untracked();
                                    let sid = active_session.get().unwrap_or_default();
                                    let on_resume = Callback::new(resume_turn);
                                    let class = if commentary {
                                        "msg assistant commentary"
                                    } else {
                                        class_for(&item)
                                    };
                                    let user_index =
                                        user_turn_index(&items.get_untracked(), i).map(|index| {
                                            index
                                                + active_session
                                                    .get_untracked()
                                                    .and_then(|id| {
                                                        transcript_pages.with_untracked(|pages| {
                                                            pages.get(&id).copied()
                                                        })
                                                    })
                                                    .map_or(0, |page| page.user_offset)
                                        });
                                    let data_user_index =
                                        user_index.map(|index| index.to_string());
                                    view! {
                                        <div class=class
                                            class:outline-target=move || user_index.is_some_and(|index| {
                                                conversation_outline_selected.get() == Some(index)
                                            })
                                            data-ui-index=i.to_string()
                                            data-user-index=data_user_index>
                                            {render_item(
                                                i, &item, &arts, on_artifact_select, on_file_link,
                                                run_records, busy.read_only(), compact_assistant, active_acp_agent_id.get().is_none(), edit_message, branch_message, sid,
                                                respond_confirm, on_resume, on_queue,
                                            )}
                                        </div>
                                    }.into_view()
                                }
                                ThreadRow::Steps { items, live } => {
                                    let sid = active_session.get().unwrap_or_default();
                                    // ponytail: position-keyed; move to stable
                                    // row ids if mid-list edits ever shift groups.
                                    let group_id = format!("{sid}:steps:{start}");
                                    view! {
                                        <div class="steps-wrap">{
                                            render_steps_group(
                                                items,
                                                live,
                                                false,
                                                group_id,
                                                step_disclosure_state,
                                            )
                                        }</div>
                                    }.into_view()
                                },
                                ThreadRow::Activity { items } => {
                                    let sid = active_session.get().unwrap_or_default();
                                    let group_id = format!("{sid}:activity:{start}");
                                    view! {
                                        <div class="steps-wrap">{
                                            render_steps_group(
                                                items,
                                                false,
                                                true,
                                                group_id,
                                                step_disclosure_state,
                                            )
                                        }</div>
                                    }.into_view()
                                },
                            }
                        }
                    />
                    {move || (!busy.get()).then(|| active_session.get()).flatten().and_then(|id| {
                        transcript_pages.get().get(&id).copied().and_then(|page| {
                            let (_, start, total) = items.with(|rows| {
                                transcript_render_window(
                                    rows,
                                    page.window_user_start,
                                    TRANSCRIPT_RENDER_TURNS,
                                )
                            });
                            (start + TRANSCRIPT_RENDER_TURNS < total).then(|| view! {
                                <div class="transcript-page-control">
                                    <button
                                        type="button"
                                        class="transcript-load-older"
                                        on:click=move |_| show_newer_loaded.call(())
                                    >
                                        {t(locale.get(), "transcript.show_newer")}
                                    </button>
                                </div>
                            })
                        })
                    })}
                </div>
            </div>
            {move || {
                let rows = conversation_outline.get();
                (!rows.is_empty()).then(|| {
                    if conversation_outline_open.get() {
                        let count = rows.len().to_string();
                        let entries = rows
                            .iter()
                            .enumerate()
                            .map(|(position, entry)| {
                                let target = entry.user_index;
                                let before_seq =
                                    rows.get(position + 1).and_then(|next| next.seq);
                                let clean = user_message_presentation(&entry.text).body;
                                let label = if clean.is_empty() {
                                    t(locale.get(), "outline.attachment")
                                } else {
                                    clean
                                };
                                let aria_label = label.clone();
                                let title = label.clone();
                                view! {
                                    <button
                                        type="button"
                                        class="conversation-outline-item"
                                        class:active=move || conversation_outline_selected.get() == Some(target)
                                        aria-label=aria_label
                                        title=title
                                        prop:disabled=move || {
                                            if !busy.get() {
                                                return false;
                                            }
                                            let offset = active_session
                                                .get()
                                                .and_then(|id| transcript_pages
                                                    .get()
                                                    .get(&id)
                                                    .copied())
                                                .map_or(0, |page| page.user_offset);
                                            !conversation_outline_target_is_loaded(
                                                &items.get(),
                                                offset,
                                                target,
                                            )
                                        }
                                        on:click=move |_| {
                                            jump_to_conversation_outline.call((target, before_seq));
                                        }
                                    >
                                        <span class="conversation-outline-number" aria-hidden="true">
                                            {target + 1}
                                        </span>
                                        <span class="conversation-outline-text">{label}</span>
                                    </button>
                                }
                            })
                            .collect_view();
                        view! {
                            <nav
                                class="conversation-outline-panel"
                                data-testid="conversation-outline"
                                aria-label=move || t(locale.get(), "outline.title")
                            >
                                <header>
                                    <div>
                                        <strong>{move || t(locale.get(), "outline.title")}</strong>
                                        <span>{move || tf(locale.get(), "outline.questions_n", &[("n", &count)])}</span>
                                    </div>
                                    <button
                                        type="button"
                                        class="icon-btn"
                                        title=move || t(locale.get(), "outline.hide")
                                        aria-label=move || t(locale.get(), "outline.hide")
                                        on:click=move |_| conversation_outline_open.set(false)
                                    >
                                        {compose_icon("close")}
                                    </button>
                                </header>
                                <div class="conversation-outline-list">{entries}</div>
                            </nav>
                        }
                        .into_view()
                    } else {
                        let stride = (rows.len() + 27) / 28;
                        let marks = rows
                            .iter()
                            .step_by(stride.max(1))
                            .map(|entry| {
                                let width = 45 + entry.text.chars().count().min(40);
                                let target = entry.user_index;
                                view! {
                                    <span
                                        class="conversation-outline-mark"
                                        class:active=move || conversation_outline_selected.get() == Some(target)
                                        style=format!("width:{width}%")
                                    ></span>
                                }
                            })
                            .collect_view();
                        view! {
                            <button
                                type="button"
                                class="conversation-outline-toggle"
                                data-testid="conversation-outline-toggle"
                                title=move || t(locale.get(), "outline.show")
                                aria-label=move || t(locale.get(), "outline.show")
                                aria-expanded="false"
                                on:click=move |_| conversation_outline_open.set(true)
                            >
                                <span class="conversation-outline-marks" aria-hidden="true">{marks}</span>
                            </button>
                        }
                        .into_view()
                    }
                })
            }}
            </div>

            {move || active_session.get().and_then(|session_id| {
                let transfers = run_records
                    .get()
                    .into_iter()
                    .filter(|run| run.frame_id.as_deref() == Some(session_id.as_str()))
                    .filter_map(|run| {
                        let progress = run_progress(&run)?;
                        transfer_progress_visible(&progress, &run.status).then_some((run, progress))
                    })
                    .collect::<Vec<_>>();
                (!transfers.is_empty()).then(|| view! {
                    <div class="transfer-tray" aria-live="polite">
                        {transfers.into_iter().map(|(run, progress)| {
                            let run_id = run.id.clone();
                            let cancellable = matches!(run.status.as_str(), "submitted" | "running");
                            let direction = progress.direction.clone();
                            let icon = match direction.as_str() {
                                "download" => "↓",
                                "relay" => "↔",
                                _ => "↑",
                            };
                            view! {
                                <section class="transfer-card" data-run-id=run.id>
                                    <div class="transfer-card-head">
                                        <span class="transfer-card-icon">{icon}</span>
                                        <strong>{run.title}</strong>
                                        <span>{run.context_id}</span>
                                        {cancellable.then(|| view! {
                                            <button type="button" class="icon-btn transfer-cancel"
                                                title=t(locale.get(), "runs.cancel")
                                                aria-label=t(locale.get(), "runs.cancel")
                                                on:click=move |_| {
                                                    let run_id = run_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                        let _ = invoke("cancel_run", arg).await;
                                                        refresh_runs(run_records, locale);
                                                    });
                                                }>{compose_icon("close")}</button>
                                        })}
                                    </div>
                                    {run_progress_meter(progress, locale.get())}
                                </section>
                            }
                        }).collect_view()}
                    </div>
                })
            })}

            <div class="composer" class:center-hidden=move || center_file_open.get() && !center_split.get()>
                {move || stopping_session.get().is_some().then(|| view! {
                    <div class="stopping-toast">
                        <span class="stopping-spinner"></span>
                        <div class="stopping-text">
                            <strong>{move || t(locale.get(), "composer.stopping")}</strong>
                            <span>{move || t(locale.get(), "composer.stopping_hint")}</span>
                        </div>
                    </div>
                })}
                <div class="composer-inner"
                    class:composer-dragover=move || drag_over.get()
                    on:dragover=on_drag_over
                    on:dragleave=on_drag_leave
                    on:drop=on_drop>
                    <div class="composer-resizer"
                        title=move || t(locale.get(), "composer.resize_hint")
                        on:mousedown=on_composer_resize_start></div>
                    <input id="composer-file-input" type="file" multiple=true class="composer-file-input"
                        on:change=on_files_selected />
                    {move || (!attachments.get().is_empty()).then(|| view! {
                        <div class="composer-attachments">
                            {attachments.get().into_iter().map(|att| {
                                let remove_key = match &att {
                                    ComposerAttachment::Uploading { key, .. }
                                    | ComposerAttachment::Ready { key, .. }
                                    | ComposerAttachment::Error { key, .. } => key.clone(),
                                };
                                let (name, path, state, error) = match att {
                                    ComposerAttachment::Uploading { name, .. } => {
                                        let label = if name.is_empty() {
                                            t(locale.get(), "composer.uploading").into()
                                        } else {
                                            name
                                        };
                                        (label, None, "uploading", None)
                                    }
                                    ComposerAttachment::Ready { name, path, .. } => (name, Some(path), "ready", None),
                                    ComposerAttachment::Error { name, error, .. } => {
                                        (name, None, "error", Some(error))
                                    }
                                };
                                let kind = path.as_deref().and_then(file_kind).unwrap_or("file");
                                let is_image = kind == "image";
                                // Both the JS and backend size guards phrase the rejection as
                                // "…byte limit"; surface an actionable hint for that case.
                                let too_large = error.as_deref().is_some_and(|e| e.contains("byte limit"));
                                let meta_key = match state {
                                    "uploading" => "composer.uploading",
                                    "error" if too_large => "composer.upload_too_large",
                                    "error" => "composer.upload_failed",
                                    _ if is_image => "attachment.image",
                                    _ => "attachment.file",
                                };
                                let hover = if too_large {
                                    t(locale.get(), "composer.upload_too_large_hint").to_string()
                                } else {
                                    error.unwrap_or_default()
                                };
                                let preview = if is_image {
                                    path.clone().map(|path| view! {
                                        <AttachmentThumbnail path=path alt=name.clone() />
                                    }.into_view())
                                } else {
                                    Some(view! {
                                        <span class="composer-attachment-icon">{compose_icon("doc")}</span>
                                    }.into_view())
                                };
                                view! {
                                    <div class=format!("composer-attachment-row {state} {kind}")
                                        title=hover>
                                        {preview}
                                        <span class="composer-attachment-copy">
                                            <span class=format!("composer-attachment {state}")>{name}</span>
                                            <span class="composer-attachment-meta">{move || t(locale.get(), meta_key)}</span>
                                        </span>
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
                                            aria-label=move || t(locale.get(), "composer.remove_attachment")
                                            on:click=move |_| attachments.update(|items| {
                                                items.retain(|a| match a {
                                                    ComposerAttachment::Uploading { key, .. }
                                                    | ComposerAttachment::Ready { key, .. }
                                                    | ComposerAttachment::Error { key, .. } => key != &remove_key,
                                                });
                                            })>{compose_icon("close")}</button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    {move || (!composer_references.get().is_empty()).then(|| view! {
                        <div class="composer-attachments composer-reference-chips">
                            {composer_references.get().into_iter().map(|reference| {
                                let key = reference.key();
                                let label = reference.label();
                                let kind = reference.kind();
                                let (icon, meta_key) = match kind {
                                    "skill" => ("skill", "attachment.skill"),
                                    "session" => ("chat", "attachment.session"),
                                    "project" => ("folder", "attachment.project"),
                                    "context" => ("server", "attachment.context"),
                                    "runtime" => ("terminal", "attachment.runtime"),
                                    _ => ("doc", "attachment.artifact"),
                                };
                                view! {
                                    <div class=format!("composer-attachment-row composer-reference-card {kind}")
                                        data-reference-kind=kind title=label.clone()>
                                        <span class="composer-attachment-icon">{compose_icon(icon)}</span>
                                        <span class="composer-attachment-copy">
                                            <span class="composer-attachment ready">{label}</span>
                                            <span class="composer-attachment-meta">{move || t(locale.get(), meta_key)}</span>
                                        </span>
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
                                            aria-label=move || t(locale.get(), "composer.remove_attachment")
                                            on:click=move |_| composer_references.update(|items| items.retain(|item| item.key() != key))>{compose_icon("close")}</button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    {move || (!composer_quotes.get().is_empty()).then(|| view! {
                        <div class="composer-attachments composer-reference-chips">
                            {composer_quotes.get().into_iter().enumerate().map(|(idx, quote)| {
                                let label = quote_label(&quote.text);
                                let title = quote.source.as_ref().map_or_else(
                                    || quote.text.clone(),
                                    |source| format!("{source}\n\n{}", quote.text),
                                );
                                let source = quote.source.clone();
                                view! {
                                    <div class="composer-attachment-row composer-reference-card quote" title=title>
                                        <span class="composer-attachment-icon">{compose_icon("chat")}</span>
                                        <span class="composer-attachment-copy">
                                            <span class="composer-attachment ready">{label}</span>
                                            <span class="composer-attachment-meta">{move || source.clone().unwrap_or_else(|| t(locale.get(), "attachment.quote").into())}</span>
                                        </span>
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
                                            aria-label=move || t(locale.get(), "composer.remove_attachment")
                                            on:click=move |_| composer_quotes.update(|items| {
                                                if idx < items.len() {
                                                    items.remove(idx);
                                                }
                                            })>{compose_icon("close")}</button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    <div class="composer-mention-anchor">
                        <textarea
                            id="composer-input"
                            style=move || {
                                if composer_h_custom.get() {
                                    format!("height:{}px", composer_h.get())
                                } else {
                                    format!("max-height:{}px", composer_h.get())
                                }
                            }
                            prop:value={move || input.get()}
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                match active_composer_trigger(&v) {
                                    Some((_, mode, q)) => { picker_query.set(q); picker_index.set(0); picker_mode.set(Some(mode)); }
                                    None => picker_mode.set(None),
                                }
                                input.set(v);
                            }
                            on:keydown:undelegated=on_send
                            on:paste=on_paste
                            prop:placeholder=move || tf(
                                locale.get(),
                                "composer.placeholder",
                                &[("modifier", if is_mac() { "Cmd" } else { "Ctrl" })],
                            )
                        ></textarea>
                        {move || picker_mode.get().map(|mode| {
                            let loc = locale.get();
                            let matches = picker_items.get();
                            let title = match mode {
                                ComposerPickerMode::Artifact => "composer.ref_artifacts",
                                ComposerPickerMode::Session => "composer.ref_sessions",
                                ComposerPickerMode::Skill => "composer.ref_skills",
                            };
                            view! {
                                <div class="mention-backdrop" on:mousedown=move |_| picker_mode.set(None)></div>
                                <div class="mention-menu">
                                    <div class="mention-group-label">{t(loc, title)}</div>
                                    {matches.into_iter().enumerate().map(|(i, item)| {
                                        let (name, sub, icon) = match item {
                                            // Uploads are artifacts too, so the origin badge is the
                                            // only thing separating a file the user dropped in from
                                            // one the agent produced.
                                            ComposerPickerItem::Artifact(a) => {
                                                let source = format!("{} · {}", a.session_title.unwrap_or_default(), a.project_name.unwrap_or_default());
                                                if a.origin.as_deref() == Some("upload") {
                                                    (a.name, format!("{} · {source}", t(loc, "composer.ref_upload")), "upload")
                                                } else {
                                                    (a.name, source, "attach")
                                                }
                                            }
                                            ComposerPickerItem::Session(s) => (s.title, s.project_name, "review"),
                                            ComposerPickerItem::Project { id: _, name } => (
                                                "#project".to_string(),
                                                tf(loc, "composer.ref_project_sub", &[("project", &name)]),
                                                "folder",
                                            ),
                                            ComposerPickerItem::Skill(s) => (s.name, s.description, "skill"),
                                            ComposerPickerItem::Context { id, label } => (label, id, "server"),
                                            ComposerPickerItem::Runtime { context_id, context_label, language } => (
                                                format!("{} runtime", language_display(&language)),
                                                format!("{context_label} · {context_id}"),
                                                "terminal",
                                            ),
                                        };
                                        view! {
                                            <button type="button" class="mention-item" class:active=move || picker_index.get() == i
                                                on:mousemove=move |_| picker_index.set(i)
                                                on:mousedown=move |ev| { ev.prevent_default(); select_picker_item.call(i); }>
                                                <span class="mention-item-icon">{compose_icon(icon)}</span>
                                                <span class="mention-item-text"><span class="mention-item-name">{name}</span><span class="mention-item-sub">{sub}</span></span>
                                            </button>
                                        }
                                    }).collect_view()}
                                    <div class="mention-menu-hint">{t(loc, "composer.mention_hint")}</div>
                                </div>
                            }
                        })}
                    </div>
                    {move || active_session.get().and_then(|session_id| {
                        active_acp_agent_id.get()?;
                        let options = acp_session_configs.get().get(&session_id).cloned().unwrap_or_default();
                        let modes_state = acp_session_modes.get().get(&session_id).cloned();
                        let mode = modes_state.as_ref()
                            .and_then(|state| state.get("currentModeId"))
                            .and_then(serde_json::Value::as_str).map(str::to_string);
                        // `availableModes` from the initial SessionModeState drives the
                        // picker; a single-mode agent stays a read-only chip.
                        let available_modes: Vec<(String, String)> = modes_state.as_ref()
                            .and_then(|state| state.get("availableModes"))
                            .and_then(serde_json::Value::as_array)
                            .map(|arr| arr.iter().filter_map(|m| {
                                let id = m.get("id").and_then(serde_json::Value::as_str)?.to_string();
                                let name = m.get("name").and_then(serde_json::Value::as_str).unwrap_or(&id).to_string();
                                Some((id, name))
                            }).collect())
                            .unwrap_or_default();
                        (!options.is_empty() || mode.is_some()).then(|| view! {
                            <div class="acp-composer-config" data-testid="acp-session-config">
                                {(!options.iter().any(|option| {
                                    option.get("id").and_then(serde_json::Value::as_str) == Some("mode")
                                        || option
                                            .get("name")
                                            .and_then(serde_json::Value::as_str)
                                            .is_some_and(|name| name.eq_ignore_ascii_case("mode"))
                                }))
                                    .then(|| {
                                        mode.map(|mode| {
                                            let current_label = available_modes.iter()
                                                .find(|(id, _)| id == &mode)
                                                .map(|(_, name)| name.clone())
                                                .unwrap_or_else(|| mode.clone());
                                            if available_modes.len() < 2 {
                                                return view! {
                                                    <span class="acp-config-chip acp-mode" title="Session mode">
                                                        <span class="acp-config-key">"mode"</span>
                                                        <span class="acp-config-val">{current_label}</span>
                                                    </span>
                                                }.into_view();
                                            }
                                            let session_id = session_id.clone();
                                            view! {
                                                <div class="acp-config-chip acp-config-select acp-mode-select" title="Session mode"
                                                    class:open=move || acp_config_menu_open.get().as_deref() == Some(ACP_MODE_MENU)>
                                                    <button type="button" class="acp-config-trigger" aria-label="Session mode"
                                                        on:click=move |_| {
                                                            acp_config_menu_open.update(|open| {
                                                                *open = if open.as_deref() == Some(ACP_MODE_MENU) { None } else { Some(ACP_MODE_MENU.into()) };
                                                            });
                                                        }>
                                                        <span class="acp-config-key">"mode"</span>
                                                        <span class="acp-config-val">{current_label}</span>
                                                    </button>
                                                    {move || (acp_config_menu_open.get().as_deref() == Some(ACP_MODE_MENU)).then(|| {
                                                        let session_id = session_id.clone();
                                                        let current_mode = mode.clone();
                                                        view! {
                                                            <div class="acp-config-backdrop" on:click=move |_| acp_config_menu_open.set(None)></div>
                                                            <div class="acp-config-menu" role="listbox">
                                                                {available_modes.clone().into_iter().map(|(mode_id, label)| {
                                                                    let selected = mode_id == current_mode;
                                                                    let session_id = session_id.clone();
                                                                    view! {
                                                                        <button type="button" class="acp-config-option" class:active=selected
                                                                            role="option" aria-selected=selected
                                                                            on:click=move |_| {
                                                                                acp_config_menu_open.set(None);
                                                                                let frame_id = session_id.clone();
                                                                                let mode_id = mode_id.clone();
                                                                                let args = to_value(&serde_json::json!({
                                                                                    "frameId": frame_id,
                                                                                    "modeId": mode_id,
                                                                                })).unwrap();
                                                                                spawn_local(async move {
                                                                                    if let Ok(value) = invoke_checked("set_acp_session_mode", args).await {
                                                                                        if let Some(applied) = value.as_string() {
                                                                                            // `session/set_mode` returns no state, so apply the
                                                                                            // selected id locally, preserving availableModes.
                                                                                            acp_session_modes.update(|all| {
                                                                                                let entry = all.entry(frame_id).or_insert_with(|| serde_json::json!({}));
                                                                                                if let serde_json::Value::Object(map) = entry {
                                                                                                    map.insert("currentModeId".into(), serde_json::Value::String(applied));
                                                                                                }
                                                                                            });
                                                                                        }
                                                                                    }
                                                                                });
                                                                            }>
                                                                            <span class="acp-config-option-label">{label}</span>
                                                                            {selected.then(|| view! { <span class="acp-config-option-check">"✓"</span> })}
                                                                        </button>
                                                                    }
                                                                }).collect_view()}
                                                            </div>
                                                        }
                                                    })}
                                                </div>
                                            }.into_view()
                                        })
                                    })}
                                {options.into_iter().map(|option| {
                                    let config_id = option.get("id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                                    let name = option.get("name").and_then(serde_json::Value::as_str).unwrap_or(&config_id).to_string();
                                    let description = option.get("description").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                                    if option.get("type").and_then(serde_json::Value::as_str) == Some("boolean") {
                                        let checked = option.get("currentValue").and_then(serde_json::Value::as_bool).unwrap_or(false);
                                        let session_id = session_id.clone();
                                        view! {
                                            <label class="acp-config-chip acp-config-toggle" title=description class:on=checked>
                                                <input type="checkbox" checked=checked on:change=move |event| {
                                                    let checked = event_target_checked(&event);
                                                    let frame_id = session_id.clone();
                                                    let args = to_value(&serde_json::json!({ "frameId": frame_id, "configId": config_id, "value": { "type": "boolean", "value": checked } })).unwrap();
                                                    spawn_local(async move { if let Ok(value) = invoke_checked("set_acp_session_config", args).await {
                                                        if let Ok(options) = serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>(value) { acp_session_configs.update(|all| { all.insert(frame_id, options); }); }
                                                    }});
                                                }/>
                                                <span class="acp-config-key">{name}</span>
                                                <span class="acp-config-val">{if checked { "On" } else { "Off" }}</span>
                                            </label>
                                        }.into_view()
                                    } else {
                                        let current = option.get("currentValue").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                                        let choices = acp_select_options(&option);
                                        let session_id = session_id.clone();
                                        let menu_id = config_id.clone();
                                        let current_label = choices.iter()
                                            .find(|(value, _)| value == &current)
                                            .map(|(_, label)| label.clone())
                                            .unwrap_or_else(|| current.clone());
                                        let open_id = menu_id.clone();
                                        view! {
                                            <div class="acp-config-chip acp-config-select" title=description
                                                class:open=move || acp_config_menu_open.get().as_deref() == Some(open_id.as_str())>
                                                <button type="button" class="acp-config-trigger" aria-label=name.clone()
                                                    on:click=move |_| {
                                                        let id = menu_id.clone();
                                                        acp_config_menu_open.update(|open| {
                                                            *open = if open.as_deref() == Some(id.as_str()) { None } else { Some(id) };
                                                        });
                                                    }>
                                                    <span class="acp-config-key">{name.clone()}</span>
                                                    <span class="acp-config-val">{current_label}</span>
                                                </button>
                                                {move || (acp_config_menu_open.get().as_deref() == Some(config_id.as_str())).then(|| {
                                                    let session_id = session_id.clone();
                                                    let config_id = config_id.clone();
                                                    let current = current.clone();
                                                    view! {
                                                        <div class="acp-config-backdrop" on:click=move |_| acp_config_menu_open.set(None)></div>
                                                        <div class="acp-config-menu" role="listbox">
                                                            {choices.clone().into_iter().map(|(value, label)| {
                                                                let selected = value == current;
                                                                let session_id = session_id.clone();
                                                                let config_id = config_id.clone();
                                                                view! {
                                                                    <button type="button" class="acp-config-option" class:active=selected
                                                                        role="option" aria-selected=selected
                                                                        on:click=move |_| {
                                                                            acp_config_menu_open.set(None);
                                                                            let frame_id = session_id.clone();
                                                                            let args = to_value(&serde_json::json!({
                                                                                "frameId": frame_id,
                                                                                "configId": config_id,
                                                                                "value": { "value": value },
                                                                            })).unwrap();
                                                                            spawn_local(async move {
                                                                                if let Ok(value) = invoke_checked("set_acp_session_config", args).await {
                                                                                    if let Ok(options) = serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>(value) {
                                                                                        acp_session_configs.update(|all| { all.insert(frame_id, options); });
                                                                                    }
                                                                                }
                                                                            });
                                                                        }>
                                                                        <span class="acp-config-option-label">{label}</span>
                                                                        {selected.then(|| view! { <span class="acp-config-option-check">"✓"</span> })}
                                                                    </button>
                                                                }
                                                            }).collect_view()}
                                                        </div>
                                                    }
                                                })}
                                            </div>
                                        }.into_view()
                                    }
                                }).collect_view()}
                            </div>
                        })
                    })}
                    <div class="composer-actions">
                        <div class="composer-tools">
                            <button type="button" class="composer-plus"
                                class:active=move || compose_menu_open.get()
                                title=move || t(locale.get(), "composer.add")
                                on:click=move |_| compose_menu_open.update(|o| *o = !*o)>
                                <span class="gi plus"></span>
                            </button>
                            {move || compose_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compose_menu_open.set(false)></div>
                                <div class="compose-menu">
                                    <div class="compose-menu-title">{move || t(locale.get(), "composer.compose")}</div>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.group_add")}</div>
                                        <button type="button" class="compose-item" disabled=composer_blocked
                                            on:click=move |ev| { compose_menu_open.set(false); pick_files(ev); }>
                                            <span class="compose-item-icon">{compose_icon("attach")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.attach_files")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.attach_files_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |ev| { compose_menu_open.set(false); open_files(ev); }>
                                            <span class="compose-item-icon">{compose_icon("folder")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.your_files")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.your_files_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                    </div>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.group_session")}</div>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                let loc = locale.get();
                                                status.set(t(loc, "status.reviewing"));
                                                let sid = active_session.get();
                                                spawn_local(async move {
                                                    let arg = to_value(&tauri_args::review_session(&sid)).unwrap();
                                                    if let Err(err) = invoke_checked("review_session", arg).await {
                                                        status.set(tf(loc, "status.review_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]));
                                                    }
                                                });
                                            }>
                                            <span class="compose-item-icon">{compose_icon("review")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.request_review")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.request_review_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                input.set(t(locale.get(), "composer.skill_prompt").into());
                                                focus_composer();
                                            }>
                                            <span class="compose-item-icon">{compose_icon("skill")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.save_skill")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.save_skill_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                open_settings_fn(Some("skills".into()));
                                            }>
                                            <span class="compose-item-icon">{compose_icon("skill")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "skills.manage")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "skills.manage_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                    </div>
                                </div>
                            })}
                            <button type="button" class="composer-compute"
                                class:active=move || agent_menu_open.get()
                                class:has-resource=move || !session_execution_contexts.get().is_empty()
                                title=move || t(locale.get(), "composer.agent_options")
                                aria-label=move || t(locale.get(), "composer.agent_options")
                                on:click=move |_| {
                                    let opening = !agent_menu_open.get_untracked();
                                    agent_menu_open.set(opening);
                                    reviewer_model_menu_open.set(false);
                                    specialist_menu_open.set(false);
                                    compute_menu_open.set(false);
                                    if opening {
                                        refresh_specialists();
                                        refresh_memory();
                                        refresh_execution_contexts(execution_contexts);
                                        refresh_runtimes(runtime_infos);
                                        refresh_runs(run_records, locale);
                                    }
                                }>
                                {compose_icon("controls")}
                            </button>
                            {move || agent_menu_open.get().then(|| {
                                let locked = items.with(|rows| !rows.is_empty());
                                view! {
                                <div class="compose-backdrop" on:click=move |_| {
                                    agent_menu_open.set(false);
                                    reviewer_model_menu_open.set(false);
                                    specialist_menu_open.set(false);
                                    compute_menu_open.set(false);
                                }></div>
                                <div class="compose-menu agent-menu" role="menu"
                                    aria-label=move || t(locale.get(), "composer.agent_options")>
                                    <label class="agent-menu-row">
                                        <span>{move || t(locale.get(), "composer.delegation")}</span>
                                        <span class="toggle agent-menu-toggle">
                                            <input type="checkbox" prop:checked=move || delegation_enabled.get()
                                                disabled=move || delegation_setting_busy.get()
                                                on:change=move |ev| {
                                                    let enabled = event_target_checked(&ev);
                                                    delegation_enabled.set(enabled);
                                                    delegation_setting_busy.set(true);
                                                    spawn_local(async move {
                                                        let (session_id, created_session) = match active_session.get_untracked() {
                                                            Some(session_id) => (session_id, false),
                                                            None if enabled => {
                                                                let Some(session_id) = invoke("new_session", JsValue::UNDEFINED).await.as_string() else {
                                                                    delegation_enabled.set(false);
                                                                    delegation_setting_busy.set(false);
                                                                    return;
                                                                };
                                                                (session_id, true)
                                                            }
                                                            None => {
                                                                delegation_enabled.set(false);
                                                                delegation_setting_busy.set(false);
                                                                return;
                                                            }
                                                        };
                                                        let args = to_value(&serde_json::json!({
                                                            "sessionId": session_id.clone(),
                                                            "enabled": enabled,
                                                        })).unwrap();
                                                        let saved = invoke_checked("set_session_delegation_enabled", args).await
                                                            .ok()
                                                            .and_then(|value| value.as_bool());
                                                        if created_session {
                                                            active_session.set(Some(session_id.clone()));
                                                            items.set(vec![]);
                                                            refresh_session_history();
                                                        }
                                                        if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                                                            delegation_enabled.set(saved.unwrap_or(!enabled));
                                                            delegation_setting_busy.set(false);
                                                        }
                                                    });
                                                } />
                                            <span class="toggle-track" aria-hidden="true"></span>
                                        </span>
                                    </label>
                                    <label class="agent-menu-row">
                                        <span>{move || t(locale.get(), "composer.agent_completion")}</span>
                                        <select class="agent-menu-select"
                                            data-testid="agent-completion-policy"
                                            disabled=move || !delegation_enabled.get() || active_session.get().is_none() || agent_completion_busy.get()
                                            on:change=move |event| {
                                                let mut next = agent_completion.get_untracked();
                                                next.policy = if dom_value(&event) == "background" {
                                                    AgentCompletionPolicy::Background
                                                } else {
                                                    AgentCompletionPolicy::Inline
                                                };
                                                if next.policy == AgentCompletionPolicy::Inline {
                                                    next.auto_resume = false;
                                                }
                                                save_agent_completion.call(next);
                                            }>
                                            <option value="inline" prop:selected=move || agent_completion.get().policy == AgentCompletionPolicy::Inline>
                                                {move || t(locale.get(), "composer.agent_completion.inline")}
                                            </option>
                                            <option value="background" prop:selected=move || agent_completion.get().policy == AgentCompletionPolicy::Background>
                                                {move || t(locale.get(), "composer.agent_completion.background")}
                                            </option>
                                        </select>
                                    </label>
                                    {move || (agent_completion.get().policy == AgentCompletionPolicy::Background).then(|| view! {
                                        <label class="agent-menu-row">
                                            <span>{move || t(locale.get(), "composer.agent_auto_resume")}</span>
                                            <span class="toggle agent-menu-toggle">
                                                <input type="checkbox"
                                                    data-testid="agent-auto-resume"
                                                    prop:checked=move || agent_completion.get().auto_resume
                                                    disabled=move || !delegation_enabled.get() || agent_completion_busy.get()
                                                    on:change=move |event| {
                                                        let mut next = agent_completion.get_untracked();
                                                        next.auto_resume = event_target_checked(&event);
                                                        save_agent_completion.call(next);
                                                    } />
                                                <span class="toggle-track" aria-hidden="true"></span>
                                            </span>
                                        </label>
                                    })}
                                    <label class="agent-menu-row">
                                        <span>{move || t(locale.get(), "composer.auto_review")}</span>
                                        <span class="toggle agent-menu-toggle">
                                            <input type="checkbox" prop:checked=move || auto_review_enabled.get()
                                                on:change=move |ev| {
                                                    let enabled = event_target_checked(&ev);
                                                    auto_review_enabled.set(enabled);
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
                                                        if invoke_checked("set_auto_review_enabled", arg).await.is_err() {
                                                            auto_review_enabled.set(!enabled);
                                                        }
                                                    });
                                                } />
                                            <span class="toggle-track" aria-hidden="true"></span>
                                        </span>
                                    </label>
                                    <button type="button" class="agent-menu-row" aria-haspopup="menu"
                                        on:click=move |_| {
                                            reviewer_model_menu_open.update(|open| *open = !*open);
                                            specialist_menu_open.set(false);
                                            compute_menu_open.set(false);
                                        }>
                                        <span>{move || t(locale.get(), "composer.reviewer_model")}</span>
                                        <span class="agent-menu-value">{move || {
                                            specialists.get().into_iter()
                                                .find(|specialist| specialist.id == "reviewer")
                                                .and_then(|reviewer| reviewer_backend_label(
                                                    &reviewer,
                                                    &models.get(),
                                                    &acp_agents.get(),
                                                    &t(locale.get(), "composer.reviewer.follow_session"),
                                                    &t(locale.get(), "composer.reviewer.missing_acp"),
                                                ))
                                                .unwrap_or_else(|| t(locale.get(), "composer.reviewer.default_http"))
                                        }}</span>
                                        <span class="agent-menu-chevron">{compose_icon("chevron-right")}</span>
                                    </button>
                                    <label class="agent-menu-row">
                                        <span>{move || t(locale.get(), "settings.nav.memory")}</span>
                                        <span class="toggle agent-menu-toggle">
                                            <input type="checkbox" prop:checked=move || memory_view.get().map(|view| view.enabled).unwrap_or(true)
                                                on:change=move |ev| {
                                                    let enabled = event_target_checked(&ev);
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
                                                        if let Ok(value) = invoke_checked("set_memory_enabled", arg).await {
                                                            if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(value) {
                                                                memory_view.set(Some(view));
                                                            }
                                                        }
                                                    });
                                                } />
                                            <span class="toggle-track" aria-hidden="true"></span>
                                        </span>
                                    </label>
                                    <div class="agent-menu-separator"></div>
                                    <button type="button" class="agent-menu-row" aria-haspopup="menu"
                                        disabled=locked
                                        title=move || locked.then(|| t(locale.get(), "composer.specialist.locked")).unwrap_or_default()
                                        on:click=move |_| {
                                            specialist_menu_open.update(|open| *open = !*open);
                                            reviewer_model_menu_open.set(false);
                                            compute_menu_open.set(false);
                                        }>
                                        <span>{move || t(locale.get(), "composer.specialist")}</span>
                                        <span class="agent-menu-value">{move || session_specialist.get()
                                            .map(|specialist| specialist.name)
                                            .unwrap_or_else(|| t(locale.get(), "composer.specialist.none"))}</span>
                                        <span class="agent-menu-chevron">{compose_icon("chevron-right")}</span>
                                    </button>
                                    <button type="button" class="agent-menu-row" aria-haspopup="menu"
                                        on:click=move |_| {
                                            compute_menu_open.update(|open| *open = !*open);
                                            reviewer_model_menu_open.set(false);
                                            specialist_menu_open.set(false);
                                        }>
                                        <span>{move || t(locale.get(), "composer.compute")}</span>
                                        <span class="agent-menu-value">{move || {
                                            let count = session_execution_contexts.get().len();
                                            if count == 0 { t(locale.get(), "compute.default_local") }
                                            else { tf(locale.get(), "composer.compute_count", &[("n", &count.to_string())]) }
                                        }}</span>
                                        <span class="agent-menu-chevron">{compose_icon("chevron-right")}</span>
                                    </button>

                                    {move || reviewer_model_menu_open.get().then(|| view! {
                                        <div class="compose-menu agent-submenu reviewer-model-menu" role="menu"
                                            aria-label=move || t(locale.get(), "composer.reviewer_model")>
                                            {{
                                                let mut choices = vec![(
                                                    "http:".to_string(),
                                                    t(locale.get(), "composer.reviewer.default_http"),
                                                ), (
                                                    "follow_session".to_string(),
                                                    t(locale.get(), "composer.reviewer.follow_session"),
                                                )];
                                                choices.extend(models.get().into_iter().map(|model| {
                                                    (format!("http:{}", model.id), model.label)
                                                }));
                                                choices.extend(acp_agents.get().into_iter().map(|agent| {
                                                    (format!("acp:{}", agent.id), format!("{} · ACP", agent.label))
                                                }));
                                                choices.into_iter().map(|(backend_key, label)| {
                                                    let selected_key = backend_key.clone();
                                                    let current = specialists.get().into_iter()
                                                        .find(|specialist| specialist.id == "reviewer")
                                                        .map(|reviewer| reviewer_backend_key(&reviewer))
                                                        .unwrap_or_default();
                                                    view! {
                                                        <button type="button" class="agent-submenu-row"
                                                            on:click=move |_| {
                                                                let Some(mut reviewer) = specialists.get_untracked().into_iter()
                                                                    .find(|specialist| specialist.id == "reviewer") else { return; };
                                                                set_reviewer_backend(&mut reviewer, &selected_key);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "spec": reviewer })).unwrap();
                                                                    if let Ok(value) = invoke_checked("save_specialist_cmd", arg).await {
                                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(value) {
                                                                            specialists.set(list);
                                                                        }
                                                                    }
                                                                });
                                                                agent_menu_open.set(false);
                                                                reviewer_model_menu_open.set(false);
                                                            }>
                                                            <span>{label}</span>
                                                            {(current == backend_key).then(|| view! { <span class="agent-menu-check">{compose_icon("check")}</span> })}
                                                        </button>
                                                    }
                                                }).collect_view()
                                            }}
                                        </div>
                                    })}
                                    {move || specialist_menu_open.get().then(|| view! {
                                        <div class="compose-menu agent-submenu specialist-menu" role="menu"
                                            aria-label=move || t(locale.get(), "composer.specialist")>
                                            <button type="button" class="agent-submenu-row" on:click=move |_| {
                                                agent_menu_open.set(false);
                                                specialist_menu_open.set(false);
                                                pick_specialist(String::new());
                                            }>
                                                <span>{move || t(locale.get(), "composer.specialist.none")}</span>
                                                {move || session_specialist.get().is_none().then(|| view! { <span class="agent-menu-check">{compose_icon("check")}</span> })}
                                            </button>
                                            {move || specialists.get().into_iter().filter(|specialist| specialist.id != "reviewer" && specialist.id != "reader").map(|specialist| {
                                                let id = specialist.id.clone();
                                                let selected_id = id.clone();
                                                view! {
                                                    <button type="button" class="agent-submenu-row" on:click=move |_| {
                                                        agent_menu_open.set(false);
                                                        specialist_menu_open.set(false);
                                                        pick_specialist(id.clone());
                                                    }>
                                                        <span>{specialist.name}</span>
                                                        {move || session_specialist.get().as_ref().is_some_and(|current| current.id == selected_id)
                                                            .then(|| view! { <span class="agent-menu-check">{compose_icon("check")}</span> })}
                                                    </button>
                                                }
                                            }).collect_view()}
                                        </div>
                                    })}
                                    {move || compute_menu_open.get().then(|| view! {
                                        <div class="compose-menu agent-submenu compute-menu" role="menu"
                                            aria-label=move || t(locale.get(), "composer.compute")>
                                            <button type="button" class="agent-submenu-row" on:click=move |_| {
                                                agent_menu_open.set(false);
                                                compute_menu_open.set(false);
                                                open_add_host_form.call(());
                                            }>
                                                <span>{move || t(locale.get(), "compute.add_host")}</span>
                                            </button>
                                            <div class="compute-menu-search">
                                                {compose_icon("search")}
                                                <input type="search" inputmode="search" autocomplete="off"
                                                    aria-label=move || t(locale.get(), "compute.search")
                                                    placeholder=move || t(locale.get(), "compute.search")
                                                    prop:value=move || compute_search.get()
                                                    on:input=move |ev| compute_search.set(event_target_value(&ev)) />
                                            </div>
                                            <div class="agent-menu-separator"></div>
                                            <div class="compute-resource-list">
                                                {move || {
                                                    let query = compute_search.get().trim().to_lowercase();
                                                    ssh_hosts.get().into_iter().filter(|host| {
                                                        query.is_empty() || host.alias.to_lowercase().contains(&query)
                                                    }).map(|host| {
                                                    let context_id = format!("ssh:{}", host.alias);
                                                    let enabled = session_execution_contexts.get().contains(&context_id);
                                                    let toggle_id = context_id.clone();
                                                    view! {
                                                        <button type="button" class="agent-submenu-row compute-resource-row"
                                                            class:enabled=enabled data-context-id=context_id.clone()
                                                            aria-pressed=enabled.to_string()
                                                            on:click=move |_| {
                                                                toggle_session_compute_resource.call((toggle_id.clone(), !enabled));
                                                            }>
                                                            <span class="compute-resource-icon">{compose_icon("server")}</span>
                                                            <span class="compute-resource-name">{host.alias}</span>
                                                            <span class="compute-resource-state">
                                                                {if enabled { t(locale.get(), "compute.enabled") } else { t(locale.get(), "compute.disabled") }}
                                                            </span>
                                                        </button>
                                                    }
                                                }).collect_view()}}
                                                {move || {
                                                    let query = compute_search.get().trim().to_lowercase();
                                                    execution_contexts.get().into_iter()
                                                        .filter(|ctx| ctx.kind == "wsl")
                                                        .filter(|ctx| query.is_empty() || ctx.label.to_lowercase().contains(&query))
                                                        .map(|ctx| {
                                                    let context_id = ctx.id.clone();
                                                    let enabled = session_execution_contexts.get().contains(&context_id);
                                                    let toggle_id = context_id.clone();
                                                    let name = if ctx.label.trim().is_empty() { ctx.id.clone() } else { ctx.label.clone() };
                                                    let is_default = serde_json::from_str::<serde_json::Value>(&ctx.config_json)
                                                        .ok()
                                                        .and_then(|cfg| cfg.get("is_default").and_then(|v| v.as_bool()))
                                                        .unwrap_or(false);
                                                    view! {
                                                        <button type="button" class="agent-submenu-row compute-resource-row"
                                                            class:enabled=enabled data-context-id=context_id.clone()
                                                            aria-pressed=enabled.to_string()
                                                            on:click=move |_| {
                                                                toggle_session_compute_resource.call((toggle_id.clone(), !enabled));
                                                            }>
                                                            <span class="compute-resource-icon">{compose_icon("terminal")}</span>
                                                            <span class="compute-resource-name-wrap">
                                                                <span class="compute-resource-name">{name}</span>
                                                                {is_default.then(|| view! {
                                                                    <span class="compute-resource-default">{t(locale.get(), "compute.default")}</span>
                                                                })}
                                                            </span>
                                                            <span class="compute-resource-state">
                                                                {if enabled { t(locale.get(), "compute.enabled") } else { t(locale.get(), "compute.disabled") }}
                                                            </span>
                                                        </button>
                                                    }
                                                }).collect_view()}}
                                            </div>
                                            <button type="button" class="agent-submenu-row compute-manage-row"
                                                on:click=move |_| {
                                                    agent_menu_open.set(false);
                                                    compute_menu_open.set(false);
                                                    settings_section.set("environments".into());
                                                    show_settings.set(true);
                                                }>
                                                <span>{move || t(locale.get(), "compute.manage")}</span>
                                            </button>
                                        </div>
                                    })}
                                </div>
                                }
                            })}
                        </div>
                        <div class="composer-buttons">
                            {move || (!models.get().is_empty() || !acp_agents.get().is_empty()).then(|| view! {
                                <div class="model-picker">
                                    <button type="button" class="model-picker-btn" class:active=move || model_menu_open.get()
                                        on:click=move |_| model_menu_open.update(|o| *o = !*o)>
                                        <span class="model-picker-label">{move || {
                                            if let Some(id) = active_acp_agent_id.get() {
                                                acp_agents.get().into_iter().find(|agent| agent.id == id).map(|agent| agent.label).unwrap_or_else(|| "ACP Agent".into())
                                            } else {
                                                let l = models.get();
                                                let selected = active_session.get().and_then(|session_id| {
                                                    session_model_ids.with(|models| models.get(&session_id).cloned())
                                                });
                                                model_label(&l, selected.as_deref()).unwrap_or_default()
                                            }
                                        }}</span>
                                        <span class="model-picker-chev">"▾"</span>
                                    </button>
                                    {move || model_menu_open.get().then(|| view! {
                                        <div class="model-menu-backdrop" on:click=move |_| model_menu_open.set(false)></div>
                                        <div class="model-menu">
                                            {move || {
                                                let list = models.get();
                                                let selected = active_session.get().and_then(|session_id| {
                                                    session_model_ids.with(|models| models.get(&session_id).cloned())
                                                });
                                                let acp_selected = active_acp_agent_id.get().is_some();
                                                let acp_locked = acp_selected && items.with(|rows| !rows.is_empty());
                                                list.into_iter().map(|m| {
                                                    let pick_id = m.id.clone();
                                                    let pick_label = m.label.clone();
                                                    let is_active = !acp_selected
                                                        && selected.as_deref().map_or(m.active, |id| id == m.id);
                                                    let show_sub = !m.model.is_empty() && m.model != m.label;
                                                    view! {
                                                        <div class="model-menu-row" class:active=is_active>
                                                            <button type="button" class="model-menu-pick"
                                                                disabled=acp_locked
                                                                on:click=move |_| {
                                                                if acp_locked {
                                                                    show_warning_toast(&t(locale.get(), "models.locked_hint"));
                                                                    return;
                                                                }
                                                                model_menu_open.set(false);
                                                                if is_active {
                                                                    return;
                                                                }
                                                                let id = pick_id.clone();
                                                                if model_switch_warning_disabled() || items.with(|rows| rows.is_empty()) {
                                                                    switch_http_model.call((id, false));
                                                                } else {
                                                                    model_switch_confirm.set(Some((id, pick_label.clone())));
                                                                }
                                                            }>
                                                                <span class="model-menu-text">
                                                                    <span class="model-menu-label">{m.label.clone()}</span>
                                                                    {show_sub.then(|| view! { <span class="model-menu-sub">{m.model.clone()}</span> })}
                                                                </span>
                                                                {is_active.then(|| view! { <span class="model-menu-check">"✓"</span> })}
                                                            </button>
                                                        </div>
                                                    }
                                                }).collect_view()
                                            }}
                                            {move || (!acp_agents.get().is_empty()).then(|| view! {
                                                <div class="compose-group-label">"ACP Agents"</div>
                                                {acp_agents.get().into_iter().map(|agent| {
                                                    let id = agent.id.clone();
                                                    let active = active_acp_agent_id.get().as_deref() == Some(agent.id.as_str());
                                                    let starts_new_session = items.with(|rows| !rows.is_empty()) && !active;
                                                    view! {
                                                        <div class="model-menu-row" class:active=active>
                                                            <button type="button" class="model-menu-pick"
                                                                title=starts_new_session.then_some("Start a new session with this ACP Agent")
                                                                on:click=move |_| {
                                                                    model_menu_open.set(false);
                                                                    if !starts_new_session {
                                                                        if let Some(frame_id) = active_session.get_untracked() {
                                                                            provisional_acp_selection.set(Some((frame_id, id.clone())));
                                                                        }
                                                                        active_acp_agent_id.set(Some(id.clone()));
                                                                        return;
                                                                    }
                                                                    let agent_id = id.clone();
                                                                    demo_mode.set(false);
                                                                    if let Some(old) = active_session.get_untracked() {
                                                                        transcripts.update(|cache| {
                                                                            cache.insert(old, items.get_untracked());
                                                                        });
                                                                    }
                                                                    sel_artifact.set(0);
                                                                    right_tab.set(RightTab::Artifacts);
                                                                    spawn_local(async move {
                                                                        let Some(frame_id) = invoke("new_session", JsValue::UNDEFINED).await.as_string() else {
                                                                            status.set(t(locale.get(), "status.send_failed").into());
                                                                            return;
                                                                        };
                                                                        provisional_acp_selection.set(Some((frame_id.clone(), agent_id.clone())));
                                                                        active_acp_agent_id.set(Some(agent_id));
                                                                        active_session.set(Some(frame_id));
                                                                        items.set(vec![]);
                                                                        refresh_session_history();
                                                                        focus_composer();
                                                                        show_toast(&t(locale.get(), "composer.acp_new_session_toast"));
                                                                    });
                                                                }>
                                                                <span class="model-menu-text">
                                                                    <span class="model-menu-label">{agent.label.clone()}</span>
                                                                    <span class="model-menu-sub">"ACP · local stdio"</span>
                                                                </span>
                                                                {active.then(|| view! { <span class="model-menu-check">"✓"</span> })}
                                                            </button>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            })}
                                            {move || active_acp_agent_id.get().is_none().then(|| view! {
                                                <div class="model-menu-effort" on:click=|ev| ev.stop_propagation()>
                                                    <span class="model-menu-effort-label">{move || t(locale.get(), "settings.reasoning_effort")}</span>
                                                    <select class="model-menu-effort-select"
                                                        on:change=move |ev| {
                                                            let v = dom_value(&ev);
                                                            let effort = if v == "default" { String::new() } else { v };
                                                            let Some(m) = models.get_untracked().into_iter().find(|m| m.active) else { return; };
                                                            let profile = serde_json::json!({
                                                                "id": m.id,
                                                                "label": m.label,
                                                                "provider": m.provider,
                                                                "api_url": m.api_url,
                                                                "model": m.model,
                                                                "max_tokens": m.max_tokens,
                                                                "reasoning_effort": effort,
                                                                "supports_vision": m.supports_vision,
                                                                "use_for_vision": m.use_for_vision,
                                                            });
                                                            let use_for_vision = m.use_for_vision;
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({
                                                                    "profile": profile,
                                                                    "key": Option::<String>::None,
                                                                    "useForVision": use_for_vision,
                                                                })).unwrap();
                                                                if let Ok(v) = invoke_checked("save_model", arg).await {
                                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                        models.set(list);
                                                                    }
                                                                }
                                                            });
                                                        }>
                                                        <option value="default"
                                                            prop:selected=move || models.get().iter().find(|m| m.active).map(|m| m.reasoning_effort.is_empty()).unwrap_or(true)>
                                                            {move || t(locale.get(), "settings.reasoning_effort.default")}
                                                        </option>
                                                        {["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"].into_iter().map(|lvl| view! {
                                                            <option value=lvl
                                                                prop:selected=move || models.get().iter().find(|m| m.active).is_some_and(|m| m.reasoning_effort == lvl)>
                                                                {lvl}
                                                            </option>
                                                        }).collect_view()}
                                                    </select>
                                                </div>
                                            })}
                                            <button type="button" class="model-menu-add" on:click=move |_| {
                                                model_menu_open.set(false);
                                                open_settings_fn(Some("models".into()));
                                                show_acp_agents.set(false);
                                                acp_form.set(None);
                                                model_form.set(None);
                                                model_form_key.set(String::new());
                                                model_form_msg.set(None);
                                                acp_form_msg.set(None);
                                            }>{move || t(locale.get(), "models.manage")}</button>
                                        </div>
                                    })}
                                </div>
                            })}
                            {move || busy.get().then(|| view! {
                                <button type="button" class="stop"
                                    disabled=move || active_session.get() == stopping_session.get()
                                    on:click=stop>
                                    {move || t(locale.get(), if active_session.get() == stopping_session.get() { "composer.stopping" } else { "composer.stop" })}
                                </button>
                            })}
                            <div class="send-split">
                                <button class="send" disabled=composer_blocked on:click=move |_| send.call(ComposerSendAction::Normal)>
                                    {move || t(locale.get(), if busy.get() { "composer.queue_button" } else { "composer.send" })}
                                </button>
                                <button type="button" class="send-menu-toggle"
                                    disabled=composer_blocked
                                    aria-label=move || t(locale.get(), "composer.send_options")
                                    title=move || t(locale.get(), "composer.send_options")
                                    on:click=move |_| send_mode_menu_open.update(|o| *o = !*o)>
                                    {compose_icon("chevron-down")}
                                </button>
                                {move || send_mode_menu_open.get().then(|| view! {
                                    <div class="send-menu-backdrop" on:click=move |_| send_mode_menu_open.set(false)></div>
                                    <div class="send-mode-menu">
                                        {move || (busy.get() && active_acp_agent_id.get().is_none()).then(|| view! {
                                            <button type="button" class="send-mode-item"
                                                disabled=composer_blocked
                                                on:click=move |_| {
                                                    send_mode_menu_open.set(false);
                                                    send.call(ComposerSendAction::GuideAppend);
                                                }>
                                                <span class="compose-item-icon">{compose_icon("up")}</span>
                                                <span>{move || t(locale.get(), "composer.cut_in_now")}</span>
                                            </button>
                                        })}
                                        {move || busy.get().then(|| view! {
                                            <button type="button" class="send-mode-item"
                                                disabled=composer_blocked
                                                on:click=move |_| {
                                                    send_mode_menu_open.set(false);
                                                    send.call(ComposerSendAction::InterruptReplace);
                                                }>
                                                <span class="compose-item-icon">{compose_icon("sync")}</span>
                                                <span>{move || t(locale.get(), "composer.interrupt_replace")}</span>
                                            </button>
                                        })}
                                        <button type="button" class="send-mode-item"
                                            disabled=move || side_chat_busy.get()
                                            on:click=move |_| {
                                                send_mode_menu_open.set(false);
                                                let q = message_with_attachments(&input.get(), &attachment_paths(&attachments.get()));
                                                if q.trim().is_empty() {
                                                    ensure_right_tab(
                                                        RightTab::SideChat,
                                                        show_right,
                                                        open_right_tabs,
                                                        right_tab,
                                                    );
                                                } else {
                                                    input.set(String::new());
                                                    attachments.set(vec![]);
                                                    send_side_chat(q);
                                                }
                                            }>
                                            <span class="compose-item-icon">{compose_icon("chat")}</span>
                                            <span>{move || t(locale.get(), "composer.side_chat")}</span>
                                        </button>
                                        <button type="button" class="send-mode-item"
                                            on:click=move |_| {
                                                send_mode_menu_open.set(false);
                                                send.call(ComposerSendAction::BranchNew);
                                            }>
                                            <span class="compose-item-icon">{compose_icon("branch")}</span>
                                            <span>{move || t(locale.get(), "composer.branch_session")}</span>
                                        </button>
                                    </div>
                                })}
                            </div>
                        </div>
                    </div>
                    <div class="composer-hint">{move || {
                        if send_with_modifier.get() {
                            tf(
                                locale.get(),
                                "composer.hint_modifier",
                                &[("modifier", if is_mac() { "Cmd" } else { "Ctrl" })],
                            )
                        } else {
                            t(locale.get(), "composer.hint").into()
                        }
                    }}</div>
                </div>
            </div>
        </main>

        {move || show_right.get().then(|| view! {
            <div class="resizer" on:mousedown=on_resize_start></div>
            <button type="button" class="rightpane-backdrop"
                aria-label=move || t(locale.get(), "right.close")
                on:click=move |_| show_right.set(false)></button>
            <section class="rightpane" style=move || {
                let width = right_w
                    .get()
                    .min(max_right_pane_width(show_sidebar.get(), sidebar_w.get()));
                format!("width:{width}px")
            }>
                <div class="rp-tabs">
                    <div class="rp-tab-scroll">
                        {move || {
                            let loc = locale.get();
                            let active = right_tab.get();
                            let art_n = artifacts.get().len();
                            let notebook_n = notebook_cells.get().len();
                            let prov_n = items.get().iter().filter(|i| matches!(i, ChatItem::Tool { .. })).count();
                            let highlight_n = session_highlight_count(active_session.get(), &library_items.get());
                            open_right_tabs.get().into_iter().map(|tab| {
                                let label = match tab {
                                    RightTab::Artifacts => tab_count(loc, "right.artifacts", art_n),
                                    RightTab::Agents => t(loc, "right.agents").into(),
                                    RightTab::Notebook => tab_count(loc, "right.notebook", notebook_n),
                                    RightTab::Highlights => tab_count(loc, "right.highlights", highlight_n),
                                    RightTab::Provenance => tab_count(loc, "right.provenance", prov_n),
                                    RightTab::File => t(loc, "right.file").into(),
                                    RightTab::Hosts => t(loc, "contexts.title").into(),
                                    RightTab::SideChat => t(loc, "sidechat.title").into(),
                                };
                                let is_active = active == tab;
                                view! {
                                    // Drag state is only read in per-element class closures and
                                    // handlers — reading it in the list closure above would
                                    // rebuild the strip mid-drag and abort the native drag.
                                    <div class="rp-tab-wrap"
                                        attr:draggable="true"
                                        class:dragging=move || rp_tab_drag.get() == Some(tab)
                                        class:drop-target=move || rp_tab_drop.get() == Some(tab)
                                        on:dragstart=move |ev: web_sys::DragEvent| {
                                            ev.stop_propagation();
                                            if let Some(dt) = ev.data_transfer() {
                                                let _ = dt.set_effect_allowed("move");
                                                let _ = dt.set_data("text/plain", "rp-tab");
                                            }
                                            rp_tab_drag.set(Some(tab));
                                        }
                                        on:dragend=move |_| {
                                            rp_tab_drag.set(None);
                                            rp_tab_drop.set(None);
                                        }
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            if rp_tab_drag.get_untracked().is_none() { return; }
                                            allow_drop(&ev);
                                            if rp_tab_drop.get_untracked() != Some(tab) {
                                                rp_tab_drop.set(Some(tab));
                                            }
                                        }
                                        on:drop=move |ev: web_sys::DragEvent| {
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            let src = rp_tab_drag.get_untracked();
                                            rp_tab_drag.set(None);
                                            rp_tab_drop.set(None);
                                            let Some(src) = src else { return; };
                                            if src == tab { return; }
                                            open_right_tabs.update(|tabs| {
                                                let (Some(from), Some(to)) = (
                                                    tabs.iter().position(|t| *t == src),
                                                    tabs.iter().position(|t| *t == tab),
                                                ) else { return; };
                                                let moved = tabs.remove(from);
                                                // Removal shifts the target left on rightward
                                                // drags, so inserting at its original index
                                                // lands after it — and before it on leftward
                                                // drags, matching native tab strips.
                                                tabs.insert(to, moved);
                                            });
                                        }>
                                        <button type="button" class="rp-tab" class:active=is_active
                                            on:click=move |_| {
                                                right_tab.set(tab);
                                                match tab {
                                                    RightTab::File => refresh_active_file_dir(
                                                        file_source,
                                                        file_cwd,
                                                        file_entries,
                                                        remote_file_cwd,
                                                        remote_file_entries,
                                                        remote_file_loading,
                                                        remote_file_error,
                                                    ),
                                                    RightTab::Hosts => {
                                                        refresh_execution_contexts(execution_contexts);
                                                        refresh_runtimes(runtime_infos);
                                                        refresh_runs(run_records, locale);
                                                    }
                                                    RightTab::Agents => {
                                                        refresh_agent_workflows(agent_panel)
                                                    }
                                                    _ => {}
                                                }
                                            }>{label}</button>
                                        <button type="button" class="rp-tab-close"
                                            aria-label=move || t(locale.get(), "right.close_tab")
                                            on:click=move |ev| {
                                                ev.stop_propagation();
                                                close_right_tab(tab, show_right, open_right_tabs, right_tab);
                                            }>{compose_icon("close")}</button>
                                    </div>
                                }.into_view()
                            }).collect_view()
                        }}
                    </div>
                    <div class="rp-tab-add-wrap">
                        <button type="button" class="rp-tab-add"
                            aria-label=move || t(locale.get(), "right.add_tab")
                            class:active=move || right_tab_add_menu_open.get()
                            on:click=move |_| right_tab_add_menu_open.update(|o| *o = !*o)>{compose_icon("plus")}</button>
                        {move || right_tab_add_menu_open.get().then(|| view! {
                            <div class="rp-tab-add-backdrop" on:click=move |_| right_tab_add_menu_open.set(false)></div>
                            <div class="rp-tab-add-menu">
                                {move || {
                                    let loc = locale.get();
                                    let open = open_right_tabs.get();
                                    let art_n = artifacts.get().len();
                                    let notebook_n = notebook_cells.get().len();
                                    let prov_n = items.get().iter().filter(|i| matches!(i, ChatItem::Tool { .. })).count();
                                    let highlight_n = session_highlight_count(active_session.get(), &library_items.get());
                                    ALL_RIGHT_TABS.iter().copied().map(|tab| {
                                        let label = match tab {
                                            RightTab::Artifacts => tab_count(loc, "right.artifacts", art_n),
                                            RightTab::Agents => t(loc, "right.agents").into(),
                                            RightTab::Notebook => tab_count(loc, "right.notebook", notebook_n),
                                            RightTab::Highlights => tab_count(loc, "right.highlights", highlight_n),
                                            RightTab::Provenance => tab_count(loc, "right.provenance", prov_n),
                                            RightTab::File => t(loc, "right.file").into(),
                                            RightTab::Hosts => t(loc, "contexts.title").into(),
                                            RightTab::SideChat => t(loc, "sidechat.title").into(),
                                        };
                                        let is_open = open.contains(&tab);
                                        view! {
                                            <button type="button" class="rp-tab-add-item" class:open=is_open
                                                on:click=move |_| {
                                                    right_tab_add_menu_open.set(false);
                                                    ensure_right_tab(tab, show_right, open_right_tabs, right_tab);
                                                    match tab {
                                                        RightTab::File => refresh_active_file_dir(
                                                            file_source,
                                                            file_cwd,
                                                            file_entries,
                                                            remote_file_cwd,
                                                            remote_file_entries,
                                                            remote_file_loading,
                                                            remote_file_error,
                                                        ),
                                                        RightTab::Hosts => {
                                                            refresh_execution_contexts(execution_contexts);
                                                            refresh_runtimes(runtime_infos);
                                                            refresh_runs(run_records, locale);
                                                        }
                                                        RightTab::Agents => {
                                                            refresh_agent_workflows(agent_panel)
                                                        }
                                                        _ => {}
                                                    }
                                                }>
                                                <span>{label}</span>
                                                {is_open.then(|| view! { <span>"✓"</span> })}
                                            </button>
                                        }.into_view()
                                    }).collect_view()
                                }}
                            </div>
                        })}
                    </div>
                    {move || matches!(right_tab.get(), RightTab::Artifacts | RightTab::File).then(|| view! {
                        <div class="rp-view-modes" role="group">
                            <button type="button" class="rp-view-mode" class:active=move || !rp_grid.get()
                                title=move || t(locale.get(), "right.view.list")
                                aria-pressed=move || (!rp_grid.get()).to_string()
                                on:click=move |_| rp_grid.set(false)>{compose_icon("list")}</button>
                            <button type="button" class="rp-view-mode" class:active=move || rp_grid.get()
                                title=move || t(locale.get(), "right.view.grid")
                                aria-pressed=move || rp_grid.get().to_string()
                                on:click=move |_| rp_grid.set(true)>{compose_icon("grid")}</button>
                        </div>
                    })}
                    <button class="icon-btn" title=move || t(locale.get(), "right.close") on:click=move |_| show_right.set(false)>{compose_icon("close")}</button>
                </div>
                <div class="rp-doc">
                    {move || match right_tab.get() {
                        RightTab::Artifacts => {
                            let arts = artifacts.get();
                            let loc = locale.get();
                            if arts.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">{t(loc, "right.no_artifacts.title")}</div>
                                        <p>{t(loc, "right.no_artifacts.body")}</p>
                                    </div>
                                }.into_view()
                            } else {
                                // Build the tile list from `arts` only — do NOT read
                                // `sel_artifact` in this (outer) scope, or selecting a
                                // tile re-runs the whole branch and rebuilds `.rp-tiles`,
                                // resetting its scroll to the top (#25). Selection is
                                // isolated to the `.active` class and the nested `.rp-view`
                                // closure below, so the scroll container is preserved.
                                let groups = group_artifact_indices(&arts);
                                let tile_groups = groups.into_iter().map(|(key, indices)| {
                                    let label = artifact_group_label(&key, loc);
                                    let count = indices.len();
                                    let key_toggle = key.clone();
                                    let key_class = key.clone();
                                    let key_aria = key.clone();
                                    let tiles = indices.into_iter().map(|i| {
                                        let a = &arts[i];
                                        let name = a.name.clone();
                                        let kind = a.kind.to_string();
                                        let meta = artifact_meta(a, loc);
                                        let file = if let PreviewData::File { path, kind } = &a.data {
                                            Some((path.clone(), kind.clone()))
                                        } else {
                                            None
                                        };
                                        let file_click = file.clone();
                                        let context_path = file.as_ref().map(|(path, _)| path.clone()).unwrap_or_default();
                                        let name_click = name.clone();
                                        let tools = file.map(|(path, fkind)| {
                                        let (dl, vn) = (path.clone(), name.clone());
                                        view! {
                                            <div class="rp-tile-tools">
                                                <button type="button" class="rp-tile-tool"
                                                    title=move || t(locale.get(), "artifact.download")
                                                    on:click=move |ev| { ev.stop_propagation(); download_artifact(dl.clone()); }>{compose_icon("download")}</button>
                                                <button type="button" class="rp-tile-tool"
                                                    title=move || t(locale.get(), "artifact.more")
                                                    on:click=move |ev: web_sys::MouseEvent| {
                                                        ev.stop_propagation();
                                                        let open = matches!(artifact_menu.get(), Some((mi, _, _)) if mi == i);
                                                        artifact_menu.set(if open { None } else { Some((i, ev.client_x(), ev.client_y())) });
                                                    }>{compose_icon("more")}</button>
                                            </div>
                                            {move || {
                                                let (mi, cx, cy) = artifact_menu.get()?;
                                                (mi == i).then(|| {
                                                let (p, n, k) = (path.clone(), vn.clone(), fkind.clone());
                                                let (mv, sp, dw) = (p.clone(), p.clone(), p.clone());
                                                let rv = p.clone();
                                                let oc = CenterFileTab::new(p.clone(), n.clone(), k.clone());
                                                let (mvn, mvk) = (n.clone(), k.clone());
                                                view! {
                                                    <div class="rp-tile-menu-backdrop" on:click=move |_| artifact_menu.set(None)></div>
                                                    <div class="rp-tile-menu"
                                                        style=format!("right:calc(100vw - {cx}px);top:{cy}px")>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); modal_artifact.set(Some((mv.clone(), mvn.clone(), mvk.clone()))); }>
                                                            {move || t(locale.get(), "artifact.open_viewer")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| {
                                                                artifact_menu.set(None);
                                                                center_files.update(|files| {
                                                                    if !files.iter().any(|file| file.path == oc.path) {
                                                                        files.push(oc.clone());
                                                                    }
                                                                });
                                                                center_file.set(Some(oc.path.clone()));
                                                            }>
                                                            {move || t(locale.get(), "center.open_file")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| {
                                                                artifact_menu.set(None);
                                                                reveal_in_files(&sp, file_source, file_cwd, file_query, file_entries, show_right, open_right_tabs, right_tab);
                                                            }>
                                                            {move || t(locale.get(), "artifact.reveal_in_files")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| {
                                                                artifact_menu.set(None);
                                                                ensure_right_tab(
                                                                    RightTab::Provenance,
                                                                    show_right,
                                                                    open_right_tabs,
                                                                    right_tab,
                                                                );
                                                            }>
                                                            {move || t(locale.get(), "artifact.provenance")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); download_artifact(dw.clone()); }>
                                                            {move || t(locale.get(), "artifact.download")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); reveal_in_file_manager(rv.clone()); }>
                                                            {move || t(locale.get(), "ctx.reveal_in_manager")}</button>
                                                    </div>
                                                }
                                            })
                                            }}
                                        }.into_view()
                                    });
                                    view! {
                                        <div class="rp-tile" class:active=move || sel_artifact.get() == i
                                            data-artifact-name=name.clone()
                                            data-artifact-path=context_path>
                                            <button type="button" class="rp-tile-main"
                                                on:click=move |_| {
                                                    artifact_menu.set(None);
                                                    if let Some((path, kind)) = &file_click {
                                                        if opens_in_modal(kind) {
                                                            modal_artifact.set(Some((path.clone(), name_click.clone(), kind.clone())));
                                                            return;
                                                        }
                                                    }
                                                    sel_artifact.set(i);
                                                    show_art_preview.set(true);
                                                }>
                                                <span class="rp-tile-text">
                                                    <span class="rp-tile-name">{name}</span>
                                                    <span class="rp-tile-meta">{meta}</span>
                                                </span>
                                                <span class=format!("rp-badge {}", kind)>{kind.clone()}</span>
                                            </button>
                                            {tools}
                                        </div>
                                    }.into_view()
                                    }).collect_view();
                                    view! {
                                        <div class="rp-art-group"
                                            class:collapsed=move || collapsed_art_groups.get().contains(&key_class)
                                            data-art-group=key.clone()>
                                            <button type="button" class="rp-art-group-label"
                                                aria-expanded=move || (!collapsed_art_groups.get().contains(&key_aria)).to_string()
                                                on:click=move |_| {
                                                    collapsed_art_groups.update(|set| {
                                                        if set.contains(&key_toggle) { set.remove(&key_toggle); }
                                                        else { set.insert(key_toggle.clone()); }
                                                    });
                                                }>
                                                <span class="rp-art-group-caret">"▾"</span>
                                                <span class="rp-art-group-name">{label}</span>
                                                <span class="rp-art-group-count">{count}</span>
                                            </button>
                                            <div class="rp-art-group-items">{tiles}</div>
                                        </div>
                                    }.into_view()
                                }).collect_view();
                                let arts_for_view = arts.clone();
                                view! {
                                    <div class="rp-artifacts-body" class:preview-hidden=move || !show_art_preview.get()>
                                        <div class="rp-tiles" class:grid=move || rp_grid.get()>{tile_groups}</div>
                                        {move || show_art_preview.get().then(|| {
                                            let arts = arts_for_view.clone();
                                            let sel = sel_artifact.get().min(arts.len().saturating_sub(1));
                                            let cur = arts[sel].clone();
                                            let dom_id = format!("rp-{sel}");
                                            // image/pdf/csv aren't rendered inline — offer the modal viewer.
                                            let modal_file = if let PreviewData::File { path, kind } = &cur.data {
                                                opens_in_modal(kind).then(|| (path.clone(), cur.name.clone(), kind.clone()))
                                            } else {
                                                None
                                            };
                                            view! {
                                                <div class="rp-view">
                                                    <div class="rp-view-head">
                                                        <span class=format!("rp-badge {}", cur.kind)>{cur.kind.to_string()}</span>
                                                        <span class="rp-view-name">{cur.name.clone()}</span>
                                                        <div class="spacer"></div>
                                                        <button class="icon-btn" type="button"
                                                            title=move || t(locale.get(), "right.close_preview")
                                                            on:click=move |_| show_art_preview.set(false)>{compose_icon("close")}</button>
                                                    </div>
                                                    {match modal_file {
                                                        Some((p, n, k)) => view! {
                                                            <button class="rp-open-viewer" type="button"
                                                                on:click=move |_| modal_artifact.set(Some((p.clone(), n.clone(), k.clone())))>
                                                                {move || t(locale.get(), "artifact.open_viewer")}
                                                            </button>
                                                        }.into_view(),
                                                        None => artifact_preview(&cur, dom_id, loc).into_view(),
                                                    }}
                                                </div>
                                            }
                                        })}
                                    </div>
                                }.into_view()
                            }
                        }
                        RightTab::Agents => agent_workflows_panel(
                            agent_panel,
                            specialists,
                            models,
                            sessions,
                            delegation_enabled,
                            locale,
                            takeover_session.clone(),
                            refresh_agent_sessions.clone(),
                            agent_config_to_chat.clone(),
                        ).into_view(),
                        RightTab::Notebook => {
                            view! {
                                <NotebookView cells=notebook_cells.get() locale=locale.get()
                                    active_session=active_session.read_only()
                                    library_items=library_items.read_only()
                                    on_library_changed=refresh_library_items />
                            }.into_view()
                        }
                        RightTab::Highlights => {
                            let session = active_session.get();
                            let excerpts = library_items
                                .get()
                                .into_iter()
                                .filter(|item| {
                                    item.kind == "text"
                                        && session.as_deref()
                                            == Some(item.source_session_id.as_str())
                                })
                                .collect::<Vec<_>>();
                            view! {
                                <HighlightsPane locale=locale.get() excerpts=excerpts
                                    on_library_changed=refresh_library_items />
                            }.into_view()
                        }
                        RightTab::File => {
                            let loc = locale.get();
                            let ssh_contexts = execution_contexts
                                .get()
                                .into_iter()
                                .filter(|context| context.kind == "ssh")
                                .collect::<Vec<_>>();
                            view! {
                                <div class="rp-files">
                                    <label class="fb-source-label">
                                        <span>{t(loc, "files.source")}</span>
                                        <select class="fb-source" aria-label=t(loc, "files.source")
                                            prop:value=move || file_source.get()
                                            on:change=move |event| {
                                                let next = dom_value(&event);
                                                file_source.set(next.clone());
                                                file_query.set(String::new());
                                                if next == "local" {
                                                    refresh_dir(file_cwd, file_entries);
                                                } else {
                                                    remote_file_cwd.set("~".into());
                                                    refresh_remote_dir(
                                                        next,
                                                        remote_file_cwd,
                                                        remote_file_entries,
                                                        remote_file_loading,
                                                        remote_file_error,
                                                        file_source,
                                                    );
                                                }
                                            }>
                                            <option value="local">{t(loc, "files.local_project")}</option>
                                            {ssh_contexts.into_iter().map(|context| {
                                                let label = if context.label.trim().is_empty() {
                                                    context.id.trim_start_matches("ssh:").to_string()
                                                } else {
                                                    context.label
                                                };
                                                view! { <option value=context.id>{format!("{label} · SSH")}</option> }
                                            }).collect_view()}
                                        </select>
                                    </label>
                                    {move || {
                                        let source = file_source.get();
                                        if source == "local" {
                                        let cwd = file_cwd.get();
                                        let parent = if cwd == "." { None } else { Some(parent_path(&cwd)) };
                                        view! {
                                            <div class="fb-crumb">
                                                {parent.map(|p| {
                                                    let p_click = p.clone();
                                                    view! {
                                                        <button class="fb-up" on:click=move |_| {
                                                            file_query.set(String::new());
                                                            file_cwd.set(p_click.clone());
                                                            refresh_dir(file_cwd, file_entries);
                                                        }>{compose_icon("up")}</button>
                                                    }.into_view()
                                                })}
                                                <span class="fb-path">{cwd.clone()}</span>
                                            </div>
                                            <div class="fb-actions">
                                                <button type="button" on:click=move |_| {
                                                    file_entry_input.set(String::new());
                                                    file_entry_error.set(None);
                                                    file_entry_modal.set(Some(FileEntryModal::CreateFile));
                                                }>
                                                    {compose_icon("doc")}
                                                    <span>{t(loc, "files.new_file")}</span>
                                                </button>
                                                <button type="button" on:click=move |_| {
                                                    file_entry_input.set(String::new());
                                                    file_entry_error.set(None);
                                                    file_entry_modal.set(Some(FileEntryModal::CreateDirectory));
                                                }>
                                                    {compose_icon("folder")}
                                                    <span>{t(loc, "files.new_directory")}</span>
                                                </button>
                                                <button type="button" on:click=move |_| {
                                                    refresh_dir(file_cwd, file_entries);
                                                    if !file_query.get_untracked().trim().is_empty() {
                                                        refresh_file_search(file_query, file_search_hits);
                                                    }
                                                }>
                                                    {compose_icon("sync")}
                                                    <span>{t(loc, "files.refresh")}</span>
                                                </button>
                                            </div>
                                            <input class="fb-search" type="text"
                                                placeholder=move || t(locale.get(), "files.search")
                                                prop:value=move || file_query.get()
                                                on:input=move |ev| file_query.set(event_target_value(&ev)) />
                                            <div class="fb-list" class:grid=move || rp_grid.get()>
                                                {move || {
                                                    let q = file_query.get();
                                                    if !q.trim().is_empty() {
                                                        let hits = file_search_hits.get();
                                                        if hits.is_empty() {
                                                            return view! {
                                                                <div class="rp-empty rp-files-empty">
                                                                    <p>{t(loc, "files.no_matches")}</p>
                                                                </div>
                                                            }.into_view();
                                                        }
                                                        hits.into_iter().map(|hit| {
                                                            let name = hit.name.clone();
                                                            let path = hit.path.clone();
                                                            let dir = file_dir_label(&path);
                                                            if hit.is_dir {
                                                                let path_click = path.clone();
                                                                view! {
                                                                    <button class="fb-row dir" data-workspace-path=path.clone() on:click=move |_| {
                                                                        file_query.set(String::new());
                                                                        file_cwd.set(path_click.clone());
                                                                        refresh_dir(file_cwd, file_entries);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("folder")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                        <span class="fb-path-rel">{dir}</span>
                                                                    </button>
                                                                }.into_view()
                                                            } else {
                                                                let path_open = path.clone();
                                                                view! {
                                                                    <button class="fb-row" data-workspace-path=path.clone() on:click=move |_| {
                                                                        open_workspace_file(path_open.clone(), modal_artifact);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("doc")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                        <span class="fb-path-rel">{dir}</span>
                                                                        <span class="fb-size">{format_bytes(hit.size)}</span>
                                                                    </button>
                                                                }.into_view()
                                                            }
                                                        }).collect_view()
                                                    } else {
                                                        file_entries.get().into_iter().map(|e| {
                                                            let name = e.name.clone();
                                                            let full = join_path(&file_cwd.get(), &name);
                                                            if e.is_dir {
                                                                let full_click = full.clone();
                                                                view! {
                                                                    <button class="fb-row dir" data-workspace-path=full.clone() on:click=move |_| {
                                                                        file_query.set(String::new());
                                                                        file_cwd.set(full_click.clone());
                                                                        refresh_dir(file_cwd, file_entries);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("folder")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                    </button>
                                                                }.into_view()
                                                            } else {
                                                                let full_open = full.clone();
                                                                view! {
                                                                    <button class="fb-row" data-workspace-path=full.clone() on:click=move |_| {
                                                                        open_workspace_file(full_open.clone(), modal_artifact);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("doc")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                        <span class="fb-size">{format_bytes(e.size)}</span>
                                                                    </button>
                                                                }.into_view()
                                                            }
                                                        }).collect_view()
                                                    }
                                                }}
                                            </div>
                                            {move || project_info.get().map(|p| view! {
                                                <div class="hint fb-root">{tf(loc, "files.root", &[("path", &p.root)])}</div>
                                            })}
                                        }.into_view()
                                    } else {
                                        let cwd = remote_file_cwd.get();
                                        let parent = if cwd == "/" || cwd == "~" {
                                            None
                                        } else {
                                            Some(parent_path(&cwd))
                                        };
                                        let source_for_up = source.clone();
                                        let source_for_path = source.clone();
                                        view! {
                                            <div class="fb-crumb remote">
                                                {parent.map(|path| {
                                                    let path_click = path.clone();
                                                    let context_id = source_for_up.clone();
                                                    view! {
                                                        <button class="fb-up" aria-label=t(loc, "files.up") on:click=move |_| {
                                                            remote_file_cwd.set(path_click.clone());
                                                            refresh_remote_dir(
                                                                context_id.clone(),
                                                                remote_file_cwd,
                                                                remote_file_entries,
                                                                remote_file_loading,
                                                                remote_file_error,
                                                                file_source,
                                                            );
                                                        }>{compose_icon("up")}</button>
                                                    }.into_view()
                                                })}
                                                <input class="fb-path fb-path-input" type="text"
                                                    aria-label=t(loc, "files.go_to")
                                                    prop:value=move || remote_file_cwd.get()
                                                    on:input=move |event| remote_file_cwd.set(event_target_value(&event))
                                                    on:keydown=move |event: web_sys::KeyboardEvent| {
                                                        if event.key() == "Enter" {
                                                            event.prevent_default();
                                                            refresh_remote_dir(
                                                                source_for_path.clone(),
                                                                remote_file_cwd,
                                                                remote_file_entries,
                                                                remote_file_loading,
                                                                remote_file_error,
                                                                file_source,
                                                            );
                                                        }
                                                    } />
                                            </div>
                                            <div class="fb-list" class:grid=move || rp_grid.get()>
                                                {if remote_file_loading.get() {
                                                    view! { <div class="rp-empty rp-files-empty"><p>{t(loc, "loading")}</p></div> }.into_view()
                                                } else if let Some(error) = remote_file_error.get() {
                                                    let retry_context = source.clone();
                                                    let setup = is_ssh_setup_error(&error);
                                                    let jump_context = ssh_setup_context_id(
                                                        Some(source.as_str()),
                                                        &error,
                                                    );
                                                    view! {
                                                        <div class="rp-empty rp-files-empty fb-remote-error">
                                                            <p>{localize_backend(loc, &error)}</p>
                                                            <div class="fb-error-actions">
                                                                {setup.then(|| {
                                                                    let jump_id = jump_context.clone().unwrap_or_else(|| source.clone());
                                                                    view! {
                                                                        <button type="button" class="fb-retry primary"
                                                                            data-testid="ssh-setup-jump"
                                                                            on:click=move |_| {
                                                                                let context_id = jump_id.clone();
                                                                                let contexts = execution_contexts.get_untracked();
                                                                                let ctx = contexts.into_iter().find(|c| c.id == context_id);
                                                                                let label = ctx.as_ref().map(|c| {
                                                                                    if c.label.trim().is_empty() { c.id.clone() } else { c.label.clone() }
                                                                                }).unwrap_or_else(|| context_id.clone());
                                                                                let detail = ctx.as_ref()
                                                                                    .and_then(|c| ssh_connectivity_gap(c))
                                                                                    .unwrap_or_else(|| "not probed yet".into());
                                                                                // Land in Settings → Environments so the user can fix
                                                                                // identity/host config, and open the probe dialog.
                                                                                open_settings_fn(Some("environments".into()));
                                                                                ssh_connectivity_modal.set(Some(
                                                                                    SshConnectivityModal::from_gap(
                                                                                        context_id,
                                                                                        label,
                                                                                        detail,
                                                                                        false,
                                                                                    ),
                                                                                ));
                                                                            }>
                                                                            {t(loc, "ssh_check.jump_probe")}
                                                                        </button>
                                                                    }.into_view()
                                                                })}
                                                                <button type="button" class="fb-retry" on:click=move |_| {
                                                                    refresh_remote_dir(
                                                                        retry_context.clone(),
                                                                        remote_file_cwd,
                                                                        remote_file_entries,
                                                                        remote_file_loading,
                                                                        remote_file_error,
                                                                        file_source,
                                                                    );
                                                                }>{t(loc, "files.retry")}</button>
                                                            </div>
                                                        </div>
                                                    }.into_view()
                                                } else if remote_file_entries.get().is_empty() {
                                                    view! { <div class="rp-empty rp-files-empty"><p>{t(loc, "files.empty_remote")}</p></div> }.into_view()
                                                } else {
                                                    remote_file_entries.get().into_iter().map(|entry| {
                                                        let name = entry.name.clone();
                                                        let full = join_path(&remote_file_cwd.get(), &name);
                                                        if entry.is_dir {
                                                            let full_click = full.clone();
                                                            let context_id = source.clone();
                                                            view! {
                                                                <button class="fb-row dir remote-dir" data-remote-path=full.clone() on:click=move |_| {
                                                                    remote_file_cwd.set(full_click.clone());
                                                                    refresh_remote_dir(
                                                                        context_id.clone(),
                                                                        remote_file_cwd,
                                                                        remote_file_entries,
                                                                        remote_file_loading,
                                                                        remote_file_error,
                                                                        file_source,
                                                                    );
                                                                }>
                                                                    <span class="fb-icon">{compose_icon("folder")}</span>
                                                                    <span class="fb-name">{name}</span>
                                                                </button>
                                                            }.into_view()
                                                        } else {
                                                            let download_uri = context_menu::remote_file_download_uri(&source, &full);
                                                            let preview_path = format!("remote:{source}:{full}");
                                                            let preview_key = preview_path.clone();
                                                            // The row can't be a <button> (it nests the download
                                                            // one), so spell out the button semantics the local
                                                            // rows get for free.
                                                            view! {
                                                                <div class="fb-row remote-file" data-remote-path=full
                                                                    data-remote-context=source.clone()
                                                                    role="button" tabindex="0"
                                                                    on:click=move |_| {
                                                                        open_workspace_file(preview_path.clone(), modal_artifact);
                                                                    }
                                                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                                        if ev.key() == "Enter" || ev.key() == " " {
                                                                            ev.prevent_default();
                                                                            open_workspace_file(preview_key.clone(), modal_artifact);
                                                                        }
                                                                    }>
                                                                    <span class="fb-icon">{compose_icon("doc")}</span>
                                                                    <span class="fb-name">{name}</span>
                                                                    <span class="fb-size">{format_bytes(entry.size)}</span>
                                                                    {download_uri.map(|uri| {
                                                                        let download = uri.clone();
                                                                        view! {
                                                                            <button type="button" class="fb-row-action"
                                                                                title=move || t(locale.get(), "artifact.download")
                                                                                aria-label=move || t(locale.get(), "artifact.download")
                                                                                on:click=move |ev: web_sys::MouseEvent| {
                                                                                    ev.prevent_default();
                                                                                    ev.stop_propagation();
                                                                                    download_artifact(download.clone());
                                                                                }>{compose_icon("download")}</button>
                                                                        }
                                                                    })}
                                                                </div>
                                                            }.into_view()
                                                        }
                                                    }).collect_view()
                                                }}
                                            </div>
                                            <div class="hint fb-root">{t(loc, "files.remote_read_only")}</div>
                                        }.into_view()
                                    }}}
                                </div>
                            }.into_view()
                        }
                        RightTab::Provenance => {
                            let loc = locale.get();
                            let tools: Vec<_> = items.get().iter().filter_map(|it| match it {
                                ChatItem::Tool { name, ok, input, output, .. } => Some((name.clone(), *ok, input.clone(), output.clone())),
                                _ => None,
                            }).collect();
                            if tools.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">{t(loc, "right.no_tools.title")}</div>
                                        <p>{t(loc, "right.no_tools.body")}</p>
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                    <div class="prov-list">
                                        {tools.into_iter().map(|(name, ok, input, output)| view! {
                                            <details class="prov-item" open=ok != Some(true)>
                                                <summary class="prov-head">
                                                    <span class="prov-name">{name.clone()}</span>
                                                    {match ok {
                                                        Some(true) => view! { <span class="ok">"✓"</span> }.into_view(),
                                                        Some(false) => view! { <span class="fail">"✗"</span> }.into_view(),
                                                        None => view! { <span class="run">"…"</span> }.into_view(),
                                                    }}
                                                </summary>
                                                {(!input.is_empty()).then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.input")}</div>
                                                    <pre class="prov-body">{input.clone()}</pre>
                                                })}
                                                {(!output.is_empty()).then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.output")}</div>
                                                    <pre class="prov-body">{output.clone()}</pre>
                                                })}
                                            </details>
                                        }).collect_view()}
                                    </div>
                                }.into_view()
                            }
                        }
                        RightTab::Hosts => {
                            let loc = locale.get();
                            view! {
                                <div class="rp-contexts">
                                    <div class="context-list-pane">
                                    {move || {
                                        let contexts = execution_contexts.get().into_iter()
                                            .filter(|context| {
                                                context.kind == "local"
                                                    || session_execution_contexts.get().contains(&context.id)
                                            })
                                            .collect::<Vec<_>>();
                                        view! { <section class="control-section">
                                        <div class="control-section-head">
                                            <span>{t(loc, "contexts.execution")}</span>
                                            <span class="control-count">{contexts.len().to_string()}</span>
                                        </div>
                                        {if contexts.is_empty() {
                                            view! { <div class="control-empty">{t(loc, "contexts.empty")}</div> }.into_view()
                                        } else {
                                            contexts.into_iter().map(|ctx| {
                                                let status = ctx.last_probe_status.clone().unwrap_or_else(|| "unknown".into());
                                                let status_class = format!("context-status {status}");
                                                let summary = context_capability_summary(&ctx);
                                                let label = if ctx.label.trim().is_empty() { ctx.id.clone() } else { ctx.label.clone() };
                                                let active_context_id = ctx.id.clone();
                                                let pressed_context_id = ctx.id.clone();
                                                let select_context_id = ctx.id.clone();
                                                let runtime_context_id = ctx.id.clone();
                                                let runs_context_id = ctx.id.clone();
                                                let probe_context_id = ctx.id.clone();
                                                let terminal_context_id = ctx.id.clone();
                                                let runtime_config_context = ctx.clone();
                                                let config_context_id = ctx.id.clone();
                                                view! {
                                                    <div class="context-card"
                                                        class:active=move || selected_context_id.get().as_deref() == Some(active_context_id.as_str())>
                                                        <button type="button" class="context-card-select"
                                                            aria-pressed=move || (selected_context_id.get().as_deref() == Some(pressed_context_id.as_str())).to_string()
                                                            aria-label=t(loc, "contexts.machine_info")
                                                            on:click=move |_| {
                                                                selected_context_id.set(Some(select_context_id.clone()));
                                                                context_details_modal.set(Some((select_context_id.clone(), ContextModalKind::Machine)));
                                                            }>
                                                            <div class="context-card-head">
                                                                <span class="context-id">{ctx.id.clone()}</span>
                                                                <span class=status_class>{status}</span>
                                                            </div>
                                                            <div class="context-label">{label}</div>
                                                            <div class="context-meta">{ctx.kind.clone()}{" · "}{summary}</div>
                                                            {ctx.last_probe_error.clone().map(|err| view! {
                                                                <div class="context-error">{err}</div>
                                                            })}
                                                        </button>
                                                        <div class="context-card-actions">
                                                            <div class="context-card-tools">
                                                                <button type="button" class="context-terminal context-runtimes"
                                                                    title=t(loc, "contexts.view_runtimes")
                                                                    aria-label=t(loc, "contexts.view_runtimes")
                                                                    on:click=move |_| {
                                                                        selected_context_id.set(Some(runtime_context_id.clone()));
                                                                        context_details_modal.set(Some((runtime_context_id.clone(), ContextModalKind::Runtimes)));
                                                                    }>{compose_icon("terminal")}</button>
                                                                <button type="button" class="context-terminal context-runs"
                                                                    title=t(loc, "contexts.view_runs")
                                                                    aria-label=t(loc, "contexts.view_runs")
                                                                    on:click=move |_| {
                                                                        selected_context_id.set(Some(runs_context_id.clone()));
                                                                        context_details_modal.set(Some((runs_context_id.clone(), ContextModalKind::Runs)));
                                                                    }>{compose_icon("list")}</button>
                                                                <button type="button" class="context-terminal context-runtime-config"
                                                                    title=t(loc, "contexts.configure_interpreters")
                                                                    aria-label=t(loc, "contexts.configure_interpreters")
                                                                    on:click=move |_| {
                                                                        selected_context_id.set(Some(config_context_id.clone()));
                                                                        runtime_interpreter_form.set(Some(
                                                                            RuntimeInterpreterForm::from_context(&runtime_config_context)
                                                                        ));
                                                                    }>{compose_icon("edit")}</button>
                                                                <button type="button" class="context-terminal context-probe"
                                                                    title=t(loc, "contexts.probe")
                                                                    aria-label=t(loc, "contexts.probe")
                                                                    on:click=move |_| {
                                                                        let context_id = probe_context_id.clone();
                                                                        selected_context_id.set(Some(context_id.clone()));
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
                                                                            match invoke_checked("probe_execution_context", arg).await {
                                                                                Ok(value) => {
                                                                                    show_probe_stopped_toast(&value, locale);
                                                                                    refresh_execution_contexts(execution_contexts);
                                                                                    refresh_runtimes(runtime_infos);
                                                                                }
                                                                                Err(error) => {
                                                                                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                                                                                    show_toast(&message);
                                                                                }
                                                                            }
                                                                        });
                                                                    }>"↻"</button>
                                                                <button type="button" class="context-terminal"
                                                                    title=t(loc, "contexts.open_terminal")
                                                                    aria-label=t(loc, "contexts.open_terminal")
                                                                    on:click=move |_| {
                                                                        selected_context_id.set(Some(terminal_context_id.clone()));
                                                                        open_terminal_for_context.call(terminal_context_id.clone());
                                                                    }>{compose_icon("terminal")}</button>
                                                            </div>
                                                        </div>
                                                    </div>
                                                }.into_view()
                                            }).collect_view()
                                        }}
                                        <div class="context-actions">
                                            <button type="button" class="rp-hosts-add"
                                                on:click=move |_| {
                                                    settings_section.set("environments".into());
                                                    show_settings.set(true);
                                                }>{t(loc, "compute.manage")}</button>
                                        </div>
                                    </section> }.into_view()
                                    }}
                                    </div>
                                </div>
                            }.into_view()
                        }
                        RightTab::SideChat => {
                            view! {
                                <div class="sidechat-in-pane">
                                    <div class="sidechat-log" id=SIDE_CHAT_SCROLLER_ID>
                                        {move || {
                                            let rows = side_chat_items.get();
                                            if rows.is_empty() && !side_chat_busy.get() {
                                                view! { <div class="sidechat-empty">{move || t(locale.get(), "sidechat.empty")}</div> }.into_view()
                                            } else {
                                                rows.into_iter().map(|item| match item {
                                                    ChatItem::User(text) => view! {
                                                        <div class="sidechat-row user"><div class="sidechat-bubble">{text}</div></div>
                                                    }.into_view(),
                                                    ChatItem::Assistant { text, model, .. } => {
                                                        let error = text.starts_with("Error: ");
                                                        view! {
                                                            <div class="sidechat-row assistant">
                                                                {model.filter(|_| !error).map(|m| view! { <div class="sidechat-model-label">{m}</div> })}
                                                                <div class="sidechat-answer" class:error=error inner_html=md_to_html(&text)></div>
                                                            </div>
                                                        }.into_view()
                                                    }
                                                    _ => view! {}.into_view(),
                                                }).collect_view()
                                            }
                                        }}
                                        {move || side_chat_busy.get().then(|| view! {
                                            <div class="sidechat-thinking">{move || t(locale.get(), "sidechat.thinking")}</div>
                                        })}
                                    </div>
                                    <div class="sidechat-composer">
                                        <textarea
                                            prop:value=move || side_chat_input.get()
                                            prop:placeholder=move || t(locale.get(), "sidechat.placeholder")
                                            on:input=move |ev| side_chat_input.set(event_target_value(&ev))
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ime_composing(&ev) { return; }
                                                if ev.key() == "Enter" && !ev.shift_key() {
                                                    ev.prevent_default();
                                                    send_side_chat(side_chat_input.get());
                                                }
                                            }
                                        ></textarea>
                                        <div class="sidechat-actions">
                                            {move || (!models.get().is_empty() || !acp_agents.get().is_empty()).then(|| view! {
                                                <div class="sidechat-model">
                                                    <button type="button" class="sidechat-model-btn"
                                                        class:active=move || side_chat_model_menu_open.get()
                                                        on:click=move |_| side_chat_model_menu_open.update(|o| *o = !*o)>
                                                        {move || {
                                                            if let Some(id) = side_chat_acp_agent.get() {
                                                                acp_agents.get().into_iter().find(|agent| agent.id == id).map(|agent| agent.label).unwrap_or_else(|| "ACP Agent".into())
                                                            } else {
                                                                let l = models.get();
                                                                l.iter().find(|m| m.active).or_else(|| l.first()).map(|m| m.label.clone()).unwrap_or_default()
                                                            }
                                                        }}
                                                        <span>"▾"</span>
                                                    </button>
                                                    {move || side_chat_model_menu_open.get().then(|| view! {
                                                        <div class="sidechat-model-backdrop" on:click=move |_| side_chat_model_menu_open.set(false)></div>
                                                        <div class="sidechat-model-menu">
                                                            {move || models.get().into_iter().map(|m| {
                                                                let pick_id = m.id.clone();
                                                                let is_active = m.active && side_chat_acp_agent.get().is_none();
                                                                view! {
                                                                    <button type="button" class="sidechat-model-row" class:active=is_active
                                                                        on:click=move |_| {
                                                                            side_chat_model_menu_open.set(false);
                                                                            side_chat_acp_agent.set(None);
                                                                            let id = pick_id.clone();
                                                                            spawn_local(async move {
                                                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                                if let Ok(v) = invoke_checked("set_active_model", arg).await {
                                                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                        models.set(list);
                                                                                    }
                                                                                }
                                                                            });
                                                                        }>
                                                                        <span>{m.label.clone()}</span>
                                                                        {is_active.then(|| view! { <span>"✓"</span> })}
                                                                    </button>
                                                                }
                                                            }).collect_view()}
                                                            {move || (!acp_agents.get().is_empty()).then(|| view! {
                                                                <div class="sidechat-model-group">"ACP Agents"</div>
                                                                {acp_agents.get().into_iter().map(|agent| {
                                                                    let id = agent.id.clone();
                                                                    let selected = side_chat_acp_agent.get().as_deref() == Some(agent.id.as_str());
                                                                    view! {
                                                                        <button type="button" class="sidechat-model-row" class:active=selected
                                                                            on:click=move |_| {
                                                                                side_chat_model_menu_open.set(false);
                                                                                side_chat_acp_agent.set(Some(id.clone()));
                                                                            }>
                                                                            <span>{agent.label.clone()}</span>
                                                                            {selected.then(|| view! { <span>"✓"</span> })}
                                                                        </button>
                                                                    }
                                                                }).collect_view()}
                                                            })}
                                                        </div>
                                                    })}
                                                </div>
                                            })}
                                            <button type="button" class="sidechat-send"
                                                disabled=move || side_chat_busy.get() || side_chat_input.get().trim().is_empty()
                                                on:click=move |_| send_side_chat(side_chat_input.get())>
                                                {move || t(locale.get(), "composer.send")}
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            }.into_view()
                        }
                    }}
                </div>
            </section>
        }.into_view())}
        </div>

        <Show when=move || !terminal_sessions.get().is_empty()>
            <section class="terminal-dock" data-testid="terminal-dock"
                class:terminal-dock-hidden=move || !terminal_panel_open.get()
                style=move || format!("height:{}px", terminal_h.get())>
                <div class="terminal-dock-resize" aria-hidden="true"
                    on:mousedown=on_terminal_resize_start></div>
                <header class="terminal-dock-head">
                    <div class="terminal-dock-tabs" role="tablist">
                        <For
                            each=move || terminal_sessions.get()
                            key=|session| session.id.clone()
                            let:session
                        >
                            {
                                let tab_session_id = session.id.clone();
                                let tab_active_id = session.id.clone();
                                let tab_id = terminal_tab_id(&session.id);
                                let panel_id = terminal_element_id(&session.id);
                                view! {
                                    <button id=tab_id type="button" role="tab" class="terminal-dock-tab"
                                        class:active=move || active_terminal_id.get().as_deref() == Some(tab_active_id.as_str())
                                        aria-selected=move || active_terminal_id.get().as_deref() == Some(session.id.as_str())
                                        aria-controls=panel_id
                                        title=session.title.clone()
                                        on:click=move |_| {
                                            active_terminal_id.set(Some(tab_session_id.clone()));
                                            terminal_add_menu_open.set(false);
                                        }>
                                        {compose_icon("terminal")}
                                        <span class="terminal-dock-title">{session.title}</span>
                                    </button>
                                }
                            }
                        </For>
                        <div class="terminal-dock-add-wrap">
                            <button type="button" class="terminal-dock-action icon terminal-dock-add"
                                class:active=move || terminal_add_menu_open.get()
                                title=move || t(locale.get(), "terminal.new")
                                aria-label=move || t(locale.get(), "terminal.new")
                                on:click=move |_| terminal_add_menu_open.update(|open| *open = !*open)>
                                {compose_icon("plus")}
                            </button>
                            {move || terminal_add_menu_open.get().then(|| view! {
                                <div class="terminal-dock-add-menu">
                                    <div class="terminal-dock-add-label">{move || t(locale.get(), "terminal.choose_context")}</div>
                                    {move || execution_contexts.get().into_iter().map(|context| {
                                        let context_id = context.id.clone();
                                        let label = if context.label.trim().is_empty() {
                                            context.id.clone()
                                        } else {
                                            context.label.clone()
                                        };
                                        view! {
                                            <button type="button" class="terminal-dock-add-item"
                                                on:click=move |_| open_terminal_for_context.call(context_id.clone())>
                                                {compose_icon("terminal")}
                                                <span>{label}</span>
                                                <small>{context.id}</small>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            })}
                        </div>
                    </div>
                    {move || terminal_sessions.get().into_iter()
                        .find(|session| Some(&session.id) == active_terminal_id.get().as_ref())
                        .map(|session| view! {
                            <span class="terminal-dock-meta">{session.context_id}{" · "}{session.display_cwd}</span>
                        })}
                    <span class="terminal-dock-spacer"></span>
                    <button type="button" class="terminal-dock-action danger"
                        disabled=move || terminal_sessions.get().into_iter()
                            .find(|session| Some(&session.id) == active_terminal_id.get().as_ref())
                            .is_none_or(|session| !session.running)
                        on:click=move |_| {
                            let Some(session_id) = active_terminal_id.get_untracked() else { return; };
                            spawn_local(async move {
                                let arg = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
                                if invoke_checked("terminate_terminal", arg).await.is_ok() {
                                    terminal_sessions.update(|sessions| {
                                        if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
                                            session.running = false;
                                        }
                                    });
                                }
                            });
                        }>{move || t(locale.get(), "terminal.terminate")}</button>
                    <button type="button" class="terminal-dock-action icon"
                        title=move || t(locale.get(), "terminal.close")
                        aria-label=move || t(locale.get(), "terminal.close")
                        on:click=move |_| {
                            terminal_add_menu_open.set(false);
                            terminal_panel_open.set(false);
                        }>{compose_icon("close")}</button>
                </header>
                <div class="terminal-dock-frames">
                    <For
                        each=move || terminal_sessions.get()
                        key=|session| session.id.clone()
                        let:session
                    >
                        <TerminalHost
                            session_id=session.id
                            active_terminal_id=active_terminal_id
                        />
                    </For>
                </div>
            </section>
        </Show>
        </div>

        {move || dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_resize_move
                on:mouseup=move |_| dragging.set(false)></div>
        })}

        {move || sidebar_dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_sidebar_resize_move
                on:mouseup=on_sidebar_resize_end></div>
        })}

        {move || composer_dragging.get().then(|| view! {
            <div class="drag-overlay drag-overlay-row"
                on:mousemove=on_composer_resize_move
                on:mouseup=on_composer_resize_end></div>
        })}

        {move || terminal_dragging.get().then(|| view! {
            <div class="drag-overlay drag-overlay-row"
                on:mousemove=on_terminal_resize_move
                on:mouseup=move |_| terminal_dragging.set(false)></div>
        })}

        {move || session_transfer.get().map(|transfer| {
            let active_project_id = project_info
                .get()
                .map(|project| project.id)
                .unwrap_or_default();
            let targets = proj_list
                .get()
                .into_iter()
                .filter(|project| project.id != active_project_id)
                .collect::<Vec<_>>();
            let has_target = !targets.is_empty() && !transfer.target_project_id.is_empty();
            let title_key = if transfer.mode == SessionTransferMode::Copy {
                "session.copy_title"
            } else {
                "session.move_title"
            };
            let action_key = if transfer.mode == SessionTransferMode::Copy {
                "session.copy_action"
            } else {
                "session.move_action"
            };
            view! {
            <div class="overlay">
                <div class="modal session-transfer-modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <div class="hint">{tf(locale.get(), "session.transfer_hint", &[("title", &transfer.title)])}</div>
                    <label>
                        {move || t(locale.get(), "session.target_project")}
                        <select
                            prop:value=transfer.target_project_id
                            disabled=move || session_transfer_busy.get()
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                session_transfer.update(|transfer| {
                                    if let Some(transfer) = transfer {
                                        transfer.target_project_id = value;
                                    }
                                });
                            }>
                            {targets.into_iter().map(|project| view! {
                                <option value=project.id>{project.name}</option>
                            }).collect_view()}
                        </select>
                    </label>
                    {(!has_target).then(|| view! {
                        <div class="hint session-transfer-error">{move || t(locale.get(), "session.no_target_project")}</div>
                    })}
                    {move || session_transfer_error.get().map(|error| view! {
                        <div class="hint session-transfer-error">{error}</div>
                    })}
                    <div class="row">
                        <button type="button"
                            disabled=move || session_transfer_busy.get()
                            on:click=move |_| {
                                session_transfer.set(None);
                                session_transfer_error.set(None);
                            }>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary"
                            disabled=move || !has_target || session_transfer_busy.get()
                            on:click=save_session_transfer>{move || t(locale.get(), action_key)}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || rename_session_target.get().map(|(id, _)| {
            let id_key = id.clone();
            let id_btn = id.clone();
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), "session.rename_title")}</h2>
                    <label>
                        <input
                            id="rename-session-input"
                            type="text"
                            autofocus=true
                            prop:value=move || rename_session_input.get()
                            on:input=move |ev| rename_session_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if (ev.ctrl_key() || ev.meta_key())
                                    && ev.key().eq_ignore_ascii_case("a")
                                {
                                    ev.prevent_default();
                                    if let Some(target) = ev.target() {
                                        if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                            input.select();
                                        }
                                    }
                                    return;
                                }
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    let title = rename_session_input.get().trim().to_string();
                                    if title.is_empty() { return; }
                                    let id = id_key.clone();
                                    rename_session_target.set(None);
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                        if invoke_checked("rename_session", arg).await.is_ok() {
                                            refresh_session_history();
                                        }
                                    });
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| rename_session_target.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            let title = rename_session_input.get().trim().to_string();
                            if title.is_empty() { return; }
                            let id = id_btn.clone();
                            rename_session_target.set(None);
                            spawn_local(async move {
                                let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                if invoke_checked("rename_session", arg).await.is_ok() {
                                    refresh_session_history();
                                }
                            });
                        }>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || folder_modal.get().map(|mode| {
            let mode_save = mode.clone();
            let mode_enter = mode.clone();
            let title_key = match &mode {
                FolderModal::Create => "folder.new_title",
                FolderModal::Rename(_) => "folder.rename_prompt",
            };
            let label_key = match &mode {
                FolderModal::Create => "folder.new_prompt",
                FolderModal::Rename(_) => "folder.new_prompt",
            };
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <label>
                        {move || t(locale.get(), label_key)}
                        <input
                            id="folder-modal-input"
                            type="text"
                            autofocus=true
                            prop:value=move || folder_modal_input.get()
                            on:input=move |ev| folder_modal_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    save_folder_modal(mode_enter.clone());
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| folder_modal.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| save_folder_modal(mode_save.clone())>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || file_entry_modal.get().map(|mode| {
            let mode_save = mode.clone();
            let mode_enter = mode.clone();
            let (title_key, action_key, location) = match &mode {
                FileEntryModal::CreateFile => (
                    "files.new_file",
                    "files.create",
                    file_cwd.get_untracked(),
                ),
                FileEntryModal::CreateDirectory => (
                    "files.new_directory",
                    "files.create",
                    file_cwd.get_untracked(),
                ),
                FileEntryModal::Rename { path, is_dir } => (
                    if *is_dir { "files.rename_directory" } else { "files.rename_file" },
                    "files.rename",
                    parent_path(path),
                ),
            };
            view! {
                <div class="overlay">
                    <div class="modal file-entry-modal">
                        <h2>{move || t(locale.get(), title_key)}</h2>
                        <div class="hint file-entry-location">
                            {move || tf(locale.get(), "files.location", &[("path", &location)])}
                        </div>
                        <label>
                            {move || t(locale.get(), "files.name")}
                            <input
                                id="file-entry-modal-input"
                                type="text"
                                autofocus=true
                                disabled=move || file_entry_busy.get()
                                prop:value=move || file_entry_input.get()
                                on:input=move |ev| {
                                    file_entry_input.set(dom_value(&ev));
                                    file_entry_error.set(None);
                                }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        save_file_entry_modal.call(mode_enter.clone());
                                    }
                                }
                            />
                        </label>
                        {move || file_entry_error.get().map(|error| view! {
                            <div class="settings-error" role="alert">{error}</div>
                        })}
                        <div class="row">
                            <button disabled=move || file_entry_busy.get() on:click=move |_| {
                                file_entry_modal.set(None);
                                file_entry_error.set(None);
                            }>{move || t(locale.get(), "settings.cancel")}</button>
                            <button class="primary" disabled=move || file_entry_busy.get()
                                on:click=move |_| save_file_entry_modal.call(mode_save.clone())>
                                {move || if file_entry_busy.get() {
                                    t(locale.get(), "files.working")
                                } else {
                                    t(locale.get(), action_key)
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            }.into_view()
        })}

        {move || ui_confirm.get().map(|action| {
            let action_ok = action.clone();
            let message = match &action {
                UiConfirm::DeleteFolder(_) => t(locale.get(), "folder.delete_confirm").to_string(),
                UiConfirm::DeleteSession(_) => t(locale.get(), "session.delete_confirm").to_string(),
                UiConfirm::DeleteFileEntry { path, is_dir } => tf(
                    locale.get(),
                    if *is_dir { "files.delete_directory_confirm" } else { "files.delete_file_confirm" },
                    &[("path", path)],
                ),
            };
            let action_key = match &action {
                UiConfirm::DeleteFolder(_) => "ctx.delete_folder",
                UiConfirm::DeleteSession(_) => "ctx.delete_session",
                UiConfirm::DeleteFileEntry { is_dir: true, .. } => "files.delete_directory",
                UiConfirm::DeleteFileEntry { is_dir: false, .. } => "files.delete_file",
            };
            view! {
            <div class="overlay">
                <div class="modal confirm-modal">
                    <h2>{move || t(locale.get(), "confirm.title")}</h2>
                    <div class="hint">{message}</div>
                    <div class="row">
                        <button on:click=move |_| ui_confirm.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            ui_confirm.set(None);
                            match action_ok.clone() {
                                UiConfirm::DeleteFolder(id) => {
                                    let folders = folders;
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                        if invoke_checked("delete_folder", arg).await.is_ok() {
                                            refresh_folders(folders);
                                            refresh_session_history();
                                        }
                                    });
                                }
                                UiConfirm::DeleteSession(id) => {
                                    let active_session = active_session;
                                    let items = items;
                                    let transcripts = transcripts;
                                    let running = running;
                                    let pending_turns = pending_turns;
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                        if invoke_checked("delete_session", arg).await.is_ok() {
                                            transcripts.update(|m| { m.remove(&id); });
                                            running.update(|r| { r.remove(&id); });
                                            pending_turns.update(|m| { m.remove(&id); });
                                            if active_session.get().as_deref() == Some(id.as_str()) {
                                                active_session.set(None);
                                                items.set(vec![]);
                                            }
                                            refresh_session_history();
                                        }
                                    });
                                }
                                UiConfirm::DeleteFileEntry { path, is_dir } => {
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "path": path.clone() })).unwrap();
                                        match invoke_checked("delete_entry", arg).await {
                                            Ok(_) => {
                                                let prefix = format!("{path}/");
                                                center_files.update(|files| files.retain(|file| {
                                                    file.path != path
                                                        && !(is_dir && file.path.starts_with(&prefix))
                                                }));
                                                center_file.update(|active| {
                                                    let should_close = active.as_ref().is_some_and(|file| {
                                                        file == &path
                                                            || (is_dir && file.starts_with(&prefix))
                                                    });
                                                    if should_close {
                                                        *active = None;
                                                    }
                                                });
                                                refresh_dir(file_cwd, file_entries);
                                                if !file_query.get_untracked().trim().is_empty() {
                                                    refresh_file_search(file_query, file_search_hits);
                                                }
                                            }
                                            Err(error) => show_toast(&localize_backend(
                                                locale.get_untracked(),
                                                &js_error_text(error),
                                            )),
                                        }
                                    });
                                }
                            }
                        }>{move || t(locale.get(), action_key)}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || model_switch_confirm.get().map(|(id, label)| {
            let switch_yes = switch_http_model.clone();
            let switch_without_future_warning = switch_http_model.clone();
            let yes_id = id.clone();
            let dont_ask_id = id.clone();
            view! {
                <div class="overlay" data-testid="model-switch-confirm-overlay">
                    <div class="modal confirm-modal" data-testid="model-switch-confirm">
                        <h2>{move || t(locale.get(), "models.switch_confirm_title")}</h2>
                        <div class="hint">{move || tf(
                            locale.get(),
                            "models.switch_confirm_hint",
                            &[("model", &label)],
                        )}</div>
                        <div class="row">
                            <button on:click=move |_| model_switch_confirm.set(None)>
                                {move || t(locale.get(), "models.switch_no")}
                            </button>
                            <button on:click=move |_| {
                                model_switch_confirm.set(None);
                                switch_without_future_warning.call((dont_ask_id.clone(), true));
                            }>{move || t(locale.get(), "models.switch_dont_ask")}</button>
                            <button class="primary" on:click=move |_| {
                                model_switch_confirm.set(None);
                                switch_yes.call((yes_id.clone(), false));
                            }>{move || t(locale.get(), "models.switch_yes")}</button>
                        </div>
                    </div>
                </div>
            }.into_view()
        })}

        {move || show_proj_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal proj-settings-modal">
                    <div class="ps-head">
                        <h2>{move || t(locale.get(), "proj_settings.title")}</h2>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "settings.cancel")
                            on:click=move |_| show_proj_settings.set(false)>{compose_icon("close")}</button>
                    </div>
                    <label>
                        {move || t(locale.get(), "proj_settings.name")}
                        <input prop:value=move || proj_settings.get().name
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.name = v); } />
                    </label>
                    <label>
                        {move || t(locale.get(), "proj_settings.description")}
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                        <textarea class="ps-textarea" rows="2"
                            prop:value=move || proj_settings.get().description
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.description = v); }></textarea>
                    </label>
                    <label>
                        {move || t(locale.get(), "proj_settings.agent_context")}
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                        <textarea class="ps-textarea ps-ctx" rows="8"
                            prop:value=move || proj_settings.get().agent_context
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.agent_context = v); }></textarea>
                    </label>
                    <div class="row">
                        <button type="button" disabled=move || proj_settings_busy.get()
                            on:click=move |_| show_proj_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary"
                            disabled=move || proj_settings_busy.get() || proj_settings.get().name.trim().is_empty()
                            on:click=save_proj_settings>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        })}

        {move || modal_artifact.get().map(|(path, name, kind)| {
            let session = active_session.get();
            let arts_for_nav = artifacts.get();
            let (prev_artifact, next_artifact) = modal_image_nav_targets(&arts_for_nav, &path, &kind);
            let can_prev = prev_artifact.is_some();
            let can_next = next_artifact.is_some();
            view! {
                <ArtifactModal path=path name=name kind=kind session=session
                    can_prev=can_prev
                    can_next=can_next
                    on_prev=Callback::new(move |_| {
                        if let Some((path, name, kind)) = prev_artifact.clone() {
                            modal_artifact.set(Some((path, name, kind)));
                        }
                    })
                    on_next=Callback::new(move |_| {
                        if let Some((path, name, kind)) = next_artifact.clone() {
                            modal_artifact.set(Some((path, name, kind)));
                        }
                    })
                    on_close=Callback::new(move |_| modal_artifact.set(None))
                    on_open_center=Callback::new(move |(path, name, kind): ModalArtifact| {
                        let tab = CenterFileTab::new(path.clone(), name, kind);
                        center_files.update(|files| {
                            if !files.iter().any(|file| file.path == path) {
                                files.push(tab.clone());
                            }
                        });
                        center_file.set(Some(path));
                        show_projects.set(false);
                        modal_artifact.set(None);
                    })
                    on_open_path=Callback::new(move |(p, _k): (String, String)| {
                        reveal_in_files(&p, file_source, file_cwd, file_query, file_entries, show_right, open_right_tabs, right_tab);
                        modal_artifact.set(None);
                    })
                    library_items=library_items.read_only()
                    on_library_changed=refresh_library_items />
            }
        })}
        <SettingsView
            state=SettingsViewState {
                locale, theme_mode, light_palette, dark_palette, ui_font_size, code_font_size, selection_popup_enabled, send_with_modifier, update_check_enabled, show_settings, settings_section, open_conn_key, channels_open, connectors, model_form,
                conn_form, memory_selected, specialist_form, settings, bootstrap, settings_message,
                settings_busy, model_form_open, model_form_key, models, model_form_msg, show_acp_agents,
                acp_agents, active_acp_agent_id, acp_form, acp_form_msg, acp_infos, specialists,
                specialist_form_open, memory_view, memory_editor, memory_msg, skills_list,
                skill_filter_tag, skills_search, skills_msg, plugins_list, plugins_msg, plugin_install_open, cred_status, cred_inputs,
                custom_credentials, cred_msg, approval_grants, conns_view, conn_form_open,
                conn_form_kind, conn_test_msg, custom_conn_tools, custom_conn_tools_loading,
                custom_conn_tool_errors, pet_status, ssh_hosts, execution_contexts,
                runtime_interpreter_form, probing_context_id, delete_confirm,
            }
            open_project=switch_project
            go_settings_section=Callback::new(move |section: String| go_settings_section(&section))
            close_settings_subpage=Callback::new(move |_: ()| close_settings_subpage())
            check_updates=Callback::new(check_updates)
            save_settings=Callback::new(save_settings)
            save_model_form=Callback::new(save_model_form)
            save_specialist_form=Callback::new(save_specialist_form)
            test_reviewer_form=Callback::new(test_reviewer_form)
            validate_model_form=Callback::new(validate_model_form)
            start_specialist_chat=start_specialist_chat
            refresh_conns=Callback::new(move |_: ()| refresh_conns())
            refresh_skills=Callback::new(move |_: ()| refresh_skills())
            refresh_approval_grants=Callback::new(move |_: ()| refresh_approval_grants())
            load_memory_file=Callback::new(load_memory_file)
            load_custom_conn_tools=Callback::new(load_custom_conn_tools)
            save_skill_tags=save_skill_tags
            set_visible_skills_enabled=set_visible_skills_enabled
            install_skill_from=Callback::new(install_skill_from)
            install_plugin_from=install_plugin_from
            install_plugin_url=install_plugin_url
            set_plugin_enabled=set_plugin_enabled
            use_plugin=use_plugin
            remove_plugin=remove_plugin
            remove_specialist=Callback::new(remove_specialist_fn)
            open_add_host=open_add_host_form
            edit_ssh_host=edit_ssh_host
            import_ssh_hosts=Callback::new(move |_: ()| {
                spawn_local(async move {
                    let value = invoke("import_ssh_config_hosts", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(value) {
                        ssh_hosts.set(list);
                        refresh_execution_contexts(execution_contexts);
                    }
                });
            })
            import_wsl_contexts=Callback::new(move |_: ()| {
                spawn_local(async move {
                    match invoke_checked("import_wsl_contexts", JsValue::UNDEFINED).await {
                        Ok(value) => match serde_wasm_bindgen::from_value::<Vec<ExecutionContext>>(value) {
                            Ok(contexts) => execution_contexts.set(contexts),
                            Err(error) => show_toast(&error.to_string()),
                        },
                        Err(error) => {
                            let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                            show_toast(&message);
                        }
                    }
                });
            })
            remove_ssh_host=Callback::new(move |alias: String| {
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({ "alias": alias })).unwrap();
                    let value = invoke("remove_ssh_host", args).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(value) {
                        ssh_hosts.set(list);
                        refresh_execution_contexts(execution_contexts);
                    }
                });
            })
            probe_compute_resource=Callback::new(move |context_id: String| {
                if probing_context_id.get_untracked().is_some() {
                    return;
                }
                probing_context_id.set(Some(context_id.clone()));
                let label = execution_contexts
                    .get_untracked()
                    .into_iter()
                    .find(|context| context.id == context_id)
                    .map(|context| if context.label.trim().is_empty() {
                        context.id
                    } else {
                        context.label
                    })
                    .unwrap_or_else(|| context_id.clone());
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
                    match invoke_checked("probe_execution_context", args).await {
                        Ok(value) => {
                            match serde_wasm_bindgen::from_value::<ExecutionContext>(value) {
                                Ok(updated) => {
                                    execution_contexts.update(|contexts| {
                                        if let Some(existing) = contexts.iter_mut().find(|context| context.id == updated.id) {
                                            *existing = updated.clone();
                                        } else {
                                            contexts.push(updated.clone());
                                        }
                                    });
                                    if updated.last_probe_status.as_deref() == Some("ok") {
                                        let partial = serde_json::from_str::<serde_json::Value>(
                                            &updated.capabilities_json,
                                        )
                                        .ok()
                                        .is_some_and(|capabilities| {
                                            ["os", "arch", "hostname"].iter().any(|key| {
                                                capabilities
                                                    .get(key)
                                                    .and_then(|value| value.as_str())
                                                    .is_none_or(str::is_empty)
                                            })
                                        });
                                        let key = if partial {
                                            "contexts.probe_success_partial"
                                        } else {
                                            "contexts.probe_success"
                                        };
                                        show_toast(&t(locale.get_untracked(), key));
                                    } else {
                                        let detail = updated.last_probe_error.clone()
                                            .filter(|error| !error.trim().is_empty())
                                            .unwrap_or_else(|| "probe failed".into());
                                        if updated.kind == "ssh" {
                                            ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                                updated.id,
                                                if updated.label.trim().is_empty() { label.clone() } else { updated.label },
                                                detail,
                                                false,
                                            )));
                                        } else {
                                            show_warning_toast(&localize_backend(locale.get_untracked(), &detail));
                                        }
                                    }
                                }
                                Err(error) => show_toast(&error.to_string()),
                            }
                        }
                        Err(error) => {
                            let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                            if context_id.starts_with("ssh:") {
                                ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                    context_id.clone(),
                                    label,
                                    message,
                                    false,
                                )));
                            } else {
                                show_toast(&message);
                            }
                        }
                    }
                    probing_context_id.set(None);
                });
            })
        />

        {(!is_windows()).then(|| view! {
            <PetOverlay status=pet_status active_session=active_session running=running
                approval_pending=approval_pending activity=pet_activity show_projects=show_projects
                show_settings=show_settings center_file_open=center_file_open />
        })}



        <AddHostOverlay
            locale=locale show_add_host=show_add_host host_alias=host_alias host_hostname=host_hostname
            host_notes=host_notes host_user=host_user host_port=host_port host_identity=host_identity
            host_auth_method=host_auth_method host_password=host_password host_has_password=host_has_password
            editing_host_alias=editing_host_alias
            ssh_hosts=ssh_hosts execution_contexts=execution_contexts
        />
        <ContextDetailsOverlay
            modal=context_details_modal runtime_environment=runtime_environment
            runtime_environment_pinned=runtime_environment_pinned
            runtime_environment_position=runtime_environment_position
            contexts=execution_contexts runtimes=runtime_infos
            runs=run_records active_project=project_info projects=proj_list
            runtime_interpreter_form=runtime_interpreter_form object_states=runtime_object_states
            locale=locale selection_popup=selection_popup
        />
        {move || runtime_environment_pinned.get().then(|| view! {
            <RuntimeEnvironmentPanel selected=runtime_environment pinned=runtime_environment_pinned
                position=runtime_environment_position context_modal=context_details_modal
                locale=locale states=runtime_object_states runtimes=runtime_infos
                selection_popup=selection_popup />
        })}
        <RuntimeInterpreterOverlay
            locale=locale form=runtime_interpreter_form execution_contexts=execution_contexts
            runtimes=runtime_infos
        />
        <CapabilitiesOverlay
            locale=locale show_capabilities=show_capabilities bootstrap=bootstrap caps=caps busy=busy
            start_env_setup=Callback::new(start_env_setup)
        />
        <OnboardingOverlay
            locale=locale show_onboarding=show_onboarding onboard_step=onboard_step
            onboard_provider=onboard_provider onboard_key=onboard_key
            save_onboard_key=save_onboard_key
            dismiss_onboard=Callback::new(dismiss_onboard)
        />
        <ContextMenuPortal menu=ctx_menu.read_only() set_menu=ctx_menu.write_only() on_pick=on_ctx_pick />
        </div>
    }
}

/// True for items whose `render_item` produces an empty view, so the thread
/// loop can drop their wrapper `<div>` and avoid a dangling `.thread` gap (#19).
fn renders_nothing(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.trim().is_empty())
        || matches!(item, ChatItem::Tool { name, .. } if name == "attempt_completion")
}

fn class_for(item: &ChatItem) -> &'static str {
    match item {
        ChatItem::User(_) => "msg user",
        ChatItem::QueuedUser { .. } => "msg user queued",
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => "tool-wrap",
        ChatItem::Assistant { .. } => "msg assistant",
        ChatItem::Reasoning(_) => "msg reasoning",
        ChatItem::Tool { name, .. } if is_run_monitor_tool(name) => "tool-wrap run-monitor-wrap",
        ChatItem::Tool { .. } => "tool-wrap",
        ChatItem::ApprovalPending { .. } => "tool-wrap approval-wrap-row",
        ChatItem::AcpPermission { .. } => "tool-wrap approval-wrap-row",
        ChatItem::AcpTool { .. } => "tool-wrap",
        ChatItem::Usage { .. } => "usage-row",
        ChatItem::ReviewTransition { .. } => "review-transition-row",
        ChatItem::Review(_) => "tool-wrap",
        ChatItem::Plan(_) => "tool-wrap plan-wrap",
    }
}

/// "482" below 1k, "12.3k" above — same scale the status bar uses.
fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{:.1}k", n as f64 / 1000.0)
    }
}

#[cfg(test)]
mod token_format_tests {
    use super::fmt_tokens;

    #[test]
    fn small_counts_are_not_rounded_to_zero() {
        assert_eq!(fmt_tokens(81), "81");
        assert_eq!(fmt_tokens(136_286), "136.3k");
    }
}

/// One thread render unit: either a single message, or a coalesced steps panel.
#[derive(Clone)]
enum ThreadRow {
    Item {
        i: usize,
        item: ChatItem,
        commentary: bool,
        compact_assistant: bool,
    },
    Steps {
        items: Vec<ChatItem>,
        live: bool,
    },
    Activity {
        items: Vec<ChatItem>,
    },
}

/// Compact, foldable summary of consecutive tool calls. Collapsed by default;
/// auto-opens while it is the live tail so progress stays visible.
///
/// Built as a manual accordion (signal + `class:open`) rather than
/// `<details>/<summary>`: the UA disclosure marker survives `list-style:none`
/// + `::-webkit-details-marker` here (WebKit and Blink alike), and there is no
/// portable way to drop it — so we don't render one.
fn disclosure_open(states: RwSignal<HashMap<String, bool>>, id: &str, automatic: bool) -> bool {
    states.with(|values| values.get(id).copied().unwrap_or(automatic))
}

fn toggle_disclosure(states: RwSignal<HashMap<String, bool>>, id: &str, automatic: bool) {
    states.update(|values| {
        let current = values.get(id).copied().unwrap_or(automatic);
        values.insert(id.to_string(), !current);
    });
}

fn render_steps_group(
    items: Vec<ChatItem>,
    live: bool,
    completed_turn: bool,
    group_id: String,
    disclosure_state: RwSignal<HashMap<String, bool>>,
) -> impl IntoView {
    let locale = use_locale();
    let n_tools = items
        .iter()
        .filter(|c| matches!(c, ChatItem::Tool { .. } | ChatItem::AcpTool { .. }))
        .count();
    let now = now_ms();
    let total_ms: u64 = items
        .iter()
        .map(|c| match c {
            ChatItem::Tool {
                duration_ms: Some(d),
                ..
            } => *d,
            ChatItem::Tool {
                duration_ms: None,
                started_at_ms: Some(s),
                ok: None,
                ..
            } if live => now.saturating_sub(*s),
            _ => 0,
        })
        .sum();
    let title = move || {
        if completed_turn {
            t(locale.get(), "chat.activity_done").to_string()
        } else if live {
            t(locale.get(), "chat.steps_running").to_string()
        } else if n_tools == 1 {
            t(locale.get(), "chat.steps_1").to_string()
        } else {
            tf(locale.get(), "chat.steps_n", &[("n", &n_tools.to_string())])
        }
    };
    let total_label =
        (total_ms > 0 && (!live || n_tools > 0)).then(|| format_duration_ms(total_ms));
    // The group re-renders on every streaming delta (fingerprint-keyed row),
    // so this static line tracks the in-flight step while collapsed.
    let now_line = live.then(|| steps_now_line(&items)).flatten();
    let rows = items.into_iter().enumerate().map(|(position, it)| match it {
        ChatItem::Assistant { text, .. } => {
            let step_id = format!("{group_id}:progress:{position}");
            let class_id = step_id.clone();
            let aria_id = step_id.clone();
            let toggle_id = step_id.clone();
            let detail: String = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim()
                .chars()
                .take(100)
                .collect();
            let html = md_to_html(&text);
            view! {
                <div class="step step-progress"
                    class:open=move || disclosure_open(disclosure_state, &class_id, false)>
                    <button type="button" class="step-head"
                        aria-expanded=move || disclosure_open(disclosure_state, &aria_id, false).to_string()
                        on:click=move |_| toggle_disclosure(disclosure_state, &toggle_id, false)>
                        <span class="step-icon progress"></span>
                        <span class="step-name">{move || t(locale.get(), "chat.progress")}</span>
                        <span class="step-detail">{detail}</span>
                    </button>
                    <div class="step-progress-body body md" inner_html=html></div>
                </div>
            }.into_view()
        }
        ChatItem::Reasoning(text) => {
            let step_id = format!("{group_id}:reasoning:{position}");
            let class_id = step_id.clone();
            let aria_id = step_id.clone();
            let toggle_id = step_id.clone();
            view! {
                <div class="step step-think"
                    class:open=move || disclosure_open(disclosure_state, &class_id, false)>
                    <button type="button" class="step-head"
                        aria-expanded=move || disclosure_open(disclosure_state, &aria_id, false).to_string()
                        on:click=move |_| toggle_disclosure(disclosure_state, &toggle_id, false)>
                        <span class="step-icon think"></span>
                        <span class="step-name">{move || t(locale.get(), "chat.thinking")}</span>
                    </button>
                    <div class="step-think-body">{text}</div>
                </div>
            }.into_view()
        }
        ChatItem::Tool { name, ok, input, output, started_at_ms, duration_ms, .. } => {
            let step_id = format!("{group_id}:tool:{position}");
            let automatic = ok.is_none() && live;
            let class_id = step_id.clone();
            let aria_id = step_id.clone();
            let toggle_id = step_id.clone();
            let (badge_key, title) = tool_card_label(&name, &input);
            let mut detail: String = input
                .lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
                .chars().take(80).collect();
            if detail == title {
                detail.clear();
            }
            let lines = if output.is_empty() { 0 } else { output.lines().count() };
            let has_body = !input.is_empty() || !output.is_empty();
            let icon = match ok {
                Some(true) => view! { <span class="step-icon ok">"✓"</span> }.into_view(),
                Some(false) => view! { <span class="step-icon fail">"✗"</span> }.into_view(),
                None => view! { <span class="step-icon run"><span class="run-dot"></span></span> }.into_view(),
            };
            let meta_text = step_tool_meta(locale.get(), duration_ms, started_at_ms, ok, lines, now);
            let meta = meta_text.map(|text| view! { <span class="step-meta">{text}</span> });
            view! {
                <div class="step"
                    class:open=move || disclosure_open(disclosure_state, &class_id, automatic)
                    class=("no-body", !has_body)>
                    <button type="button" class="step-head" disabled=!has_body
                        aria-expanded=move || (has_body && disclosure_open(disclosure_state, &aria_id, automatic)).to_string()
                        on:click=move |_| {
                        if has_body {
                            toggle_disclosure(disclosure_state, &toggle_id, automatic)
                        }
                    }>
                        {icon}
                        {badge_key.map(|key| view! {
                            <span class="tool-badge">{move || t(locale.get(), key)}</span>
                        })}
                        <span class="step-name">{title}</span>
                        {(!detail.is_empty()).then(|| view! { <span class="step-detail">{detail}</span> })}
                        {meta}
                    </button>
                    {has_body.then(|| view! {
                        <div class="step-body">
                            {(!input.is_empty()).then(|| view! { <pre class="tool-input">{input.clone()}</pre> })}
                            {(!output.is_empty()).then(|| view! { <pre class="tool-output">{output.clone()}</pre> })}
                        </div>
                    })}
                </div>
            }.into_view()
        }
        ChatItem::AcpTool { call_id, title, kind, status, content, locations, .. } => {
            let failed = status == "failed";
            let done = matches!(status.as_str(), "completed" | "failed");
            let running = !done;
            let stable_part = if call_id.is_empty() {
                format!("position-{position}")
            } else {
                call_id.clone()
            };
            let step_id = format!("{group_id}:acp:{stable_part}");
            let automatic = running && live;
            let class_id = step_id.clone();
            let aria_id = step_id.clone();
            let toggle_id = step_id.clone();
            let detail = acp_tool_step_detail(&kind, &content, &locations);
            let body = acp_tool_step_body(&content, &locations);
            let has_body = !body.is_empty();
            let icon = if failed {
                view! { <span class="step-icon fail">"✗"</span> }.into_view()
            } else if done {
                view! { <span class="step-icon ok">"✓"</span> }.into_view()
            } else {
                view! { <span class="step-icon run"><span class="run-dot"></span></span> }.into_view()
            };
            let meta = (!done).then(|| status.clone());
            view! {
                <div class="step acp-tool" data-testid="acp-tool" data-status=status.clone()
                    class:open=move || disclosure_open(disclosure_state, &class_id, automatic)
                    class=("no-body", !has_body)>
                    <button type="button" class="step-head" disabled=!has_body
                        aria-expanded=move || (has_body && disclosure_open(disclosure_state, &aria_id, automatic)).to_string()
                        on:click=move |_| {
                        if has_body {
                            toggle_disclosure(disclosure_state, &toggle_id, automatic)
                        }
                    }>
                        {icon}
                        <span class="step-name">{title.clone()}</span>
                        {(!detail.is_empty()).then(|| view! { <span class="step-detail">{detail.clone()}</span> })}
                        {meta.map(|text| view! { <span class="step-meta">{text}</span> })}
                    </button>
                    {has_body.then(|| view! {
                        <div class="step-body">
                            <pre class="tool-output">{body.clone()}</pre>
                        </div>
                    })}
                </div>
            }.into_view()
        }
        _ => view! {}.into_view(),
    }).collect_view();
    let class_group_id = group_id.clone();
    let aria_group_id = group_id.clone();
    let toggle_group_id = group_id.clone();
    view! {
        <div class="steps"
            class=("activity-summary", completed_turn)
            class:open=move || disclosure_open(disclosure_state, &class_group_id, live)>
            <button type="button" class="steps-head"
                aria-expanded=move || disclosure_open(disclosure_state, &aria_group_id, live).to_string()
                on:click=move |_| {
                toggle_disclosure(disclosure_state, &toggle_group_id, live)
            }>
                <span class="steps-chevron"></span>
                <span class="steps-title">{title}</span>
                {now_line.map(|text| view! { <span class="steps-now">{text}</span> })}
                {total_label.map(|label| view! { <span class="steps-meta">{label}</span> })}
            </button>
            <div class="steps-body">{rows}</div>
        </div>
    }
}

/// Latest step of a live run as "name · detail", shown in the collapsed
/// steps header so folding the panel hides detail, not progress.
fn steps_now_line(items: &[ChatItem]) -> Option<String> {
    let first_line = |s: &str| -> String {
        s.lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .chars()
            .take(80)
            .collect()
    };
    items.iter().rev().find_map(|item| match item {
        ChatItem::Tool { name, input, .. } => {
            let (_, title) = tool_card_label(name, input);
            let detail = first_line(input);
            Some(if detail.is_empty() || detail == title {
                title
            } else {
                format!("{title} · {detail}")
            })
        }
        ChatItem::AcpTool {
            title,
            kind,
            content,
            locations,
            ..
        } => {
            let detail = acp_tool_step_detail(kind, content, locations);
            Some(if detail.is_empty() {
                title.clone()
            } else {
                format!("{title} · {detail}")
            })
        }
        _ => None,
    })
}

#[cfg(test)]
mod steps_now_line_tests {
    use super::steps_now_line;
    use crate::dto::ChatItem;

    fn tool(name: &str, input: &str) -> ChatItem {
        ChatItem::Tool {
            name: name.into(),
            ok: None,
            input: input.into(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
        }
    }

    #[test]
    fn shows_latest_step() {
        let items = vec![
            ChatItem::Reasoning("hmm".into()),
            tool("python", "\nfrom pypdf import PdfReader\nmore"),
        ];
        assert_eq!(
            steps_now_line(&items),
            Some("python · from pypdf import PdfReader".into())
        );
        assert_eq!(steps_now_line(&[ChatItem::Reasoning("hmm".into())]), None);
        assert_eq!(steps_now_line(&[]), None);
    }
}

fn acp_tool_is_terminal_stub(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.starts_with('[') && trimmed.contains("\"terminalId\"") && !trimmed.contains('\n')
}

fn acp_tool_step_detail(kind: &str, content: &str, locations: &str) -> String {
    let from_locations = locations
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if !from_locations.is_empty() {
        return from_locations.chars().take(80).collect();
    }
    if acp_tool_is_terminal_stub(content) || content.trim().is_empty() {
        return kind.chars().take(80).collect();
    }
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn acp_tool_step_body(content: &str, locations: &str) -> String {
    let mut parts = Vec::new();
    if !locations.trim().is_empty() {
        parts.push(locations.trim().to_string());
    }
    if !content.trim().is_empty() && !acp_tool_is_terminal_stub(content) {
        parts.push(content.trim().to_string());
    }
    parts.join("\n")
}

fn run_output_preview(run: &RunRecord) -> String {
    let mut output = match (&run.stdout_tail, &run.stderr_tail) {
        (Some(stdout), Some(stderr)) if !stdout.is_empty() && !stderr.is_empty() => {
            format!("{stdout}\n[stderr]\n{stderr}")
        }
        (Some(stdout), _) => stdout.clone(),
        (_, Some(stderr)) => stderr.clone(),
        _ => String::new(),
    };
    let lines = output.lines().collect::<Vec<_>>();
    if lines.len() > 8 {
        output = lines[lines.len() - 8..].join("\n");
    }
    output
}

#[component]
fn RunMonitorCard(
    run_id: String,
    runs: RwSignal<Vec<RunRecord>>,
    tool_ok: Option<bool>,
    tool_output: String,
) -> impl IntoView {
    let locale = use_locale();
    let fallback = serde_json::from_str::<RunRecord>(&tool_output).ok();
    let lookup_id = run_id.clone();
    view! {
        {move || {
            let run = runs
                .get()
                .into_iter()
                .find(|run| run.id == lookup_id)
                .or_else(|| fallback.clone());
            let Some(run) = run else {
                let failed = tool_ok == Some(false);
                let status = if failed { "failed" } else { "running" };
                let status_class = format!("run-status {status}");
                let detail = if failed && !tool_output.trim().is_empty() {
                    tool_output.clone()
                } else {
                    t(locale.get(), "runs.waiting_record").to_string()
                };
                return view! {
                    <article class="run-monitor-card" data-testid="run-monitor-card" data-run-id=run_id.clone()>
                        <div class="run-monitor-head">
                            <span class="run-monitor-icon"><span class="run-dot"></span></span>
                            <div class="run-monitor-title">
                                <strong>{t(locale.get(), "runs.monitoring")}</strong>
                                <code>{run_id.clone()}</code>
                            </div>
                            <span class=status_class>{run_status_label(locale.get(), status)}</span>
                        </div>
                        <div class="run-monitor-empty">{detail}</div>
                    </article>
                }.into_view();
            };
            let title = run_title(&run);
            let status = run.status.clone();
            let status_class = format!("run-status {status}");
            let active = matches!(status.as_str(), "submitted" | "running" | "cancelling");
            let cancellable = matches!(status.as_str(), "submitted" | "running");
            let started = run.started_at.unwrap_or(run.created_at);
            let ended = run.ended_at.unwrap_or_else(|| js_sys::Date::now() as i64 / 1000);
            let elapsed_value = transfer_duration(ended.saturating_sub(started) as u64);
            let elapsed = tf(locale.get(), "runs.elapsed", &[("time", &elapsed_value)]);
            let meta = format!("{} · {} · {elapsed}", run.context_id, run.kind);
            let progress = run_progress(&run);
            let output = run_output_preview(&run);
            let command = run.command.clone().filter(|value| !value.trim().is_empty());
            let remote_workdir = run.remote_workdir.clone();
            let poll_error = run.last_poll_error.clone().filter(|value| !value.trim().is_empty());
            let cancel_id = run.id.clone();
            view! {
                <article class="run-monitor-card" data-testid="run-monitor-card" data-run-id=run.id>
                    <div class="run-monitor-head">
                        <span class="run-monitor-icon">{
                            if active {
                                view! { <span class="run-dot"></span> }.into_view()
                            } else if status == "succeeded" {
                                view! { <span class="run-monitor-done">"✓"</span> }.into_view()
                            } else {
                                view! { <span class="run-monitor-failed">"!"</span> }.into_view()
                            }
                        }</span>
                        <div class="run-monitor-title">
                            <strong>{title}</strong>
                            <code>{lookup_id.clone()}</code>
                        </div>
                        <span class=status_class>{run_status_label(locale.get(), &status)}</span>
                        {cancellable.then(|| view! {
                            <button type="button" class="icon-btn run-monitor-cancel"
                                title=t(locale.get(), "runs.cancel")
                                aria-label=t(locale.get(), "runs.cancel")
                                on:click=move |_| {
                                    let run_id = cancel_id.clone();
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                        let _ = invoke("cancel_run", arg).await;
                                    });
                                }>{compose_icon("close")}</button>
                        })}
                    </div>
                    <div class="run-monitor-meta">{meta}</div>
                    {progress.map(|progress| run_progress_meter(progress, locale.get()))}
                    {command.map(|command| view! { <div class="run-monitor-command">{command}</div> })}
                    {remote_workdir.map(|workdir| view! {
                        <div class="run-monitor-remote">
                            <span>{t(locale.get(), "runs.remote_workdir")}</span>
                            <code>{workdir}</code>
                        </div>
                    })}
                    {(!output.is_empty()).then(|| view! {
                        <div class="run-monitor-output">
                            <span>{t(locale.get(), "runs.output")}</span>
                            <pre>{output}</pre>
                        </div>
                    })}
                    {poll_error.map(|error| view! { <div class="context-error">{error}</div> })}
                </article>
            }.into_view()
        }}
    }
}

fn render_item(
    ui_index: usize,
    item: &ChatItem,
    artifacts: &[Artifact],
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
    runs: RwSignal<Vec<RunRecord>>,
    busy: ReadSignal<bool>,
    compact_assistant: bool,
    can_modify: bool,
    on_edit: impl Fn(usize) + Clone + 'static,
    on_branch: impl Fn(usize) + Clone + 'static,
    session_id: String,
    on_approval: Callback<(String, bool, Option<String>, String)>,
    on_resume: Callback<usize>,
    on_queue: Callback<QueueOp>,
) -> impl IntoView {
    let locale = use_locale();
    match item {
        ChatItem::User(s) => view! {
            <UserMessage
                text=s.clone()
                ui_index=ui_index
                busy=busy
                can_modify=can_modify
                on_copy=Callback::new(copy_text)
                on_edit=Callback::new(on_edit)
                on_branch=Callback::new(on_branch)
                on_file=on_file
            />
        }.into_view(),
        ChatItem::QueuedUser { id, text } => view! {
            <QueuedMessage
                id=*id
                text=text.clone()
                can_cut_in=can_modify
                on_queue=on_queue
            />
        }.into_view(),
        ChatItem::Assistant { text, .. } if text.trim().is_empty() => view! {}.into_view(),
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => {
            let msg = text.strip_prefix("Error: ").unwrap_or(text.as_str()).to_string();
            let copy = msg.clone();
            let hint_src = msg.clone();
            view! {
                <div class="finding err">
                    <div class="finding-head">
                        <span class="finding-tag">{move || format!("● {}", t(locale.get(), "chat.error"))}</span>
                        <span class="finding-title">{msg}</span>
                        {can_modify.then(|| view! {
                            <button type="button" class="tool-btn"
                                disabled=move || busy.get()
                                on:click=move |_| on_resume.call(ui_index)>
                                {move || t(locale.get(), "chat.resume")}
                            </button>
                        })}
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                    {move || i18n::api_error_hint(locale.get(), &hint_src).map(|hint| view! {
                        <div class="finding-body">{hint}</div>
                    })}
                </div>
            }.into_view()
        }
        ChatItem::Assistant { text, .. } if compact_assistant => view! {
            <div class="assistant-wrap">
                <div class="body md" inner_html=md_to_html(text)></div>
            </div>
        }.into_view(),
        ChatItem::Assistant { text, model, resources } => view! {
            <AssistantMessage
                text=text.clone()
                model=model.clone()
                resources=resources.clone()
                artifacts=artifacts.to_vec()
                source_item=ui_index
                on_artifact=on_artifact
                on_file=on_file
                on_copy=Callback::new(copy_text)
            />
        }.into_view(),
        ChatItem::Tool { name, .. } if name == "attempt_completion" => view! {}.into_view(),
        ChatItem::Tool { name, ok, input, output, .. } if is_run_monitor_tool(name) => view! {
            <RunMonitorCard
                run_id=input.trim().to_string()
                runs=runs
                tool_ok=*ok
                tool_output=output.clone()
            />
        }.into_view(),
        ChatItem::Reasoning(s) => {
            view! {
                <details class="rz">
                    <summary>{move || t(locale.get(), "chat.thinking")}</summary>
                    <div class="body">{s.clone()}</div>
                </details>
            }.into_view()
        }
        ChatItem::Tool { name, ok, input, output, .. } => view! {
            <ToolBlock name=name.clone() ok=*ok input=input.clone() output=output.clone() />
        }.into_view(),
        ChatItem::Usage { input, output, reasoning, cached } => {
            let (input, output, reasoning, cached) = (*input, *output, *reasoning, *cached);
            view! {
                <div class="usage-line" title=move || t(locale.get(), "msg.usage_title")>
                    {move || {
                        let loc = locale.get();
                        let mut s = tf(loc, "msg.usage", &[
                            ("in", &fmt_tokens(input)),
                            ("out", &fmt_tokens(output)),
                        ]);
                        if cached > 0 {
                            s.push_str(&tf(loc, "msg.usage.cached", &[("c", &fmt_tokens(cached))]));
                        }
                        if reasoning > 0 {
                            s.push_str(&tf(loc, "msg.usage.reasoning", &[("r", &fmt_tokens(reasoning))]));
                        }
                        s
                    }}
                </div>
            }.into_view()
        }
        ChatItem::AcpTool { title, status, content, locations, .. } => view! {
            <article class="tool-card" data-testid="acp-tool" data-status=status.clone()>
                <header><strong>{title.clone()}</strong><span>{status.clone()}</span></header>
                {(!content.is_empty()).then(|| view! { <pre>{content.clone()}</pre> })}
                {(!locations.is_empty()).then(|| view! { <pre>{locations.clone()}</pre> })}
            </article>
        }.into_view(),
        ChatItem::ApprovalPending { tool, preview, message: _ } => view! {
            <ApprovalCard tool=tool.clone() preview=preview.clone() session_id=session_id.clone() on_decide=on_approval />
        }.into_view(),
        ChatItem::AcpPermission { request_id, tool, options } => {
            let request_id = request_id.clone();
            view! {
                <article class="approval-card" data-testid="acp-permission-card">
                    <header><strong>{tool.clone()}</strong><span>"ACP permission"</span></header>
                    <footer class="approval-actions">
                        {options.clone().into_iter().map(|option| {
                            let request_id = request_id.clone();
                            let option_id = option.id.clone();
                            let class = if option.kind.starts_with("allow") { "primary" } else { "" };
                            view! {
                                <button type="button" class=class on:click=move |_| {
                                    let request_id = request_id.clone();
                                    let option_id = option_id.clone();
                                    spawn_local(async move {
                                        let args = to_value(&serde_json::json!({ "requestId": request_id, "optionId": option_id })).unwrap();
                                        let _ = invoke_checked("respond_acp_permission", args).await;
                                    });
                                }>{option.name}</button>
                            }
                        }).collect_view()}
                    </footer>
                </article>
            }.into_view()
        }
        ChatItem::ReviewTransition { phase, model } => {
            let (icon, message_key, data_phase) = match phase {
                ReviewTransitionPhase::Reviewing => {
                    ("↗", "review.transition_to_reviewer", "reviewing")
                }
                ReviewTransitionPhase::Correcting => {
                    ("↩", "review.transition_to_agent", "correcting")
                }
                ReviewTransitionPhase::Passed => {
                    ("✓", "review.transition_passed", "passed")
                }
            };
            let model = model.clone();
            view! {
                <div class="review-transition" data-phase=data_phase>
                    <span class="review-transition-line"></span>
                    <span class="review-transition-icon">{icon}</span>
                    <span class="review-transition-text">{move || t(locale.get(), message_key)}</span>
                    {model.map(|model| view! { <span class="review-transition-model">{model}</span> })}
                    <span class="review-transition-line"></span>
                </div>
            }.into_view()
        }
        ChatItem::Plan(plan) => {
            let html = md_to_html(&plan.text);
            view! {
                <article class="plan-card" data-testid="plan-card">
                    <header class="plan-card-head">
                        <span class="plan-card-icon">{compose_icon("plan")}</span>
                        <div><strong>{move || t(locale.get(), "plan.card.title")}</strong></div>
                    </header>
                    <div class="plan-card-body markdown" inner_html=html></div>
                </article>
            }.into_view()
        }
        ChatItem::Review(report) => {
            let report = report.clone();
            let count = report.findings.len();
            let unreviewable = report.review_status == "unreviewable";
            let coverage = report.evidence_coverage.to_string();
            let count_text = count.to_string();
            let all_resolved = count > 0
                && report
                    .findings
                    .iter()
                    .all(|finding| finding.status == "resolved");
            let has_unaddressed = report
                .findings
                .iter()
                .any(|finding| finding.status == "unaddressed");
            let copy = format!(
                "{}\n\n{}",
                report.summary,
                report
                    .findings
                    .iter()
                    .map(|finding| format!(
                        "- {}\n  Evidence: {}\n  Fix: {}",
                        finding.claim, finding.evidence, finding.fix
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let model = match (report.reviewer_model.trim(), report.reviewer_effort.trim()) {
                ("", "") => String::new(),
                (model, "") => model.to_string(),
                ("", effort) => effort.to_string(),
                (model, effort) => format!("{model} · {effort}"),
            };
            let summary = report.summary.clone();
            let coverage_gaps = report.coverage_gaps.clone();
            let findings = report
                .findings
                .into_iter()
                .enumerate()
                .map(|(index, finding)| {
                    let resolved = finding.status == "resolved";
                    let status_key = match finding.status.as_str() {
                        "resolved" => "review.resolved",
                        "unaddressed" => "review.unaddressed",
                        _ => "review.open",
                    };
                    let verdict_class = format!("review-pill verdict {}", finding.verdict);
                    let severity_class = format!("review-pill severity {}", finding.severity);
                    let message_index = finding.message_index;
                    view! {
                        <div class="review-finding" class:resolved=resolved>
                            <div class="review-finding-head">
                                <span class="review-finding-number">{index + 1}</span>
                                <span class=verdict_class>{finding.verdict}</span>
                                <span class=severity_class>{finding.severity}</span>
                                <span class="review-pill status">{move || t(locale.get(), status_key)}</span>
                                <button type="button" class="tool-btn review-jump"
                                    on:click=move |_| scroll_to_transcript(message_index)>
                                    {move || t(locale.get(), "review.go_to_transcript")}
                                </button>
                            </div>
                            <div class="review-claim">{finding.claim}</div>
                            <div class="review-detail">
                                <strong>{move || t(locale.get(), "review.evidence")}</strong>
                                <span>{finding.evidence}</span>
                            </div>
                            <div class="review-detail">
                                <strong>{move || t(locale.get(), "review.fix")}</strong>
                                <span>{finding.fix}</span>
                            </div>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="review-card">
                    <div class="review-head">
                        <span class="review-badge">"🔍"</span>
                        <span>{move || t(locale.get(), "review.title")}</span>
                        <span class="review-count">{move || tf(locale.get(), "review.findings_n", &[("n", &count_text)])}</span>
                        {(!model.is_empty()).then(|| view! { <span class="review-model">{model}</span> })}
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                    <div class="review-summary">{summary}</div>
                    {(count == 0 && !unreviewable).then(|| view! {
                        <div class="review-empty">"✓ "{move || t(locale.get(), "review.no_findings")}</div>
                    })}
                    {unreviewable.then(|| view! {
                        <div class="review-empty review-unreviewable">
                            "⚠ "{move || tf(locale.get(), "review.unreviewable", &[("pct", &coverage)])}
                        </div>
                    })}
                    {(!coverage_gaps.is_empty()).then(|| view! {
                        <details class="review-coverage-gaps">
                            <summary>{move || t(locale.get(), "review.coverage_gaps")}</summary>
                            <ul>{coverage_gaps.into_iter().map(|gap| view! { <li>{gap}</li> }).collect_view()}</ul>
                        </details>
                    })}
                    {findings}
                    {(count > 0).then(|| view! {
                        <div class="review-foot" class:resolved=all_resolved class:unaddressed=has_unaddressed>
                            {move || {
                                let key = if all_resolved {
                                    "review.all_fixed"
                                } else if has_unaddressed {
                                    "review.needs_attention"
                                } else {
                                    "review.agent_correcting"
                                };
                                t(locale.get(), key)
                            }}
                        </div>
                    })}
                </div>
            }.into_view()
        }
    }
}

pub fn main() {
    console_error_panic_hook::set_once();
    let is_pet_window = window().location().search().ok().is_some_and(|query| {
        query
            .split('&')
            .any(|part| part == "?pet=desktop" || part == "pet=desktop")
    });
    if is_pet_window {
        mount_to_body(PetDesktop);
    } else {
        mount_to_body(App);
    }
}
