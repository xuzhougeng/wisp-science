mod bindings;
mod context_menu;
mod dto;
mod i18n;
mod notebook;
mod overlays;
mod project_landing;
mod settings_view;
mod sidebar;
mod text;
mod window_titlebar;

use bindings::{
    attach_chat_autoscroll, force_chat_bottom, invoke, invoke_checked, invoke_timeout, is_mac,
    is_windows, listen, listen_native_file_drop, native_drop_in_composer, open_external_url,
    pasted_image_count, schedule_chat_follow, CHAT_SCROLLER_ID, CHAT_THREAD_ID,
};
use context_menu::{ContextMenuPortal, CtxMenu};
use dto::*;
use i18n::{empty_subtitle, empty_title, localize_backend, set_document_lang, tab_count, tf, t, use_locale, Locale, EMPTY_SUBTITLE_COUNT, EMPTY_TITLE_COUNT};
use notebook::{collect_notebook_cells, NotebookCache, NotebookView};
use overlays::{AddHostOverlay, CapabilitiesOverlay, OnboardingOverlay};
use project_landing::{ProjectLanding, ProjectLandingState};
use settings_view::{SettingsView, SettingsViewState};
use sidebar::{Sidebar, SidebarState};
use window_titlebar::WindowTitlebar;
use leptos::{ev, window_event_listener, *};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use text::{
    dom_value, event_target_value, format_bytes,
    format_duration_ms, group_artifact_indices, join_path, md_to_html, opens_in_system_browser,
    parent_path, provider_defaults, provider_value,
};
use serde_wasm_bindgen::to_value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use futures_channel::oneshot;
use std::collections::VecDeque;

/// Stable substring of the backend's missing-key error (`src-tauri` `send_message`),
/// used to turn that failure into an actionable "open Settings" prompt.
const NO_API_KEY_MARK: &str = "No API key set";
const HOME_SEARCH_PROJECT_LIMIT: usize = 6;
const HOME_SEARCH_ARTIFACT_LIMIT: usize = 8;
const HOME_SEARCH_SESSION_LIMIT: usize = 6;
const THEME_STORAGE_KEY: &str = "wisp-theme";

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

fn decode_runtime_snapshot(value: JsValue) -> Option<RuntimeSnapshot> {
    serde_wasm_bindgen::from_value::<RuntimeSnapshot>(value.clone()).ok().or_else(|| {
        serde_wasm_bindgen::from_value::<serde_json::Value>(value).ok()
            .and_then(|envelope| envelope.get("snapshot").cloned())
            .and_then(|snapshot| serde_json::from_value(snapshot).ok())
    })
}

fn is_codex_config_changed_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("configuration changed")
        || message.contains("config version")
        || (message.contains("expected version") && message.contains("current"))
}

fn decode_resolved_turn_config(value: JsValue) -> Option<ResolvedTurnConfig> {
    serde_wasm_bindgen::from_value::<ResolvedTurnConfig>(value.clone()).ok().or_else(|| {
        serde_wasm_bindgen::from_value::<serde_json::Value>(value).ok()
            .and_then(|envelope| envelope.get("config").or_else(|| envelope.get("resolved")).cloned())
            .and_then(|config| serde_json::from_value(config).ok())
    })
}

fn decode_session_codex_state(value: JsValue) -> Option<SessionCodexState> {
    serde_wasm_bindgen::from_value::<SessionCodexState>(value.clone()).ok().or_else(|| {
        serde_wasm_bindgen::from_value::<serde_json::Value>(value).ok()
            .and_then(|value| serde_json::from_value(value).ok())
    })
}

fn decode_proposed_plan(value: JsValue) -> Option<ProposedPlanRecord> {
    serde_wasm_bindgen::from_value::<Option<ProposedPlanRecord>>(value.clone()).ok().flatten().or_else(|| {
        serde_wasm_bindgen::from_value::<serde_json::Value>(value).ok()
            .and_then(|envelope| envelope.get("plan").cloned())
            .and_then(|plan| serde_json::from_value(plan).ok())
    })
}

fn refresh_codex_turn_audits(
    session_id: String,
    active_session: RwSignal<Option<String>>,
    audits: RwSignal<Vec<CodexTurnConfigAudit>>,
) {
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
        if let Ok(value) = invoke_checked("get_codex_turn_configs", args).await {
            if let Ok(records) = serde_wasm_bindgen::from_value::<Vec<CodexTurnConfigAudit>>(value) {
                if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                    audits.set(records);
                }
            }
        }
    });
}

fn audit_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn actual_verification_unavailable(value: &serde_json::Value) -> bool {
    value.is_null()
        || value.as_object().is_some_and(|object| {
            object.is_empty()
                || object.get("verification").and_then(|value| value.as_str()) == Some("unavailable")
                || object.get("verified").and_then(|value| value.as_bool()) == Some(false)
        })
}

fn proposed_plan_card(plan: ProposedPlanRecord) -> PlanCard {
    PlanCard {
        complete: !matches!(plan.status.as_str(), "streaming" | "draft"),
        actionable: plan.status == "pending" && !plan.id.is_empty() && plan.revision > 0,
        text: plan.text,
        native: plan.native,
        plan_id: plan.id,
        revision: plan.revision,
    }
}

fn upsert_persisted_plan(items: &mut Vec<ChatItem>, plan: ProposedPlanRecord) {
    let card = proposed_plan_card(plan);
    if let Some(index) = items.iter().rposition(|item| matches!(
        item,
        ChatItem::Plan(existing) if existing.plan_id == card.plan_id && existing.revision == card.revision
    )) {
        items[index] = ChatItem::Plan(card);
    } else {
        let index = trailing_queue_start(items);
        items.insert(index, ChatItem::Plan(card));
    }
}

fn request_codex_runtime_snapshot_with_ack(
    refresh: bool,
    report_error: bool,
    show_loading: bool,
    sync_profile_overrides: bool,
    request_nonce: Rc<Cell<u64>>,
    models: RwSignal<Vec<ModelProfile>>,
    expected_project_id: RwSignal<String>,
    runtime: RwSignal<Option<RuntimeSnapshot>>,
    runtime_error: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    profile_overrides: RwSignal<CodexModeOverrides>,
    clear_refresh_pending_on_success: Option<RwSignal<bool>>,
) {
    if show_loading { loading.set(true); }
    let request_id = request_nonce.get().wrapping_add(1);
    request_nonce.set(request_id);
    spawn_local(async move {
        let command = if refresh { "refresh_codex_runtime_snapshot" } else { "get_codex_runtime_snapshot" };
        let response = invoke_checked(command, JsValue::UNDEFINED).await;
        if request_nonce.get() != request_id { return; }
        match response {
            Ok(value) => {
                if let Some(snapshot) = decode_runtime_snapshot(value) {
                    let profiles = models.get_untracked();
                    let active_codex_id = profiles.iter()
                        .find(|profile| profile.active)
                        .or_else(|| profiles.first())
                        .filter(|profile| active_profile_uses_codex(std::slice::from_ref(*profile)))
                        .map(|profile| profile.id.as_str());
                    if !snapshot.profile_id.is_empty()
                        && !profiles.is_empty()
                        && active_codex_id != Some(snapshot.profile_id.as_str())
                    {
                        loading.set(false);
                        return;
                    }
                    let expected_project = expected_project_id.get_untracked();
                    if !snapshot.project_id.is_empty()
                        && !expected_project.is_empty()
                        && snapshot.project_id != expected_project
                    {
                        loading.set(false);
                        return;
                    }
                    // Passive reads must never overwrite edits still sitting in
                    // the Profile form. Only initial load and an explicit user
                    // refresh opt into reloading this layer.
                    if sync_profile_overrides {
                        if let Some(saved) = snapshot.profile_overrides.clone() {
                            profile_overrides.set(saved);
                        }
                    }
                    runtime.set(Some(snapshot));
                    runtime_error.set(None);
                    if let Some(pending) = clear_refresh_pending_on_success {
                        pending.set(false);
                    }
                } else if report_error {
                    runtime_error.set(Some("Codex runtime returned an unsupported snapshot format.".into()));
                }
            }
            Err(error) if report_error => runtime_error.set(Some(js_error_text(error))),
            Err(_) => {} // passive reads preserve the last usable snapshot
        }
        if request_nonce.get() == request_id { loading.set(false); }
    });
}

fn request_codex_runtime_snapshot(
    refresh: bool,
    report_error: bool,
    show_loading: bool,
    sync_profile_overrides: bool,
    request_nonce: Rc<Cell<u64>>,
    models: RwSignal<Vec<ModelProfile>>,
    expected_project_id: RwSignal<String>,
    runtime: RwSignal<Option<RuntimeSnapshot>>,
    runtime_error: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    profile_overrides: RwSignal<CodexModeOverrides>,
) {
    request_codex_runtime_snapshot_with_ack(
        refresh,
        report_error,
        show_loading,
        sync_profile_overrides,
        request_nonce,
        models,
        expected_project_id,
        runtime,
        runtime_error,
        loading,
        profile_overrides,
        None,
    );
}

fn active_profile_uses_codex(models: &[ModelProfile]) -> bool {
    models.iter().find(|model| model.active).or_else(|| models.first())
        .is_some_and(|model| matches!(model.provider.as_str(), "codex" | "codex_cli" | "codex_local" | "codex-local"))
}

fn active_profile_uses_local_runner(models: &[ModelProfile]) -> bool {
    models.iter().find(|model| model.active).or_else(|| models.first())
        .is_some_and(|model| matches!(
            model.provider.as_str(),
            "codex" | "codex_cli" | "codex_local" | "codex-local" | "claude" | "claude_code" | "claude-code"
        ))
}

fn optional_profile_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "inherit" | "default" | "codex-default" | "inherit_local_codex_default"
        ))
    .then(|| value.to_string())
}

fn codex_overrides_from_profile(profile: &ModelProfile) -> CodexModeOverrides {
    CodexModeOverrides {
        normal: CodexModeOverride {
            model: optional_profile_value(&profile.normal_model),
            effort: optional_profile_value(&profile.normal_reasoning_effort),
        },
        plan: CodexModeOverride {
            model: optional_profile_value(&profile.plan_model),
            effort: optional_profile_value(&profile.plan_reasoning_effort),
        },
        service_tier: optional_profile_value(&profile.service_tier),
        personality: optional_profile_value(&profile.personality),
        summary: optional_profile_value(&profile.reasoning_summary),
        verbosity: optional_profile_value(&profile.verbosity),
        web_search: optional_profile_value(&profile.runner_web_search_mode),
        sandbox: optional_profile_value(&profile.runner_sandbox),
    }
}

fn composer_codex_efforts(
    snapshot: &RuntimeSnapshot,
    model: Option<&str>,
    inherited_effective_model: Option<&str>,
) -> Vec<String> {
    if let Some(model_id) = model {
        if let Some(model) = snapshot.models.iter().find(|item| item.id == model_id) {
            return model.supported_reasoning_efforts.clone();
        }
        return vec!["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
            .into_iter().map(str::to_string).collect();
    }
    if let Some(model) = inherited_effective_model
        .and_then(|value| snapshot.models.iter().find(|item| item.id == value))
    {
        return model.supported_reasoning_efforts.clone();
    }
    vec!["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
        .into_iter().map(str::to_string).collect()
}

/// Remove only the rows this send inserted optimistically.  Matching the
/// adjacent empty assistant placeholder avoids deleting streamed/persisted
/// content if the turn actually began before a later transport error.
fn remove_optimistic_send_rows(rows: &mut Vec<ChatItem>, display_message: &str) {
    let Some(index) = rows.iter().rposition(|item| matches!(
        item,
        ChatItem::User(value) | ChatItem::QueuedUser(value) if value == display_message
    )) else { return; };
    if matches!(rows.get(index), Some(ChatItem::QueuedUser(_))) {
        rows.remove(index);
        return;
    }
    if matches!(rows.get(index + 1), Some(ChatItem::Assistant { text, .. }) if text.is_empty()) {
        rows.drain(index..=index + 1);
    }
}

fn rollback_optimistic_send(
    target_session: Option<&str>,
    active_session: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    display_message: &str,
) {
    if let Some(session_id) = target_session {
        route_items(active_session, items, transcripts, session_id, |rows| {
            remove_optimistic_send_rows(rows, display_message)
        });
    } else {
        items.update(|rows| remove_optimistic_send_rows(rows, display_message));
    }
}

fn mark_optimistic_send_failed(rows: &mut Vec<ChatItem>, display_message: &str, error: &str) {
    let Some(index) = rows.iter().rposition(|item| matches!(
        item,
        ChatItem::User(value) if value == display_message
    )) else { return; };
    if let Some(ChatItem::Assistant { text, .. }) = rows.get_mut(index + 1) {
        if text.is_empty() { *text = format!("Error: {error}"); }
    }
}

fn split_turn_started_error(error: &str) -> (bool, &str) {
    error.strip_prefix("[turn-started] ")
        .map_or((false, error), |message| (true, message))
}

fn restore_failed_composer(
    input: RwSignal<String>,
    attachments: RwSignal<Vec<ComposerAttachment>>,
    references: RwSignal<Vec<ComposerReferenceChip>>,
    draft: String,
    saved_attachments: Vec<ComposerAttachment>,
    saved_references: Vec<ComposerReferenceChip>,
) {
    // Do not trample text/files the user may already have started adding for
    // their next turn while the failed request was in flight.
    if input.get_untracked().is_empty() { input.set(draft); }
    if attachments.get_untracked().is_empty() { attachments.set(saved_attachments); }
    if references.get_untracked().is_empty() { references.set(saved_references); }
}

#[derive(Clone, Copy)]
struct CodexOverrideUi {
    active_session: RwSignal<Option<String>>,
    visible_overrides: RwSignal<CodexModeOverrides>,
    visible_mode: RwSignal<String>,
    needs_confirmation: RwSignal<bool>,
    write_conflicted: RwSignal<bool>,
    status: RwSignal<String>,
    locale: RwSignal<Locale>,
}

struct PendingCodexOverrideWrite {
    session_id: Option<String>,
    mode: String,
    generation: Option<String>,
    overrides: CodexModeOverrides,
    ui: CodexOverrideUi,
    completion: Option<oneshot::Sender<Result<(), String>>>,
}

#[derive(Default)]
struct CodexOverrideWriteQueue {
    running: HashSet<String>,
    pending: HashMap<String, VecDeque<PendingCodexOverrideWrite>>,
    revisions: HashMap<String, String>,
}

thread_local! {
    static CODEX_OVERRIDE_WRITES: RefCell<CodexOverrideWriteQueue> = RefCell::new(CodexOverrideWriteQueue::default());
}

fn set_codex_override_revision(session_id: &str, revision: String) -> bool {
    if revision.is_empty() { return false; }
    CODEX_OVERRIDE_WRITES.with(|queue| {
        let mut queue = queue.borrow_mut();
        let is_stale = queue.revisions.get(session_id).is_some_and(|current| {
            match (current.parse::<u64>(), revision.parse::<u64>()) {
                (Ok(current), Ok(candidate)) => candidate < current,
                _ => false,
            }
        });
        if is_stale { return false; }
        queue.revisions.insert(session_id.to_string(), revision);
        true
    })
}

fn enqueue_session_codex_overrides(
    session_id: Option<String>,
    mode: String,
    generation: Option<String>,
    overrides: CodexModeOverrides,
    ui: CodexOverrideUi,
    completion: Option<oneshot::Sender<Result<(), String>>>,
) {
    if ui.write_conflicted.get_untracked() {
        if let Some(completion) = completion {
            let _ = completion.send(Err("Codex session configuration must be confirmed after a revision conflict.".into()));
        }
        return;
    }
    let key = session_id.clone().unwrap_or_else(|| "__pending_session__".into());
    let should_start = CODEX_OVERRIDE_WRITES.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.pending.entry(key.clone()).or_default().push_back(PendingCodexOverrideWrite {
            session_id, mode, generation, overrides, ui, completion,
        });
        queue.running.insert(key.clone())
    });
    if !should_start { return; }
    spawn_local(async move {
        loop {
            let request = CODEX_OVERRIDE_WRITES.with(|queue| {
                queue.borrow_mut().pending.get_mut(&key).and_then(VecDeque::pop_front)
            });
            let Some(request) = request else {
                CODEX_OVERRIDE_WRITES.with(|queue| {
                    let mut queue = queue.borrow_mut();
                    queue.pending.remove(&key);
                    queue.running.remove(&key);
                });
                break;
            };
            let mut expected_revision = CODEX_OVERRIDE_WRITES.with(|queue| {
                queue.borrow().revisions.get(&key).cloned()
            });
            if expected_revision.is_none() {
                if let Some(session_id) = request.session_id.as_ref() {
                    let args = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
                    match invoke_checked("get_session_codex_overrides", args).await {
                        Ok(value) => if let Some(saved) = decode_session_codex_state(value) {
                            if !saved.revision.is_empty() {
                                set_codex_override_revision(session_id, saved.revision);
                                expected_revision = CODEX_OVERRIDE_WRITES.with(|queue| {
                                    queue.borrow().revisions.get(&key).cloned()
                                });
                            }
                        },
                        Err(error) => {
                            let message = js_error_text(error);
                            if let Some(completion) = request.completion { let _ = completion.send(Err(message.clone())); }
                            request.ui.status.set(tf(request.ui.locale.get_untracked(), "codex.override_failed", &[
                                ("msg", &localize_backend(request.ui.locale.get_untracked(), &message)),
                            ]));
                            continue;
                        }
                    }
                }
            }
            let args = to_value(&serde_json::json!({
                "sessionId": request.session_id.clone(),
                "mode": request.mode.clone(),
                "configVersion": request.generation.clone(),
                "overrides": request.overrides.clone(),
                "expectedRevision": expected_revision,
            })).unwrap();
            match invoke_checked("set_session_codex_overrides", args).await {
                Ok(value) => {
                    if let Some(saved) = decode_session_codex_state(value) {
                        if let Some(session_id) = request.session_id.as_ref() {
                            set_codex_override_revision(session_id, saved.revision);
                        }
                    }
                    if let Some(completion) = request.completion { let _ = completion.send(Ok(())); }
                }
                Err(error) => {
                    let message = js_error_text(error);
                    let conflict = {
                        let lower = message.to_ascii_lowercase();
                        lower.contains("revision") || lower.contains("conflict") || lower.contains("changed")
                    };
                    if let Some(completion) = request.completion { let _ = completion.send(Err(message.clone())); }
                    let abandoned = CODEX_OVERRIDE_WRITES.with(|queue| {
                        queue.borrow_mut().pending.remove(&key).unwrap_or_default()
                    });
                    for abandoned in abandoned {
                        if let Some(completion) = abandoned.completion {
                            let _ = completion.send(Err(message.clone()));
                        }
                    }
                    if conflict {
                        request.ui.needs_confirmation.set(true);
                        request.ui.write_conflicted.set(true);
                        if let Some(session_id) = request.session_id.as_ref() {
                            let args = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
                            if let Ok(value) = invoke_checked("get_session_codex_overrides", args).await {
                                if let Some(saved) = decode_session_codex_state(value) {
                                    let accepted = set_codex_override_revision(session_id, saved.revision);
                                    if accepted && request.ui.active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                                        request.ui.visible_overrides.set(saved.overrides);
                                        request.ui.visible_mode.set(if saved.mode == "plan" { "plan".into() } else { "default".into() });
                                    }
                                }
                            }
                        }
                    }
                    request.ui.status.set(tf(request.ui.locale.get_untracked(), "codex.override_failed", &[
                        ("msg", &localize_backend(request.ui.locale.get_untracked(), &message)),
                    ]));
                }
            }
        }
    });
}

fn persist_session_codex_overrides(
    session_id: Option<String>,
    mode: String,
    generation: Option<String>,
    overrides: CodexModeOverrides,
    ui: CodexOverrideUi,
) {
    enqueue_session_codex_overrides(session_id, mode, generation, overrides, ui, None);
}

async fn persist_session_codex_overrides_await(
    session_id: Option<String>,
    mode: String,
    generation: Option<String>,
    overrides: CodexModeOverrides,
    ui: CodexOverrideUi,
) -> Result<(), String> {
    let (sender, receiver) = oneshot::channel();
    enqueue_session_codex_overrides(session_id, mode, generation, overrides, ui, Some(sender));
    receiver.await.unwrap_or_else(|_| Err("Codex session configuration write was cancelled.".into()))
}

fn is_plan_command(text: &str) -> bool {
    let command = text.trim_start();
    command == "/plan" || command.starts_with("/plan ") || command == "/default"
}

mod app_support;
use app_support::*;

