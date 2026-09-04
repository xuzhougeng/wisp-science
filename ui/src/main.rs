mod acp;
mod agent_workflows;
mod app_overlays;
mod bindings;
mod channels_view;
mod chat_render;
mod context_menu;
mod dto;
mod i18n;
mod library;
mod mcp_app;
mod notebook;
mod overlays;
mod pet;
mod project_landing;
mod publication;
mod research;
mod runtime_views;
mod session_modals;
mod settings_view;
mod sidebar;
mod text;
mod trajectory;
mod window_titlebar;

use agent_workflows::{
    agent_workflows_panel, refresh_agent_resources, refresh_agent_workflows, AgentPanelState,
};
use app_overlays::{
    advance_browser_tab_cleanup, present_browser_needs_human, present_browser_tab_cleanup,
    BrowserNeedsHumanOverlay, BrowserNeedsHumanOverlayState, BrowserTabCleanupOverlay,
    BrowserTabCleanupOverlayState, ContextRecoveryOverlay, ContextRecoveryOverlayState,
    ExternalLinkConfirm, ProjectExportPrompt, ProjectExportPromptState, ProjectTransferOverlay,
    ProjectTransferOverlayState, SshConnectivityOverlay, SshConnectivityOverlayState,
    TurnMemoryOverlay, TurnMemoryOverlayState, UpdateCheckOverlay, UpdateCheckOverlayState,
};
use bindings::{
    add_workspace_file_to_motif, attach_chat_autoscroll, cancel_saved_marks_apply, clear_selection,
    close_mcp_app, force_chat_bottom, invoke, invoke_checked, is_mac, is_windows,
    jump_chat_to_item, jump_chat_to_user, listen, listen_current_window, listen_native_file_drop,
    native_drop_in_composer, open_browser_extension_page, open_external_url, pasted_image_count,
    preserve_chat_prepend_position, preview_selection, restore_chat_session_scroll,
    schedule_chat_follow, set_saved_marks, set_window_title, CHAT_SCROLLER_ID, CHAT_THREAD_ID,
};
use context_menu::{ContextMenuPortal, CtxMenu};
use dto::*;
use i18n::{
    empty_subtitle, empty_title, localize_backend, send_failed, set_document_lang, t, tab_count,
    tf, Locale, EMPTY_SUBTITLE_COUNT, EMPTY_TITLE_COUNT,
};
use leptos::{ev, window_event_listener, *};
use library::{refresh_library, refresh_session_library, HighlightsPane, LibraryScreen};
use notebook::{collect_notebook_cells, NotebookCache, NotebookView};
use overlays::{
    AddHostOverlay, CapabilitiesOverlay, OnboardingOverlay, RunReviewModal, RunReviewOverlay,
    RuntimeInterpreterOverlay, ShareOverlay, StoragePrefsOverlay,
};
use pet::{PetDesktop, PetOverlay};
use project_landing::{ProjectLanding, ProjectLandingState};
use publication::{PublicationEvidenceSource, PublicationWorkspaceModal};
use research::{refresh_research_graph, ResearchGraphModal};
use serde_wasm_bindgen::{from_value, to_value};
use session_modals::{
    BranchMergeDetailOverlay, BranchMergeOverlay, BranchMergeOverlayState, EditConfirmOverlay,
    EditConfirmOverlayState, ExplorationOverlay, ExplorationOverlayState, ExplorationOverlayView,
    FileEntryOverlay, FileEntryOverlayState, FolderModalOverlay, FolderModalOverlayState,
    ModelSwitchConfirmOverlay, ModelSwitchConfirmOverlayState, ProjSettingsOverlay,
    ProjSettingsOverlayState, RenameSessionOverlay, RenameSessionOverlayState,
    SessionTransferOverlay, SessionTransferOverlayState, TurnUndoOverlay, TurnUndoOverlayState,
};
use settings_view::{known_effort_values, ALL_EFFORT_VALUES};
use settings_view::{DeleteConfirm, SettingsView, SettingsViewState};
use sidebar::{Sidebar, SidebarState};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use text::{
    dom_value, event_target_checked, event_target_value, file_kind, format_bytes,
    group_artifact_indices, ime_composing, is_runtime_code_selection, join_path, md_to_html,
    note_composition_end, opens_in_system_browser, parent_path, provider_defaults,
    runtime_language, user_message_presentation, DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL,
};
use trajectory::TrajectoryOverlay;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use window_titlebar::{app_window_title, WindowTitlebar};

/// Stable substring of the backend's missing-key error (`src-tauri` `send_message`),
/// used to turn that failure into an actionable "open Settings" prompt.
const NO_API_KEY_MARK: &str = "No API key set";
const HOME_SEARCH_PROJECT_LIMIT: usize = 6;
const HOME_SEARCH_ARTIFACT_LIMIT: usize = 8;
const HOME_SEARCH_SESSION_LIMIT: usize = 6;
const TRANSCRIPT_RENDER_TURNS: usize = 20;
const TRANSCRIPT_WINDOW_STEP: usize = 20;
const TRANSCRIPT_LIVE_TRIM_TURNS: usize = TRANSCRIPT_RENDER_TURNS + TRANSCRIPT_WINDOW_STEP;
const CENTER_PANE_MIN_WIDTH: f64 = 360.0;
const CENTER_CHAT_MIN_WIDTH: f64 = 320.0;
const CENTER_DOCUMENT_MIN_WIDTH: f64 = 240.0;
const RIGHT_PANE_MIN_WIDTH: f64 = 320.0;
const RIGHT_PANE_MAX_WIDTH: f64 = 900.0;
const PANE_RESIZER_WIDTH: f64 = 5.0;
const SIDEBAR_RESIZER_WIDTH: f64 = 10.0;
const THEME_STORAGE_KEY: &str = "wisp-theme";
const SIDE_CHAT_SCROLLER_ID: &str = "side-chat-scroller";
const SIDE_CHAT_INPUT_ID: &str = "side-chat-input";

fn service_tier_enabled(value: &str) -> bool {
    matches!(value.trim(), "priority" | "fast")
}

fn supports_fast_service_tier(profile: &ModelProfile) -> bool {
    profile.is_chat_model()
        && matches!(
            profile.provider.trim(),
            "openai" | "openai_responses" | "openai-responses" | "responses"
        )
}

fn selected_fast_profile(
    models: &[ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
    acp_selected: bool,
) -> Option<ModelProfile> {
    if acp_selected
        || session_id
            .and_then(|id| session_models.get(id))
            .is_some_and(|id| id.starts_with("acp:"))
    {
        return None;
    }
    session_profile(models, session_models, session_id)
        .filter(|profile| supports_fast_service_tier(profile))
        .cloned()
}

/// Let component-owned inner surfaces consume Escape before the app-level
/// stack sees it. The listener is capture-phase and owner-scoped, so it does
/// not depend on focus landing inside the surface and is removed on cleanup.
pub(crate) fn window_capture_escape(mut close_topmost: impl FnMut() -> bool + 'static) {
    let listener = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::wrap(Box::new(
        move |event: web_sys::KeyboardEvent| {
            if event.key() != "Escape"
                || event.default_prevented()
                || ime_composing(&event)
                || !close_topmost()
            {
                return;
            }
            event.prevent_default();
            event.stop_propagation();
        },
    ));
    let window = web_sys::window();
    if let Some(window) = &window {
        let _ = window.add_event_listener_with_callback_and_bool(
            "keydown",
            listener.as_ref().unchecked_ref(),
            true,
        );
    }
    on_cleanup(move || {
        if let Some(window) = window {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "keydown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    });
}

fn session_highlight_count(session: Option<String>, items: &[LibraryItemSummary]) -> usize {
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

mod app_support;
use acp::*;
use app_support::*;
pub(crate) use chat_render::*;
use mcp_app::*;
use runtime_views::*;

#[allow(clippy::too_many_arguments)]
fn request_turn_memory_proposal(
    session_id: String,
    turn_index: Option<usize>,
    automatic: bool,
    proposal: RwSignal<Option<TurnMemoryProposal>>,
    editor: RwSignal<String>,
    scope: RwSignal<String>,
    replace_id: RwSignal<String>,
    loading: RwSignal<HashSet<String>>,
    error: RwSignal<Option<String>>,
    status: RwSignal<String>,
    locale: RwSignal<Locale>,
) {
    if loading.with_untracked(|ids| ids.contains(&session_id)) {
        return;
    }
    loading.update(|ids| {
        ids.insert(session_id.clone());
    });
    if !automatic {
        status.set(t(locale.get_untracked(), "memory.proposal.generating"));
    }
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "sessionId": session_id.clone(),
            "turnIndex": turn_index,
            "automatic": automatic,
        }))
        .unwrap();
        match invoke_checked("propose_turn_memory", args).await {
            Ok(value) => match from_value::<Option<TurnMemoryProposal>>(value) {
                Ok(Some(next)) if proposal.get_untracked().is_none() => {
                    editor.set(next.content.clone());
                    scope.set(next.scope.clone());
                    replace_id.set(String::new());
                    error.set(None);
                    proposal.set(Some(next));
                    status.set(t(locale.get_untracked(), "memory.proposal.ready"));
                }
                Ok(_) => {
                    if !automatic {
                        status.set(t(locale.get_untracked(), "memory.proposal.none"));
                    }
                }
                Err(parse_error) => {
                    status.set(tf(
                        locale.get_untracked(),
                        "memory.proposal.failed",
                        &[("msg", &parse_error.to_string())],
                    ));
                }
            },
            Err(invoke_error) => {
                status.set(tf(
                    locale.get_untracked(),
                    "memory.proposal.failed",
                    &[(
                        "msg",
                        &localize_backend(locale.get_untracked(), &js_error_text(invoke_error)),
                    )],
                ));
            }
        }
        loading.update(|ids| {
            ids.remove(&session_id);
        });
    });
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
    let ui_font_family = create_rw_signal(load_ui_font_family());
    let code_font_family = create_rw_signal(load_code_font_family());
    let appearance_hydrated = create_rw_signal(false);
    create_effect(move |_| {
        apply_font_prefs(
            ui_font_size.get(),
            code_font_size.get(),
            &ui_font_family.get(),
            &code_font_family.get(),
        )
    });
    let selection_popup_enabled = create_rw_signal(load_selection_popup_enabled());
    create_effect(move |_| save_selection_popup_enabled(selection_popup_enabled.get()));
    let send_with_modifier = create_rw_signal(load_send_with_modifier());
    create_effect(move |_| save_send_with_modifier(send_with_modifier.get()));
    let custom_css = create_rw_signal(load_custom_css());
    create_effect(move |_| apply_custom_css(&custom_css.get()));
    create_effect(move |_| {
        if !appearance_hydrated.get() {
            return;
        }
        let prefs = AppearancePrefs {
            theme: theme_mode.get(),
            light_palette: light_palette.get(),
            dark_palette: dark_palette.get(),
            ui_font_size: ui_font_size.get(),
            code_font_size: code_font_size.get(),
            ui_font_family: ui_font_family.get(),
            code_font_family: code_font_family.get(),
            selection_popup_enabled: selection_popup_enabled.get(),
            send_with_modifier: send_with_modifier.get(),
            custom_css: custom_css.get(),
        };
        spawn_local(async move {
            let _ = invoke_checked(
                "set_appearance_prefs",
                to_value(&serde_json::json!({ "prefs": prefs })).unwrap(),
            )
            .await;
        });
    });

    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    // Expensive transcript projections do not need token-by-token freshness.
    // This revision advances for ordinary settled edits and for the structural
    // events explicitly marked in the streaming handlers below.
    let transcript_projection_epoch = create_rw_signal(0_u64);
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
        if items.with(Vec::is_empty) {
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
    let motif_selection = create_rw_signal(None::<MotifSelection>);
    let uploading = create_rw_signal(false);
    let remote_file_uploading = create_rw_signal(false);
    let files_drag_over = create_rw_signal(false);
    let remote_files_refresh_tick = create_rw_signal(0u32);
    let drag_over = create_rw_signal(false);
    // Per-session streaming state. `running` is the set of session ids with an
    // in-flight turn; `transcripts` caches the live transcript of background
    // (non-active) sessions so switching to them shows streaming progress.
    let running = create_rw_signal::<HashSet<String>>(HashSet::new());
    let reviewing = create_rw_signal::<HashSet<String>>(HashSet::new());
    let approval_pending = create_rw_signal::<HashSet<String>>(HashSet::new());
    let pet_activity = create_rw_signal((String::from("idle"), 0_u64));
    let pending_turns = create_rw_signal::<HashMap<String, usize>>(HashMap::new());
    let transcripts = create_rw_signal::<HashMap<String, Vec<ChatItem>>>(HashMap::new());
    let transcript_pages = create_rw_signal::<HashMap<String, TranscriptPageState>>(HashMap::new());
    let conversation_outlines =
        create_rw_signal::<HashMap<String, Vec<SessionOutlineItem>>>(HashMap::new());
    let conversation_outline_open = create_rw_signal(false);
    let conversation_outline_mounted = create_rw_signal(false);
    let conversation_outline_selected = create_rw_signal::<Option<usize>>(None);
    create_effect(move |_| {
        if conversation_outline_open.get() {
            conversation_outline_mounted.set(true);
            return;
        }
        if !conversation_outline_mounted.get_untracked() {
            return;
        }
        set_timeout(
            move || {
                if !conversation_outline_open.get_untracked() {
                    conversation_outline_mounted.set(false);
                }
            },
            // Keep mounted through `--motion-duration-medium` so the close animation can play.
            std::time::Duration::from_millis(280),
        );
    });
    let busy = create_rw_signal(false);
    let turn_undo_dialog = create_rw_signal::<Option<TurnUndoDialog>>(None);
    let turn_undo_busy = create_rw_signal(false);
    let turn_undo_error = create_rw_signal::<Option<String>>(None);
    // ui_index of a user message whose edit would discard later conversation;
    // Some means the edit/branch confirm modal is open for that message.
    let edit_confirm = create_rw_signal::<Option<usize>>(None);
    // Interrupting a running turn (especially a language runtime) is not instant, so
    // keep track of the session whose Stop click is waiting for the backend.
    let stopping_session = create_rw_signal::<Option<String>>(None);
    let show_settings = create_rw_signal(false);
    let settings_section = create_rw_signal(String::from("general"));
    let memory_view = create_rw_signal(None::<MemoryView>);
    let memory_selected = create_rw_signal(None::<String>);
    let memory_editor = create_rw_signal(String::new());
    let memory_msg = create_rw_signal(None::<(bool, String)>);
    let turn_memory_proposal = create_rw_signal(None::<TurnMemoryProposal>);
    let turn_memory_editor = create_rw_signal(String::new());
    let turn_memory_scope = create_rw_signal(String::from("project"));
    let turn_memory_replace_id = create_rw_signal(String::new());
    let turn_memory_loading = create_rw_signal(HashSet::<String>::new());
    let turn_memory_busy = create_rw_signal(false);
    let turn_memory_error = create_rw_signal(None::<String>);
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
    let conn_form_open = create_memo(move |_| conn_form.get().is_some());
    // Same reason, one level deeper: the connection form swaps stdio/http fields
    // on `kind`; track just `kind` so editing command/url doesn't rebuild them.
    let conn_form_kind = create_memo(move |_| conn_form.get().map(|f| f.kind).unwrap_or_default());
    let settings = create_rw_signal(Settings::default());
    let follow_up_questions = create_rw_signal(HashMap::<String, Vec<String>>::new());
    let follow_up_generation = create_rw_signal(HashMap::<String, u64>::new());
    // This mirrors the last persisted sync configuration. Keep it separate
    // from `settings`, which also holds unsaved edits while Settings is open.
    let sync_actions_available = create_rw_signal(false);
    let pet_status = create_rw_signal(PetStatus::default());
    let run_records = create_rw_signal::<Vec<RunSummary>>(vec![]);
    // A transcript row is remounted when a later turn changes its keyed
    // projection. Keep explicit Run-card dismissals above the card component
    // so a dismissed terminal Run cannot flash back during that remount.
    let dismissed_run_cards = create_rw_signal::<HashSet<String>>(HashSet::new());
    // Configured model profiles + the composer's bottom-right picker state.
    let models = create_rw_signal::<Vec<ModelProfile>>(vec![]);
    let active_session = create_rw_signal::<Option<String>>(None);
    // The stopping banner belongs to the session where Stop was clicked, and
    // only while that session is still running. Switching conversations must
    // not carry it over; a missed or late Done must not leave it over Send.
    create_effect(move |_| {
        let next = next_stopping_session(
            stopping_session.get(),
            active_session.get().as_deref(),
            &running.get(),
        );
        if stopping_session.get() != next {
            stopping_session.set(next);
        }
    });
    let sessions = create_rw_signal::<Vec<SessionInfo>>(vec![]);
    // Trajectory (轨迹) modal: the fetched per-session snapshot plus lightweight
    // live cells for the in-flight turn. The Done/Error refetch reconciles the
    // live cells with exact backend data.
    let trajectory_open = create_rw_signal(false);
    let trajectory_snapshot = create_rw_signal::<Option<TrajectorySnapshotDto>>(None);
    let trajectory_live = create_rw_signal::<Vec<TrajectoryCellDto>>(vec![]);
    let fetch_trajectory: Rc<dyn Fn(String)> = Rc::new(move |frame_id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": frame_id })).unwrap();
            if let Ok(value) = invoke_checked("load_session_trajectory", arg).await {
                if let Ok(snap) = serde_wasm_bindgen::from_value::<TrajectorySnapshotDto>(value) {
                    // A slow fetch must not overwrite a newer session's view.
                    if active_session.get_untracked().as_deref() == Some(snap.frame_id.as_str()) {
                        trajectory_snapshot.set(Some(snap));
                    }
                }
            }
        });
    });
    // Fetch when the modal opens and on session change; never let one
    // session's timeline bleed into another.
    let trajectory_session = create_rw_signal::<Option<String>>(None);
    let fetch_trajectory_fx = fetch_trajectory.clone();
    create_effect(move |_| {
        let session = active_session.get();
        let open = trajectory_open.get();
        if trajectory_session.get_untracked() != session {
            trajectory_session.set(session.clone());
            trajectory_snapshot.set(None);
            trajectory_live.set(vec![]);
        }
        if open {
            if let Some(id) = session {
                fetch_trajectory_fx(id);
            }
        }
    });
    let active_branch_state = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let active = active_session.get();
        let state = active.and_then(|id| {
            sessions.with(|rows| {
                rows.iter()
                    .find(|session| session.id == id)
                    .and_then(|session| session.branch_state.clone())
            })
        });
        active_branch_state.set(state);
    });
    let conversation_branches =
        create_rw_signal::<HashMap<String, Vec<SessionBranchLink>>>(HashMap::new());
    let explorations = create_rw_signal::<Vec<ExplorationSummary>>(vec![]);
    // Exploration frames are intentionally absent from the ordinary session
    // query. Keep their ids separately so a sidebar refresh never mistakes the
    // active exploration for a newly-created, untitled conversation draft.
    let exploration_frames = create_rw_signal::<HashSet<String>>(HashSet::new());
    create_effect(move |_| {
        exploration_frames.set(
            explorations
                .get()
                .into_iter()
                .map(|row| row.exploration.frame_id)
                .collect(),
        );
    });
    let mainline_frozen = create_memo(move |_| {
        active_session.get().is_some_and(|frame_id| {
            explorations.with(|rows| rows.iter().any(|row| row.source_frame_id == frame_id))
        })
    });
    let active_is_exploration = create_memo(move |_| {
        active_session.get().is_some_and(|frame_id| {
            explorations.with(|rows| rows.iter().any(|row| row.exploration.frame_id == frame_id))
        })
    });
    let composer_scope_locked = create_memo(move |_| {
        active_session.get().is_some_and(|frame_id| {
            explorations.with(|rows| {
                rows.iter()
                    .find(|row| row.exploration.frame_id == frame_id)
                    .is_some_and(|row| row.exploration.status != "active")
            }) || mainline_frozen.get()
                || matches!(
                    active_branch_state.get().as_deref(),
                    Some("merged" | "orphaned")
                )
        })
    });
    let exploration_overlay = create_rw_signal::<Option<ExplorationOverlay>>(None);
    let exploration_name = create_rw_signal(String::new());
    let exploration_preview = create_rw_signal::<Option<ExplorationPromotionPreview>>(None);
    let exploration_busy = create_rw_signal(false);
    let exploration_error = create_rw_signal::<Option<String>>(None);
    let session_has_items = create_memo(move |_| items.with(|rows| !rows.is_empty()));
    let can_share = create_memo(move |_| items.with(|rows| transcript_has_shareable(rows)));
    let conversation_outline = create_memo(move |_| {
        let Some(id) = active_session.get() else {
            return Vec::new();
        };
        let _ = transcript_projection_epoch.get();
        let persisted = conversation_outlines
            .with(|outlines| outlines.get(&id).cloned())
            .unwrap_or_default();
        let user_offset = transcript_pages
            .with(|pages| pages.get(&id).copied())
            .map_or(0, |page| page.user_offset);
        items.with_untracked(|rows| merge_conversation_outline(&persisted, rows, user_offset))
    });
    let center_conversation_title = create_memo(move |_| {
        let loc = locale.get();
        let _ = transcript_projection_epoch.get();
        if let Some(id) = active_session.get() {
            if let Some(title) = sessions.with(|sessions| {
                sessions
                    .iter()
                    .find(|session| session.id == id)
                    .and_then(|session| {
                        let clean = user_message_presentation(&session.title).body;
                        (!clean.trim().is_empty()).then_some(clean)
                    })
            }) {
                return title;
            }
        }
        items.with_untracked(|items| {
            items
                .iter()
                .find_map(|item| match item {
                    ChatItem::User(message) => {
                        let clean = user_message_presentation(message).body;
                        let title = clean.trim();
                        if title.is_empty() {
                            None
                        } else if title.chars().count() > 48 {
                            Some(format!("{}…", title.chars().take(48).collect::<String>()))
                        } else {
                            Some(title.to_string())
                        }
                    }
                    _ => None,
                })
                .unwrap_or_else(|| i18n::t(loc, "center.new_session").into())
        })
    });
    let loaded_conversation_user_range = create_memo(move |_| {
        let _ = transcript_projection_epoch.get();
        let user_offset = active_session
            .get()
            .and_then(|id| transcript_pages.with(|pages| pages.get(&id).copied()))
            .map_or(0, |page| page.user_offset);
        let loaded = items.with_untracked(|rows| {
            rows.iter()
                .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
                .count()
        });
        user_offset..user_offset + loaded
    });
    create_effect(move |_| {
        let _ = active_session.get();
        conversation_outline_open.set(false);
        conversation_outline_selected.set(None);
    });
    let session_model_ids = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    // Map presence means the backend value has loaded; its nested None means
    // this conversation inherits the selected model profile's Fast default.
    let session_service_tiers = create_rw_signal::<HashMap<String, Option<String>>>(HashMap::new());
    // A pre-send Fast override is held in memory until the first frame exists.
    // None means the selected profile default needs no override.
    let pending_service_tier = create_rw_signal::<Option<String>>(None);
    let service_tier_busy = create_rw_signal(false);
    let acp_agents = create_rw_signal::<Vec<AcpAgentProfile>>(vec![]);
    let active_acp_agent_id = create_rw_signal::<Option<String>>(None);
    let fast_profile = Signal::derive(move || {
        selected_fast_profile(
            &models.get(),
            &session_model_ids.get(),
            active_session.get().as_deref(),
            active_acp_agent_id.get().is_some(),
        )
    });
    let fast_enabled = create_memo(move |_| {
        let profile_default = fast_profile
            .get()
            .as_ref()
            .is_some_and(|profile| service_tier_enabled(&profile.service_tier));
        match active_session.get() {
            Some(session_id) => session_service_tiers
                .with(|values| values.get(&session_id).cloned())
                .flatten()
                .map_or(profile_default, |value| service_tier_enabled(&value)),
            None => pending_service_tier
                .get()
                .map_or(profile_default, |value| service_tier_enabled(&value)),
        }
    });
    // A staged pre-send override belongs to the selected HTTP profile. If the
    // user switches profile and the staged target now equals that profile's
    // default (or selects ACP/unsupported), there is nothing to persist.
    create_effect(move |_| {
        if active_session.get().is_some() {
            return;
        }
        match (fast_profile.get(), pending_service_tier.get()) {
            (None, Some(_)) => pending_service_tier.set(None),
            (Some(profile), Some(value))
                if service_tier_enabled(&profile.service_tier) == service_tier_enabled(&value) =>
            {
                pending_service_tier.set(None)
            }
            _ => {}
        }
    });
    let fast_is_session_override = create_memo(move |_| match active_session.get() {
        Some(session_id) => session_service_tiers
            .with(|values| values.get(&session_id).is_some_and(Option::is_some)),
        None => pending_service_tier.get().is_some(),
    });
    let fast_loaded = create_memo(move |_| {
        active_session.get().is_none_or(|session_id| {
            session_service_tiers.with(|values| values.contains_key(&session_id))
        })
    });
    let settings_busy = create_rw_signal(false);
    let settings_message = create_rw_signal::<Option<(bool, String)>>(None);
    // Model & specialist settings domain: form signals + save/validate/test
    // handlers live in `app_support::model_settings`; `App` only wires them.
    let model_settings = ModelSettingsState::new(
        models,
        acp_agents,
        settings,
        settings_busy,
        settings_message,
        locale,
    );
    let model_form = model_settings.model_form;
    let model_catalog_limits = model_settings.model_catalog_limits;
    let model_form_key = model_settings.model_form_key;
    let model_form_msg = model_settings.model_form_msg;
    let specialists = model_settings.specialists;
    let specialist_form = model_settings.specialist_form;
    // Some/None memos (not the form contents) gate the sub-form panes; see the
    // comment on `conn_form_open` above.
    let model_form_open = create_memo(move |_| model_form.get().is_some());
    let specialist_form_open = create_memo(move |_| specialist_form.get().is_some());
    let acp_context_usage =
        create_rw_signal::<HashMap<String, ContextUsageSnapshot>>(HashMap::new());
    let context_usage = ContextUsageState::new();
    let context_usage_open = context_usage.open;
    let context_usage_mode = context_usage.mode;
    let context_usage_geom = context_usage.geom;
    let context_usage_dragging = context_usage.dragging;
    let context_usage_tracking = context_usage.tracking;
    let context_usage_resizing = context_usage.resizing;
    let context_usage_suppress_click = context_usage.suppress_click;
    let context_usage_details = context_usage.details;
    let context_usage_detail_open = context_usage.detail_open;
    let active_context_usage = create_memo(move |_| {
        let session_id = active_session.get()?;
        if active_acp_agent_id.get().is_some() {
            acp_context_usage.with(|all| all.get(&session_id).cloned())
        } else {
            let _ = transcript_projection_epoch.get();
            let snapshot = items.with_untracked(|rows| latest_context_usage(rows));
            // A usage row only appears after a turn runs, so its `max` names
            // the model that produced it. Re-base an idle session on the model
            // it is bound to now. While a turn is running, keep the window of
            // the actual cached Agent: a mid-turn model switch is intentionally
            // applied at the next boundary, not hot-swapped under the request.
            snapshot.map(|mut snapshot| {
                if !running.get().contains(&session_id) {
                    if let Some(max) = session_context_window(
                        &models.get(),
                        &session_model_ids.get(),
                        Some(&session_id),
                    ) {
                        snapshot.max = max as usize;
                    }
                }
                snapshot
            })
        }
    });
    let context_usage_flash = create_rw_signal(false);
    {
        let last_used = Rc::new(Cell::new(None::<usize>));
        create_effect(move |_| {
            let Some(snapshot) = active_context_usage.get() else {
                last_used.set(None);
                return;
            };
            let previous = last_used.replace(Some(snapshot.used));
            if previous.is_some_and(|previous| snapshot.used < previous) {
                context_usage_flash.set(true);
                set_timeout(
                    move || context_usage_flash.set(false),
                    std::time::Duration::from_millis(700),
                );
            }
        });
    }
    create_effect(move |_| {
        let _ = active_session.get();
        context_usage_open.set(false);
        context_usage_details.set(None);
        context_usage_detail_open.set(None);
    });
    create_effect(move |_| {
        let open = context_usage_open.get();
        let docked = context_usage_mode.get() == ContextUsageMode::Docked;
        // Wait for the slot to enter/leave the layout. Docked open pins the
        // latest reply against the panel (B2); close/undock keep the existing
        // follow helper so a scrolled-up reading position is not yanked.
        set_timeout(
            move || {
                if open && docked {
                    force_chat_bottom();
                } else {
                    schedule_chat_follow();
                }
            },
            std::time::Duration::from_millis(0),
        );
    });
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
    let show_projects = create_rw_signal(true); // app lands on the Projects screen
    let show_library = create_rw_signal(false);
    let show_session_import = create_rw_signal(None::<SessionImportProvider>);
    let library_items = create_rw_signal::<Vec<LibraryItemSummary>>(vec![]);
    let session_library_items = create_rw_signal::<Vec<LibraryItem>>(vec![]);
    let refresh_library_items = Callback::new(move |_: ()| {
        refresh_library(library_items);
        refresh_session_library(session_library_items, active_session.read_only());
    });
    refresh_library_items.call(());
    create_effect(move |_| {
        let _ = active_session.get();
        refresh_session_library(session_library_items, active_session.read_only());
    });
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    provide_context(project_info.read_only());
    let demo_mode = create_rw_signal(false); // true = the synthetic "Example project" is open
    let scratch_open = create_rw_signal(false); // ephemeral scratch chat overlay
    let feedback_context = create_rw_signal::<Option<String>>(None);
    let project_open_error = create_rw_signal(None::<String>);
    let project_transfer = create_rw_signal(None::<ProjectTransferProgress>);
    let project_export_prompt = create_rw_signal(None::<(String, String)>);
    // Destination of a link click waiting for the user's confirmation before
    // it reaches the system browser.
    let external_link_confirm = create_rw_signal(None::<String>);
    let app_shell_entering = create_rw_signal(false);
    let project_transition_epoch = Rc::new(Cell::new(0u64));
    let project_transition_target = Rc::new(RefCell::new(None::<String>));
    let project_open_gate = Rc::new(RefCell::new(ProjectOpenGate::default()));
    let model_menu_open = create_rw_signal(false);
    // Per-model effort flyout inside the model menu: (model id, left, top) in
    // viewport coordinates. Rendered `position: fixed` so the menu's scroll
    // box doesn't clip it.
    let effort_menu_for = create_rw_signal(None::<(String, f64, f64)>);
    // Shift the parent model menu left only while its right-side effort flyout
    // is open, keeping both surfaces adjacent and inside the viewport.
    let effort_menu_shift = create_rw_signal(0.0_f64);
    // The effort flyout is a sibling of the scrollable model menu; collapse it
    // whenever its parent picker closes.
    create_effect(move |_| {
        if !model_menu_open.get() {
            effort_menu_for.set(None);
        }
    });
    // Persist a reasoning-effort default onto the model profile itself
    // (Cursor-style per-model effort). Sessions without an explicit override
    // inherit the new default on their next turn.
    let apply_model_effort = Callback::new(move |(id, effort): (String, String)| {
        effort_menu_for.set(None);
        model_settings.apply_model_effort(id, effort);
    });
    let model_switch_confirm = create_rw_signal::<Option<(String, String, bool)>>(None);
    let status = create_rw_signal(String::new());
    let toggle_fast = Callback::new(move |_: ()| {
        if service_tier_busy.get_untracked() || busy.get_untracked() {
            return;
        }
        let Some(profile) = fast_profile.get_untracked() else {
            return;
        };
        let target_enabled = !fast_enabled.get_untracked();
        let profile_default = service_tier_enabled(&profile.service_tier);
        let override_value = if target_enabled == profile_default {
            None
        } else if target_enabled {
            Some("priority".to_string())
        } else {
            Some(String::new())
        };
        let Some(session_id) = active_session.get_untracked() else {
            pending_service_tier.set(override_value);
            return;
        };
        service_tier_busy.set(true);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sessionId": session_id.clone(),
                "serviceTier": override_value.clone(),
            }))
            .unwrap();
            match invoke_checked("set_session_service_tier", args).await {
                Ok(_) => {
                    session_service_tiers.update(|values| {
                        values.insert(session_id, override_value);
                    });
                }
                Err(error) => show_warning_toast(&localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                )),
            }
            service_tier_busy.set(false);
        });
    });
    let compaction_active = create_rw_signal(false);
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
    // Queue (#433): monotonic key for optimistic queued follow-ups, shared with the
    // backend queue item so edit/cancel/cut-in target the same row.
    let queue_seq = create_rw_signal(0u64);
    let side_chat_input = create_rw_signal(String::new());
    let side_chat_quotes = create_rw_signal::<Vec<ComposerQuote>>(vec![]);
    let side_chat_items = create_rw_signal::<Vec<SideChatItem>>(vec![]);
    let side_chat_busy = create_rw_signal(false);
    let side_chat_model_menu_open = create_rw_signal(false);
    // Side chat routes through this ACP Agent when set; None = the active model.
    let side_chat_acp_agent = create_rw_signal::<Option<String>>(None);
    // Owned here so the window-level Escape stack can close the confirm before
    // it falls through to closing the whole settings page.
    let delete_confirm = create_rw_signal(None::<DeleteConfirm>);
    let plugin_install_open = create_rw_signal(false);
    // Skills & plugins domain: pane state + install/enable handlers live in
    // `app_support::extensions`; `App` only wires them.
    let extensions = ExtensionsState::new(plugin_install_open, locale);
    let skills_list = extensions.skills_list;
    let skills_search = extensions.skills_search;
    let skills_msg = extensions.skills_msg;
    let skill_filter_tag = extensions.skill_filter_tag;
    let plugins_list = extensions.plugins_list;
    let plugins_msg = extensions.plugins_msg;
    let update_check_busy = create_rw_signal(false);
    let update_check_modal = create_rw_signal::<Option<UpdateCheckModal>>(None);
    // Newer release found by the silent auto-check → sidebar prompt card.
    let update_banner = create_rw_signal::<Option<AvailableUpdate>>(None);
    // Live web retrieval failed because the Chrome extension is disconnected.
    // Root-owned so Escape can dismiss it without first focusing the banner.
    let browser_offline_notice = create_rw_signal::<Option<BrowserOfflineNotice>>(None);
    // A connected extension can still be too old for this build. Keep this
    // separate from the offline verdict: live retrieval may work while newer
    // capabilities fail, and the remediation is an extension update.
    let browser_extension_status = create_rw_signal::<Option<BrowserExtensionStatus>>(None);
    let browser_extension_update_busy = create_rw_signal(false);
    let browser_extension_update_outcome = create_rw_signal::<Option<String>>(None);
    let browser_extension_update_error = create_rw_signal::<Option<String>>(None);
    let browser_extension_dismissed_version = create_rw_signal::<Option<String>>(None);
    let refresh_browser_extension_status = Callback::new(move |()| {
        spawn_local(async move {
            let value = invoke("browser_extension_status", JsValue::UNDEFINED).await;
            let Ok(status) = from_value::<BrowserExtensionStatus>(value) else {
                return;
            };
            if status.update_required {
                let target = status.bundled_version.clone().unwrap_or_default();
                if browser_extension_dismissed_version
                    .get_untracked()
                    .as_deref()
                    != Some(target.as_str())
                {
                    browser_extension_status.set(Some(status));
                }
            } else {
                browser_extension_status.set(None);
                browser_extension_update_outcome.set(None);
                browser_extension_update_error.set(None);
            }
        });
    });
    refresh_browser_extension_status.call(());
    {
        // A manual Reload tears down and recreates the service worker without a
        // Tauri event. Poll while the app is alive so successful reloads are
        // detected and the compatibility banner clears automatically.
        let ticks = Cell::new(0_u8);
        let startup_ticks = Cell::new(0_u8);
        let refresh = Closure::wrap(Box::new(move || {
            let tick = (ticks.get() + 1) % 5;
            ticks.set(tick);
            let startup_tick = startup_ticks.get();
            if startup_tick < 5 {
                startup_ticks.set(startup_tick + 1);
            }
            if startup_tick < 5 || browser_extension_status.get_untracked().is_some() || tick == 0 {
                refresh_browser_extension_status.call(());
            }
        }) as Box<dyn FnMut()>);
        let poll_window = web_sys::window();
        let interval = poll_window.as_ref().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    refresh.as_ref().unchecked_ref(),
                    2_000,
                )
                .ok()
        });
        on_cleanup(move || {
            if let (Some(window), Some(interval)) = (poll_window, interval) {
                window.clear_interval_with_handle(interval);
            }
            drop(refresh);
        });
    }
    // A transcript records the connection state at tool-call time. Recheck the
    // live bridge once whenever a notice appears so a reconnect does not leave
    // a stale banner on screen or revive one after a session reload.
    create_effect(move |_| {
        let Some(notice) = browser_offline_notice.get() else {
            return;
        };
        spawn_local(async move {
            let value = invoke("extension_connected", JsValue::UNDEFINED).await;
            if value.as_bool().unwrap_or(false) {
                set_browser_offline_notice(browser_offline_notice, &notice.frame_id, None);
            }
        });
    });
    let browser_tab_cleanup = create_rw_signal(None::<BrowserTabCleanupPrompt>);
    let browser_tab_cleanup_queue = create_rw_signal(Vec::<BrowserTabCleanupPrompt>::new());
    let browser_tab_cleanup_selected = create_rw_signal(HashSet::<(String, i64)>::new());
    let browser_tab_cleanup_busy = create_rw_signal(false);
    let browser_tab_cleanup_error = create_rw_signal(None::<String>);
    let browser_needs_human = create_rw_signal(None::<BrowserNeedsHumanPrompt>);
    let browser_needs_human_busy = create_rw_signal(false);
    let browser_needs_human_error = create_rw_signal(None::<String>);
    // "不再提醒更新" opt-out; loaded on startup, mirrored by the settings toggle.
    let update_check_enabled = create_rw_signal(true);
    // Set when a send fails because no API key is configured, so the status bar
    // can offer a one-click jump to Settings instead of a dead-end message.
    let needs_api_key = create_rw_signal(false);
    // A built-in model context-overflow error opens a root-owned recovery
    // choice. Keep it in the app Escape stack: it can appear without focus
    // moving into the dialog, immediately after an async AgentEvent::Error.
    let context_recovery_dialog = create_rw_signal::<Option<String>>(None);
    let context_recovery_busy = create_rw_signal(false);
    let context_recovery_error = create_rw_signal::<Option<String>>(None);
    let refresh_models = move || model_settings.refresh_models();
    // Tauri's native drag/drop event contains absolute paths (including
    // directories). Drops on a remote Files panel upload via scp; drops on
    // the composer stay as path references and must not go through `upload_file`.
    let native_drop_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let remote_target = native_drop_remote_target_value(payload.clone());
        let inside_composer = native_drop_in_composer(payload.clone());
        let value =
            serde_wasm_bindgen::from_value::<serde_json::Value>(payload).unwrap_or_default();
        let kind = value
            .get("kind")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(kind.as_str(), "enter" | "over" | "hover" | "hovered") {
            files_drag_over.set(remote_target.is_some());
            drag_over.set(inside_composer && remote_target.is_none());
            return;
        }
        if matches!(kind.as_str(), "leave" | "cancel" | "cancelled") {
            files_drag_over.set(false);
            drag_over.set(false);
            return;
        }
        if !matches!(kind.as_str(), "drop" | "dropped") {
            return;
        }
        files_drag_over.set(false);
        drag_over.set(false);
        let paths = value
            .get("paths")
            .and_then(|item| item.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if let Some((context_id, destination_dir)) = remote_target {
            if !paths.is_empty() {
                upload_to_remote_context(
                    context_id,
                    destination_dir,
                    Some(paths),
                    remote_file_uploading,
                    remote_files_refresh_tick,
                );
            }
            return;
        }
        if !inside_composer {
            return;
        }
        for path in paths {
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
    let refresh_specialists = move || model_settings.refresh_specialists();
    // Per-session specialist (persona) picker, gated to before the first message.
    let session_specialist = create_rw_signal::<Option<Specialist>>(None);
    let demos = create_rw_signal::<Vec<DemoInfo>>(vec![]);
    let command_palette_open = create_rw_signal(false);
    let action_palette_open = create_rw_signal(false);
    let (privacy_active_initial, privacy_projects_initial) = load_privacy_mode();
    let privacy_mode_active = create_rw_signal(privacy_active_initial);
    let privacy_hidden_project_ids = create_rw_signal(privacy_projects_initial);
    let privacy_mode_modal_open = create_rw_signal(false);
    // Top-nav project switcher dropdown + Project Settings modal.
    let show_proj_menu = create_rw_signal(false);
    let proj_list = create_rw_signal::<Vec<ProjectSummary>>(vec![]);
    let show_proj_settings = create_rw_signal(false);
    let proj_settings = create_rw_signal(ProjectSettings::default());
    let proj_settings_baseline = create_rw_signal(ProjectSettings::default());
    let proj_settings_busy = create_rw_signal(false);

    // Session history (left sidebar).
    let session_history_cursor = create_rw_signal::<Option<SessionCursor>>(None);
    let session_history_loading = create_rw_signal(false);
    let refresh_session_history = move || {
        refresh_sessions(
            sessions,
            pending_turns,
            running,
            session_history_cursor,
            active_session,
            exploration_frames,
        )
    };
    let folders = create_rw_signal::<Vec<FolderInfo>>(vec![]);
    let collapsed_folders = create_rw_signal::<HashSet<String>>(HashSet::new());
    let drag_session = create_rw_signal::<Option<String>>(None);
    let drop_target = create_rw_signal::<Option<String>>(None);
    let session_execution_contexts = create_rw_signal::<HashSet<String>>(HashSet::new());
    let default_execution_context = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
            if let Ok(value) = invoke_checked("get_session_model", args).await {
                if let Some(model_id) = value.as_string() {
                    if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                        session_model_ids.update(|models| {
                            models.insert(session_id.clone(), model_id);
                        });
                    }
                }
            }
        });
    });
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
            match invoke_checked("get_session_service_tier", args).await {
                Ok(value) => {
                    let tier = value.as_string();
                    session_service_tiers.update(|values| {
                        values.insert(session_id, tier);
                    });
                }
                Err(error) => {
                    session_service_tiers.update(|values| {
                        values.insert(session_id, None);
                    });
                    web_sys::console::warn_1(
                        &format!("get_session_service_tier failed: {:?}", error).into(),
                    );
                }
            }
        });
    });
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            if !session_execution_contexts.get_untracked().is_empty() {
                session_execution_contexts.set(HashSet::new());
            }
            return;
        };
        if !session_execution_contexts.get_untracked().is_empty() {
            session_execution_contexts.set(HashSet::new());
        }
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
    // `acp-session-state` only fires while a turn runs, so after an app restart
    // the mode controls would stay hidden until the user sent a message. Seed
    // the cached `availableModes` on open instead — `or_insert` so a session
    // that already has live state (including `currentModeId`) is left alone.
    create_effect(move |_| {
        let Some(session_id) = active_session.get() else {
            return;
        };
        if acp_session_modes.with_untracked(|all| all.contains_key(&session_id)) {
            return;
        }
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "frameId": session_id.clone() })).unwrap();
            let Ok(value) = invoke_checked("get_acp_session_state", args).await else {
                return;
            };
            let Ok(Some(modes)) =
                serde_wasm_bindgen::from_value::<Option<serde_json::Value>>(value)
            else {
                return;
            };
            acp_session_modes.update(|all| {
                all.entry(session_id).or_insert(modes);
            });
        });
    });

    refresh_session_history();
    refresh_folders(folders);
    refresh_explorations(explorations);
    create_effect(move |_| {
        let Some(project) = project_info.get() else {
            return;
        };
        let _ = project.id;
        refresh_explorations(explorations);
    });
    // `busy` is "the active session is currently streaming" — derived from the
    // per-session `running` set so it stays correct when the user switches
    // conversations or a background turn finishes.
    create_effect(move |_| {
        let r = running.get();
        let b = active_session
            .get()
            .map(|id| r.contains(&id))
            .unwrap_or(false);
        if b {
            cancel_saved_marks_apply();
        }
        busy.set(b);
    });
    // Settled transcript edits refresh projections automatically. While a turn
    // streams, stop subscribing to `items`; structural event handlers advance
    // the revision explicitly, and the final busy -> idle transition refreshes
    // once more with the completed assistant text.
    create_effect(move |_| {
        if busy.get() {
            return;
        }
        items.with(|_| ());
        transcript_projection_epoch.update(|revision| {
            *revision = revision.wrapping_add(1);
        });
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
    let pane_layout = PaneLayoutState::new();
    let show_sidebar = create_rw_signal(true);
    let sidebar_w = pane_layout.sidebar_w;
    let sidebar_dragging = pane_layout.sidebar_dragging;
    let show_right = create_rw_signal(false);
    let right_w = pane_layout.right_w;
    let dragging = pane_layout.right_dragging;
    let composer_h = pane_layout.composer_h;
    let composer_h_custom = pane_layout.composer_h_custom;
    let composer_dragging = pane_layout.composer_dragging;
    let terminal_sessions = create_rw_signal::<Vec<TerminalSessionSummary>>(vec![]);
    let active_terminal_id = create_rw_signal(None::<String>);
    let terminal_panel_open = create_rw_signal(false);
    let terminal_add_menu_open = create_rw_signal(false);
    let terminal_h = pane_layout.terminal_h;
    let terminal_dragging = pane_layout.terminal_dragging;

    // Artifacts and notebook cells are projections of the active transcript.
    let proto_cache = Rc::new(RefCell::new(ProtoCache::new()));
    let artifacts_all = create_memo(move |_| {
        let _ = active_session.get();
        let _ = transcript_projection_epoch.get();
        items.with_untracked(|list| {
            collect_artifacts(list, locale.get(), &mut proto_cache.borrow_mut())
        })
    });
    // File-backed artifacts are scraped from chat text, so a file that was
    // renamed or overwritten still lingers and 404s on click (#41). Ask the
    // backend which referenced files are gone and drop them from the list.
    let missing_paths = create_rw_signal(std::collections::HashSet::<String>::new());
    let artifact_file_paths = create_memo(move |_| {
        artifacts_all.with(|artifacts| {
            artifacts
                .iter()
                .filter_map(|artifact| match &artifact.data {
                    PreviewData::File { path, .. } => Some(path.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    });
    create_effect(move |_| {
        let paths = artifact_file_paths.get();
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
    // Artifacts the backend registered in the database rather than mentioned in
    // chat — harvested run outputs, delegated-agent results, MCP-bridge writes,
    // uploads. The transcript scan above is blind to all of them, so the panel
    // asks `list_artifacts` for the session too. Refetched when the session
    // changes and at each turn boundary, which is when new rows appear.
    let db_artifacts = create_rw_signal::<Vec<ArtifactInfo>>(vec![]);
    create_effect(move |_| {
        let _ = busy.get();
        let Some(session_id) = active_session.get() else {
            db_artifacts.set(vec![]);
            return;
        };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
            let value = invoke("list_artifacts", arg).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(value) {
                db_artifacts.set(rows);
            }
        });
    });
    let artifacts = create_memo(move |_| {
        let miss = missing_paths.get();
        let root = project_info
            .get()
            .map(|project| project.root)
            .unwrap_or_default();
        current_artifacts(&artifacts_all.get(), &db_artifacts.get(), &root, &miss)
    });
    let notebook_cache = Rc::new(RefCell::new(NotebookCache::new()));
    let notebook_cells = create_memo(move |_| {
        let _ = active_session.get();
        let _ = transcript_projection_epoch.get();
        items.with_untracked(|list| collect_notebook_cells(list, &mut notebook_cache.borrow_mut()))
    });
    let provenance_rows = create_memo(move |_| {
        let _ = active_session.get();
        items.with(|rows| {
            rows.iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    matches!(item, ChatItem::Tool { .. }).then(|| (index, item.fingerprint()))
                })
                .collect::<Vec<_>>()
        })
    });
    let artifact_count = create_memo(move |_| artifacts.with(Vec::len));
    let notebook_count = create_memo(move |_| notebook_cells.with(Vec::len));
    let provenance_count = create_memo(move |_| provenance_rows.with(Vec::len));
    let highlight_count = create_memo(move |_| {
        let session = active_session.get();
        library_items.with(|items| session_highlight_count(session, items))
    });
    let monitored_run_ids = create_memo(move |_| {
        let _ = active_session.get();
        let _ = transcript_projection_epoch.get();
        items.with_untracked(|rows| {
            rows.iter()
                .filter_map(|item| match item {
                    ChatItem::Tool { name, input, .. } if is_run_monitor_tool(name) => {
                        Some(input.trim().to_string())
                    }
                    _ => None,
                })
                .collect::<HashSet<_>>()
        })
    });
    let automatic_session_runs = create_memo(move |_| {
        let Some(frame_id) = active_session.get() else {
            return Vec::new();
        };
        let monitored = monitored_run_ids.get();
        let now = js_sys::Date::now() as i64 / 1000;
        let mut runs = run_records.with(|runs| {
            runs.iter()
                .filter(|run| run.frame_id.as_deref() == Some(frame_id.as_str()))
                .filter(|run| !monitored.contains(&run.id))
                .filter(|run| {
                    matches!(run.status.as_str(), "submitted" | "running" | "cancelling")
                        || run
                            .ended_at
                            .is_some_and(|ended| now.saturating_sub(ended) <= 60)
                })
                .map(|run| (run.id.clone(), run.created_at))
                .collect::<Vec<_>>()
        });
        runs.sort_by_key(|(_, created_at)| *created_at);
        runs
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
    // Project-scoped, and the agent writes to it mid-turn — refetched when the
    // sidebar modal is opened rather than kept live.
    let research_graph = create_rw_signal(ResearchGraph::default());
    let show_research_graph = create_rw_signal(false);
    let show_publication_workspace = create_rw_signal(false);
    let publication_binding_source = create_rw_signal::<Option<PublicationEvidenceSource>>(None);
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
    let workflow_studio_state = AgentPanelState::new(active_session);
    refresh_agent_resources(workflow_studio_state, specialists);
    let file_source = create_rw_signal("local".to_string());
    let file_query = create_rw_signal(String::new());
    let file_cwd = create_rw_signal(".".to_string());
    let file_sort = create_rw_signal(load_view_pref(
        FILE_SORT_PREF,
        FILE_SORT_NAME,
        &[FILE_SORT_NAME, FILE_SORT_SIZE, FILE_SORT_MODIFIED],
    ));
    let file_sort_menu_open = create_rw_signal(false);
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let file_search_hits = create_rw_signal::<Vec<FileSearchHit>>(vec![]);
    let selecting_workspace_entries = create_rw_signal(false);
    let selected_workspace_paths = create_rw_signal::<HashSet<String>>(HashSet::new());
    let remote_file_cwd = create_rw_signal("~".to_string());
    let remote_file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let remote_file_loading = create_rw_signal(false);
    let remote_file_error = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        if remote_files_refresh_tick.get() == 0 {
            return;
        }
        refresh_active_file_dir(
            file_source,
            file_cwd,
            file_entries,
            remote_file_cwd,
            remote_file_entries,
            remote_file_loading,
            remote_file_error,
        );
    });
    let center_files = create_rw_signal::<Vec<CenterFileTab>>(vec![]);
    let center_file = create_rw_signal::<Option<String>>(None);
    let snapshot_workspace_path = create_memo(move |_| {
        let path = center_file.get()?;
        snapshot_workspace_source(
            &path,
            &items.get(),
            project_info
                .get()
                .as_ref()
                .map(|project| project.root.as_str()),
        )
    });
    // Live MCP Apps use the same center-tab surface as files, but their HTML,
    // tool input, and result stay in a separate instance map so in-memory tab
    // snapshots do not repeatedly clone multi-megabyte payloads. The backend
    // persists the presentation event once so a reopened session can restore it.
    let mcp_apps = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    // Successful edit/write tool calls bump the matching tab's revision. The
    // preview subtree is keyed by this value and re-reads the saved file.
    let center_file_revisions = create_rw_signal::<HashMap<String, u64>>(HashMap::new());
    let center_file_open = create_memo(move |_| !demo_mode.get() && center_file.get().is_some());
    // Split view: keep the main conversation beside the open document instead of
    // hiding it. Same session, same history — only the layout moves.
    let center_split = create_rw_signal(false);
    let center_split_on = create_memo(move |_| center_split.get() && center_file_open.get());
    let center_chat_w = pane_layout.center_chat_w;
    let center_split_dragging = pane_layout.center_split_dragging;
    // Runtime binding for R/Python previews: file path -> execution context id.
    // The language comes from the extension, so the context is the whole binding.
    // In-memory on purpose — a runtime dies with the app, so a binding that
    // outlived one would point at a process that no longer exists.
    let center_runtime_binding = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    let center_console = create_rw_signal::<RuntimeConsoles>(RuntimeConsoles::new());
    let center_plots = create_rw_signal::<RuntimePlots>(RuntimePlots::new());
    let center_run_busy = create_rw_signal::<Option<String>>(None);
    let center_runtime_panel = create_rw_signal(false);
    // Unsaved editor drafts per file path. Kept outside the editor component so
    // a preview remount (agent FileChanged bumps the revision) cannot drop what
    // the user typed.
    let center_editor_drafts = create_rw_signal::<HashMap<String, String>>(HashMap::new());
    // RStudio-quadrant geometry: right column width and bottom row height as
    // percentages of the workbench, adjusted by dragging the pane dividers.
    let center_runtime_right_w = create_rw_signal(34.0_f64);
    let center_runtime_bottom_h = create_rw_signal(32.0_f64);
    let center_runtime_col_dragging = create_rw_signal(false);
    let center_runtime_row_dragging = create_rw_signal(false);
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
    let side_chat_by_session =
        create_rw_signal::<HashMap<String, Vec<SideChatItem>>>(HashMap::new());
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
        side_chat_quotes.set(vec![]);
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
    let caps = create_rw_signal::<Option<Capabilities>>(None);
    let bootstrap = create_rw_signal::<Option<BootstrapStatus>>(None);
    let show_onboarding = create_rw_signal(false);
    let onboard_step = create_rw_signal(0usize);
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

    let on_artifact_select = Callback::new(move |idx: usize| {
        let arts = artifacts.get();
        if let Some(a) = arts.get(idx) {
            if let PreviewData::File { path, kind } = &a.data {
                modal_artifact.set(Some((path.clone(), a.name.clone(), kind.clone())));
            } else {
                ensure_right_tab(RightTab::Artifacts, show_right, open_right_tabs, right_tab);
                sel_artifact.set(idx);
                show_art_preview.set(true);
            }
        }
    });

    let on_file_link = Callback::new(move |resource: ModalArtifact| {
        modal_artifact.set(Some(resource));
    });

    // Inline @ artifact, # session, and / skill or Workflow pickers all share one cursor
    // model and one chip list. Uploads remain separate because they have async
    // progress/error state; selected catalog items are already durable records.
    let composer_references = create_rw_signal::<Vec<ComposerReferenceChip>>(vec![]);
    // Quoted selections retain their source path. The persisted message still
    // carries ordinary text, but the agent now knows which workspace file a
    // "change this" request must edit.
    let composer_quotes = create_rw_signal::<Vec<ComposerQuote>>(vec![]);
    let close_scratch = Callback::new(move |_: ()| {
        spawn_local(async move {
            let _ = invoke("close_scratch_chat", JsValue::UNDEFINED).await;
            scratch_open.set(false);
            items.set(vec![]);
            active_session.set(None);
            show_right.set(false);
            center_file.set(None);
        });
    });
    let open_scratch = Callback::new(move |_: ()| {
        if demo_mode.get_untracked() {
            return;
        }
        command_palette_open.set(false);
        action_palette_open.set(false);
        spawn_local(async move {
            let v = invoke("start_scratch_chat", JsValue::UNDEFINED).await;
            let Ok(info) = serde_wasm_bindgen::from_value::<ScratchChatInfo>(v) else {
                status.set(send_failed(locale.get(), ""));
                return;
            };
            scratch_open.set(true);
            active_session.set(Some(info.session_id));
            items.set(vec![]);
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            show_sidebar.set(false);
            show_right.set(false);
            center_file.set(None);
            focus_composer();
        });
    });
    // Floating action popup over a text selection: (text, source file path, x, y).
    // The source path is Some only when the selection is inside a file preview —
    // it gates the "annotate" action and names the review sidecar.
    let selection_popup = create_rw_signal::<Option<(String, Option<String>, i32, i32)>>(None);
    let quick_actions = create_rw_signal::<Vec<QuickAction>>(vec![]);
    let workflow_templates = create_rw_signal::<Vec<WorkflowTemplate>>(vec![]);
    let selected_workflow_template = create_rw_signal::<Option<String>>(None);
    let refresh_quick_actions = move || {
        spawn_local(async move {
            if let Ok(value) = invoke_checked("list_quick_actions", JsValue::UNDEFINED).await {
                if let Ok(mut actions) = serde_wasm_bindgen::from_value::<Vec<QuickAction>>(value) {
                    actions.sort_by_key(|action| action.sort_order);
                    quick_actions.set(actions);
                }
            }
        });
    };
    let refresh_workflow_templates = move || {
        spawn_local(async move {
            if let Ok(value) = invoke_checked("list_workflow_templates", JsValue::UNDEFINED).await {
                if let Ok(templates) =
                    serde_wasm_bindgen::from_value::<Vec<WorkflowTemplate>>(value)
                {
                    workflow_templates.set(templates);
                }
            }
        });
    };
    refresh_quick_actions();
    refresh_workflow_templates();
    // Session mode flags backing the agent-menu toggles and the /plan and
    // /permission commands. Declared with the composer picker state so the
    // picker, the slash runner, and the agent menu all share one copy.
    let local_plan_mode = create_rw_signal::<Option<bool>>(Some(false));
    let plan_mode_busy = create_rw_signal(false);
    let full_permission_enabled = create_rw_signal(false);
    let full_permission_busy = create_rw_signal(false);
    let ui_confirm = create_rw_signal::<Option<UiConfirm>>(None);
    // `/share` preview dialog: Some(rows) while open, None when closed.
    let share_draft = create_rw_signal::<Option<Vec<ShareMessage>>>(None);
    let open_share = Callback::new(move |()| {
        let rows = items.with_untracked(|list| share_messages(list));
        if rows.is_empty() {
            status.set(t(locale.get_untracked(), "composer.cmd_share_empty"));
        } else {
            share_draft.set(Some(rows));
        }
    });
    // One flag, two backends, exactly like the composer toggle: a built-in
    // session reads its own plan flag, an ACP-bound one reads the agent's
    // mode. `None` = the session is ACP-bound, so the toggle drives the ACP
    // mode picker instead of this flag. A session-less composer counts as
    // built-in.
    let plan_mode_active = Signal::derive(move || {
        if let Some(enabled) = local_plan_mode.get() {
            return enabled;
        }
        let Some(session_id) = active_session.get() else {
            return false;
        };
        acp_session_modes
            .with(|all| acp_current_mode_id(all.get(&session_id)).is_some_and(is_plan_mode_id))
    });
    // Agents without a plan mode still push plan updates — a Claude Code todo
    // list arrives as one. The card renders, but there is no mode to approve
    // out of, so it is badged as a read-only compatibility plan instead.
    // Built-in sessions always have one to approve out of.
    let plan_compat = Signal::derive(move || {
        if local_plan_mode.get().is_some() {
            return false;
        }
        let Some(session_id) = active_session.get() else {
            return true;
        };
        acp_session_modes.with(|all| plan_mode_pair(all.get(&session_id)).is_none())
    });
    // Single entry point behind the agent-menu toggle and /plan: ACP-bound
    // sessions switch the agent's own plan/default mode pair, built-in ones
    // flip the local flag (creating a session when the composer has none).
    let set_plan_first = Callback::new(move |enabled: bool| {
        let loc = locale.get_untracked();
        let session_id = active_session.get_untracked();
        let acp_pair = match (local_plan_mode.get_untracked(), &session_id) {
            (None, Some(id)) => acp_session_modes.with_untracked(|all| plan_mode_pair(all.get(id))),
            _ => None,
        };
        let Some((plan_mode, exit_mode)) = acp_pair else {
            local_plan_mode.set(Some(enabled));
            plan_mode_busy.set(true);
            spawn_local(async move {
                let (session_id, created_session) = match active_session.get_untracked() {
                    Some(session_id) => (session_id, false),
                    None if enabled => {
                        let Some(session_id) =
                            invoke("new_session", JsValue::UNDEFINED).await.as_string()
                        else {
                            local_plan_mode.set(Some(false));
                            plan_mode_busy.set(false);
                            return;
                        };
                        (session_id, true)
                    }
                    None => {
                        local_plan_mode.set(Some(false));
                        plan_mode_busy.set(false);
                        return;
                    }
                };
                let args = to_value(&serde_json::json!({
                    "sessionId": session_id.clone(),
                    "enabled": enabled,
                }))
                .unwrap();
                let saved = invoke_checked("set_session_plan_mode", args)
                    .await
                    .ok()
                    .and_then(|value| value.as_bool());
                if created_session {
                    active_session.set(Some(session_id.clone()));
                    items.set(vec![]);
                    refresh_session_history();
                }
                if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                    local_plan_mode.set(Some(saved.unwrap_or(!enabled)));
                    plan_mode_busy.set(false);
                }
                if saved.is_some() {
                    show_toast(&t(
                        loc,
                        if enabled {
                            "plan.enabled"
                        } else {
                            "plan.default_enabled"
                        },
                    ));
                }
            });
            return;
        };
        let target = if enabled { plan_mode } else { exit_mode };
        let Some(session_id) = session_id else {
            return;
        };
        spawn_local(async move {
            if apply_acp_mode(acp_session_modes, session_id, target).await {
                show_toast(&t(
                    loc,
                    if enabled {
                        "plan.enabled"
                    } else {
                        "plan.default_enabled"
                    },
                ));
            }
        });
    });
    // Turning Full Permission off needs no warning; enabling always goes
    // through UiConfirm::EnableFullPermission. Shared by the agent-menu
    // toggle and /permission ask.
    let disable_full_permission = Callback::new(move |_| {
        let Some(session_id) = active_session.get_untracked() else {
            full_permission_enabled.set(false);
            return;
        };
        full_permission_enabled.set(false);
        full_permission_busy.set(true);
        let loc = locale.get_untracked();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sessionId": session_id.clone(),
                "enabled": false,
            }))
            .unwrap();
            let disabled = invoke_checked("set_session_full_permission", args)
                .await
                .ok()
                .and_then(|value| value.as_bool())
                == Some(false);
            if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                full_permission_enabled.set(!disabled);
            }
            full_permission_busy.set(false);
            if disabled {
                show_toast(&t(loc, "full_permission.disabled"));
            }
        });
    });
    let picker_mode = create_rw_signal(None::<ComposerPickerMode>);
    let picker_token_range = create_rw_signal(None::<(usize, usize)>);
    let picker_query = create_rw_signal(String::new());
    let picker_index = create_rw_signal(0usize);
    // Set once the command action callbacks exist (below `request_turn_memory`).
    // The send path and the picker both route built-in slash commands through
    // it, so a typed command behaves exactly like a picked one.
    let slash_command_runner = create_rw_signal(None::<Callback<String, bool>>);
    let picker_artifacts = create_rw_signal(Vec::<ArtifactInfo>::new());
    let picker_sessions = create_rw_signal(Vec::<SessionSearchInfo>::new());
    // Declared up here (not with the other context-view signals) so the
    // composer @ menu can offer servers and runtimes alongside artifacts.
    let execution_contexts = create_rw_signal::<Vec<ExecutionContext>>(vec![]);
    // Also up here: the agent event handler below refreshes runtime status and
    // the open memory environment after each finished python/r tool call.
    let runtime_infos = create_rw_signal::<Vec<RuntimeInfo>>(vec![]);
    let runtime_environment = create_rw_signal(None::<RuntimeSlot>);
    let runtime_object_states =
        create_rw_signal::<HashMap<String, RuntimeObjectState>>(HashMap::new());
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
            ComposerPickerMode::Skill => {
                if skills_list.get_untracked().is_empty() {
                    spawn_local(async move {
                        let v = invoke("list_skills", JsValue::UNDEFINED).await;
                        if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                            skills_list.set(rows);
                        }
                    });
                }
                if workflow_templates.get_untracked().is_empty() {
                    refresh_workflow_templates();
                }
            }
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
                // Built-in commands the shell itself intercepts, hidden when
                // the current session cannot run them (same conditions as the
                // buttons they mirror: /compact and /rewind are local-session
                // only, /fork needs a branchable mainline, /review and
                // /remember need at least one turn, /share needs a shareable
                // user/assistant/thinking row).
                let acp = active_acp_agent_id.get().is_some();
                let has_items = session_has_items.get();
                let branchable =
                    active_branch_state.get().is_none() && !active_is_exploration.get();
                let available = |name: &str| match name {
                    "compact" => !acp,
                    "rewind" => !acp && has_items,
                    "fork" => !acp && branchable,
                    "review" | "remember" => has_items,
                    "share" => can_share.get(),
                    "context" => active_context_usage.get().is_some(),
                    // Hidden where the agent has no plan mode to switch into
                    // (same condition that drops the agent-menu toggle row).
                    "plan" => !plan_compat.get(),
                    _ => true,
                };
                let loc = locale.get();
                let mut commands: Vec<ComposerPickerItem> = slash_command_matches(&query)
                    .into_iter()
                    .filter(|name| available(name))
                    .map(|name| {
                        let key = format!("composer.cmd_{name}_sub");
                        ComposerPickerItem::Command {
                            name: name.to_string(),
                            description: t(loc, &key),
                        }
                    })
                    .collect();
                let mut workflows = workflow_templates
                    .get()
                    .into_iter()
                    .filter(|workflow| {
                        workflow.name.to_lowercase().contains(&query)
                            || workflow.description.to_lowercase().contains(&query)
                            || workflow.proposal.goal.to_lowercase().contains(&query)
                    })
                    .collect::<Vec<_>>();
                workflows.sort_by_key(|workflow| (!workflow.builtin, workflow.name.clone()));
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
                commands.extend(
                    workflows
                        .into_iter()
                        .map(ComposerPickerItem::Workflow)
                        .chain(rows.into_iter().map(ComposerPickerItem::Skill)),
                );
                commands
            }
            None => vec![],
        }
    });
    let select_picker_item = Callback::new(move |i: usize| {
        let Some(item) = picker_items.get().get(i).cloned() else {
            return;
        };
        // Built-in commands run in the shell, not the model: payload commands
        // (/compact, /fork …, /btw …) fill the composer, action commands run
        // immediately. Neither attaches a reference chip.
        if let ComposerPickerItem::Command { name, .. } = item {
            let current = input.get_untracked();
            let stripped = picker_token_range.get_untracked().and_then(|(start, end)| {
                (start <= end
                    && end <= current.len()
                    && current.is_char_boundary(start)
                    && current.is_char_boundary(end))
                .then(|| (start, format!("{}{}", &current[..start], &current[end..])))
            });
            picker_mode.set(None);
            if slash_command_fills_text(&name) {
                // Payload commands keep a trailing space so the trigger token
                // is closed and the picker does not reopen while typing it.
                let filled = if name == "compact" {
                    format!("/{name}")
                } else {
                    format!("/{name} ")
                };
                let caret = stripped.map(|(start, rest)| {
                    let mut next = rest.clone();
                    next.insert_str(start, &filled);
                    input.set(next);
                    rest[..start].encode_utf16().count() as u32 + filled.len() as u32
                });
                if let Some(caret) = caret {
                    focus_composer_at(caret);
                } else {
                    focus_composer();
                }
            } else {
                let remaining = stripped.map(|(start, rest)| {
                    input.set(rest.clone());
                    (start, rest)
                });
                if let Some(runner) = slash_command_runner.get_untracked() {
                    runner.call(format!("/{name}"));
                }
                // The runner clears the composer; restore any draft text that
                // preceded the command token.
                match remaining {
                    Some((start, rest)) if !rest.trim().is_empty() => {
                        let caret = rest[..start].encode_utf16().count() as u32;
                        input.set(rest);
                        focus_composer_at(caret);
                    }
                    _ => focus_composer(),
                }
            }
            return;
        }
        let reference = match item {
            // Commands were handled above; they never become reference chips.
            ComposerPickerItem::Command { .. } => return,
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
            ComposerPickerItem::Workflow(workflow) => ComposerReferenceChip::Workflow {
                id: workflow.id,
                name: workflow.name,
            },
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
        let current = input.get_untracked();
        let caret = picker_token_range.get_untracked().and_then(|(start, end)| {
            (start <= end
                && end <= current.len()
                && current.is_char_boundary(start)
                && current.is_char_boundary(end))
            .then(|| {
                let caret = current[..start].encode_utf16().count() as u32;
                input.set(format!("{}{}", &current[..start], &current[end..]));
                caret
            })
        });
        composer_references.update(|items| {
            if !items.iter().any(|item| item.key() == reference.key()) {
                items.push(reference);
            }
        });
        picker_mode.set(None);
        if let Some(caret) = caret {
            focus_composer_at(caret);
        } else {
            focus_composer();
        }
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
            sync_actions_available.set(project_sync_backend_configured(&cfg));
            let loc = Locale::from_code(&cfg.locale);
            locale.set(loc);
            set_document_lang(loc);
            settings.set(cfg);
        }
        let appearance = invoke("get_appearance_prefs", JsValue::UNDEFINED).await;
        if let Ok(view) = serde_wasm_bindgen::from_value::<AppearancePrefsView>(appearance) {
            if view.saved {
                theme_mode.set(view.prefs.theme);
                light_palette.set(view.prefs.light_palette);
                dark_palette.set(view.prefs.dark_palette);
                ui_font_size.set(view.prefs.ui_font_size);
                code_font_size.set(view.prefs.code_font_size);
                ui_font_family.set(view.prefs.ui_font_family);
                code_font_family.set(view.prefs.code_font_family);
                selection_popup_enabled.set(view.prefs.selection_popup_enabled);
                send_with_modifier.set(view.prefs.send_with_modifier);
                custom_css.set(view.prefs.custom_css);
            }
        }
        appearance_hydrated.set(true);
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

    // Wire the agent event stream once. Every event carries the session frame
    // id; route transcript mutations to `items` (active session) or the
    // `transcripts` cache (background session) so parallel conversations don't
    // interleave in the view.
    let items_cb = items;
    let active_cb = active_session;
    let transcripts_cb = transcripts;
    let browser_offline_cb = browser_offline_notice;
    let running_cb = running;
    let pending_cb = pending_turns;
    let approval_cb = approval_pending;
    let conversation_outlines_cb = conversation_outlines;
    let transcript_projection_epoch_cb = transcript_projection_epoch;
    let trajectory_live_cb = trajectory_live;
    let trajectory_open_cb = trajectory_open;
    let fetch_trajectory_cb = fetch_trajectory.clone();
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
    let compaction_active_cb = compaction_active;
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
        move |(frame_id, payload, replace): (String, serde_json::Value, bool)| {
            let instance_id = mcp_app_instance_id(&frame_id, &payload);
            if !replace && mcp_apps_cb.with_untracked(|apps| apps.contains_key(&instance_id)) {
                return;
            }
            let Ok(payload_json) = serde_json::to_string(&payload) else {
                return;
            };
            let title = mcp_app_title(&payload);
            mcp_apps_cb.update(|apps| {
                apps.insert(instance_id.clone(), payload_json);
            });
            center_files_cb.update(|files| {
                if !files.iter().any(|file| file.path == instance_id) {
                    files.push(CenterFileTab::new(
                        instance_id.clone(),
                        title,
                        "mcp_app".into(),
                    ));
                }
            });
            center_file_cb.set(Some(instance_id));
            center_split_cb.set(true);
            show_right_cb.set(false);
        },
    );
    let project_info_cb = project_info;
    // Roll the active conversation back to just before user message `ui_index`
    // and return its text to the composer. Shared by message-edit and by the
    // automatic rollback when a turn dies on a model that cannot take images.
    let rewind_to_user_item = move |ui_index: usize| {
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
            if invoke_checked("rewind_session", arg).await.is_ok() {
                if let Some(id) = sid.filter(|id| !id.is_empty()) {
                    let loaded = invoke(
                        "load_session",
                        to_value(&serde_json::json!({ "id": id.clone() })).unwrap(),
                    )
                    .await;
                    if let Ok(page) = serde_wasm_bindgen::from_value::<LoadedSessionPage>(loaded) {
                        conversation_branches.update(|branches| {
                            branches.insert(id.clone(), page.branches.clone());
                        });
                        active_branch_state.set(page.branch_state.clone());
                        let mut chats = page
                            .items
                            .into_iter()
                            .map(LoadedItem::into_chat)
                            .collect::<Vec<_>>();
                        settle_question_cards(&mut chats);
                        items.set(chats);
                    }
                    refresh_session_history();
                }
            }
        });
    };
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
        let finish_compaction = |frame_id: &str| {
            if active_cb.get_untracked().as_deref() == Some(frame_id) {
                compaction_active_cb.set(false);
            }
        };
        let refresh_transcript_projections = |frame_id: &str| {
            if active_cb.get_untracked().as_deref() == Some(frame_id) {
                transcript_projection_epoch_cb.update(|revision| {
                    *revision = revision.wrapping_add(1);
                });
            }
        };
        // Lightweight trajectory cells for the in-flight turn. Kept minimal on
        // purpose: the Done/Error refetch replaces them with exact backend
        // data, so these only bridge the live view while a turn runs.
        let trajectory_push = |frame_id: &str, cell: TrajectoryCellDto| {
            if active_cb.get_untracked().as_deref() == Some(frame_id) {
                trajectory_live_cb.update(|cells| cells.push(cell));
            }
        };
        let trajectory_settle = |frame_id: &str| {
            if active_cb.get_untracked().as_deref() == Some(frame_id) {
                trajectory_live_cb.set(vec![]);
                if trajectory_open_cb.get_untracked() {
                    fetch_trajectory_cb(frame_id.to_string());
                }
            }
        };
        match ev {
            AgentEvent::CompactionStarted { frame_id, .. } => {
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    compaction_active_cb.set(true);
                }
            }
            AgentEvent::User { frame_id, text } => {
                dismiss_follow_up_questions(follow_up_questions, follow_up_generation, &frame_id);
                // The banner judges the answer on screen; a new turn has none yet.
                set_browser_offline_notice(browser_offline_cb, &frame_id, None);
                set_pet_activity(&frame_id, "running");
                flush_now();
                let outline_text = text.clone();
                let live_user_summary: String = text
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect();
                let model = session_model_label(
                    &models_cb.get_untracked(),
                    &session_models_cb.get_untracked(),
                    Some(&frame_id),
                );
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    start_user_turn(v, text, model.clone());
                });
                conversation_outlines_cb.update(|outlines| {
                    let outline = outlines.entry(frame_id.clone()).or_default();
                    let user_index = outline
                        .last()
                        .map_or(0, |entry| entry.user_index.saturating_add(1));
                    outline.push(SessionOutlineItem {
                        user_index,
                        seq: None,
                        text: outline_text,
                        sent_at: Some(now_secs()),
                        response_at: None,
                    });
                });
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    // A user message opens a fresh turn: reset the live cells.
                    trajectory_live_cb.set(vec![TrajectoryCellDto {
                        kind: "user".to_string(),
                        summary: live_user_summary,
                        ts: Some(now_ms() as i64),
                        ..Default::default()
                    }]);
                }
                refresh_transcript_projections(&frame_id);
            }
            AgentEvent::MessageBoundary { frame_id, seq } => {
                let needs_seq = conversation_outlines_cb.with_untracked(|outlines| {
                    outlines
                        .get(&frame_id)
                        .and_then(|outline| outline.last())
                        .is_some_and(|entry| entry.seq.is_none())
                });
                if needs_seq {
                    conversation_outlines_cb.update(|outlines| {
                        if let Some(entry) = outlines
                            .get_mut(&frame_id)
                            .and_then(|outline| outline.last_mut())
                        {
                            entry.seq = Some(seq);
                        }
                    });
                }
            }
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
                finish_compaction(&frame_id);
                set_pet_activity(&frame_id, "running");
                let needs_response_time = conversation_outlines_cb.with_untracked(|outlines| {
                    outlines
                        .get(&frame_id)
                        .and_then(|outline| outline.last())
                        .is_some_and(|entry| entry.response_at.is_none())
                });
                if needs_response_time {
                    conversation_outlines_cb.update(|outlines| {
                        if let Some(entry) = outlines
                            .get_mut(&frame_id)
                            .and_then(|outline| outline.last_mut())
                        {
                            entry.response_at = Some(now_secs());
                        }
                    });
                }
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    // One live assistant cell per contiguous text run, seeded
                    // from the first delta; later deltas only stream into the
                    // chat bubble.
                    let seed_assistant = trajectory_live_cb.with_untracked(|cells| {
                        cells.last().is_none_or(|cell| cell.kind != "assistant")
                    });
                    if seed_assistant {
                        trajectory_push(
                            &frame_id,
                            TrajectoryCellDto {
                                kind: "assistant".to_string(),
                                summary: delta.trim().chars().take(160).collect(),
                                ts: Some(now_ms() as i64),
                                ..Default::default()
                            },
                        );
                    }
                }
                queue(frame_id, PendingDelta::Text(delta));
            }
            AgentEvent::Reasoning { frame_id, delta } => {
                finish_compaction(&frame_id);
                set_pet_activity(&frame_id, "running");
                queue(frame_id, PendingDelta::Reasoning(delta));
            }
            AgentEvent::ToolCall {
                frame_id,
                name,
                preview,
            } => {
                finish_compaction(&frame_id);
                set_pet_activity(&frame_id, "review");
                flush_now();
                trajectory_push(
                    &frame_id,
                    TrajectoryCellDto {
                        kind: "tool".to_string(),
                        summary: if preview.is_empty() {
                            name.clone()
                        } else {
                            format!("{name} · {preview}")
                        },
                        ts: Some(now_ms() as i64),
                        ..Default::default()
                    },
                );
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    // The plan and question tools have no call card: their
                    // results carry the whole body, and that lands as a card.
                    if name == PROPOSE_PLAN_TOOL || name == ASK_USER_TOOL {
                        return;
                    }
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
                });
                refresh_transcript_projections(&frame_id);
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    schedule_chat_follow();
                }
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
                    // A submitted plan renders as the plan card, the same shape
                    // the ACP path streams and the reload path rebuilds. A
                    // refused call (bad entries) stays an ordinary tool row so
                    // the agent's error is visible.
                    if name == PROPOSE_PLAN_TOOL && ok {
                        if let Ok(payload) = serde_json::from_str(&content) {
                            let mut card = parse_plan_card(&payload);
                            card.state = PlanState::Streaming;
                            upsert_plan_card(v, card);
                            return;
                        }
                    }
                    // A submitted question renders as the question card; each
                    // call is its own question, so append instead of upsert. A
                    // refused call stays an ordinary tool row.
                    if name == ASK_USER_TOOL && ok {
                        if let Ok(payload) = serde_json::from_str(&content) {
                            let idx = process_item_insert_index(v);
                            v.insert(idx, ChatItem::Question(parse_question_card(&payload)));
                            return;
                        }
                    }
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
                });
                // The tool rows of the running turn are the whole verdict, so
                // recompute from them instead of latching one event: a refused
                // attempt after a successful scan must not claim the answer has
                // no live results (#921).
                if is_browser_retrieval_tool(&name) {
                    let notice = if active_cb.get_untracked().as_deref() == Some(frame_id.as_str())
                    {
                        items_cb.with_untracked(|rows| {
                            browser_offline_notice_from_items(&frame_id, rows)
                        })
                    } else {
                        transcripts_cb.with_untracked(|cache| {
                            cache
                                .get(&frame_id)
                                .and_then(|rows| browser_offline_notice_from_items(&frame_id, rows))
                        })
                    };
                    set_browser_offline_notice(browser_offline_cb, &frame_id, notice);
                }
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    trajectory_live_cb.update(|cells| {
                        if let Some(cell) = cells
                            .iter_mut()
                            .rev()
                            .find(|cell| cell.kind == "tool" && cell.ok.is_none())
                        {
                            cell.ok = Some(ok);
                            cell.is_error = !ok;
                            if event_ms > 0 {
                                cell.duration_ms = Some(event_ms as i64);
                            }
                        }
                    });
                }
                refresh_transcript_projections(&frame_id);
                if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                    schedule_chat_follow();
                }
                // A finished python/r cell changed interpreter state. Refresh
                // the runtime status chips and, when the memory environment
                // for that language is open, re-inspect it so the variable
                // table follows the agent without a manual sync click.
                if matches!(name.as_str(), "python" | "r")
                    && active_cb.get_untracked().as_deref() == Some(frame_id.as_str())
                {
                    refresh_runtime_environment_after_tool(
                        name.clone(),
                        runtime_environment,
                        runtime_object_states,
                        runtime_infos,
                        locale,
                    );
                }
            }
            AgentEvent::ToolPresentation {
                frame_id,
                presentation_id: _,
                presentation_kind,
                payload,
            } => {
                if presentation_kind == "app_prefs" {
                    apply_prefs_patch(
                        &parse_app_prefs_payload(&payload),
                        theme_mode,
                        light_palette,
                        dark_palette,
                        ui_font_size,
                        code_font_size,
                        ui_font_family,
                        code_font_family,
                        selection_popup_enabled,
                        send_with_modifier,
                        custom_css,
                        locale,
                        settings,
                    );
                } else if presentation_kind == "mcp_app"
                    && active_cb.get_untracked().as_deref() == Some(frame_id.as_str())
                {
                    show_mcp_app.call((frame_id, payload, true));
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
                context_usage,
                ..
            } => {
                // One usage row per reply: each round's usage (one API call)
                // is folded into the turn's row, which floats to the tail so
                // it never splits the coalesced tool-steps panel.
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    upsert_turn_usage(
                        v,
                        input,
                        output,
                        reasoning,
                        cached,
                        ctx_tokens,
                        max_context,
                        context_usage,
                    );
                });
                refresh_transcript_projections(&frame_id);
            }
            AgentEvent::Compaction {
                frame_id,
                before,
                after,
                strategy,
            } => {
                finish_compaction(&frame_id);
                let auto_continue = strategy == "auto_continue";
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    items.push(ChatItem::Compaction {
                        before,
                        after,
                        strategy,
                    });
                });
                if active_cb.get().as_deref() == Some(&frame_id) {
                    let before = before.to_string();
                    let after = after.to_string();
                    status_cb.set(if auto_continue {
                        tf(
                            locale_cb.get(),
                            "chat.auto_continued",
                            &[("count", before.as_str()), ("limit", after.as_str())],
                        )
                    } else {
                        tf(
                            locale_cb.get(),
                            "status.compact",
                            &[("before", before.as_str()), ("after", after.as_str())],
                        )
                    });
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
                stop_reason,
            } => {
                finish_compaction(&frame_id);
                flush_now();
                conversation_outlines_cb.update(|outlines| {
                    if let Some(entry) = outlines
                        .get_mut(&frame_id)
                        .and_then(|outline| outline.last_mut())
                    {
                        entry.response_at = Some(now_secs());
                    }
                });
                notify_desktop(&frame_id, "done", "");
                let outline = conversation_outlines_cb
                    .with_untracked(|outlines| outlines.get(&frame_id).cloned())
                    .unwrap_or_default();
                let mut page = transcript_pages
                    .with_untracked(|pages| pages.get(&frame_id).copied())
                    .unwrap_or_default();
                let mut trimmed = false;
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    strip_approval_pending(items);
                    settle_plan_cards(items);
                    let Some((first_item, dropped_turns)) = transcript_tail_trim_point(
                        items,
                        TRANSCRIPT_LIVE_TRIM_TURNS,
                        TRANSCRIPT_RENDER_TURNS,
                    ) else {
                        return;
                    };
                    let user_offset = page.user_offset.saturating_add(dropped_turns);
                    let Some(before_seq) = outline
                        .iter()
                        .find(|entry| entry.user_index == user_offset)
                        .and_then(|entry| entry.seq)
                    else {
                        return;
                    };
                    items.drain(..first_item);
                    page.next_before_seq = Some(before_seq);
                    page.user_offset = user_offset;
                    page.loading = false;
                    page.window_user_start = usize::MAX;
                    trimmed = true;
                });
                if trimmed {
                    transcript_pages.update(|pages| {
                        pages.insert(frame_id.clone(), page);
                    });
                }
                refresh_transcript_projections(&frame_id);
                trajectory_settle(&frame_id);
                approval_cb.update(|s| {
                    s.remove(&frame_id);
                });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                set_pet_activity(&frame_id, "jumping");
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                refresh_session_history();
                if stop_reason
                    .as_deref()
                    .is_none_or(|reason| reason == "end_turn")
                {
                    request_turn_memory_proposal(
                        frame_id.clone(),
                        None,
                        true,
                        turn_memory_proposal,
                        turn_memory_editor,
                        turn_memory_scope,
                        turn_memory_replace_id,
                        turn_memory_loading,
                        turn_memory_error,
                        status_cb,
                        locale_cb,
                    );
                }
                let has_final_answer =
                    if active_cb.get_untracked().as_deref() == Some(frame_id.as_str()) {
                        items_cb.with_untracked(|items| latest_turn_has_final_answer(items))
                    } else {
                        transcripts_cb.with_untracked(|transcripts| {
                            transcripts
                                .get(&frame_id)
                                .is_some_and(|items| latest_turn_has_final_answer(items))
                        })
                    };
                if settings.get_untracked().follow_up_questions && has_final_answer {
                    let generation = follow_up_generation.try_update(|generations| {
                        let generation = generations.entry(frame_id.clone()).or_default();
                        *generation += 1;
                        *generation
                    });
                    spawn_local(async move {
                        let args = to_value(&serde_json::json!({
                            "sessionId": frame_id.clone(),
                        }))
                        .unwrap();
                        let Ok(value) = invoke_checked("generate_follow_up_questions", args).await
                        else {
                            return;
                        };
                        let Ok(questions) = serde_wasm_bindgen::from_value::<Vec<String>>(value)
                        else {
                            return;
                        };
                        if questions.len() == 3
                            && follow_up_generation
                                .with_untracked(|generations| generations.get(&frame_id).copied())
                                == generation
                        {
                            follow_up_questions.update(|all| {
                                all.insert(frame_id, questions);
                            });
                        }
                    });
                }
            }
            AgentEvent::Error { frame_id, message } => {
                finish_compaction(&frame_id);
                flush_now();
                conversation_outlines_cb.update(|outlines| {
                    if let Some(entry) = outlines
                        .get_mut(&frame_id)
                        .and_then(|outline| outline.last_mut())
                    {
                        entry.response_at = Some(now_secs());
                    }
                });
                notify_desktop(&frame_id, "error", &message);
                // A model that cannot take images leaves the attachment sitting
                // in history, so every later send fails the same way. Toast the
                // fix and roll the turn back into the composer instead of
                // parking an error card on a conversation that is now stuck.
                let rolled_back = i18n::is_image_unsupported(&message)
                    && active_cb.get_untracked().as_deref() == Some(&frame_id)
                    && {
                        let last_user = items_cb
                            .get_untracked()
                            .iter()
                            .rposition(|item| matches!(item, ChatItem::User(_)));
                        if let Some(index) = last_user {
                            // Truncating at the user turn also drops any
                            // approval-pending rows the dead turn left behind.
                            rewind_to_user_item(index);
                            show_warning_toast(&t(locale_cb.get_untracked(), "err.hint.image"));
                        }
                        last_user.is_some()
                    };
                let offer_context_recovery = !rolled_back
                    && i18n::is_context_limit_error(&message)
                    && active_cb.get_untracked().as_deref() == Some(&frame_id)
                    // ACP owns its remote transcript and cannot run Wisp's
                    // /compact + resume path. Do not offer an action that
                    // cannot preserve its opaque session state.
                    && active_acp_agent_id.get_untracked().is_none();
                if !rolled_back {
                    let model = session_model_label(
                        &models_cb.get_untracked(),
                        &session_models_cb.get_untracked(),
                        Some(&frame_id),
                    );
                    route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                        strip_approval_pending(v);
                        settle_plan_cards(v);
                        v.push(ChatItem::Assistant {
                            text: format!("Error: {message}"),
                            model,
                            resources: Vec::new(),
                        });
                    });
                }
                refresh_transcript_projections(&frame_id);
                trajectory_settle(&frame_id);
                approval_cb.update(|s| {
                    s.remove(&frame_id);
                });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                set_pet_activity(&frame_id, "failed");
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                if offer_context_recovery {
                    // The modal supersedes any transient surface left open
                    // when the async error arrived.
                    selection_popup.set(None);
                    context_recovery_busy.set(false);
                    context_recovery_error.set(None);
                    context_recovery_dialog.set(Some(frame_id));
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
                refresh_transcript_projections(&frame_id);
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
            // Deliberately ignored, not forgotten: `edit` emits Diff from
            // `before()` — ahead of the write, and even when the edit then
            // fails — and emits FileChanged for the same path from `run()`
            // once the bytes land. Refreshing on Diff would just reload the
            // pre-edit file. It stays in the enum because dropping a variant
            // from a tagged enum breaks deserialization of the events the
            // backend still sends.
            AgentEvent::Diff { .. } => {}
            AgentEvent::FileChanged { frame_id, path } => {
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |items| {
                    let index = process_item_insert_index(items);
                    items.insert(index, ChatItem::FileChanged(path.clone()));
                });
                refresh_transcript_projections(&frame_id);
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

    let browser_cleanup_pending = browser_tab_cleanup;
    let browser_cleanup_queue = browser_tab_cleanup_queue;
    let browser_cleanup_selected = browser_tab_cleanup_selected;
    let browser_cleanup_error = browser_tab_cleanup_error;
    let browser_cleanup_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(prompt) = serde_wasm_bindgen::from_value::<BrowserTabCleanupPrompt>(payload) {
            present_browser_tab_cleanup(
                browser_cleanup_pending,
                browser_cleanup_queue,
                browser_cleanup_selected,
                browser_cleanup_error,
                prompt,
            );
        }
    }) as Box<dyn FnMut(JsValue)>);
    let browser_cleanup_js = browser_cleanup_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    std::mem::forget(browser_cleanup_cb);
    spawn_local(async move {
        let _ = listen("browser-tab-cleanup", &browser_cleanup_js).await;
        if let Ok(value) =
            invoke_checked("list_pending_browser_tab_cleanups", JsValue::UNDEFINED).await
        {
            if let Ok(prompts) =
                serde_wasm_bindgen::from_value::<Vec<BrowserTabCleanupPrompt>>(value)
            {
                for prompt in prompts {
                    present_browser_tab_cleanup(
                        browser_cleanup_pending,
                        browser_cleanup_queue,
                        browser_cleanup_selected,
                        browser_cleanup_error,
                        prompt,
                    );
                }
            }
        }
    });
    let browser_human_pending = browser_needs_human;
    let browser_human_error = browser_needs_human_error;
    let browser_human_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(prompt) = serde_wasm_bindgen::from_value::<BrowserNeedsHumanPrompt>(payload) {
            present_browser_needs_human(browser_human_pending, browser_human_error, prompt);
        }
    }) as Box<dyn FnMut(JsValue)>);
    let browser_human_js = browser_human_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    std::mem::forget(browser_human_cb);
    spawn_local(async move {
        let _ = listen("browser-needs-human", &browser_human_js).await;
        if let Ok(value) =
            invoke_checked("list_pending_browser_needs_human", JsValue::UNDEFINED).await
        {
            if let Ok(prompt) = serde_wasm_bindgen::from_value::<BrowserNeedsHumanPrompt>(value) {
                present_browser_needs_human(browser_human_pending, browser_human_error, prompt);
            }
        }
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
                let mut card = parse_plan_card(&update.payload);
                card.state = PlanState::Streaming;
                route_items(
                    active_session,
                    items,
                    transcripts,
                    &update.frame_id,
                    |rows| upsert_plan_card(rows, card),
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
                let used = update
                    .payload
                    .get("used")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let max = update
                    .payload
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                acp_context_usage.update(|all| {
                    all.insert(
                        update.frame_id,
                        ContextUsageSnapshot {
                            used,
                            max,
                            breakdown: None,
                            estimated: false,
                        },
                    );
                });
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

    let project_transfer_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(progress) = serde_wasm_bindgen::from_value::<ProjectTransferProgress>(payload)
        else {
            return;
        };
        project_transfer.set(Some(progress));
    }) as Box<dyn FnMut(JsValue)>);
    let project_transfer_js: js_sys::Function = project_transfer_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    project_transfer_cb.forget();
    spawn_local(async move {
        let _ = listen_current_window("project-transfer-progress", &project_transfer_js).await;
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

    // ACP `ask_user`: the bridge parks the agent's question until the user
    // answers, so the card mirrors the permission flow — request event inserts
    // it, resolved event settles it.
    let ask_user_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(request) = serde_wasm_bindgen::from_value::<AskUserRequest>(payload) else {
            return;
        };
        let mut card = parse_question_card(&request.payload);
        card.source = PlanSource::Acp;
        card.request_id = Some(request.request_id);
        card.state = QuestionState::Pending;
        notify_desktop(&request.frame_id, "attention", &card.question);
        route_items(
            active_session,
            items,
            transcripts,
            &request.frame_id,
            |rows| {
                let idx = process_item_insert_index(rows);
                rows.insert(idx, ChatItem::Question(card));
            },
        );
    }) as Box<dyn FnMut(JsValue)>);
    let ask_user_js: js_sys::Function = ask_user_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    ask_user_cb.forget();
    spawn_local(async move {
        let _ = listen("ask-user-request", &ask_user_js).await;
    });

    let ask_resolved_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let Ok(resolved) = serde_wasm_bindgen::from_value::<AskUserResolved>(payload) else {
            return;
        };
        route_items(
            active_session,
            items,
            transcripts,
            &resolved.frame_id,
            |rows| {
                for row in rows {
                    if let ChatItem::Question(card) = row {
                        if card.request_id.as_deref() == Some(resolved.request_id.as_str()) {
                            card.state = if resolved.expired {
                                QuestionState::Expired
                            } else {
                                QuestionState::Answered
                            };
                        }
                    }
                }
            },
        );
    }) as Box<dyn FnMut(JsValue)>);
    let ask_resolved_js: js_sys::Function = ask_resolved_cb
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    ask_resolved_cb.forget();
    spawn_local(async move {
        let _ = listen("ask-user-resolved", &ask_resolved_js).await;
    });

    let stop = move |_| {
        if stopping_session.get().is_some() {
            return;
        }
        // Stop only the active session's turn; background conversations keep running.
        let Some(sid) = active_session.get() else {
            return;
        };
        stopping_session.set(Some(sid.clone()));
        spawn_local(async move {
            let arg = to_value(&tauri_args::stop_agent(&Some(sid.clone()))).unwrap();
            if let Err(error) = invoke_checked("stop_agent", arg).await {
                if stopping_session.get_untracked().as_deref() == Some(sid.as_str()) {
                    stopping_session.set(None);
                    let loc = locale.get_untracked();
                    let detail = localize_backend(loc, &js_error_text(error));
                    show_warning_toast(&tf(
                        loc,
                        "composer.stop_failed",
                        &[("msg", detail.as_str())],
                    ));
                }
            }
        });
    };

    let send = Callback::new(move |action: ComposerSendAction| {
        if demo_mode.get_untracked() {
            return;
        }
        // Shell-owned slash commands never reach the model; the picker inserts
        // the same text, so typed and picked commands behave identically.
        if action == ComposerSendAction::Normal {
            if let Some(runner) = slash_command_runner.get_untracked() {
                if runner.call(input.get()) {
                    return;
                }
            }
        }
        let message = input.get();
        let saved_attachments = attachments.get();
        let refs = composer_references.get();
        let quotes = composer_quotes.get();
        let paths = attachment_paths(&saved_attachments);
        let display_message = message_with_composer_context(&message, &paths, &refs, &quotes);
        let attached_feedback = feedback_context.get();
        let agent_message = attached_feedback.as_ref().map_or_else(
            || display_message.clone(),
            |context| {
                format!(
                    "{display_message}\n\nFeedback context: {}",
                    serde_json::to_string(context).unwrap_or_default()
                )
            },
        );
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
        let active = active_session.get();
        let creates_session = active.is_none();
        let pending_fast = pending_service_tier.get();
        // Any prior send-failed hint (e.g. the max_tokens truncation notice) is
        // stale once a new turn is committed; the Ok path never cleared it, so it
        // lingered forever. Clear it here so continuing the conversation dismisses it.
        status.set(String::new());
        if active_acp_agent_id.get().is_some() && action == ComposerSendAction::BranchNew {
            status.set("ACP protocol v1 does not support branching a bound session.".into());
            return;
        }
        if active_is_exploration.get() && action == ComposerSendAction::BranchNew {
            status.set(localize_backend(
                locale.get(),
                "Conversation branches cannot be created inside an exploration.",
            ));
            return;
        }
        if active_branch_state.get().is_some() && action == ComposerSendAction::BranchNew {
            status.set(localize_backend(
                locale.get(),
                "Conversation branches cannot be branched again.",
            ));
            return;
        }
        let branch = action == ComposerSendAction::BranchNew;
        let queued = !branch && active.as_ref().is_some_and(|id| running.get().contains(id));
        // Do not wait for AgentEvent::User: send_message can sit on the
        // session lock / prompt build for a long time while the optimistic
        // user bubble is already on screen.
        if let Some(id) = active.as_ref() {
            dismiss_follow_up_questions(follow_up_questions, follow_up_generation, id);
        }
        // Queue (#433): a plain send into a busy session parks behind the
        // running turn — cancellable / restorable to the composer until the
        // driver runs it — instead of a dialog. Cut-in / interrupt-replace are
        // explicit dropdown choices.
        if queued && action == ComposerSendAction::Normal {
            let Some(session) = active.clone() else {
                return;
            };
            let qid = queue_seq.get() + 1;
            queue_seq.set(qid);
            input.set(String::new());
            attachments.set(vec![]);
            motif_selection.set(None);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            picker_mode.set(None);
            route_items(active_session, items, transcripts, &session, |rows| {
                rows.push(ChatItem::QueuedUser {
                    id: qid,
                    text: display_message.clone(),
                });
            });
            transcript_projection_epoch.update(|revision| {
                *revision = revision.wrapping_add(1);
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
                if let Err(error) = invoke_checked("enqueue_turn", args).await {
                    route_items(active_session, items, transcripts, &session, |rows| {
                        remove_optimistic_send_rows(rows, &enqueue_msg);
                    });
                    transcript_projection_epoch.update(|revision| {
                        *revision = revision.wrapping_add(1);
                    });
                    status.set(send_failed(locale.get(), &js_error_text(error)));
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
        motif_selection.set(None);
        composer_references.set(vec![]);
        composer_quotes.set(vec![]);
        picker_mode.set(None);
        feedback_context.set(None);
        spawn_local(async move {
            let id = if branch {
                let args = to_value(&tauri_args::branch_session(
                    &active,
                    Some(message.trim()),
                    None,
                    Some("after_response"),
                ))
                .unwrap();
                match invoke_string_id("branch_session", args).await {
                    Ok(id) => id,
                    Err(error) => {
                        input.set(message);
                        attachments.set(saved_attachments);
                        composer_references.set(refs);
                        composer_quotes.set(quotes);
                        feedback_context.set(attached_feedback.clone());
                        status.set(send_failed(locale.get(), &error));
                        return;
                    }
                }
            } else if let Some(id) = active {
                id
            } else {
                match invoke_new_session().await {
                    Ok(id) => id,
                    Err(error) => {
                        input.set(message);
                        attachments.set(saved_attachments);
                        composer_references.set(refs);
                        composer_quotes.set(quotes);
                        feedback_context.set(attached_feedback.clone());
                        status.set(send_failed(locale.get(), &error));
                        return;
                    }
                }
            };
            if creates_session && agent_id.is_none() {
                if let Some(service_tier) = pending_fast.clone() {
                    let args = to_value(&serde_json::json!({
                        "sessionId": id.clone(),
                        "serviceTier": service_tier.clone(),
                    }))
                    .unwrap();
                    if let Err(error) = invoke_checked("set_session_service_tier", args).await {
                        active_session.set(Some(id.clone()));
                        input.set(message);
                        attachments.set(saved_attachments);
                        composer_references.set(refs);
                        composer_quotes.set(quotes);
                        feedback_context.set(attached_feedback.clone());
                        status.set(send_failed(locale.get(), &js_error_text(error)));
                        refresh_session_history();
                        return;
                    }
                    session_service_tiers.update(|values| {
                        values.insert(id.clone(), Some(service_tier));
                    });
                }
                pending_service_tier.set(None);
            }
            // Mark the turn pending before touching active_session so the
            // session→ACP lookup effect does not clear a just-selected agent
            // while send_message is still binding a newly activated session.
            // For the already-active session, insert its optimistic assistant
            // first so the busy transition cannot briefly mistake the prior
            // answer for the new live row and remount its Markdown.
            let activates_session = active_session.get_untracked().as_deref() != Some(id.as_str());
            if activates_session {
                begin_pending_turn(pending_turns, running, &id);
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
            if !activates_session {
                begin_pending_turn(pending_turns, running, &id);
            }
            transcript_projection_epoch.update(|revision| {
                *revision = revision.wrapping_add(1);
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
                message: agent_message,
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
                    // Prefer transcript evidence: Error/Reasoning/Tool events can
                    // finish before invoke rejects, and a bare string must not
                    // treat an already-started turn as a pre-start rollback.
                    // Draft restore must follow whether the bubble was actually
                    // removed — not a prior snapshot of "started" that races
                    // the live agent bus.
                    let (_, status_message) = split_turn_started_error(&raw);
                    let rolled_back = Rc::new(Cell::new(false));
                    let rolled_back_flag = rolled_back.clone();
                    route_items(active_session, items, transcripts, &id, |rows| {
                        let (started, message_text) =
                            send_failed_after_start(rows, &display_message, &raw);
                        let had_user = rows.iter().any(|item| {
                            matches!(item, ChatItem::User(value) if value == &display_message)
                                || matches!(
                                    item,
                                    ChatItem::QueuedUser { text, .. } if text == &display_message
                                )
                        });
                        if started {
                            mark_optimistic_send_failed(rows, &display_message, message_text);
                        } else {
                            remove_optimistic_send_rows(rows, &display_message);
                        }
                        let kept_user = rows.iter().any(|item| {
                            matches!(item, ChatItem::User(value) if value == &display_message)
                                || matches!(
                                    item,
                                    ChatItem::QueuedUser { text, .. } if text == &display_message
                                )
                        });
                        rolled_back_flag.set(had_user && !kept_user);
                    });
                    transcript_projection_epoch.update(|revision| {
                        *revision = revision.wrapping_add(1);
                    });
                    if rolled_back.get() {
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
                        feedback_context.set(attached_feedback.clone());
                    }
                    if raw.contains(NO_API_KEY_MARK) {
                        needs_api_key.set(true);
                    }
                    status.set(tf(
                        locale.get(),
                        "status.send_failed",
                        &[("msg", &localize_backend(locale.get(), status_message))],
                    ));
                }
            }
            finish_pending_turn(pending_turns, running, &id);
        });
    });
    let send_side_chat = move |request: (String, Vec<ComposerQuote>, bool)| {
        if demo_mode.get_untracked() {
            return;
        }
        let (question, quotes, clear_draft) = request;
        let question = message_with_read_only_quotes(&question, &quotes);
        if question.is_empty() || side_chat_busy.get() {
            return;
        }
        ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab);
        if clear_draft {
            side_chat_input.set(String::new());
            side_chat_quotes.set(vec![]);
        }
        side_chat_items.update(|v| v.push(SideChatItem::User(question.clone())));
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
                Ok(value) => match from_value::<SideChatResponse>(value) {
                    Ok(response) => SideChatItem::Assistant {
                        text: response.answer,
                        model: (!response.no_evidence).then_some(model).flatten(),
                        evidence: response.evidence,
                        snapshot_version: response.snapshot_version,
                        no_evidence: response.no_evidence,
                        error: false,
                    },
                    Err(error) => SideChatItem::Assistant {
                        text: format!("Error: invalid side-chat response: {error}"),
                        model: None,
                        evidence: Vec::new(),
                        snapshot_version: 0,
                        no_evidence: false,
                        error: true,
                    },
                },
                Err(err) => SideChatItem::Assistant {
                    text: format!(
                        "Error: {}",
                        localize_backend(locale.get(), &js_error_text(err))
                    ),
                    model: None,
                    evidence: Vec::new(),
                    snapshot_version: 0,
                    no_evidence: false,
                    error: true,
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
        // Editing a message with later conversation after it would discard
        // that conversation permanently — confirm first and offer a branch.
        if items.with(|list| list.len() > ui_index + 1) {
            edit_confirm.set(Some(ui_index));
            return;
        }
        rewind_to_user_item(ui_index);
    };
    let undo_message = Callback::new(move |assistant_ui_index: usize| {
        if busy.get() || turn_undo_busy.get() {
            return;
        }
        let list = items.get();
        let Some(user_ui_index) = list.get(..assistant_ui_index).and_then(|prefix| {
            prefix
                .iter()
                .rposition(|item| matches!(item, ChatItem::User(_)))
        }) else {
            return;
        };
        let Some(ChatItem::User(text)) = list.get(user_ui_index) else {
            return;
        };
        let Some(session_id) = active_session.get().filter(|id| !id.is_empty()) else {
            return;
        };
        let Some(local_user_index) = user_message_index(&list, user_ui_index) else {
            return;
        };
        let user_index = local_user_index
            + transcript_pages
                .with(|pages| pages.get(&session_id).copied())
                .map_or(0, |page| page.user_offset);
        let draft = composer_text_from_user_message(text);

        turn_undo_busy.set(true);
        turn_undo_error.set(None);
        spawn_local(async move {
            let args = to_value(&tauri_args::turn_undo(&session_id, user_index)).unwrap();
            match invoke_checked("preview_turn_undo", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<TurnUndoPreview>(value) {
                    Ok(preview)
                        if active_session.get_untracked().as_deref()
                            == Some(session_id.as_str()) =>
                    {
                        turn_undo_dialog.set(Some(TurnUndoDialog {
                            session_id,
                            user_index,
                            user_ui_index,
                            draft,
                            preview,
                        }));
                    }
                    Ok(_) => {}
                    Err(error) => show_toast(&error.to_string()),
                },
                Err(error) => show_toast(&localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                )),
            }
            turn_undo_busy.set(false);
        });
    });
    let confirm_turn_undo = Callback::new(move |_: ()| {
        if turn_undo_busy.get() {
            return;
        }
        let Some(dialog) = turn_undo_dialog.get() else {
            return;
        };
        turn_undo_busy.set(true);
        turn_undo_error.set(None);
        spawn_local(async move {
            let args = to_value(&tauri_args::turn_undo(
                &dialog.session_id,
                dialog.user_index,
            ))
            .unwrap();
            match invoke_checked("undo_turn", args).await {
                Ok(_) => {
                    if active_session.get_untracked().as_deref() == Some(dialog.session_id.as_str())
                    {
                        let updated = items.with_untracked(|rows| {
                            rows.iter()
                                .take(dialog.user_ui_index)
                                .cloned()
                                .collect::<Vec<_>>()
                        });
                        items.set(updated);
                        conversation_outlines.update(|outlines| {
                            if let Some(outline) = outlines.get_mut(&dialog.session_id) {
                                outline.retain(|entry| entry.user_index < dialog.user_index);
                            }
                        });
                        attachments.set(vec![]);
                        composer_references.set(vec![]);
                        composer_quotes.set(vec![]);
                        input.set(dialog.draft);
                        let artifact_args = to_value(
                            &serde_json::json!({ "sessionId": dialog.session_id.clone() }),
                        )
                        .unwrap();
                        if let Ok(value) = invoke_checked("list_artifacts", artifact_args).await {
                            if let Ok(rows) =
                                serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(value)
                            {
                                db_artifacts.set(rows);
                            }
                        }
                        refresh_session_history();
                        focus_composer();
                    }
                    turn_undo_dialog.set(None);
                }
                Err(error) => {
                    turn_undo_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    )));
                }
            }
            turn_undo_busy.set(false);
        });
    });
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
            if active_branch_state.get_untracked().is_some()
                || active_is_exploration.get_untracked()
            {
                return;
            }
            let Some((user_idx, draft, prefix_items)) = items.with(|list| {
                let user_ui_index = list
                    .iter()
                    .take(ui_index.saturating_add(1))
                    .rposition(|item| matches!(item, ChatItem::User(_)))?;
                let user_idx = user_message_index(list, user_ui_index)?;
                let ChatItem::User(text) = list.get(user_ui_index)? else {
                    return None;
                };
                Some((
                    user_idx,
                    composer_text_from_user_message(text),
                    list.iter().take(user_ui_index).cloned().collect::<Vec<_>>(),
                ))
            }) else {
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
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            spawn_local(async move {
                let checkpoint_kind = if matches!(
                    items.with_untracked(|rows| rows.get(ui_index).cloned()),
                    Some(ChatItem::User(_))
                ) {
                    "before_user"
                } else {
                    "after_response"
                };
                let arg = to_value(&tauri_args::branch_session(
                    &sid,
                    Some(draft.as_str()),
                    Some(user_idx),
                    Some(checkpoint_kind),
                ))
                .unwrap();
                let id = match invoke_string_id("branch_session", arg).await {
                    Ok(id) => id,
                    Err(error) => {
                        status.set(send_failed(locale.get(), &error));
                        return;
                    }
                };
                if let Some(source_id) = sid.clone() {
                    conversation_branches.update(|branches| {
                        branches
                            .entry(source_id.clone())
                            .or_default()
                            .push(SessionBranchLink {
                                id: id.clone(),
                                title: draft.clone(),
                                source_session_id: source_id,
                                checkpoint_user_index: user_idx,
                                checkpoint_kind: checkpoint_kind.into(),
                                merged: false,
                                merge_summary: None,
                            });
                    });
                }
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
                                {
                                    let mut chats = page
                                        .items
                                        .into_iter()
                                        .map(LoadedItem::into_chat)
                                        .collect::<Vec<_>>();
                                    settle_question_cards(&mut chats);
                                    chats
                                },
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
                replace_visible_transcript(
                    sid,
                    Some(&id),
                    branch_items,
                    items,
                    transcripts,
                    running,
                );
                if let Some(page_state) = page_state {
                    transcript_pages.update(|pages| {
                        pages.insert(id.clone(), page_state);
                    });
                }
                // A conversation branch inherits the selected checkpoint but
                // starts a fresh task. Reusing the source prompt here made the
                // branch feel like destructive rewind and invited an
                // accidental duplicate send.
                input.set(String::new());
                active_session.set(Some(id));
                active_branch_state.set(Some("active".into()));
                refresh_session_history();
                focus_composer();
            });
        }
    };

    // Queue (#433): edit / cancel / cut-in a parked follow-up from the composer card.
    let on_queue = Callback::new(move |op: QueueOp| {
        let sid = active_session.get_untracked().unwrap_or_default();
        if sid.is_empty() {
            return;
        }
        let restore = matches!(op, QueueOp::Edit(_));
        let (id, action, message): (u64, &'static str, Option<String>) = match op {
            QueueOp::Cancel(id) | QueueOp::Edit(id) => {
                let mut draft = String::new();
                route_items(active_session, items, transcripts, &sid, |rows| {
                    if restore {
                        if let Some(ChatItem::QueuedUser { text, .. }) = rows.iter().find(
                            |it| matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id),
                        ) {
                            draft = composer_text_from_user_message(text);
                        }
                    }
                    rows.retain(
                        |it| !matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id),
                    );
                });
                if restore {
                    input.set(draft);
                    focus_composer();
                }
                (id, "cancel", None)
            }
            // The bubble stays; it promotes to a User row when the running turn
            // folds it in and emits the matching User event.
            QueueOp::CutIn(id) => (id, "cutin", None),
            // Reorder (#433): swap with the neighbouring queued row locally, then
            // mirror it server-side. Queued rows sit contiguously at the tail, so
            // a neighbour that is not a QueuedUser means we are at an end → no-op.
            QueueOp::MoveUp(id) | QueueOp::MoveDown(id) => {
                let up = matches!(op, QueueOp::MoveUp(_));
                route_items(active_session, items, transcripts, &sid, |rows| {
                    let Some(i) = rows.iter().position(
                        |it| matches!(it, ChatItem::QueuedUser { id: qid, .. } if *qid == id),
                    ) else {
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
        if action != "cutin" {
            transcript_projection_epoch.update(|revision| {
                *revision = revision.wrapping_add(1);
            });
        }
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
    let composer_queue_offset = Signal::derive(move || {
        active_session
            .get()
            .and_then(|id| transcript_pages.with(|pages| pages.get(&id).copied()))
            .map_or(0, |page| page.user_offset)
    });
    let composer_queue_can_cut_in = Signal::derive(move || {
        active_acp_agent_id.get().is_none()
            && !matches!(
                active_branch_state.get().as_deref(),
                Some("merged" | "orphaned")
            )
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
                                let mut chats: Vec<ChatItem> =
                                    page.items.into_iter().map(LoadedItem::into_chat).collect();
                                settle_question_cards(&mut chats);
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
                                if active_session.get().as_deref() == Some(&id) {
                                    items.set(chats);
                                    force_chat_bottom();
                                } else {
                                    transcripts.update(|m| {
                                        m.insert(id.clone(), chats);
                                    });
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

    let compact_context_recovery = Callback::new(move |id: String| {
        if context_recovery_busy.get_untracked() {
            return;
        }
        context_recovery_busy.set(true);
        context_recovery_error.set(None);
        spawn_local(async move {
            let compact = to_value(&SendMessageArgs {
                session_id: Some(id.clone()),
                message: "/compact".into(),
                attachments: vec![],
                references: vec![],
                resume: false,
                acp_agent_id: None,
                guide: None,
                replace: None,
            })
            .unwrap();
            if let Err(error) = invoke_checked("send_message", compact).await {
                let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                context_recovery_error.set(Some(message));
                context_recovery_busy.set(false);
                return;
            }

            // /compact rewrites only the model context. The existing error row
            // stays in the visual transcript until we remove it here; the
            // completed tool rows remain and Resume continues after them.
            if active_session.get_untracked().as_deref() == Some(id.as_str()) {
                let model = session_model_label(
                    &models.get_untracked(),
                    &session_model_ids.get_untracked(),
                    Some(&id),
                );
                items.update(|rows| {
                    if let Some(index) = rows.iter().rposition(is_error_assistant) {
                        rows.remove(index);
                    }
                    ensure_streaming_assistant(rows, model);
                });
            }
            context_recovery_dialog.set(None);
            context_recovery_error.set(None);
            begin_pending_turn(pending_turns, running, &id);
            force_chat_bottom();

            let resume = to_value(&SendMessageArgs {
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
            if let Err(error) = invoke_checked("send_message", resume).await {
                let raw = js_error_text(error);
                if raw.contains(NO_API_KEY_MARK) {
                    needs_api_key.set(true);
                }
                status.set(tf(
                    locale.get_untracked(),
                    "status.send_failed",
                    &[("msg", &localize_backend(locale.get_untracked(), &raw))],
                ));
            }
            finish_pending_turn(pending_turns, running, &id);
            context_recovery_busy.set(false);
            refresh_session_history();
        });
    });

    let new_session_context_recovery = Callback::new(move |source_id: String| {
        if demo_mode.get_untracked() || context_recovery_busy.get_untracked() {
            return;
        }
        context_recovery_busy.set(true);
        context_recovery_error.set(None);
        spawn_local(async move {
            let id = match invoke_new_session().await {
                Ok(id) => id,
                Err(error) => {
                    context_recovery_error.set(Some(send_failed(locale.get_untracked(), &error)));
                    context_recovery_busy.set(false);
                    return;
                }
            };

            let prompt = t(locale.get_untracked(), "context_recovery.new_prompt");
            let model = active_model_label(&models.get_untracked());
            let initial = vec![
                ChatItem::User(prompt.clone()),
                ChatItem::Assistant {
                    text: String::new(),
                    model,
                    resources: Vec::new(),
                },
            ];
            replace_visible_transcript(
                active_session.get_untracked(),
                None,
                initial,
                items,
                transcripts,
                running,
            );
            active_acp_agent_id.set(None);
            active_session.set(Some(id.clone()));
            transcript_pages.update(|pages| {
                pages.entry(id.clone()).or_default().window_user_start = usize::MAX;
            });
            input.set(String::new());
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            context_recovery_dialog.set(None);
            context_recovery_error.set(None);
            begin_pending_turn(pending_turns, running, &id);
            force_chat_bottom();
            refresh_session_history();

            let args = to_value(&SendMessageArgs {
                session_id: Some(id.clone()),
                message: prompt,
                attachments: vec![],
                references: vec![ComposerReferenceArg::Session { id: source_id }],
                resume: false,
                acp_agent_id: None,
                guide: None,
                replace: None,
            })
            .unwrap();
            if let Err(error) = invoke_checked("send_message", args).await {
                let raw = js_error_text(error);
                if raw.contains(NO_API_KEY_MARK) {
                    needs_api_key.set(true);
                }
                status.set(tf(
                    locale.get_untracked(),
                    "status.send_failed",
                    &[("msg", &localize_backend(locale.get_untracked(), &raw))],
                ));
            }
            finish_pending_turn(pending_turns, running, &id);
            context_recovery_busy.set(false);
            refresh_session_history();
        });
    });

    let pick_files = move |_: ()| {
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
        picker_mode.set(None);
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

    let composer_blocked = move || {
        demo_mode.get()
            || uploading.get()
            || composer_scope_locked.get()
            || active_session
                .get()
                .is_some_and(|id| reviewing.with(|ids| ids.contains(&id)))
    };

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
        let banner = update_banner;
        spawn_local(async move {
            match invoke_checked("check_for_updates", JsValue::UNDEFINED).await {
                Ok(v) => match serde_wasm_bindgen::from_value::<UpdateCheck>(v) {
                    Ok(update) if update.update_available => {
                        banner.set(Some(AvailableUpdate {
                            version: update.latest_version.clone(),
                        }));
                        let text = tf(
                            loc.get(),
                            "status.update_available",
                            &[("version", &update.latest_version)],
                        );
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text);
                        let next = if update.downloaded {
                            UpdateCheckModal::ReadyToInstall {
                                version: update.latest_version,
                                release_url: update.release_url,
                            }
                        } else {
                            UpdateCheckModal::Available {
                                version: update.latest_version,
                                notes: update.notes,
                                release_url: update.release_url,
                                install_supported: update.install_supported,
                                downloading: update.downloading,
                            }
                        };
                        if matches!(modal.get_untracked(), Some(UpdateCheckModal::Checking)) {
                            modal.set(Some(next));
                        }
                    }
                    Ok(update) => {
                        banner.set(None);
                        let text = tf(
                            loc.get(),
                            "status.up_to_date",
                            &[("version", &update.current_version)],
                        );
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text);
                        if matches!(modal.get_untracked(), Some(UpdateCheckModal::Checking)) {
                            modal.set(Some(UpdateCheckModal::UpToDate {
                                version: update.current_version,
                            }));
                        }
                    }
                    Err(_) => {
                        let text = t(loc.get(), "status.update_check_complete").to_string();
                        msg.set(Some((true, text.clone())));
                        status_msg.set(text.clone());
                        if matches!(modal.get_untracked(), Some(UpdateCheckModal::Checking)) {
                            modal.set(Some(UpdateCheckModal::Failed {
                                message: text,
                                release_url: Some(
                                    "https://github.com/xuzhougeng/wisp-science/releases".into(),
                                ),
                            }));
                        }
                    }
                },
                Err(err) => {
                    let text = localize_backend(loc.get(), &js_error_text(err));
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text.clone());
                    if matches!(modal.get_untracked(), Some(UpdateCheckModal::Checking)) {
                        modal.set(Some(UpdateCheckModal::Failed {
                            message: text,
                            release_url: Some(
                                "https://github.com/xuzhougeng/wisp-science/releases".into(),
                            ),
                        }));
                    }
                }
            }
            busy.set(false);
        });
    });
    let check_updates = {
        let run_update_check = run_update_check.clone();
        move |_| run_update_check()
    };

    let refresh_skills = move || extensions.refresh_skills();

    let reload_skills = Callback::new(move |_: ()| extensions.reload_skills());

    let install_skill_from = move |path: String| extensions.install_skill_from(path);

    let refresh_plugins = move || extensions.refresh_plugins();

    let install_plugin_from =
        Callback::new(move |(path, expected_sha256): (String, Option<String>)| {
            extensions.install_plugin_from(path, expected_sha256)
        });
    let install_plugin_url =
        Callback::new(move |(source_url, expected_sha256): (String, String)| {
            extensions.install_plugin_url(source_url, expected_sha256)
        });
    let set_plugin_enabled =
        Callback::new(move |(id, version, enabled): (String, String, bool)| {
            extensions.set_plugin_enabled(id, version, enabled)
        });
    let remove_plugin =
        Callback::new(move |(id, version): (String, String)| extensions.remove_plugin(id, version));

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
            // Always load the window's active project when entering Memory;
            // the picker can then browse another workspace without switching chat.
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
        let project_id = memory_view
            .get_untracked()
            .map(|view| view.project_id)
            .unwrap_or_default();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": name,
                "projectId": project_id,
            }))
            .unwrap();
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
            "quick-actions" => {
                refresh_quick_actions();
                refresh_workflow_templates();
            }
            "workflows" => {
                refresh_workflow_templates();
                refresh_agent_resources(workflow_studio_state, specialists);
            }
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
        refresh_quick_actions();
        refresh_workflow_templates();
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
    let open_capability_settings = Callback::new(move |section: String| {
        show_capabilities.set(false);
        open_settings_fn(Some(section));
    });

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
            sync_actions_available.set(project_sync_backend_configured(&cfg));
            busy.set(false);
            show.set(false);
            status_msg.set(t(loc.get(), "status.settings_saved").into());
            s.set(cfg);
            refresh_pet.call(());
        });
    };

    let save_model_form = move |_| model_settings.save_model_form();

    let validate_model_form = move |_| model_settings.validate_model_form();

    let test_reviewer_form = move |_| model_settings.test_reviewer_form();

    let save_specialist_form = move |_| model_settings.save_specialist_form();

    let remove_specialist_fn = move |id: String| model_settings.remove_specialist(id);

    let start_new_session = Callback::new(move |_: ()| {
        if demo_mode.get_untracked() {
            return;
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            // Guard the malformed-response case before moving the visible
            // transcript, so failure leaves the current session untouched (#15).
            let id = match invoke_new_session().await {
                Ok(id) => id,
                Err(error) => {
                    status.set(send_failed(locale.get(), &error));
                    return;
                }
            };
            restore_chat_session_scroll(&id);
            replace_visible_transcript(
                active_session.get_untracked(),
                None,
                Vec::new(),
                items,
                transcripts,
                running,
            );
            active_session.set(Some(id));
            refresh_session_history();
            focus_composer();
        });
    });
    let new_session = move |_| start_new_session.call(());
    let compact_from_usage = Callback::new(move |_: ()| {
        let Some(id) = active_session.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return;
        }
        context_usage_open.set(false);
        spawn_local(async move {
            let args = to_value(&SendMessageArgs {
                session_id: Some(id),
                message: "/compact".into(),
                attachments: vec![],
                references: vec![],
                resume: false,
                acp_agent_id: None,
                guide: None,
                replace: None,
            })
            .unwrap();
            let _ = invoke_checked("send_message", args).await;
        });
    });
    let new_session_from_usage = Callback::new(move |_: ()| {
        context_usage_open.set(false);
        start_new_session.call(());
    });

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
            if demo_mode.get_untracked() || busy.get() {
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
                let id = match invoke_new_session().await {
                    Ok(id) => id,
                    Err(error) => {
                        status.set(send_failed(locale.get(), &error));
                        return;
                    }
                };
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

    let start_issue_report = {
        let items = items;
        let locale = locale;
        let demo_mode = demo_mode;
        let center_file = center_file;
        let active_session = active_session;
        let sel_artifact = sel_artifact;
        let right_tab = right_tab;
        let models = models;
        let transcripts = transcripts;
        move |_| {
            demo_mode.set(false);
            center_file.set(None);
            replace_visible_transcript(
                active_session.get_untracked(),
                None,
                Vec::new(),
                items,
                transcripts,
                running,
            );
            attachments.set(vec![]);
            composer_references.set(vec![]);
            composer_quotes.set(vec![]);
            sel_artifact.set(0);
            right_tab.set(RightTab::Artifacts);
            active_session.set(None);
            input.set(String::new());
            let model = active_model_label(&models.get_untracked())
                .unwrap_or_else(|| "not configured".into());
            feedback_context.set(Some(issue_report_chat_prompt(
                locale.get_untracked(),
                bootstrap.get_untracked().as_ref(),
                &model,
            )));
            show_sidebar.set(false);
            show_right.set(false);
            focus_composer();
        }
    };

    let use_plugin = Callback::new(
        move |(plugin_id, version, display_name, skill_names, enabled): (
            String,
            String,
            String,
            Vec<String>,
            bool,
        )| {
            if demo_mode.get_untracked() {
                return;
            }
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

                let session_id = match invoke_new_session().await {
                    Ok(session_id) => session_id,
                    Err(error) => {
                        status.set(send_failed(locale.get(), &error));
                        return;
                    }
                };
                let initial = vec![
                    ChatItem::User(prompt.clone()),
                    ChatItem::Assistant {
                        text: String::new(),
                        model: turn_model,
                        resources: Vec::new(),
                    },
                ];
                replace_visible_transcript(
                    active_session.get_untracked(),
                    None,
                    initial,
                    items,
                    transcripts,
                    running,
                );
                demo_mode.set(false);
                show_settings.set(false);
                attachments.set(vec![]);
                sel_artifact.set(0);
                right_tab.set(RightTab::Artifacts);
                active_session.set(Some(session_id.clone()));
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
        restore_chat_session_scroll(&id);
        // Swap the visible transcript before changing the active id. Agent
        // events are app-wide and route by `active_session`; publishing the new
        // id while `items` still belongs to the old frame creates a transition
        // window where the new frame can render over the old conversation
        // (#595). A cached transcript gives running/recent sessions an
        // immediate view; an uncached idle session intentionally shows empty
        // until its persisted page arrives.
        replace_visible_transcript(
            active_session.get_untracked(),
            Some(&id),
            Vec::new(),
            items,
            transcripts,
            running,
        );
        let is_running = running.get().contains(&id);
        active_session.set(Some(id.clone()));
        active_branch_state.set(sessions.with_untracked(|rows| {
            rows.iter()
                .find(|session| session.id == id)
                .and_then(|session| session.branch_state.clone())
        }));
        if is_running {
            // Mid-stream: render the cached transcript immediately, but still
            // reconcile the separately persisted Plan claim/status. This keeps
            // session switching and restart semantics identical.
            transcript_pages.update(|pages| {
                pages.entry(id.clone()).or_default().window_user_start = usize::MAX;
            });
            restore_chat_session_scroll(&id);
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
                conversation_branches.update(|branches| {
                    branches.insert(id.clone(), page.branches.clone());
                });
                active_branch_state.set(page.branch_state.clone());
                conversation_outlines.update(|outlines| {
                    outlines.insert(id.clone(), page.outline.clone());
                });
                let mut chats: Vec<ChatItem> =
                    page.items.into_iter().map(LoadedItem::into_chat).collect();
                settle_question_cards(&mut chats);
                // The session may have started a turn while this idle-page
                // request was in flight. Its live cache/items are newer than
                // the page snapshot, so never replace them with the stale load.
                if running.get_untracked().contains(&id) {
                    return;
                }
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
                // Only repaint the view if we're still on this session — a rapid
                // switch could have moved on while the load was in flight, and an
                // unguarded set would clobber the newer view with stale rows (#53).
                if active_session.get().as_deref() == Some(&id) {
                    items.set(chats.clone());
                    // The latest turn's tool rows are the whole verdict, so a
                    // reload cannot revive an offline banner the turn's own
                    // successful retrieval already answered (#887).
                    set_browser_offline_notice(
                        browser_offline_notice,
                        &id,
                        browser_offline_notice_from_items(&id, &chats),
                    );
                    for presentation in presentations {
                        if presentation.presentation_kind == "mcp_app" {
                            show_mcp_app.call((id.clone(), presentation.payload, false));
                        }
                    }
                    restore_chat_session_scroll(&id);
                } else {
                    transcripts.update(|m| {
                        m.insert(id.clone(), chats);
                    });
                }
            }
        });
    });
    let open_exploration = {
        let load_session = load_session.clone();
        Callback::new(move |exploration: Exploration| {
            let id = exploration.id.clone();
            let frame_id = exploration.frame_id.clone();
            let load_session = load_session.clone();
            spawn_local(async move {
                let args = to_value(&tauri_args::exploration(&id)).unwrap();
                match invoke_checked("open_exploration", args).await {
                    Ok(_) => load_session.call(frame_id),
                    Err(error) => status.set(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    )),
                }
            });
        })
    };
    let open_exploration_preview = Callback::new(move |exploration_id: String| {
        exploration_overlay.set(Some(ExplorationOverlay::Preview {
            exploration_id: exploration_id.clone(),
        }));
        exploration_preview.set(None);
        exploration_error.set(None);
        exploration_busy.set(true);
        spawn_local(async move {
            let args = to_value(&tauri_args::exploration(&exploration_id)).unwrap();
            match invoke_checked("preview_exploration_promotion", args).await {
                Ok(value) => match from_value::<ExplorationPromotionPreview>(value) {
                    Ok(value) => exploration_preview.set(Some(value)),
                    Err(error) => exploration_error.set(Some(error.to_string())),
                },
                Err(error) => exploration_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ))),
            }
            exploration_busy.set(false);
        });
    });
    let start_exploration_from_head = Callback::new(move |turn_index: usize| {
        let Some(source_frame_id) = active_session.get_untracked() else {
            return;
        };
        if active_branch_state.get_untracked().is_some() {
            return;
        }
        if explorations.with_untracked(|rows| {
            rows.iter()
                .any(|row| row.exploration.frame_id == source_frame_id)
        }) {
            return;
        }
        let number = explorations.with_untracked(|rows| {
            rows.iter()
                .filter(|row| row.source_frame_id == source_frame_id)
                .count()
                + 1
        });
        exploration_name.set(tf(
            locale.get_untracked(),
            "exploration.default_name",
            &[("n", &number.to_string())],
        ));
        exploration_preview.set(None);
        exploration_error.set(None);
        exploration_overlay.set(Some(ExplorationOverlay::Start {
            source_frame_id,
            turn_index,
        }));
    });
    let create_exploration_from_overlay = {
        let load_session = load_session.clone();
        Callback::new(
            move |(source_frame_id, turn_index, name): (String, usize, String)| {
                if exploration_busy.get_untracked() {
                    return;
                }
                exploration_busy.set(true);
                exploration_error.set(None);
                let load_session = load_session.clone();
                spawn_local(async move {
                    let args = to_value(&tauri_args::start_exploration(
                        &source_frame_id,
                        Some(turn_index),
                        &name,
                    ))
                    .unwrap();
                    match invoke_checked("start_exploration", args).await {
                        Ok(value) => match from_value::<Exploration>(value) {
                            Ok(exploration) => {
                                exploration_frames.update(|frames| {
                                    frames.insert(exploration.frame_id.clone());
                                });
                                exploration_overlay.set(None);
                                exploration_name.set(String::new());
                                refresh_explorations(explorations);
                                refresh_session_history();
                                load_session.call(exploration.frame_id);
                            }
                            Err(error) => exploration_error.set(Some(error.to_string())),
                        },
                        Err(error) => exploration_error.set(Some(localize_backend(
                            locale.get_untracked(),
                            &js_error_text(error),
                        ))),
                    }
                    exploration_busy.set(false);
                });
            },
        )
    };
    let promote_exploration_from_overlay = {
        let load_session = load_session.clone();
        Callback::new(move |(exploration_id, guard): (String, String)| {
            exploration_busy.set(true);
            exploration_error.set(None);
            let load_session = load_session.clone();
            spawn_local(async move {
                let args =
                    to_value(&tauri_args::promote_exploration(&exploration_id, &guard)).unwrap();
                match invoke_checked("promote_exploration", args).await {
                    Ok(value) => match from_value::<ExplorationPromotionResult>(value) {
                        Ok(result) => {
                            exploration_overlay.set(None);
                            exploration_preview.set(None);
                            let resolved_frames = explorations.with_untracked(|rows| {
                                rows.iter()
                                    .filter(|row| row.source_frame_id == result.mainline_frame_id)
                                    .map(|row| row.exploration.frame_id.clone())
                                    .collect::<HashSet<_>>()
                            });
                            exploration_frames.update(|frames| {
                                frames.retain(|frame_id| !resolved_frames.contains(frame_id));
                            });
                            explorations.update(|rows| {
                                rows.retain(|row| row.source_frame_id != result.mainline_frame_id);
                            });
                            refresh_explorations(explorations);
                            refresh_session_history();
                            load_session.call(result.mainline_frame_id);
                        }
                        Err(error) => exploration_error.set(Some(error.to_string())),
                    },
                    Err(error) => {
                        exploration_error.set(Some(localize_backend(
                            locale.get_untracked(),
                            &js_error_text(error),
                        )));
                        let args = to_value(&tauri_args::exploration(&exploration_id)).unwrap();
                        if let Ok(value) =
                            invoke_checked("preview_exploration_promotion", args).await
                        {
                            if let Ok(value) = from_value::<ExplorationPromotionPreview>(value) {
                                exploration_preview.set(Some(value));
                            }
                        }
                    }
                }
                exploration_busy.set(false);
            });
        })
    };
    let open_exploration_manual_resolution = Callback::new(move |exploration_id: String| {
        if exploration_busy.get_untracked() {
            return;
        }
        exploration_busy.set(true);
        exploration_error.set(None);
        spawn_local(async move {
            let args = to_value(&tauri_args::exploration(&exploration_id)).unwrap();
            if let Err(error) = invoke_checked("open_exploration_manual_resolution", args).await {
                exploration_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                )));
            }
            exploration_busy.set(false);
        });
    });
    let finish_exploration_manual_resolution = {
        let load_session = load_session.clone();
        Callback::new(move |exploration_id: String| {
            if exploration_busy.get_untracked() {
                return;
            }
            let source_frame_id = explorations.with_untracked(|rows| {
                rows.iter()
                    .find(|row| row.exploration.id == exploration_id)
                    .map(|row| row.source_frame_id.clone())
            });
            let Some(source_frame_id) = source_frame_id else {
                exploration_error.set(Some(t(
                    locale.get_untracked(),
                    "exploration.manual_missing_source",
                )));
                return;
            };
            exploration_busy.set(true);
            exploration_error.set(None);
            let load_session = load_session.clone();
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "sourceFrameId": source_frame_id.clone(),
                }))
                .unwrap();
                match invoke_checked("abandon_exploration_round", args).await {
                    Ok(_) => {
                        exploration_overlay.set(None);
                        exploration_preview.set(None);
                        let resolved_frames = explorations.with_untracked(|rows| {
                            rows.iter()
                                .filter(|row| row.source_frame_id == source_frame_id)
                                .map(|row| row.exploration.frame_id.clone())
                                .collect::<HashSet<_>>()
                        });
                        exploration_frames.update(|frames| {
                            frames.retain(|frame_id| !resolved_frames.contains(frame_id));
                        });
                        explorations.update(|rows| {
                            rows.retain(|row| row.source_frame_id != source_frame_id);
                        });
                        refresh_explorations(explorations);
                        refresh_session_history();
                        load_session.call(source_frame_id);
                    }
                    Err(error) => exploration_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    ))),
                }
                exploration_busy.set(false);
            });
        })
    };
    let discard_exploration_from_overlay = {
        let load_session = load_session.clone();
        Callback::new(move |exploration_id: String| {
            exploration_busy.set(true);
            exploration_error.set(None);
            let source_frame_id = explorations.with_untracked(|rows| {
                rows.iter()
                    .find(|row| row.exploration.id == exploration_id)
                    .map(|row| row.source_frame_id.clone())
            });
            let discarded_frame_id = explorations.with_untracked(|rows| {
                rows.iter()
                    .find(|row| row.exploration.id == exploration_id)
                    .map(|row| row.exploration.frame_id.clone())
            });
            let load_session = load_session.clone();
            spawn_local(async move {
                let args = to_value(&tauri_args::exploration(&exploration_id)).unwrap();
                match invoke_checked("discard_exploration", args).await {
                    Ok(_) => {
                        exploration_overlay.set(None);
                        exploration_preview.set(None);
                        refresh_explorations(explorations);
                        if active_session.get_untracked() == discarded_frame_id {
                            if let Some(source_frame_id) = source_frame_id {
                                load_session.call(source_frame_id);
                            }
                        }
                    }
                    Err(error) => exploration_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    ))),
                }
                exploration_busy.set(false);
            });
        })
    };
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
            let still_active = active_session.get_untracked().as_deref() == Some(id.as_str());
            if still_active {
                preserve_chat_prepend_position();
                items.update(|current| {
                    current.splice(0..0, older);
                    // Settle over the merged window: an old page's question
                    // finds its answering user message in the loaded rows.
                    settle_question_cards(current);
                });
            } else {
                transcripts.update(|saved| {
                    let current = saved.entry(id.clone()).or_default();
                    current.splice(0..0, older);
                    settle_question_cards(current);
                });
            }
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

    let jump_to_review_message = Callback::new(move |message_index: usize| {
        if let Some(ui_index) =
            items.with_untracked(|rows| review_message_ui_index(rows, message_index))
        {
            jump_chat_to_item(ui_index);
        }
    });

    let request_session_review = Callback::new(move |session_id: String| {
        if reviewing.with_untracked(|ids| ids.contains(&session_id)) {
            return;
        }
        reviewing.update(|ids| {
            ids.insert(session_id.clone());
        });
        let loc = locale.get_untracked();
        status.set(t(loc, "status.reviewing"));
        spawn_local(async move {
            let arg = to_value(&tauri_args::review_session(&Some(session_id.clone()))).unwrap();
            if let Err(err) = invoke_checked("review_session", arg).await {
                status.set(tf(
                    loc,
                    "status.review_failed",
                    &[("msg", &localize_backend(loc, &js_error_text(err)))],
                ));
            }
            reviewing.update(|ids| {
                ids.remove(&session_id);
            });
        });
    });

    let request_turn_memory = Callback::new(move |(session_id, turn_index): (String, usize)| {
        request_turn_memory_proposal(
            session_id,
            Some(turn_index),
            false,
            turn_memory_proposal,
            turn_memory_editor,
            turn_memory_scope,
            turn_memory_replace_id,
            turn_memory_loading,
            turn_memory_error,
            status,
            locale,
        );
    });

    // Built-in slash commands the shell executes itself. Returns true when the
    // text was a known command and was consumed (never reaches the model).
    // "/compact" is the exception: the session backend intercepts it, so it
    // falls through to the normal send path.
    slash_command_runner.set(Some(Callback::new(move |text: String| -> bool {
        let Some((name, payload)) = parse_slash_command(&text) else {
            return false;
        };
        match name {
            "compact" => return false,
            "fork" => {
                if active_branch_state.get_untracked().is_some()
                    || active_is_exploration.get_untracked()
                {
                    input.set(String::new());
                    status.set(localize_backend(
                        locale.get_untracked(),
                        if active_is_exploration.get_untracked() {
                            "Conversation branches cannot be created inside an exploration."
                        } else {
                            "Conversation branches cannot be branched again."
                        },
                    ));
                    return true;
                }
                if payload.is_empty() {
                    input.set(String::new());
                    status.set(t(locale.get_untracked(), "composer.cmd_fork_empty"));
                } else {
                    input.set(payload.to_string());
                    send.call(ComposerSendAction::BranchNew);
                }
                return true;
            }
            "save-as-skill" => {
                input.set(t(locale.get_untracked(), "composer.skill_prompt").into());
                focus_composer();
                return true;
            }
            // Permission modes: `full` flips the session's Full Permission
            // flag (through the same warning modal as the agent-menu toggle),
            // `ask` returns to per-call approval, `auto` is not built yet.
            "permission" => {
                input.set(String::new());
                match payload {
                    "full" => {
                        if full_permission_enabled.get_untracked() {
                            show_toast(&t(locale.get_untracked(), "permission.full_already"));
                        } else {
                            ui_confirm.set(Some(UiConfirm::EnableFullPermission));
                        }
                    }
                    "ask" => {
                        if full_permission_enabled.get_untracked() {
                            disable_full_permission.call(());
                        } else {
                            show_toast(&t(locale.get_untracked(), "permission.ask_already"));
                        }
                    }
                    "auto" => {
                        show_toast(&t(locale.get_untracked(), "permission.auto_unavailable"));
                    }
                    _ => status.set(t(locale.get_untracked(), "composer.cmd_permission_usage")),
                }
                return true;
            }
            _ => {}
        }
        input.set(String::new());
        match name {
            "btw" => {
                if payload.is_empty() {
                    ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab);
                } else {
                    send_side_chat((payload.to_string(), vec![], false));
                }
            }
            // Same target as the message-level undo button: the latest
            // completed assistant turn. The preview dialog confirms before
            // anything is rolled back.
            "rewind" => {
                let target = items.with_untracked(|list| {
                    list.iter().enumerate().rev().find_map(|(index, item)| {
                        matches!(
                            item,
                            ChatItem::Assistant { text, .. }
                                if !text.trim().is_empty() && !text.starts_with("Error: ")
                        )
                        .then_some(index)
                    })
                });
                if let Some(index) = target {
                    undo_message.call(index);
                }
            }
            "review" => {
                if let Some(sid) = active_session.get_untracked().filter(|id| !id.is_empty()) {
                    request_session_review.call(sid);
                }
            }
            // Same target as the message-level memory button: the latest
            // completed assistant turn's owning user turn.
            "remember" => {
                let target = active_session
                    .get_untracked()
                    .filter(|id| !id.is_empty())
                    .and_then(|sid| {
                        let list = items.get_untracked();
                        let index = list.iter().rposition(|item| {
                            matches!(item, ChatItem::Assistant { text, .. } if !text.trim().is_empty())
                        })?;
                        let turn = owning_user_turn_index(&list, index)?;
                        let offset = transcript_pages
                            .with_untracked(|pages| pages.get(&sid).copied())
                            .map_or(0, |page| page.user_offset);
                        Some((sid, turn + offset))
                    });
                if let Some((sid, turn)) = target {
                    request_turn_memory.call((sid, turn));
                }
            }
            "context" => {
                if active_context_usage.get_untracked().is_some() {
                    context_usage_open.set(true);
                }
            }
            // Same switch as the agent menu's Plan first toggle.
            "plan" => {
                if !plan_compat.get_untracked() {
                    set_plan_first.call(!plan_mode_active.get_untracked());
                }
            }
            "skills" => open_settings_fn(Some("skills".into())),
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
            "upload" => pick_files(()),
            // Open the share preview over the current transcript; thinking
            // rows are listed but deselected (hidden from the export).
            "share" => open_share.call(()),
            "trajectory" => trajectory_open.set(true),
            _ => {}
        }
        true
    })));

    let jump_to_conversation_outline =
        Callback::new(move |(target, before_seq): (usize, Option<i64>)| {
            let Some(id) = active_session.get_untracked() else {
                return;
            };
            let user_offset = transcript_pages
                .with_untracked(|pages| pages.get(&id).copied())
                .map_or(0, |page| page.user_offset);
            if conversation_outline_target_is_loaded(&items.get_untracked(), user_offset, target) {
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
                let mut chats = page
                    .items
                    .into_iter()
                    .map(LoadedItem::into_chat)
                    .collect::<Vec<_>>();
                settle_question_cards(&mut chats);
                let chats = chats;
                let loaded_turns = chats
                    .iter()
                    .filter(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
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
                if active_session.get_untracked().as_deref() == Some(id.as_str()) {
                    items.set(chats);
                    jump_chat_to_user(target);
                } else {
                    transcripts.update(|saved| {
                        saved.insert(id.clone(), chats);
                    });
                }
            });
        });

    let load_demo = move |info: DemoInfo| {
        let id = info.id.clone();
        let items = items;
        // Demos are read-only transcripts; they don't stream, so we don't touch
        // `running`. We do stash the current chat so returning to it is possible.
        replace_visible_transcript(
            active_session.get_untracked(),
            None,
            Vec::new(),
            items,
            transcripts,
            running,
        );
        attachments.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        active_session.set(None);
        spawn_local(async move {
            let v = invoke(
                "load_demo",
                to_value(&serde_json::json!({ "id": id })).unwrap(),
            )
            .await;
            if let Ok(demo) = serde_wasm_bindgen::from_value::<Demo>(v) {
                let mut view = if !demo.items.is_empty() {
                    demo.items.into_iter().map(LoadedItem::into_chat).collect()
                } else {
                    let mut legacy = vec![ChatItem::User(demo.request.clone())];
                    if let Some(t) = &demo.thinking {
                        if !t.is_empty() {
                            legacy.push(ChatItem::Reasoning(t.clone()));
                        }
                    }
                    legacy.push(ChatItem::Assistant {
                        text: demo.response.clone(),
                        model: None,
                        resources: Vec::new(),
                    });
                    legacy
                };
                settle_question_cards(&mut view);
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

    // ponytail: an ACP plan update is a todo list with no id to approve, so
    // "approve" is the mode switch plus one ordinary turn. Upgrade to a
    // structured approval only if ACP ever gains a plan-decision request.
    let on_plan_decision = Callback::new(move |decision: PlanDecision| {
        let loc = locale.get_untracked();
        let Some(session_id) = active_session.get_untracked() else {
            return;
        };
        // Built-in sessions leave plan mode by clearing their own flag; ACP ones
        // by switching the agent back to its non-plan mode.
        let native = local_plan_mode.get_untracked().is_some();
        let exit_mode = (!native)
            .then(|| acp_session_modes.with_untracked(|all| plan_mode_pair(all.get(&session_id))))
            .flatten()
            .map(|(_, exit)| exit);
        spawn_local(async move {
            // Await leaving plan mode before sending: the switch and the prompt
            // are separate calls, and a prompt that lands first would just run
            // the next turn with the tool gate still closed.
            if native {
                let args = to_value(&serde_json::json!({
                    "sessionId": session_id.clone(),
                    "enabled": false,
                }))
                .unwrap();
                if invoke_checked("set_session_plan_mode", args).await.is_err() {
                    return;
                }
                local_plan_mode.set(Some(false));
            } else if let Some(exit_mode) = exit_mode {
                if !apply_acp_mode(acp_session_modes, session_id, exit_mode).await {
                    return;
                }
            }
            // The plan itself is already persisted, so "save and exit" is the
            // mode switch and nothing else — that is its whole difference from
            // approve.
            if decision == PlanDecision::SaveExit {
                show_toast(&t(loc, "plan.saved"));
                return;
            }
            // A draft in the composer is the user's own go-ahead; only fill in a
            // default so approving never discards what they typed.
            if input.get_untracked().trim().is_empty() {
                input.set(t(loc, "plan.approve").into());
            }
            send.call(ComposerSendAction::Normal);
            show_toast(&t(loc, "plan.executing"));
        });
    });

    // Answer a question card. Built-in source: the answer is an ordinary user
    // message on the normal send path — the agent reads it next turn. ACP
    // source: resolve the bridge's pending request; the answer returns inside
    // the agent's still-running turn.
    let on_question_answer = Callback::new(
        move |(ui_index, request_id, answer): (usize, Option<String>, String)| {
            let answer = answer.trim().to_string();
            if answer.is_empty() {
                return;
            }
            // Settle the card before sending: the send appends rows, so the
            // pre-send index is still the card's.
            items.update(|rows| {
                if let Some(ChatItem::Question(card)) = rows.get_mut(ui_index) {
                    card.state = QuestionState::Answered;
                }
            });
            match request_id {
                Some(request_id) => spawn_local(async move {
                    let args = to_value(&serde_json::json!({
                        "requestId": request_id,
                        "answer": answer,
                    }))
                    .unwrap();
                    let _ = invoke_checked("respond_ask_user", args).await;
                }),
                None => {
                    // The send callback reads the composer synchronously, so
                    // swap the answer in and restore any draft right after.
                    let draft = input.get_untracked();
                    input.set(answer);
                    send.call(ComposerSendAction::Normal);
                    if !draft.trim().is_empty() {
                        input.set(draft);
                    }
                }
            }
        },
    );

    let on_sidebar_resize_start =
        move |ev: web_sys::MouseEvent| pane_layout.sidebar_resize_start(ev);
    let on_sidebar_resize_move = move |ev: web_sys::MouseEvent| pane_layout.sidebar_resize_move(ev);
    let on_sidebar_resize_end = move |_| pane_layout.sidebar_resize_end();

    let on_resize_start = move |ev: web_sys::MouseEvent| pane_layout.right_resize_start(ev);
    let on_resize_move =
        move |ev: web_sys::MouseEvent| pane_layout.right_resize_move(ev, show_sidebar.get());

    let on_center_split_resize_start =
        move |ev: web_sys::MouseEvent| pane_layout.center_split_resize_start(ev);
    let on_center_split_resize_move =
        move |ev: web_sys::MouseEvent| pane_layout.center_split_resize_move(ev);

    let on_composer_resize_start =
        move |ev: web_sys::MouseEvent| pane_layout.composer_resize_start(ev);
    let on_composer_resize_move =
        move |ev: web_sys::MouseEvent| pane_layout.composer_resize_move(ev);
    let on_composer_resize_end = move |_| pane_layout.composer_resize_end();

    let on_terminal_resize_start =
        move |ev: web_sys::MouseEvent| pane_layout.terminal_resize_start(ev);
    let on_terminal_resize_move =
        move |ev: web_sys::MouseEvent| pane_layout.terminal_resize_move(ev);

    let on_context_usage_header_down =
        Callback::new(move |ev: web_sys::MouseEvent| context_usage.header_down(ev));
    let on_context_usage_header_dblclick =
        Callback::new(move |ev: web_sys::MouseEvent| context_usage.header_dblclick(ev));
    let on_context_usage_dock = Callback::new(move |()| context_usage.dock());
    let on_context_usage_drag_move = move |ev: web_sys::MouseEvent| context_usage.drag_move(ev);
    let on_context_usage_drag_end = move |_| context_usage.drag_end();
    let on_context_usage_resize_start =
        Callback::new(move |ev: web_sys::MouseEvent| context_usage.resize_begin(ev));
    let on_context_usage_resize_move = move |ev: web_sys::MouseEvent| context_usage.resize_move(ev);
    let on_context_usage_resize_end = move |_| context_usage.resize_end();

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
        if demo_mode.get_untracked() {
            return;
        }
        close_details_ancestor(&ev);
        show_settings.set(false);
        let loc = locale.get();
        let prompt = t(loc, "specialists.chat_prompt").to_string();
        spawn_local(async move {
            let id = match invoke_new_session().await {
                Ok(id) => id,
                Err(error) => {
                    status.set(send_failed(loc, &error));
                    return;
                }
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

    let save_skill_tags =
        Callback::new(move |(name, raw): (String, String)| extensions.save_skill_tags(name, raw));

    let set_visible_skills_enabled =
        Callback::new(move |enabled: bool| extensions.set_visible_skills_enabled(enabled));

    let dismiss_onboarding = Callback::new(move |_| {
        show_onboarding.set(false);
        spawn_local(async move {
            let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await;
        });
    });
    let dismiss_onboard = move |_| dismiss_onboarding.call(());

    // Onboarding step 0: save the entered key as DeepSeek models (flash as
    // the default, pro for heavier work), reusing the same `save_model`
    // command as Settings. Blank key = skip.
    // ponytail: onboarding is DeepSeek-only; other providers go through Settings › Models.
    let save_onboard_key = Callback::new(move |_| {
        let key = onboard_key.get();
        if key.trim().is_empty() {
            return;
        }
        let provider = "openai".to_string();
        let (api_url, _) = provider_defaults(&provider);
        // `save_model` makes every newly created profile the active one, so
        // the model the user should land on has to be saved last.
        let wanted = [DEEPSEEK_PRO_MODEL, DEEPSEEK_FLASH_MODEL];
        spawn_local(async move {
            for model in wanted {
                let arg = to_value(&serde_json::json!({
                    "profile": {
                        "id": "",
                        "label": "",
                        "provider": provider,
                        "api_url": api_url,
                        "model": model,
                        "max_tokens": 8192,
                        "reasoning_effort": "",
                        "supports_vision": false,
                        "use_for_vision": false,
                        "use_for_image_generation": false,
                        "use_for_video_generation": false,
                    },
                    "key": Some(key.clone()),
                    "useForVision": false,
                    "useForImageGeneration": false,
                    "useForVideoGeneration": false,
                }))
                .unwrap();
                if let Ok(v) = invoke_checked("save_model", arg).await {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                }
            }
            // Bind the built-in Reader to the flash tier so reading-heavy work
            // runs on the cheap model out of the box. An already-bound Reader
            // is the user's choice — leave it alone.
            let flash_id = models
                .get_untracked()
                .iter()
                .find(|p| p.model == DEEPSEEK_FLASH_MODEL)
                .map(|p| p.id.clone());
            if let Some(flash_id) = flash_id {
                if let Ok(v) = invoke_checked("list_specialists", JsValue::UNDEFINED).await {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                        if let Some(mut reader) = list
                            .into_iter()
                            .find(|s| s.id == "reader" && s.model_id.trim().is_empty())
                        {
                            reader.model_id = flash_id;
                            let arg = to_value(&serde_json::json!({ "spec": reader })).unwrap();
                            if let Ok(v) = invoke_checked("save_specialist_cmd", arg).await {
                                if let Ok(list) =
                                    serde_wasm_bindgen::from_value::<Vec<Specialist>>(v)
                                {
                                    specialists.set(list);
                                }
                            }
                        }
                    }
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
    let branch_merge_open = create_rw_signal::<Option<String>>(None);
    let branch_merge_preview = create_rw_signal::<Option<SessionBranchMergePreview>>(None);
    let branch_merge_draft = create_rw_signal(String::new());
    let branch_merge_busy = create_rw_signal(false);
    let branch_merge_error = create_rw_signal::<Option<String>>(None);
    let branch_merge_guidance_open = create_rw_signal(false);
    let branch_merge_guidance = create_rw_signal(String::new());
    let branch_merge_detail = create_rw_signal::<Option<(String, String)>>(None);
    let generate_branch_summary = Callback::new(
        move |(id, expected_guard_hash, current_version, user_guidance): (
            String,
            String,
            Option<String>,
            Option<String>,
        )| {
            branch_merge_busy.set(true);
            branch_merge_error.set(None);
            spawn_local(async move {
                let args = to_value(&serde_json::json!({
                    "id": id.clone(),
                    "expectedGuardHash": expected_guard_hash,
                    "currentVersion": current_version,
                    "userGuidance": user_guidance,
                }))
                .unwrap();
                let summary = invoke_checked("summarize_session_branch_merge", args)
                    .await
                    .and_then(|value| {
                        value.as_string().ok_or_else(|| {
                            wasm_bindgen::JsValue::from_str("Branch summary returned invalid text.")
                        })
                    });
                if branch_merge_open.get_untracked().as_deref() == Some(id.as_str()) {
                    match summary {
                        Ok(text) => branch_merge_draft.set(text),
                        Err(error) => branch_merge_error.set(Some(localize_backend(
                            locale.get_untracked(),
                            &js_error_text(error),
                        ))),
                    }
                    branch_merge_busy.set(false);
                }
            });
        },
    );
    let merge_branch_summary = {
        let load_main = load_session.clone();
        Callback::new(
            move |(id, expected_guard_hash, summary): (String, String, String)| {
                branch_merge_busy.set(true);
                branch_merge_error.set(None);
                let load_main = load_main.clone();
                let approved_summary = summary.clone();
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({
                        "id": id,
                        "expectedGuardHash": expected_guard_hash,
                        "summary": summary,
                    }))
                    .unwrap();
                    match invoke_checked("merge_session_branch_summary", args).await {
                        Ok(value) => match from_value::<SessionBranchMerge>(value) {
                            Ok(result) => {
                                conversation_branches.update(|by_source| {
                                    if let Some(branches) =
                                        by_source.get_mut(&result.main_session_id)
                                    {
                                        if let Some(branch) = branches
                                            .iter_mut()
                                            .find(|branch| branch.id == result.branch_session_id)
                                        {
                                            branch.merged = true;
                                            branch.merge_summary = Some(approved_summary.clone());
                                        }
                                    }
                                });
                                sessions.update(|rows| {
                                    if let Some(branch) = rows
                                        .iter_mut()
                                        .find(|session| session.id == result.branch_session_id)
                                    {
                                        branch.branch_state = Some("merged".into());
                                    }
                                });
                                transcripts.update(|stored| {
                                    stored.remove(&result.main_session_id);
                                });
                                branch_merge_open.set(None);
                                branch_merge_preview.set(None);
                                branch_merge_draft.set(String::new());
                                refresh_session_history();
                                load_main.call(result.main_session_id);
                                show_toast(&t(locale.get_untracked(), "branch.merge_success"));
                            }
                            Err(error) => branch_merge_error.set(Some(error.to_string())),
                        },
                        Err(error) => branch_merge_error.set(Some(localize_backend(
                            locale.get_untracked(),
                            &js_error_text(error),
                        ))),
                    }
                    branch_merge_busy.set(false);
                });
            },
        )
    };
    let compose_menu_open = create_rw_signal(false);
    let agent_menu_open = create_rw_signal(false);
    let reviewer_model_menu_open = create_rw_signal(false);
    let compute_menu_open = create_rw_signal(false);
    let compute_search = create_rw_signal(String::new());
    let hosts_attach_search = create_rw_signal(String::new());
    let specialist_menu_open = create_rw_signal(false);
    let auto_review_enabled = create_rw_signal(false);
    let auto_failure_analysis = create_rw_signal(AutoFailureAnalysisSettings::default());
    let delegation_enabled = create_rw_signal(false);
    let delegation_setting_busy = create_rw_signal(false);
    let agent_completion = create_rw_signal(AgentCompletionSettings::default());
    let agent_completion_busy = create_rw_signal(false);
    create_effect(move |_| {
        delegation_enabled.set(false);
        delegation_setting_busy.set(false);
        full_permission_enabled.set(false);
        full_permission_busy.set(false);
        plan_mode_busy.set(false);
        // Reset before the fetch: otherwise the previous session's flag shows
        // on the new one for as long as the round trip takes.
        local_plan_mode.set(Some(false));
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
            let plan = invoke_checked("get_session_plan_mode", args.clone())
                .await
                .ok()
                .map(|value| value.as_bool());
            let full_permission = invoke_checked("get_session_full_permission", args.clone())
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
                full_permission_enabled.set(full_permission.unwrap_or(false));
                local_plan_mode.set(plan.unwrap_or(None));
                agent_completion.set(completion.unwrap_or_default());
            }
        });
    });
    let run_quick_action = {
        let load_session = load_session.clone();
        Callback::new(
            move |(action_id, selection, source_path): (String, String, Option<String>)| {
                if demo_mode.get_untracked() {
                    return;
                }
                selection_popup.set(None);
                ctx_menu.set(None);
                clear_selection();
                let action = quick_actions
                    .get_untracked()
                    .into_iter()
                    .find(|action| action.id == action_id);
                if let Some(action) = action
                    .as_ref()
                    .filter(|action| quick_action_uses_current_conversation(action))
                {
                    composer_quotes.update(|quotes| {
                        let quote =
                            ComposerQuote::from_selection(selection.clone(), source_path.clone());
                        if !quotes.contains(&quote) {
                            quotes.push(quote);
                        }
                    });
                    composer_references.update(|references| {
                        let skill = ComposerReferenceChip::Skill {
                            name: LITERATURE_REVIEW_SKILL.into(),
                        };
                        if !references
                            .iter()
                            .any(|reference| reference.key() == skill.key())
                        {
                            references.push(skill);
                        }
                    });
                    let prompt = t(
                        locale.get_untracked(),
                        "quick_action.literature_composer_prompt",
                    );
                    input.update(|current| {
                        *current = append_composer_prompt(current, &prompt);
                    });
                    let name = quick_action_label(locale.get_untracked(), action);
                    status.set(tf(
                        locale.get_untracked(),
                        "quick_action.prepared",
                        &[("name", &name)],
                    ));
                    focus_composer();
                    return;
                }
                let load_session = load_session.clone();
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({
                        "actionId": action_id,
                        "input": {
                            "selection": selection,
                            "sourcePath": source_path,
                        },
                    }))
                    .unwrap();
                    match invoke_checked("run_quick_action", args).await {
                        Ok(value) => {
                            let Ok(run) = serde_wasm_bindgen::from_value::<QuickActionRun>(value)
                            else {
                                status.set(tf(
                                    locale.get_untracked(),
                                    "quick_action.failed",
                                    &[("msg", "Invalid backend response")],
                                ));
                                return;
                            };
                            let name = quick_action_label(locale.get_untracked(), &run.action);
                            load_session.call(run.session_id);
                            delegation_enabled.set(true);
                            ensure_right_tab(
                                RightTab::Agents,
                                show_right,
                                open_right_tabs,
                                right_tab,
                            );
                            refresh_agent_workflows(agent_panel);
                            refresh_session_history();
                            status.set(tf(
                                locale.get_untracked(),
                                if run.started {
                                    "quick_action.started"
                                } else {
                                    "quick_action.created_draft"
                                },
                                &[("name", &name)],
                            ));
                        }
                        Err(error) => {
                            let loc = locale.get_untracked();
                            let message = localize_backend(loc, &js_error_text(error));
                            status.set(tf(loc, "quick_action.failed", &[("msg", &message)]));
                        }
                    }
                });
            },
        )
    };
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
    spawn_local(async move {
        let value = invoke("get_auto_failure_analysis_settings", JsValue::UNDEFINED).await;
        if let Ok(settings) = from_value::<AutoFailureAnalysisSettings>(value) {
            auto_failure_analysis.set(settings);
        }
    });
    let save_auto_failure_analysis = Callback::new(move |next: AutoFailureAnalysisSettings| {
        let previous = auto_failure_analysis.get_untracked();
        auto_failure_analysis.set(next);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "settings": next })).unwrap();
            match invoke_checked("set_auto_failure_analysis_settings", args).await {
                Ok(value) => {
                    if let Ok(saved) = from_value::<AutoFailureAnalysisSettings>(value) {
                        auto_failure_analysis.set(saved);
                    }
                }
                Err(_) => auto_failure_analysis.set(previous),
            }
        });
    });
    let ssh_hosts = create_rw_signal::<Vec<SshHost>>(vec![]);
    let selected_context_id = create_rw_signal::<Option<String>>(None);
    let probing_context_id = create_rw_signal::<Option<String>>(None);
    let context_details_modal = create_rw_signal::<Option<(String, ContextModalKind)>>(None);
    let runtime_interpreter_form = create_rw_signal(None::<RuntimeInterpreterForm>);
    let storage_prefs_form = create_rw_signal(None::<StoragePrefsForm>);
    let run_review_modal = create_rw_signal(None::<String>);
    provide_context(RunReviewModal(run_review_modal));
    // Deferred results-review prompting (#897): monitored run cards nominate
    // candidates here; this root effect waits until the owning session is
    // idle, asks the backend whether each candidate has an unresolved product
    // decision, and opens the modal for the newest one that does. Exploratory
    // command runs never enter the queue, and dismissed or empty workspaces
    // never prompt.
    let pending_run_reviews = create_rw_signal(Vec::<String>::new());
    provide_context(crate::overlays::PendingRunReviews(pending_run_reviews));
    create_effect(move |_| {
        if run_review_modal.get().is_some() {
            return;
        }
        let running_now = running.get();
        let ready: Vec<String> = pending_run_reviews.with(|ids| {
            ids.iter()
                .filter(|id| {
                    run_records.with(|runs| {
                        runs.iter()
                            .find(|run| run.id == **id)
                            .and_then(|run| run.frame_id.clone())
                            .is_none_or(|frame| !running_now.contains(&frame))
                    })
                })
                .cloned()
                .collect()
        });
        if ready.is_empty() {
            return;
        }
        pending_run_reviews.update(|ids| ids.retain(|id| !ready.contains(id)));
        spawn_local(async move {
            for id in ready.iter().rev() {
                let args = to_value(&serde_json::json!({ "runId": id })).unwrap();
                match invoke_checked("should_prompt_run_review", args).await {
                    Ok(value) if value.as_bool() == Some(true) => {
                        run_review_modal.set(Some(id.clone()));
                        break;
                    }
                    _ => {}
                }
            }
        });
    });
    // Closing the review modal (X, Escape, or after cleanup) persists the
    // dismissal so this run never auto-prompts again; the manual entry points
    // on run cards and the runs panel stay available.
    create_effect(move |previous: Option<Option<String>>| {
        let current = run_review_modal.get();
        if let Some(Some(previous_id)) = previous {
            if current.as_deref() != Some(previous_id.as_str()) {
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({ "runId": previous_id })).unwrap();
                    let _ = invoke_checked("dismiss_run_review", args).await;
                });
            }
        }
        current
    });
    let runtime_environment_pinned = create_rw_signal(false);
    let runtime_environment_position = create_rw_signal((16, 16));
    let run_clock = create_rw_signal(now_secs());
    // The transfer tray needs the shared clock only while the active session
    // has an active or briefly lingering transfer. Once the last card expires,
    // this effect reruns with `clock_active = false` and drops its run_clock
    // dependency; historical progress records then stay idle between run-list
    // updates instead of rebuilding the tray every second.
    let transfer_tray_clock_active = create_rw_signal(false);
    let transfer_tray_now = create_rw_signal(run_clock.get_untracked());
    create_effect(move |_| {
        let clock_active = transfer_tray_clock_active.get();
        let now = if clock_active {
            run_clock.get()
        } else {
            run_clock.get_untracked()
        };
        let has_visible_transfer = active_session.get().is_some_and(|session_id| {
            run_records.with(|records| {
                records.iter().any(|run| {
                    run.frame_id.as_deref() == Some(session_id.as_str())
                        && run_progress(run).is_some_and(|progress| {
                            transfer_progress_visible(&progress, &run.status, now)
                        })
                })
            })
        });
        transfer_tray_now.set(now);
        if clock_active != has_visible_transfer {
            transfer_tray_clock_active.set(has_visible_transfer);
        }
    });
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
            if demo_mode.get_untracked() {
                return;
            }
            spawn_local(async move {
                let prefs_context_id = context_id.clone();
                let (session_id, created) = match active_session.get_untracked() {
                    Some(session_id) => (session_id, false),
                    None => match invoke_new_session().await {
                        Ok(session_id) => (session_id, true),
                        Err(error) => {
                            show_toast(&send_failed(locale.get_untracked(), &error));
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
                        // First enable of a server in this project: ask where
                        // uploads, run workdirs, and retrieved results go.
                        if enabled && prefs_context_id != "local" {
                            let args = to_value(&serde_json::json!({
                                "contextId": prefs_context_id.clone(),
                            }))
                            .unwrap();
                            if let Ok(value) =
                                invoke_checked("get_context_storage_prefs", args).await
                            {
                                if let Ok(prefs) =
                                    serde_wasm_bindgen::from_value::<ContextStoragePrefsView>(value)
                                {
                                    if !prefs.confirmed {
                                        let label = execution_contexts
                                            .get_untracked()
                                            .into_iter()
                                            .find(|context| context.id == prefs_context_id)
                                            .map(|context| context.label)
                                            .filter(|label| !label.trim().is_empty())
                                            .unwrap_or_else(|| prefs_context_id.clone());
                                        storage_prefs_form.set(Some(StoragePrefsForm::from_view(
                                            prefs, label, true,
                                        )));
                                    }
                                }
                            }
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

    let set_default_compute_resource = Callback::new(move |context_id: Option<String>| {
        if demo_mode.get_untracked() {
            return;
        }
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
            match invoke_checked("set_default_execution_context", args).await {
                Ok(value) => {
                    let Ok(saved) = serde_wasm_bindgen::from_value::<Option<String>>(value) else {
                        return;
                    };
                    default_execution_context.set(saved.clone());
                    // Make the new default usable in the current session right away.
                    if let Some(id) = saved {
                        apply_session_compute_resource.call((id, true));
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    show_toast(&message);
                }
            }
        });
    });

    let activate_terminal_session = Callback::new(move |session: TerminalSessionSummary| {
        let session_id = session.id.clone();
        terminal_sessions.update(|sessions| {
            if let Some(existing) = sessions.iter_mut().find(|item| item.id == session_id) {
                *existing = session;
            } else {
                sessions.push(session);
            }
        });
        active_terminal_id.set(Some(session_id));
        terminal_panel_open.set(true);
        terminal_add_menu_open.set(false);
    });

    let open_terminal_for_context = Callback::new(move |context_id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
            match invoke_checked("open_terminal", arg).await {
                Ok(value) => {
                    match serde_wasm_bindgen::from_value::<TerminalSessionSummary>(value) {
                        Ok(session) => activate_terminal_session.call(session),
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

    let close_terminal_session = Callback::new(move |session_id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
            match invoke_checked("close_terminal", arg).await {
                Ok(_) => {
                    let closing_active =
                        active_terminal_id.get_untracked().as_deref() == Some(session_id.as_str());
                    let mut next_active = None;
                    let mut sessions_empty = false;
                    let mut removed = false;
                    terminal_sessions.update(|sessions| {
                        let Some(index) =
                            sessions.iter().position(|session| session.id == session_id)
                        else {
                            return;
                        };
                        sessions.remove(index);
                        removed = true;
                        sessions_empty = sessions.is_empty();
                        if closing_active {
                            next_active = sessions
                                .get(index)
                                .or_else(|| sessions.last())
                                .map(|session| session.id.clone());
                        }
                    });
                    if removed && closing_active {
                        active_terminal_id.set(next_active);
                    }
                    if removed && sessions_empty {
                        terminal_add_menu_open.set(false);
                        terminal_panel_open.set(false);
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
    refresh_default_execution_context(default_execution_context);
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
        // UI liveness heartbeat for the backend watchdog: a webview whose
        // renderer died (process crash / WASM panic) stops beating and gets
        // reloaded; see `run_ui_watchdog` in src-tauri/src/lib.rs.
        let beat = Closure::wrap(Box::new(move || {
            spawn_local(async move {
                let _ = invoke("ui_heartbeat", JsValue::UNDEFINED).await;
            });
        }) as Box<dyn FnMut()>);
        if let Some(window) = web_sys::window() {
            let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                beat.as_ref().unchecked_ref(),
                5_000,
            );
            let _ = window.add_event_listener_with_callback("focus", beat.as_ref().unchecked_ref());
            if let Some(document) = window.document() {
                let _ = document.add_event_listener_with_callback(
                    "visibilitychange",
                    beat.as_ref().unchecked_ref(),
                );
            }
        }
        beat.forget();
    }
    {
        let ticks = Cell::new(0_u8);
        let refresh = Closure::wrap(Box::new(move || {
            run_clock.set(now_secs());
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
            // While a turn runs, agent python/r cells move runtimes between
            // starting/busy/ready outside any UI action; poll so the composer
            // strip and memory environment status stay current. The equality
            // guard in refresh_runtimes keeps unchanged polls from
            // republishing (and re-rendering) anything.
            if busy.get_untracked()
                || (show_right.get_untracked() && right_tab.get_untracked() == RightTab::Hosts)
            {
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
    // Re-underline saved excerpts when a structural/settled transcript revision
    // or the library changes. Token batches deliberately do not rescan the DOM.
    create_effect(move |_| {
        let _ = transcript_projection_epoch.get();
        if active_session
            .get()
            .is_some_and(|id| running.get().contains(&id))
        {
            return;
        }
        let texts = match active_session.get() {
            Some(session) => session_library_items.with(|items| {
                items
                    .iter()
                    .filter(|item| item.kind == "text" && item.source_session_id == session)
                    .map(|item| item.code.to_string())
                    .collect::<Vec<_>>()
            }),
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
        let demos = demos;
        let folder_modal = folder_modal;
        let folder_modal_input = folder_modal_input;
        let ui_confirm = ui_confirm;
        let active_session = active_session;
        let artifacts = artifacts;
        let db_artifacts = db_artifacts;
        let attachments = attachments;
        let branch_message_from_context_menu = branch_message.clone();
        Callback::new(move |(action, payload): (String, String)| {
            if action == "branchMessage" {
                if let Ok(ui_index) = payload.parse::<usize>() {
                    branch_message_from_context_menu(ui_index);
                }
                return;
            }
            if action == "quoteSelection" {
                let (source, text) = payload
                    .split_once('\u{1e}')
                    .unwrap_or(("", payload.as_str()));
                let source = (!source.is_empty()).then(|| source.to_string());
                composer_quotes.update(|items| {
                    items.push(ComposerQuote::from_selection(text, source.clone()))
                });
                clear_selection();
                if selection_targets_center_file(
                    source.as_deref(),
                    center_file.get_untracked().as_deref(),
                ) {
                    center_split.set(true);
                    show_right.set(false);
                }
                focus_composer();
                return;
            }
            if action == "quoteSelectionSideChat" {
                let (source, text) = payload
                    .split_once('\u{1e}')
                    .unwrap_or(("", payload.as_str()));
                let source = (!source.is_empty()).then(|| source.to_string());
                side_chat_quotes
                    .update(|items| items.push(ComposerQuote::from_selection(text, source)));
                clear_selection();
                ensure_right_tab(RightTab::SideChat, show_right, open_right_tabs, right_tab);
                focus_element_soon(SIDE_CHAT_INPUT_ID);
                return;
            }
            if action == "explainSelection" {
                clear_selection();
                send_side_chat((
                    t(locale.get(), "selection.explain_prompt").into(),
                    vec![ComposerQuote::plain(payload)],
                    false,
                ));
                return;
            }
            if action == "runQuickAction" {
                let mut parts = payload.splitn(3, '\u{1e}');
                let action_id = parts.next().unwrap_or_default().to_string();
                let source = parts
                    .next()
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let selection = parts.next().unwrap_or_default().to_string();
                run_quick_action.call((action_id, selection, source));
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
            if action == "openWorkspacePathInSystem" {
                open_workspace_path_in_system(payload);
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
            if matches!(
                action.as_str(),
                "attachWorkspaceFile" | "attachWorkspaceDirectory"
            ) {
                let _ = attach_ready_path(attachments, payload);
                focus_composer();
                return;
            }
            if action == "addWorkspaceFileToMotif" {
                if let Some(instance_id) = active_motif_instance(&mcp_apps.get_untracked()) {
                    spawn_local(async move {
                        match add_workspace_file_to_motif(&instance_id, &payload).await {
                            Ok(_) => show_toast(&tf(
                                locale.get_untracked(),
                                "motif.added_file",
                                &[(
                                    "name",
                                    payload.rsplit(['/', '\\']).next().unwrap_or(&payload),
                                )],
                            )),
                            Err(error) => show_warning_toast(&js_error_text(error)),
                        }
                    });
                } else {
                    let _ = attach_ready_path(attachments, payload.clone());
                    let filename = payload.rsplit(['/', '\\']).next().unwrap_or(&payload);
                    input.update(|draft| {
                        if !draft.trim().is_empty() {
                            draft.push_str("\n\n");
                        }
                        draft.push_str(&tf(
                            locale.get_untracked(),
                            "motif.open_and_add_prompt",
                            &[("name", filename)],
                        ));
                    });
                    show_warning_toast(&t(locale.get_untracked(), "motif.open_first"));
                    focus_composer();
                }
                return;
            }
            if action == "registerWorkspaceArtifact" {
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({
                        "path": payload,
                        "contentType": null,
                    }))
                    .unwrap();
                    match invoke_checked("register_artifact", arg).await {
                        Ok(value) => match serde_wasm_bindgen::from_value::<ArtifactInfo>(value) {
                            Ok(artifact) => {
                                let name = artifact.name.clone();
                                db_artifacts.update(|items| {
                                    items.retain(|item| item.id != artifact.id);
                                    items.insert(0, artifact);
                                });
                                show_toast(&tf(
                                    locale.get_untracked(),
                                    "artifact.registered",
                                    &[("name", &name)],
                                ));
                            }
                            Err(error) => show_warning_toast(&error.to_string()),
                        },
                        Err(error) => show_warning_toast(&localize_backend(
                            locale.get_untracked(),
                            &js_error_text(error),
                        )),
                    }
                });
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
                selected_workspace_paths.set(HashSet::new());
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
            if let Some(act) = context_menu::exploration_action(&action, &payload) {
                match act {
                    context_menu::ExplorationAction::Open(id) => {
                        if let Some(exploration) = explorations.with_untracked(|rows| {
                            rows.iter()
                                .find(|row| row.exploration.id == id)
                                .map(|row| row.exploration.clone())
                        }) {
                            open_exploration.call(exploration);
                        }
                    }
                    context_menu::ExplorationAction::SelectAsMainline(id)
                    | context_menu::ExplorationAction::ViewDiff(id) => {
                        open_exploration_preview.call(id);
                    }
                    context_menu::ExplorationAction::Discard(id) => {
                        open_exploration_preview.call(id);
                    }
                }
                return;
            }
            if let Some(context_menu::DemoAction::CopyToProject(id)) =
                context_menu::demo_action(&action, &payload)
            {
                let title = demos
                    .get()
                    .into_iter()
                    .find(|demo| demo.id == id)
                    .map(|demo| demo.title)
                    .unwrap_or_else(|| id.clone());
                session_transfer_error.set(None);
                session_transfer.set(Some(SessionTransfer {
                    id,
                    title,
                    mode: SessionTransferMode::Copy,
                    target_project_id: String::new(),
                    from_demo: true,
                }));
                spawn_local(async move {
                    let value = invoke("list_projects", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(value) {
                        let default_target = list
                            .first()
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
                return;
            }
            if let Some(act) = context_menu::session_action(&action, &payload) {
                match act {
                    context_menu::SessionAction::Open(id) => open_session.call(id),
                    context_menu::SessionAction::AbandonExploration(id) => {
                        ui_confirm.set(Some(UiConfirm::AbandonExploration(id)));
                    }
                    context_menu::SessionAction::MergeBranch(id) => {
                        branch_merge_open.set(Some(id.clone()));
                        branch_merge_preview.set(None);
                        branch_merge_draft.set(String::new());
                        branch_merge_error.set(None);
                        branch_merge_busy.set(false);
                        branch_merge_guidance_open.set(false);
                        branch_merge_guidance.set(String::new());
                        spawn_local(async move {
                            let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                            match invoke_checked("preview_session_branch_merge", args).await {
                                Ok(value) => match from_value::<SessionBranchMergePreview>(value) {
                                    Ok(result) => {
                                        let guard_hash = result.guard_hash.clone();
                                        if branch_merge_open.get_untracked().as_deref()
                                            != Some(id.as_str())
                                        {
                                            return;
                                        }
                                        branch_merge_preview.set(Some(result));
                                        generate_branch_summary.call((id, guard_hash, None, None));
                                    }
                                    Err(error) => {
                                        if branch_merge_open.get_untracked().as_deref()
                                            == Some(id.as_str())
                                        {
                                            branch_merge_error.set(Some(error.to_string()));
                                        }
                                    }
                                },
                                Err(error) => {
                                    if branch_merge_open.get_untracked().as_deref()
                                        == Some(id.as_str())
                                    {
                                        branch_merge_error.set(Some(localize_backend(
                                            locale.get_untracked(),
                                            &js_error_text(error),
                                        )));
                                    }
                                }
                            }
                        });
                    }
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
                            from_demo: false,
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
                            let arg = to_value(&serde_json::json!({ "id": id, "pinned": pinned }))
                                .unwrap();
                            if invoke_checked("set_session_pinned", arg).await.is_ok() {
                                refresh_session_history();
                            }
                        });
                    }
                    context_menu::SessionAction::ReloadProjectRules(id) => {
                        ui_confirm.set(Some(UiConfirm::ReloadProjectRules(id)));
                    }
                    context_menu::SessionAction::Delete(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteSessions(vec![id])));
                    }
                    context_menu::SessionAction::DeleteBranch(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteSessions(vec![id])));
                    }
                }
            }
            context_menu::run_action(&action, &payload, copy_text);
        })
    };
    let on_context_menu = move |ev: web_sys::MouseEvent| {
        if context_menu::uses_native_text_menu(&ev) {
            ctx_menu.set(None);
            return;
        }
        let loc = locale.get();
        let center = center_file.get_untracked();
        let project_root = project_info.get_untracked().map(|project| project.root);
        let selected_paths = if selecting_workspace_entries.get_untracked() {
            selected_workspace_paths
                .get_untracked()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if let Some(menu) = context_menu::build(
            &ev,
            loc,
            active_session.get().is_some(),
            center.as_deref(),
            &quick_actions.get_untracked(),
            project_root.as_deref(),
            &selected_paths,
        ) {
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

    // Feed ime_composing: a keyCode-229 keydown right after this is the IME
    // confirm key; a 229 keydown far from it is a real key (see text.rs).
    window_event_listener(ev::compositionend, |ev| {
        note_composition_end(ev.time_stamp());
    });

    // Escape stack: visually topmost surface → menus → drag cancel → right
    // pane → approval reject last. Component-owned inner surfaces consume
    // Escape through `window_capture_escape` before this handler runs.
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
        // Topmost: the link confirmation can be raised from any surface.
        if external_link_confirm.get().is_some() {
            ev.prevent_default();
            external_link_confirm.set(None);
            return;
        }
        if browser_needs_human.get().is_some() {
            ev.prevent_default();
            if !browser_needs_human_busy.get() {
                browser_needs_human.set(None);
                browser_needs_human_error.set(None);
            }
            return;
        }
        if browser_tab_cleanup.get().is_some() {
            ev.prevent_default();
            if !browser_tab_cleanup_busy.get() {
                if let Some(prompt) = browser_tab_cleanup.get_untracked() {
                    let turn_id = prompt.turn_id.clone();
                    spawn_local(async move {
                        let arg = to_value(&serde_json::json!({ "turnId": turn_id })).unwrap();
                        let _ = invoke_checked("dismiss_browser_tab_cleanup", arg).await;
                    });
                }
                advance_browser_tab_cleanup(
                    browser_tab_cleanup,
                    browser_tab_cleanup_queue,
                    browser_tab_cleanup_selected,
                    browser_tab_cleanup_error,
                    browser_tab_cleanup_busy,
                );
            }
            return;
        }
        if turn_memory_proposal.get().is_some() {
            ev.prevent_default();
            if !turn_memory_busy.get() {
                turn_memory_proposal.set(None);
                turn_memory_editor.set(String::new());
                turn_memory_replace_id.set(String::new());
                turn_memory_error.set(None);
            }
            return;
        }
        if context_recovery_dialog.get().is_some() {
            ev.prevent_default();
            if !context_recovery_busy.get() {
                context_recovery_dialog.set(None);
                context_recovery_error.set(None);
            }
            return;
        }
        // The model switch confirm renders above the export/link/memory
        // dialogs and everything below; Escape cancels the switch only.
        if model_switch_confirm.get().is_some() {
            ev.prevent_default();
            model_switch_confirm.set(None);
            return;
        }
        if project_export_prompt.get().is_some() {
            ev.prevent_default();
            project_export_prompt.set(None);
            return;
        }
        if selection_popup.get().is_some() {
            ev.prevent_default();
            selection_popup.set(None);
            return;
        }
        if ctx_menu.get().is_some() {
            ev.prevent_default();
            ctx_menu.set(None);
            return;
        }
        if share_draft.get().is_some() {
            ev.prevent_default();
            share_draft.set(None);
            return;
        }
        if trajectory_open.get() {
            ev.prevent_default();
            trajectory_open.set(false);
            return;
        }
        if let Some(modal) = update_check_modal.get() {
            ev.prevent_default();
            if modal.dismissible() {
                update_check_modal.set(None);
            }
            return;
        }
        if show_session_import.get().is_some() {
            ev.prevent_default();
            show_session_import.set(None);
            return;
        }
        if privacy_mode_modal_open.get() {
            ev.prevent_default();
            privacy_mode_modal_open.set(false);
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
        if scratch_open.get() {
            ev.prevent_default();
            close_scratch.call(());
            return;
        }

        if branch_merge_detail.get().is_some() {
            ev.prevent_default();
            branch_merge_detail.set(None);
            return;
        }
        if branch_merge_open.get().is_some() {
            ev.prevent_default();
            if !branch_merge_busy.get() {
                branch_merge_open.set(None);
                branch_merge_preview.set(None);
                branch_merge_draft.set(String::new());
                branch_merge_error.set(None);
            }
            return;
        }
        if exploration_overlay.get().is_some() {
            ev.prevent_default();
            if !exploration_busy.get() {
                exploration_overlay.set(None);
                exploration_preview.set(None);
                exploration_error.set(None);
            }
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
        if run_review_modal.get().is_some() {
            ev.prevent_default();
            run_review_modal.set(None);
            return;
        }
        if storage_prefs_form.get().is_some() {
            ev.prevent_default();
            storage_prefs_form.set(None);
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
        if modal_artifact.get().is_some() {
            ev.prevent_default();
            modal_artifact.set(None);
            return;
        }
        if show_publication_workspace.get() {
            ev.prevent_default();
            show_publication_workspace.set(false);
            publication_binding_source.set(None);
            return;
        }
        if show_research_graph.get() {
            ev.prevent_default();
            show_research_graph.set(false);
            return;
        }
        if inbox_open.get() {
            ev.prevent_default();
            inbox_open.set(false);
            return;
        }
        if show_settings.get() && !settings_busy.get() {
            ev.prevent_default();
            show_settings.set(false);
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
        if show_library.get() {
            ev.prevent_default();
            show_library.set(false);
            return;
        }

        if show_projects.get() {
            if project_transfer
                .get()
                .is_some_and(|transfer| transfer.is_complete() || transfer.is_failed())
            {
                ev.prevent_default();
                project_transfer.set(None);
            }
            return;
        }

        // --- overlays (most interrupting first) ---
        if turn_undo_dialog.get().is_some() && !turn_undo_busy.get() {
            ev.prevent_default();
            turn_undo_dialog.set(None);
            turn_undo_error.set(None);
            return;
        }
        if edit_confirm.get().is_some() {
            ev.prevent_default();
            edit_confirm.set(None);
            return;
        }
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
        if context_usage_open.get() {
            ev.prevent_default();
            context_usage_open.set(false);
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
        if reviewer_model_menu_open.get() || compute_menu_open.get() || specialist_menu_open.get() {
            ev.prevent_default();
            reviewer_model_menu_open.set(false);
            compute_menu_open.set(false);
            specialist_menu_open.set(false);
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
        if effort_menu_for.get().is_some() {
            ev.prevent_default();
            effort_menu_for.set(None);
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
        if runtime_environment_pinned.get() {
            ev.prevent_default();
            runtime_environment.set(None);
            runtime_environment_pinned.set(false);
            return;
        }
        if terminal_add_menu_open.get() {
            ev.prevent_default();
            terminal_add_menu_open.set(false);
            return;
        }
        if conversation_outline_open.get() {
            ev.prevent_default();
            conversation_outline_open.set(false);
            return;
        }

        // --- drag cancel ---
        if dragging.get() {
            ev.prevent_default();
            dragging.set(false);
            return;
        }
        if center_split_dragging.get() {
            ev.prevent_default();
            center_split_dragging.set(false);
            return;
        }
        if center_runtime_col_dragging.get() || center_runtime_row_dragging.get() {
            ev.prevent_default();
            center_runtime_col_dragging.set(false);
            center_runtime_row_dragging.set(false);
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

        // A finished background transfer is a low-priority, non-modal card.
        // Let every visible overlay, menu, drag, and pane consume Escape first.
        if project_transfer
            .get()
            .is_some_and(|transfer| transfer.is_complete() || transfer.is_failed())
        {
            ev.prevent_default();
            project_transfer.set(None);
            return;
        }

        if browser_offline_notice.get().is_some() {
            ev.prevent_default();
            browser_offline_notice.set(None);
            return;
        }
        if let Some(status) = browser_extension_status.get() {
            ev.prevent_default();
            browser_extension_dismissed_version
                .set(Some(status.bundled_version.unwrap_or_default()));
            browser_extension_status.set(None);
            browser_extension_update_outcome.set(None);
            browser_extension_update_error.set(None);
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
    // issue #97). Every render path (chat markdown, file preview, right pane,
    // review) lands here, so this is also where the destination — usually
    // model-authored — is confirmed before the OS handler sees it. The app is
    // a single-page UI: no anchor may ever navigate the webview itself, so
    // the default is always suppressed. Relative paths, "#", and javascript:
    // are simply inert.
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
                    ev.prevent_default();
                    if opens_in_system_browser(&href) {
                        external_link_confirm.set(Some(href));
                    }
                }
                return;
            }
            el = n.parent_element();
        }
    });

    // Docked panel dismisses on an outside click, but the click itself must
    // still land (caret in the input, session switch, inspector toggle).
    window_event_listener(ev::click, move |ev| {
        if context_usage_suppress_click.get_untracked() {
            context_usage_suppress_click.set(false);
            return;
        }
        if !context_usage_open.get_untracked()
            || context_usage_mode.get_untracked() != ContextUsageMode::Docked
            || context_usage_dragging.get_untracked()
            || context_usage_resizing.get_untracked()
        {
            return;
        }
        if event_inside_selector(&ev, "[data-testid='context-usage-panel']")
            || event_inside_selector(&ev, "[data-testid='context-usage-trigger']")
        {
            return;
        }
        context_usage_open.set(false);
    });

    window_event_listener(ev::mousemove, move |ev| {
        if context_usage_tracking.get() || context_usage_dragging.get() {
            on_context_usage_drag_move(ev);
        }
    });
    window_event_listener(ev::mouseup, move |ev| {
        if context_usage_tracking.get() || context_usage_dragging.get() {
            on_context_usage_drag_end(ev);
        }
    });

    window_event_listener(ev::resize, move |_| {
        let (viewport_w, viewport_h) = viewport_size();
        if let Some(geom) = context_usage_geom.get_untracked() {
            let clamped =
                clamp_context_usage_geom(geom.x, geom.y, geom.w, geom.h, viewport_w, viewport_h);
            if clamped != geom {
                context_usage_geom.set(Some(clamped));
            }
        }
        // The pinned R/Python inspector stores pixel coordinates from the
        // current window. Restoring from maximize leaves those coordinates
        // past the new right/bottom edge, so the panel vanishes until the
        // window is maximized again.
        if runtime_environment_pinned.get_untracked() {
            let (x, y) = runtime_environment_position.get_untracked();
            let next =
                clamp_runtime_environment_position_in(x, y, viewport_w as i32, viewport_h as i32);
            if next != (x, y) {
                runtime_environment_position.set(next);
            }
        }
    });

    // Selecting text inside any file preview (tagged `data-file-path`) raises the
    // same quote popup the chat uses. Papers also get annotate / literature;
    // R/Python source gets Run instead. Runs on window
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
        // File links and buttons own the click (image preview, artifact chip).
        // Do not also raise the quote bar from a leftover selection of the
        // control's label — that stacks on top of the preview.
        if context_menu::selection_popup_blocked(&ev) {
            if matches!(selection_popup.get_untracked(), Some((_, Some(_), _, _))) {
                selection_popup.set(None);
            }
            return;
        }
        let json = preview_selection(ev.client_x(), ev.client_y());
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

    // Image comment pins → one revision-request quote in the composer, the
    // same landing as "Ask AI in the conversation". A center-pane preview
    // stays open beside the chat (like ask-AI); a modal preview closes.
    window_event_listener_untyped("wisp:pins-ask-ai", move |ev| {
        use wasm_bindgen::JsCast;
        let Some(detail) = ev
            .dyn_ref::<web_sys::CustomEvent>()
            .and_then(|ce| serde_wasm_bindgen::from_value::<PinsAskAi>(ce.detail()).ok())
        else {
            return;
        };
        composer_quotes.update(|items| items.push(ComposerQuote::plain(detail.text)));
        modal_artifact.set(None);
        if center_file.get_untracked().as_deref() == Some(detail.path.as_str()) {
            center_split.set(true);
            show_right.set(false);
        }
        focus_composer();
    });

    // User-initiated scroll (wheel/trackpad) — not follow-scroll — should
    // hide the fixed quote popup, whose client coordinates are now stale.
    window_event_listener(ev::wheel, move |_| {
        if selection_popup.get_untracked().is_some() {
            selection_popup.set(None);
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
            if project_transfer
                .get_untracked()
                .is_some_and(|transfer| transfer.is_exporting_project(&project_id))
            {
                let message = t(locale.get_untracked(), "projects.transfer.export_locked");
                project_open_error.set(Some(message.clone()));
                status.set(message);
                return;
            }
            let request_epoch = transition_epoch.get().wrapping_add(1);
            transition_epoch.set(request_epoch);
            *transition_target.borrow_mut() = Some(project_id.clone());

            project_open_error.set(None);
            status.set(String::new());
            show_proj_menu.set(false);
            show_research_graph.set(false);
            research_graph.set(ResearchGraph::default());
            demo_mode.set(false);
            // Move the visible rows into the inactive cache so background
            // sessions keep streaming without cloning a long transcript.
            replace_visible_transcript(
                active_session.get_untracked(),
                None,
                Vec::new(),
                items,
                transcripts,
                running,
            );
            active_session.set(None);
            collapsed_folders.set(HashSet::new());
            selecting_workspace_entries.set(false);
            selected_workspace_paths.set(HashSet::new());
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

                let session_id = match session_id {
                    Some(session_id) => Some(session_id),
                    None if settings.get_untracked().resume_last_session => {
                        invoke_latest_used_session().await
                    }
                    None => None,
                };
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
    // session in an already-open project window (#423). This listener must be
    // window-scoped: the generic event listener is app-wide, so a targeted
    // completion navigation would otherwise repoint every project window.
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
        let _ = listen_current_window("open-session", &open_session_js).await;
    });
    // Cross-project conversation opens may still target the project's own
    // window (#423). Choosing a workspace itself always repoints this window.
    let opens_in_project_window = move |project_id: &str| -> bool {
        !show_projects.get_untracked()
            && matches!(project_info.get_untracked(), Some(p) if p.id != project_id)
    };
    // Workspace pickers always switch the active project in this window. New
    // windows remain available only through actions explicitly labelled as such.
    let switch_project = {
        let open_project_transition = open_project_transition;
        Callback::new(move |id: String| {
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
            let v = invoke(
                "get_project_settings",
                to_value(&serde_json::json!({})).unwrap(),
            )
            .await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<ProjectSettings>(v) {
                proj_settings_baseline.set(s.clone());
                proj_settings.set(s);
                show_proj_settings.set(true);
            }
        });
    };
    let commit_proj_settings = move || {
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
    let save_proj_settings = move |_| {
        let form = proj_settings.get();
        let baseline = proj_settings_baseline.get();
        if form.agent_context.trim() != baseline.agent_context.trim() {
            ui_confirm.set(Some(UiConfirm::SaveAgentContext));
            return;
        }
        commit_proj_settings();
    };

    let move_sessions_to = {
        Callback::new(
            move |(session_ids, folder_id): (Vec<String>, Option<String>)| {
                spawn_local(async move {
                    let mut moved = false;
                    for session_id in session_ids {
                        let arg = to_value(&serde_json::json!({
                            "id": session_id,
                            "folderId": folder_id,
                        }))
                        .unwrap();
                        moved |= invoke_checked("move_session", arg).await.is_ok();
                    }
                    if moved {
                        refresh_session_history();
                    }
                });
            },
        )
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

    let save_session_transfer = {
        let open_project_transition = open_project_transition;
        move |_| {
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
                if transfer.from_demo {
                    let args = to_value(&serde_json::json!({
                        "id": transfer.id,
                        "targetProjectId": transfer.target_project_id,
                    }))
                    .unwrap();
                    match invoke_checked("copy_demo_to_project", args).await {
                        Ok(value) => {
                            let session_id =
                                serde_wasm_bindgen::from_value::<String>(value).unwrap_or_default();
                            status.set(tf(
                                locale.get(),
                                "session.copy_demo_success",
                                &[("project", &target_name)],
                            ));
                            session_transfer.set(None);
                            session_transfer_busy.set(false);
                            if !session_id.is_empty() {
                                open_project_transition
                                    .call((transfer.target_project_id, Some(session_id)));
                            }
                        }
                        Err(error) => {
                            session_transfer_error
                                .set(Some(localize_backend(locale.get(), &js_error_text(error))));
                            session_transfer_busy.set(false);
                        }
                    }
                    return;
                }
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
        }
    };

    let palette_open_session = {
        let open_project_transition = open_project_transition;
        Callback::new(move |(project_id, session_id): (String, String)| {
            if project_transfer
                .get_untracked()
                .is_some_and(|transfer| transfer.is_exporting_project(&project_id))
            {
                status.set(t(locale.get_untracked(), "projects.transfer.export_locked").into());
                return;
            }
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
            if project_transfer
                .get_untracked()
                .is_some_and(|transfer| transfer.is_exporting_project(&project_id))
            {
                status.set(t(locale.get_untracked(), "projects.transfer.export_locked").into());
                return;
            }
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
                if project_transfer
                    .get_untracked()
                    .is_some_and(|transfer| transfer.is_exporting_project(&project_id))
                {
                    status.set(t(locale.get_untracked(), "projects.transfer.export_locked").into());
                    return;
                }
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
        if demo_mode.get_untracked() {
            return;
        }
        attachments.set(vec![]);
        composer_references.set(vec![]);
        composer_quotes.set(vec![]);
        sel_artifact.set(0);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let id = match invoke_new_session().await {
                Ok(id) => id,
                Err(error) => {
                    status.set(send_failed(locale.get(), &error));
                    return;
                }
            };
            replace_visible_transcript(
                active_session.get_untracked(),
                None,
                Vec::new(),
                items,
                transcripts,
                running,
            );
            active_session.set(Some(id));
            refresh_session_history();
            focus_composer();
        });
    });
    let palette_project_settings = Callback::new(move |_: ()| {
        spawn_local(async move {
            let v = invoke(
                "get_project_settings",
                to_value(&serde_json::json!({})).unwrap(),
            )
            .await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<ProjectSettings>(v) {
                proj_settings_baseline.set(s.clone());
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
    let start_project_export = Callback::new(move |id: String| {
        if project_transfer
            .get_untracked()
            .is_some_and(|transfer| transfer.is_active())
        {
            return;
        }
        project_export_prompt.set(None);
        project_open_error.set(None);
        project_transfer.set(Some(ProjectTransferProgress::selecting(
            "export",
            Some(id.clone()),
        )));
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
            match invoke_checked("export_project", args).await {
                Ok(value) => {
                    if let Ok(Some(path)) = serde_wasm_bindgen::from_value::<Option<String>>(value)
                    {
                        project_transfer.set(Some(ProjectTransferProgress::complete(
                            "export",
                            Some(id),
                            Some(path),
                        )));
                    } else {
                        project_transfer.set(None);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    status.set(message.clone());
                    project_transfer.set(Some(ProjectTransferProgress::failed(
                        "export",
                        Some(id),
                        message,
                    )));
                }
            }
        });
    });
    let open_project_export = Callback::new(move |(id, workspace): (String, String)| {
        if project_transfer
            .get_untracked()
            .is_some_and(|transfer| transfer.is_active())
        {
            return;
        }
        project_open_error.set(None);
        project_export_prompt.set(Some((id, workspace)));
    });
    let export_current_project = {
        let open_project_export = open_project_export;
        Callback::new(move |_: ()| {
            if show_projects.get_untracked() || demo_mode.get_untracked() {
                return;
            }
            let Some(project) = project_info.get_untracked() else {
                return;
            };
            open_project_export.call((project.id, project.root));
        })
    };
    let palette_attach = Callback::new(move |reference: ComposerReferenceChip| {
        if !composer_references
            .get()
            .iter()
            .any(|item| item.key() == reference.key())
        {
            composer_references.update(|items| items.push(reference));
        }
    });
    #[derive(Default, serde::Deserialize)]
    struct SessionArchiveImportSummary {
        frame_id: String,
        status: String,
        message_count: usize,
    }
    // One-shot requests from the Windows titlebar File menu into the projects
    // landing screen, which owns the create/import dialogs.
    let menu_new_project = create_rw_signal(false);
    let menu_import_project = create_rw_signal(false);
    let palette_action = {
        let new_session = palette_new_session.clone();
        let open_scratch = open_scratch.clone();
        let project_settings = palette_project_settings.clone();
        let manage_skills = palette_manage_skills.clone();
        let run_update_check = run_update_check.clone();
        let export_current_project = export_current_project.clone();
        Callback::new(move |action: &'static str| match action {
            // On the projects landing, "new" can only mean a new project —
            // there is no workspace to hold a session yet.
            "new" => {
                if show_projects.get_untracked() {
                    menu_new_project.set(true);
                } else {
                    new_session.call(())
                }
            }
            "new-project" => {
                if show_projects.get_untracked() {
                    menu_new_project.set(true);
                }
            }
            "import-project" => {
                if show_projects.get_untracked() {
                    menu_import_project.set(true);
                }
            }
            "scratch" => open_scratch.call(()),
            "search" => command_palette_open.set(true),
            "commands" => action_palette_open.set(true),
            "projects" => show_projects.set(true),
            "library" => show_library.update(|show| *show = !*show),
            "settings" => {
                show_settings.set(true);
                settings_section.set("models".into());
            }
            "privacy-mode" => privacy_mode_modal_open.set(true),
            "import-codex" => {
                if project_info.get_untracked().is_some() && !demo_mode.get_untracked() {
                    show_session_import.set(Some(SessionImportProvider::Codex));
                }
            }
            "import-claude" => {
                if project_info.get_untracked().is_some() && !demo_mode.get_untracked() {
                    show_session_import.set(Some(SessionImportProvider::Claude));
                }
            }
            "import-session" => {
                if let Some(project) = project_info
                    .get_untracked()
                    .filter(|_| !demo_mode.get_untracked())
                {
                    spawn_local(async move {
                        let arg = to_value(&serde_json::json!({})).unwrap();
                        let value = match invoke_checked("import_session_archive", arg).await {
                            Ok(value) => value,
                            Err(error) => {
                                show_toast(&localize_backend(
                                    locale.get_untracked(),
                                    &js_error_text(error),
                                ));
                                return;
                            }
                        };
                        if value.is_null() {
                            return;
                        }
                        let summary =
                            serde_wasm_bindgen::from_value::<SessionArchiveImportSummary>(value)
                                .unwrap_or_default();
                        let loc = locale.get_untracked();
                        let key = match summary.status.as_str() {
                            "imported" => "import.session_imported",
                            "updated" => "import.session_updated",
                            _ => "import.session_skipped",
                        };
                        show_toast(&tf(loc, key, &[("n", &summary.message_count.to_string())]));
                        refresh_sessions(
                            sessions,
                            pending_turns,
                            running,
                            session_history_cursor,
                            active_session,
                            exploration_frames,
                        );
                        refresh_folders(folders);
                        if summary.status != "skipped" && !summary.frame_id.is_empty() {
                            open_project_transition.call((project.id, Some(summary.frame_id)));
                        }
                    });
                }
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
            "font-ui-increase" => ui_font_size.update(|size| *size = (*size + 1).min(30)),
            "font-ui-decrease" => ui_font_size.update(|size| *size = size.saturating_sub(1)),
            "font-code-increase" => code_font_size.update(|size| *size = (*size + 1).min(30)),
            "font-code-decrease" => code_font_size.update(|size| *size = size.saturating_sub(1)),
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
                        "import-codex" => Some("import-codex"),
                        "import-claude" => Some("import-claude"),
                        "import-session" => Some("import-session"),
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
        scratch_open.get()
            || (project_info.get().is_some() && !show_projects.get() && !demo_mode.get())
    });
    let home_page = Signal::derive(move || show_projects.get());
    let window_title = Signal::derive(move || {
        if scratch_open.get() {
            app_window_title(Some("Scratch"))
        } else if show_projects.get() {
            app_window_title(None)
        } else if demo_mode.get() {
            app_window_title(Some(&t(locale.get(), "projects.example")))
        } else {
            app_window_title(project_info.get().as_ref().map(|p| p.name.as_str()))
        }
    });
    create_effect(move |_| {
        let title = window_title.get();
        spawn_local(async move {
            set_window_title(&title).await;
        });
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
            "h" if ev.shift_key() => {
                ev.prevent_default();
                command_palette_open.set(false);
                action_palette_open.set(false);
                privacy_mode_modal_open.set(true);
            }
            "k" => {
                ev.prevent_default();
                action_palette_open.set(false);
                command_palette_open.update(|open| *open = !*open);
            }
            "n" => {
                ev.prevent_default();
                if ev.shift_key() {
                    open_scratch.call(());
                } else {
                    shortcut_action.call("new");
                }
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

    // Undo eligibility changes at turn boundaries, but the assistant Markdown
    // does not. Publish the one eligible index separately so adding/removing
    // its button never remounts and reparses the whole message row.
    let undo_assistant_index = create_memo(move |_| {
        if busy.get() || active_acp_agent_id.get().is_some() {
            return None;
        }
        items.with(|list| {
            let queue_start = trailing_queue_start(list);
            (queue_start == list.len())
                .then(|| {
                    list.iter().enumerate().rev().find_map(|(index, item)| {
                        matches!(
                            item,
                            ChatItem::Assistant { text, .. }
                                if !text.trim().is_empty() && !text.starts_with("Error: ")
                        )
                        .then_some(index)
                    })
                })
                .flatten()
        })
    });

    let confirm_turn_memory = Callback::new(move |_: ()| {
        let Some(draft) = turn_memory_proposal.get_untracked() else {
            return;
        };
        let content = turn_memory_editor.get_untracked().trim().to_string();
        if content.is_empty() {
            turn_memory_error.set(Some(t(locale.get_untracked(), "memory.proposal.empty")));
            return;
        }
        let scope = turn_memory_scope.get_untracked();
        let global_scope = scope == "global";
        let replace_id = global_scope
            .then(|| turn_memory_replace_id.get_untracked())
            .filter(|id| !id.trim().is_empty());
        turn_memory_busy.set(true);
        turn_memory_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sessionId": draft.session_id,
                "turnIndex": draft.turn_index,
                "scope": scope,
                "content": content,
                "replaceId": replace_id,
            }))
            .unwrap();
            match invoke_checked("confirm_turn_memory", args).await {
                Ok(_) => {
                    turn_memory_proposal.set(None);
                    turn_memory_editor.set(String::new());
                    turn_memory_replace_id.set(String::new());
                    show_toast(&t(
                        locale.get_untracked(),
                        if global_scope {
                            "memory.proposal.saved_global"
                        } else {
                            "memory.proposal.saved"
                        },
                    ));
                }
                Err(error) => turn_memory_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ))),
            }
            turn_memory_busy.set(false);
        });
    });

    view! {
        {is_windows().then(|| view! {
            <WindowTitlebar locale=locale has_current_project=has_current_project
                home=home_page brand=window_title on_action=palette_action.clone() />
        })}
        <ActionPalette open=action_palette_open has_current_project=has_current_project
            on_action=palette_action />
        <CommandPalette open=command_palette_open current_project_id=palette_project_id
            privacy_mode_active=privacy_mode_active
            privacy_hidden_project_ids=privacy_hidden_project_ids
            on_open_project=command_palette_open_project on_open_session=command_palette_open_session on_open_artifact=palette_open_artifact
            on_command=palette_action
            on_new_session=palette_new_session on_open_scratch=open_scratch
            on_project_settings=palette_project_settings
            on_manage_skills=palette_manage_skills on_attach=palette_attach />
        <PrivacyModeModal
            open=privacy_mode_modal_open
            active=privacy_mode_active
            hidden_project_ids=privacy_hidden_project_ids
            on_hide=Callback::new(move |project_ids: HashSet<String>| {
                save_privacy_mode(true, &project_ids);
                privacy_hidden_project_ids.set(project_ids);
                privacy_mode_active.set(true);
                privacy_mode_modal_open.set(false);
            })
            on_restore=Callback::new(move |_| {
                privacy_mode_active.set(false);
                privacy_hidden_project_ids.with_untracked(|ids| save_privacy_mode(false, ids));
                privacy_mode_modal_open.set(false);
            })
        />
        <ProjectExportPrompt
            state=ProjectExportPromptState { locale, prompt: project_export_prompt }
            on_export_zip=start_project_export
            on_copy_path=Callback::new(move |path: String| {
                copy_text(path);
                show_toast(&t(locale.get_untracked(), "projects.folder_path_copied"));
            })
        />
        <ProjectTransferOverlay state=ProjectTransferOverlayState { locale, project_transfer } />
        <ExternalLinkConfirm locale=locale pending=external_link_confirm />
        <BrowserTabCleanupOverlay
            state=BrowserTabCleanupOverlayState {
                locale,
                pending: browser_tab_cleanup,
                selected: browser_tab_cleanup_selected,
                busy: browser_tab_cleanup_busy,
                error: browser_tab_cleanup_error,
            }
            on_keep=Callback::new(move |_| {
                if browser_tab_cleanup_busy.get_untracked() {
                    return;
                }
                let Some(prompt) = browser_tab_cleanup.get_untracked() else {
                    return;
                };
                browser_tab_cleanup_busy.set(true);
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "turnId": prompt.turn_id })).unwrap();
                    let _ = invoke_checked("dismiss_browser_tab_cleanup", arg).await;
                    advance_browser_tab_cleanup(
                        browser_tab_cleanup,
                        browser_tab_cleanup_queue,
                        browser_tab_cleanup_selected,
                        browser_tab_cleanup_error,
                        browser_tab_cleanup_busy,
                    );
                });
            })
            on_close=Callback::new(move |tabs: Vec<BrowserTabCleanupItem>| {
                if browser_tab_cleanup_busy.get_untracked() {
                    return;
                }
                let Some(prompt) = browser_tab_cleanup.get_untracked() else {
                    return;
                };
                browser_tab_cleanup_busy.set(true);
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({
                        "turnId": prompt.turn_id,
                        "tabs": tabs,
                    })).unwrap();
                    match invoke_checked("confirm_browser_tab_cleanup", arg).await {
                        Ok(_) => advance_browser_tab_cleanup(
                            browser_tab_cleanup,
                            browser_tab_cleanup_queue,
                            browser_tab_cleanup_selected,
                            browser_tab_cleanup_error,
                            browser_tab_cleanup_busy,
                        ),
                        Err(err) => {
                            browser_tab_cleanup_busy.set(false);
                            browser_tab_cleanup_error.set(Some(js_error_text(err)));
                        }
                    }
                });
            })
        />
        <BrowserNeedsHumanOverlay
            state=BrowserNeedsHumanOverlayState {
                locale,
                pending: browser_needs_human,
                busy: browser_needs_human_busy,
                error: browser_needs_human_error,
            }
            on_later=Callback::new(move |_| {
                if browser_needs_human_busy.get_untracked() {
                    return;
                }
                browser_needs_human.set(None);
                browser_needs_human_error.set(None);
            })
            on_show=Callback::new(move |tab: BrowserNeedsHumanTab| {
                if browser_needs_human_busy.get_untracked() {
                    return;
                }
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({
                        "session": tab.session,
                        "tabId": tab.tab_id,
                    })).unwrap();
                    let _ = invoke_checked("focus_browser_needs_human", arg).await;
                });
            })
            on_done=Callback::new(move |tabs: Vec<BrowserNeedsHumanTab>| {
                if browser_needs_human_busy.get_untracked() {
                    return;
                }
                browser_needs_human_busy.set(true);
                browser_needs_human_error.set(None);
                let continue_message = t(locale.get_untracked(), "browser.needs_human.continue");
                let still_message = t(locale.get_untracked(), "browser.needs_human.still");
                let fail_message = t(locale.get_untracked(), "browser.needs_human.error");
                let fallback_session = active_session.get_untracked();
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "tabs": tabs })).unwrap();
                    match invoke_checked("confirm_browser_needs_human", arg).await {
                        Ok(value) => {
                            let result = serde_wasm_bindgen::from_value::<BrowserNeedsHumanConfirmResult>(value)
                                .unwrap_or_default();
                            browser_needs_human_busy.set(false);
                            if result.still_required.is_empty() {
                                let frame_id = result
                                    .cleared
                                    .first()
                                    .map(|tab| tab.frame_id.clone())
                                    .filter(|id| !id.is_empty())
                                    .or(fallback_session);
                                browser_needs_human.set(None);
                                browser_needs_human_error.set(None);
                                if let Some(session_id) = frame_id {
                                    let args = to_value(&SendMessageArgs {
                                        session_id: Some(session_id),
                                        message: continue_message,
                                        attachments: vec![],
                                        references: vec![],
                                        resume: false,
                                        acp_agent_id: None,
                                        guide: None,
                                        replace: None,
                                    }).unwrap();
                                    let _ = invoke_checked("send_message", args).await;
                                }
                            } else {
                                present_browser_needs_human(
                                    browser_needs_human,
                                    browser_needs_human_error,
                                    BrowserNeedsHumanPrompt { tabs: result.still_required },
                                );
                                browser_needs_human_error.set(Some(still_message));
                            }
                        }
                        Err(_) => {
                            browser_needs_human_busy.set(false);
                            browser_needs_human_error.set(Some(fail_message));
                        }
                    }
                });
            })
        />
        <TurnMemoryOverlay
            state=TurnMemoryOverlayState {
                locale,
                proposal: turn_memory_proposal,
                editor: turn_memory_editor,
                scope: turn_memory_scope,
                replace_id: turn_memory_replace_id,
                busy: turn_memory_busy,
                error: turn_memory_error,
            }
            on_confirm=confirm_turn_memory
        />
        <ProjectLanding
            state=ProjectLandingState {
                show_projects, demo_mode, items, active_session, project_open_error,
                demos, modal_artifact, locale, running, approval_pending,
                sync_actions_available, command_palette_open, project_transfer,
                privacy_mode_active, privacy_hidden_project_ids,
                menu_new_project, menu_import_project,
            }
            open_project=switch_project
            open_project_session=palette_open_session
            open_scratch=open_scratch
            open_settings=Callback::new(move |section: Option<String>| open_settings_fn(section))
            open_library=Callback::new(move |_| show_library.set(true))
            open_project_export=open_project_export
        />
        <SessionImportModal
            locale=locale
            open=show_session_import
            on_imported=Callback::new(move |_| {
                refresh_sessions(
                    sessions,
                    pending_turns,
                    running,
                    session_history_cursor,
                    active_session,
                    exploration_frames,
                );
                refresh_folders(folders);
            })
        />
        {move || show_library.get().then(|| view! {
            <LibraryScreen
                locale=locale.read_only()
                items=library_items.read_only()
                on_close=Callback::new(move |_| show_library.set(false))
                on_open_source=palette_open_session
                on_changed=refresh_library_items
                on_insert=Callback::new(move |text: String| {
                    input.set(text);
                    show_library.set(false);
                    focus_composer();
                })
                can_insert=Signal::derive(move || !show_projects.get())
            />
        })}
        {move || show_research_graph.get().then(|| view! {
            <ResearchGraphModal
                locale=locale.read_only()
                graph=research_graph.read_only()
                on_close=Callback::new(move |_| show_research_graph.set(false))
            />
        })}
        {move || show_publication_workspace.get().then(|| view! {
            <PublicationWorkspaceModal
                locale=locale.read_only()
                binding_source=publication_binding_source
                on_close=Callback::new(move |_| {
                    publication_binding_source.set(None);
                    show_publication_workspace.set(false);
                })
            />
        })}
        <SshConnectivityOverlay
            state=SshConnectivityOverlayState {
                locale, ssh_connectivity_modal, ssh_connectivity_busy, execution_contexts,
                remote_file_cwd, remote_file_entries, remote_file_loading, remote_file_error,
                file_source,
            }
            apply_session_compute_resource=apply_session_compute_resource
            edit_ssh_host=edit_ssh_host
            open_settings=Callback::new(move |section: Option<String>| open_settings_fn(section))
        />
        <UpdateCheckOverlay state=UpdateCheckOverlayState { locale, update_check_modal, update_check_enabled, update_banner } />
        <div class="app"
            class:app-entering=move || app_shell_entering.get()
            class:scratch-mode=move || scratch_open.get()
            // Onboarding lives in this shell, so hiding it on the projects
            // landing swallowed the first-run overlay entirely.
            class:app-hidden=move || show_projects.get() && !scratch_open.get() && !show_settings.get() && !show_onboarding.get() && modal_artifact.get().is_none()
            on:contextmenu=on_context_menu>
        <Sidebar
            state=SidebarState {
                locale, show_sidebar, sidebar_w, show_proj_menu, show_projects, demo_mode, project_info, proj_list,
                sessions, folders, drag_session, drop_target, active_session, running,
                explorations,
                attention: approval_pending,
                rename_session_input, rename_session_target, collapsed_folders, folder_modal_input,
                folder_modal, demos, session_history_cursor, session_history_loading,
                update_banner,
            }
            open_update=Callback::new(move |_| {
                run_update_check();
            })
            toggle_proj_menu=Callback::new(toggle_proj_menu)
            open_proj_settings=Callback::new(open_proj_settings)
            switch_project=switch_project
            new_session=Callback::new(new_session)
            open_search=Callback::new(move |_| {
                action_palette_open.set(false);
                command_palette_open.set(true);
            })
            new_folder=Callback::new(new_folder)
            open_files=Callback::new(open_files)
            open_research_graph=Callback::new(move |_| {
                show_research_graph.set(true);
                refresh_research_graph(research_graph);
            })
            open_publication_workspace=Callback::new(move |_| {
                publication_binding_source.set(None);
                show_publication_workspace.set(true);
            })
            open_library=Callback::new(move |_| show_library.set(true))
            load_demo=Callback::new(load_demo)
            open_demo_actions=Callback::new(move |(ev, id, title): (web_sys::MouseEvent, String, String)| {
                ctx_menu.set(Some(context_menu::demo_menu(
                    ev.client_x() as f64,
                    ev.client_y() as f64,
                    &id,
                    &title,
                    locale.get(),
                )));
            })
            load_session=load_session
            open_exploration=open_exploration
            open_exploration_actions=Callback::new(move |(ev, id, status): (web_sys::MouseEvent, String, String)| {
                ctx_menu.set(Some(context_menu::exploration_menu(
                    ev.client_x() as f64,
                    ev.client_y() as f64,
                    &id,
                    &status,
                    locale.get(),
                )));
            })
            load_older_sessions=Callback::new(move |_| load_older_sessions(
                sessions,
                pending_turns,
                running,
                session_history_cursor,
                session_history_loading,
            ))
            move_sessions_to=move_sessions_to
            delete_sessions=Callback::new(move |ids: Vec<String>| {
                ui_confirm.set(Some(UiConfirm::DeleteSessions(ids)));
            })
            open_session_actions=Callback::new(move |(ev, id, title, pinned, is_branch, branch_merged, has_branch_family, has_exploration_round, stale_prompt): (web_sys::MouseEvent, String, String, bool, bool, bool, bool, bool, bool)| {
                ctx_menu.set(Some(context_menu::session_menu(
                    ev.client_x() as f64,
                    ev.client_y() as f64,
                    &id,
                    &title,
                    pinned,
                    is_branch,
                    branch_merged,
                    has_branch_family,
                    has_exploration_round,
                    stale_prompt,
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
            open_issue_report=Callback::new(start_issue_report)
            open_settings=Callback::new(open_settings)
            on_sidebar_resize_start=Callback::new(on_sidebar_resize_start)
        />

        <div class="workspace-area">
        <div class="workspace-main">
        <main class="center" class:split=move || center_split_on.get()
            style=move || center_chat_w.get()
                .map(|width| format!("--center-chat-width:{width}px"))
                .unwrap_or_default()>
            <div class="topbar">
                <div class="scratch-topbar">
                    <span class="scratch-title">{move || t(locale.get(), "scratch.title")}</span>
                    <button type="button" class="icon-btn scratch-close"
                        title=move || t(locale.get(), "scratch.close")
                        aria-label=move || t(locale.get(), "scratch.close")
                        on:click=move |_| close_scratch.call(())>
                        {compose_icon("close")}
                    </button>
                </div>
                {move || (!scratch_open.get() && !show_sidebar.get()).then(|| view! {
                    <button class="icon-btn" title=move || t(locale.get(), "sidebar.show") on:click=move |_| show_sidebar.set(true)>{compose_icon("chevron")}</button>
                })}
                <div class="center-tabs" role="tablist">
                    <button type="button" class="center-tab" class:active=move || center_file.get().is_none()
                        title=move || if demo_mode.get() {
                            t(locale.get(), "projects.example").into()
                        } else {
                            center_conversation_title.get()
                        }
                        on:click=move |_| center_file.set(None)>
                        <span class="center-tab-label">{move || if demo_mode.get() {
                            t(locale.get(), "projects.example").into()
                        } else {
                            center_conversation_title.get()
                        }}</span>
                    </button>
                    <For
                        each=move || if demo_mode.get() { Vec::new() } else { center_files.get() }
                        key=|file| file.path.clone()
                        children=move |file| {
                            let path = file.path;
                            let select_path = path.clone();
                            let close_path = path.clone();
                            let label = file.name;
                            view! {
                                <div class="center-tab-wrap">
                                    <button type="button" class="center-tab" class:active=move || center_file.get().as_ref() == Some(&path)
                                        title=label.clone() data-center-path=path.clone()
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
                {move || session_specialist.get().map(|s| view! {
                    <span class="session-specialist" title=s.name.clone()>{s.name}</span>
                })}
                {move || {
                    if needs_api_key.get() {
                        Some(view! {
                            <span class="hint hint-action">
                                {move || t(locale.get(), "err.no_api_key")}" "
                                <button type="button" class="link-inline" on:click=move |_| open_settings_fn(Some("models".into()))>
                                    {move || t(locale.get(), "status.open_settings")}
                                </button>
                            </span>
                        }.into_view())
                    } else if compaction_active.get() {
                        Some(view! {
                            <div class="context-compaction-live" role="status" data-testid="context-compaction-live">
                                <span class="context-compaction-spectrum" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></span>
                                <span class="context-compaction-live-copy">
                                    <strong>{move || t(locale.get(), "chat.compacting_title")}</strong>
                                    <span>{move || t(locale.get(), "chat.compacting_note")}</span>
                                </span>
                            </div>
                        }.into_view())
                    } else if active_session
                        .get()
                        .is_some_and(|id| reviewing.with(|ids| ids.contains(&id)))
                    {
                        Some(view! {
                            <div class="review-live" role="status" data-testid="review-live">
                                <span class="review-live-lens" aria-hidden="true">
                                    <i></i><i></i><i></i>
                                </span>
                                <span class="context-compaction-live-copy">
                                    <strong>{move || t(locale.get(), "chat.reviewing_title")}</strong>
                                    <span>{move || t(locale.get(), "chat.reviewing_note")}</span>
                                </span>
                            </div>
                        }.into_view())
                    } else {
                        let s = status.get();
                        (!s.is_empty()).then(|| {
                            view! { <span class="hint" title=s.clone()>{s}</span> }.into_view()
                        })
                    }
                }}
                <div class="spacer"></div>
                <div class="topbar-actions">
                <button type="button" class="icon-btn" data-testid="share-topbar"
                    title=move || {
                        if can_share.get() {
                            t(locale.get(), "share.topbar")
                        } else {
                            t(locale.get(), "composer.cmd_share_empty")
                        }
                    }
                    aria-label=move || t(locale.get(), "share.topbar")
                    disabled=move || demo_mode.get() || !can_share.get()
                    on:click=move |_| open_share.call(())>
                    {compose_icon("share")}
                </button>
                <button type="button" class="icon-btn" data-testid="trajectory-topbar"
                    title=move || t(locale.get(), "trajectory.topbar")
                    aria-label=move || t(locale.get(), "trajectory.topbar")
                    class:active=move || trajectory_open.get()
                    on:click=move |_| trajectory_open.set(true)>
                    {compose_icon("timeline")}
                </button>
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
                    disabled=move || scratch_open.get() || demo_mode.get()
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
                    disabled=move || scratch_open.get() || demo_mode.get()
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
                    }>{compose_icon("panel")}</button>
                </div>
            </div>

            {move || (!demo_mode.get()).then(|| center_file.get()).flatten().and_then(|path| {
                center_files.get().into_iter().find(|file| file.path == path)
            }).map(|file| {
                let path = file.path.clone();
                let display_path = project_info
                    .get()
                    .and_then(|project| workspace_relative_path(&project.root, &path))
                    .unwrap_or_else(|| path.replace('\\', "/"));
                let heading_path = path.clone();
                let heading_name = file.name.clone();
                let heading_display = display_path.clone();
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
                        data-file-path=path.clone()
                        style=move || (run_language.is_some() && center_runtime_panel.get()).then(|| format!(
                            "--runtime-right-w:{:.2}%;--runtime-bottom-h:{:.2}%",
                            center_runtime_right_w.get(),
                            center_runtime_bottom_h.get(),
                        ))>
                        <div class="center-file-head">
                            <span>{
                                let mcp_label = is_mcp_app.then_some(label);
                                move || {
                                    if let Some(label) = mcp_label.clone() {
                                        label
                                    } else {
                                        center_preview_heading(
                                            &heading_path,
                                            &heading_name,
                                            &heading_display,
                                            snapshot_workspace_path.get().as_deref(),
                                        )
                                    }
                                }
                            }</span>
                            {(artifact_version_id_path(&path).is_some()
                                || artifact_id_path(&path).is_some())
                                .then(|| view! {
                                    <span class="center-file-snapshot-badge">
                                        {move || t(locale.get(), "center.snapshot")}
                                    </span>
                                })}
                            <div class="spacer"></div>
                            {move || snapshot_workspace_path.get().map(|workspace_path| {
                                let open_path = workspace_path.clone();
                                view! {
                                    <button type="button" class="center-file-btn" data-open-editor=""
                                        title=move || t(locale.get(), "center.open_editor")
                                        aria-label=move || t(locale.get(), "center.open_editor")
                                        on:click=move |_| {
                                            let tab = CenterFileTab::from_path(open_path.clone());
                                            center_files.update(|files| {
                                                if !files.iter().any(|file| file.path == open_path) {
                                                    files.push(tab.clone());
                                                }
                                            });
                                            center_file.set(Some(open_path.clone()));
                                        }>
                                        {compose_icon("code")}
                                        <span>{move || t(locale.get(), "center.open_editor")}</span>
                                    </button>
                                }
                            })}
                            // Bind this script to a runtime. The editor can run a
                            // selection or the whole saved file; this control only
                            // chooses which process those actions talk to.
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
                        {if let Some(language) = run_language.filter(|_| !is_mcp_app) {
                            // R/Python sources are directly editable; everything
                            // else keeps the read-only preview.
                            let editor_path = path.clone();
                            let editor_options = create_memo(move |_| {
                                runtime_binding_options(&execution_contexts.get(), language)
                            });
                            let editor_bound = {
                                let path = editor_path.clone();
                                create_memo(move |_| {
                                    let stored = center_runtime_binding.get().get(&path).cloned();
                                    resolve_runtime_binding(&editor_options.get(), stored.as_deref())
                                })
                            };
                            let editor_run = Callback::new({
                                let path = editor_path.clone();
                                move |code: String| {
                                    let Some(context_id) = editor_bound.get_untracked() else {
                                        return;
                                    };
                                    run_in_runtime(
                                        path.clone(),
                                        context_id,
                                        language.to_string(),
                                        code,
                                        locale.get_untracked(),
                                        RuntimeRunCtx {
                                            consoles: center_console,
                                            plots: center_plots,
                                            busy: center_run_busy,
                                            runtimes: runtime_infos,
                                            project: project_info,
                                            object_states: runtime_object_states,
                                            inspector_open: center_runtime_panel,
                                            locale,
                                        },
                                    );
                                }
                            });
                            let editor_run_script = Callback::new({
                                let path = editor_path.clone();
                                move |_: ()| {
                                    let Some(context_id) = editor_bound.get_untracked() else {
                                        return;
                                    };
                                    run_script_in_runtime(
                                        path.clone(),
                                        context_id,
                                        language.to_string(),
                                        locale.get_untracked(),
                                        RuntimeRunCtx {
                                            consoles: center_console,
                                            plots: center_plots,
                                            busy: center_run_busy,
                                            runtimes: runtime_infos,
                                            project: project_info,
                                            object_states: runtime_object_states,
                                            inspector_open: center_runtime_panel,
                                            locale,
                                        },
                                    );
                                }
                            });
                            view! {
                                <RpCodeEditor
                                    dom_id=dom_id.clone()
                                    path=path.clone()
                                    lang=language.to_string()
                                    drafts=center_editor_drafts
                                    busy=center_run_busy
                                    on_run=editor_run
                                    on_run_script=editor_run_script
                                />
                            }.into_view()
                        } else if is_mcp_app {
                            mcp_apps.get().get(&path).cloned().map(|payload_json| view! {
                                <McpAppPreview
                                    instance_id=path.clone()
                                    payload_json=payload_json
                                    on_selection=Callback::new(move |selection: MotifSelection| {
                                        let block = selection.composer_text();
                                        input.update(|draft| {
                                            if !draft.trim().is_empty() {
                                                draft.push_str("\n\n");
                                            }
                                            draft.push_str(&block);
                                        });
                                        motif_selection.set(Some(selection));
                                        focus_composer();
                                    })
                                />
                            }).into_view()
                        } else {
                            view! {
                                <WorkspaceFilePreview
                                    dom_id=dom_id.clone()
                                    path=path.clone()
                                    kind=kind.clone()
                                    filename=file.name.clone()
                                />
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
                        {run_language.map(|language| {
                            let console_file = console_file.clone();
                            let console_options = create_memo(move |_| {
                                runtime_binding_options(&execution_contexts.get(), language)
                            });
                            let console_bound = {
                                let path = console_file.clone();
                                create_memo(move |_| {
                                    let stored = center_runtime_binding.get().get(&path).cloned();
                                    resolve_runtime_binding(&console_options.get(), stored.as_deref())
                                })
                            };
                            // Typing in the console prompt runs against the same
                            // bound runtime as a selection run would.
                            let on_run = Callback::new({
                                let path = console_file.clone();
                                move |code: String| {
                                    let Some(context_id) = console_bound.get_untracked() else {
                                        return;
                                    };
                                    run_in_runtime(
                                        path.clone(),
                                        context_id,
                                        language.to_string(),
                                        code,
                                        locale.get_untracked(),
                                        RuntimeRunCtx {
                                            consoles: center_console,
                                            plots: center_plots,
                                            busy: center_run_busy,
                                            runtimes: runtime_infos,
                                            project: project_info,
                                            object_states: runtime_object_states,
                                            inspector_open: center_runtime_panel,
                                            locale,
                                        },
                                    );
                                }
                            });
                            let plots_file = console_file.clone();
                            move || center_runtime_panel.get().then(|| view! {
                                <CenterRuntimeConsole path=console_file.clone() consoles=center_console
                                    language_label=language_display(language).to_string()
                                    busy=center_run_busy on_run=on_run />
                                <CenterRuntimePlots path=plots_file.clone() plots=center_plots />
                                // Pane dividers: drag to resize the quadrants.
                                <div class="center-runtime-col-resizer" role="separator"
                                    aria-orientation="vertical"
                                    on:mousedown=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        center_runtime_col_dragging.set(true);
                                    }></div>
                                <div class="center-runtime-row-resizer" role="separator"
                                    aria-orientation="horizontal"
                                    on:mousedown=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        center_runtime_row_dragging.set(true);
                                    }></div>
                            })
                        })}
                    </div>
                }
            })}
            {move || selection_popup.get().filter(|_| !demo_mode.get()).map(|(text, source, x, y)| {
                // Run the selection in the file's bound runtime — the RStudio
                // reflex. Only for R/Python sources, where a runtime exists.
                let runtime_source = is_runtime_code_selection(source.as_deref());
                let x = if runtime_source {
                    selection_popup_x_with_max_width(x, 320)
                } else {
                    selection_popup_x(x)
                };
                let y = if runtime_source {
                    selection_popup_y_with_clearance(y, 200)
                } else {
                    selection_popup_y(y)
                };
                let quote = text.clone();
                let quote_source = source.clone();
                let quote_source_for_click = quote_source.clone();
                let side_quote = text.clone();
                let side_quote_source = source.clone();
                let explain = text.clone();
                let annotate_text = text.clone();
                let annotate_source = source.clone();
                let action_selection = text.clone();
                let action_source = source.clone();
                // Only chat-transcript selections (no source path) can be saved
                // as a highlight; file-preview selections have their own actions.
                let star_text = source.is_none().then(|| text.clone());
                let run_selection = source.as_deref()
                    .and_then(runtime_language)
                    .map(|language| (source.clone().unwrap_or_default(), language, text));
                let popup_class = if runtime_source {
                    "selection-popup selection-popup-code"
                } else {
                    "selection-popup"
                };
                view! {
                    <div class=popup_class style=format!("left:{x}px;top:{y}px")>
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
                                plots: center_plots,
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
                        {runtime_source.then(|| view! {
                            <span class="selection-popup-sep" aria-hidden="true"></span>
                        })}
                        {(!runtime_source).then(|| quick_actions.get().into_iter()
                            .filter(|action| action.enabled && action.context == "selection")
                            .map(|action| {
                            let action_id = action.id.clone();
                            let selection = action_selection.clone();
                            let source = action_source.clone();
                            let label = quick_action_label(locale.get(), &action);
                            view! {
                                <button type="button" class="selection-popup-btn"
                                    data-quick-action=action.id
                                    title=action.description
                                    on:click=move |_| {
                                        run_quick_action.call((
                                            action_id.clone(),
                                            selection.clone(),
                                            source.clone(),
                                        ));
                                    }>
                                    {compose_icon(&action.icon)}
                                    <span>{label}</span>
                                </button>
                            }
                            }).collect_view())}
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
                                if selection_targets_center_file(
                                    quote_source_for_click.as_deref(),
                                    center_file.get_untracked().as_deref(),
                                ) {
                                    center_split.set(true);
                                    show_right.set(false);
                                }
                                focus_composer();
                            }>
                            {compose_icon("plus")}
                            <span>{move || if selection_targets_center_file(
                                quote_source.as_deref(),
                                center_file.get().as_deref(),
                            ) {
                                t(locale.get(), "selection.ask_ai")
                            } else {
                                t(locale.get(), "selection.add_to_chat")
                            }}</span>
                        </button>
                        <button type="button" class="selection-popup-btn"
                            on:click=move |_| {
                                side_chat_quotes.update(|items| items.push(
                                    ComposerQuote::from_selection(
                                        side_quote.clone(),
                                        side_quote_source.clone(),
                                    )
                                ));
                                selection_popup.set(None);
                                clear_selection();
                                ensure_right_tab(
                                    RightTab::SideChat,
                                    show_right,
                                    open_right_tabs,
                                    right_tab,
                                );
                                focus_element_soon(SIDE_CHAT_INPUT_ID);
                            }>
                            {compose_icon("chat")}
                            <span>{t(locale.get(), "selection.quote_side_chat")}</span>
                        </button>
                        <button type="button" class="selection-popup-btn"
                            on:click=move |_| {
                                selection_popup.set(None);
                                clear_selection();
                                send_side_chat((
                                    t(locale.get(), "selection.explain_prompt").into(),
                                    vec![ComposerQuote::plain(explain.clone())],
                                    false,
                                ));
                            }>
                            {compose_icon("sparkles")}
                            <span>{t(locale.get(), "selection.explain")}</span>
                        </button>
                        // Annotate → append the passage to reviews/<file>.md, which the
                        // agent reads back with its ordinary tools. Offered on papers
                        // and other previews, not on R/Python source (run/ask/quote).
                        {annotate_source.filter(|_| !runtime_source).map(|src| {
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
            <div class="center-split-resizer" role="separator" aria-orientation="vertical"
                aria-label=move || t(locale.get(), "center.resize_split")
                on:mousedown=on_center_split_resize_start></div>
            <div class="chat-stage" class:center-hidden=move || center_file_open.get() && !center_split.get()>
            <div class="chat" id=CHAT_SCROLLER_ID
                on:mouseup=move |ev| {
                    // Primary button only: a right-click mouseup would re-raise
                    // the popup on top of the context menu. Also honors the
                    // "selection quick actions" setting.
                    if ev.button() != 0 {
                        return;
                    }
                    // Clicking a path/file link opens the preview on `click`.
                    // mouseup runs first and must not treat the link text as a
                    // quote selection, or both overlays appear together.
                    if context_menu::selection_popup_blocked(&ev) {
                        selection_popup.set(None);
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
                    // Follow-scroll (follow-ups, the runtime strip remounting)
                    // used to dismiss the quote popup while the selection was
                    // still live. Only clear it when the selection is gone.
                    if selection_popup.get_untracked().is_some()
                        && context_menu::selection_text().is_none()
                    {
                        selection_popup.set(None);
                    }
                }>
                <div class="thread" id=CHAT_THREAD_ID>
                    {move || active_session.get().and_then(|frame_id| {
                        let rows = explorations.get();
                        if let Some(summary) = rows.iter().find(|row| {
                            row.exploration.frame_id == frame_id
                                && matches!(
                                    row.exploration.status.as_str(),
                                    "creating" | "active" | "promoting"
                                )
                        }).cloned() {
                            let isolation_key = if summary.isolation_is_full() {
                                "exploration.isolation_full"
                            } else {
                                "exploration.isolation_partial"
                            };
                            let exploration = summary.exploration;
                            let id_for_diff = exploration.id.clone();
                            let id_for_promote = exploration.id.clone();
                            let id_for_discard = exploration.id.clone();
                            let status_key = "exploration.status_active";
                            Some(view! {
                                <section class="exploration-banner branch" data-testid="exploration-banner">
                                    <div class="exploration-banner-copy">
                                        <span class="exploration-banner-eyebrow">{t(locale.get(), "exploration.banner_label")}</span>
                                        <strong>{exploration.name}</strong>
                                        <span>{format!("{} · {} · {}", t(locale.get(), status_key), t(locale.get(), isolation_key), t(locale.get(), "exploration.external_warning_short"))}</span>
                                    </div>
                                    <div class="exploration-banner-actions">
                                        <button type="button" on:click=move |_| open_exploration_preview.call(id_for_diff.clone())>{t(locale.get(), "exploration.view_diff")}</button>
                                        <button type="button" class="primary" disabled=exploration.status != "active"
                                            on:click=move |_| open_exploration_preview.call(id_for_promote.clone())>{t(locale.get(), "exploration.promote")}</button>
                                        <button type="button" class="danger-text"
                                            on:click=move |_| open_exploration_preview.call(id_for_discard.clone())>{t(locale.get(), "exploration.discard")}</button>
                                    </div>
                                </section>
                            }.into_view())
                        } else {
                            let active_count = rows
                                .iter()
                                .filter(|row| {
                                    row.source_frame_id == frame_id
                                        && matches!(
                                            row.exploration.status.as_str(),
                                            "creating" | "active" | "promoting"
                                        )
                                })
                                .count();
                            let latest_turn_index = items.with(|rows| {
                                rows.iter()
                                    .filter(|item| matches!(item, ChatItem::User(_)))
                                    .count()
                                    .saturating_sub(1)
                            }) + transcript_pages
                                .with(|pages| pages.get(&frame_id).copied())
                                .map_or(0, |page| page.user_offset);
                            (active_count > 0).then(|| view! {
                                <section class="exploration-banner mainline" data-testid="mainline-exploration-banner">
                                    <div class="exploration-banner-copy">
                                        <span class="exploration-banner-eyebrow">{t(locale.get(), "exploration.mainline_label")}</span>
                                        <strong>{tf(locale.get(), "exploration.mainline_count", &[("n", &active_count.to_string())])}</strong>
                                        <span>{t(locale.get(), "exploration.mainline_warning")}</span>
                                    </div>
                                    <button type="button" on:click=move |_| start_exploration_from_head.call(latest_turn_index)>{t(locale.get(), "exploration.start_another")}</button>
                                </section>
                            }.into_view())
                        }
                    })}
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
                            let busy_now = busy.get();
                            // `load_session` deliberately swaps the visible rows before
                            // publishing their session id. Carry the id in every keyed row
                            // so that second update rebuilds callbacks which must target the
                            // newly active session (notably background approval cards).
                            let thread_session_id = active_session.get().unwrap_or_default();
                            let branch_projection_revision = conversation_branches.with(|all| {
                                all.get(&thread_session_id)
                                    .into_iter()
                                    .flatten()
                                    .filter(|branch| branch.merged)
                                    .count() as u64
                            });
                            // Inline exploration cards are projected into otherwise
                            // immutable assistant rows. Include their identity and
                            // status in the keyed-row fingerprint so hard deletion,
                            // creation, or status changes remount the affected row.
                            let exploration_projection_revision = explorations.with(|rows| {
                                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                                rows.iter()
                                    .filter(|row| row.source_frame_id == thread_session_id)
                                    .for_each(|row| {
                                        row.checkpoint_user_index.hash(&mut hasher);
                                        row.exploration.id.hash(&mut hasher);
                                        row.exploration.status.hash(&mut hasher);
                                    });
                                hasher.finish()
                            });
                            let user_offset = transcript_pages
                                .with(|pages| pages.get(&thread_session_id).copied())
                                .map_or(0, |page| page.user_offset);
                            let requested_start = if busy_now {
                                usize::MAX
                            } else {
                                transcript_pages.with(|pages| {
                                    pages
                                        .get(&thread_session_id)
                                        .map(|page| page.window_user_start)
                                        .unwrap_or(usize::MAX)
                                })
                            };
                            // Rows carry message indices, never cloned messages;
                            // `children` clones lazily, so a flush only pays for
                            // rows whose fingerprint key actually changed.
                            conversation_outline.with(|outline| items.with(|list| {
                            // Queued user turns live after the active turn and
                            // must not make its process group look historical.
                            let queue_start = trailing_queue_start(list);
                            let last = queue_start.saturating_sub(1);
                            let live_assistant_index = busy_now.then(|| {
                                list[..queue_start]
                                    .iter()
                                    .rposition(|item| matches!(item, ChatItem::Assistant { .. }))
                            }).flatten();
                            let live_reasoning_index = busy_now.then(|| {
                                let turn_start = list[..queue_start]
                                    .iter()
                                    .rposition(|item| matches!(item, ChatItem::User(_)))?;
                                list[turn_start + 1..queue_start]
                                    .iter()
                                    .rposition(|item| matches!(item, ChatItem::Reasoning(_)))
                                    .map(|offset| turn_start + 1 + offset)
                            }).flatten();
                            // Keep process layers separate while the turn runs;
                            // once complete, fold commentary + reasoning + tools
                            // into one activity summary before the final answer.
                            let mut rows: Vec<(String, usize, bool, u64, ThreadRow)> = Vec::new();
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
                                    let mut indices: Vec<usize> = Vec::new();
                                    for j in i..end {
                                        if is_turn_activity_at(list, j) {
                                            indices.push(j);
                                        }
                                    }
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    for idx in &indices { (idx, list[*idx].fingerprint()).hash(&mut h); }
                                    true.hash(&mut h);
                                    let ui_indices = indices
                                        .iter()
                                        .map(|index| index.to_string())
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    let user_index = list[..start]
                                        .iter()
                                        .filter(|item| matches!(item, ChatItem::User(_)))
                                        .count()
                                        .checked_sub(1)
                                        .map(|index| index + user_offset);
                                    let duration_ms = user_index
                                        .and_then(|index| outline.iter().find(|entry| entry.user_index == index))
                                        .and_then(|entry| turn_duration_ms(entry.sent_at, entry.response_at));
                                    duration_ms.hash(&mut h);
                                    rows.push((thread_session_id.clone(), start, false, h.finish(), ThreadRow::Activity {
                                        indices,
                                        ui_indices,
                                        duration_ms,
                                    }));
                                    i = end;
                                } else if is_tool_activity(&list[i]) {
                                    let start = i;
                                    let mut indices: Vec<usize> = Vec::new();
                                    let mut j = i;
                                    while j < window.end {
                                        if renders_nothing(&list[j]) { j += 1; continue; }
                                        if is_tool_activity(&list[j]) { indices.push(j); j += 1; }
                                        else { break; }
                                    }
                                    // Usage is metadata for the whole reply, not
                                    // a boundary that closes the live step run.
                                    let live = busy_now && (j > last || list[j..=last].iter().all(|item| {
                                        renders_nothing(item) || matches!(item, ChatItem::Usage { .. })
                                    }));
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    for idx in &indices { (idx, list[*idx].fingerprint()).hash(&mut h); }
                                    live.hash(&mut h);
                                    let ui_indices = indices
                                        .iter()
                                        .map(|index| index.to_string())
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    rows.push((thread_session_id.clone(), start, false, h.finish(), ThreadRow::Steps {
                                        indices,
                                        live,
                                        ui_indices,
                                    }));
                                    i = j;
                                } else {
                                    let commentary = is_commentary_at(list, i);
                                    // A live assistant row keeps one stable owner while
                                    // its Markdown prefix advances on a separate budget.
                                    // A following tool turns it into settled commentary
                                    // and deliberately remounts the compact row.
                                    let streaming_assistant = live_assistant_index == Some(i)
                                        && !commentary
                                        && matches!(
                                            &list[i],
                                            ChatItem::Assistant { text, .. }
                                                if !text.starts_with("Error: ")
                                        );
                                    let streaming_reasoning = live_reasoning_index == Some(i);
                                    let compact_assistant = commentary
                                        || live_assistant_index == Some(i);
                                    let timestamp = transcript_item_timestamp(
                                        list,
                                        i,
                                        user_offset,
                                        &outline,
                                    );
                                    let mut fp = if streaming_assistant || streaming_reasoning {
                                        0
                                    } else {
                                        list[i].fingerprint()
                                    };
                                    fp ^= (commentary as u64) << 63;
                                    fp ^= (compact_assistant as u64) << 62;
                                    fp ^= timestamp.unwrap_or_default() as u64;
                                    fp ^= branch_projection_revision.rotate_left(17);
                                    fp ^= exploration_projection_revision.rotate_left(29);
                                    rows.push((thread_session_id.clone(), i, streaming_assistant || streaming_reasoning, fp, ThreadRow::Item {
                                        i,
                                        timestamp,
                                        commentary,
                                        compact_assistant,
                                        streaming_assistant,
                                        streaming_reasoning,
                                    }));
                                    i += 1;
                                }
                            }
                            // Runs without an inline `monitor_run` tool row only carry a
                            // session id. Use their persisted creation time to put the
                            // fallback card before the next user turn instead of always
                            // appending it to the live end of the conversation.
                            let automatic_runs = automatic_session_runs.get();
                            let mut automatic_runs = automatic_runs.into_iter().peekable();
                            let mut anchored = Vec::with_capacity(rows.len() + automatic_runs.len());
                            let mut synthetic_start = list.len();
                            for row in rows {
                                let next_user_at = match &row.4 {
                                    ThreadRow::Item { i, timestamp, .. }
                                        if matches!(list[*i], ChatItem::User(_)) => *timestamp,
                                    _ => None,
                                };
                                if let Some(next_user_at) = next_user_at {
                                    while automatic_runs
                                        .peek()
                                        .is_some_and(|(_, created_at)| *created_at < next_user_at)
                                    {
                                        let (run_id, _) = automatic_runs.next().unwrap();
                                        let mut h = std::collections::hash_map::DefaultHasher::new();
                                        run_id.hash(&mut h);
                                        anchored.push((
                                            thread_session_id.clone(), synthetic_start, false, h.finish(),
                                            ThreadRow::AutoRun { run_id },
                                        ));
                                        synthetic_start += 1;
                                    }
                                }
                                anchored.push(row);
                            }
                            for (run_id, _) in automatic_runs {
                                let mut h = std::collections::hash_map::DefaultHasher::new();
                                run_id.hash(&mut h);
                                anchored.push((
                                    thread_session_id.clone(), synthetic_start, false, h.finish(),
                                    ThreadRow::AutoRun { run_id },
                                ));
                                synthetic_start += 1;
                            }
                            anchored
                            }))
                        }
                        key=|(session_id, start, streaming, fp, _)| {
                            (session_id.clone(), *start, *streaming, *fp)
                        }
                        children=move |(session_id, start, _, _, row)| {
                            match row {
                                ThreadRow::AutoRun { run_id } => view! {
                                    <div class="tool-wrap run-monitor-wrap auto-run-monitor"
                                        data-testid="auto-run-monitor">
                                        <RunMonitorCard
                                            run_id=run_id
                                            runs=run_records
                                            clock=run_clock.read_only()
                                            tool_ok=None
                                            tool_output=String::new()
                                            dismissed_runs=dismissed_run_cards
                                        />
                                    </div>
                                }.into_view(),
                                ThreadRow::Item {
                                    i,
                                    timestamp,
                                    commentary,
                                    compact_assistant,
                                    streaming_assistant,
                                    streaming_reasoning,
                                } => {
                                    // Rebuilt only when the fingerprint key changed,
                                    // so this is the one clone that actually pays off.
                                    let item = if streaming_reasoning {
                                        ChatItem::Reasoning(String::new())
                                    } else {
                                        items.with_untracked(|list| list[i].clone())
                                    };
                                    let on_resume = Callback::new(resume_turn);
                                    let class = if commentary {
                                        "msg assistant commentary"
                                    } else {
                                        class_for(&item)
                                    };
                                    let user_index = items
                                        .with_untracked(|rows| user_turn_index(rows, i))
                                        .map(|index| {
                                            index
                                                + transcript_pages
                                                    .with_untracked(|pages| {
                                                        pages.get(&session_id).copied()
                                                    })
                                                    .map_or(0, |page| page.user_offset)
                                        });
                                    let explore_turn_index = items
                                        .with_untracked(|rows| owning_user_turn_index(rows, i))
                                        .map(|index| {
                                            index
                                                + transcript_pages
                                                    .with_untracked(|pages| {
                                                        pages.get(&session_id).copied()
                                                    })
                                                    .map_or(0, |page| page.user_offset)
                                        });
                                    let data_user_index =
                                        user_index.map(|index| index.to_string());
                                    let branch_anchor = if matches!(&item, ChatItem::User(_)) {
                                        user_index.map(|index| (index, "before_user"))
                                    } else if matches!(&item, ChatItem::Assistant { .. }) {
                                        explore_turn_index.map(|index| (index, "after_response"))
                                    } else {
                                        None
                                    };
                                    let message_branches = branch_anchor.map_or_else(Vec::new, |(index, kind)| {
                                        conversation_branches.with(|all| {
                                            all.get(&session_id)
                                                .into_iter()
                                                .flatten()
                                                .filter(|branch| {
                                                    branch.checkpoint_user_index == index
                                                        && branch.checkpoint_kind == kind
                                                })
                                                .cloned()
                                                .collect::<Vec<_>>()
                                        })
                                    });
                                    let message_explorations = if matches!(&item, ChatItem::Assistant { .. }) {
                                        explore_turn_index.map_or_else(Vec::new, |index| {
                                            explorations.with(|rows| {
                                                rows.iter()
                                                    .filter(|row| {
                                                        row.source_frame_id == session_id
                                                            && row.checkpoint_user_index == index
                                                    })
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                            })
                                        })
                                    } else {
                                        Vec::new()
                                    };
                                    let can_undo = Signal::derive(move || {
                                        !compact_assistant
                                            && !matches!(active_branch_state.get().as_deref(), Some("merged" | "orphaned"))
                                            && undo_assistant_index.get() == Some(i)
                                    });
                                    let show_actions = Signal::derive(move || !busy.get());
                                    let can_branch = Signal::derive(move || {
                                        active_branch_state.get().is_none()
                                            && active_acp_agent_id.get().is_none()
                                            && !active_is_exploration.get()
                                            && !busy.get()
                                    });
                                    let show_explore = Signal::derive(move || {
                                        if compact_assistant
                                            || active_acp_agent_id.get().is_some()
                                            || active_branch_state.get().is_some()
                                            || explore_turn_index.is_none()
                                        {
                                            return false;
                                        }
                                        let Some(frame_id) = active_session.get() else {
                                            return false;
                                        };
                                        let is_latest_completed = items.with(|rows| {
                                            rows.iter().rposition(|item| {
                                                matches!(item, ChatItem::Assistant { text, .. } if !text.trim().is_empty())
                                            }) == Some(i)
                                        });
                                        is_latest_completed && !explorations.with(|rows| {
                                            rows.iter().any(|row| {
                                                row.exploration.frame_id == frame_id
                                            })
                                        })
                                    });
                                    let can_explore = Signal::derive(move || {
                                        if !show_explore.get() || busy.get() {
                                            return false;
                                        }
                                        let Some(frame_id) = active_session.get() else {
                                            return false;
                                        };
                                        let joins_current_round = explorations.with(|rows| {
                                            rows.iter()
                                                .find(|row| {
                                                    matches!(
                                                        row.exploration.status.as_str(),
                                                        "creating" | "active" | "promoting"
                                                    )
                                                })
                                                .is_none_or(|row| row.source_frame_id == frame_id)
                                        });
                                        if !joins_current_round {
                                            return false;
                                        }
                                        true
                                    });
                                    view! {
                                        <div class=class
                                            class:outline-target=move || user_index.is_some_and(|index| {
                                                conversation_outline_selected.get() == Some(index)
                                            })
                                            data-ui-index=i.to_string()
                                            data-user-index=data_user_index>
                                            {if streaming_assistant {
                                                view! {
                                                    <StreamingAssistantMessage
                                                        items=items
                                                        source_item=i
                                                        on_artifact=on_artifact_select
                                                        on_file=on_file_link
                                                    />
                                                }.into_view()
                                            } else if streaming_reasoning {
                                                view! {
                                                    <StreamingReasoningMessage
                                                        items=items
                                                        source_item=i
                                                        session_id=session_id
                                                        disclosure_state=step_disclosure_state
                                                    />
                                                }.into_view()
                                            } else {
                                                render_item(
                                                    i, &item, timestamp, artifacts, on_artifact_select, on_file_link,
                                                    run_records, run_clock.read_only(), busy.read_only(), compact_assistant,
                                                    active_acp_agent_id.get().is_none()
                                                        && !matches!(active_branch_state.get_untracked().as_deref(), Some("merged" | "orphaned")),
                                                    can_branch, show_actions, can_undo, show_explore, can_explore, edit_message, branch_message, undo_message, explore_turn_index.unwrap_or_default(), start_exploration_from_head, session_id,
                                                    request_turn_memory, request_session_review, respond_confirm, on_resume,
                                                    step_disclosure_state,
                                                    plan_mode_active, plan_compat, on_plan_decision,
                                                    on_question_answer, jump_to_review_message,
                                                    dismissed_run_cards,
                                                    Callback::new(move |detail| branch_merge_detail.set(Some(detail))),
                                                ).into_view()
                                            }}
                                            {(!message_branches.is_empty() || !message_explorations.is_empty()).then(|| view! {
                                                <div class="message-branch-links">
                                                    {message_branches.into_iter().map(|branch| {
                                                        let open = load_session.clone();
                                                        let open_id = branch.id.clone();
                                                        let merged = branch.merged;
                                                        let merge_summary = branch.merge_summary.clone();
                                                        let title = if branch.title.trim().is_empty() {
                                                            t(locale.get(), "sidebar.untitled").to_string()
                                                        } else {
                                                            branch.title
                                                        };
                                                        let detail_title = title.clone();
                                                        view! {
                                                            <div class="message-branch-entry">
                                                                <button type="button" class="message-branch-link"
                                                                    data-testid="message-branch-link"
                                                                    data-session-id=branch.id
                                                                    data-session-title=title.clone()
                                                                    data-session-branch="true"
                                                                    data-session-family="true"
                                                                    data-branch-merged=if merged { "true" } else { "false" }
                                                                    on:click=move |_| open.call(open_id.clone())>
                                                                    <span aria-hidden="true">{compose_icon("branch")}</span>
                                                                    <span>{title}</span>
                                                                </button>
                                                                {merge_summary.map(|summary| {
                                                                    let detail_summary = summary.clone();
                                                                    view! {
                                                                        <button type="button" class="branch-merge-card" data-testid="branch-merge-card"
                                                                            on:click=move |_| branch_merge_detail.set(Some((detail_title.clone(), detail_summary.clone())))>
                                                                            <span class="branch-merge-card-icon" aria-hidden="true">{compose_icon("check")}</span>
                                                                            <span class="branch-merge-card-copy">
                                                                                <strong>{t(locale.get(), "branch.merged_result")}</strong>
                                                                            </span>
                                                                            <span class="branch-merge-card-open">{compose_icon("chevron-right")}</span>
                                                                        </button>
                                                                    }
                                                                })}
                                                            </div>
                                                        }
                                                    }).collect_view()}
                                                    {message_explorations.into_iter().map(|summary| {
                                                        let isolation_is_full = summary.isolation_is_full();
                                                        let exploration = summary.exploration;
                                                        let exploration_for_open = exploration.clone();
                                                        let open = open_exploration.clone();
                                                        let status_key = match exploration.status.as_str() {
                                                            "active" => "exploration.status_active",
                                                            "promoting" => "exploration.status_promoting",
                                                            "creating" => "exploration.status_creating",
                                                            _ => "exploration.status_failed",
                                                        };
                                                        let isolation_key = if isolation_is_full {
                                                            "exploration.isolation_full"
                                                        } else {
                                                            "exploration.isolation_partial"
                                                        };
                                                        view! {
                                                            <div class="message-branch-entry message-exploration-entry">
                                                                <button type="button" class="message-branch-link exploration-message-card"
                                                                    data-testid="exploration-message-card"
                                                                    data-exploration-id=exploration.id.clone()
                                                                    data-exploration-status=exploration.status.clone()
                                                                    title=exploration.name.clone()
                                                                    on:click=move |_| open.call(exploration_for_open.clone())>
                                                                    <span aria-hidden="true">{compose_icon("flask")}</span>
                                                                    <span class="message-exploration-copy">
                                                                        <strong>{exploration.name}</strong>
                                                                        <span>{format!("{} · {}", t(locale.get(), status_key), t(locale.get(), isolation_key))}</span>
                                                                    </span>
                                                                </button>
                                                            </div>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            })}
                                        </div>
                                    }.into_view()
                                }
                                ThreadRow::Steps { indices, live, ui_indices } => {
                                    // ponytail: position-keyed; move to stable
                                    // row ids if mid-list edits ever shift groups.
                                    let group_id = format!("{session_id}:steps:{start}");
                                    view! {
                                        <div class="steps-wrap" data-ui-indices=ui_indices>{
                                            render_steps_group(
                                                indices,
                                                items,
                                                live,
                                                false,
                                                None,
                                                group_id,
                                                step_disclosure_state,
                                            )
                                        }</div>
                                    }.into_view()
                                },
                                ThreadRow::Activity { indices, ui_indices, duration_ms } => {
                                    let group_id = format!("{session_id}:activity:{start}");
                                    view! {
                                        <div class="steps-wrap" data-ui-indices=ui_indices>{
                                            render_steps_group(
                                                indices,
                                                items,
                                                false,
                                                true,
                                                duration_ms,
                                                group_id,
                                                step_disclosure_state,
                                            )
                                        }</div>
                                    }.into_view()
                                },
                            }
                        }
                    />
                    {move || (!busy.get()).then(|| active_session.get()).flatten().and_then(|frame_id| {
                        follow_up_questions.with(|all| all.get(&frame_id).cloned()).map(|questions| {
                            let close_frame_id = frame_id.clone();
                            view! {
                                <section class="follow-up-questions" data-testid="follow-up-questions">
                                    <div class="follow-up-questions-head">
                                        <span>{move || compose_icon("review")}</span>
                                        <strong>{move || t(locale.get(), "follow_up.title")}</strong>
                                        <button type="button" class="follow-up-close"
                                            title=move || t(locale.get(), "follow_up.close")
                                            aria-label=move || t(locale.get(), "follow_up.close")
                                            on:click=move |_| follow_up_questions.update(|all| {
                                                all.remove(&close_frame_id);
                                            })>
                                            {compose_icon("chevron-down")}
                                        </button>
                                    </div>
                                    <div class="follow-up-options">
                                        {questions.into_iter().map(|question| {
                                            let selected = question.clone();
                                            view! {
                                                <button type="button" on:click=move |_| {
                                                    input.set(selected.clone());
                                                    focus_composer();
                                                }>
                                                    <span aria-hidden="true">"↳"</span>
                                                    <span>{question}</span>
                                                </button>
                                            }
                                        }).collect_view()}
                                    </div>
                                </section>
                            }
                        })
                    })}
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
            // Static element; scroll.js toggles `.visible` — no reactive rebuild.
            <button type="button" id="chat-jump-pill" class="chat-jump-pill"
                aria-label=move || t(locale.get(), "chat.jump_bottom")
                on:click=move |_| force_chat_bottom()>
                {compose_icon("chevron-down")}
                {move || t(locale.get(), "chat.jump_bottom")}
            </button>
            {move || {
                let rows = conversation_outline.get();
                (!rows.is_empty()).then(|| {
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
                            let sent_at = entry.sent_at.filter(|timestamp| *timestamp > 0);
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
                                        !loaded_conversation_user_range
                                            .get()
                                            .contains(&target)
                                    }
                                    on:click=move |_| {
                                        jump_to_conversation_outline.call((target, before_seq));
                                    }
                                >
                                    <span class="conversation-outline-number" aria-hidden="true">
                                        {target + 1}
                                    </span>
                                    <span class="conversation-outline-copy">
                                        <span class="conversation-outline-text">{label}</span>
                                        {sent_at.map(|timestamp| {
                                            let compact = format_message_time(timestamp);
                                            view! {
                                                <time
                                                    class="conversation-outline-time"
                                                    data-timestamp=timestamp.to_string()
                                                    title=move || tf(
                                                        locale.get(),
                                                        "msg.sent_at",
                                                        &[("time", &format_message_datetime(timestamp, locale.get()))],
                                                    )
                                                >
                                                    {compact}
                                                </time>
                                            }
                                        })}
                                    </span>
                                </button>
                            }
                        })
                        .collect_view();
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
                            class:is-hidden=move || conversation_outline_mounted.get()
                            data-testid="conversation-outline-toggle"
                            title=move || t(locale.get(), "outline.show")
                            aria-label=move || t(locale.get(), "outline.show")
                            aria-expanded=move || conversation_outline_open.get().to_string()
                            aria-hidden=move || conversation_outline_mounted.get().to_string()
                            on:click=move |_| conversation_outline_open.set(true)
                        >
                            <span class="conversation-outline-marks" aria-hidden="true">{marks}</span>
                        </button>
                        {conversation_outline_mounted.get().then(|| view! {
                            <nav
                                class="conversation-outline-panel"
                                class:is-open=move || conversation_outline_open.get()
                                data-testid="conversation-outline"
                                aria-label=move || t(locale.get(), "outline.title")
                                aria-hidden=move || (!conversation_outline_open.get()).to_string()
                                prop:inert=move || !conversation_outline_open.get()
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
                        })}
                    }
                })
            }}
            </div>

            {move || active_session.get().and_then(|session_id| {
                // Finished transfers linger briefly for confirmation. The
                // clock-driving effect above stops updating this signal after
                // the final card expires.
                let now = transfer_tray_now.get();
                let transfers = run_records.with(|records| {
                    records
                        .iter()
                    .filter(|run| run.frame_id.as_deref() == Some(session_id.as_str()))
                    .filter_map(|run| {
                            let progress = run_progress(run)?;
                        transfer_progress_visible(&progress, &run.status, now)
                                .then_some((run.clone(), progress))
                    })
                        .collect::<Vec<_>>()
                });
                (!transfers.is_empty()).then(|| view! {
                    <div class="transfer-tray" aria-live="polite">
                        {transfers.into_iter().map(|(run, progress)| {
                            let run_id = run.id.clone();
                            let cancellable = matches!(
                                run.status.as_str(),
                                "submitted" | "running" | "cancelling"
                            );
                            let cancel_label = if run.status == "cancelling" {
                                t(locale.get(), "runs.force_cancel")
                            } else {
                                t(locale.get(), "runs.cancel")
                            };
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
                                        {cancellable.then(|| {
                                            let tip = cancel_label.clone();
                                            view! {
                                            <button type="button" class="icon-btn transfer-cancel"
                                                title=tip.clone()
                                                aria-label=tip
                                                on:click=move |_| {
                                                    let run_id = run_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                        let _ = invoke("cancel_run", arg).await;
                                                        refresh_runs(run_records, locale);
                                                    });
                                                }>{compose_icon("close")}</button>
                                            }
                                        })}
                                    </div>
                                    {run_progress_meter(progress, locale.get())}
                                </section>
                            }
                        }).collect_view()}
                    </div>
                })
            })}

            <div class="composer"
                class:center-hidden=move || center_file_open.get() && !center_split.get()
                class:demo-read-only=move || demo_mode.get()>
                {move || browser_extension_status.get().map(|status| {
                    let loc = locale.get();
                    let current = status
                        .current_version
                        .clone()
                        .unwrap_or_else(|| t(loc, "browser.extension.unknown"));
                    let bundled = status
                        .bundled_version
                        .clone()
                        .unwrap_or_else(|| t(loc, "browser.extension.unknown"));
                    let path = status.extension_path.clone().unwrap_or_default();
                    let manual = browser_extension_update_outcome
                        .get()
                        .as_deref()
                        == Some("manual_reload_required");
                    let title_key = if manual {
                        "browser.extension.manual_title"
                    } else if !status.connected {
                        "browser.extension.waiting_title"
                    } else {
                        "browser.extension.title"
                    };
                    let body_key = if manual {
                        "browser.extension.manual_body"
                    } else if !status.connected {
                        "browser.extension.waiting_body"
                    } else {
                        "browser.extension.body"
                    };
                    let body = tf(
                        loc,
                        body_key,
                        &[("current", current.as_str()), ("required", bundled.as_str())],
                    );
                    let backend_error = browser_extension_update_error
                        .get()
                        .or_else(|| status.error.clone());
                    let target_version = bundled.clone();
                    let update_path = path.clone();
                    let copy_path = path.clone();
                    view! {
                        <section class="exploration-banner browser-extension-update"
                            data-testid="browser-extension-update-banner" role="status">
                            <div class="exploration-banner-copy">
                                <span class="exploration-banner-eyebrow">{t(loc, "browser.extension.eyebrow")}</span>
                                <strong>{t(loc, title_key)}</strong>
                                <span>{body}</span>
                                {(!path.is_empty()).then(|| view! {
                                    <code class="browser-extension-path" data-testid="browser-extension-path">{path}</code>
                                })}
                                {backend_error.map(|error| view! {
                                    <span class="browser-extension-error" role="alert">{error}</span>
                                })}
                            </div>
                            <div class="exploration-banner-actions">
                                {(!manual).then(|| view! {
                                    <button type="button" class="primary"
                                        disabled=move || browser_extension_update_busy.get()
                                        on:click=move |_| {
                                            if browser_extension_update_busy.get_untracked() {
                                                return;
                                            }
                                            browser_extension_update_busy.set(true);
                                            browser_extension_update_error.set(None);
                                            let update_path = update_path.clone();
                                            spawn_local(async move {
                                                let value = invoke("update_browser_extension", JsValue::UNDEFINED).await;
                                                match from_value::<BrowserExtensionUpdateResult>(value) {
                                                    Ok(result) => {
                                                        let outcome = result.outcome.clone();
                                                        browser_extension_update_outcome.set(Some(outcome.clone()));
                                                        browser_extension_update_error
                                                            .set(result.error.clone().or_else(|| result.status.error.clone()));
                                                        if outcome == "updated" {
                                                            browser_extension_status.set(None);
                                                            show_actionable_toast(&t(
                                                                locale.get_untracked(),
                                                                "browser.extension.updated",
                                                            ));
                                                        } else {
                                                            browser_extension_status.set(Some(result.status));
                                                            if outcome == "manual_reload_required" {
                                                                if !update_path.is_empty() {
                                                                    if let Some(window) = web_sys::window() {
                                                                        let _ = wasm_bindgen_futures::JsFuture::from(
                                                                            window.navigator().clipboard().write_text(&update_path),
                                                                        )
                                                                        .await;
                                                                    }
                                                                }
                                                                show_actionable_warning_toast(&t(
                                                                    locale.get_untracked(),
                                                                    "browser.extension.manual_ready",
                                                                ));
                                                            }
                                                        }
                                                    }
                                                    Err(error) => {
                                                        browser_extension_update_error.set(Some(error.to_string()));
                                                        show_actionable_warning_toast(&t(
                                                            locale.get_untracked(),
                                                            "browser.extension.failed",
                                                        ));
                                                    }
                                                }
                                                browser_extension_update_busy.set(false);
                                            });
                                        }>
                                        {move || if browser_extension_update_busy.get() {
                                            t(locale.get(), "browser.extension.updating")
                                        } else {
                                            t(locale.get(), "browser.extension.update")
                                        }}
                                    </button>
                                })}
                                {(!copy_path.is_empty()).then(|| view! {
                                    <button type="button"
                                        on:click=move |_| {
                                            let path = copy_path.clone();
                                            spawn_local(async move {
                                                if let Some(window) = web_sys::window() {
                                                    let _ = wasm_bindgen_futures::JsFuture::from(
                                                        window.navigator().clipboard().write_text(&path),
                                                    )
                                                    .await;
                                                    show_actionable_toast(&t(
                                                        locale.get_untracked(),
                                                        "browser.extension.path_copied",
                                                    ));
                                                }
                                            });
                                        }>{t(loc, "browser.extension.copy_path")}</button>
                                })}
                                <button type="button"
                                    on:click=move |_| {
                                        spawn_local(async move {
                                            let setup = from_value::<BrowserExtensionSetup>(
                                                open_browser_extension_page().await,
                                            )
                                            .unwrap_or_default();
                                            if !setup.opened {
                                                show_actionable_warning_toast(&t(
                                                    locale.get_untracked(),
                                                    "browser.extension.open_failed",
                                                ));
                                            }
                                        });
                                    }>{t(loc, "browser.extension.open_page")}</button>
                                <button type="button"
                                    on:click=move |_| refresh_browser_extension_status.call(())>
                                    {t(loc, "browser.extension.recheck")}
                                </button>
                                <button type="button"
                                    on:click=move |_| {
                                        browser_extension_dismissed_version.set(Some(target_version.clone()));
                                        browser_extension_status.set(None);
                                        browser_extension_update_outcome.set(None);
                                        browser_extension_update_error.set(None);
                                    }>{t(loc, "browser.extension.dismiss")}</button>
                            </div>
                        </section>
                    }
                })}
                {move || {
                    let Some(notice) = browser_offline_notice.get() else {
                        return None;
                    };
                    (active_session.get().as_deref() == Some(notice.frame_id.as_str())).then(|| {
                        let retry_text = notice.retry_text.clone();
                        let can_retry = !retry_text.trim().is_empty();
                        view! {
                            <section class="exploration-banner browser-offline" data-testid="browser-offline-banner" role="status">
                                <div class="exploration-banner-copy">
                                    <span class="exploration-banner-eyebrow">{t(locale.get(), "browser.offline.eyebrow")}</span>
                                    <strong>{t(locale.get(), "browser.offline.title")}</strong>
                                    <span>{t(locale.get(), "browser.offline.body")}</span>
                                </div>
                                <div class="exploration-banner-actions">
                                    <button type="button" class="primary"
                                        disabled=!can_retry
                                        on:click=move |_| {
                                            if retry_text.trim().is_empty() {
                                                return;
                                            }
                                            input.set(retry_text.clone());
                                            browser_offline_notice.set(None);
                                            send.call(ComposerSendAction::Normal);
                                        }>{t(locale.get(), "browser.offline.retry")}</button>
                                    <button type="button"
                                        on:click=move |_| {
                                            spawn_local(async move {
                                                let reply = open_browser_extension_page().await;
                                                let setup = from_value::<BrowserExtensionSetup>(reply)
                                                    .unwrap_or_default();
                                                let path = setup
                                                    .extension_path
                                                    .filter(|path| !path.trim().is_empty());
                                                let has_path = path.is_some();
                                                // Keep the remaining manual steps to one paste:
                                                // the extension path goes onto the clipboard.
                                                if let Some(path) = path {
                                                    if let Some(window) = web_sys::window() {
                                                        let _ = wasm_bindgen_futures::JsFuture::from(
                                                            window.navigator().clipboard().write_text(&path),
                                                        )
                                                        .await;
                                                    }
                                                }
                                                if setup.opened && has_path {
                                                    show_actionable_toast(&t(locale.get_untracked(), "browser.offline.setup_done"));
                                                } else {
                                                    show_actionable_warning_toast(&t(locale.get_untracked(), "browser.offline.setup_failed"));
                                                }
                                            });
                                        }>{t(locale.get(), "browser.offline.setup")}</button>
                                    <button type="button"
                                        on:click=move |_| browser_offline_notice.set(None)>{t(locale.get(), "browser.offline.dismiss")}</button>
                                </div>
                            </section>
                        }
                    })
                }}
                {move || demo_mode.get().then(|| view! {
                    <div class="demo-read-only-notice" data-testid="demo-read-only" role="status">
                        {t(locale.get(), "projects.example_read_only")}
                    </div>
                })}
                {move || next_stopping_session(
                    stopping_session.get(),
                    active_session.get().as_deref(),
                    &running.get(),
                ).is_some().then(|| view! {
                    <div class="stopping-toast" data-testid="stopping-toast" role="status">
                        <span class="stopping-spinner"></span>
                        <div class="stopping-text">
                            <strong>{move || t(locale.get(), "composer.stopping")}</strong>
                            <span>{move || t(locale.get(), "composer.stopping_hint")}</span>
                        </div>
                    </div>
                })}
                {move || {
                    let Some(snapshot) = active_context_usage.get() else {
                        return None;
                    };
                    (context_usage_open.get()
                        && context_usage_mode.get() == ContextUsageMode::Docked)
                        .then(|| {
                            view! {
                                <div class="context-usage-slot">
                                    <ContextUsagePanel
                                        snapshot=snapshot
                                        floating=false
                                        locale=locale.read_only()
                                        context_usage_open=context_usage_open
                                        context_usage_details=context_usage_details
                                        context_usage_detail_open=context_usage_detail_open
                                        context_usage_geom=context_usage_geom
                                        on_header_down=on_context_usage_header_down
                                        on_header_dblclick=on_context_usage_header_dblclick
                                        on_dock=on_context_usage_dock
                                        on_resize_start=on_context_usage_resize_start
                                        on_compact=compact_from_usage
                                        on_new_session=new_session_from_usage
                                        compact_disabled=Signal::derive(move || busy.get())
                                    />
                                </div>
                            }
                        })
                }}
                {move || (!demo_mode.get()).then(|| view! {
                    <ComposerQueue
                        items=items
                        user_offset=composer_queue_offset
                        can_cut_in=composer_queue_can_cut_in
                        on_queue=on_queue
                    />
                })}
                {move || (!demo_mode.get()).then(|| view! {
                    <SessionRuntimeStrip
                        locale=locale
                        execution_contexts=execution_contexts
                        session_execution_contexts=session_execution_contexts
                        default_execution_context=default_execution_context
                        runtimes=runtime_infos
                        active_project=project_info
                        projects=proj_list
                        runtime_environment=runtime_environment
                        runtime_environment_pinned=runtime_environment_pinned
                        object_states=runtime_object_states
                        context_details_modal=context_details_modal
                        selected_context_id=selected_context_id
                    />
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
                    {move || feedback_context.get().is_some().then(|| view! {
                        <div class="composer-attachments composer-reference-chips" data-testid="feedback-context">
                            <div class="composer-attachment-row composer-reference-card context">
                                <span class="composer-attachment-icon">{compose_icon("server")}</span>
                                <span class="composer-attachment-copy">
                                    <span class="composer-attachment ready">{move || t(locale.get(), "issue_report.context")}</span>
                                    <span class="composer-attachment-meta">{move || t(locale.get(), "issue_report.context_attached")}</span>
                                </span>
                                <button type="button" class="composer-attachment-remove"
                                    title=move || t(locale.get(), "composer.remove_attachment")
                                    aria-label=move || t(locale.get(), "composer.remove_attachment")
                                    on:click=move |_| feedback_context.set(None)>{compose_icon("close")}</button>
                            </div>
                        </div>
                    })}
                    {move || motif_selection.get().map(|selection| {
                        let selection_label = selection.feature_name.as_deref()
                            .filter(|name| !name.trim().is_empty())
                            .map(|name| format!("{} · {name}", selection.record_name.clone()))
                            .unwrap_or_else(|| selection.record_name.clone());
                        view! { <div class="composer-attachments composer-reference-chips" data-testid="motif-selection-reference">
                            <div class="composer-attachment-row composer-reference-card motif-selection">
                                <span class="composer-attachment-icon">{compose_icon("dna")}</span>
                                <span class="composer-attachment-copy">
                                    <span class="composer-attachment ready">{selection_label}</span>
                                    <span class="composer-attachment-meta">{format!("{}-{} · {} bp", selection.start, selection.end, selection.length_bp())}</span>
                                </span>
                                <button type="button" class="composer-attachment-remove"
                                    title=move || t(locale.get(), "composer.remove_attachment")
                                    aria-label=move || t(locale.get(), "composer.remove_attachment")
                                    on:click=move |_| {
                                        let block = selection.composer_text();
                                        input.update(|draft| *draft = draft.replace(&block, "").trim().to_string());
                                        motif_selection.set(None);
                                    }>{compose_icon("close")}</button>
                            </div>
                        </div> }
                    })}
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
                            disabled=move || composer_scope_locked.get()
                            style=move || {
                                if composer_h_custom.get() {
                                    format!("height:{}px", composer_h.get())
                                } else {
                                    format!("max-height:{}px", composer_h.get())
                                }
                            }
                            prop:value={move || input.get()}
                            on:input=move |ev: web_sys::Event| {
                                let Some(input_event) = ev.dyn_ref::<web_sys::InputEvent>() else {
                                    return;
                                };
                                let Some(textarea) = ev.target()
                                    .and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
                                else {
                                    return;
                                };
                                let v = textarea.value();
                                let input_type = input_event.input_type();
                                let prior_mode = picker_mode.get_untracked();
                                let prior_range = picker_token_range.get_untracked();
                                let manual_edit = matches!(
                                    input_type.as_str(),
                                    "insertText"
                                        | "insertCompositionText"
                                        | "insertFromComposition"
                                        | "deleteCompositionText"
                                        | "deleteContentBackward"
                                        | "deleteContentForward"
                                );
                                let active = textarea
                                    .selection_start()
                                    .ok()
                                    .flatten()
                                    .and_then(|caret| active_composer_trigger(&v, caret as usize));
                                match active {
                                    Some((start, end, mode, query))
                                        if manual_edit
                                            && composer_picker_accepts_edit(
                                                &input_type,
                                                (prior_mode == Some(mode)).then_some(mode),
                                                prior_range.map(|(prior_start, _)| prior_start),
                                                start,
                                                query.is_empty(),
                                            ) =>
                                    {
                                        picker_token_range.set(Some((start, end)));
                                        picker_query.set(query);
                                        picker_index.set(0);
                                        picker_mode.set(Some(mode));
                                    }
                                    _ => picker_mode.set(None),
                                }
                                input.set(v);
                            }
                            on:keydown:undelegated=on_send
                            on:paste=on_paste
                            prop:placeholder=move || {
                                if matches!(active_branch_state.get().as_deref(), Some("merged" | "orphaned")) {
                                    t(locale.get(), "branch.frozen_placeholder").into()
                                } else if mainline_frozen.get() {
                                    t(locale.get(), "exploration.mainline_frozen_placeholder").into()
                                } else if composer_scope_locked.get() {
                                    t(locale.get(), "exploration.read_only_placeholder").into()
                                } else {
                                    tf(
                                        locale.get(),
                                        "composer.placeholder",
                                        &[("modifier", if is_mac() { "Cmd" } else { "Ctrl" })],
                                    )
                                }
                            }
                        ></textarea>
                        {move || picker_mode.get().map(|mode| {
                            let loc = locale.get();
                            let matches = picker_items.get();
                            // The `/` menu layers its rows under small section
                            // labels; the reference pickers keep one title for
                            // the whole list.
                            let grouped = matches!(mode, ComposerPickerMode::Skill);
                            let title = match mode {
                                ComposerPickerMode::Artifact => Some("composer.ref_artifacts"),
                                ComposerPickerMode::Session => Some("composer.ref_sessions"),
                                ComposerPickerMode::Skill => None,
                            };
                            let mut last_section = None;
                            view! {
                                <div class="mention-backdrop" on:mousedown=move |_| picker_mode.set(None)></div>
                                <div class="mention-menu">
                                    {title.map(|key| view! { <div class="mention-group-label">{t(loc, key)}</div> })}
                                    {matches.into_iter().enumerate().map(|(i, item)| {
                                        let section = if grouped { picker_item_section(&item) } else { None };
                                        let header = (section != last_section).then_some(section).flatten();
                                        last_section = section;
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
                                            ComposerPickerItem::Command { name, description } => {
                                                let icon = slash_command_icon(&name);
                                                (format!("/{name}"), description, icon)
                                            }
                                            ComposerPickerItem::Workflow(workflow) => (
                                                workflow.name,
                                                workflow.description,
                                                "branch",
                                            ),
                                            ComposerPickerItem::Context { id, label } => (label, id, "server"),
                                            ComposerPickerItem::Runtime { context_id, context_label, language } => (
                                                format!("{} runtime", language_display(&language)),
                                                format!("{context_label} · {context_id}"),
                                                "terminal",
                                            ),
                                        };
                                        view! {
                                            {header.map(|key| view! { <div class="mention-group-label">{t(loc, key)}</div> })}
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
                                {compose_icon("plus")}
                            </button>
                            {move || compose_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compose_menu_open.set(false)></div>
                                <div class="compose-menu">
                                    <div class="compose-menu-title">{move || t(locale.get(), "composer.compose")}</div>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.group_add")}</div>
                                        <button type="button" class="compose-item" disabled=composer_blocked
                                            on:click=move |_| { compose_menu_open.set(false); pick_files(()); }>
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
                                        <button type="button" class="compose-item" disabled=composer_blocked
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                if let Some(sid) = active_session.get() {
                                                    request_session_review.call(sid);
                                                }
                                            }>
                                            <span class="compose-item-icon">{compose_icon("review")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.request_review")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.request_review_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item" disabled=move || !can_share.get()
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                open_share.call(());
                                            }>
                                            <span class="compose-item-icon">{compose_icon("share")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "share.title")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.share_sub")}</span>
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
                                class:has-resource=move || {
                                    !session_execution_contexts.get().is_empty()
                                        || is_remote_default_context_id(
                                            default_execution_context.get().as_deref(),
                                        )
                                }
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
                                let locked = session_has_items.get();
                                view! {
                                <div class="compose-backdrop" on:click=move |_| {
                                    agent_menu_open.set(false);
                                    reviewer_model_menu_open.set(false);
                                    specialist_menu_open.set(false);
                                    compute_menu_open.set(false);
                                }></div>
                                <div class="compose-menu agent-menu" role="menu"
                                    aria-label=move || t(locale.get(), "composer.agent_options")>
                                    {move || {
                                        // One control, two backends: ACP-bound sessions switch the
                                        // agent's own plan/default mode pair (and get no row when the
                                        // agent has none), built-in sessions flip the local flag.
                                        let session_id = active_session.get();
                                        let local = local_plan_mode.get();
                                        let acp_pair = match (&local, &session_id) {
                                            (None, Some(id)) => acp_session_modes
                                                .with(|all| plan_mode_pair(all.get(id))),
                                            _ => None,
                                        };
                                        if local.is_none() && acp_pair.is_none() {
                                            return None;
                                        }
                                        // `plan_mode_active` already resolves the same
                                        // two backends — keep one source of truth.
                                        let on_plan = plan_mode_active.get();
                                        Some(view! {
                                            <label class="agent-menu-row"
                                                title=move || t(locale.get(), if on_plan { "plan.switch_default" } else { "plan.switch_plan" })>
                                                <span>{move || t(locale.get(), "composer.plan_first")}</span>
                                                <span class="toggle agent-menu-toggle">
                                                    <input type="checkbox" data-testid="plan-first-toggle"
                                                        prop:checked=on_plan
                                                        disabled=move || plan_mode_busy.get()
                                                        on:change=move |ev| {
                                                            set_plan_first.call(event_target_checked(&ev));
                                                        } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </span>
                                            </label>
                                        })
                                    }}
                                    <label class="agent-menu-row"
                                        title=move || t(locale.get(), "full_permission.confirm_body")>
                                        <span>{move || t(locale.get(), "composer.full_permission")}</span>
                                        <span class="toggle agent-menu-toggle">
                                            <input type="checkbox"
                                                data-testid="full-permission-toggle"
                                                prop:checked=move || full_permission_enabled.get()
                                                disabled=move || full_permission_busy.get()
                                                on:change=move |event| {
                                                    let enabled = event_target_checked(&event);
                                                    if enabled {
                                                        // The mode is not active until the warning
                                                        // is confirmed. Keep the underlying input in
                                                        // sync while the modal is open.
                                                        if let Some(target) = event.target() {
                                                            if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                                                input.set_checked(false);
                                                            }
                                                        }
                                                        ui_confirm.set(Some(UiConfirm::EnableFullPermission));
                                                        return;
                                                    }
                                                    disable_full_permission.call(());
                                                } />
                                            <span class="toggle-track" aria-hidden="true"></span>
                                        </span>
                                    </label>
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
                                    <label class="agent-menu-row">
                                        <span>{move || t(locale.get(), "composer.auto_failure_analysis")}</span>
                                        <span class="toggle agent-menu-toggle">
                                            <input type="checkbox"
                                                data-testid="auto-failure-analysis"
                                                prop:checked=move || auto_failure_analysis.get().enabled
                                                on:change=move |event| {
                                                    let mut next = auto_failure_analysis.get_untracked();
                                                    next.enabled = event_target_checked(&event);
                                                    save_auto_failure_analysis.call(next);
                                                } />
                                            <span class="toggle-track" aria-hidden="true"></span>
                                        </span>
                                    </label>
                                    {move || auto_failure_analysis.get().enabled.then(|| view! {
                                        <label class="agent-menu-row agent-menu-setting">
                                            <span>{move || t(locale.get(), "composer.failure_rate_threshold")}</span>
                                            <input class="agent-menu-number" type="number" min="1" max="100" step="1"
                                                data-testid="failure-rate-threshold"
                                                prop:value=move || auto_failure_analysis.get().failure_rate_threshold.to_string()
                                                on:change=move |event| {
                                                    let Ok(value) = dom_value(&event).parse::<u8>() else { return; };
                                                    let mut next = auto_failure_analysis.get_untracked();
                                                    next.failure_rate_threshold = value;
                                                    save_auto_failure_analysis.call(next);
                                                } />
                                        </label>
                                        <label class="agent-menu-row agent-menu-setting">
                                            <span>{move || t(locale.get(), "composer.minimum_failures")}</span>
                                            <input class="agent-menu-number" type="number" min="1" max="100" step="1"
                                                data-testid="minimum-failures"
                                                prop:value=move || auto_failure_analysis.get().minimum_failures.to_string()
                                                on:change=move |event| {
                                                    let Ok(value) = dom_value(&event).parse::<u16>() else { return; };
                                                    let mut next = auto_failure_analysis.get_untracked();
                                                    next.minimum_failures = value;
                                                    save_auto_failure_analysis.call(next);
                                                } />
                                        </label>
                                    })}
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
                                            let default_id = default_execution_context.get();
                                            let label = default_id.as_ref().map(|id| {
                                                compute_default_label(id, &execution_contexts.get())
                                            });
                                            compute_menu_summary(
                                                locale.get(),
                                                default_id.as_deref(),
                                                label.as_deref(),
                                                session_execution_contexts.get().len(),
                                            )
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
                                                choices.extend(
                                                    models
                                                        .get()
                                                        .into_iter()
                                                        .filter(ModelProfile::is_chat_model)
                                                        .map(|model| {
                                                            (format!("http:{}", model.id), model.label)
                                                        }),
                                                );
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
                                            <p class="compute-menu-hint">{move || t(locale.get(), "compute.menu_hint")}</p>
                                            <div class="compute-default-field">
                                                <label>
                                                    <span>{move || t(locale.get(), "environments.default_analysis")}</span>
                                                    <DefaultAnalysisSelect
                                                        locale=locale
                                                        execution_contexts=execution_contexts
                                                        default_execution_context=default_execution_context
                                                        on_change=set_default_compute_resource
                                                        test_id="compute-default-analysis".to_string()
                                                    />
                                                </label>
                                            </div>
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
                                                    let is_analysis_default =
                                                        default_execution_context.get().as_deref() == Some(context_id.as_str());
                                                    let toggle_id = context_id.clone();
                                                    let default_id = context_id.clone();
                                                    view! {
                                                        <div class="agent-submenu-row compute-resource-row"
                                                            class:enabled=enabled data-context-id=context_id.clone()>
                                                            <button type="button" class="compute-resource-toggle"
                                                                aria-pressed=enabled.to_string()
                                                                on:click=move |_| {
                                                                    toggle_session_compute_resource.call((toggle_id.clone(), !enabled));
                                                                }>
                                                                <span class="compute-resource-icon">{compose_icon("server")}</span>
                                                                <span class="compute-resource-name-wrap">
                                                                    <span class="compute-resource-name">{host.alias}</span>
                                                                    {is_analysis_default.then(|| view! {
                                                                        <span class="compute-resource-default">{t(locale.get(), "compute.analysis_default")}</span>
                                                                    })}
                                                                </span>
                                                                <span class="compute-resource-state">
                                                                    {t(locale.get(), compute_resource_state_key(enabled, is_analysis_default))}
                                                                </span>
                                                            </button>
                                                            <button type="button" class="compute-resource-default-toggle"
                                                                class:active=is_analysis_default
                                                                title=move || t(locale.get(), if is_analysis_default { "compute.clear_default" } else { "compute.set_default" })
                                                                aria-label=move || t(locale.get(), if is_analysis_default { "compute.clear_default" } else { "compute.set_default" })
                                                                on:click=move |_| {
                                                                    set_default_compute_resource.call(if is_analysis_default { None } else { Some(default_id.clone()) });
                                                                }>
                                                                {compose_icon("star")}
                                                            </button>
                                                        </div>
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
                                                    let is_analysis_default =
                                                        default_execution_context.get().as_deref() == Some(context_id.as_str());
                                                    let toggle_id = context_id.clone();
                                                    let default_id = context_id.clone();
                                                    let name = if ctx.label.trim().is_empty() { ctx.id.clone() } else { ctx.label.clone() };
                                                    let is_wsl_default = serde_json::from_str::<serde_json::Value>(&ctx.config_json)
                                                        .ok()
                                                        .and_then(|cfg| cfg.get("is_default").and_then(|v| v.as_bool()))
                                                        .unwrap_or(false);
                                                    view! {
                                                        <div class="agent-submenu-row compute-resource-row"
                                                            class:enabled=enabled data-context-id=context_id.clone()>
                                                            <button type="button" class="compute-resource-toggle"
                                                                aria-pressed=enabled.to_string()
                                                                on:click=move |_| {
                                                                    toggle_session_compute_resource.call((toggle_id.clone(), !enabled));
                                                                }>
                                                                <span class="compute-resource-icon">{compose_icon("terminal")}</span>
                                                                <span class="compute-resource-name-wrap">
                                                                    <span class="compute-resource-name">{name}</span>
                                                                    {is_wsl_default.then(|| view! {
                                                                        <span class="compute-resource-default">{t(locale.get(), "compute.wsl_default")}</span>
                                                                    })}
                                                                    {is_analysis_default.then(|| view! {
                                                                        <span class="compute-resource-default">{t(locale.get(), "compute.analysis_default")}</span>
                                                                    })}
                                                                </span>
                                                                <span class="compute-resource-state">
                                                                    {t(locale.get(), compute_resource_state_key(enabled, is_analysis_default))}
                                                                </span>
                                                            </button>
                                                            <button type="button" class="compute-resource-default-toggle"
                                                                class:active=is_analysis_default
                                                                title=move || t(locale.get(), if is_analysis_default { "compute.clear_default" } else { "compute.set_default" })
                                                                aria-label=move || t(locale.get(), if is_analysis_default { "compute.clear_default" } else { "compute.set_default" })
                                                                on:click=move |_| {
                                                                    set_default_compute_resource.call(if is_analysis_default { None } else { Some(default_id.clone()) });
                                                                }>
                                                                {compose_icon("star")}
                                                            </button>
                                                        </div>
                                                    }
                                                }).collect_view()}}
                                            </div>
                                            <button type="button" class="agent-submenu-row compute-add-host-row" on:click=move |_| {
                                                agent_menu_open.set(false);
                                                compute_menu_open.set(false);
                                                open_add_host_form.call(());
                                            }>
                                                <span>{move || t(locale.get(), "compute.add_host")}</span>
                                            </button>
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
                            {move || active_context_usage.get().map(|snapshot| {
                                let pct = context_percent(snapshot.used, snapshot.max);
                                let gauge_angle = -90.0 + pct as f64 * 0.9;
                                let tone = context_usage_tone(snapshot.used, snapshot.max);
                                let percent_label = context_usage_percent_label(snapshot.used, snapshot.max);
                                let tooltip = context_usage_tooltip(&snapshot, locale.get());
                                let aria = if snapshot.max == 0 {
                                    t(locale.get(), "context_usage.open_unknown")
                                } else {
                                    tf(
                                        locale.get(),
                                        "context_usage.open_pct",
                                        &[("pct", &pct.to_string())],
                                    )
                                };
                                let tone_warn = tone == ContextUsageTone::Warn;
                                let tone_danger = tone == ContextUsageTone::Danger;
                                let tone_unknown = tone == ContextUsageTone::Unknown;
                                view! {
                                    <button type="button" class="context-usage-trigger"
                                        class:is-warn=tone_warn
                                        class:is-danger=tone_danger
                                        class:is-unknown=tone_unknown
                                        class:is-compacted=move || context_usage_flash.get()
                                        data-testid="context-usage-trigger"
                                        data-tone=tone.as_str()
                                        style=format!("--context-gauge-angle:{gauge_angle:.1}deg")
                                        title=tooltip
                                        aria-label=aria
                                        aria-expanded=move || context_usage_open.get().to_string()
                                        aria-controls="context-usage-panel"
                                        on:click=move |event| {
                                            event.stop_propagation();
                                            let opening = !context_usage_open.get_untracked();
                                            context_usage_open.set(opening);
                                            if opening && active_acp_agent_id.get_untracked().is_none()
                                                && context_usage_details.get_untracked().is_none()
                                            {
                                                if let Some(session_id) = active_session.get_untracked() {
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({
                                                            "sessionId": session_id,
                                                        })).unwrap();
                                                        if let Ok(value) = invoke_checked("get_context_usage_details", arg).await {
                                                            if let Ok(details) = from_value::<ContextUsageDetails>(value) {
                                                                context_usage_details.set(Some(details));
                                                            }
                                                        }
                                                    });
                                                }
                                            }
                                        }>
                                        {compose_icon("gauge")}
                                        <span class="context-usage-pct" data-testid="context-usage-percent">{percent_label}</span>
                                    </button>
                                }
                            })}
                            {move || fast_profile.get().map(|_| {
                                let enabled = fast_enabled.get();
                                let session_override = fast_is_session_override.get();
                                let saving = service_tier_busy.get();
                                let running_now = busy.get();
                                let loaded = fast_loaded.get();
                                let key = if running_now {
                                    "composer.fast.running"
                                } else if saving || !loaded {
                                    "composer.fast.saving"
                                } else {
                                    match (enabled, session_override) {
                                        (true, true) => "composer.fast.on_session",
                                        (true, false) => "composer.fast.on_profile",
                                        (false, true) => "composer.fast.off_session",
                                        (false, false) => "composer.fast.off_profile",
                                    }
                                };
                                let title = t(locale.get(), key).to_string();
                                view! {
                                    <button type="button" class="composer-fast"
                                        class:enabled=enabled
                                        class:pending=saving || !loaded
                                        data-testid="composer-fast-toggle"
                                        aria-pressed=enabled.to_string()
                                        aria-label=title.clone()
                                        title=title
                                        disabled=running_now || saving || !loaded
                                        on:click=move |_| toggle_fast.call(())>
                                        {compose_icon("bolt")}
                                    </button>
                                }
                            })}
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
                                        <div class="model-menu"
                                            style=move || effort_menu_for.get()
                                                .map(|_| format!("right:{:.0}px", effort_menu_shift.get()))
                                                .unwrap_or_default()
                                            on:click=move |_| effort_menu_for.set(None)
                                            on:scroll=move |_| effort_menu_for.set(None)>
                                            {move || {
                                                let list = models.get();
                                                let selected = active_session.get().and_then(|session_id| {
                                                    session_model_ids.with(|models| models.get(&session_id).cloned())
                                                });
                                                let acp_selected = active_acp_agent_id.get().is_some();
                                                let acp_locked = acp_selected && session_has_items.get();
                                                list.into_iter().filter(ModelProfile::is_chat_model).map(|m| {
                                                    let pick_id = m.id.clone();
                                                    let pick_label = m.label.clone();
                                                    let pick_supports_vision = m.supports_vision;
                                                    let is_active = !acp_selected
                                                        && selected.as_deref().map_or(m.active, |id| id == m.id);
                                                    let show_sub = !m.model.is_empty() && m.model != m.label;
                                                    let effort = m.reasoning_effort.clone();
                                                    let effort_id = m.id.clone();
                                                    let effort_id_open = m.id.clone();
                                                    let effort_id_expanded = m.id.clone();
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
                                                                    model_switch_confirm.set(Some((
                                                                        id,
                                                                        pick_label.clone(),
                                                                        !pick_supports_vision,
                                                                    )));
                                                                }
                                                            }>
                                                                <span class="model-menu-text">
                                                                    <span class="model-menu-label">{m.label.clone()}</span>
                                                                    {show_sub.then(|| view! { <span class="model-menu-sub">{m.model.clone()}</span> })}
                                                                    {(!effort.is_empty()).then(|| view! {
                                                                        <span class="model-menu-effort-tag">{effort}</span>
                                                                    })}
                                                                </span>
                                                                {is_active.then(|| view! { <span class="model-menu-check">"✓"</span> })}
                                                            </button>
                                                            <button type="button" class="model-menu-effort-edit"
                                                                class:open=move || effort_menu_for.get().as_ref().is_some_and(|(open_id, _, _)| open_id == &effort_id_open)
                                                                title=move || t(locale.get(), "settings.reasoning_effort")
                                                                attr:aria-expanded=move || if effort_menu_for.get().as_ref().is_some_and(|(open_id, _, _)| open_id == &effort_id_expanded) { "true" } else { "false" }
                                                                on:click=move |ev| {
                                                                    ev.stop_propagation();
                                                                    if effort_menu_for.get_untracked().as_ref().is_some_and(|(open_id, _, _)| open_id == &effort_id) {
                                                                        effort_menu_for.set(None);
                                                                        return;
                                                                    }
                                                                    let Some(el) = ev.target().and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok()) else { return; };
                                                                    let rect = el.get_bounding_client_rect();
                                                                    let menu_rect = el
                                                                        .closest(".model-menu")
                                                                        .ok()
                                                                        .flatten()
                                                                        .map(|menu| menu.get_bounding_client_rect());
                                                                    let menu_right = menu_rect
                                                                        .as_ref()
                                                                        .map(|menu| menu.right())
                                                                        .unwrap_or(rect.right());
                                                                    let menu_left = menu_rect
                                                                        .as_ref()
                                                                        .map(|menu| menu.left())
                                                                        .unwrap_or(rect.left());
                                                                    // When switching directly from one model's effort editor to
                                                                    // another, the menu is already shifted left. Recover its
                                                                    // unshifted coordinates before calculating the next flyout.
                                                                    let applied_shift = if effort_menu_for.get_untracked().is_some() {
                                                                        effort_menu_shift.get_untracked()
                                                                    } else {
                                                                        0.0
                                                                    };
                                                                    let base_menu_right = menu_right + applied_shift;
                                                                    let base_menu_left = menu_left + applied_shift;
                                                                    // Keep in sync with the flyout width in chat.css.
                                                                    const FLYOUT_WIDTH: f64 = 200.0;
                                                                    // Generous height allowance (default + every known level + label)
                                                                    // so the flyout never runs past the viewport bottom.
                                                                    const FLYOUT_MAX_HEIGHT: f64 = 340.0;
                                                                    let window = web_sys::window();
                                                                    let viewport_w = window
                                                                        .as_ref()
                                                                        .and_then(|w| w.inner_width().ok())
                                                                        .and_then(|w| w.as_f64())
                                                                        .unwrap_or(1280.0);
                                                                    let viewport_h = window
                                                                        .and_then(|w| w.inner_height().ok())
                                                                        .and_then(|h| h.as_f64())
                                                                        .unwrap_or(800.0);
                                                                    let desired_left = base_menu_right + 6.0;
                                                                    let max_left = (viewport_w - FLYOUT_WIDTH - 8.0).max(8.0);
                                                                    let shift = (desired_left - max_left)
                                                                        .max(0.0)
                                                                        .min((base_menu_left - 8.0).max(0.0));
                                                                    let left = desired_left - shift;
                                                                    let top = (rect.top() - 4.0).clamp(8.0, (viewport_h - FLYOUT_MAX_HEIGHT - 8.0).max(8.0));
                                                                    effort_menu_shift.set(shift);
                                                                    effort_menu_for.set(Some((effort_id.clone(), left, top)));
                                                                }>
                                                                <span class="model-menu-effort-edit-label">{move || t(locale.get(), "menu.edit")}</span>
                                                            </button>
                                                        </div>
                                                    }
                                                }).collect_view()
                                            }}
                                            {move || (!acp_agents.get().is_empty()).then(|| view! {
                                                <div class="compose-group-label model-menu-acp-label">"ACP"</div>
                                                {acp_agents.get().into_iter().map(|agent| {
                                                    let id = agent.id.clone();
                                                    let active = active_acp_agent_id.get().as_deref() == Some(agent.id.as_str());
                                                    let starts_new_session = session_has_items.get() && !active;
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
                                                                    sel_artifact.set(0);
                                                                    right_tab.set(RightTab::Artifacts);
                                                                    spawn_local(async move {
                                                                        let frame_id = match invoke_new_session().await {
                                                                            Ok(frame_id) => frame_id,
                                                                            Err(error) => {
                                                                                status.set(send_failed(locale.get(), &error));
                                                                                return;
                                                                            }
                                                                        };
                                                                        replace_visible_transcript(
                                                                            active_session.get_untracked(),
                                                                            None,
                                                                            Vec::new(),
                                                                            items,
                                                                            transcripts,
                                                                            running,
                                                                        );
                                                                        provisional_acp_selection.set(Some((frame_id.clone(), agent_id.clone())));
                                                                        active_acp_agent_id.set(Some(agent_id));
                                                                        active_session.set(Some(frame_id));
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
                                            {move || active_acp_agent_id.get().and_then(|_| {
                                                let session_id = active_session.get()?;
                                                let options = acp_session_configs
                                                    .get()
                                                    .get(&session_id)
                                                    .cloned()
                                                    .unwrap_or_default();
                                                let modes_state = acp_session_modes.get().get(&session_id).cloned();
                                                let mode = modes_state
                                                    .as_ref()
                                                    .and_then(|state| state.get("currentModeId"))
                                                    .and_then(serde_json::Value::as_str)
                                                    .map(str::to_string);
                                                let available_modes: Vec<(String, String)> = modes_state
                                                    .as_ref()
                                                    .and_then(|state| state.get("availableModes"))
                                                    .and_then(serde_json::Value::as_array)
                                                    .map(|modes| {
                                                        modes
                                                            .iter()
                                                            .filter_map(|mode| {
                                                                let id = mode
                                                                    .get("id")
                                                                    .and_then(serde_json::Value::as_str)?
                                                                    .to_string();
                                                                let name = mode
                                                                    .get("name")
                                                                    .and_then(serde_json::Value::as_str)
                                                                    .unwrap_or(&id)
                                                                    .to_string();
                                                                Some((id, name))
                                                            })
                                                            .collect()
                                                    })
                                                    .unwrap_or_default();
                                                (!options.is_empty() || mode.is_some()).then(|| view! {
                                                    <div class="model-menu-configs" data-testid="acp-session-config">
                                                        <div class="compose-group-label">
                                                            {move || t(locale.get(), "composer.session_options")}
                                                        </div>
                                                        {(!options.iter().any(|option| {
                                                            option.get("id").and_then(serde_json::Value::as_str) == Some("mode")
                                                                || option
                                                                    .get("name")
                                                                    .and_then(serde_json::Value::as_str)
                                                                    .is_some_and(|name| name.eq_ignore_ascii_case("mode"))
                                                        }))
                                                            .then(|| {
                                                                mode.map(|mode| {
                                                                    let current_label = available_modes
                                                                        .iter()
                                                                        .find(|(id, _)| id == &mode)
                                                                        .map(|(_, name)| name.clone())
                                                                        .unwrap_or_else(|| mode.clone());
                                                                    if available_modes.len() < 2 {
                                                                        return view! {
                                                                            <div class="model-menu-config-row" title="Session mode">
                                                                                <span class="model-menu-config-label">"Mode"</span>
                                                                                <span class="model-menu-config-value">{current_label}</span>
                                                                            </div>
                                                                        }
                                                                        .into_view();
                                                                    }
                                                                    let frame_id = session_id.clone();
                                                                    view! {
                                                                        <label class="model-menu-config-row" title="Session mode">
                                                                            <span class="model-menu-config-label">"Mode"</span>
                                                                            <select class="model-menu-config-select" aria-label="Session mode"
                                                                                on:change=move |event| {
                                                                                    let mode_id = dom_value(&event);
                                                                                    let frame_id = frame_id.clone();
                                                                                    spawn_local(async move {
                                                                                        apply_acp_mode(acp_session_modes, frame_id, mode_id).await;
                                                                                    });
                                                                                }>
                                                                                {available_modes.into_iter().map(|(mode_id, label)| {
                                                                                    let selected = mode_id == mode;
                                                                                    view! {
                                                                                        <option value=mode_id prop:selected=selected>{label}</option>
                                                                                    }
                                                                                }).collect_view()}
                                                                            </select>
                                                                        </label>
                                                                    }
                                                                    .into_view()
                                                                })
                                                            })}
                                                        {options.into_iter().map(|option| {
                                                            let config_id = option
                                                                .get("id")
                                                                .and_then(serde_json::Value::as_str)
                                                                .unwrap_or_default()
                                                                .to_string();
                                                            let name = option
                                                                .get("name")
                                                                .and_then(serde_json::Value::as_str)
                                                                .unwrap_or(&config_id)
                                                                .to_string();
                                                            let description = option
                                                                .get("description")
                                                                .and_then(serde_json::Value::as_str)
                                                                .unwrap_or_default()
                                                                .to_string();
                                                            if option.get("type").and_then(serde_json::Value::as_str) == Some("boolean") {
                                                                let checked = option
                                                                    .get("currentValue")
                                                                    .and_then(serde_json::Value::as_bool)
                                                                    .unwrap_or(false);
                                                                let frame_id = session_id.clone();
                                                                view! {
                                                                    <label class="model-menu-config-row" title=description>
                                                                        <span class="model-menu-config-label">{name.clone()}</span>
                                                                        <span class="toggle model-menu-config-toggle">
                                                                            <input type="checkbox" aria-label=name prop:checked=checked
                                                                                on:change=move |event| {
                                                                                    let checked = event_target_checked(&event);
                                                                                    let frame_id = frame_id.clone();
                                                                                    let args = to_value(&serde_json::json!({
                                                                                        "frameId": frame_id,
                                                                                        "configId": config_id,
                                                                                        "value": { "type": "boolean", "value": checked },
                                                                                    })).unwrap();
                                                                                    spawn_local(async move {
                                                                                        if let Ok(value) = invoke_checked("set_acp_session_config", args).await {
                                                                                            if let Ok(options) = serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>(value) {
                                                                                                acp_session_configs.update(|all| {
                                                                                                    all.insert(frame_id, options);
                                                                                                });
                                                                                            }
                                                                                        }
                                                                                    });
                                                                                } />
                                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                                        </span>
                                                                    </label>
                                                                }
                                                                .into_view()
                                                            } else {
                                                                let current = option
                                                                    .get("currentValue")
                                                                    .and_then(serde_json::Value::as_str)
                                                                    .unwrap_or_default()
                                                                    .to_string();
                                                                let choices = acp_select_options(&option);
                                                                let frame_id = session_id.clone();
                                                                view! {
                                                                    <label class="model-menu-config-row" title=description>
                                                                        <span class="model-menu-config-label">{name.clone()}</span>
                                                                        <select class="model-menu-config-select" aria-label=name
                                                                            on:change=move |event| {
                                                                                let value = dom_value(&event);
                                                                                let frame_id = frame_id.clone();
                                                                                let args = to_value(&serde_json::json!({
                                                                                    "frameId": frame_id,
                                                                                    "configId": config_id,
                                                                                    "value": { "value": value },
                                                                                })).unwrap();
                                                                                spawn_local(async move {
                                                                                    if let Ok(value) = invoke_checked("set_acp_session_config", args).await {
                                                                                        if let Ok(options) = serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>(value) {
                                                                                            acp_session_configs.update(|all| {
                                                                                                all.insert(frame_id, options);
                                                                                            });
                                                                                        }
                                                                                    }
                                                                                });
                                                                            }>
                                                                            {choices.into_iter().map(|(value, label)| {
                                                                                let selected = value == current;
                                                                                view! {
                                                                                    <option value=value prop:selected=selected>{label}</option>
                                                                                }
                                                                            }).collect_view()}
                                                                        </select>
                                                                    </label>
                                                                }
                                                                .into_view()
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                })
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
                                        {move || effort_menu_for.get().and_then(|(id, left, top)| {
                                            let profile = models.get().into_iter().find(|m| m.id == id)?;
                                            let current = profile.reasoning_effort.clone();
                                            let mut values: Vec<String> = known_effort_values(&profile.provider, &profile.model)
                                                .unwrap_or(ALL_EFFORT_VALUES)
                                                .iter()
                                                .map(|v| v.to_string())
                                                .collect();
                                            // Keep a stored value visible even when the curated list
                                            // for this model doesn't include it.
                                            if !current.is_empty() && !values.iter().any(|v| v == &current) {
                                                values.push(current.clone());
                                            }
                                            let default_selected = current.is_empty();
                                            let style = format!("left:{left:.0}px;top:{top:.0}px");
                                            let default_id = id.clone();
                                            Some(view! {
                                                <div class="model-menu-effort-flyout" style=style data-effort-for=id.clone()
                                                    on:click=|ev| ev.stop_propagation()>
                                                    <div class="model-menu-effort-flyout-label">{move || t(locale.get(), "settings.reasoning_effort")}</div>
                                                    <button type="button" class="model-menu-effort-option" data-effort="default"
                                                        on:click=move |_| apply_model_effort.call((default_id.clone(), String::new()))>
                                                        <span class="model-menu-effort-option-label">{move || t(locale.get(), "settings.reasoning_effort.default")}</span>
                                                        {default_selected.then(|| view! { <span class="model-menu-effort-check">{compose_icon("check")}</span> })}
                                                    </button>
                                                    {values.into_iter().map(|lvl| {
                                                        let selected = !default_selected && lvl == current;
                                                        let pick = lvl.clone();
                                                        let option_id = id.clone();
                                                        view! {
                                                            <button type="button" class="model-menu-effort-option" data-effort=lvl.clone()
                                                                on:click=move |_| apply_model_effort.call((option_id.clone(), pick.clone()))>
                                                                <span class="model-menu-effort-option-label">{lvl}</span>
                                                                {selected.then(|| view! { <span class="model-menu-effort-check">{compose_icon("check")}</span> })}
                                                            </button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            })
                                        })}
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
                                                    send_side_chat((q, vec![], false));
                                                }
                                            }>
                                            <span class="compose-item-icon">{compose_icon("chat")}</span>
                                            <span>{move || t(locale.get(), "composer.side_chat")}</span>
                                        </button>
                                        {move || (active_branch_state.get().is_none()
                                            && !active_is_exploration.get()).then(|| view! {
                                            <button type="button" class="send-mode-item"
                                                on:click=move |_| {
                                                    send_mode_menu_open.set(false);
                                                    send.call(ComposerSendAction::BranchNew);
                                                }>
                                                <span class="compose-item-icon">{compose_icon("branch")}</span>
                                                <span>{move || t(locale.get(), "composer.branch_session")}</span>
                                            </button>
                                        })}
                                    </div>
                                })}
                            </div>
                        </div>
                    </div>
                    <div class="composer-footer">
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
            </div>
        </main>

        {move || (show_right.get() && !scratch_open.get() && !demo_mode.get()).then(|| view! {
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
                            let art_n = artifact_count.get();
                            let notebook_n = notebook_count.get();
                            let prov_n = provenance_count.get();
                            let highlight_n = highlight_count.get();
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
                                            // Labels ellipsize past 180px (long file names);
                                            // the tooltip keeps the full name one hover away.
                                            title=label.clone()
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
                                    let art_n = artifact_count.get();
                                    let notebook_n = notebook_count.get();
                                    let prov_n = provenance_count.get();
                                    let highlight_n = highlight_count.get();
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
                                let project_root = project_info
                                    .get()
                                    .map(|project| project.root)
                                    .unwrap_or_default();
                                let groups = group_artifact_indices(&arts, &project_root);
                                let tile_groups = groups.into_iter().map(|(key, indices)| {
                                    let label = artifact_group_label(&key, loc);
                                    let count = indices.len();
                                    let key_toggle = key.clone();
                                    let key_class = key.clone();
                                    let key_aria = key.clone();
                                    let tiles = indices.into_iter().map(|i| {
                                        let a = &arts[i];
                                        let name = a.name.clone();
                                        let discarded = a.source_discarded;
                                        let kind = a.kind.to_string();
                                        let meta = artifact_meta(a, loc);
                                        let file = if let PreviewData::File { path, kind } = &a.data {
                                            Some((path.clone(), kind.clone()))
                                        } else {
                                            None
                                        };
                                        let workspace_path = a.location.clone().or_else(|| {
                                            file.as_ref().map(|(path, _)| path.clone())
                                        });
                                        let file_click = file.clone();
                                        let context_path = workspace_path.clone().unwrap_or_default();
                                        let context_location = context_path.clone();
                                        let name_click = name.clone();
                                        let publication_source = db_artifacts.get().iter()
                                            .find(|artifact| artifact.id == a.id)
                                            .map(|artifact| PublicationEvidenceSource {
                                                kind: "artifact",
                                                id: artifact.id.clone(),
                                                label: artifact.name.clone(),
                                            });
                                        let tools = file.map(|(path, fkind)| {
                                        let (dl, vn) = (path.clone(), name.clone());
                                        let workspace_path = workspace_path.clone();
                                        let publication_source = publication_source.clone();
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
                                                let (mv, dw) = (p.clone(), p.clone());
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
                                                        {workspace_path.clone().map(|workspace_path| {
                                                            let attach_path = workspace_path.clone();
                                                            let reveal_path = workspace_path.clone();
                                                            let manager_path = workspace_path;
                                                            view! {
                                                                <button type="button" class="rp-tile-menu-item"
                                                                    on:click=move |_| {
                                                                        artifact_menu.set(None);
                                                                        let _ = attach_ready_path(attachments, attach_path.clone());
                                                                        focus_composer();
                                                                    }>
                                                                    {move || t(locale.get(), "ctx.attach_file")}</button>
                                                                <button type="button" class="rp-tile-menu-item"
                                                                    on:click=move |_| {
                                                                        artifact_menu.set(None);
                                                                        reveal_in_files(&reveal_path, file_source, file_cwd, file_query, file_entries, show_right, open_right_tabs, right_tab);
                                                                    }>
                                                                    {move || t(locale.get(), "artifact.reveal_in_files")}</button>
                                                                <button type="button" class="rp-tile-menu-item"
                                                                    on:click=move |_| { artifact_menu.set(None); reveal_in_file_manager(manager_path.clone()); }>
                                                                    {move || t(locale.get(), "ctx.reveal_in_manager")}</button>
                                                            }
                                                        })}
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
                                                        {publication_source.clone().map(|source| view! {
                                                            <button type="button" class="rp-tile-menu-item"
                                                                on:click=move |_| {
                                                                    artifact_menu.set(None);
                                                                    publication_binding_source.set(Some(source.clone()));
                                                                    show_publication_workspace.set(true);
                                                                }>
                                                                {move || t(locale.get(), "publication.use")}
                                                            </button>
                                                        })}
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
                                            data-artifact-path=context_path
                                            data-artifact-location=context_location>
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
                                                {discarded.then(|| view! {
                                                    <span class="rp-badge discarded">{t(loc, "artifact.source_discarded")}</span>
                                                })}
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
                                                <div class="rp-view" data-preview-kind=cur.kind>
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
                            sessions,
                            delegation_enabled,
                            locale,
                            Callback::new(move |_: ()| {
                                open_settings_fn(Some("workflows".into()));
                                refresh_agent_resources(workflow_studio_state, specialists);
                            }),
                        ).into_view(),
                        RightTab::Notebook => {
                            view! {
                                <NotebookView cells=notebook_cells.get() locale=locale.get()
                                    active_session=active_session.read_only()
                                    library_items=session_library_items.read_only()
                                    on_library_changed=refresh_library_items />
                            }.into_view()
                        }
                        RightTab::Highlights => {
                            let session = active_session.get();
                            let excerpts = session_library_items.with(|items| {
                                items
                                    .iter()
                                .filter(|item| {
                                    item.kind == "text"
                                        && session.as_deref()
                                            == Some(item.source_session_id.as_str())
                                })
                                    .cloned()
                                    .collect::<Vec<_>>()
                            });
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
                                <div class="rp-files" class:drop-target=move || files_drag_over.get()>
                                    <label class="fb-source-label">
                                        <span>{t(loc, "files.source")}</span>
                                        <select class="fb-source" aria-label=t(loc, "files.source")
                                            prop:value=move || file_source.get()
                                            on:change=move |event| {
                                                let next = dom_value(&event);
                                                selecting_workspace_entries.set(false);
                                                selected_workspace_paths.set(HashSet::new());
                                                file_sort_menu_open.set(false);
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
                                                            selected_workspace_paths.set(HashSet::new());
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
                                                <button type="button"
                                                    aria-pressed=move || selecting_workspace_entries.get().to_string()
                                                    on:click=move |_| {
                                                        let selecting = !selecting_workspace_entries.get_untracked();
                                                        selecting_workspace_entries.set(selecting);
                                                        selected_workspace_paths.set(HashSet::new());
                                                    }>
                                                    <span>{move || t(locale.get(), if selecting_workspace_entries.get() {
                                                        "settings.cancel"
                                                    } else {
                                                        "files.select_entries"
                                                    })}</span>
                                                </button>
                                                <FileSortControl sort_by=file_sort menu_open=file_sort_menu_open />
                                            </div>
                                            <input class="fb-search" type="text"
                                                placeholder=move || t(locale.get(), "files.search")
                                                prop:value=move || file_query.get()
                                                on:input=move |ev| {
                                                    selected_workspace_paths.set(HashSet::new());
                                                    file_query.set(event_target_value(&ev));
                                                } />
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
                                                                let path_selected = path.clone();
                                                                let path_pressed = path.clone();
                                                                view! {
                                                                    <button class="fb-row dir" data-workspace-path=path.clone()
                                                                        class:selected=move || selected_workspace_paths.get().contains(&path_selected)
                                                                        attr:aria-pressed=move || selecting_workspace_entries.get().then(|| {
                                                                            selected_workspace_paths.get().contains(&path_pressed).to_string()
                                                                        })
                                                                        on:click=move |_| {
                                                                        if selecting_workspace_entries.get_untracked() {
                                                                            toggle_workspace_path(selected_workspace_paths, &path_click);
                                                                            return;
                                                                        }
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
                                                                let path_selected = path.clone();
                                                                let path_pressed = path.clone();
                                                                view! {
                                                                    <button class="fb-row" data-workspace-path=path.clone()
                                                                        class:selected=move || selected_workspace_paths.get().contains(&path_selected)
                                                                        attr:aria-pressed=move || selecting_workspace_entries.get().then(|| {
                                                                            selected_workspace_paths.get().contains(&path_pressed).to_string()
                                                                        })
                                                                        on:click=move |_| {
                                                                        if selecting_workspace_entries.get_untracked() {
                                                                            toggle_workspace_path(selected_workspace_paths, &path_open);
                                                                            return;
                                                                        }
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
                                                        let sort = file_sort.get();
                                                        let mut entries = file_entries.get();
                                                        sort_dir_entries(&mut entries, &sort);
                                                        if entries.is_empty() {
                                                            // Nothing listed also covers "the tab
                                                            // was opened before a project was", so
                                                            // the whole block re-runs the sidebar
                                                            // Files action instead of just saying
                                                            // the folder is empty.
                                                            return view! {
                                                                <button type="button" class="rp-empty rp-empty-clickable"
                                                                    title=t(loc, "right.browse_files")
                                                                    on:click=open_files>
                                                                    <span class="rp-empty-icon"></span>
                                                                    <div class="rp-empty-title">{t(loc, "right.no_file.title")}</div>
                                                                    <p>{t(loc, "right.no_file.body")}</p>
                                                                    <span class="rp-empty-action">{t(loc, "right.browse_files")}</span>
                                                                </button>
                                                            }.into_view();
                                                        }
                                                        entries.into_iter().map(|e| {
                                                            let name = e.name.clone();
                                                            let full = join_path(&file_cwd.get(), &name);
                                                            if e.is_dir {
                                                                let full_click = full.clone();
                                                                let full_selected = full.clone();
                                                                let full_pressed = full.clone();
                                                                view! {
                                                                    <button class="fb-row dir" data-workspace-path=full.clone()
                                                                        class:selected=move || selected_workspace_paths.get().contains(&full_selected)
                                                                        attr:aria-pressed=move || selecting_workspace_entries.get().then(|| {
                                                                            selected_workspace_paths.get().contains(&full_pressed).to_string()
                                                                        })
                                                                        on:click=move |_| {
                                                                        if selecting_workspace_entries.get_untracked() {
                                                                            toggle_workspace_path(selected_workspace_paths, &full_click);
                                                                            return;
                                                                        }
                                                                        file_query.set(String::new());
                                                                        file_cwd.set(full_click.clone());
                                                                        refresh_dir(file_cwd, file_entries);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("folder")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                        {file_row_meta_view(&e, &sort)}
                                                                    </button>
                                                                }.into_view()
                                                            } else {
                                                                let full_open = full.clone();
                                                                let full_selected = full.clone();
                                                                let full_pressed = full.clone();
                                                                view! {
                                                                    <button class="fb-row" data-workspace-path=full.clone()
                                                                        class:selected=move || selected_workspace_paths.get().contains(&full_selected)
                                                                        attr:aria-pressed=move || selecting_workspace_entries.get().then(|| {
                                                                            selected_workspace_paths.get().contains(&full_pressed).to_string()
                                                                        })
                                                                        on:click=move |_| {
                                                                        if selecting_workspace_entries.get_untracked() {
                                                                            toggle_workspace_path(selected_workspace_paths, &full_open);
                                                                            return;
                                                                        }
                                                                        open_workspace_file(full_open.clone(), modal_artifact);
                                                                    }>
                                                                        <span class="fb-icon">{compose_icon("doc")}</span>
                                                                        <span class="fb-name">{name}</span>
                                                                        {file_row_meta_view(&e, &sort)}
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
                                        let source_for_upload = source.clone();
                                        let source_for_refresh = source.clone();
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
                                            <div class="fb-actions">
                                                <button type="button"
                                                    data-testid="files-remote-upload"
                                                    disabled=move || remote_file_uploading.get()
                                                    on:click=move |_| {
                                                        upload_to_remote_context(
                                                            source_for_upload.clone(),
                                                            remote_file_cwd.get_untracked(),
                                                            None,
                                                            remote_file_uploading,
                                                            remote_files_refresh_tick,
                                                        );
                                                    }>
                                                    {compose_icon("upload")}
                                                    <span>{move || t(locale.get(), if remote_file_uploading.get() {
                                                        "files.uploading"
                                                    } else {
                                                        "files.upload"
                                                    })}</span>
                                                </button>
                                                <button type="button" on:click=move |_| {
                                                    refresh_remote_dir(
                                                        source_for_refresh.clone(),
                                                        remote_file_cwd,
                                                        remote_file_entries,
                                                        remote_file_loading,
                                                        remote_file_error,
                                                        file_source,
                                                    );
                                                }>
                                                    {compose_icon("sync")}
                                                    <span>{t(loc, "files.refresh")}</span>
                                                </button>
                                                <FileSortControl sort_by=file_sort menu_open=file_sort_menu_open />
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
                                                    let sort = file_sort.get();
                                                    let mut entries = remote_file_entries.get();
                                                    sort_dir_entries(&mut entries, &sort);
                                                    entries.into_iter().map(|entry| {
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
                                                                    {file_row_meta_view(&entry, &sort)}
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
                                                                    {file_row_meta_view(&entry, &sort)}
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
                                            <div class="hint fb-root">{t(loc, "files.remote_upload_hint")}</div>
                                        }.into_view()
                                    }}}
                                </div>
                            }.into_view()
                        }
                        RightTab::Provenance => {
                            view! {
                                <ProvenancePane items=items rows=provenance_rows />
                            }
                            .into_view()
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
                                                let can_detach = ctx.kind != "local";
                                                let active_context_id = ctx.id.clone();
                                                let pressed_context_id = ctx.id.clone();
                                                let select_context_id = ctx.id.clone();
                                                let runtime_context_id = ctx.id.clone();
                                                let runs_context_id = ctx.id.clone();
                                                let probe_context_id = ctx.id.clone();
                                                let terminal_context_id = ctx.id.clone();
                                                let detach_context_id = ctx.id.clone();
                                                let runtime_config_context = ctx.clone();
                                                let config_context_id = ctx.id.clone();
                                                let storage_context_id = ctx.id.clone();
                                                let storage_context_label = label.clone();
                                                let can_edit_storage = ctx.kind != "local";
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
                                                                    }>{compose_icon("runtime-panel")}</button>
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
                                                                {can_edit_storage.then(|| {
                                                                    let remote_files_context_id = ctx.id.clone();
                                                                    view! {
                                                                    <button type="button" class="context-terminal context-remote-files"
                                                                        title=t(loc, "contexts.remote_files")
                                                                        aria-label=t(loc, "contexts.remote_files")
                                                                        on:click=move |_| {
                                                                            selected_context_id.set(Some(remote_files_context_id.clone()));
                                                                            context_details_modal.set(Some((remote_files_context_id.clone(), ContextModalKind::RemoteFiles)));
                                                                        }>{compose_icon("database")}</button>
                                                                }})}
                                                                {can_edit_storage.then(|| view! {
                                                                    <button type="button" class="context-terminal context-storage-prefs"
                                                                        title=t(loc, "contexts.storage_prefs")
                                                                        aria-label=t(loc, "contexts.storage_prefs")
                                                                        on:click=move |_| {
                                                                            let context_id = storage_context_id.clone();
                                                                            let context_label = storage_context_label.clone();
                                                                            spawn_local(async move {
                                                                                let args = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
                                                                                match invoke_checked("get_context_storage_prefs", args).await {
                                                                                    Ok(value) => {
                                                                                        if let Ok(prefs) = serde_wasm_bindgen::from_value::<ContextStoragePrefsView>(value) {
                                                                                            storage_prefs_form.set(Some(StoragePrefsForm::from_view(prefs, context_label, false)));
                                                                                        }
                                                                                    }
                                                                                    Err(error) => {
                                                                                        let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                                                                                        show_toast(&message);
                                                                                    }
                                                                                }
                                                                            });
                                                                        }>{compose_icon("folder")}</button>
                                                                })}
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
                                                                    }>{compose_icon("sync")}</button>
                                                                <button type="button" class="context-terminal"
                                                                    title=t(loc, "contexts.open_terminal")
                                                                    aria-label=t(loc, "contexts.open_terminal")
                                                                    on:click=move |_| {
                                                                        selected_context_id.set(Some(terminal_context_id.clone()));
                                                                        open_terminal_for_context.call(terminal_context_id.clone());
                                                                    }>{compose_icon("terminal")}</button>
                                                                {can_detach.then(|| view! {
                                                                    <button type="button" class="context-terminal context-detach"
                                                                        title=t(loc, "contexts.detach")
                                                                        aria-label=t(loc, "contexts.detach")
                                                                        on:click=move |_| {
                                                                            toggle_session_compute_resource
                                                                                .call((detach_context_id.clone(), false));
                                                                        }>{compose_icon("minus")}</button>
                                                                })}
                                                            </div>
                                                        </div>
                                                    </div>
                                                }.into_view()
                                            }).collect_view()
                                        }}
                                        <div class="context-attach" data-testid="context-attach">
                                            <div class="control-section-head">
                                                <span>{t(loc, "contexts.attach")}</span>
                                            </div>
                                            <div class="compute-menu-search context-attach-search">
                                                {compose_icon("search")}
                                                <input type="search" inputmode="search" autocomplete="off"
                                                    aria-label=t(loc, "compute.search")
                                                    placeholder=t(loc, "compute.search")
                                                    prop:value=move || hosts_attach_search.get()
                                                    on:input=move |ev| hosts_attach_search.set(event_target_value(&ev)) />
                                            </div>
                                            <div class="context-attach-list">
                                                {move || {
                                                    let query = hosts_attach_search.get().trim().to_lowercase();
                                                    let enabled = session_execution_contexts.get();
                                                    let mut rows = Vec::new();
                                                    for host in ssh_hosts.get() {
                                                        let context_id = format!("ssh:{}", host.alias);
                                                        if enabled.contains(&context_id) {
                                                            continue;
                                                        }
                                                        if !query.is_empty()
                                                            && !host.alias.to_lowercase().contains(&query)
                                                        {
                                                            continue;
                                                        }
                                                        let toggle_id = context_id.clone();
                                                        let name = host.alias.clone();
                                                        rows.push(view! {
                                                            <button type="button"
                                                                class="context-attach-row"
                                                                data-context-id=context_id.clone()
                                                                on:click=move |_| {
                                                                    toggle_session_compute_resource
                                                                        .call((toggle_id.clone(), true));
                                                                }>
                                                                <span class="compute-resource-icon">{compose_icon("server")}</span>
                                                                <span class="compute-resource-name">{name}</span>
                                                                <span class="compute-resource-state">
                                                                    {t(loc, "contexts.attach_action")}
                                                                </span>
                                                            </button>
                                                        }.into_view());
                                                    }
                                                    for ctx in execution_contexts.get()
                                                        .into_iter()
                                                        .filter(|ctx| ctx.kind == "wsl")
                                                    {
                                                        if enabled.contains(&ctx.id) {
                                                            continue;
                                                        }
                                                        let name = if ctx.label.trim().is_empty() {
                                                            ctx.id.clone()
                                                        } else {
                                                            ctx.label.clone()
                                                        };
                                                        if !query.is_empty()
                                                            && !name.to_lowercase().contains(&query)
                                                            && !ctx.id.to_lowercase().contains(&query)
                                                        {
                                                            continue;
                                                        }
                                                        let context_id = ctx.id.clone();
                                                        let toggle_id = context_id.clone();
                                                        rows.push(view! {
                                                            <button type="button"
                                                                class="context-attach-row"
                                                                data-context-id=context_id.clone()
                                                                on:click=move |_| {
                                                                    toggle_session_compute_resource
                                                                        .call((toggle_id.clone(), true));
                                                                }>
                                                                <span class="compute-resource-icon">{compose_icon("terminal")}</span>
                                                                <span class="compute-resource-name">{name}</span>
                                                                <span class="compute-resource-state">
                                                                    {t(loc, "contexts.attach_action")}
                                                                </span>
                                                            </button>
                                                        }.into_view());
                                                    }
                                                    if rows.is_empty() {
                                                        view! {
                                                            <div class="control-empty">{t(loc, "contexts.attach_empty")}</div>
                                                        }.into_view()
                                                    } else {
                                                        rows.collect_view()
                                                    }
                                                }}
                                            </div>
                                        </div>
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
                                    <div class="sidechat-head">
                                        <span class="sidechat-title">{move || t(locale.get(), "sidechat.title")}</span>
                                        <button type="button" class="icon-btn"
                                            title=move || t(locale.get(), "sidechat.close")
                                            aria-label=move || t(locale.get(), "sidechat.close")
                                            on:click=move |_| close_right_tab(
                                                RightTab::SideChat,
                                                show_right,
                                                open_right_tabs,
                                                right_tab,
                                            )>{compose_icon("close")}</button>
                                    </div>
                                    <div class="sidechat-log" id=SIDE_CHAT_SCROLLER_ID>
                                        {move || {
                                            let rows = side_chat_items.get();
                                            if rows.is_empty() && !side_chat_busy.get() {
                                                view! { <div class="sidechat-empty">{move || t(locale.get(), "sidechat.empty")}</div> }.into_view()
                                            } else {
                                                rows.into_iter().map(|item| match item {
                                                    SideChatItem::User(text) => view! {
                                                        <div class="sidechat-row user"><div class="sidechat-bubble">{text}</div></div>
                                                    }.into_view(),
                                                    SideChatItem::Assistant {
                                                        text,
                                                        model,
                                                        evidence,
                                                        snapshot_version,
                                                        no_evidence,
                                                        error,
                                                    } => {
                                                        let answer = if no_evidence {
                                                            t(locale.get(), "sidechat.no_evidence").to_string()
                                                        } else {
                                                            text
                                                        };
                                                        let evidence_count = evidence.len().to_string();
                                                        let snapshot = snapshot_version.to_string();
                                                        view! {
                                                            <div class="sidechat-row assistant">
                                                                {model.filter(|_| !error).map(|m| view! { <div class="sidechat-model-label">{m}</div> })}
                                                                <div class="sidechat-answer" class:error=error inner_html=md_to_html(&answer)></div>
                                                                {(!evidence.is_empty()).then(|| view! {
                                                                    <details class="sidechat-evidence" data-testid="sidechat-evidence">
                                                                        <summary>{tf(
                                                                            locale.get(),
                                                                            "sidechat.evidence_summary",
                                                                            &[("n", &evidence_count), ("version", &snapshot)],
                                                                        )}</summary>
                                                                        <div class="sidechat-evidence-list">
                                                                            {evidence.into_iter().enumerate().map(|(index, source)| {
                                                                                let source_number = index + 1;
                                                                                let turn = source.turn.to_string();
                                                                                let locator = source.event_seq
                                                                                    .map(|seq| format!("event {seq}"))
                                                                                    .or_else(|| source.message_seq.map(|seq| format!("message {seq}")))
                                                                                    .unwrap_or_else(|| source.source_id.clone());
                                                                                view! {
                                                                                    <article class="sidechat-evidence-item"
                                                                                        data-source-id=source.source_id>
                                                                                        <div class="sidechat-evidence-meta">
                                                                                            {format!("[S{source_number}] · {} · {} · {locator}",
                                                                                                tf(locale.get(), "sidechat.turn", &[("n", &turn)]),
                                                                                                source.role,
                                                                                            )}
                                                                                        </div>
                                                                                        <blockquote>{source.excerpt}</blockquote>
                                                                                        {(!source.relevance.is_empty()).then(|| view! {
                                                                                            <div class="sidechat-evidence-why">{source.relevance}</div>
                                                                                        })}
                                                                                    </article>
                                                                                }
                                                                            }).collect_view()}
                                                                        </div>
                                                                    </details>
                                                                })}
                                                            </div>
                                                        }.into_view()
                                                    }
                                                }).collect_view()
                                            }
                                        }}
                                        {move || side_chat_busy.get().then(|| view! {
                                            <div class="sidechat-thinking">{move || t(locale.get(), "sidechat.thinking")}</div>
                                        })}
                                    </div>
                                    <div class="sidechat-composer">
                                        {move || (!side_chat_quotes.get().is_empty()).then(|| view! {
                                            <div class="composer-attachments composer-reference-chips sidechat-quotes">
                                                {side_chat_quotes.get().into_iter().enumerate().map(|(idx, quote)| {
                                                    let label = quote_label(&quote.text);
                                                    let title = quote.source.as_ref().map_or_else(
                                                        || quote.text.clone(),
                                                        |source| format!("{source}\n\n{}", quote.text),
                                                    );
                                                    let source = quote.source.clone();
                                                    view! {
                                                        <div class="composer-attachment-row composer-reference-card quote"
                                                            data-testid="sidechat-quote" title=title>
                                                            <span class="composer-attachment-icon">{compose_icon("chat")}</span>
                                                            <span class="composer-attachment-copy">
                                                                <span class="composer-attachment ready">{label}</span>
                                                                <span class="composer-attachment-meta">{move || source.clone().unwrap_or_else(|| t(locale.get(), "attachment.quote").into())}</span>
                                                            </span>
                                                            <button type="button" class="composer-attachment-remove"
                                                                title=move || t(locale.get(), "composer.remove_attachment")
                                                                aria-label=move || t(locale.get(), "composer.remove_attachment")
                                                                on:click=move |_| side_chat_quotes.update(|items| {
                                                                    if idx < items.len() {
                                                                        items.remove(idx);
                                                                    }
                                                                })>{compose_icon("close")}</button>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        })}
                                        <textarea
                                            id=SIDE_CHAT_INPUT_ID
                                            prop:value=move || side_chat_input.get()
                                            prop:placeholder=move || t(locale.get(), "sidechat.placeholder")
                                            on:input=move |ev| side_chat_input.set(event_target_value(&ev))
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ime_composing(&ev) { return; }
                                                if ev.key() == "Enter" && !ev.shift_key() {
                                                    ev.prevent_default();
                                                    send_side_chat((
                                                        side_chat_input.get(),
                                                        side_chat_quotes.get(),
                                                        true,
                                                    ));
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
                                                                l.iter()
                                                                    .find(|m| m.active && m.is_chat_model())
                                                                    .or_else(|| l.iter().find(|m| m.is_chat_model()))
                                                                    .map(|m| m.label.clone())
                                                                    .unwrap_or_default()
                                                            }
                                                        }}
                                                        <span>"▾"</span>
                                                    </button>
                                                    {move || side_chat_model_menu_open.get().then(|| view! {
                                                        <div class="sidechat-model-backdrop" on:click=move |_| side_chat_model_menu_open.set(false)></div>
                                                        <div class="sidechat-model-menu">
                                                            {move || models.get().into_iter().filter(ModelProfile::is_chat_model).map(|m| {
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
                                                disabled=move || side_chat_busy.get()
                                                    || (side_chat_input.get().trim().is_empty()
                                                        && side_chat_quotes.get().is_empty())
                                                on:click=move |_| send_side_chat((
                                                    side_chat_input.get(),
                                                    side_chat_quotes.get(),
                                                    true,
                                                ))>
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
                class:terminal-dock-hidden=move || !terminal_panel_open.get() || demo_mode.get()
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
                                let close_session_id = session.id.clone();
                                let tab_id = terminal_tab_id(&session.id);
                                let panel_id = terminal_element_id(&session.id);
                                view! {
                                    <div class="terminal-dock-tab"
                                        class:active=move || active_terminal_id.get().as_deref() == Some(tab_active_id.as_str())
                                    >
                                        <button id=tab_id type="button" role="tab" class="terminal-dock-tab-main"
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
                                        <button type="button" class="terminal-dock-tab-close"
                                            title=move || t(locale.get(), "terminal.close_session")
                                            aria-label=move || t(locale.get(), "terminal.close_session")
                                            on:click=move |_| close_terminal_session.call(close_session_id.clone())>
                                            {compose_icon("close")}
                                        </button>
                                    </div>
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
                    <button type="button" class="terminal-dock-action icon"
                        title=move || t(locale.get(), "terminal.collapse")
                        aria-label=move || t(locale.get(), "terminal.collapse")
                        on:click=move |_| {
                            terminal_add_menu_open.set(false);
                            terminal_panel_open.set(false);
                        }>{compose_icon("minus")}</button>
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

        {move || {
            let Some(snapshot) = active_context_usage.get() else {
                return None;
            };
            (context_usage_open.get()
                && context_usage_mode.get() == ContextUsageMode::Floating)
                .then(|| {
                    view! {
                        <ContextUsagePanel
                            snapshot=snapshot
                            floating=true
                            locale=locale.read_only()
                            context_usage_open=context_usage_open
                            context_usage_details=context_usage_details
                            context_usage_detail_open=context_usage_detail_open
                            context_usage_geom=context_usage_geom
                            on_header_down=on_context_usage_header_down
                            on_header_dblclick=on_context_usage_header_dblclick
                            on_dock=on_context_usage_dock
                            on_resize_start=on_context_usage_resize_start
                            on_compact=compact_from_usage
                            on_new_session=new_session_from_usage
                            compact_disabled=Signal::derive(move || busy.get())
                        />
                    }
                })
        }}

        {move || dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_resize_move
                on:mouseup=move |_| dragging.set(false)></div>
        })}

        {move || center_split_dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_center_split_resize_move
                on:mouseup=move |_| center_split_dragging.set(false)></div>
        })}

        {move || center_runtime_col_dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=move |ev: web_sys::MouseEvent| {
                    let Some(rect) = runtime_workbench_rect() else { return; };
                    if rect.width() > 0.0 {
                        center_runtime_right_w.set(
                            ((rect.right() - ev.client_x() as f64) / rect.width() * 100.0)
                                .clamp(16.0, 70.0),
                        );
                    }
                }
                on:mouseup=move |_| center_runtime_col_dragging.set(false)></div>
        })}

        {move || center_runtime_row_dragging.get().then(|| view! {
            <div class="drag-overlay drag-overlay-row"
                on:mousemove=move |ev: web_sys::MouseEvent| {
                    let Some(rect) = runtime_workbench_rect() else { return; };
                    if rect.height() > 0.0 {
                        center_runtime_bottom_h.set(
                            ((rect.bottom() - ev.client_y() as f64) / rect.height() * 100.0)
                                .clamp(12.0, 70.0),
                        );
                    }
                }
                on:mouseup=move |_| center_runtime_row_dragging.set(false)></div>
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

        {move || context_usage_dragging.get().then(|| view! {
            <div class="drag-overlay context-usage-move" aria-hidden="true"></div>
        })}

        {move || context_usage_resizing.get().then(|| view! {
            <div class="drag-overlay context-usage-resize-overlay"
                on:mousemove=on_context_usage_resize_move
                on:mouseup=on_context_usage_resize_end></div>
        })}

        <BranchMergeOverlay
            state=BranchMergeOverlayState {
                locale,
                open: branch_merge_open,
                preview: branch_merge_preview,
                draft: branch_merge_draft,
                busy: branch_merge_busy,
                error: branch_merge_error,
                guidance_open: branch_merge_guidance_open,
                guidance: branch_merge_guidance,
            }
            on_merge=merge_branch_summary
            on_generate=generate_branch_summary
        />
        <BranchMergeDetailOverlay locale=locale detail=branch_merge_detail />

        <ExplorationOverlayView
            state=ExplorationOverlayState {
                locale,
                overlay: exploration_overlay,
                name: exploration_name,
                preview: exploration_preview,
                busy: exploration_busy,
                error: exploration_error,
            }
            on_start=create_exploration_from_overlay
            on_promote=promote_exploration_from_overlay
            on_discard=discard_exploration_from_overlay
            on_open_manual_resolution=open_exploration_manual_resolution
            on_finish_manual_resolution=finish_exploration_manual_resolution
        />

        <SessionTransferOverlay
            state=SessionTransferOverlayState {
                locale, session_transfer, session_transfer_busy, session_transfer_error,
                project_info, proj_list,
            }
            on_save=Callback::new(save_session_transfer)
        />

        <RenameSessionOverlay
            state=RenameSessionOverlayState { locale, rename_session_target, rename_session_input }
            on_renamed=Callback::new(move |(id, title): (String, String)| {
                // Patch the cached row before the async list refresh so the
                // sidebar does not flash the old title. After #888 the backend
                // list already includes a named draft; this is only the
                // immediate paint.
                sessions.update(|rows| {
                    if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
                        row.title = title;
                    }
                });
                refresh_session_history();
            })
        />

        <FolderModalOverlay
            state=FolderModalOverlayState { locale, folder_modal, folder_modal_input }
            on_save=Callback::new(save_folder_modal)
        />

        <FileEntryOverlay
            state=FileEntryOverlayState {
                locale, file_entry_modal, file_entry_input, file_entry_busy,
                file_entry_error, file_cwd,
            }
            on_save=save_file_entry_modal
        />

        <TurnUndoOverlay
            state=TurnUndoOverlayState {
                locale, turn_undo_dialog, turn_undo_busy, turn_undo_error,
            }
            on_confirm=confirm_turn_undo
        />

        <EditConfirmOverlay
            state=EditConfirmOverlayState {
                locale,
                edit_confirm,
                can_branch: Signal::derive(move || {
                    active_branch_state.get().is_none() && !active_is_exploration.get()
                }),
            }
            on_branch=Callback::new(branch_message)
            on_rewind=Callback::new(rewind_to_user_item)
        />

        {move || ui_confirm.get().map(|action| {
            let action_ok = action.clone();
            let is_full_permission = matches!(&action, UiConfirm::EnableFullPermission);
            let title_key = if is_full_permission {
                "full_permission.confirm_title"
            } else {
                "confirm.title"
            };
            let message = match &action {
                UiConfirm::EnableFullPermission => t(locale.get(), "full_permission.confirm_body").to_string(),
                UiConfirm::DeleteFolder(_) => t(locale.get(), "folder.delete_confirm").to_string(),
                UiConfirm::DeleteSessions(ids) if ids.len() == 1 => t(locale.get(), "session.delete_confirm").to_string(),
                UiConfirm::DeleteSessions(ids) => tf(
                    locale.get(),
                    "session.delete_many_confirm",
                    &[("n", &ids.len().to_string())],
                ),
                UiConfirm::AbandonExploration(_) => t(locale.get(), "exploration.abandon_confirm_body").to_string(),
                UiConfirm::DeleteFileEntry { path, is_dir } => tf(
                    locale.get(),
                    if *is_dir { "files.delete_directory_confirm" } else { "files.delete_file_confirm" },
                    &[("path", path)],
                ),
                UiConfirm::ReloadProjectRules(_) => t(locale.get(), "session.reload_rules_hint").to_string(),
                UiConfirm::SaveAgentContext => t(locale.get(), "proj_settings.agent_context_confirm").to_string(),
            };
            let action_key = match &action {
                UiConfirm::EnableFullPermission => "full_permission.confirm_action",
                UiConfirm::DeleteFolder(_) => "ctx.delete_folder",
                UiConfirm::DeleteSessions(_) => "ctx.delete_session",
                UiConfirm::AbandonExploration(_) => "exploration.abandon",
                UiConfirm::DeleteFileEntry { is_dir: true, .. } => "files.delete_directory",
                UiConfirm::DeleteFileEntry { is_dir: false, .. } => "files.delete_file",
                UiConfirm::ReloadProjectRules(_) => "session.reload_rules_action",
                UiConfirm::SaveAgentContext => "proj_settings.agent_context_confirm_action",
            };
            view! {
            <div class="overlay">
                <div class="modal confirm-modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <div class="hint">{message}</div>
                    <div class="row">
                        <button on:click=move |_| ui_confirm.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" class:danger=is_full_permission on:click=move |_| {
                            ui_confirm.set(None);
                            match action_ok.clone() {
                                UiConfirm::EnableFullPermission => {
                                    full_permission_busy.set(true);
                                    let loc = locale.get_untracked();
                                    spawn_local(async move {
                                        let (session_id, created_session) = match active_session.get_untracked() {
                                            Some(session_id) => (session_id, false),
                                            None => {
                                                let Some(session_id) = invoke("new_session", JsValue::UNDEFINED).await.as_string() else {
                                                    full_permission_busy.set(false);
                                                    return;
                                                };
                                                (session_id, true)
                                            }
                                        };
                                        let args = to_value(&serde_json::json!({
                                            "sessionId": session_id.clone(),
                                            "enabled": true,
                                        })).unwrap();
                                        let enabled = invoke_checked("set_session_full_permission", args)
                                            .await
                                            .ok()
                                            .and_then(|value| value.as_bool())
                                            .unwrap_or(false);
                                        if created_session && enabled {
                                            active_session.set(Some(session_id.clone()));
                                            items.set(vec![]);
                                            refresh_session_history();
                                        }
                                        if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
                                            full_permission_enabled.set(enabled);
                                        }
                                        full_permission_busy.set(false);
                                        if enabled {
                                            show_toast(&t(loc, "full_permission.enabled"));
                                        }
                                    });
                                }
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
                                UiConfirm::DeleteSessions(ids) => {
                                    let active_session = active_session;
                                    let items = items;
                                    let transcripts = transcripts;
                                    let running = running;
                                    let pending_turns = pending_turns;
                                    spawn_local(async move {
                                        let mut deleted = HashSet::new();
                                        for id in ids {
                                            let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                            if invoke_checked("delete_session", arg).await.is_ok() {
                                                deleted.insert(id);
                                            }
                                        }
                                        if !deleted.is_empty() {
                                            transcripts.update(|stored| {
                                                stored.retain(|id, _| !deleted.contains(id));
                                            });
                                            running.update(|stored| {
                                                stored.retain(|id| !deleted.contains(id));
                                            });
                                            pending_turns.update(|stored| {
                                                stored.retain(|id, _| !deleted.contains(id));
                                            });
                                            if active_session.get().is_some_and(|id| deleted.contains(&id)) {
                                                active_session.set(None);
                                                items.set(vec![]);
                                            }
                                            refresh_session_history();
                                        }
                                    });
                                }
                                UiConfirm::AbandonExploration(source_frame_id) => {
                                    let load_session = load_session.clone();
                                    spawn_local(async move {
                                        let args = to_value(&serde_json::json!({
                                            "sourceFrameId": source_frame_id.clone(),
                                        })).unwrap();
                                        match invoke_checked("abandon_exploration_round", args).await {
                                            Ok(_) => {
                                                exploration_overlay.set(None);
                                                exploration_preview.set(None);
                                                refresh_explorations(explorations);
                                                refresh_session_history();
                                                load_session.call(source_frame_id);
                                            }
                                            Err(error) => show_toast(&localize_backend(
                                                locale.get_untracked(),
                                                &js_error_text(error),
                                            )),
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
                                UiConfirm::ReloadProjectRules(id) => {
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "frameId": id })).unwrap();
                                        match invoke_checked("reload_project_rules", arg).await {
                                            Ok(_) => refresh_session_history(),
                                            Err(error) => show_toast(&localize_backend(
                                                locale.get_untracked(),
                                                &js_error_text(error),
                                            )),
                                        }
                                    });
                                }
                                UiConfirm::SaveAgentContext => {
                                    commit_proj_settings();
                                }
                            }
                        }>{move || t(locale.get(), action_key)}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        <ModelSwitchConfirmOverlay
            state=ModelSwitchConfirmOverlayState { locale, model_switch_confirm }
            on_switch=switch_http_model
        />

        <ProjSettingsOverlay
            state=ProjSettingsOverlayState {
                locale, show_proj_settings, proj_settings, proj_settings_busy,
            }
            on_save=Callback::new(save_proj_settings)
        />

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
                    on_rerun=Callback::new(move |text: String| {
                        input.set(text);
                        focus_composer();
                        modal_artifact.set(None);
                    })
                    library_items=library_items.read_only()
                    on_library_changed=refresh_library_items />
            }
        })}
        <SettingsView
            state=SettingsViewState {
                locale, theme_mode, light_palette, dark_palette, ui_font_size, code_font_size, ui_font_family, code_font_family, selection_popup_enabled, send_with_modifier, custom_css, update_check_enabled, show_settings, settings_section, open_conn_key, channels_open, connectors, model_form, model_catalog_limits,
                conn_form, memory_selected, specialist_form, settings, bootstrap, settings_message,
                settings_busy, model_form_open, model_form_key, models, model_form_msg, show_acp_agents,
                acp_agents, active_acp_agent_id, acp_form, acp_form_msg, acp_infos, specialists,
                quick_actions, workflow_templates, workflow_studio: workflow_studio_state,
                selected_workflow_template,
                specialist_form_open, memory_view, memory_editor, memory_msg, skills_list,
                skill_filter_tag, skills_search, skills_msg, plugins_list, plugins_msg, plugin_install_open, cred_status, cred_inputs,
                custom_credentials, cred_msg, approval_grants, conns_view, conn_form_open,
                conn_form_kind, conn_test_msg, custom_conn_tools, custom_conn_tools_loading,
                custom_conn_tool_errors, pet_status, ssh_hosts, execution_contexts,
                default_execution_context, runtime_interpreter_form, probing_context_id,
                delete_confirm,
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
            reload_skills=reload_skills
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
            set_default_compute_resource=set_default_compute_resource
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
            open_terminal_session=activate_terminal_session
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
            runs=run_records research_graph=research_graph modal_artifact=modal_artifact
            active_project=project_info projects=proj_list
            runtime_interpreter_form=runtime_interpreter_form object_states=runtime_object_states
            locale=locale selection_popup=selection_popup
            on_use_in_publication=Callback::new(move |source| {
                publication_binding_source.set(Some(source));
                show_publication_workspace.set(true);
            })
        />
        {move || runtime_environment_pinned.get().then(|| view! {
            <RuntimeEnvironmentPanel selected=runtime_environment pinned=runtime_environment_pinned
                position=runtime_environment_position context_modal=context_details_modal
                locale=locale states=runtime_object_states runtimes=runtime_infos
                contexts=execution_contexts active_project=project_info projects=proj_list
                selection_popup=selection_popup />
        })}
        <RuntimeInterpreterOverlay
            locale=locale form=runtime_interpreter_form execution_contexts=execution_contexts
            runtimes=runtime_infos
        />
        <StoragePrefsOverlay locale=locale form=storage_prefs_form />
        <RunReviewOverlay locale=locale modal=run_review_modal runs=run_records />
        <ShareOverlay
            locale=locale
            draft=share_draft
        />
        <TrajectoryOverlay
            open=trajectory_open
            snapshot=trajectory_snapshot
            live=trajectory_live
            busy=busy
            session_id=active_session
        />
        <CapabilitiesOverlay
            locale=locale show_capabilities=show_capabilities
            bootstrap=bootstrap caps=caps busy=busy open_settings_section=open_capability_settings
            start_env_setup=Callback::new(start_env_setup)
        />
        <OnboardingOverlay
            locale=locale show_onboarding=show_onboarding onboard_step=onboard_step
            onboard_key=onboard_key
            save_onboard_key=save_onboard_key
            dismiss_onboard=Callback::new(dismiss_onboard)
        />
        <ContextRecoveryOverlay
            state=ContextRecoveryOverlayState {
                locale, context_recovery_dialog, context_recovery_busy, context_recovery_error,
            }
            on_compact=compact_context_recovery
            on_new_session=new_session_context_recovery
        />
        <ContextMenuPortal menu=ctx_menu.read_only() set_menu=ctx_menu.write_only() on_pick=on_ctx_pick />
        </div>
    }
}

/// `console_error_panic_hook` plus one deliberate downgrade: leptos 0.6
/// runs a `create_effect`'s first pass in a microtask bound to its owner, and
/// an owner disposed in between makes `with_owner` panic with
/// `OwnerDisposed`. Keyed rows (streaming turns, artifact-card rebuilds) hit
/// that race routinely; under release `panic = "abort"` it used to take the
/// whole renderer down — a dead window with the backend still running. A
/// disposed-owner effect has nothing left to update, so the correct handling
/// is to drop it with a console warning instead of aborting.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let message = format!("{info}");
        if message.contains("OwnerDisposed") {
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
                "dropped reactive effect for a disposed owner: {message}"
            )));
            return;
        }
        // Everything else keeps the standard hook behavior (error + stack).
        console_error_panic_hook::hook(info);
    }));
}

pub fn main() {
    install_panic_hook();
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