#[component]
fn App() -> impl IntoView {
    let locale = create_rw_signal(Locale::detect_browser());
    provide_context(locale.read_only());
    let theme_mode = create_rw_signal(load_theme_mode());
    create_effect(move |_| apply_theme_mode(&theme_mode.get()));

    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
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
    let pending_turns = create_rw_signal::<HashMap<String, usize>>(HashMap::new());
    let transcripts = create_rw_signal::<HashMap<String, Vec<ChatItem>>>(HashMap::new());
    let busy = create_rw_signal(false);
    // Interrupting a running turn (esp. the python kernel) is not instant, so
    // keep track of the session whose Stop click is waiting for the backend.
    let stopping_session = create_rw_signal::<Option<String>>(None);
    let show_settings = create_rw_signal(false);
    let settings_section = create_rw_signal(String::from("general"));
    let skills_list = create_rw_signal(Vec::<SkillRow>::new());
    let skills_search = create_rw_signal(String::new());
    let skills_msg = create_rw_signal(None::<(bool, String)>);
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
    let conn_form = create_rw_signal(None::<ConnForm>);
    let conn_test_msg = create_rw_signal(None::<(bool,String)>);
    // Service credentials (Settings → Credentials, #115). `cred_status` maps a
    // credential id -> whether a value is stored; `cred_inputs` holds the
    // in-progress edit per id; one shared status message.
    let cred_status = create_rw_signal(std::collections::HashMap::<String, bool>::new());
    let cred_inputs = create_rw_signal(std::collections::HashMap::<String, String>::new());
    let cred_msg = create_rw_signal(None::<(bool,String)>);
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
    // Configured model profiles + the composer's bottom-right picker state.
    let models = create_rw_signal::<Vec<ModelProfile>>(vec![]);
    let show_projects = create_rw_signal(true); // app lands on the Projects screen
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    let demo_mode = create_rw_signal(false); // true = the synthetic "Example project" is open
    let project_open_error = create_rw_signal(None::<String>);
    let project_transition_epoch = Rc::new(Cell::new(0u64));
    let project_transition_target = Rc::new(RefCell::new(None::<String>));
    let project_open_gate = Rc::new(RefCell::new(ProjectOpenGate::default()));
    let model_menu_open = create_rw_signal(false);
    let codex_config_menu_open = create_rw_signal(false);
    let codex_runtime = create_rw_signal(None::<RuntimeSnapshot>);
    let codex_runtime_error = create_rw_signal(None::<String>);
    let codex_runtime_loading = create_rw_signal(false);
    let codex_settings_action_loading = create_rw_signal(false);
    let codex_runtime_request_nonce = Rc::new(Cell::new(0u64));
    let codex_config_needs_confirmation = create_rw_signal(false);
    let codex_session_override_conflict = create_rw_signal(false);
    let codex_runtime_refresh_pending = create_rw_signal(false);
    let last_codex_generation = Rc::new(RefCell::new(String::new()));
    let last_codex_runtime_scope = Rc::new(RefCell::new(None::<String>));
    let codex_expected_project_id = create_rw_signal(String::new());
    let codex_profile_overrides = create_rw_signal(CodexModeOverrides::default());
    let codex_session_overrides = create_rw_signal(CodexModeOverrides::default());
    let codex_preview = create_rw_signal(None::<ResolvedTurnConfig>);
    let codex_preview_loading = create_rw_signal(false);
    let codex_preview_request_nonce = Rc::new(Cell::new(0u64));
    let codex_settings_preview_nonce = Rc::new(Cell::new(0u64));
    let codex_preview_normal = create_rw_signal(None::<ResolvedTurnConfig>);
    let codex_preview_plan = create_rw_signal(None::<ResolvedTurnConfig>);
    let codex_turn_audits = create_rw_signal(Vec::<CodexTurnConfigAudit>::new());
    let collaboration_mode = create_rw_signal(String::from("default"));
    let status = create_rw_signal(String::new());
    {
        let last_codex_generation = last_codex_generation.clone();
        create_effect(move |_| {
            let generation = codex_runtime.get().map(|snapshot| snapshot.config_version).unwrap_or_default();
            let previous = std::mem::replace(&mut *last_codex_generation.borrow_mut(), generation.clone());
            if !previous.is_empty() && !generation.is_empty() && previous != generation {
                codex_runtime_refresh_pending.set(false);
                codex_config_needs_confirmation.set(true);
            }
        });
    }
    {
        let last_codex_runtime_scope = last_codex_runtime_scope.clone();
        let profile_runtime_request_nonce = codex_runtime_request_nonce.clone();
        let profile_preview_request_nonce = codex_preview_request_nonce.clone();
        let profile_settings_preview_nonce = codex_settings_preview_nonce.clone();
        create_effect(move |_| {
            let list = models.get();
            if list.is_empty() { return; }
            let profile = list.iter().find(|model| model.active).or_else(|| list.first())
                .filter(|profile| matches!(profile.provider.as_str(), "codex_cli" | "codex" | "codex_local" | "codex-local"));
            if let Some(profile) = profile {
                codex_profile_overrides.set(codex_overrides_from_profile(profile));
            } else {
                codex_profile_overrides.set(CodexModeOverrides::default());
            }
            let project = project_info.get();
            let is_project_landing = show_projects.get();
            let is_demo = demo_mode.get();
            let next_scope = if is_project_landing || is_demo {
                None
            } else {
                profile.zip(project.as_ref()).map(|(profile, project)| {
                    format!("{}\u{1f}{}\u{1f}{}", profile.id, project.id, project.root)
                })
            };
            let next_project_id = next_scope.as_ref()
                .and_then(|_| project.as_ref().map(|project| project.id.clone()))
                .unwrap_or_default();
            let awaiting_project = !is_project_landing && !is_demo && project.is_none() && profile.is_some();
            let previous = std::mem::replace(
                &mut *last_codex_runtime_scope.borrow_mut(),
                next_scope.clone(),
            );
            // `show_projects=false` is set before `get_project_info` resolves.
            // Even when both the old and new scopes are `None`, keep Codex
            // gated so the composer cannot briefly expose the exec fallback.
            if awaiting_project {
                codex_runtime_loading.set(true);
                codex_runtime_error.set(None);
            }
            if previous == next_scope { return; }

            // Never display or send a catalog/config snapshot resolved for the
            // previous Profile. Increment both request epochs so late responses
            // from that Profile are ignored as well.
            codex_runtime.set(None);
            codex_preview.set(None);
            codex_preview_normal.set(None);
            codex_preview_plan.set(None);
            codex_session_override_conflict.set(false);
            codex_runtime_loading.set(awaiting_project);
            codex_expected_project_id.set(next_project_id);
            profile_preview_request_nonce.set(profile_preview_request_nonce.get().wrapping_add(1));
            profile_settings_preview_nonce.set(profile_settings_preview_nonce.get().wrapping_add(1));
            profile_runtime_request_nonce.set(profile_runtime_request_nonce.get().wrapping_add(1));
            if next_scope.is_some() {
                codex_runtime_refresh_pending.set(false);
                // Selecting (or initially restoring) a Codex Profile adopts its
                // inherited snapshot as the new baseline. Confirmation is only
                // required when that baseline subsequently changes.
                codex_config_needs_confirmation.set(false);
                status.set(String::new());
                request_codex_runtime_snapshot(
                    false, false, true, false, profile_runtime_request_nonce.clone(), models, codex_expected_project_id,
                    codex_runtime, codex_runtime_error, codex_runtime_loading,
                    codex_profile_overrides,
                );
            } else {
                codex_runtime_refresh_pending.set(false);
                codex_config_needs_confirmation.set(false);
            }
        });
    }
    let send_mode_menu_open = create_rw_signal(false);
    let side_chat_input = create_rw_signal(String::new());
    let side_chat_items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    let side_chat_busy = create_rw_signal(false);
    let side_chat_model_menu_open = create_rw_signal(false);
    let settings_busy = create_rw_signal(false);
    let settings_message = create_rw_signal::<Option<(bool, String)>>(None);
    // Set when a send fails because no API key is configured, so the status bar
    // can offer a one-click jump to Settings instead of a dead-end message.
    let needs_api_key = create_rw_signal(false);
    let refresh_models = move || spawn_local(async move {
        let v = invoke("list_models", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
            if let Some(profile) = list.iter().find(|model| model.active).or_else(|| list.first())
                .filter(|profile| matches!(profile.provider.as_str(), "codex_cli" | "codex" | "codex_local" | "codex-local"))
            {
                codex_profile_overrides.set(codex_overrides_from_profile(profile));
            }
            models.set(list);
        }
    });
    // Tauri's native drag/drop event contains absolute paths (including
    // directories). Keep those paths as references; unlike the browser File
    // picker they must not be copied through `upload_file` first.
    let native_drop_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let inside = native_drop_in_composer(payload.clone());
        let value = serde_wasm_bindgen::from_value::<serde_json::Value>(payload).unwrap_or_default();
        let kind = value.get("kind").and_then(|item| item.as_str()).unwrap_or("").to_ascii_lowercase();
        if matches!(kind.as_str(), "enter" | "over" | "hover" | "hovered") {
            drag_over.set(inside);
            return;
        }
        if matches!(kind.as_str(), "leave" | "cancel" | "cancelled") {
            drag_over.set(false);
            return;
        }
        if !matches!(kind.as_str(), "drop" | "dropped") { return; }
        drag_over.set(false);
        if !inside { return; }
        let paths = value.get("paths").and_then(|item| item.as_array()).cloned().unwrap_or_default();
        for path in paths.into_iter().filter_map(|item| item.as_str().map(str::to_string)) {
            if attachments.get_untracked().iter().any(|attachment| matches!(attachment, ComposerAttachment::Ready { path: existing, .. } if existing == &path)) {
                continue;
            }
            let name = path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).unwrap_or(&path).to_string();
            attachments.update(|items| items.push(ComposerAttachment::Ready {
                key: format!("native:{path}"), name, path,
            }));
        }
        if !active_profile_uses_local_runner(&models.get_untracked()) {
            status.set(t(locale.get_untracked(), "composer.native_path_api_hint").into());
        }
    }) as Box<dyn FnMut(JsValue)>);
    let native_drop_js = native_drop_cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(native_drop_cb);
    spawn_local(async move { let _ = listen_native_file_drop(&native_drop_js).await; });
    let refresh_specialists = move || spawn_local(async move {
        let v = invoke("list_specialists", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) { specialists.set(list); }
    });
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
    let folders = create_rw_signal::<Vec<FolderInfo>>(vec![]);
    let collapsed_folders = create_rw_signal::<HashSet<String>>(HashSet::new());
    let drag_session = create_rw_signal::<Option<String>>(None);
    let drop_target = create_rw_signal::<Option<String>>(None);
    let active_session = create_rw_signal::<Option<String>>(None);
    let codex_override_ui = CodexOverrideUi {
        active_session,
        visible_overrides: codex_session_overrides,
        visible_mode: collaboration_mode,
        needs_confirmation: codex_config_needs_confirmation,
        write_conflicted: codex_session_override_conflict,
        status,
        locale,
    };
    refresh_sessions(sessions);
    refresh_folders(folders);

    // Load the selected local runtime once when Models settings opens. Runtime
    // changes are applied only by the explicit refresh action (or send-time
    // validation), so merely leaving Settings open never churns the actor.
    {
        let settings_runtime_request_nonce = codex_runtime_request_nonce.clone();
        create_effect(move |_| {
            if show_settings.get() && settings_section.get() == "models"
                && !show_projects.get() && project_info.get().is_some()
            {
                request_codex_runtime_snapshot(
                    false, false, false, false, settings_runtime_request_nonce.clone(), models, codex_expected_project_id,
                    codex_runtime, codex_runtime_error,
                    codex_runtime_loading, codex_profile_overrides,
                );
            }
        });
    }
    create_effect(move |_| {
        codex_session_override_conflict.set(false);
        let Some(session_id) = active_session.get() else {
            codex_session_overrides.set(CodexModeOverrides::default());
            codex_turn_audits.set(vec![]);
            collaboration_mode.set("default".into());
            return;
        };
        // Reset immediately so a rapid session switch cannot momentarily send
        // the previous conversation's override. Restore the persisted layer
        // only if this is still the active session when the command resolves.
        codex_session_overrides.set(CodexModeOverrides::default());
        codex_turn_audits.set(vec![]);
        collaboration_mode.set("default".into());
        refresh_codex_turn_audits(session_id.clone(), active_session, codex_turn_audits);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
            if let Ok(value) = invoke_checked("get_session_codex_overrides", args).await {
                if let Some(saved) = decode_session_codex_state(value) {
                    let accepted = set_codex_override_revision(&session_id, saved.revision.clone());
                    if accepted && active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                        codex_session_overrides.set(saved.overrides);
                        collaboration_mode.set(if saved.mode == "plan" { "plan".into() } else { "default".into() });
                    }
                }
            }
        });
    });

    // Preview is the composer's source of truth. It is refreshed from the same
    // backend resolver whenever the mode/session/runtime generation or the
    // visible session override changes.
    {
        let request_nonce = codex_preview_request_nonce.clone();
        create_effect(move |_| {
            let mode = collaboration_mode.get();
            let session_id = active_session.get();
            let overrides = codex_session_overrides.get();
            let generation = codex_runtime.get().map(|snapshot| snapshot.config_version);
            let request_id = request_nonce.get().wrapping_add(1);
            request_nonce.set(request_id);
            if !active_profile_uses_codex(&models.get()) {
                codex_preview.set(None);
                codex_preview_loading.set(false);
                return;
            }
            if generation.is_none() {
                codex_preview.set(None);
                codex_preview_loading.set(false);
                return;
            }
            // Never show the previous mode/session as if it described the new
            // selection while its resolver call is still pending.
            codex_preview.set(None);
            codex_preview_loading.set(true);
            let response_nonce = request_nonce.clone();
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "sessionId": session_id.clone(),
                    "mode": mode.clone(),
                    "overrides": overrides.clone(),
                    "configVersion": generation.clone(),
                    "previewScope": "session",
                })).unwrap();
                let response = invoke_checked("preview_codex_turn_config", args).await;
                let still_current = response_nonce.get() == request_id
                    && active_session.get_untracked() == session_id
                    && collaboration_mode.get_untracked() == mode
                    && codex_session_overrides.get_untracked() == overrides
                    && codex_runtime.get_untracked().map(|snapshot| snapshot.config_version) == generation
                    && active_profile_uses_codex(&models.get_untracked());
                if still_current {
                    if let Ok(value) = response {
                        codex_preview.set(decode_resolved_turn_config(value));
                    }
                    codex_preview_loading.set(false);
                }
            });
        });
    }

    // `busy` is "the active session is currently streaming" — derived from the
    // per-session `running` set so it stays correct when the user switches
    // conversations or a background turn finishes.
    create_effect(move |_| {
        let r = running.get();
        let b = active_session.get().map(|id| r.contains(&id)).unwrap_or(false);
        busy.set(b);
    });

    // Refresh the session's specialist whenever the active session changes
    // (including on load and on "no session").
    create_effect(move |_| {
        let Some(sid) = active_session.get() else { session_specialist.set(None); return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
            let v = invoke("get_session_specialist", arg).await;
            if active_session.get_untracked().as_deref() == Some(sid.as_str()) {
                session_specialist.set(serde_wasm_bindgen::from_value::<Option<Specialist>>(v).ok().flatten());
            }
        });
    });
    let pick_specialist = move |id: String| {
        let Some(sid) = active_session.get() else { return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid, "id": id })).unwrap();
            if invoke_checked("set_session_specialist", arg).await.is_ok() {
                let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
                let v = invoke("get_session_specialist", arg).await;
                if active_session.get_untracked().as_deref() == Some(sid.as_str()) {
                    session_specialist.set(serde_wasm_bindgen::from_value::<Option<Specialist>>(v).ok().flatten());
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
    let right_w = create_rw_signal(440.0_f64);
    let dragging = create_rw_signal(false);
    let drag_start_x = create_rw_signal(0.0_f64);
    let drag_start_w = create_rw_signal(0.0_f64);
    let composer_h = create_rw_signal(load_composer_h());
    let composer_h_custom = create_rw_signal(composer_h_custom());
    let composer_dragging = create_rw_signal(false);
    let composer_drag_start_y = create_rw_signal(0.0_f64);
    let composer_drag_start_h = create_rw_signal(0.0_f64);

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
        let paths: Vec<String> = artifacts_all.get().iter()
            .filter_map(|a| match &a.data { PreviewData::File { path, .. } => Some(path.clone()), _ => None })
            .collect();
        if paths.is_empty() { missing_paths.set(std::collections::HashSet::new()); return; }
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
        artifacts_all.get().into_iter()
            .filter(|a| match &a.data { PreviewData::File { path, .. } => !miss.contains(path), _ => true })
            .collect::<Vec<_>>()
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
    let open_right_tabs = create_rw_signal(vec![RightTab::Artifacts, RightTab::Notebook]);
    let right_tab_add_menu_open = create_rw_signal(false);
    let file_query = create_rw_signal(String::new());
    let file_cwd = create_rw_signal(".".to_string());
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let file_search_hits = create_rw_signal::<Vec<FileSearchHit>>(vec![]);
    // Runtime-change events are notifications only. Preserve the turn snapshot
    // and let the user decide when to refresh; opening Settings must never
    // restart or continually reread the local Codex actor in the background.
    let runtime_changed_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(value) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) else { return; };
        let profile_id = value.get("profileId").or_else(|| value.get("profile_id"))
            .and_then(|value| value.as_str()).unwrap_or("");
        let project_id = value.get("projectId").or_else(|| value.get("project_id"))
            .and_then(|value| value.as_str()).unwrap_or("");
        let _pending = value.get("pending").and_then(|value| value.as_bool()).unwrap_or(false);
        let profiles = models.get_untracked();
        let profile_matches = profiles.iter()
            .find(|profile| profile.active)
            .or_else(|| profiles.first())
            .is_some_and(|profile| {
                active_profile_uses_codex(std::slice::from_ref(profile))
                    && profile.id == profile_id
            });
        let project_matches = project_info.get_untracked().as_ref().is_none_or(|project| {
            project.id.is_empty() || project_id.is_empty() || project.id == project_id
        });
        if !profile_matches || !project_matches { return; }
        codex_config_needs_confirmation.set(true);
        codex_runtime_refresh_pending.set(true);
        status.set(t(locale.get_untracked(), "codex.refresh_detected").into());
    }) as Box<dyn FnMut(JsValue)>);
    let runtime_changed_js = runtime_changed_cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(runtime_changed_cb);
    spawn_local(async move { let _ = listen("codex-runtime-changed", &runtime_changed_js).await; });
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
        let q = file_query.get();
        if q.trim().is_empty() {
            file_search_hits.set(vec![]);
            return;
        }
        refresh_file_search(file_query, file_search_hits);
    });

    let on_artifact_select = Callback::new(move |idx: usize| {
        let arts = artifacts.get();
        if let Some(a) = arts.get(idx) {
            if let PreviewData::File { path, kind } = &a.data {
                if opens_in_modal(kind) {
                    modal_artifact.set(Some((path.clone(), a.name.clone(), kind.clone())));
                    return;
                }
                open_workspace_file(path.clone(), modal_artifact);
            } else {
                ensure_right_tab(
                    RightTab::Artifacts,
                    show_right,
                    open_right_tabs,
                    right_tab,
                );
                sel_artifact.set(idx);
                show_art_preview.set(true);
            }
        }
    });

    let on_file_link = Callback::new(move |(path, _kind): (String, String)| {
        open_workspace_file(path, modal_artifact);
    });

    // Inline @ artifact, # session, and / skill pickers all share one cursor
    // model and one chip list. Uploads remain separate because they have async
    // progress/error state; selected catalog items are already durable records.
    let composer_references = create_rw_signal::<Vec<ComposerReferenceChip>>(vec![]);
    let picker_mode = create_rw_signal(None::<ComposerPickerMode>);
    let picker_query = create_rw_signal(String::new());
    let picker_index = create_rw_signal(0usize);
    let picker_artifacts = create_rw_signal(Vec::<ArtifactInfo>::new());
    let picker_sessions = create_rw_signal(Vec::<SessionSearchInfo>::new());
    create_effect(move |_| {
        let Some(mode) = picker_mode.get() else { return; };
        let query = picker_query.get();
        match mode {
            ComposerPickerMode::Artifact => spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "query": query, "limit": 40, "allProjects": true })).unwrap();
                let v = invoke("search_artifacts", arg).await;
                if picker_mode.get_untracked() == Some(mode) && picker_query.get_untracked() == query {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(v) { picker_artifacts.set(rows); }
                }
            }),
            ComposerPickerMode::Session => spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "query": query, "limit": 40 })).unwrap();
                let v = invoke("search_sessions", arg).await;
                if picker_mode.get_untracked() == Some(mode) && picker_query.get_untracked() == query {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(v) { picker_sessions.set(rows); }
                }
            }),
            ComposerPickerMode::Skill if skills_list.get_untracked().is_empty() => spawn_local(async move {
                let v = invoke("list_skills", JsValue::UNDEFINED).await;
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) { skills_list.set(rows); }
            }),
            ComposerPickerMode::Skill => {},
        }
    });
    let picker_items = create_memo(move |_| {
        let query = picker_query.get().to_lowercase();
        match picker_mode.get() {
            Some(ComposerPickerMode::Artifact) => {
                let current_session = active_session.get();
                let current_project = project_info.get().map(|p| p.id);
                let mut rows = picker_artifacts.get();
                rows.sort_by_key(|a| (
                    if a.session_id.as_deref() == current_session.as_deref() { 0 } else if a.project_id.as_deref() == current_project.as_deref() { 1 } else { 2 },
                    std::cmp::Reverse(a.ts),
                ));
                rows.into_iter().map(ComposerPickerItem::Artifact).collect()
            }
            Some(ComposerPickerMode::Session) => {
                let current_project = project_info.get().map(|p| p.id);
                let mut rows: Vec<_> = picker_sessions.get().into_iter()
                    .filter(|s| active_session.get().as_deref() != Some(s.id.as_str())).collect();
                rows.sort_by_key(|s| (current_project.as_deref() != Some(s.project_id.as_str()), std::cmp::Reverse(s.activity_at)));
                rows.into_iter().map(ComposerPickerItem::Session).collect()
            }
            Some(ComposerPickerMode::Skill) => {
                let mut rows: Vec<_> = skills_list.get().into_iter().filter(|s| s.enabled && (
                    s.name.to_lowercase().contains(&query) || s.description.to_lowercase().contains(&query) ||
                    s.tags.iter().any(|tag| tag.to_lowercase().contains(&query))
                )).collect();
                rows.sort_by_key(|s| (!s.builtin, s.name.clone()));
                rows.into_iter().map(ComposerPickerItem::Skill).collect()
            }
            None => vec![],
        }
    });
    let select_picker_item = Callback::new(move |i: usize| {
        let Some(item) = picker_items.get().get(i).cloned() else { return; };
        let reference = match item {
            ComposerPickerItem::Artifact(a) => ComposerReferenceChip::Artifact { id: a.id, name: a.name },
            ComposerPickerItem::Session(s) => ComposerReferenceChip::Session { id: s.id, title: s.title, project_name: s.project_name },
            ComposerPickerItem::Skill(s) => ComposerReferenceChip::Skill { name: s.name },
        };
        input.update(|s| {
            if let Some((at, _, _)) = active_composer_trigger(s) { s.truncate(at); }
        });
        composer_references.update(|items| {
            if !items.iter().any(|item| item.key() == reference.key()) { items.push(reference); }
        });
        picker_mode.set(None);
        focus_composer();
    });

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
            if s.show { show_onboarding.set(true); }
        }
        let b = invoke("get_bootstrap_status", JsValue::UNDEFINED).await;
        if let Ok(st) = serde_wasm_bindgen::from_value::<BootstrapStatus>(b) {
            bootstrap.set(Some(st));
        }
        refresh_models();
    });

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
    let status_cb = status;
    let locale_cb = locale;
    let models_cb = models;
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
        let flush_now = || flush_delta_buf(&cb_buf, active_cb, items_cb, transcripts_cb, models_cb);
        let queue = |fid: String, d: PendingDelta| {
            queue_delta(&cb_buf, fid, d);
            schedule_delta_flush(&cb_buf, &cb_scheduled, active_cb, items_cb, transcripts_cb, models_cb);
        };
        match ev {
            AgentEvent::User { frame_id, text } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let model = active_model_label(&models_cb.get());
                    start_user_turn(v, text, model);
                })
            }
            AgentEvent::Text { frame_id, delta } => queue(frame_id, PendingDelta::Text(delta)),
            AgentEvent::Reasoning { frame_id, delta } => queue(frame_id, PendingDelta::Reasoning(delta)),
            AgentEvent::ToolCall { frame_id, name, preview } => { flush_now(); route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                let idx = trailing_queue_start(v);
                v.insert(idx, ChatItem::Tool {
                    name,
                    ok: None,
                    input: preview,
                    output: String::new(),
                    started_at_ms: Some(now_ms()),
                    duration_ms: None,
                });
            }) }
            AgentEvent::ToolResult { frame_id, name, ok, content, duration_ms: event_ms } => { flush_now(); route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                let queue_start = trailing_queue_start(v);
                let idx = v[..queue_start].iter().rposition(|c| matches!(c, ChatItem::Tool { name: n, ok: None, .. } if n == &name));
                if let Some(i) = idx {
                    if let ChatItem::Tool { ok: o, output, started_at_ms, duration_ms, .. } = &mut v[i] {
                        *o = Some(ok);
                        *output = content.clone();
                        finalize_tool_duration(started_at_ms, duration_ms, event_ms);
                    }
                } else {
                    let dur = if event_ms > 0 { Some(event_ms) } else { None };
                    v.insert(queue_start, ChatItem::Tool {
                        name: name.clone(),
                        ok: Some(ok),
                        input: String::new(),
                        output: content.clone(),
                        started_at_ms: None,
                        duration_ms: dur,
                    });
                }
                if name == "attempt_completion" && ok {
                    promote_assistant_text(v, &content);
                }
            }) }
            AgentEvent::Usage { frame_id, input, output, ctx_tokens, max_context, .. } => {
                // Status bar reflects only the active session's usage.
                if active_cb.get().as_deref() == Some(&frame_id) {
                    let pct = if max_context > 0 { ctx_tokens * 100 / max_context } else { 0 };
                    let loc = locale_cb.get();
                    status_cb.set(tf(loc, "status.usage", &[
                        ("in", &format!("{:.1}", input as f64 / 1000.0)),
                        ("out", &format!("{:.1}", output as f64 / 1000.0)),
                        ("pct", &pct.to_string()),
                    ]));
                }
            }
            AgentEvent::Compaction { frame_id, before, after, .. } => {
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(tf(locale_cb.get(), "status.compact", &[
                        ("before", &before.to_string()),
                        ("after", &after.to_string()),
                    ]));
                }
            }
            AgentEvent::Stdout { frame_id, chunk } => queue(frame_id, PendingDelta::Stdout(chunk)),
            AgentEvent::Done { frame_id } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    strip_approval_pending(items);
                    items.retain(|item| !matches!(item, ChatItem::PlanQuestion(_)));
                });
                approval_cb.update(|s| { s.remove(&frame_id); });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                refresh_sessions(sessions);
                refresh_codex_turn_audits(frame_id.clone(), active_cb, codex_turn_audits);
            }
            AgentEvent::Error { frame_id, message } => {
                flush_now();
                let model = active_model_label(&models_cb.get());
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    strip_approval_pending(v);
                    v.retain(|item| !matches!(item, ChatItem::PlanQuestion(_)));
                    v.push(ChatItem::Assistant { text: format!("Error: {message}"), model });
                });
                approval_cb.update(|s| { s.remove(&frame_id); });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                refresh_codex_turn_audits(frame_id.clone(), active_cb, codex_turn_audits);
            }
            AgentEvent::ReviewStarted { frame_id } => {
                flush_now();
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.reviewing"));
                }
            }
            AgentEvent::CorrectionStarted { frame_id, model } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let index = trailing_queue_start(v);
                    v.insert(index, ChatItem::Assistant {
                        text: String::new(),
                        model: (!model.is_empty()).then_some(model),
                    });
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.correcting"));
                }
            }
            AgentEvent::Review { frame_id, report } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| upsert_review(v, report));
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.review_done"));
                }
            }
            AgentEvent::PlanDelta { frame_id, delta, native } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let queue_start = trailing_queue_start(items);
                    if let Some(ChatItem::Plan(plan)) = items[..queue_start].last_mut() {
                        if !plan.complete {
                            plan.text.push_str(&delta);
                            plan.native &= native;
                            return;
                        }
                    }
                    for item in &mut items[..queue_start] {
                        if let ChatItem::Plan(plan) = item { plan.actionable = false; }
                    }
                    items.insert(queue_start, ChatItem::Plan(PlanCard { text: delta, complete: false, native, actionable: false, plan_id: String::new(), revision: 0 }));
                });
            }
            AgentEvent::PlanUpdated { frame_id, plan, native } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let queue_start = trailing_queue_start(items);
                    if let Some(index) = items[..queue_start].iter().rposition(|item| matches!(item, ChatItem::Plan(_))) {
                        items[index] = ChatItem::Plan(PlanCard { text: plan.clone(), complete: false, native, actionable: false, plan_id: String::new(), revision: 0 });
                    } else {
                        items.insert(queue_start, ChatItem::Plan(PlanCard { text: plan.clone(), complete: false, native, actionable: false, plan_id: String::new(), revision: 0 }));
                    }
                });
            }
            AgentEvent::FinalPlan { frame_id, plan, native, plan_id, revision } => {
                flush_now();
                let actionable = !plan_id.is_empty() && revision > 0;
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let queue_start = trailing_queue_start(items);
                    if let Some(index) = items[..queue_start].iter().rposition(|item| matches!(item, ChatItem::Plan(_))) {
                        items[index] = ChatItem::Plan(PlanCard { text: plan.clone(), complete: true, native, actionable, plan_id: plan_id.clone(), revision });
                    } else {
                        items.insert(queue_start, ChatItem::Plan(PlanCard { text: plan.clone(), complete: true, native, actionable, plan_id: plan_id.clone(), revision }));
                    }
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "plan.ready"));
                }
            }
            AgentEvent::RequestUserInput { frame_id, question_id, question, options, is_other, is_secret } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let index = trailing_queue_start(items);
                    items.insert(index, ChatItem::PlanQuestion(PlanQuestion { question_id, question, options, is_other, is_secret }));
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "plan.needs_input"));
                }
            }
            AgentEvent::RequestUserInputResolved { frame_id, question_id } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    items.retain(|item| !matches!(item, ChatItem::PlanQuestion(question) if question.question_id == question_id));
                });
            }
            AgentEvent::ModelRerouted { frame_id, requested_model, effective_model } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    if let Some(ChatItem::Assistant { model, .. }) = items.iter_mut().rev().find(|item| matches!(item, ChatItem::Assistant { .. })) {
                        *model = Some(if requested_model.is_empty() {
                            effective_model.clone()
                        } else {
                            format!("{requested_model} → {effective_model}")
                        });
                    }
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    codex_preview.update(|preview| if let Some(config) = preview {
                        config.requested_model = requested_model.clone();
                        config.effective_model = effective_model.clone();
                        config.sources.insert("model".into(), "codex_server_reroute".into());
                    });
                    status_cb.set(tf(locale_cb.get(), "codex.rerouted", &[
                        ("requested", &requested_model), ("effective", &effective_model),
                    ]));
                }
            }
            AgentEvent::Diff { .. } => {}
        }
    }) as Box<dyn FnMut(JsValue)>);
    let agent_js = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(cb);
    // wasm-bindgen only runs an async extern's JS body when the returned
    // future is polled, so we must await `listen` (not fire-and-forget it).
    spawn_local(async move { let _ = listen("agent", &agent_js).await; });

    // Confirm handler: render an inline approval card in the session thread
    // (not a global modal — see README inline tool-approval card).
    let confirm_active = active_session;
    let confirm_items = items;
    let confirm_transcripts = transcripts;
    let confirm_pending = approval_pending;
    let confirm_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(v) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) {
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let fid = v.get("frame_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
            if msg.is_empty() || fid.is_empty() {
                return;
            }
            let mut tool = v.get("tool").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let mut preview = v.get("preview").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if tool.is_empty() {
                if let Some(rest) = msg.strip_prefix("Run tool '") {
                    if let Some((t, _)) = rest.split_once("'?") {
                        tool = t.to_string();
                    }
                } else if msg.starts_with("Dangerous command detected") {
                    tool = "shell".into();
                }
            }
            route_items(confirm_active, confirm_items, confirm_transcripts, &fid, |v| {
                strip_approval_pending(v);
                if preview.is_empty() {
                    preview = last_tool_input(v, &tool);
                }
                v.push(ChatItem::ApprovalPending {
                    tool,
                    preview,
                    message: msg,
                });
            });
            confirm_pending.update(|s| {
                s.insert(fid);
            });
            force_chat_bottom();
        }
    }) as Box<dyn FnMut(JsValue)>);
    let confirm_js = confirm_cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(confirm_cb);
    spawn_local(async move { let _ = listen("confirm-request", &confirm_js).await; });

    let stop = move |_| {
        if stopping_session.get().is_some() { return; }
        // Stop only the active session's turn; background conversations keep running.
        let sid = active_session.get();
        stopping_session.set(sid.clone());
        spawn_local(async move {
            let arg = to_value(&tauri_args::stop_agent(&sid)).unwrap();
            let _ = invoke("stop_agent", arg).await;
        });
    };

    let composer_runtime_request_nonce = codex_runtime_request_nonce.clone();
    let send = Callback::new(move |action: ComposerSendAction| {
        let text = input.get();
        let trimmed = text.trim();
        let using_codex = active_profile_uses_codex(&models.get());
        let using_codex_app_server = using_codex && codex_runtime.get().is_some();
        let using_local_runner = active_profile_uses_local_runner(&models.get());
        if action == ComposerSendAction::Normal && trimmed == "/default" {
            collaboration_mode.set("default".into());
            if using_local_runner {
                persist_session_codex_overrides(
                    active_session.get(), "default".into(),
                    using_codex.then(|| codex_runtime.get().map(|snapshot| snapshot.config_version)).flatten(),
                    if using_codex { codex_session_overrides.get() } else { CodexModeOverrides::default() },
                    codex_override_ui,
                );
            }
            picker_mode.set(None);
            input.set(String::new());
            status.set(t(locale.get(), "plan.default_enabled").into());
            return;
        }
        let mut turn_text = text.clone();
        if action == ComposerSendAction::Normal && (trimmed == "/plan" || trimmed.starts_with("/plan ")) {
            if collaboration_mode.get_untracked() != "plan" {
                collaboration_mode.set("plan".into());
            }
            picker_mode.set(None);
            turn_text = trimmed.strip_prefix("/plan").unwrap_or("").trim_start().to_string();
            if turn_text.trim().is_empty() {
                if using_local_runner {
                    persist_session_codex_overrides(
                        active_session.get(), "plan".into(),
                        using_codex.then(|| codex_runtime.get().map(|snapshot| snapshot.config_version)).flatten(),
                        if using_codex { codex_session_overrides.get() } else { CodexModeOverrides::default() },
                        codex_override_ui,
                    );
                }
                input.set(String::new());
                status.set(t(locale.get(), "plan.enabled").into());
                return;
            }
        } else if action == ComposerSendAction::PlanFirst {
            if collaboration_mode.get_untracked() != "plan" {
                collaboration_mode.set("plan".into());
            }
        }
        if using_codex && codex_runtime_loading.get() {
            status.set(t(locale.get(), "codex.runtime.wait").into());
            return;
        }
        if using_codex && codex_config_needs_confirmation.get() {
            codex_config_menu_open.set(true);
            let key = if codex_runtime_refresh_pending.get() {
                "codex.refresh_detected"
            } else {
                "codex.confirm_changed"
            };
            status.set(t(locale.get(), key).into());
            return;
        }
        let preview_matches_mode = codex_preview.get().is_some_and(|config| {
            (collaboration_mode.get() == "plan") == (config.mode == "plan")
        });
        if using_codex_app_server && (codex_preview_loading.get() || !preview_matches_mode) {
            status.set(t(locale.get(), "codex.preview.wait").into());
            return;
        }
        let turn_mode = collaboration_mode.get();
        let saved_attachments = attachments.get();
        let paths = attachment_paths(&saved_attachments);
        let refs = composer_references.get();
        let saved_references = refs.clone();
        let reference_args = refs.iter().map(ComposerReferenceChip::arg).collect::<Vec<_>>();
        let display_message = message_with_references(&turn_text, &paths, &refs);
        let mut message = display_message.clone();
        if turn_mode == "plan" && !using_local_runner {
            message = plan_first_message(&message);
        }
        if display_message.trim().is_empty() || uploading.get() { return; }
        let active = active_session.get();
        let branch = action == ComposerSendAction::BranchNew;
        let branch_items = items.get();
        if !branch && active.as_ref().is_some_and(|id| running.get().contains(id)) {
            items.update(|v| v.push(ChatItem::QueuedUser(display_message.clone())));
            force_chat_bottom();
        } else if !branch {
            let model = if using_codex && !using_codex_app_server {
                let profiles = models.get();
                let profile = profiles.iter().find(|profile| profile.active).or_else(|| profiles.first());
                profile.and_then(|profile| {
                    let layer = codex_session_overrides.get();
                    let mode_layer = layer.for_mode(&turn_mode);
                    let profile_model = if turn_mode == "plan" { &profile.plan_model } else { &profile.normal_model };
                    mode_layer.model.clone()
                        .or_else(|| optional_profile_value(profile_model))
                        .or_else(|| optional_profile_value(&profile.model))
                }).or_else(|| active_model_label(&profiles))
            } else {
                codex_preview.get().map(|config| config.effective_model().to_string())
                    .filter(|model| !model.is_empty()).or_else(|| active_model_label(&models.get()))
            };
            items.update(|v| {
                v.push(ChatItem::User(display_message.clone()));
                v.push(ChatItem::Assistant { text: String::new(), model });
            });
            force_chat_bottom();
        }
        needs_api_key.set(false);
        input.set(String::new());
        attachments.set(vec![]);
        composer_references.set(vec![]);
        let locale = locale;
        let status = status;
        let running = running;
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let sessions = sessions;
        let stopping_session = stopping_session;
        let pending_turns = pending_turns;
        let config_generation = using_codex.then(|| codex_runtime.get().map(|snapshot| snapshot.config_version)).flatten();
        let session_overrides = using_codex.then(|| codex_session_overrides.get());
        let send_collaboration_mode = using_local_runner.then(|| turn_mode.clone());
        let draft_for_retry = text.clone();
        let send_runtime_request_nonce = composer_runtime_request_nonce.clone();
        spawn_local(async move {
            let restore_failure = |target_session: Option<&str>| {
                if !branch {
                    rollback_optimistic_send(
                        target_session,
                        active_session,
                        items,
                        transcripts,
                        &display_message,
                    );
                }
                restore_failed_composer(
                    input,
                    attachments,
                    composer_references,
                    draft_for_retry.clone(),
                    saved_attachments.clone(),
                    saved_references.clone(),
                );
            };
            // Resolve once more immediately before the turn. If the local
            // runtime changed since the composer's snapshot, restore the draft
            // and require an explicit retry rather than sending stale values.
            if using_codex_app_server {
                let args = to_value(&serde_json::json!({
                    "sessionId": active.clone(),
                    "mode": turn_mode.clone(),
                    "overrides": session_overrides.clone(),
                    "configVersion": config_generation.clone(),
                    "previewScope": "session",
                    "validateRuntime": true,
                })).unwrap();
                let resolved = match invoke_checked("preview_codex_turn_config", args).await {
                    Ok(value) => match decode_resolved_turn_config(value) {
                        Some(resolved) => resolved,
                        None => {
                            restore_failure(active.as_deref());
                            status.set(tf(locale.get(), "status.send_failed", &[(
                                "msg", "Codex returned an unsupported configuration preview.",
                            )]));
                            return;
                        }
                    },
                    Err(error) => {
                        let raw = js_error_text(error);
                        let changed = is_codex_config_changed_error(&raw);
                        restore_failure(active.as_deref());
                        if changed {
                            codex_config_needs_confirmation.set(true);
                            codex_runtime_refresh_pending.set(true);
                            request_codex_runtime_snapshot_with_ack(
                                false, false, true, false, send_runtime_request_nonce.clone(), models, codex_expected_project_id,
                                codex_runtime, codex_runtime_error,
                                codex_runtime_loading, codex_profile_overrides,
                                Some(codex_runtime_refresh_pending),
                            );
                            codex_config_menu_open.set(true);
                            status.set(t(locale.get(), "codex.config_changed").into());
                        } else {
                            status.set(tf(locale.get(), "status.send_failed", &[(
                                "msg", &localize_backend(locale.get(), &raw),
                            )]));
                        }
                        return;
                    }
                };
                let stale = config_generation.as_ref().is_some_and(|generation| {
                    !generation.is_empty()
                        && !resolved.config_version.is_empty()
                        && generation != &resolved.config_version
                });
                let effective_model = resolved.effective_model().to_string();
                if !effective_model.is_empty() && !branch {
                    items.update(|rows| if let Some(ChatItem::Assistant { text, model }) = rows.last_mut() {
                        if text.is_empty() { *model = Some(effective_model.clone()); }
                    });
                }
                codex_preview.set(Some(resolved));
                if stale {
                    restore_failure(active.as_deref());
                    codex_config_needs_confirmation.set(true);
                    codex_runtime_refresh_pending.set(true);
                    request_codex_runtime_snapshot_with_ack(
                        false, false, true, false, send_runtime_request_nonce.clone(), models, codex_expected_project_id,
                        codex_runtime, codex_runtime_error,
                        codex_runtime_loading, codex_profile_overrides,
                        Some(codex_runtime_refresh_pending),
                    );
                    codex_config_menu_open.set(true);
                    status.set(t(locale.get(), "codex.config_changed").into());
                    return;
                }
            }
            // Resolve the target session: use the active one, or create a fresh
            // frame up front so streamed events can be routed before the first delta.
            let id = if branch {
                let arg = to_value(&tauri_args::branch_session(&active, Some(turn_text.trim()), None)).unwrap();
                match invoke("branch_session", arg).await.as_string() {
                    Some(s) => s,
                    None => {
                        restore_failure(active.as_deref());
                        let loc = locale.get();
                        status.set(t(loc, "status.send_failed").into());
                        return;
                    }
                }
            } else { match active.clone() {
                Some(id) => id,
                None => {
                    let v = invoke("new_session", JsValue::UNDEFINED).await;
                    match v.as_string() {
                        Some(s) => s,
                        None => {
                            // Bridge returned no id (e.g. legacy mock); bail without
                            // flipping running so the user can retry.
                            restore_failure(None);
                            let loc = locale.get();
                            status.set(t(loc, "status.send_failed").into());
                            return;
                        }
                    }
                }
            }};
            if branch {
                if let Some(old) = active.clone() {
                    transcripts.update(|m| { m.insert(old, branch_items.clone()); });
                }
                items.set(branch_items);
                force_chat_bottom();
            }
            if using_local_runner {
                if let Err(message) = persist_session_codex_overrides_await(
                    Some(id.clone()),
                    turn_mode.clone(),
                    config_generation.clone(),
                    session_overrides.clone().unwrap_or_default(),
                    codex_override_ui,
                ).await {
                    restore_failure(active.as_deref());
                    let lower = message.to_ascii_lowercase();
                    if is_codex_config_changed_error(&message) {
                        codex_config_needs_confirmation.set(true);
                        codex_runtime_refresh_pending.set(true);
                        request_codex_runtime_snapshot_with_ack(
                            false, false, true, false, send_runtime_request_nonce.clone(), models, codex_expected_project_id,
                            codex_runtime, codex_runtime_error,
                            codex_runtime_loading, codex_profile_overrides,
                            Some(codex_runtime_refresh_pending),
                        );
                        codex_config_menu_open.set(true);
                        status.set(t(locale.get(), "codex.config_changed").into());
                    } else if lower.contains("revision") || lower.contains("conflict") {
                        // The serialized override writer already reread the
                        // session layer/revision. Do not restart or refresh the
                        // Codex runtime for a session-only compare-and-swap.
                        codex_config_needs_confirmation.set(true);
                        codex_config_menu_open.set(true);
                        status.set(tf(locale.get(), "codex.override_failed", &[(
                            "msg", &localize_backend(locale.get(), &message),
                        )]));
                    } else {
                        status.set(tf(locale.get(), "status.send_failed", &[(
                            "msg", &localize_backend(locale.get(), &message),
                        )]));
                    }
                    return;
                }
            }
            active_session.set(Some(id.clone()));
            begin_pending_turn(pending_turns, running, &id);
            let arg = to_value(&SendMessageArgs {
                session_id: Some(id.clone()), message, attachments: paths,
                references: reference_args, resume: false,
                collaboration_mode: send_collaboration_mode,
                codex_config_generation: config_generation,
                codex_overrides: session_overrides,
            }).unwrap();
            match invoke_checked("send_message", arg).await {
                Ok(_) => {
                    // send_message is awaited for the whole turn, so it resolves only
                    // once the turn has finished AND been persisted. Clear `running`
                    // here rather than trusting the separate `Done` broadcast — a
                    // dropped broadcast used to pin the session on "运行中" until an
                    // app restart (#34).
                    finish_pending_turn(pending_turns, running, &id);
                    if stopping_session.get().as_deref() == Some(&id) {
                        stopping_session.set(None);
                    }
                    // If the live view desynced (a tool row left unresolved by a
                    // missed event), reconcile it from the authoritative DB so the
                    // completed result shows without a restart. Healthy turns keep
                    // their richer streamed view (incl. tool inputs) untouched.
                    let is_active = active_session.get().as_deref() == Some(&id);
                    let stranded = if is_active {
                        items.with(|v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. })))
                    } else {
                        transcripts.with(|m| m.get(&id).map_or(false, |v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. }))))
                    };
                    if stranded {
                        let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                            let chats: Vec<ChatItem> = list.into_iter().map(LoadedItem::into_chat).collect();
                            transcripts.update(|m| { m.insert(id.clone(), chats.clone()); });
                            if active_session.get().as_deref() == Some(&id) {
                                items.set(chats);
                                force_chat_bottom();
                            }
                        }
                    }
                    refresh_sessions(sessions);
                    refresh_codex_turn_audits(id.clone(), active_session, codex_turn_audits);
                }
                Err(err) => {
                    let loc = locale.get();
                    let raw = js_error_text(err);
                    let (turn_started, visible_error) = split_turn_started_error(&raw);
                    if !turn_started {
                        restore_failure(Some(&id));
                    } else if !branch {
                        route_items(active_session, items, transcripts, &id, |rows| {
                            mark_optimistic_send_failed(rows, &display_message, visible_error)
                        });
                    }
                    if turn_started {
                        refresh_codex_turn_audits(id.clone(), active_session, codex_turn_audits);
                    }
                    if visible_error.contains(NO_API_KEY_MARK) { needs_api_key.set(true); }
                    status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, visible_error))]));
                    finish_pending_turn(pending_turns, running, &id);
                    if stopping_session.get().as_deref() == Some(&id) {
                        stopping_session.set(None);
                    }
                }
            }
        });
    });

    let send_side_chat = move |question: String| {
        let question = question.trim().to_string();
        if question.is_empty() || side_chat_busy.get() {
            return;
        }
        ensure_right_tab(
            RightTab::SideChat,
            show_right,
            open_right_tabs,
            right_tab,
        );
        side_chat_input.set(String::new());
        side_chat_items.update(|v| v.push(ChatItem::User(question.clone())));
        side_chat_busy.set(true);
        let sid = active_session.get();
        let model = active_model_label(&models.get());
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "sessionId": sid,
                "question": question,
            }))
            .unwrap();
            match invoke_checked("side_chat", arg).await {
                Ok(v) => {
                    let text = v.as_string().unwrap_or_default();
                    side_chat_items.update(|items| {
                        items.push(ChatItem::Assistant { text, model: model.clone() });
                    });
                }
                Err(err) => {
                    side_chat_items.update(|items| {
                        items.push(ChatItem::Assistant {
                            text: format!("Error: {}", localize_backend(locale.get(), &js_error_text(err))),
                            model: model.clone(),
                        });
                    });
                }
            }
            side_chat_busy.set(false);
        });
    };

    let on_send = move |ev: web_sys::KeyboardEvent| {
        // While an IME is composing (e.g. Chinese pinyin), Enter confirms the
        // candidate — its keydown reports isComposing — so let the IME handle
        // every key and never send/navigate mid-composition (#108).
        if ev.is_composing() {
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
                "Enter" | "Tab" => { ev.prevent_default(); select_picker_item.call(picker_index.get()); }
                "Escape" => { ev.prevent_default(); picker_mode.set(None); }
                _ => {}
            }
            return;
        }
        if ev.key() == "Enter" && !ev.shift_key() { ev.prevent_default(); send.call(ComposerSendAction::Normal); }
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
        items.set(list.into_iter().take(ui_index).collect());
        input.set(draft);
        focus_composer();
        let sid = active_session.get();
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
        let sessions = sessions;
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
            let prefix_items = list.iter().take(ui_index).cloned().collect::<Vec<_>>();
            let draft = composer_text_from_user_message(text);
            attachments.set(vec![]);
            composer_references.set(vec![]);
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
                if let Some(old) = sid {
                    transcripts.update(|m| {
                        m.insert(old, list.clone());
                        m.insert(id.clone(), prefix_items.clone());
                    });
                }
                items.set(prefix_items);
                input.set(draft);
                active_session.set(Some(id));
                refresh_sessions(sessions);
                focus_composer();
            });
        }
    };

    let resume_turn = {
        let locale = locale;
        let status = status;
        let running = running;
        let busy = busy;
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let sessions = sessions;
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
            let use_codex = active_profile_uses_codex(&models.get());
            let use_local_runner = active_profile_uses_local_runner(&models.get());
            let resume_mode = use_local_runner.then(|| collaboration_mode.get());
            let resume_generation = use_codex.then(|| codex_runtime.get().map(|snapshot| snapshot.config_version)).flatten();
            let resume_overrides = use_codex.then(|| codex_session_overrides.get());
            let model = active_model_label(&models.get());
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
                    collaboration_mode: resume_mode,
                    codex_config_generation: resume_generation,
                    codex_overrides: resume_overrides,
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
                            items.with(|v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. })))
                        } else {
                            transcripts.with(|m| {
                                m.get(&id)
                                    .map_or(false, |v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. })))
                            })
                        };
                        if stranded {
                            let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                                let chats: Vec<ChatItem> = list.into_iter().map(LoadedItem::into_chat).collect();
                                transcripts.update(|m| { m.insert(id.clone(), chats.clone()); });
                                if active_session.get().as_deref() == Some(&id) {
                                    items.set(chats);
                                    force_chat_bottom();
                                }
                            }
                        }
                        refresh_sessions(sessions);
                    }
                    Err(err) => {
                        let loc = locale.get();
                        let raw = js_error_text(err);
                        if raw.contains(NO_API_KEY_MARK) {
                            needs_api_key.set(true);
                        }
                        status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &raw))]));
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
        let Some(window) = web_sys::window() else { return; };
        let Some(doc) = window.document() else { return; };
        let Some(el) = doc.get_element_by_id("composer-file-input") else { return; };
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

    let check_updates = move |_| {
        if settings_busy.get() { return; }
        settings_busy.set(true);
        settings_message.set(Some((true, t(locale.get(), "status.checking_updates").into())));
        let msg = settings_message;
        let busy = settings_busy;
        let loc = locale;
        spawn_local(async move {
            match invoke_checked("check_for_updates", JsValue::UNDEFINED).await {
                Ok(v) => {
                    let text = v.as_string().unwrap_or_else(|| t(loc.get(), "status.update_check_complete").into());
                    msg.set(Some((true, localize_backend(loc.get(), &text))));
                }
                Err(err) => msg.set(Some((false, localize_backend(loc.get(), &js_error_text(err))))),
            }
            busy.set(false);
        });
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
                    skills_msg.set(Some((false, localize_backend(locale.get(), &js_error_text(err)))));
                }
            }
        });
    };

    let refresh_conns = move || {
        spawn_local(async move {
            let v = invoke("list_mcp_connections", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnView>(v) { conns_view.set(Some(view)); }
            let c = invoke("list_connectors", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnectorsView>(c) { connectors.set(Some(view)); }
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
        custom_conn_tools_loading.update(|s| { s.insert(id.clone()); });
        custom_conn_tool_errors.update(|m| { m.remove(&id); });
        spawn_local(async move {
            let conn = build_conn_json(&conn_form_from_row(&row), false);
            let out = invoke_checked("test_mcp_connection", to_value(&serde_json::json!({ "conn": conn })).unwrap()).await;
            match out.and_then(|v| serde_wasm_bindgen::from_value::<Vec<ConnectorTool>>(v).map_err(|e| JsValue::from_str(&e.to_string()))) {
                Ok(tools) => custom_conn_tools.update(|m| { m.insert(id.clone(), tools); }),
                Err(err) => custom_conn_tool_errors.update(|m| { m.insert(id.clone(), js_error_text(err)); }),
            }
            custom_conn_tools_loading.update(|s| { s.remove(&id); });
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
        specialist_form.set(None);
        conn_form.set(None);
        open_conn_key.set(None);
        conn_test_msg.set(None);
        memory_selected.set(None);
        memory_editor.set(String::new());
        memory_msg.set(None);
        skills_msg.set(None);
    };

    let go_settings_section = move |sec: &str| {
        close_settings_subpage();
        settings_section.set(sec.into());
        match sec {
            "models" => refresh_models(),
            "specialists" => refresh_specialists(),
            "memory" => refresh_memory(),
            "skills" => refresh_skills(),
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
        refresh_conns();
        refresh_models();
        refresh_specialists();
        refresh_memory();
        refresh_credentials();
        refresh_approval_grants();
        spawn_local(async move {
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                let cfg = normalized_settings(cfg);
                let l = Locale::from_code(&cfg.locale);
                loc.set(l);
                set_document_lang(l);
                s.set(cfg);
            } else {
                msg.set(Some((false, t(loc.get(), "status.failed_load_settings").into())));
            }
        });
    };
    let open_settings = move |_| open_settings_fn(None);

    let save_settings = move |_| {
        if settings_busy.get() { return; }
        let mut cfg = normalized_settings(settings.get());
        cfg.locale = locale.get().code().into();
        let s = settings;
        let show = show_settings;
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        let loc = locale;
        busy.set(true);
        let saving = t(loc.get(), "status.saving_settings").to_string();
        msg.set(Some((true, saving.clone())));
        status_msg.set(saving);
        spawn_local(async move {
            let settings_result = invoke_checked(
                "set_settings",
                to_value(&serde_json::json!({ "settings": cfg.clone() })).unwrap(),
            ).await;
            if let Err(err) = settings_result {
                let l = loc.get();
                let text = tf(l, "status.save_failed", &[("msg", &localize_backend(l, &js_error_text(err)))]);
                msg.set(Some((false, text.clone())));
                status_msg.set(text);
                busy.set(false);
                return;
            }
            busy.set(false);
            show.set(false);
            status_msg.set(t(loc.get(), "status.settings_saved").into());
            s.set(cfg);
        });
    };

    let save_model_form = move |_| {
        if settings_busy.get() { return; }
        let Some(form) = model_form.get() else { return; };
        let loc = locale.get();
        let key = model_form_key.get();
        let has_key = form.id.as_ref()
            .and_then(|id| models.get().iter().find(|m| &m.id == id).map(|m| m.has_api_key))
            .unwrap_or(false);
        let cfg = model_form_to_settings(&form, has_key && key.is_empty());
        let local_runner = matches!(form.provider.as_str(), "codex" | "codex_cli" | "codex_local" | "codex-local" | "claude_code" | "claude-code");
        if !local_runner {
            if let Some(err_key) = settings_required_error_key(&cfg, &key) {
                let err = t(loc, err_key);
                let text = tf(loc, "status.save_failed", &[("msg", &err)]);
                model_form_msg.set(Some((false, text)));
                return;
            }
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        let provider = match form.provider.as_str() {
            "codex" | "codex_cli" | "codex_local" | "codex-local" => "codex_cli",
            "claude_code" | "claude-code" | "claude" => "claude_code",
            other => provider_value(other),
        };
        let profile = serde_json::json!({
            "id": form.id.clone().unwrap_or_default(),
            "label": form.label.trim(),
            "provider": provider,
            "api_url": form.api_url.trim(),
            "model": form.model.trim(),
            "max_tokens": form.max_tokens,
            "reasoning_effort": form.reasoning_effort.trim(),
            "supports_vision": form.supports_vision,
            "use_for_vision": form.use_for_vision,
            "runner_command": form.runner_command.trim(),
            "runner_profile": form.runner_profile.trim(),
            "runner_sandbox": form.runner_sandbox.trim(),
            "runner_web_search_mode": form.runner_web_search_mode.trim(),
            "runner_claude_command": form.runner_claude_command.trim(),
            "runner_persistent": form.runner_persistent,
            "normal_model": form.normal_model.trim(),
            "normal_reasoning_effort": form.normal_reasoning_effort.trim(),
            "plan_model": form.plan_model.trim(),
            "plan_reasoning_effort": form.plan_reasoning_effort.trim(),
            "service_tier": form.service_tier.trim(),
            "personality": form.personality.trim(),
            "reasoning_summary": form.reasoning_summary.trim(),
            "verbosity": form.verbosity.trim(),
        });
        let key_arg = if key.is_empty() { None } else { Some(key) };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "profile": profile,
                "key": key_arg,
                "useForVision": form.use_for_vision,
            })).unwrap();
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

    let explicit_runtime_request_nonce = codex_runtime_request_nonce.clone();
    let refresh_codex_runtime = Callback::new(move |_: ()| {
        if show_projects.get_untracked() || project_info.get_untracked().is_none() { return; }
        request_codex_runtime_snapshot_with_ack(
            true, true, true, true, explicit_runtime_request_nonce.clone(), models, codex_expected_project_id,
            codex_runtime, codex_runtime_error,
            codex_runtime_loading, codex_profile_overrides,
            Some(codex_runtime_refresh_pending),
        );
    });

    let settings_preview_nonce = codex_settings_preview_nonce.clone();
    let preview_codex_configs = Callback::new(move |_: ()| {
        let overrides = codex_profile_overrides.get();
        let generation = codex_runtime.get().map(|snapshot| snapshot.config_version);
        let request_id = settings_preview_nonce.get().wrapping_add(1);
        settings_preview_nonce.set(request_id);
        let response_nonce = settings_preview_nonce.clone();
        codex_preview_normal.set(None);
        codex_preview_plan.set(None);
        codex_settings_action_loading.set(true);
        spawn_local(async move {
            for (mode, output) in [("default", codex_preview_normal), ("plan", codex_preview_plan)] {
                let args = to_value(&serde_json::json!({
                    "sessionId": null,
                    "mode": mode,
                    "overrides": overrides,
                    "configVersion": generation,
                    "previewScope": "profile",
                })).unwrap();
                match invoke_checked("preview_codex_turn_config", args).await {
                    Ok(value) => {
                        if response_nonce.get() != request_id { return; }
                        if let Some(config) = decode_resolved_turn_config(value) { output.set(Some(config)); }
                    }
                    Err(error) if response_nonce.get() == request_id => {
                        codex_runtime_error.set(Some(js_error_text(error)))
                    }
                    Err(_) => return,
                }
            }
            if response_nonce.get() == request_id { codex_settings_action_loading.set(false); }
        });
    });

    create_effect(move |_| {
        let visible = show_settings.get() && settings_section.get() == "models";
        let has_runtime = codex_runtime.get().is_some();
        let _profile_layer = codex_profile_overrides.get();
        if visible && has_runtime && active_profile_uses_codex(&models.get()) {
            preview_codex_configs.call(());
        }
    });

    let save_codex_profile = Callback::new(move |_: ()| {
        let Some(profile) = models.get().into_iter().find(|model| model.active)
            .or_else(|| models.get().into_iter().next())
            .filter(|profile| matches!(profile.provider.as_str(), "codex_cli" | "codex" | "codex_local" | "codex-local"))
        else {
            codex_runtime_error.set(Some(t(locale.get(), "codex.no_profile").into()));
            return;
        };
        let overrides = codex_profile_overrides.get();
        codex_settings_action_loading.set(true);
        let profile_json = serde_json::json!({
            "id": profile.id,
            "label": profile.label,
            "provider": "codex_cli",
            "api_url": profile.api_url,
            "model": profile.model,
            "max_tokens": profile.max_tokens,
            "reasoning_effort": profile.reasoning_effort,
            "supports_vision": profile.supports_vision,
            "use_for_vision": profile.use_for_vision,
            "runner_command": profile.runner_command,
            "runner_profile": profile.runner_profile,
            "runner_sandbox": overrides.sandbox.clone().unwrap_or_default(),
            "runner_web_search_mode": overrides.web_search.clone().unwrap_or_default(),
            "runner_claude_command": profile.runner_claude_command,
            "runner_persistent": profile.runner_persistent,
            "normal_model": overrides.normal.model.clone().unwrap_or_default(),
            "normal_reasoning_effort": overrides.normal.effort.clone().unwrap_or_default(),
            "plan_model": overrides.plan.model.clone().unwrap_or_default(),
            "plan_reasoning_effort": overrides.plan.effort.clone().unwrap_or_default(),
            "service_tier": overrides.service_tier.clone().unwrap_or_default(),
            "personality": overrides.personality.clone().unwrap_or_default(),
            "reasoning_summary": overrides.summary.clone().unwrap_or_default(),
            "verbosity": overrides.verbosity.clone().unwrap_or_default(),
        });
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "profile": profile_json,
                "key": null,
                "useForVision": profile.use_for_vision,
            })).unwrap();
            match invoke_checked("save_model", args).await {
                Ok(value) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(value) { models.set(list); }
                    codex_runtime_error.set(None);
                    status.set(t(locale.get(), "status.settings_saved").into());
                    codex_settings_action_loading.set(false);
                    preview_codex_configs.call(());
                }
                Err(error) => {
                    codex_runtime_error.set(Some(localize_backend(locale.get(), &js_error_text(error))));
                    codex_settings_action_loading.set(false);
                }
            }
        });
    });

    let save_specialist_form = move |_| {
        let Some(spec) = specialist_form.get() else { return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "spec": spec })).unwrap();
            match invoke_checked("save_specialist_cmd", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                        specialists.set(list);
                    }
                    specialist_form.set(None);
                }
                Err(err) => {
                    // Same surface the model form uses for its failures.
                    model_form_msg.set(Some((false, localize_backend(locale.get_untracked(), &js_error_text(err)))));
                }
            }
        });
    };

    let remove_specialist_fn = move |id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            if let Ok(v) = invoke_checked("remove_specialist", arg).await {
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                    specialists.set(list);
                }
            }
        });
    };

    let validate_model_form = move |_| {
        if settings_busy.get() { return; }
        let Some(form) = model_form.get() else { return; };
        let loc = locale.get();
        let key = model_form_key.get();
        let local_provider = match form.provider.as_str() {
            "codex_cli" | "codex" | "codex_local" | "codex-local" => Some("codex_cli"),
            "claude_code" | "claude-code" | "claude" => Some("claude_code"),
            _ => None,
        };
        let has_key = models.get().iter().find(|m| Some(m.id.as_str()) == form.id.as_deref()).map(|m| m.has_api_key).unwrap_or(false);
        let mut cfg = model_form_to_settings(&form, has_key);
        if let Some(provider) = local_provider { cfg.provider = provider.into(); }
        if local_provider.is_none() {
            if let Some(err_key) = settings_required_error_key(&cfg, &key) {
                let err = t(loc, err_key);
                model_form_msg.set(Some((false, tf(loc, "status.validation_failed", &[("msg", &err)]))));
                return;
            }
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.validating").into())));
        spawn_local(async move {
            let res = invoke_timeout(
                "validate_settings",
                to_value(&serde_json::json!({ "settings": cfg, "key": key })).unwrap(),
                35_000,
            ).await;
            match res {
                Ok(v) => {
                    let raw = v.as_string().unwrap_or_else(|| t(loc, "status.validation_succeeded").into());
                    model_form_msg.set(Some((true, localize_backend(loc, &raw))));
                }
                Err(err) => {
                    model_form_msg.set(Some((false, tf(loc, "status.validation_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]))));
                }
            }
            settings_busy.set(false);
        });
    };

    let new_session = move |_| {
        demo_mode.set(false); // starting a fresh chat leaves the demo view
        // Stash the current transcript under its id so a running turn keeps
        // streaming into the cache, then create a fresh frame and show it.
        // We do NOT cancel any running turn — parallel conversations keep going.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
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
            refresh_sessions(sessions);
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
        let sessions = sessions;
        let models = models;
        move |_| {
            if busy.get() { return; }
            show_capabilities.set(false);
            attachments.set(vec![]);
            sel_artifact.set(0);
            right_tab.set(RightTab::Artifacts);
            let text: String = t(locale.get(), "caps.env_setup_prompt").into();
            let turn_model = active_model_label(&models.get());
            items.set(vec![
                ChatItem::User(text.clone()),
                ChatItem::Assistant { text: String::new(), model: turn_model },
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
                running.update(|r| { r.insert(id.clone()); });
                refresh_sessions(sessions);
                let arg = to_value(&SendMessageArgs {
                    session_id: Some(id.clone()), message: text, attachments: vec![], references: vec![], resume: false,
                    collaboration_mode: None, codex_config_generation: None, codex_overrides: None,
                }).unwrap();
                match invoke_checked("send_message", arg).await {
                    // The awaited command resolving is the reliable turn-complete
                    // signal; clear `running` here so a dropped `Done` broadcast
                    // can't pin the session on "运行中" (#34).
                    Ok(_) => { running.update(|r| { r.remove(&id); }); refresh_sessions(sessions); }
                    Err(err) => {
                        let loc = locale.get();
                        let raw = js_error_text(err);
                        if raw.contains(NO_API_KEY_MARK) { needs_api_key.set(true); }
                        status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &raw))]));
                        running.update(|r| { r.clear(); });
                    }
                }
            });
        }
    };

    let load_session = Callback::new(move |id: String| {
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        // Stash the transcript we're leaving under its id.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
        }
        let is_running = running.get().contains(&id);
        active_session.set(Some(id.clone()));
        if is_running {
            // Mid-stream: render the cached transcript immediately, but still
            // reconcile the separately persisted Plan claim/status. This keeps
            // session switching and restart semantics identical.
            items.set(transcripts.with(|m| m.get(&id).cloned().unwrap_or_default()));
            force_chat_bottom();
            let running_id = id.clone();
            spawn_local(async move {
                let args = to_value(&serde_json::json!({ "sessionId": running_id.clone() })).unwrap();
                if let Ok(value) = invoke_checked("get_latest_proposed_plan", args).await {
                    if let Some(plan) = decode_proposed_plan(value).filter(|plan| !plan.text.trim().is_empty()) {
                        route_items(active_session, items, transcripts, &running_id, |rows| {
                            upsert_persisted_plan(rows, plan)
                        });
                    }
                }
            });
            return;
        }
        // Idle session: load from DB and overwrite any stale cache entry.
        spawn_local(async move {
            let v = invoke("load_session", to_value(&serde_json::json!({ "id": id.clone() })).unwrap()).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                let mut chats: Vec<ChatItem> = list.into_iter().map(LoadedItem::into_chat).collect();
                // Proposed plans are persisted separately from ordinary chat
                // rows. Older backends simply reject this optional command.
                let plan_arg = to_value(&serde_json::json!({ "sessionId": id.clone() })).unwrap();
                if let Ok(value) = invoke_checked("get_latest_proposed_plan", plan_arg).await {
                    if let Some(plan) = decode_proposed_plan(value).filter(|plan| !plan.text.trim().is_empty()) {
                        upsert_persisted_plan(&mut chats, plan);
                    }
                }
                transcripts.update(|m| { m.insert(id.clone(), chats.clone()); });
                // Only repaint the view if we're still on this session — a rapid
                // switch could have moved on while the load was in flight, and an
                // unguarded set would clobber the newer view with stale rows (#53).
                if active_session.get().as_deref() == Some(&id) {
                    items.set(chats);
                    force_chat_bottom();
                }
            }
        });
    });

    let load_demo = move |info: DemoInfo| {
        let id = info.id.clone();
        let items = items;
        // Demos are read-only transcripts; they don't stream, so we don't touch
        // `running`. We do stash the current chat so returning to it is possible.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        active_session.set(None);
        spawn_local(async move {
            // Fresh session so the demo doesn't mix into a real conversation.
            let _ = invoke("new_session", JsValue::UNDEFINED).await;
            let v = invoke("load_demo", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
            if let Ok(demo) = serde_wasm_bindgen::from_value::<Demo>(v) {
                let mut view = vec![ChatItem::User(demo.request.clone())];
                if let Some(t) = &demo.thinking {
                    if !t.is_empty() { view.push(ChatItem::Reasoning(t.clone())); }
                }
                view.push(ChatItem::Assistant { text: demo.response.clone(), model: None });
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
        Callback::new(move |(sid, approved, feedback, scope): (String, bool, Option<String>, String)| {
            route_items(active_session, items, transcripts, &sid, strip_approval_pending);
            approval_pending.update(|s| {
                s.remove(&sid);
            });
            let arg =
                to_value(&tauri_args::confirm_response(&sid, approved, feedback.as_deref(), Some(&scope))).unwrap();
            spawn_local(async move { let _ = invoke("confirm_response", arg).await; });
        })
    };

    let plan_runtime_request_nonce = codex_runtime_request_nonce.clone();
    let on_plan_action = Callback::new(move |(action, plan_id, revision, plan_native): (String, String, i64, bool)| {
        if action == "modify" {
            collaboration_mode.set("plan".into());
            if active_profile_uses_local_runner(&models.get()) {
                let use_codex = active_profile_uses_codex(&models.get());
                persist_session_codex_overrides(
                    active_session.get(), "plan".into(),
                    use_codex.then(|| codex_runtime.get().map(|snapshot| snapshot.config_version)).flatten(),
                    if use_codex { codex_session_overrides.get() } else { CodexModeOverrides::default() },
                    codex_override_ui,
                );
            }
            input.set(String::new());
            focus_composer();
            status.set(t(locale.get(), "plan.modify_hint").into());
            return;
        }
        // An event from an older backend has no durable claim token. Rendering
        // should already hide its buttons, but keep the command boundary safe.
        if plan_id.is_empty() || revision <= 0 { return; }
        let Some(session_id) = active_session.get() else { return; };
        let approving = action == "approve";
        if plan_native && active_profile_uses_codex(&models.get()) && codex_runtime_loading.get() {
            status.set(t(locale.get(), "codex.runtime.wait").into());
            return;
        }
        if approving && plan_native && active_profile_uses_codex(&models.get())
            && codex_config_needs_confirmation.get()
        {
            codex_config_menu_open.set(true);
            let key = if codex_runtime_refresh_pending.get() {
                "codex.refresh_detected"
            } else {
                "codex.confirm_changed"
            };
            status.set(t(locale.get(), key).into());
            return;
        }
        if approving {
            begin_pending_turn(pending_turns, running, &session_id);
            status.set(t(locale.get(), "plan.executing").into());
        }
        let action_for_request = action.clone();
        // Compatibility proposals may have been produced because App Server
        // probing failed. Do not make their approval depend on a fresh native
        // preview; the backend validates their persisted config fingerprint.
        let action_uses_codex = plan_native && active_profile_uses_codex(&models.get());
        let requested_generation = codex_runtime.get().map(|snapshot| snapshot.config_version);
        let requested_overrides = codex_session_overrides.get();
        let action_runtime_request_nonce = plan_runtime_request_nonce.clone();
        spawn_local(async move {
            // Approval is an implementation turn in Default mode. Resolve that
            // exact config now and pass the same generation/override snapshot to
            // the backend action; never reuse the Plan-mode preview.
            let action_config_version = if approving && action_uses_codex {
                let preview_args = to_value(&serde_json::json!({
                    "sessionId": session_id.clone(),
                    "mode": "default",
                    "overrides": requested_overrides.clone(),
                    "configVersion": requested_generation.clone(),
                    "previewScope": "session",
                    "validateRuntime": true,
                })).unwrap();
                match invoke_checked("preview_codex_turn_config", preview_args).await {
                    Ok(value) => match decode_resolved_turn_config(value) {
                        Some(resolved) => {
                            let version = if resolved.config_version.is_empty() {
                                requested_generation.clone().unwrap_or_default()
                            } else {
                                resolved.config_version
                            };
                            if version.is_empty() {
                                status.set(tf(locale.get(), "status.send_failed", &[(
                                    "msg", "Codex did not return a configuration version for approval.",
                                )]));
                                finish_pending_turn(pending_turns, running, &session_id);
                                return;
                            }
                            let stale = requested_generation.as_ref().is_some_and(|generation| {
                                !generation.is_empty() && generation != &version
                            });
                            if stale {
                                finish_pending_turn(pending_turns, running, &session_id);
                                codex_config_needs_confirmation.set(true);
                                codex_runtime_refresh_pending.set(true);
                                request_codex_runtime_snapshot_with_ack(
                                    false, false, true, false, action_runtime_request_nonce.clone(), models, codex_expected_project_id,
                                    codex_runtime, codex_runtime_error, codex_runtime_loading,
                                    codex_profile_overrides, Some(codex_runtime_refresh_pending),
                                );
                                codex_config_menu_open.set(true);
                                status.set(t(locale.get(), "codex.config_changed").into());
                                return;
                            }
                            Some(version)
                        }
                        None => {
                            status.set(tf(locale.get(), "status.send_failed", &[(
                                "msg", "Codex returned an unsupported Default configuration preview.",
                            )]));
                            finish_pending_turn(pending_turns, running, &session_id);
                            return;
                        }
                    },
                    Err(error) => {
                        let raw = js_error_text(error);
                        finish_pending_turn(pending_turns, running, &session_id);
                        if is_codex_config_changed_error(&raw) {
                            codex_config_needs_confirmation.set(true);
                            codex_runtime_refresh_pending.set(true);
                            request_codex_runtime_snapshot_with_ack(
                                false, false, true, false, action_runtime_request_nonce.clone(), models, codex_expected_project_id,
                                codex_runtime, codex_runtime_error, codex_runtime_loading,
                                codex_profile_overrides, Some(codex_runtime_refresh_pending),
                            );
                            codex_config_menu_open.set(true);
                            status.set(t(locale.get(), "codex.config_changed").into());
                        } else {
                            status.set(tf(locale.get(), "status.send_failed", &[(
                                "msg", &localize_backend(locale.get(), &raw),
                            )]));
                        }
                        return;
                    }
                }
            } else {
                None
            };
            let args = to_value(&serde_json::json!({
                "sessionId": session_id.clone(),
                "action": action_for_request,
                "planId": plan_id.clone(),
                "revision": revision,
                "configVersion": action_config_version,
                "overrides": (approving && action_uses_codex).then_some(requested_overrides.clone()),
            })).unwrap();
            match invoke_checked("codex_plan_action", args).await {
                Ok(_) => {
                    if matches!(action.as_str(), "approve" | "save_exit") {
                        collaboration_mode.set("default".into());
                    }
                    status.set(t(locale.get(), if action == "approve" { "plan.executing" } else { "plan.saved" }).into());
                    route_items(active_session, items, transcripts, &session_id, |rows| {
                        if let Some(card) = rows.iter_mut().filter_map(|row| match row {
                            ChatItem::Plan(card) => Some(card),
                            _ => None,
                        }).find(|card| card.plan_id == plan_id && card.revision == revision) {
                            card.actionable = false;
                        }
                    });
                    let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
                    if let Ok(value) = invoke_checked("get_latest_proposed_plan", args).await {
                        if let Some(plan) = decode_proposed_plan(value)
                            .filter(|plan| plan.id == plan_id && plan.revision == revision)
                        {
                            route_items(active_session, items, transcripts, &session_id, |rows| {
                                if let Some(index) = rows.iter().rposition(|row| matches!(
                                    row,
                                    ChatItem::Plan(card) if card.plan_id == plan_id && card.revision == revision
                                )) {
                                rows[index] = ChatItem::Plan(PlanCard {
                                    text: plan.text,
                                    complete: !matches!(plan.status.as_str(), "streaming" | "draft"),
                                    native: plan.native,
                                    actionable: plan.status == "pending" && !plan.id.is_empty(),
                                    plan_id: plan.id,
                                    revision: plan.revision,
                                });
                                }
                            });
                        }
                    }
                    refresh_codex_turn_audits(
                        session_id.clone(), active_session, codex_turn_audits,
                    );
                }
                Err(error) => status.set(tf(locale.get(), "status.send_failed", &[
                    ("msg", &localize_backend(locale.get(), &js_error_text(error))),
                ])),
            }
            if approving {
                finish_pending_turn(pending_turns, running, &session_id);
            }
        });
    });

    let on_plan_answer = Callback::new(move |(question_id, answer): (String, String)| {
        let answer = answer.trim().to_string();
        if answer.is_empty() { return; }
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "questionId": question_id,
                "answers": [answer],
            })).unwrap();
            match invoke_checked("answer_codex_user_input", args).await {
                Ok(_) => {
                    items.update(|rows| rows.retain(|row| !matches!(row, ChatItem::PlanQuestion(question) if question.question_id == question_id)));
                    status.set(t(locale.get(), "plan.answer_sent").into());
                }
                Err(error) => status.set(tf(locale.get(), "status.send_failed", &[
                    ("msg", &localize_backend(locale.get(), &js_error_text(error))),
                ])),
            }
        });
    });

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
            right_w.set((drag_start_w.get() + dx).clamp(320.0, 900.0));
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
            composer_h.set((composer_drag_start_h.get() + dy).clamp(COMPOSER_H_MIN, COMPOSER_H_MAX));
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

    let open_files = move |_| {
        ensure_right_tab(
            RightTab::File,
            show_right,
            open_right_tabs,
            right_tab,
        );
        refresh_dir(file_cwd, file_entries);
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
            refresh_sessions(sessions);
            let arg = to_value(&SendMessageArgs {
                session_id: Some(id.clone()),
                message: prompt,
                attachments: vec![],
                references: vec![],
                resume: false,
                collaboration_mode: None,
                codex_config_generation: None,
                codex_overrides: None,
            })
            .unwrap();
            begin_pending_turn(pending_turns, running, &id);
            match invoke_checked("send_message", arg).await {
                Ok(_) => refresh_sessions(sessions),
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
            let _ = invoke_checked("set_skill_tags", to_value(&serde_json::json!({ "name": name, "tags": tags })).unwrap()).await;
            refresh_skills();
        });
    });

    let set_visible_skills_enabled = Callback::new(move |enabled: bool| {
        let tag = skill_filter_tag.get();
        let query = skills_search.get();
        let names = skills_list.get().into_iter()
            .filter(|s| skill_matches_filter(s, &tag, &query))
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
            let _ = invoke_checked("set_skills_enabled", to_value(&serde_json::json!({ "names": names, "enabled": enabled })).unwrap()).await;
            refresh_skills();
        });
    });

    let dismiss_onboarding = Callback::new(move |_| {
        show_onboarding.set(false);
        spawn_local(async move { let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await; });
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
    let folder_modal = create_rw_signal::<Option<FolderModal>>(None);
    let folder_modal_input = create_rw_signal(String::new());
    let ui_confirm = create_rw_signal::<Option<UiConfirm>>(None);
    let compose_menu_open = create_rw_signal(false);
    let compute_menu_open = create_rw_signal(false);
    let specialist_menu_open = create_rw_signal(false);
    let ssh_hosts = create_rw_signal::<Vec<SshHost>>(vec![]);
    let execution_contexts = create_rw_signal::<Vec<ExecutionContext>>(vec![]);
    let run_records = create_rw_signal::<Vec<RunRecord>>(vec![]);
    let show_add_host = create_rw_signal(false);
    let config_aliases = create_rw_signal::<Vec<String>>(vec![]);
    let host_alias = create_rw_signal(String::new());
    let host_user = create_rw_signal(String::new());
    let host_port = create_rw_signal(String::new());
    let host_identity = create_rw_signal(String::new());
    let host_notes = create_rw_signal(String::new());

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
    refresh_runs(run_records);
    {
        let refresh = Closure::wrap(Box::new(move || {
            refresh_runs(run_records);
        }) as Box<dyn FnMut()>);
        let _ = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    5_000,
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
        if show_add_host.get() {
            focus_and_select_soon("add-host-alias");
        }
    });
    let open_session = load_session.clone();
    let on_ctx_pick = {
        let open_session = open_session.clone();
        let sessions = sessions;
        let rename_session_target = rename_session_target;
        let rename_session_input = rename_session_input;
        let folder_modal = folder_modal;
        let folder_modal_input = folder_modal_input;
        let ui_confirm = ui_confirm;
        let active_session = active_session;
        let artifacts = artifacts;
        let input = input;
        Callback::new(move |(action, payload): (String, String)| {
            if action == "downloadFile" {
                download_artifact(payload);
                return;
            }
            if action == "copyImage" {
                spawn_local(async move {
                    if context_menu::copy_image(&payload).await { show_copy_toast(); }
                });
                return;
            }
            if action == "attachWorkspaceFile" {
                input.update(|text| {
                    if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
                    text.push_str(&format!("Use project file `{payload}` as context."));
                });
                focus_composer();
                return;
            }
            if action == "exportSession" {
                let session_id = if payload.is_empty() {
                    let Some(id) = active_session.get() else { return };
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
            if let Some(act) = context_menu::session_action(&action, &payload) {
                match act {
                    context_menu::SessionAction::Open(id) => open_session.call(id),
                    context_menu::SessionAction::Rename { id, title } => {
                        rename_session_input.set(title.clone());
                        rename_session_target.set(Some((id, title)));
                    }
                    context_menu::SessionAction::Move { id, folder_id } => {
                        let sessions = sessions;
                        spawn_local(async move {
                            let arg = to_value(&serde_json::json!({ "id": id, "folderId": folder_id })).unwrap();
                            if invoke_checked("move_session", arg).await.is_ok() {
                                refresh_sessions(sessions);
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
        if let Some(menu) = context_menu::build(&ev, loc, active_session.get().is_some()) {
            if !menu.items.is_empty() {
                ev.prevent_default();
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
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
        if ev.key() != "Escape" || ev.default_prevented() || ev.is_composing() {
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
        if folder_modal.get().is_some() {
            ev.prevent_default();
            folder_modal.set(None);
            return;
        }
        if show_add_host.get() {
            ev.prevent_default();
            show_add_host.set(false);
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
        if compute_menu_open.get() {
            ev.prevent_default();
            compute_menu_open.set(false);
            return;
        }
        if specialist_menu_open.get() {
            ev.prevent_default();
            specialist_menu_open.set(false);
            return;
        }
        if codex_config_menu_open.get() {
            ev.prevent_default();
            codex_config_menu_open.set(false);
            return;
        }
        if model_menu_open.get() {
            ev.prevent_default();
            model_menu_open.set(false);
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
        if active_session
            .get()
            .is_some_and(|_sid| items.get().iter().any(|i| matches!(i, ChatItem::ApprovalPending { .. })))
        {
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
        let mut el = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
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
        Callback::new(move |(project_id, session_id): (String, Option<String>)| {
            let request_epoch = transition_epoch.get().wrapping_add(1);
            transition_epoch.set(request_epoch);
            *transition_target.borrow_mut() = Some(project_id.clone());

            project_open_error.set(None);
            codex_runtime_error.set(None);
            status.set(String::new());
            show_proj_menu.set(false);
            demo_mode.set(false);
            items.set(vec![]);
            active_session.set(None);
            collapsed_folders.set(HashSet::new());
            project_info.set(None);
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
                        codex_runtime_error.set(Some(message.clone()));
                        status.set(message);
                        project_info.set(None);
                        *transition_target.borrow_mut() = None;
                        show_projects.set(true);
                        // Set this last so any effect triggered by returning to
                        // the landing cannot leave the pre-info send gate stuck.
                        codex_runtime_loading.set(false);
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
                refresh_sessions(sessions);
                refresh_folders(folders);
            });
        })
    };
    // Switch the active project inline (same guarded flow as the Projects screen).
    let switch_project = {
        let open_project_transition = open_project_transition;
        Callback::new(move |id: String| {
            open_project_transition.call((id, None));
        })
    };
    // Dedicated project window (#52): enter through the same serialized,
    // target-validated transition instead of maintaining a second startup path.
    if let Some(project_id) = dedicated_project_id {
        switch_project.call(project_id);
    }
    let toggle_proj_menu = move |_| {
        let opening = !show_proj_menu.get();
        show_proj_menu.set(opening);
        if opening {
            spawn_local(async move {
                let v = invoke("list_projects", JsValue::UNDEFINED).await;
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) { proj_list.set(list); }
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
        if proj_settings_busy.get() { return; }
        let form = proj_settings.get();
        if form.name.trim().is_empty() { return; }
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
                if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) { project_info.set(Some(p)); }
            }
        });
    };

    let move_session_to = {
        let sessions = sessions;
        Callback::new(move |(session_id, folder_id): (String, Option<String>)| {
            spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "id": session_id, "folderId": folder_id })).unwrap();
                if invoke_checked("move_session", arg).await.is_ok() {
                    refresh_sessions(sessions);
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

    let palette_open_session = {
        let open_project_transition = open_project_transition;
        Callback::new(move |(project_id, session_id): (String, String)| {
            open_project_transition.call((project_id, Some(session_id)));
        })
    };
    let palette_open_artifact = Callback::new(move |(path, name, kind): (String, String, String)| {
        modal_artifact.set(Some((path, name, kind)));
    });
    let palette_new_session = Callback::new(move |_: ()| {
        demo_mode.set(false);
        if let Some(old) = active_session.get() { transcripts.update(|m| { m.insert(old, items.get()); }); }
        attachments.set(vec![]);
        composer_references.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let Some(id) = invoke("new_session", JsValue::UNDEFINED).await.as_string() else {
                status.set(t(locale.get(), "status.send_failed").into());
                return;
            };
            active_session.set(Some(id));
            items.set(vec![]);
            refresh_sessions(sessions);
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
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) { skills_list.set(rows); }
        });
    });
    let palette_attach = Callback::new(move |reference: ComposerReferenceChip| {
        if !composer_references.get().iter().any(|item| item.key() == reference.key()) {
            composer_references.update(|items| items.push(reference));
        }
    });
    let palette_action = {
        let new_session = palette_new_session.clone();
        let project_settings = palette_project_settings.clone();
        let manage_skills = palette_manage_skills.clone();
        Callback::new(move |action: &'static str| match action {
            "new" => new_session.call(()),
            "search" => command_palette_open.set(true),
            "commands" => action_palette_open.set(true),
            "projects" => show_projects.set(true),
            "settings" => { show_settings.set(true); settings_section.set("models".into()); }
            "project-settings" => project_settings.call(()),
            "skills" => manage_skills.call(()),
            "toggle-sidebar" => show_sidebar.update(|show| *show = !*show),
            "artifacts" => ensure_right_tab(RightTab::Artifacts, show_right, open_right_tabs, right_tab),
            "notebook" => ensure_right_tab(RightTab::Notebook, show_right, open_right_tabs, right_tab),
            "files" => { ensure_right_tab(RightTab::File, show_right, open_right_tabs, right_tab); refresh_dir(file_cwd, file_entries); }
            "provenance" => ensure_right_tab(RightTab::Provenance, show_right, open_right_tabs, right_tab),
            "contexts" => { ensure_right_tab(RightTab::Hosts, show_right, open_right_tabs, right_tab); refresh_execution_contexts(execution_contexts); refresh_runs(run_records); }
            "side-chat" => ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab),
            "close-panel" => show_right.set(false),
            "theme-light" => theme_mode.set("light".into()),
            "theme-dark" => theme_mode.set("dark".into()),
            "theme-system" => theme_mode.set("system".into()),
            _ => {}
        })
    };
    let palette_project_id = Signal::derive(move || project_info.get().map(|p| p.id));
    let shortcut_action = palette_action.clone();
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return; };
        if ev.is_composing() || !(ev.ctrl_key() || ev.meta_key()) {
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
            "n" => { ev.prevent_default(); shortcut_action.call("new"); }
            "b" => { ev.prevent_default(); shortcut_action.call("toggle-sidebar"); }
            "," => { ev.prevent_default(); shortcut_action.call("settings"); }
            _ => {}
        }
    });
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return; };
        if ev.default_prevented()
            || ev.is_composing()
            || ev.alt_key()
            || ev.ctrl_key()
            || ev.meta_key()
            || ev.shift_key()
            || keyboard_event_targets_text_entry(ev)
        {
            return;
        }
        let Some((path, _, kind)) = modal_artifact.get() else { return; };
        let (prev_artifact, next_artifact) = modal_image_nav_targets(&artifacts.get(), &path, &kind);
        match ev.key().as_str() {
            "ArrowLeft" => {
                let Some((path, name, kind)) = prev_artifact else { return; };
                ev.prevent_default();
                modal_artifact.set(Some((path, name, kind)));
            }
            "ArrowRight" => {
                let Some((path, name, kind)) = next_artifact else { return; };
                ev.prevent_default();
                modal_artifact.set(Some((path, name, kind)));
            }
            _ => {}
        }
    });

    view! {
        {(is_windows() || is_mac()).then(|| view! {
            <WindowTitlebar locale=locale on_action=palette_action.clone() mac=is_mac() />
        })}
        <ActionPalette open=action_palette_open on_action=palette_action />
        <CommandPalette open=command_palette_open current_project_id=palette_project_id
            on_open_project=switch_project on_open_session=palette_open_session on_open_artifact=palette_open_artifact
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
        />
        <div class="app"
            class:app-hidden=move || show_projects.get() && !show_settings.get() && modal_artifact.get().is_none()
            on:contextmenu=on_context_menu>
        <Sidebar
            state=SidebarState {
                locale, show_sidebar, sidebar_w, show_proj_menu, show_projects, demo_mode, project_info, proj_list,
                sessions, folders, drag_session, drop_target, active_session, running,
                rename_session_input, rename_session_target, collapsed_folders, folder_modal_input,
                folder_modal, demos,
            }
            toggle_proj_menu=Callback::new(toggle_proj_menu)
            open_proj_settings=Callback::new(open_proj_settings)
            switch_project=switch_project
            new_session=Callback::new(new_session)
            new_folder=Callback::new(new_folder)
            open_files=Callback::new(open_files)
            load_demo=Callback::new(load_demo)
            load_session=load_session
            move_session_to=move_session_to
            open_capabilities=Callback::new(open_capabilities)
            open_settings=Callback::new(open_settings)
            on_sidebar_resize_start=Callback::new(on_sidebar_resize_start)
        />

        <main class="center">
            <div class="topbar">
                {move || (!show_sidebar.get()).then(|| view! {
                    <button class="icon-btn" title=move || t(locale.get(), "sidebar.show") on:click=move |_| show_sidebar.set(true)>{compose_icon("chevron")}</button>
                })}
                <span class="center-title">{move || {
                    let loc = locale.get();
                    if let Some(id) = active_session.get() {
                        if let Some(s) = sessions.get().iter().find(|s| s.id == id) {
                            let t = s.title.trim();
                            if !t.is_empty() { return s.title.clone(); }
                        }
                    }
                    items.get().iter().find_map(|i| match i {
                        ChatItem::User(msg) => {
                            let t = msg.trim();
                            if t.is_empty() { None }
                            else if t.chars().count() > 48 {
                                Some(format!("{}…", t.chars().take(48).collect::<String>()))
                            } else { Some(t.to_string()) }
                        }
                        _ => None,
                    }).unwrap_or_else(|| i18n::t(loc, "center.new_session").into())
                }}</span>
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
                <button class="icon-btn" title=move || t(locale.get(), "center.toggle_panel")
                    class:active=move || show_right.get()
                    on:click=move |_| {
                        show_right.update(|open| {
                            if *open {
                                *open = false;
                            } else {
                                if open_right_tabs.get_untracked().is_empty() {
                                    open_right_tabs.set(vec![RightTab::Artifacts, RightTab::Notebook]);
                                    right_tab.set(RightTab::Artifacts);
                                }
                                *open = true;
                            }
                        });
                    }><span class="gi panel"></span></button>
            </div>

            <div class="chat" id=CHAT_SCROLLER_ID>
                <div class="thread" id=CHAT_THREAD_ID>
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
                            // `with` avoids deep-cloning every message per flush;
                            // only rows being built clone their item below.
                            items.with(|list| {
                            let last = list.len().saturating_sub(1);
                            // Coalesce consecutive thinking + tool items into one
                            // foldable steps panel; render everything else as a
                            // normal row (#82). Items that render nothing (empty
                            // streaming placeholder, attempt_completion) are skipped
                            // so no `.thread` gap is left behind (#19).
                            let mut rows: Vec<(usize, u64, ThreadRow)> = Vec::new();
                            let mut i = 0usize;
                            while i < list.len() {
                                if renders_nothing(&list[i]) { i += 1; continue; }
                                if is_process_item(&list[i]) {
                                    let start = i;
                                    let mut run: Vec<(usize, ChatItem)> = Vec::new();
                                    let mut j = i;
                                    while j < list.len() {
                                        if renders_nothing(&list[j]) { j += 1; continue; }
                                        if is_process_item(&list[j]) { run.push((j, list[j].clone())); j += 1; }
                                        else { break; }
                                    }
                                    // Run reaching the tail while busy is the live one.
                                    let live = j > last && busy_now;
                                    let has_tool = run.iter().any(|(_, c)| matches!(c, ChatItem::Tool { .. }));
                                    if has_tool {
                                        let mut h = std::collections::hash_map::DefaultHasher::new();
                                        for (idx, it) in &run { (idx, it.fingerprint()).hash(&mut h); }
                                        live.hash(&mut h);
                                        let items_only: Vec<ChatItem> = run.into_iter().map(|(_, c)| c).collect();
                                        rows.push((start, h.finish(), ThreadRow::Steps { items: items_only, live }));
                                    } else {
                                        // Pure thinking (no tool): keep the bare .rz rows (#31).
                                        for (idx, it) in run {
                                            let is_last = idx == last;
                                            let fp = it.fingerprint() ^ (is_last && busy_now) as u64;
                                            rows.push((idx, fp, ThreadRow::Item { i: idx, item: it, is_last }));
                                        }
                                    }
                                    i = j;
                                } else {
                                    let is_last = i == last;
                                    let mut fp = list[i].fingerprint();
                                    // Assistant markdown embeds artifact chips (index + label).
                                    if matches!(&list[i], ChatItem::Assistant { .. }) { fp ^= arts_fp; }
                                    rows.push((i, fp, ThreadRow::Item { i, item: list[i].clone(), is_last }));
                                    i += 1;
                                }
                            }
                            rows
                            })
                        }
                        key=|(start, fp, _)| (*start, *fp)
                        children=move |(_, _, row)| {
                            match row {
                                ThreadRow::Item { i, item, is_last } => {
                                    let arts = artifacts.get_untracked();
                                    let sid = active_session.get().unwrap_or_default();
                                    let on_resume = Callback::new(resume_turn);
                                    view! {
                                        <div class=class_for(&item) data-ui-index=i.to_string()>
                                            {render_item(
                                                i, &item, &arts, on_artifact_select, on_file_link,
                                                busy.read_only(), is_last, edit_message, branch_message, sid,
                                                respond_confirm, on_resume, on_plan_action, on_plan_answer,
                                            )}
                                        </div>
                                    }.into_view()
                                }
                                ThreadRow::Steps { items, live } => view! {
                                    <div class="steps-wrap">{render_steps_group(items, live)}</div>
                                }.into_view(),
                            }
                        }
                    />
                </div>
            </div>

            <div class="composer">
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
                                let att_view = match att {
                                    ComposerAttachment::Uploading { name, .. } => {
                                        let label = if name.is_empty() {
                                            t(locale.get(), "composer.uploading").into()
                                        } else {
                                            name
                                        };
                                        view! { <span class="composer-attachment uploading">{label}</span> }.into_view()
                                    }
                                    ComposerAttachment::Ready { name, .. } => {
                                        view! { <span class="composer-attachment ready">{name}</span> }.into_view()
                                    }
                                    ComposerAttachment::Error { name, error, .. } => {
                                        view! {
                                            <span class="composer-attachment error" title=error.clone()>{name}</span>
                                        }.into_view()
                                    }
                                };
                                view! {
                                    <div class="composer-attachment-row">
                                        {att_view}
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
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
                                view! {
                                    <div class="composer-attachment-row">
                                        <span class="composer-attachment ready">{label}</span>
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
                                            on:click=move |_| composer_references.update(|items| items.retain(|item| item.key() != key))>{compose_icon("close")}</button>
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
                                if is_plan_command(&v) {
                                    // Native slash commands take precedence over
                                    // an installed skill named plan/default.
                                    picker_mode.set(None);
                                } else {
                                    match active_composer_trigger(&v) {
                                        Some((_, mode, q)) => { picker_query.set(q); picker_index.set(0); picker_mode.set(Some(mode)); }
                                        None => picker_mode.set(None),
                                    }
                                }
                                input.set(v);
                            }
                            on:keydown=on_send
                            on:paste=on_paste
                            prop:placeholder=move || t(locale.get(), "composer.placeholder")
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
                                            ComposerPickerItem::Artifact(a) => (a.name, format!("{} · {}", a.session_title.unwrap_or_default(), a.project_name.unwrap_or_default()), "attach"),
                                            ComposerPickerItem::Session(s) => (s.title, s.project_name, "review"),
                                            ComposerPickerItem::Skill(s) => (s.name, s.description, "skill"),
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
                                class:active=move || compute_menu_open.get()
                                title=move || t(locale.get(), "compute.button")
                                on:click=move |_| compute_menu_open.update(|o| *o = !*o)>
                                {compose_icon("server")}
                            </button>
                            {move || compute_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compute_menu_open.set(false)></div>
                                <div class="compose-menu compute-menu">
                                    <button type="button" class="compose-item" on:click=move |_| {
                                        compute_menu_open.set(false);
                                        show_add_host.set(true);
                                        spawn_local(async move {
                                            let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                            if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                        });
                                    }>
                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                        <span class="compose-item-text">
                                            <span class="compose-item-label">{move || t(locale.get(), "compute.add_host")}</span>
                                        </span>
                                    </button>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "hosts.title")}</div>
                                        {move || {
                                            let hs = ssh_hosts.get();
                                            if hs.is_empty() {
                                                view! { <div class="compose-item-sub" style="padding:6px 18px">{move || t(locale.get(), "compute.none")}</div> }.into_view()
                                            } else {
                                                hs.into_iter().map(|h| view! {
                                                    <button type="button" class="compose-item" on:click=move |_| {
                                                        compute_menu_open.set(false);
                                                        refresh_execution_contexts(execution_contexts);
                                                        refresh_runs(run_records);
                                                        ensure_right_tab(
                                                            RightTab::Hosts,
                                                            show_right,
                                                            open_right_tabs,
                                                            right_tab,
                                                        );
                                                    }>
                                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                                        <span class="compose-item-text"><span class="compose-item-label">{h.alias.clone()}</span></span>
                                                    </button>
                                                }.into_view()).collect_view()
                                            }
                                        }}
                                    </div>
                                </div>
                            })}
                            <button type="button" class="composer-compute"
                                class:active=move || specialist_menu_open.get()
                                title=move || t(locale.get(), "composer.specialist")
                                on:click=move |_| {
                                    refresh_specialists();
                                    specialist_menu_open.update(|o| *o = !*o);
                                }>
                                {compose_icon("skill")}
                            </button>
                            {move || specialist_menu_open.get().then(|| {
                                let locked = items.with(|l| !l.is_empty());
                                view! {
                                <div class="compose-backdrop" on:click=move |_| specialist_menu_open.set(false)></div>
                                <div class="compose-menu specialist-menu">
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.specialist")}</div>
                                        <button type="button" class="compose-item"
                                            disabled=locked
                                            title=move || locked.then(|| t(locale.get(), "composer.specialist.locked")).unwrap_or_default()
                                            on:click=move |_| {
                                                specialist_menu_open.set(false);
                                                pick_specialist(String::new());
                                            }>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.specialist.none")}</span>
                                            </span>
                                        </button>
                                        {move || specialists.get().into_iter().map(|s| {
                                            let id = s.id.clone();
                                            view! {
                                                <button type="button" class="compose-item"
                                                    disabled=locked
                                                    title=move || locked.then(|| t(locale.get(), "composer.specialist.locked")).unwrap_or_default()
                                                    on:click=move |_| {
                                                        specialist_menu_open.set(false);
                                                        pick_specialist(id.clone());
                                                    }>
                                                    <span class="compose-item-text">
                                                        <span class="compose-item-label">{s.name.clone()}</span>
                                                    </span>
                                                </button>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>
                            }})}
                        </div>
                        <div class="composer-buttons">
                            {move || (active_profile_uses_local_runner(&models.get()) && !active_profile_uses_codex(&models.get())).then(|| {
                                let plan_mode = collaboration_mode.get() == "plan";
                                view! {
                                    <div class="codex-composer-config local-runner-mode" class:plan=plan_mode>
                                        <button type="button" class="codex-mode-toggle"
                                            title=move || t(locale.get(), if plan_mode { "plan.switch_default" } else { "plan.switch_plan" })
                                            on:click=move |_| {
                                                let mode = if plan_mode { "default".to_string() } else { "plan".to_string() };
                                                collaboration_mode.set(mode.clone());
                                                persist_session_codex_overrides(
                                                    active_session.get(), mode, None, CodexModeOverrides::default(), codex_override_ui,
                                                );
                                            }>
                                            <span class="codex-mode-dot"></span>
                                            {move || t(locale.get(), if plan_mode { "codex.mode.plan" } else { "codex.mode.normal" })}
                                            {plan_mode.then(|| view! { <span class="codex-compat-mark">{move || t(locale.get(), "plan.compat")}</span> })}
                                        </button>
                                    </div>
                                }
                            })}
                            {move || (active_profile_uses_codex(&models.get()) && codex_runtime_loading.get()).then(|| view! {
                                <div class="codex-composer-config codex-runtime-loading" aria-live="polite">
                                    <span class="codex-runtime-loading-label"><span class="codex-mode-dot"></span>{t(locale.get(), "codex.runtime.loading_short")}</span>
                                </div>
                            })}
                            {move || (active_profile_uses_codex(&models.get()) && codex_runtime.get().is_none() && !codex_runtime_loading.get()).then(|| {
                                let plan_mode = collaboration_mode.get() == "plan";
                                let mode = if plan_mode { "plan".to_string() } else { "default".to_string() };
                                let profile = models.get().into_iter().find(|profile| profile.active)
                                    .or_else(|| models.get().into_iter().next()).unwrap();
                                let session_layer = codex_session_overrides.get();
                                let selected = session_layer.for_mode(&mode);
                                let profile_model = if plan_mode { &profile.plan_model } else { &profile.normal_model };
                                let profile_effort = if plan_mode { &profile.plan_reasoning_effort } else { &profile.normal_reasoning_effort };
                                let visible_model = selected.model.clone()
                                    .or_else(|| optional_profile_value(profile_model))
                                    .or_else(|| optional_profile_value(&profile.model))
                                    .unwrap_or_else(|| t(locale.get(), "codex.inherit").into());
                                let visible_effort = selected.effort.clone()
                                    .or_else(|| optional_profile_value(profile_effort))
                                    .or_else(|| optional_profile_value(&profile.reasoning_effort))
                                    .unwrap_or_else(|| t(locale.get(), "codex.inherit").into());
                                let mode_for_toggle = mode.clone();
                                let mode_for_model = mode.clone();
                                let mode_for_effort = mode.clone();
                                let mode_for_reset = mode.clone();
                                view! {
                                    <div class="codex-composer-config local-runner-mode" class:plan=plan_mode>
                                        <button type="button" class="codex-mode-toggle"
                                            title=move || t(locale.get(), if plan_mode { "plan.switch_default" } else { "plan.switch_plan" })
                                            on:click=move |_| {
                                                let next = if mode_for_toggle == "plan" { "default".to_string() } else { "plan".to_string() };
                                                collaboration_mode.set(next.clone());
                                                persist_session_codex_overrides(
                                                    active_session.get(), next, None, codex_session_overrides.get(), codex_override_ui,
                                                );
                                            }>
                                            <span class="codex-mode-dot"></span>
                                            {move || t(locale.get(), if plan_mode { "codex.mode.plan" } else { "codex.mode.normal" })}
                                            <span class="codex-compat-mark">{move || t(locale.get(), "plan.compat")}</span>
                                        </button>
                                        <button type="button" class="codex-config-toggle" class:active=move || codex_config_menu_open.get()
                                            on:click=move |_| codex_config_menu_open.update(|open| *open = !*open)>
                                            <span class="codex-config-model">{visible_model.clone()}</span>
                                            <span class="codex-config-effort">{visible_effort.clone()}</span>
                                            {move || codex_config_needs_confirmation.get().then(|| view! { <span class="codex-config-changed">"!"</span> })}
                                            <span class="model-picker-chev">"▾"</span>
                                        </button>
                                        {move || {
                                            let mode_for_model = mode_for_model.clone();
                                            let mode_for_effort = mode_for_effort.clone();
                                            let mode_for_reset = mode_for_reset.clone();
                                            let visible_model = visible_model.clone();
                                            let visible_effort = visible_effort.clone();
                                            codex_config_menu_open.get().then(|| view! {
                                            <div class="model-menu-backdrop" on:click=move |_| codex_config_menu_open.set(false)></div>
                                            <div class="codex-config-menu codex-compat-config-menu">
                                                <div class="codex-config-menu-head">
                                                    <strong>{move || t(locale.get(), "plan.compat_full")}</strong>
                                                    <span>"codex exec"</span>
                                                </div>
                                                <div class="codex-config-sources">{move || t(locale.get(), "codex.compat_profile_hint")}</div>
                                                <label><span>{move || t(locale.get(), "codex.model")}</span>
                                                    <input value=visible_model.clone() disabled=move || codex_session_override_conflict.get() on:change=move |event| {
                                                        let value = dom_value(&event);
                                                        codex_session_overrides.update(|overrides| {
                                                            let target = if mode_for_model == "plan" { &mut overrides.plan } else { &mut overrides.normal };
                                                            target.model = optional_profile_value(&value);
                                                        });
                                                        persist_session_codex_overrides(active_session.get(), mode_for_model.clone(), None, codex_session_overrides.get(), codex_override_ui);
                                                    } />
                                                </label>
                                                <label><span>{move || t(locale.get(), "codex.effort")}</span>
                                                    <input value=visible_effort.clone() disabled=move || codex_session_override_conflict.get() on:change=move |event| {
                                                        let value = dom_value(&event);
                                                        codex_session_overrides.update(|overrides| {
                                                            let target = if mode_for_effort == "plan" { &mut overrides.plan } else { &mut overrides.normal };
                                                            target.effort = optional_profile_value(&value);
                                                        });
                                                        persist_session_codex_overrides(active_session.get(), mode_for_effort.clone(), None, codex_session_overrides.get(), codex_override_ui);
                                                    } />
                                                </label>
                                                {plan_mode.then(|| view! { <div class="codex-inline-notice">{move || t(locale.get(), "codex.plan.read_only")}</div> })}
                                                {move || codex_config_needs_confirmation.get().then(|| view! {
                                                    {if codex_runtime_refresh_pending.get() {
                                                        view! { <div class="codex-refresh-required">{t(locale.get(), "codex.refresh_detected")}</div> }.into_view()
                                                    } else {
                                                        view! { <button type="button" class="codex-confirm-config" on:click=move |_| {
                                                            codex_config_needs_confirmation.set(false);
                                                            codex_session_override_conflict.set(false);
                                                            codex_config_menu_open.set(false);
                                                            status.set(t(locale.get_untracked(), "codex.confirmed").into());
                                                        }>{move || t(locale.get(), "codex.confirm_config")}</button> }.into_view()
                                                    }}
                                                })}
                                                <button type="button" class="codex-reset-overrides" on:click=move |_| {
                                                    codex_session_overrides.set(CodexModeOverrides::default());
                                                    persist_session_codex_overrides(active_session.get(), mode_for_reset.clone(), None, codex_session_overrides.get(), codex_override_ui);
                                                }>{move || t(locale.get(), "codex.reset_profile")}</button>
                                            </div>
                                            })
                                        }}
                                    </div>
                                }
                            })}
                            {move || (active_profile_uses_codex(&models.get()) && codex_runtime.get().is_some() && !codex_runtime_loading.get()).then(|| {
                                let plan_mode = collaboration_mode.get() == "plan";
                                let native = codex_runtime.get().is_some_and(|snapshot| snapshot.provider_capabilities.native_plan);
                                let preview = codex_preview.get();
                                let preview_pending = codex_preview_loading.get() || preview.as_ref().is_none_or(|config| {
                                    plan_mode != (config.mode == "plan")
                                });
                                let requested = preview.as_ref().map(|config| config.requested_model().to_string()).unwrap_or_default();
                                let effective = preview.as_ref().map(|config| config.effective_model().to_string()).unwrap_or_default();
                                let effort = (!preview_pending).then(|| preview.as_ref().map(|config| config.effective_effort().to_string()).unwrap_or_default()).unwrap_or_default();
                                let rerouted = !requested.is_empty() && !effective.is_empty() && requested != effective;
                                let model_label = if preview_pending {
                                    t(locale.get(), "codex.preview.loading_short").into()
                                } else if rerouted {
                                    format!("{requested} → {effective}")
                                } else if effective.is_empty() {
                                    t(locale.get(), "codex.inherit").into()
                                } else {
                                    effective
                                };
                                view! {
                                    <div class="codex-composer-config" class:plan=plan_mode>
                                        <button type="button" class="codex-mode-toggle"
                                            title=move || t(locale.get(), if plan_mode { "plan.switch_default" } else { "plan.switch_plan" })
                                            on:click=move |_| {
                                                let mode = if plan_mode { "default".to_string() } else { "plan".to_string() };
                                                collaboration_mode.set(mode.clone());
                                                persist_session_codex_overrides(
                                                    active_session.get(), mode,
                                                    codex_runtime.get().map(|snapshot| snapshot.config_version),
                                                    codex_session_overrides.get(), codex_override_ui,
                                                );
                                            }>
                                            <span class="codex-mode-dot"></span>
                                            {move || t(locale.get(), if plan_mode { "codex.mode.plan" } else { "codex.mode.normal" })}
                                            {(plan_mode && !native).then(|| view! { <span class="codex-compat-mark">{move || t(locale.get(), "plan.compat")}</span> })}
                                        </button>
                                        <button type="button" class="codex-config-toggle" class:active=move || codex_config_menu_open.get()
                                            on:click=move |_| codex_config_menu_open.update(|open| *open = !*open)>
                                            <span class="codex-config-model" class:rerouted=rerouted>{model_label}</span>
                                            {(!effort.is_empty()).then(|| view! { <span class="codex-config-effort">{effort}</span> })}
                                            {move || codex_config_needs_confirmation.get().then(|| view! { <span class="codex-config-changed">"!"</span> })}
                                            <span class="model-picker-chev">"▾"</span>
                                        </button>
                                        {move || codex_config_menu_open.get().then(|| {
                                            let snapshot = codex_runtime.get().unwrap_or_default();
                                            let mode = collaboration_mode.get();
                                            let layer = codex_session_overrides.get();
                                            let selected = layer.for_mode(&mode);
                                            let selected_model = selected.model.clone();
                                            let selected_effort = selected.effort.clone();
                                            let inherited_effective_model = codex_preview.get()
                                                .map(|config| config.effective_model().to_string());
                                            let efforts = composer_codex_efforts(
                                                &snapshot,
                                                selected_model.as_deref(),
                                                inherited_effective_model.as_deref(),
                                            );
                                            let generation = snapshot.config_version.clone();
                                            let generation_for_model = generation.clone();
                                            let generation_for_effort = generation.clone();
                                            let generation_for_reset = generation.clone();
                                            let title_key = if mode == "plan" { "codex.plan_override" } else { "codex.normal_override" };
                                            let mode_for_model = mode.clone();
                                            let mode_for_effort = mode.clone();
                                            let mode_for_reset = mode.clone();
                                            let snapshot_for_model = snapshot.clone();
                                            let inherited_for_model = inherited_effective_model.clone();
                                            let model_rows = snapshot.models.clone();
                                            let sources = codex_preview.get().map(|config| config.sources.into_iter()
                                                .map(|(field, source)| format!("{field}: {source}"))
                                                .collect::<Vec<_>>().join(" · ")).unwrap_or_default();
                                            let audit_content = if let Some(audit) = codex_turn_audits.get().last().cloned() {
                                                let requested = audit_json(&audit.requested);
                                                let sent = audit_json(&audit.sent);
                                                let actual = audit_json(&audit.actual);
                                                let unavailable = actual_verification_unavailable(&audit.actual);
                                                let meta = format!("{} · v{}", audit.mode, audit.config_version);
                                                view! {
                                                    <div class="codex-turn-audit-body">
                                                        <div class="codex-turn-audit-meta">{meta}</div>
                                                        <label><span>{t(locale.get(), "codex.audit.requested")}</span><pre>{requested}</pre></label>
                                                        <label><span>{t(locale.get(), "codex.audit.sent")}</span><pre>{sent}</pre></label>
                                                        <label><span>{t(locale.get(), "codex.audit.actual")}</span>
                                                            {if unavailable {
                                                                view! { <div class="codex-audit-unavailable">{t(locale.get(), "codex.audit.unavailable")}</div> }.into_view()
                                                            } else {
                                                                view! { <pre>{actual}</pre> }.into_view()
                                                            }}
                                                        </label>
                                                    </div>
                                                }.into_view()
                                            } else {
                                                view! { <div class="codex-turn-audit-empty">{t(locale.get(), "codex.audit.empty")}</div> }.into_view()
                                            };
                                            view! {
                                                <div class="model-menu-backdrop" on:click=move |_| codex_config_menu_open.set(false)></div>
                                                <div class="codex-config-menu">
                                                    <div class="codex-config-menu-head">
                                                        <strong>{move || t(locale.get(), title_key)}</strong>
                                                        <span>{format!("v{generation}")}</span>
                                                    </div>
                                                    <label><span>{move || t(locale.get(), "codex.model")}</span>
                                                        <select disabled=move || codex_session_override_conflict.get()
                                                            on:change=move |event| {
                                                                let value = dom_value(&event);
                                                                let model = (value != "__inherit__").then_some(value);
                                                                let supported = composer_codex_efforts(
                                                                    &snapshot_for_model,
                                                                    model.as_deref(),
                                                                    inherited_for_model.as_deref(),
                                                                );
                                                                let mut reset_effort = false;
                                                                codex_session_overrides.update(|overrides| {
                                                                    let target = if mode_for_model == "plan" { &mut overrides.plan } else { &mut overrides.normal };
                                                                    target.model = model;
                                                                    if target.effort.as_ref().is_some_and(|value| !supported.contains(value)) {
                                                                        target.effort = None;
                                                                        reset_effort = true;
                                                                    }
                                                                });
                                                                if reset_effort { status.set(t(locale.get_untracked(), "codex.effort_reset").into()); }
                                                                persist_session_codex_overrides(
                                                                    active_session.get(), mode_for_model.clone(), Some(generation_for_model.clone()), codex_session_overrides.get(), codex_override_ui,
                                                                );
                                                            }>
                                                            <option value="__inherit__" selected=selected_model.is_none()>{move || t(locale.get(), "codex.inherit_profile")}</option>
                                                            {model_rows.into_iter().map(|model| {
                                                                let is_selected = selected_model.as_deref() == Some(model.id.as_str());
                                                                view! { <option value=model.id.clone() selected=is_selected>{model.label()}</option> }
                                                            }).collect_view()}
                                                        </select>
                                                    </label>
                                                    <label><span>{move || t(locale.get(), "codex.effort")}</span>
                                                        <select disabled=move || codex_session_override_conflict.get()
                                                            on:change=move |event| {
                                                                let value = dom_value(&event);
                                                                let effort = (value != "__inherit__").then_some(value);
                                                                codex_session_overrides.update(|overrides| {
                                                                    let target = if mode_for_effort == "plan" { &mut overrides.plan } else { &mut overrides.normal };
                                                                    target.effort = effort;
                                                                });
                                                                persist_session_codex_overrides(
                                                                    active_session.get(), mode_for_effort.clone(), Some(generation_for_effort.clone()), codex_session_overrides.get(), codex_override_ui,
                                                                );
                                                            }>
                                                            <option value="__inherit__" selected=selected_effort.is_none()>{move || t(locale.get(), "codex.inherit_profile")}</option>
                                                            {efforts.into_iter().map(|effort| {
                                                                let is_selected = selected_effort.as_deref() == Some(effort.as_str());
                                                                view! { <option value=effort.clone() selected=is_selected>{effort}</option> }
                                                            }).collect_view()}
                                                        </select>
                                                    </label>
                                                    {(!sources.is_empty()).then(|| view! { <div class="codex-config-sources">{sources}</div> })}
                                                    <details class="codex-turn-audit" data-testid="codex-turn-audit">
                                                        <summary>{move || t(locale.get(), "codex.audit.title")}</summary>
                                                        {audit_content}
                                                    </details>
                                                    {move || codex_config_needs_confirmation.get().then(|| view! {
                                                        {if codex_runtime_refresh_pending.get() {
                                                            view! { <div class="codex-refresh-required">{t(locale.get(), "codex.refresh_detected")}</div> }.into_view()
                                                        } else {
                                                            view! { <button type="button" class="codex-confirm-config" on:click=move |_| {
                                                                codex_config_needs_confirmation.set(false);
                                                                codex_session_override_conflict.set(false);
                                                                codex_config_menu_open.set(false);
                                                                status.set(t(locale.get_untracked(), "codex.confirmed").into());
                                                            }>{move || t(locale.get(), "codex.confirm_config")}</button> }.into_view()
                                                        }}
                                                    })}
                                                    <button type="button" class="codex-reset-overrides" on:click=move |_| {
                                                        codex_session_overrides.set(CodexModeOverrides::default());
                                                        persist_session_codex_overrides(
                                                            active_session.get(), mode_for_reset.clone(), Some(generation_for_reset.clone()), codex_session_overrides.get(), codex_override_ui,
                                                        );
                                                    }>{move || t(locale.get(), "codex.reset_profile")}</button>
                                                </div>
                                            }
                                        })}
                                    </div>
                                }
                            })}
                            {move || (!models.get().is_empty()).then(|| view! {
                                <div class="model-picker">
                                    <button type="button" class="model-picker-btn" class:active=move || model_menu_open.get()
                                        on:click=move |_| model_menu_open.update(|o| *o = !*o)>
                                        <span class="model-picker-label">{move || {
                                            let l = models.get();
                                            l.iter().find(|m| m.active).or_else(|| l.first()).map(|m| m.label.clone()).unwrap_or_default()
                                        }}</span>
                                        <span class="model-picker-chev">"▾"</span>
                                    </button>
                                    {move || model_menu_open.get().then(|| view! {
                                        <div class="model-menu-backdrop" on:click=move |_| model_menu_open.set(false)></div>
                                        <div class="model-menu">
                                            {move || {
                                                let list = models.get();
                                                let can_delete = list.len() > 1;
                                                list.into_iter().map(|m| {
                                                    let pick_id = m.id.clone();
                                                    let del_id = m.id.clone();
                                                    let is_active = m.active;
                                                    let show_sub = !m.model.is_empty() && m.model != m.label;
                                                    view! {
                                                        <div class="model-menu-row" class:active=is_active>
                                                            <button type="button" class="model-menu-pick" on:click=move |_| {
                                                                model_menu_open.set(false);
                                                                let id = pick_id.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                    match invoke_checked("set_active_model", arg).await {
                                                                        Ok(v) => {
                                                                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                models.set(list);
                                                                            }
                                                                        }
                                                                        Err(err) => {
                                                                            web_sys::console::warn_1(&format!("set_active_model failed: {:?}", err).into());
                                                                        }
                                                                    }
                                                                });
                                                            }>
                                                                <span class="model-menu-text">
                                                                    <span class="model-menu-label">{m.label.clone()}</span>
                                                                    {show_sub.then(|| view! { <span class="model-menu-sub">{m.model.clone()}</span> })}
                                                                </span>
                                                                {is_active.then(|| view! { <span class="model-menu-check">"✓"</span> })}
                                                            </button>
                                                            {(can_delete && !is_active).then(|| { let id = del_id.clone(); view! {
                                                                <button type="button" class="model-menu-del"
                                                                    title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |_| {
                                                                        let id = id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                            let v = invoke("remove_model", arg).await;
                                                                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) { models.set(list); }
                                                                        });
                                                                    }>{compose_icon("close")}</button>
                                                            }})}
                                                        </div>
                                                    }
                                                }).collect_view()
                                            }}
                                            <button type="button" class="model-menu-add" on:click=move |_| {
                                                model_menu_open.set(false);
                                                model_form.set(Some(new_model_form()));
                                                model_form_key.set(String::new());
                                                model_form_msg.set(None);
                                                open_settings_fn(Some("models".into()));
                                            }>{move || t(locale.get(), "models.add")}</button>
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
                                    {move || t(locale.get(), if busy.get() { "composer.queue" } else { "composer.send" })}
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
                                        <button type="button" class="send-mode-item"
                                            on:click=move |_| {
                                                send_mode_menu_open.set(false);
                                                send.call(ComposerSendAction::PlanFirst);
                                            }>
                                            <span class="compose-item-icon">{compose_icon("plan")}</span>
                                            <span>{move || t(locale.get(), "composer.plan_first")}</span>
                                        </button>
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
                    <div class="composer-hint">{move || t(locale.get(), "composer.hint")}</div>
                </div>
            </div>
        </main>

        {move || show_right.get().then(|| view! {
            <div class="resizer" on:mousedown=on_resize_start></div>
            <button type="button" class="rightpane-backdrop"
                aria-label=move || t(locale.get(), "right.close")
                on:click=move |_| show_right.set(false)></button>
            <section class="rightpane" style=move || format!("width:{}px", right_w.get())>
                <div class="rp-tabs">
                    {move || {
                        let loc = locale.get();
                        let active = right_tab.get();
                        let art_n = artifacts.get().len();
                        let notebook_n = notebook_cells.get().len();
                        let prov_n = items.get().iter().filter(|i| matches!(i, ChatItem::Tool { .. })).count();
                        open_right_tabs.get().into_iter().map(|tab| {
                            let label = match tab {
                                RightTab::Artifacts => tab_count(loc, "right.artifacts", art_n),
                                RightTab::Notebook => tab_count(loc, "right.notebook", notebook_n),
                                RightTab::Provenance => tab_count(loc, "right.provenance", prov_n),
                                RightTab::File => t(loc, "right.file").into(),
                                RightTab::Hosts => t(loc, "contexts.title").into(),
                                RightTab::SideChat => t(loc, "sidechat.title").into(),
                            };
                            let is_active = active == tab;
                            view! {
                                <div class="rp-tab-wrap">
                                    <button type="button" class="rp-tab" class:active=is_active
                                        on:click=move |_| {
                                            right_tab.set(tab);
                                            match tab {
                                                RightTab::File => refresh_dir(file_cwd, file_entries),
                                                RightTab::Hosts => {
                                                    refresh_execution_contexts(execution_contexts);
                                                    refresh_runs(run_records);
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
                                    ALL_RIGHT_TABS.iter().copied().map(|tab| {
                                        let label = match tab {
                                            RightTab::Artifacts => tab_count(loc, "right.artifacts", art_n),
                                            RightTab::Notebook => tab_count(loc, "right.notebook", notebook_n),
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
                                                        RightTab::File => refresh_dir(file_cwd, file_entries),
                                                        RightTab::Hosts => {
                                                            refresh_execution_contexts(execution_contexts);
                                                            refresh_runs(run_records);
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
                    <div class="spacer"></div>
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
                                                                reveal_in_files(&sp, file_cwd, file_query, file_entries, show_right, open_right_tabs, right_tab);
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
                        RightTab::Notebook => {
                            view! {
                                <NotebookView cells=notebook_cells.get() locale=locale.get() />
                            }.into_view()
                        }
                        RightTab::File => {
                            let loc = locale.get();
                            let cwd = file_cwd.get();
                            let parent = if cwd == "." { None } else { Some(parent_path(&cwd)) };
                            view! {
                                <div class="rp-files">
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
                                                            <button class="fb-row dir" on:click=move |_| {
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
                                                            <button class="fb-row dir" on:click=move |_| {
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
                            let contexts = execution_contexts.get();
                            let runs = run_records.get();
                            let hs = ssh_hosts.get();
                            view! {
                                <div class="rp-contexts">
                                    <section class="control-section">
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
                                                view! {
                                                    <div class="context-card">
                                                        <div class="context-card-head">
                                                            <span class="context-id">{ctx.id.clone()}</span>
                                                            <span class=status_class>{status}</span>
                                                        </div>
                                                        <div class="context-label">{label}</div>
                                                        <div class="context-meta">{ctx.kind.clone()}{" · "}{summary}</div>
                                                        {ctx.last_probe_error.clone().map(|err| view! {
                                                            <div class="context-error">{err}</div>
                                                        })}
                                                    </div>
                                                }.into_view()
                                            }).collect_view()
                                        }}
                                        <div class="context-actions">
                                            <button type="button" class="rp-hosts-add"
                                                on:click=move |_| {
                                                    show_add_host.set(true);
                                                    spawn_local(async move {
                                                        let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                                        if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                                    });
                                                }><span class="gi plus"></span>{t(loc, "hosts.add")}</button>
                                            <button type="button" class="rp-hosts-add"
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        let v = invoke("import_ssh_config_hosts", JsValue::UNDEFINED).await;
                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                                                            ssh_hosts.set(list);
                                                            refresh_execution_contexts(execution_contexts);
                                                        }
                                                    });
                                                }><span class="gi server"></span>{t(loc, "hosts.import")}</button>
                                        </div>
                                    </section>
                                    <section class="control-section">
                                        <div class="control-section-head">
                                            <span>{t(loc, "contexts.runs")}</span>
                                            <div class="control-head-actions">
                                                <span class="control-count">{runs.len().to_string()}</span>
                                                <button type="button" class="icon-btn control-refresh"
                                                    title=t(loc, "runs.refresh")
                                                    aria-label=t(loc, "runs.refresh")
                                                    on:click=move |_| refresh_runs(run_records)>
                                                    "↻"
                                                </button>
                                            </div>
                                        </div>
                                        {if runs.is_empty() {
                                            view! { <div class="control-empty">{t(loc, "runs.empty")}</div> }.into_view()
                                        } else {
                                            runs.into_iter().map(|run| {
                                                let title = run_title(&run);
                                                let status_class = format!("run-status {}", run.status);
                                                let cancel_id = run.id.clone();
                                                let cancellable = matches!(run.status.as_str(), "submitted" | "running");
                                                let remote_workdir = run.remote_workdir.clone();
                                                let poll_error = run.last_poll_error.clone();
                                                let stdout_tail = run.stdout_tail.clone().unwrap_or_default();
                                                let stderr_tail = run.stderr_tail.clone().unwrap_or_default();
                                                        let output = match (stdout_tail.is_empty(), stderr_tail.is_empty()) {
                                                    (false, false) => format!("{stdout_tail}\n\n[stderr]\n{stderr_tail}"),
                                                    (false, true) => stdout_tail,
                                                    (true, false) => format!("[stderr]\n{stderr_tail}"),
                                                    (true, true) => String::new(),
                                                };
                                                let meta = match run.exit_code {
                                                    Some(code) => format!("{} · {} · exit {code}", run.context_id, run.kind),
                                                    None => format!("{} · {}", run.context_id, run.kind),
                                                };
                                                view! {
                                                    <div class="run-card">
                                                        <div class="run-card-head">
                                                            <span class="run-title">{title}</span>
                                                            <span class=status_class>{run.status.clone()}</span>
                                                            {cancellable.then(|| {
                                                                let run_id = cancel_id.clone();
                                                                view! {
                                                                    <button type="button" class="icon-btn run-cancel"
                                                                        title=t(loc, "runs.cancel")
                                                                        aria-label=t(loc, "runs.cancel")
                                                                        on:click=move |_| {
                                                                            let run_id = run_id.clone();
                                                                            spawn_local(async move {
                                                                                let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                                                let _ = invoke("cancel_run", arg).await;
                                                                                refresh_runs(run_records);
                                                                            });
                                                                        }>
                                                                        "×"
                                                                    </button>
                                                                }
                                                            })}
                                                        </div>
                                                        <div class="run-meta">{meta}</div>
                                                        {run.command.clone().filter(|c| !c.trim().is_empty()).map(|cmd| view! {
                                                            <div class="run-command">{cmd}</div>
                                                        })}
                                                        {remote_workdir.map(|workdir| view! {
                                                            <div class="run-remote">
                                                                <span>{t(loc, "runs.remote_workdir")}</span>
                                                                <code>{workdir}</code>
                                                            </div>
                                                        })}
                                                        {poll_error.filter(|error| !error.trim().is_empty()).map(|error| view! {
                                                            <div class="context-error">{error}</div>
                                                        })}
                                                        {(!output.is_empty()).then(|| view! {
                                                            <details class="run-output">
                                                                <summary>{t(loc, "runs.output")}</summary>
                                                                <pre>{output}</pre>
                                                            </details>
                                                        })}
                                                    </div>
                                                }.into_view()
                                            }).collect_view()
                                        }}
                                    </section>
                                    {(!hs.is_empty()).then(|| view! {
                                        <section class="control-section">
                                            <div class="control-section-head">
                                                <span>{t(loc, "hosts.title")}</span>
                                                <span class="control-count">{hs.len().to_string()}</span>
                                            </div>
                                            {hs.into_iter().map(|h| {
                                                let alias = h.alias.clone();
                                                let conn = {
                                                    let mut c = String::new();
                                                    if let Some(u) = &h.user { c.push_str(u); c.push('@'); }
                                                    c.push_str(&h.alias);
                                                    if let Some(p) = h.port { c.push_str(&format!(":{p}")); }
                                                    c
                                                };
                                                view! {
                                                    <div class="host-card">
                                                        <div class="host-card-head">
                                                            <span class="host-card-alias">{h.alias.clone()}</span>
                                                            <button type="button" class="host-card-remove"
                                                                on:click=move |_| {
                                                                    let alias = alias.clone();
                                                                    let arg = to_value(&serde_json::json!({ "alias": alias })).unwrap();
                                                                    spawn_local(async move {
                                                                        let v = invoke("remove_ssh_host", arg).await;
                                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                                                                            ssh_hosts.set(list);
                                                                            refresh_execution_contexts(execution_contexts);
                                                                        }
                                                                    });
                                                                }>{compose_icon("close")}</button>
                                                        </div>
                                                        <div class="host-card-conn">{conn}</div>
                                                        {h.notes.clone().map(|n| view! { <div class="host-card-notes">{n}</div> })}
                                                    </div>
                                                }
                                            }).collect_view()}
                                        </section>
                                    })}
                                </div>
                            }.into_view()
                        }
                        RightTab::SideChat => {
                            view! {
                                <div class="sidechat-in-pane">
                                    <div class="sidechat-log">
                                        {move || {
                                            let rows = side_chat_items.get();
                                            if rows.is_empty() && !side_chat_busy.get() {
                                                view! { <div class="sidechat-empty">{move || t(locale.get(), "sidechat.empty")}</div> }.into_view()
                                            } else {
                                                rows.into_iter().map(|item| match item {
                                                    ChatItem::User(text) => view! {
                                                        <div class="sidechat-row user"><div class="sidechat-bubble">{text}</div></div>
                                                    }.into_view(),
                                                    ChatItem::Assistant { text, model } => {
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
                                                if ev.is_composing() { return; }
                                                if ev.key() == "Enter" && !ev.shift_key() {
                                                    ev.prevent_default();
                                                    send_side_chat(side_chat_input.get());
                                                }
                                            }
                                        ></textarea>
                                        <div class="sidechat-actions">
                                            {move || (!models.get().is_empty()).then(|| view! {
                                                <div class="sidechat-model">
                                                    <button type="button" class="sidechat-model-btn"
                                                        class:active=move || side_chat_model_menu_open.get()
                                                        on:click=move |_| side_chat_model_menu_open.update(|o| *o = !*o)>
                                                        {move || {
                                                            let l = models.get();
                                                            l.iter().find(|m| m.active).or_else(|| l.first()).map(|m| m.label.clone()).unwrap_or_default()
                                                        }}
                                                        <span>"▾"</span>
                                                    </button>
                                                    {move || side_chat_model_menu_open.get().then(|| view! {
                                                        <div class="sidechat-model-backdrop" on:click=move |_| side_chat_model_menu_open.set(false)></div>
                                                        <div class="sidechat-model-menu">
                                                            {move || models.get().into_iter().map(|m| {
                                                                let pick_id = m.id.clone();
                                                                let is_active = m.active;
                                                                view! {
                                                                    <button type="button" class="sidechat-model-row" class:active=is_active
                                                                        on:click=move |_| {
                                                                            side_chat_model_menu_open.set(false);
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
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    let title = rename_session_input.get().trim().to_string();
                                    if title.is_empty() { return; }
                                    let id = id_key.clone();
                                    let sessions = sessions;
                                    rename_session_target.set(None);
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                        if invoke_checked("rename_session", arg).await.is_ok() {
                                            refresh_sessions(sessions);
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
                            let sessions = sessions;
                            rename_session_target.set(None);
                            spawn_local(async move {
                                let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                if invoke_checked("rename_session", arg).await.is_ok() {
                                    refresh_sessions(sessions);
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

        {move || ui_confirm.get().map(|action| {
            let action_ok = action.clone();
            let msg_key = match &action {
                UiConfirm::DeleteFolder(_) => "folder.delete_confirm",
                UiConfirm::DeleteSession(_) => "session.delete_confirm",
            };
            view! {
            <div class="overlay">
                <div class="modal confirm-modal">
                    <h2>{move || t(locale.get(), "confirm.title")}</h2>
                    <div class="hint">{move || t(locale.get(), msg_key)}</div>
                    <div class="row">
                        <button on:click=move |_| ui_confirm.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            ui_confirm.set(None);
                            match action_ok.clone() {
                                UiConfirm::DeleteFolder(id) => {
                                    let folders = folders;
                                    let sessions = sessions;
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                        if invoke_checked("delete_folder", arg).await.is_ok() {
                                            refresh_folders(folders);
                                            refresh_sessions(sessions);
                                        }
                                    });
                                }
                                UiConfirm::DeleteSession(id) => {
                                    let sessions = sessions;
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
                                            refresh_sessions(sessions);
                                        }
                                    });
                                }
                            }
                        }>{move || t(locale.get(), "confirm.approve")}</button>
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
                    on_open_path=Callback::new(move |(p, _k): (String, String)| {
                        reveal_in_files(&p, file_cwd, file_query, file_entries, show_right, open_right_tabs, right_tab);
                        modal_artifact.set(None);
                    }) />
            }
        })}
        <SettingsView
            state=SettingsViewState {
                locale, show_settings, settings_section, open_conn_key, connectors, model_form,
                conn_form, memory_selected, specialist_form, settings, bootstrap, settings_message,
                settings_busy, model_form_open, model_form_key, models, model_form_msg, specialists,
                specialist_form_open, memory_view, memory_editor, memory_msg, skills_list,
                skill_filter_tag, skills_search, skills_msg, cred_status, cred_inputs, cred_msg,
                approval_grants, conns_view, conn_form_open, conn_form_kind, conn_test_msg,
                custom_conn_tools, custom_conn_tools_loading, custom_conn_tool_errors,
                codex_runtime, codex_runtime_error, codex_runtime_loading, codex_settings_action_loading,
                codex_profile_overrides, codex_preview_normal, codex_preview_plan,
            }
            go_settings_section=Callback::new(move |section: String| go_settings_section(&section))
            close_settings_subpage=Callback::new(move |_: ()| close_settings_subpage())
            check_updates=Callback::new(check_updates)
            save_settings=Callback::new(save_settings)
            save_model_form=Callback::new(save_model_form)
            save_specialist_form=Callback::new(save_specialist_form)
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
            remove_specialist=Callback::new(remove_specialist_fn)
            refresh_codex_runtime=refresh_codex_runtime
            preview_codex_configs=preview_codex_configs
            save_codex_profile=save_codex_profile
        />



        <AddHostOverlay
            locale=locale show_add_host=show_add_host host_alias=host_alias config_aliases=config_aliases
            host_notes=host_notes host_user=host_user host_port=host_port host_identity=host_identity
            ssh_hosts=ssh_hosts execution_contexts=execution_contexts
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
        ChatItem::QueuedUser(_) => "msg user queued",
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => "tool-wrap",
        ChatItem::Assistant { .. } => "msg assistant",
        ChatItem::Reasoning(_) => "msg reasoning",
        ChatItem::Tool { .. } => "tool-wrap",
        ChatItem::ApprovalPending { .. } => "tool-wrap approval-wrap-row",
        ChatItem::Review(_) => "tool-wrap",
        ChatItem::Plan(_) => "tool-wrap plan-wrap",
        ChatItem::PlanQuestion(_) => "tool-wrap plan-question-wrap",
    }
}

/// A run of consecutive "process" items (thinking + tool calls) folds into one
/// collapsible steps panel; every other item renders as a normal row. Keeps the
/// main thread to messages + a foldable activity summary instead of a wall of
/// tool cards (#82).
fn is_process_item(item: &ChatItem) -> bool {
    match item {
        ChatItem::Reasoning(_) => true,
        ChatItem::Tool { name, .. } => name != "attempt_completion",
        _ => false,
    }
}

/// One thread render unit: either a single message, or a coalesced steps panel.
#[derive(Clone)]
enum ThreadRow {
    Item { i: usize, item: ChatItem, is_last: bool },
    Steps { items: Vec<ChatItem>, live: bool },
}

/// Compact, foldable summary of a thinking + tool run (#82). Collapsed by
/// default; auto-opens while it is the live tail so progress stays visible.
///
/// Built as a manual accordion (signal + `class:open`) rather than
/// `<details>/<summary>`: the UA disclosure marker survives `list-style:none`
/// + `::-webkit-details-marker` here (WebKit and Blink alike), and there is no
/// portable way to drop it — so we don't render one.
fn render_steps_group(items: Vec<ChatItem>, live: bool) -> impl IntoView {
    let locale = use_locale();
    let n_tools = items.iter().filter(|c| matches!(c, ChatItem::Tool { .. })).count();
    let now = now_ms();
    let total_ms: u64 = items.iter().map(|c| match c {
        ChatItem::Tool { duration_ms: Some(d), .. } => *d,
        ChatItem::Tool { duration_ms: None, started_at_ms: Some(s), ok: None, .. } if live => {
            now.saturating_sub(*s)
        }
        _ => 0,
    }).sum();
    let title = move || {
        if live { t(locale.get(), "chat.steps_running").to_string() }
        else if n_tools == 1 { t(locale.get(), "chat.steps_1").to_string() }
        else { tf(locale.get(), "chat.steps_n", &[("n", &n_tools.to_string())]) }
    };
    let total_label = (total_ms > 0 && (!live || n_tools > 0)).then(|| format_duration_ms(total_ms));
    let open = create_rw_signal(live);
    let rows = items.into_iter().map(|it| match it {
        ChatItem::Reasoning(text) => {
            let ropen = create_rw_signal(false);
            view! {
                <div class="step step-think" class:open=move || ropen.get()>
                    <div class="step-head" on:click=move |_| ropen.update(|o| *o = !*o)>
                        <span class="step-icon think"></span>
                        <span class="step-name">{move || t(locale.get(), "chat.thinking")}</span>
                    </div>
                    <div class="step-think-body">{text}</div>
                </div>
            }.into_view()
        }
        ChatItem::Tool { name, ok, input, output, started_at_ms, duration_ms, .. } => {
            let sopen = create_rw_signal(ok.is_none() && live);
            let detail: String = input
                .lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
                .chars().take(80).collect();
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
                <div class="step" class:open=move || sopen.get() class=("no-body", !has_body)>
                    <div class="step-head" on:click=move |_| { if has_body { sopen.update(|o| *o = !*o) } }>
                        {icon}
                        <span class="step-name">{name}</span>
                        {(!detail.is_empty()).then(|| view! { <span class="step-detail">{detail}</span> })}
                        {meta}
                    </div>
                    {has_body.then(|| view! {
                        <div class="step-body">
                            {(!input.is_empty()).then(|| view! { <pre class="tool-input">{input.clone()}</pre> })}
                            {(!output.is_empty()).then(|| view! { <pre class="tool-output">{output.clone()}</pre> })}
                        </div>
                    })}
                </div>
            }.into_view()
        }
        _ => view! {}.into_view(),
    }).collect_view();
    view! {
        <div class="steps" class:open=move || open.get()>
            <div class="steps-head" on:click=move |_| open.update(|o| *o = !*o)>
                <span class="steps-chevron"></span>
                <span class="steps-title">{title}</span>
                {total_label.map(|label| view! { <span class="steps-meta">{label}</span> })}
            </div>
            <div class="steps-body">{rows}</div>
        </div>
    }
}

fn render_item(
    ui_index: usize,
    item: &ChatItem,
    artifacts: &[Artifact],
    on_artifact: Callback<usize>,
    on_file: Callback<(String, String)>,
    busy: ReadSignal<bool>,
    is_last: bool,
    on_edit: impl Fn(usize) + Clone + 'static,
    on_branch: impl Fn(usize) + Clone + 'static,
    session_id: String,
    on_approval: Callback<(String, bool, Option<String>, String)>,
    on_resume: Callback<usize>,
    on_plan_action: Callback<(String, String, i64, bool)>,
    on_plan_answer: Callback<(String, String)>,
) -> impl IntoView {
    let locale = use_locale();
    match item {
        ChatItem::User(s) => view! {
            <UserMessage
                text=s.clone()
                ui_index=ui_index
                busy=busy
                on_copy=Callback::new(copy_text)
                on_edit=Callback::new(on_edit)
                on_branch=Callback::new(on_branch)
            />
        }.into_view(),
        ChatItem::QueuedUser(s) => view! {
            <div class="role">{move || t(locale.get(), "composer.queued")}</div>
            <div class="user-bubble queued-bubble">
                <div class="body">{s.clone()}</div>
            </div>
        }.into_view(),
        ChatItem::Assistant { text, .. } if text.trim().is_empty() => view! {}.into_view(),
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => {
            let msg = text.strip_prefix("Error: ").unwrap_or(text.as_str()).to_string();
            let copy = msg.clone();
            view! {
                <div class="finding err">
                    <div class="finding-head">
                        <span class="finding-tag">{move || format!("● {}", t(locale.get(), "chat.error"))}</span>
                        <span class="finding-title">{msg}</span>
                        <button type="button" class="tool-btn"
                            disabled=move || busy.get()
                            on:click=move |_| on_resume.call(ui_index)>
                            {move || t(locale.get(), "chat.resume")}
                        </button>
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                </div>
            }.into_view()
        }
        ChatItem::Assistant { text, model } => view! {
            <AssistantMessage
                text=text.clone()
                model=model.clone()
                artifacts=artifacts.to_vec()
                on_artifact=on_artifact
                on_file=on_file
                on_copy=Callback::new(copy_text)
            />
        }.into_view(),
        ChatItem::Tool { name, .. } if name == "attempt_completion" => view! {}.into_view(),
        ChatItem::Reasoning(s) => {
            // Auto-expand the block while it is the live, streaming item. The thread
            // is a non-keyed re-render, so every reasoning delta rebuilds this
            // <details> from scratch; a DOM-only open state would snap shut on the
            // next chunk and the user could never watch the live thinking (#31).
            let live = is_last && busy.get();
            view! {
                <details class="rz" open=live>
                    <summary>{move || t(locale.get(), "chat.thinking")}</summary>
                    <div class="body">{s.clone()}</div>
                </details>
            }.into_view()
        }
        ChatItem::Tool { name, ok, input, output, .. } => view! {
            <ToolBlock name=name.clone() ok=*ok input=input.clone() output=output.clone() />
        }.into_view(),
        ChatItem::ApprovalPending { tool, preview, message: _ } => view! {
            <ApprovalCard tool=tool.clone() preview=preview.clone() session_id=session_id.clone() on_decide=on_approval />
        }.into_view(),
        ChatItem::Plan(plan) => {
            let html = md_to_html(&plan.text);
            let complete = plan.complete;
            let native = plan.native;
            let actionable = plan.actionable;
            let plan_id_approve = plan.plan_id.clone();
            let plan_id_modify = plan.plan_id.clone();
            let plan_id_save = plan.plan_id.clone();
            let revision = plan.revision;
            let has_claim = !plan.plan_id.is_empty() && revision > 0;
            view! {
                <article class="plan-card" class:streaming=!complete class:compat=!native data-testid="plan-card">
                    <header class="plan-card-head">
                        <span class="plan-card-icon">{compose_icon("plan")}</span>
                        <div><strong>{move || t(locale.get(), "plan.card.title")}</strong><span>{move || t(locale.get(), if complete { "plan.card.ready" } else { "plan.card.streaming" })}</span></div>
                        {(!native).then(|| view! { <span class="plan-compat-badge">{move || t(locale.get(), "plan.compat_full")}</span> })}
                    </header>
                    <div class="plan-card-body markdown" inner_html=html></div>
                    {(complete && actionable && has_claim).then(|| view! {
                        <footer class="plan-card-actions">
                            <button type="button" class="primary" disabled=move || busy.get() on:click=move |_| on_plan_action.call(("approve".into(), plan_id_approve.clone(), revision, native))>{move || t(locale.get(), "plan.approve")}</button>
                            <button type="button" disabled=move || busy.get() on:click=move |_| on_plan_action.call(("modify".into(), plan_id_modify.clone(), revision, native))>{move || t(locale.get(), "plan.modify")}</button>
                            <button type="button" disabled=move || busy.get() on:click=move |_| on_plan_action.call(("save_exit".into(), plan_id_save.clone(), revision, native))>{move || t(locale.get(), "plan.save_exit")}</button>
                        </footer>
                    })}
                </article>
            }.into_view()
        }
        ChatItem::PlanQuestion(question) => {
            let answer = create_rw_signal(String::new());
            let question_id = question.question_id.clone();
            let question_id_keydown = question_id.clone();
            let question_id_click = question_id.clone();
            let question_text = question.question.clone();
            let options = question.options.clone();
            let allow_freeform = question.is_other || options.is_empty();
            let input_type = if question.is_secret { "password" } else { "text" };
            view! {
                <section class="plan-question-card" data-testid="plan-question-card">
                    <div class="plan-question-head"><span>{compose_icon("chat")}</span><strong>{move || t(locale.get(), "plan.question.title")}</strong></div>
                    <p>{question_text}</p>
                    {(!options.is_empty()).then(|| view! {
                        <div class="plan-question-options">{options.into_iter().map(|option| {
                            let value = option.label().to_string();
                            let description = option.description().to_string();
                            let send_value = value.clone();
                            let id = question_id.clone();
                            view! {
                                <button type="button" on:click=move |_| on_plan_answer.call((id.clone(), send_value.clone()))>
                                    <strong>{value}</strong>
                                    {(!description.is_empty()).then(|| view! { <span>{description}</span> })}
                                </button>
                            }
                        }).collect_view()}</div>
                    })}
                    {allow_freeform.then(|| view! {
                        <div class="plan-question-freeform">
                            <input type=input_type prop:value=move || answer.get()
                                placeholder=move || t(locale.get(), "plan.question.placeholder")
                                on:input=move |event| answer.set(event_target_value(&event))
                                on:keydown=move |event: web_sys::KeyboardEvent| {
                                    if event.key() == "Enter" && !event.shift_key() {
                                        event.prevent_default();
                                        on_plan_answer.call((question_id_keydown.clone(), answer.get()));
                                    }
                                } />
                            <button type="button" class="primary" disabled=move || answer.get().trim().is_empty()
                                on:click=move |_| on_plan_answer.call((question_id_click.clone(), answer.get()))>{move || t(locale.get(), "plan.question.send")}</button>
                        </div>
                    })}
                </section>
            }.into_view()
        }
        ChatItem::Review(report) => {
            let report = report.clone();
            let count = report.findings.len();
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
                    {(count == 0).then(|| view! {
                        <div class="review-empty">"✓ "{move || t(locale.get(), "review.no_findings")}</div>
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
    mount_to_body(App);
}

#[cfg(test)]
mod codex_ui_tests {
    use super::*;

    #[test]
    fn native_plan_commands_beat_skill_picker_only_when_complete() {
        assert!(is_plan_command("/plan"));
        assert!(is_plan_command("/plan inspect this"));
        assert!(is_plan_command(" /default"));
        assert!(!is_plan_command("/planner"));
        assert!(!is_plan_command("/pla"));
    }

    #[test]
    fn failed_send_removes_only_its_empty_optimistic_pair() {
        let mut rows = vec![
            ChatItem::Assistant { text: "older".into(), model: None },
            ChatItem::User("retry me".into()),
            ChatItem::Assistant { text: String::new(), model: Some("gpt-test".into()) },
        ];
        remove_optimistic_send_rows(&mut rows, "retry me");
        assert_eq!(rows.len(), 1);

        let mut streamed = vec![
            ChatItem::User("keep me".into()),
            ChatItem::Assistant { text: "already streamed".into(), model: None },
        ];
        remove_optimistic_send_rows(&mut streamed, "keep me");
        assert_eq!(streamed.len(), 2);
        assert_eq!(
            split_turn_started_error("[turn-started] failed"),
            (true, "failed")
        );
        assert_eq!(split_turn_started_error("failed"), (false, "failed"));
    }

    #[test]
    fn only_durable_pending_plan_is_actionable() {
        let pending = proposed_plan_card(ProposedPlanRecord {
            id: "plan-1".into(), revision: 2, text: "Do it".into(),
            native: true, status: "pending".into(),
        });
        assert!(pending.actionable);
        let saved = proposed_plan_card(ProposedPlanRecord {
            id: "plan-1".into(), revision: 2, text: "Do it".into(),
            native: true, status: "saved".into(),
        });
        assert!(!saved.actionable);
        let tokenless = proposed_plan_card(ProposedPlanRecord {
            id: String::new(), revision: 0, text: "legacy".into(),
            native: true, status: "pending".into(),
        });
        assert!(!tokenless.actionable);
    }

    #[test]
    fn runtime_adapter_payload_accepts_nullable_inherited_values() {
        let snapshot: RuntimeSnapshot = serde_json::from_value(serde_json::json!({
            "config_version": 7,
            "runtime": {
                "executable_path": "C:/tools/codex.exe",
                "version": null,
                "codex_home": "C:/project/.wisp/codex-home/profile",
                "source": "path",
                "context": "windows"
            },
            "models": [{
                "id": "gpt-test",
                "display_name": "GPT Test",
                "supported_reasoning_efforts": ["low", "max", "ultra"]
            }],
            "config": {
                "config_version": 7,
                "mode": "plan",
                "requested_model": null,
                "effective_model": "gpt-test",
                "requested_effort": null,
                "effective_effort": "max",
                "service_tier": null,
                "personality": null,
                "summary": null,
                "verbosity": null,
                "web_search": null,
                "sandbox": "read-only",
                "sources": {"model":"local_codex"},
                "warnings": []
            },
            "collaboration_modes": [{"id":"plan","label":"Plan"}],
            "provider_capabilities": {"app_server":true,"native_plan":true},
            "warnings": [],
            "refreshed_at": "123"
        })).expect("runtime payload should decode");
        assert_eq!(snapshot.config_version, "7");
        assert_eq!(snapshot.models[0].supported_reasoning_efforts, ["low", "max", "ultra"]);
        assert_eq!(snapshot.config.as_ref().unwrap().requested_model(), "");
        assert_eq!(snapshot.config.as_ref().unwrap().effective_model(), "gpt-test");
    }

    #[test]
    fn send_payload_carries_visible_codex_snapshot() {
        let args = SendMessageArgs {
            session_id: Some("s1".into()),
            message: "inspect".into(),
            attachments: vec![],
            references: vec![],
            resume: false,
            collaboration_mode: Some("plan".into()),
            codex_config_generation: Some("9007199254740993123".into()),
            codex_overrides: Some(CodexModeOverrides {
                plan: CodexModeOverride { model: Some("gpt-test".into()), effort: Some("ultra".into()) },
                ..Default::default()
            }),
        };
        let value = serde_json::to_value(args).unwrap();
        assert_eq!(value["collaborationMode"], "plan");
        assert_eq!(value["codexConfigGeneration"], "9007199254740993123");
        assert_eq!(value["codexOverrides"]["plan"]["effort"], "ultra");
    }
}
