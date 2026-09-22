//! The agent turn pipeline: `send_message` and its inner driver, the parked
//! turn queue (#433), and `stop_agent`. One user-visible turn = resolve
//! references → build/reuse the session agent → run the loop with a
//! `TauriOutput` sink → persist UI events → drive queued follow-ups.
//!
//! Split out of `lib.rs` so turn-flow changes stop churning the command
//! registration hub. Shared helpers (settings/skills/MCP loaders, event
//! plumbing) still live in `lib.rs` and are reached through `super`.

use super::*;

/// Where a user-visible turn originated. IM turns share the desktop approval
/// UI but must not inherit an unattended Allow default for mutating tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TurnOrigin {
    #[default]
    Desktop,
    Im,
    Queued(u64),
}

impl TurnOrigin {
    fn force_ask_mutations(self) -> bool {
        matches!(self, Self::Im)
    }

    fn queue_id(self) -> Option<u64> {
        match self {
            Self::Queued(id) => Some(id),
            Self::Desktop | Self::Im => None,
        }
    }
}

/// Persist the agent's compacted context as a new context epoch and return
/// its number. The previous head rows are left frozen; `context_epochs`
/// records what the compaction did (`ContextManager::last_compaction`).
///
/// `aligned` says whether the list the compaction started from was
/// row-aligned with the head epoch (true for `/compact` and for a single
/// mid-turn compaction). Only then can `kept_from_index` be mapped to the
/// durable seq of the first retained-tail message; otherwise the UI-only
/// `first_kept_seq` is left unknown.
pub(crate) async fn persist_compaction_epoch(
    store: &Store,
    frame_id: &str,
    ctx: &wisp_core::ContextManager,
    strategy: &str,
    aligned: bool,
) -> Result<i64, String> {
    let outcome = ctx.last_compaction();
    let first_kept_seq = match outcome.and_then(|outcome| outcome.kept_from_index) {
        Some(index) if aligned => store
            .load_messages_with_seq(frame_id)
            .await
            .map_err(|error| error.to_string())?
            .get(index)
            .map(|(seq, _)| *seq),
        _ => None,
    };
    let epoch = store
        .open_context_epoch(
            frame_id,
            wisp_store::OpenContextEpoch {
                messages: &ctx.messages,
                strategy,
                kind: outcome.map_or("prune_only", |outcome| outcome.kind.as_str()),
                before_tokens: outcome.map_or(0, |outcome| outcome.before),
                after_tokens: outcome.map_or(0, |outcome| outcome.after),
                checkpoint_index: outcome.and_then(|outcome| outcome.checkpoint_index),
                first_kept_seq,
                archive_ref: outcome.map(|outcome| outcome.archive_reference.as_str()),
                ui_event_seq: None,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    // Mid-turn compactions emit their Compaction event before the epoch
    // exists; attach the newest one now. `/compact` appends its own event
    // afterwards and links it itself.
    if strategy != "manual" {
        if let Ok(Some(seq)) = store.latest_compaction_ui_event_seq(frame_id).await {
            if let Err(error) = store.set_context_epoch_ui_event(frame_id, epoch, seq).await {
                tracing::warn!("link compaction event to epoch failed: {error}");
            }
        }
    }
    Ok(epoch)
}

#[tauri::command]
pub(crate) async fn send_message(
    state: State<'_, AppState>,
    app: AppHandle,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: Option<String>,
    message: String,
    attachments: Option<Vec<String>>,
    references: Option<Vec<ComposerReferenceArg>>,
    resume: Option<bool>,
    acp_agent_id: Option<String>,
    progress_observer_id: Option<u64>,
    guide: Option<bool>,
    replace: Option<bool>,
) -> Result<String, String> {
    let mut replacement_guard = None;
    let mut workflow_guard = None;
    if replace.unwrap_or(false) {
        if let Some(session_id) = session_id.as_deref().filter(|id| !id.is_empty()) {
            let runtime = state.sessions.lock().await.get(session_id).cloned();
            if let Some(rt) = runtime {
                replacement_guard = Some(ReplacementReservation::new(rt.clone()));
                stop_agent(state.clone(), Some(session_id.to_string())).await?;
                workflow_guard = Some(rt.workflow.clone().lock_owned().await);
                for id in supersede_duplicate_queued(
                    &rt,
                    &message,
                    attachments.as_deref().unwrap_or_default(),
                    references.as_deref().unwrap_or_default(),
                ) {
                    emit_queued_turn_state(&app, session_id, id, "superseded");
                }
            }
        }
    }
    let result = send_message_inner(
        state.inner(),
        app,
        window.label(),
        session_id,
        message,
        attachments,
        references,
        resume,
        acp_agent_id,
        progress_observer_id,
        guide,
        replace,
        workflow_guard,
        TurnOrigin::Desktop,
    )
    .await;
    drop(replacement_guard);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManualCompactCommand {
    intent: wisp_core::CompactIntent,
    instruction: Option<String>,
}

fn parse_manual_compact_command(message: &str) -> Option<ManualCompactCommand> {
    let command = message.trim();
    let Some(rest) = command.strip_prefix("/compact") else {
        return None;
    };
    if rest.is_empty() {
        return Some(ManualCompactCommand {
            intent: wisp_core::CompactIntent::PruneOnly,
            instruction: None,
        });
    }
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(ManualCompactCommand {
            intent: wisp_core::CompactIntent::PruneOnly,
            instruction: None,
        });
    }
    if rest == "--semantic" {
        return Some(ManualCompactCommand {
            intent: wisp_core::CompactIntent::Semantic,
            instruction: None,
        });
    }
    if let Some(instruction) = rest.strip_prefix("--semantic") {
        if instruction.chars().next().is_some_and(char::is_whitespace) {
            let instruction = instruction.trim();
            return Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::Semantic,
                instruction: (!instruction.is_empty()).then(|| instruction.to_string()),
            });
        }
        return None;
    }
    Some(ManualCompactCommand {
        intent: wisp_core::CompactIntent::Semantic,
        instruction: Some(rest.to_string()),
    })
}

struct ReplacementReservation(Arc<SessionRuntime>);

impl ReplacementReservation {
    fn new(rt: Arc<SessionRuntime>) -> Self {
        rt.replacing.fetch_add(1, Ordering::SeqCst);
        Self(rt)
    }
}

impl Drop for ReplacementReservation {
    fn drop(&mut self) {
        self.0.replacing.fetch_sub(1, Ordering::SeqCst);
    }
}

// Called while owning the workflow, after the cancelled loop has persisted.
fn supersede_duplicate_queued(
    rt: &SessionRuntime,
    replacement: &str,
    attachments: &[String],
    references: &[ComposerReferenceArg],
) -> Vec<u64> {
    let matches = |item: &QueuedItem| {
        item.message == replacement
            && item.attachments == attachments
            && item.references == references
    };
    let mut queued = rt.queued.lock().unwrap();
    let mut ids = Vec::new();
    queued.retain(|item| {
        let duplicate = matches(item);
        if duplicate {
            ids.push(item.id);
        }
        !duplicate
    });
    let mut cutins = rt.queued_cutins.lock().unwrap();
    let mut removed_guidance = Vec::new();
    cutins.retain(|(guidance_id, item)| {
        let duplicate = matches(item);
        if duplicate {
            ids.push(item.id);
            removed_guidance.push(*guidance_id);
        }
        !duplicate
    });
    if !removed_guidance.is_empty() {
        let mut pending = rt.pending_guidance.lock().unwrap();
        pending.retain(|(guidance_id, _)| !removed_guidance.contains(guidance_id));
    }
    ids
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_message_inner(
    state: &AppState,
    app: AppHandle,
    window_label: &str,
    session_id: Option<String>,
    message: String,
    attachments: Option<Vec<String>>,
    references: Option<Vec<ComposerReferenceArg>>,
    resume: Option<bool>,
    acp_agent_id: Option<String>,
    progress_observer_id: Option<u64>,
    // Guide (#410): while a turn runs, park the message for the loop to inject
    // at its next iteration instead of only queueing a whole new turn.
    guide: Option<bool>,
    // Guide (#410): roll the model context back to where the interrupted turn
    // started before running this message ("replace the current task").
    replace: Option<bool>,
    mut workflow_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    origin: TurnOrigin,
) -> Result<String, String> {
    let resume = resume.unwrap_or(false);
    // Automatic delegation resume carries an owned workflow guard and uses the
    // synthetic "main" route. It must preserve the window that launched the
    // parent task; every direct/queued user turn may claim its actual window.
    let user_routed_turn = !resume || workflow_guard.is_none();
    if !resume && message.trim().is_empty() {
        return Err("message is empty".into());
    }
    let mut ap = state.require_active(window_label)?;
    let mut explicit_scope = None;
    // A session belongs to one project for life, but the per-window active slot
    // can drift while it keeps running (another project opened in this window,
    // the "main" fallback, an agent rebuild). For explicit session ids, always
    // run the turn in the owner project — never error out on a mismatch or,
    // worse, run tools in a stranger's workspace (#182, #194).
    if let Some(id) = session_id.as_deref().filter(|id| !id.is_empty()) {
        state
            .store
            .require_unarchived_session(id)
            .await
            .map_err(|e| e.to_string())?;
        if matches!(
            state
                .store
                .session_branch_state(id)
                .await
                .map_err(|error| error.to_string())?,
            Some("merged" | "orphaned")
        ) {
            return Err(
                "This conversation branch is frozen and cannot accept new messages.".into(),
            );
        }
        let (working_project, scope) =
            exploration_commands::working_project_for_frame(state, id).await?;
        ap = working_project;
        explicit_scope = Some(scope);
    }
    let _project_activity = state.begin_project_activity(&ap.id)?;
    if let Some(id) = session_id.as_deref().filter(|id| !id.is_empty()) {
        state
            .store
            .require_unarchived_session(id)
            .await
            .map_err(|e| e.to_string())?;
    }
    ensure_project_live_approvals(state, &ap.id).await;
    let frame_scope = explicit_scope
        .clone()
        .unwrap_or_else(|| wisp_store::StateScope::mainline(ap.id.clone()));
    let exploration_isolation =
        exploration_isolation::boundary_for_scope(&state.store, &frame_scope).await?;
    let project_write_locked = exploration_commands::conversation_project_write_locked(
        &state.store,
        &frame_scope,
        session_id.as_deref().filter(|id| !id.is_empty()),
    )
    .await?;
    let saved_binding = match session_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => state
            .store
            .get_acp_session(id)
            .await
            .map_err(|error| error.to_string())?,
        None => None,
    };
    if acp_agent_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
        || saved_binding.is_some()
    {
        if project_write_locked {
            return Err(
                "exploration_mainline_frozen: ACP conversations cannot enforce the exploration read-only project lock; use the built-in Agent or finish the exploration round first."
                    .into(),
            );
        }
        if matches!(
            explicit_scope.as_ref(),
            Some(wisp_store::StateScope::Exploration { .. })
        ) {
            return Err(
                "exploration_acp_unsupported: ACP conversations cannot run inside an exploration in the MVP."
                    .into(),
            );
        }
        // ACP agents own their conversation context, so neither mid-turn
        // guidance injection nor context rollback is possible over the
        // protocol; `guide`/`replace` degrade to the plain queued turn here.
        let _ = (guide, replace);
        let frame_id = match session_id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) => {
                let owner = state
                    .store
                    .frame_project_id(id)
                    .await
                    .map_err(|error| error.to_string())?;
                if owner.as_deref() != Some(ap.id.as_str()) {
                    return Err("Session does not belong to the active project.".into());
                }
                id.to_string()
            }
            None => create_session_frame(&state.store, &ap.id).await?,
        };
        if user_routed_turn {
            state.set_notification_window(&frame_id, window_label);
        }
        // Register cancellation before Reader fan-out so Stop can interrupt the
        // retrieval phase as well as the ACP turn that follows it.
        let runtime = {
            let mut sessions = state.sessions.lock().await;
            sessions
                .entry(frame_id.clone())
                .or_insert_with(|| Arc::new(SessionRuntime::new()))
                .clone()
        };
        let _workflow = match workflow_guard.take() {
            Some(guard) => guard,
            None => runtime.workflow.clone().lock_owned().await,
        };
        runtime.cancel.store(false, Ordering::SeqCst);
        let refs = references.as_deref().unwrap_or_default();
        let skills = active_skill_index(&state.store, &ap).await;
        let mut injected_context =
            resolve_composer_references(&state.store, refs, &frame_id, &ap.root, &skills).await?;
        let package_guidance = network::package_guidance(&network::load(&state.store).await?);
        if !package_guidance.is_empty() {
            injected_context.push(package_guidance);
        }
        if let Some(memory) = memory_commands::global_memory_runtime_injection(&state.store).await {
            injected_context.push(memory);
        }
        let archive_index = state
            .store
            .research_archive_index(&ap.id)
            .await
            .map_err(|e| e.to_string())?;
        if !archive_index.is_empty() {
            injected_context.push(archive_index);
        }
        if let Some(context) = runtime.mcp_app_context_injection() {
            injected_context.push(context);
        }
        if let Some(injection) =
            resolve_reader_references(&state.store, refs, &frame_id, &message, &runtime.cancel)
                .await?
        {
            injected_context.push(injection);
        }
        enable_referenced_contexts(&state.store, refs, &frame_id).await;
        if let Some(compute) = ssh_hosts::stored_compute_section(&state.store, &frame_id).await {
            injected_context.push(compute);
        }
        let completion_deliveries = if resume {
            Vec::new()
        } else {
            state
                .store
                .list_unpresented_agent_workflow_deliveries(&frame_id)
                .await
                .map_err(|error| error.to_string())?
        };
        if !completion_deliveries.is_empty() {
            injected_context.push(delegation_completion::completion_prompt(
                &completion_deliveries,
            ));
        }
        let completion_delivery_ids = completion_deliveries
            .iter()
            .map(|delivery| delivery.id.clone())
            .collect::<Vec<_>>();
        let artifact_references = resolve_acp_artifact_references(&state.store, refs).await?;
        // Record this project's last session. Desktop sends never move the IM
        // target project; Feishu/WeChat keep their own `/project` destination.
        channels::record_last_message_session(&state.store, &frame_id)
            .await
            .map_err(|error| format!("Failed to update the shared last-message route: {error}"))?;
        let _progress_subscription =
            progress_observer_id.and_then(|id| channels::activate_progress_observer(id, &frame_id));
        let turn_start = state
            .store
            .load_messages(&frame_id)
            .await
            .map_err(|error| error.to_string())?
            .len();
        state
            .device_hub
            .mark_working(&frame_id, Some(ap.id.as_str()));
        state.running_turns.lock().await.insert(frame_id.clone());
        let result = if resume {
            acp::run_acp_internal_turn(state, &app, &ap, &frame_id, &message).await
        } else {
            acp::run_acp_turn(
                state,
                &app,
                Some(window_label),
                &ap,
                &frame_id,
                acp_agent_id.as_deref().filter(|id| !id.trim().is_empty()),
                &message,
                attachments.as_deref().unwrap_or_default(),
                &injected_context,
                &artifact_references,
                origin.queue_id(),
            )
            .await
        };
        match result {
            Ok(_stop_reason) => {
                if !completion_delivery_ids.is_empty() {
                    let _ = state
                        .store
                        .mark_agent_workflow_deliveries_presented(&completion_delivery_ids)
                        .await;
                }
                if !resume && load_auto_review_enabled(&state.store, &frame_id).await {
                    automatic_review_acp(state, &app, &ap, &frame_id, &runtime.cancel, turn_start)
                        .await;
                }
                state.running_turns.lock().await.remove(&frame_id);
                mark_seen_if_viewed(state, &frame_id).await;
                persist_and_emit_terminal_event(
                    state,
                    &app,
                    &frame_id,
                    AgentEvent::Done {
                        frame_id: frame_id.clone(),
                        stop_reason: Some(_stop_reason),
                        effective_max_iter: None,
                    },
                )
                .await;
                return Ok(frame_id);
            }
            Err(error) => {
                state.running_turns.lock().await.remove(&frame_id);
                mark_seen_if_viewed(state, &frame_id).await;
                persist_and_emit_terminal_event(
                    state,
                    &app,
                    &frame_id,
                    AgentEvent::Error {
                        frame_id: frame_id.clone(),
                        message: error.clone(),
                        effective_max_iter: None,
                    },
                )
                .await;
                // ACP prompt submission always accepts the user turn first
                // (except early validation). Resume is always mid-turn.
                return Err(client_turn_error(true, &error));
            }
        }
    }
    // Resolve the target session frame: an explicit id wins, else lazily create
    // one (mirrors the legacy first-send behavior). The frame id is what every
    // streamed event carries, so the UI can route by session.
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            let owner = state
                .store
                .frame_project_id(id)
                .await
                .map_err(|error| error.to_string())?;
            if owner.as_deref() != Some(ap.id.as_str()) {
                return Err(format!(
                    "Session '{id}' does not belong to the active project '{}'.",
                    ap.id
                ));
            }
            id.to_string()
        }
        None => create_session_frame(&state.store, &ap.id).await?,
    };
    if user_routed_turn {
        state.set_notification_window(&frame_id, window_label);
    }
    // Deliberately no set_active_frame here: see the `AppState::active_frame`
    // doc — a turn writing view state races the user's session/project switch.
    // A workflow reference is itself an accepted capability request. Persist it
    // before a Guide message can be consumed by the current loop; provider
    // profile reads below still wait for the next workflow boundary.
    if references.as_ref().is_some_and(|references| {
        references
            .iter()
            .any(|reference| matches!(reference, ComposerReferenceArg::Workflow { .. }))
    }) {
        delegation_runtime::save_session_delegation_enabled(&state.store, &ap.id, &frame_id, true)
            .await?;
    }

    // Record this project's last session on accepted send. Desktop traffic
    // must not steal the Feishu/WeChat IM project.
    channels::record_last_message_session(&state.store, &frame_id)
        .await
        .map_err(|error| format!("Failed to update the shared last-message route: {error}"))?;

    // Get or create this session's runtime. The map mutex is dropped here —
    // the per-session `agent` mutex (not this map) is what the turn holds,
    // so a turn in session A never blocks a turn in session B.
    let rt = {
        let mut sessions = state.sessions.lock().await;
        sessions
            .entry(frame_id.clone())
            .or_insert_with(|| Arc::new(SessionRuntime::new()))
            .clone()
    };
    // Guide (#410): park the message for the running loop BEFORE waiting for
    // the workflow lock. Exactly one side takes each entry: either the loop
    // drains it into the running turn, or this call reclaims it below after
    // the lock is acquired and runs a normal turn with it.
    let guidance_id = if guide.unwrap_or(false) && !resume {
        let running = state.running_turns.lock().await.contains(&frame_id);
        running.then(|| {
            let id = rt
                .guidance_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            rt.pending_guidance
                .lock()
                .unwrap()
                .push((id, message.clone()));
            id
        })
    } else {
        None
    };
    let _workflow = match workflow_guard.take() {
        Some(guard) => guard,
        None => rt.workflow.clone().lock_owned().await,
    };
    rt.cancel.store(false, Ordering::SeqCst);
    if let Some(id) = guidance_id {
        let mut pending = rt.pending_guidance.lock().unwrap();
        let before = pending.len();
        pending.retain(|(gid, _)| *gid != id);
        if pending.len() == before {
            // The loop already injected this message into the previous turn
            // (persisted + User event emitted there); nothing left to run.
            return Ok(frame_id);
        }
    }
    let mut guard = rt.agent.lock().await;
    rt.discard_stale_agent(&mut guard);
    if crate::mcp_connections::host()
        .needs_catalog_refresh(&frame_id)
        .await
    {
        *guard = None;
    }
    let _progress_subscription =
        progress_observer_id.and_then(|id| channels::activate_progress_observer(id, &frame_id));
    if rt.deleted.load(Ordering::SeqCst) {
        return Err("This session was deleted while the turn was queued.".into());
    }
    // Resolve all provider-dependent settings only after this workflow owns the
    // session. A queued follow-up may have been accepted before the previous
    // turn ended; reading its profile earlier would rebuild the invalidated
    // Agent with the model that was selected at enqueue time.
    let vision_cfg = build_vision_provider_config(&state.store, &frame_id).await;
    let fallback_max_context = state
        .store
        .get_setting("max_context")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let max_iter = state
        .store
        .get_setting("max_iter")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_ITER);
    let (session_profile_id, rebound_model) =
        models::resolve_session_profile(&state.store, &frame_id).await;
    if rebound_model {
        *guard = None;
    }
    let model_label = models::session_label(&state.store, &frame_id).await;
    let specialist = specialists::session_specialist(&state.store, &frame_id).await;
    let max_context = match &specialist {
        Some(specialist) => specialists::specialist_context_window(&state.store, specialist).await,
        None => models::profile_context_window(&state.store, &session_profile_id)
            .await
            .unwrap_or(fallback_max_context as u64),
    }
    .try_into()
    .unwrap_or(fallback_max_context);
    let delegation_enabled =
        delegation_runtime::session_delegation_enabled(&state.store, &frame_id).await;
    let plan_mode_enabled = plan_mode::session_plan_mode(&state.store, &frame_id).await;
    let (
        provider,
        api_url,
        model,
        api_key,
        max_tokens,
        reasoning_effort,
        service_tier,
        user_agent,
        send_user_agent,
        send_session_id,
        session_header_name,
    ) = match &specialist {
        Some(spec) if !spec.model_id.trim().is_empty() => {
            specialists::specialist_llm(&state.store, spec).await
        }
        _ => load_session_settings(&state.store, &frame_id).await,
    };
    let cfg = build_provider_config(
        &provider,
        &api_url,
        &api_key,
        &model,
        max_tokens,
        &reasoning_effort,
        &service_tier,
        &user_agent,
        send_user_agent,
        send_session_id,
        &session_header_name,
        Some(&frame_id),
    )?;
    let primary_supports_vision = models::supports_vision(
        &state.store,
        specialist
            .as_ref()
            .map(|specialist| specialist.model_id.as_str())
            .filter(|id| !id.trim().is_empty())
            .or(Some(session_profile_id.as_str())),
    )
    .await;
    let attached_images = if resume {
        Vec::new()
    } else {
        load_image_attachments(
            state,
            &app,
            &frame_id,
            &ap.id,
            &ap.root,
            attachments.as_deref().unwrap_or_default(),
        )
        .await?
    };
    if guard.is_some()
        && state
            .store
            .max_message_seq(&frame_id)
            .await
            .map_err(|error| error.to_string())?
            > rt.last_seq()
    {
        // A background completion was atomically appended while this runtime
        // was idle. Rebuild from SQLite so the next parent turn cannot miss it.
        *guard = None;
    }
    if guard.as_ref().is_some_and(|agent| agent.root != ap.root) {
        // The cached agent was built from a stale window slot — its shell CWD
        // and session file point into another project. Rebuild it below on the
        // session's own root (#182).
        *guard = None;
    }
    if guard
        .as_ref()
        .is_some_and(|agent| agent.tools.get("delegate_tasks").is_some() != delegation_enabled)
    {
        // Delegation is a live per-session capability. Rebuild from persisted
        // messages when the toggle changed so the next turn sees the exact
        // tool set and prompt section selected by the user.
        *guard = None;
    }
    let reused_agent = guard.is_some();
    if guard.is_none() {
        let skills = active_skill_index(&state.store, &ap).await;
        let skills = match specialist.as_ref().and_then(|s| s.skills.as_ref()) {
            Some(names) => {
                let set: HashSet<String> = names.iter().cloned().collect();
                Arc::new(skills.filtered_by_names(Some(&set)))
            }
            None => skills,
        };
        // Desktop history lives in SQLite. Do not hydrate the project-shared
        // `.wisp/session.json` (CLI leftover / other session) into model context.
        let mut agent = Agent::new_without_session_file(
            cfg.clone(),
            skills.clone(),
            ap.memory.clone(),
            ap.root.clone(),
            max_context,
            max_iter,
            load_memory_enabled(&state.store).await,
            vision_cfg.clone(),
        );
        agent.set_auto_compact(load_auto_compact_enabled(&state.store).await);
        if !project_write_locked
            && specialist
                .as_ref()
                .is_some_and(|specialist| specialist.id == "scientific_illustrator")
        {
            std::fs::create_dir_all(ap.root.join("figures"))
                .map_err(|error| format!("Failed to prepare figures directory: {error}"))?;
        }
        add_configured_image_generation_tool(
            &mut agent,
            models::image_generation_config(&state.store).await,
            llm_proxy(),
            &frame_id,
        );
        add_configured_video_generation_tool(
            &mut agent,
            models::video_generation_config(&state.store).await,
            llm_proxy(),
            &frame_id,
        );
        agent.add_tool(Box::new(browser_bridge::BrowserSetupTool::new(
            state.browser_bridge.clone(),
            state.store.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebScanTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebExecuteJsTool::new(
            state.browser_bridge.clone(),
            state.store.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebOpenTabTool::new(
            state.browser_bridge.clone(),
            state.store.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebScreenshotTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebSaveAssetsTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebAgentSendTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebAgentWaitTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(browser_bridge::WebAgentReadTool::new(
            state.browser_bridge.clone(),
        )));
        agent.add_tool(Box::new(run_context::RunInContextTool::new(
            state.store.clone(),
            state.run_manager.clone(),
            ap.id.clone(),
            Some(frame_id.clone()),
        )));
        agent.add_tool(Box::new(run_context::ConfigureSshTrustTool::new(
            state.store.clone(),
            state.run_manager.clone(),
            Some(frame_id.clone()),
        )));
        agent.add_tool(Box::new(run_context::TransferBetweenContextsTool::new(
            state.store.clone(),
            state.run_manager.clone(),
            ap.id.clone(),
            Some(frame_id.clone()),
        )));
        agent.add_tool(Box::new(run_context::GetRunTool::new_in_scope(
            state.store.clone(),
            frame_scope.clone(),
        )));
        agent.add_tool(Box::new(run_context::MonitorRunTool::new_in_scope(
            state.store.clone(),
            frame_scope.clone(),
        )));
        agent.add_tool(Box::new(run_context::CancelRunTool::new_in_scope(
            state.store.clone(),
            state.run_manager.clone(),
            frame_scope.clone(),
        )));
        agent.add_tool(Box::new(run_context::HarvestRunTool::new_in_scope(
            state.store.clone(),
            state.run_manager.clone(),
            frame_scope.clone(),
        )));
        agent.add_tool(Box::new(
            run_context::CleanupRunWorkspaceTool::new_in_scope(
                state.store.clone(),
                state.run_manager.clone(),
                frame_scope.clone(),
            ),
        ));
        agent.add_tool(Box::new(run_context::ListRemoteFilesTool::new(
            state.store.clone(),
            ap.id.clone(),
            Some(frame_id.clone()),
        )));
        agent.add_tool(Box::new(run_context::RemoveRemoteFilesTool::new(
            state.store.clone(),
            state.run_manager.clone(),
            ap.id.clone(),
            Some(frame_id.clone()),
        )));
        agent.add_tool(Box::new(method_search::PrepareMethodSearchTool::new(
            state.store.clone(),
            ap.id.clone(),
            frame_id.clone(),
        )));
        agent.add_tool(Box::new(research_graph::ResearchGraphTool::new_in_scope(
            state.store.clone(),
            frame_scope.clone(),
        )));
        agent.add_tool(Box::new(quick_actions::ExplainWorkflowTool::new(
            state.store.clone(),
        )));
        agent.add_tool(Box::new(quick_actions::SearchModelsTool::new(
            state.store.clone(),
        )));
        agent.add_tool(Box::new(
            quick_actions::CreateWorkflowTool::new(state.store.clone(), skills.clone()).in_project(
                ap.clone(),
                frame_id.clone(),
                state.app_data.clone(),
            ),
        ));
        agent.add_tool(Box::new(specialist_tool::SaveSpecialistTool {
            store: state.store.clone(),
        }));
        agent.add_tool(Box::new(configure::ConfigureTool::new(
            state.store.clone(),
            state.app_data.clone(),
            ap.id.clone(),
        )));
        // Always registered, not just in plan mode: a fork during execution
        // deserves a question as much as one during planning.
        agent.add_tool(Box::new(wisp_tools::ask_user::AskUserTool));
        if plan_mode_enabled {
            // Only while planning: outside plan mode there is nothing to approve,
            // and an always-present tool just invites plans nobody asked for.
            // Toggling the flag evicts idle runtimes, so this re-runs.
            agent.add_tool(Box::new(wisp_tools::plan::ProposePlanTool));
        }
        if delegation_enabled {
            agent.add_tool(Box::new(
                delegation_tool::DelegateTasksTool::new(
                    state.store.clone(),
                    ap.clone(),
                    frame_id.clone(),
                    state.run_manager.clone(),
                    state.runtime_manager.clone(),
                    state.app_data.clone(),
                )
                .await?,
            ));
            agent.add_tool(Box::new(delegation_tool::GetDelegatedResultTool::new(
                state.store.clone(),
                ap.id.clone(),
                frame_id.clone(),
            )));
        }
        let mut msgs =
            wisp_core::require_sqlite_session_messages(state.store.load_messages(&frame_id).await)?;
        wisp_core::bound_tool_results_in_history(&ap.root, &mut msgs);
        agent.ctx.messages = msgs;
        if let Some(message) = agent.ctx.messages.first_mut() {
            if let wisp_llm::Content::Text(prompt) = &mut message.content {
                ssh_hosts::strip_legacy_compute_section(prompt);
            }
        }
        // last_seq is durable MAX(seq). Repair after this so the incremental
        // flush writes synthetic tool results the same way a skipped-batch
        // result is persisted (#979).
        rt.sync_last_seq_from_store(&state.store, &frame_id).await?;
        if agent.ctx.repair_unpaired_tool_calls() > 0 {
            tracing::warn!(
                "repaired unpaired tool_calls in {frame_id} so the provider transcript stays paired"
            );
        }
        agent.seed_system_prompt(&skills, None);
        if let Some(message) = agent.ctx.messages.first_mut() {
            if let wisp_llm::Content::Text(prompt) = &mut message.content {
                delegation_runtime::sync_delegation_prompt(prompt, delegation_enabled);
                plan_mode::sync_plan_prompt(prompt, plan_mode_enabled);
            }
        }
        if let Some(spec) = &specialist {
            if agent.ctx.messages.len() == 1 && !spec.instructions.trim().is_empty() {
                let section = specialist_prompt_section(spec);
                if let Some(m) = agent.ctx.messages.first_mut() {
                    if let wisp_llm::Content::Text(t) = &mut m.content {
                        append_specialist_section_once(t, &section);
                    }
                }
            }
        }
        let connector_allow: Option<HashSet<String>> = specialist
            .as_ref()
            .and_then(|s| s.connectors.as_ref())
            .map(|v| v.iter().cloned().collect());
        let wiring = wire_runtimes_and_mcp(
            &mut agent.tools,
            &state.runtime_manager,
            &ap.id,
            frame_scope.scope_key(),
            &frame_id,
            &state.store,
            None,
            connector_allow.as_ref(),
        )
        .await;
        {
            let mut observed = state.plugin_runtime_errors.lock().unwrap();
            let project_errors = observed.entry(ap.id.clone()).or_default();
            for (plugin_id, errors) in &wiring.plugin_runtime_checks {
                if errors.is_empty() {
                    project_errors.remove(plugin_id);
                } else {
                    project_errors.insert(plugin_id.clone(), errors.clone());
                }
            }
        }
        if !wiring.errors.is_empty() {
            state.bootstrap.lock().unwrap().errors.extend(wiring.errors);
        }
        *guard = Some(agent);
    }
    let agent = guard
        .as_mut()
        .ok_or_else(|| "Failed to prepare the session agent.".to_string())?;
    if let Some(message) = agent.ctx.messages.first_mut() {
        if let wisp_llm::Content::Text(prompt) = &mut message.content {
            network::sync_package_guidance(prompt, &network::load(&state.store).await?);
        }
    }
    let (auto_continue, auto_continue_limit) = load_auto_continue_settings(&state.store).await;
    apply_live_agent_settings(
        agent,
        max_iter,
        load_auto_compact_enabled(&state.store).await,
        auto_continue,
        auto_continue_limit,
    );
    // InterruptReplace (#410): the user stopped the previous turn because it
    // went the wrong way — drop that turn (its user message included) from the
    // model context before running the replacement. Mirrors /compact: only the
    // persisted message rows are rewritten; the visual transcript keeps the
    // interrupted rows as history. The index is only trusted when it still
    // fits the context (an agent rebuild could have changed the row count).
    if replace.unwrap_or(false) && !resume {
        // Bind before the await below: the temporary guard in an if-let
        // scrutinee lives for the whole block and is not Send.
        let interrupted = rt.interrupted_turn_start.lock().unwrap().take();
        if let Some(start) = interrupted {
            if start < agent.ctx.messages.len() {
                // The in-memory list is row-aligned with the head epoch, so
                // the row before `start` gives the durable seq to keep.
                // Frozen epochs sit below every head seq and stay intact.
                let rows = state
                    .store
                    .load_messages_with_seq(&frame_id)
                    .await
                    .map_err(|e| format!("replace: loading the context failed: {e}"))?;
                let keep_seq = match start {
                    0 => rows.first().map_or(0, |(seq, _)| seq - 1),
                    _ => rows
                        .get(start - 1)
                        .map(|(seq, _)| *seq)
                        .ok_or_else(|| "replace: interrupted turn is out of range".to_string())?,
                };
                agent.ctx.messages.truncate(start);
                state
                    .store
                    .truncate_model_context(&frame_id, keep_seq)
                    .await
                    .map_err(|e| format!("replace: rolling back the context failed: {e}"))?;
                rt.sync_last_seq_from_store(&state.store, &frame_id).await?;
            }
        }
    }
    state
        .device_hub
        .mark_working(&frame_id, Some(ap.id.as_str()));
    // User-triggered /compact — never part of a model turn. Archive + fold the
    // in-memory context, persist the compacted working set as a new context
    // epoch (the previous rows and the visual transcript in session_ui_events
    // stay intact), and report via the existing Compaction event.
    if !resume {
        if let Some(command) = parse_manual_compact_command(&message) {
            match agent
                .compact_with_intent(command.instruction.as_deref(), command.intent)
                .await
            {
                Ok((before, after, _archive)) => {
                    let epoch = persist_compaction_epoch(
                        &state.store,
                        &frame_id,
                        &agent.ctx,
                        "manual",
                        true,
                    )
                    .await
                    .map_err(|e| {
                        format!("compact: persisting the compacted context failed: {e}")
                    })?;
                    rt.sync_last_seq_from_store(&state.store, &frame_id).await?;
                    let event = AgentEvent::Compaction {
                        frame_id: frame_id.clone(),
                        before,
                        after,
                        strategy: "manual".into(),
                        epoch: Some(epoch as u64),
                    };
                    let mut event_seq = state
                        .store
                        .next_session_ui_event_seq(&frame_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    let compaction_event_seq = event_seq;
                    append_ui_event(&state.store, &frame_id, &mut event_seq, event.clone()).await;
                    if event_seq > compaction_event_seq {
                        if let Err(error) = state
                            .store
                            .set_context_epoch_ui_event(&frame_id, epoch, compaction_event_seq)
                            .await
                        {
                            tracing::warn!("link compaction event to epoch failed: {error}");
                        }
                    }
                    emit_agent_event_in(&app, event, Some(ap.id.as_str()));
                    let (schemas, origins) = agent.tools.schemas_with_origins();
                    let context_usage = agent.ctx.context_usage(&schemas, &origins);
                    let usage_event = AgentEvent::Usage {
                        frame_id: frame_id.clone(),
                        round: 0,
                        model: model_label.clone(),
                        created_at: chrono::Utc::now().timestamp(),
                        input: 0,
                        output: 0,
                        reasoning: 0,
                        cached: 0,
                        ctx_tokens: agent.ctx.request_tokens_with_reserve(
                            wisp_core::ContextManager::estimated_tool_tokens(&schemas),
                        ),
                        max_context,
                        context_usage,
                    };
                    let mut usage_seq = state
                        .store
                        .next_session_ui_event_seq(&frame_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    append_ui_event(&state.store, &frame_id, &mut usage_seq, usage_event.clone())
                        .await;
                    emit_agent_event_in(&app, usage_event, Some(ap.id.as_str()));
                    persist_and_emit_terminal_event(
                        state,
                        &app,
                        &frame_id,
                        AgentEvent::Done {
                            frame_id: frame_id.clone(),
                            stop_reason: Some("compact".into()),
                            effective_max_iter: None,
                        },
                    )
                    .await;
                    return Ok(frame_id);
                }
                Err(e) => {
                    persist_and_emit_terminal_event(
                        state,
                        &app,
                        &frame_id,
                        AgentEvent::Error {
                            frame_id: frame_id.clone(),
                            message: e.clone(),
                            effective_max_iter: None,
                        },
                    )
                    .await;
                    return Err(e);
                }
            }
        }
    }
    log_dev_llm_dispatch(
        &frame_id,
        "primary",
        &model_label,
        &model,
        agent.provider.model(),
        reused_agent,
    );
    let completion_delivery_ids = state
        .store
        .list_unpresented_agent_workflow_deliveries(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|delivery| delivery.id)
        .collect::<Vec<_>>();
    agent.ctx.clear_runtime_injections();
    if matches!(&frame_scope, wisp_store::StateScope::Mainline { .. }) {
        let archive_index = state
            .store
            .research_archive_index(&ap.id)
            .await
            .map_err(|e| e.to_string())?;
        if !archive_index.is_empty() {
            agent.ctx.inject_user(archive_index);
        }
    }
    if let Some(memory) = memory_commands::global_memory_runtime_injection(&state.store).await {
        agent.ctx.inject_user(memory);
    }
    if let Some(injection) =
        exploration_commands::exploration_runtime_injection(&ap.root, &frame_scope)?
    {
        agent.ctx.inject_user(injection);
    }
    if project_write_locked {
        agent.ctx.inject_user(
            "An active isolated exploration has frozen project state for possible promotion. This separate mainline conversation remains available for normal discussion and read-only inspection, but project-mutating tools are unavailable. Do not attempt to write files, run commands or jobs, change project records, or call mutating external tools until the exploration round finishes.",
        );
    }
    if !resume {
        if let Some(context) = rt.mcp_app_context_injection() {
            agent.ctx.inject_user(context);
        }
        let refs = references.unwrap_or_default();
        enable_referenced_contexts(&state.store, &refs, &frame_id).await;
        if let Some(compute) = ssh_hosts::stored_compute_section(&state.store, &frame_id).await {
            agent.ctx.inject_user(compute);
        }
        let skills = active_skill_index(&state.store, &ap).await;
        for injection in
            resolve_composer_references(&state.store, &refs, &frame_id, &ap.root, &skills).await?
        {
            agent.ctx.inject_user(injection);
        }
        if let Some(injection) =
            resolve_reader_references(&state.store, &refs, &frame_id, &message, &rt.cancel).await?
        {
            agent.ctx.inject_user(injection);
        }
        // Context resolved before the turn belongs before the user's actual
        // request. Observations and review corrections injected later remain
        // at the tail.
        agent.ctx.prefix_runtime_injections_to_user();
    }
    if rt.cancel.load(Ordering::SeqCst) {
        return Err("Turn was cancelled before it started.".into());
    }

    // Incremental persistence: a background task appends each message the turn
    // produces to SQLite as it arrives (via TauriOutput::on_message), so a crash
    // no longer loses the whole turn. The task owns the running seq, so it stays
    // correct even if the in-memory context is compacted mid-turn.
    //
    // First flush any messages already in the context but not yet persisted
    // (e.g. a system prompt seeded here), so the incremental seq lines up with
    // what a later reload expects. Stop at the first append failure so later
    // rows cannot be written after a hole.
    let start_seq = {
        // Compare against the head-epoch row count, not `last_seq`: after a
        // compaction the seq space runs ahead of the row count.
        let start = state
            .store
            .message_count(&frame_id)
            .await
            .map_err(|error| format!("incremental persist failed: {error}"))?
            as usize;
        if start < agent.ctx.messages.len() {
            let mut seq = rt.last_seq();
            for m in &agent.ctx.messages[start..] {
                seq += 1;
                if let Err(error) = state.store.append_message(&frame_id, seq, m).await {
                    let _ = rt.sync_last_seq_from_store(&state.store, &frame_id).await;
                    return Err(format!("incremental persist failed: {error}"));
                }
            }
            rt.sync_last_seq_from_store(&state.store, &frame_id).await?;
        }
        rt.last_seq()
    };

    let (persist_handle, persist_tx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let store = state.store.clone();
        let fid = frame_id.clone();
        let resource_root = ap.root.clone();
        let resource_project_id = ap.id.clone();
        let resource_app = app.clone();
        let stamp = model_label.clone();
        let handle = tokio::spawn(async move {
            wisp_store::persist_seq_loop(start_seq, rx, move |seq, mut msg| {
                let store = store.clone();
                let fid = fid.clone();
                let stamp = stamp.clone();
                let resource_root = resource_root.clone();
                let resource_project_id = resource_project_id.clone();
                let resource_app = resource_app.clone();
                async move {
                    if msg.role == wisp_llm::Role::Assistant && msg.model_name.is_none() {
                        msg.model_name = Some(stamp);
                    }
                    store.append_message(&fid, seq, &msg).await?;
                    if message_uses_resource_bindings(&msg) {
                        let resources = resource_refs::bind_new_message_resources(
                            &store,
                            &resource_root,
                            &resource_project_id,
                            &fid,
                            seq,
                            &msg.content.as_text(),
                        )
                        .await;
                        if !resources.is_empty() {
                            emit_agent_event_in(
                                &resource_app,
                                AgentEvent::Resources {
                                    frame_id: fid,
                                    seq,
                                    resources: resources.iter().map(Into::into).collect(),
                                },
                                Some(resource_project_id.as_str()),
                            );
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await
        });
        (handle, tx)
    };

    let undo_user_seq = if resume {
        state
            .store
            .load_messages_with_seq(&frame_id)
            .await
            .ok()
            .and_then(|messages| {
                messages
                    .into_iter()
                    .rev()
                    .find(|(_, message)| {
                        message.role == wisp_llm::Role::User
                            && message.tool_name.as_deref()
                                != Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL)
                    })
                    .map(|(seq, _)| seq)
            })
    } else {
        Some(start_seq + 1)
    };
    let (prov_handle, prov_tx) = {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<wisp_core::ProvenanceRecord>();
        let store = state.store.clone();
        let app_data = state.app_data.clone();
        let fid = frame_id.clone();
        let undo_root = ap.root.clone();
        let handle = tokio::spawn(async move {
            let mut env_hash: Option<String> = None;
            while let Some(rec) = rx.recv().await {
                if let Some(user_seq) = undo_user_seq {
                    turn_undo::persist_provenance_changes(
                        &store,
                        &undo_root,
                        &fid,
                        user_seq,
                        &rec.file_changes,
                    )
                    .await;
                }
                if rec.language != "r" && env_hash.is_none() {
                    env_hash = capture_env(&store, &app_data).await;
                }
                // The current environment snapshot contains Python packages.
                // Do not attach it to R provenance and imply the wrong library state.
                let record_env_hash = if rec.language == "r" {
                    None
                } else {
                    env_hash.clone()
                };
                let cell_index = store.next_cell_index(&fid).await.unwrap_or(0);
                let e = wisp_store::ExecLog {
                    id: Uuid::new_v4().to_string(),
                    frame_id: fid.clone(),
                    cell_index,
                    tool: rec.tool,
                    language: rec.language,
                    source: rec.source,
                    stdout: rec.output,
                    stderr: String::new(),
                    exit_status: if rec.success {
                        "ok".into()
                    } else {
                        "error".into()
                    },
                    wall_s: None,
                    files_written: rec.files_written,
                    files_read: rec.files_read,
                    env_hash: record_env_hash,
                };
                if let Err(e) = store.insert_execution_log(&e).await {
                    tracing::warn!("provenance persist failed: {e}");
                }
            }
        });
        (handle, tx)
    };

    let (ui_event_handle, ui_event_tx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionUiMessage>();
        *rt.ui_event_writer.lock().unwrap() = Some(tx.downgrade());
        let store = state.store.clone();
        let fid = frame_id.clone();
        let seq = store
            .next_session_ui_event_seq(&fid)
            .await
            .map_err(|e| format!("{e}"))?;
        let handle = tokio::spawn(persist_ui_events(
            store,
            fid,
            seq,
            rx,
            UI_EVENT_FLUSH_INTERVAL,
        ));
        (handle, tx)
    };

    let (live_event_handle, live_event_tx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionUiMessage>();
        *rt.live_event_writer.lock().unwrap() = Some(tx.downgrade());
        let app = app.clone();
        let live_project_id = ap.id.clone();
        let handle = tokio::spawn(coalesce_live_agent_events(
            rx,
            LIVE_EVENT_FLUSH_INTERVAL,
            move |event| {
                emit_agent_event_to_surfaces_in(&app, event, Some(live_project_id.as_str()))
            },
        ));
        (handle, tx)
    };

    let provenance_scope =
        crate::native_delegation::conversation_scope(&state.store, &frame_id).await;
    let browser_turn_id = Uuid::new_v4().to_string();
    let output = TauriOutput {
        app: app.clone(),
        frame_id: frame_id.clone(),
        queue_id: StdMutex::new(origin.queue_id()),
        queue_runtime: rt.clone(),
        model: model.clone(),
        project_id: ap.id.clone(),
        project_root: ap.root.clone(),
        restrict_read_paths_to_project: exploration_isolation.is_some(),
        exploration_isolation,
        store: state.store.clone(),
        resource_leases: state.resource_leases.clone(),
        cancel: rt.cancel.clone(),
        device_hub: state.device_hub.clone(),
        confirms: state.confirms.clone(),
        awaiting_confirm: state.awaiting_confirm.clone(),
        approvals: state.approvals.clone(),
        plan_mode: plan_mode_enabled,
        project_write_locked,
        approval_grants: state.approval_grants.clone(),
        full_permission_sessions: state.full_permission_sessions.clone(),
        persist: Some(persist_tx),
        ui_events: Some(ui_event_tx),
        live_events: Some(live_event_tx),
        message_seq: std::sync::atomic::AtomicI64::new(start_seq),
        prov: Some(prov_tx),
        provenance_scope,
        turn_id: browser_turn_id.clone(),
        force_ask_mutations: origin.force_ask_mutations(),
        last_compaction_strategy: StdMutex::new(None),
    };

    let turn_start = agent.ctx.messages.len();
    let compaction_revision = agent.ctx.compaction_revision();
    // Re-stated every turn: the session may have been switched to a different
    // model since the last one, and images already in history must follow the
    // model that is about to receive them, not the one that accepted them.
    agent.ctx.supports_vision = primary_supports_vision;
    rt.effective_max_iter.store(max_iter, Ordering::SeqCst);
    rt.effective_max_iter_known.store(true, Ordering::SeqCst);
    state.running_turns.lock().await.insert(frame_id.clone());
    let mut result = if resume {
        agent
            .run_resume(&output, Some(&rt.cancel), Some(&rt.pending_guidance))
            .await
    } else {
        agent
            .run_with_images(
                &message,
                &attached_images,
                primary_supports_vision,
                &output,
                Some(&rt.cancel),
                Some(&rt.pending_guidance),
            )
            .await
    };
    // Remember where a cancelled turn began so an InterruptReplace follow-up
    // can roll the context back to it; any other outcome clears the marker.
    *rt.interrupted_turn_start.lock().unwrap() =
        (result.is_err() && rt.cancel.load(Ordering::SeqCst)).then_some(turn_start);
    if result.is_ok() {
        if !completion_delivery_ids.is_empty() {
            let _ = state
                .store
                .mark_agent_workflow_deliveries_presented(&completion_delivery_ids)
                .await;
        }
        if matches!(result, Ok(wisp_core::AgentLoopOutcome::Completed)) {
            let is_reviewer = specialist
                .as_ref()
                .is_some_and(|specialist| specialist.id == "reviewer");
            if !resume && !is_reviewer && load_auto_review_enabled(&state.store, &frame_id).await {
                automatic_review(
                    state,
                    &app,
                    &frame_id,
                    &model_label,
                    agent,
                    &output,
                    &rt.cancel,
                    turn_start,
                )
                .await;
            }
        }
    }
    // Keep the turn-start snapshot through a possible automatic correction;
    // clear it only after the whole visual turn reaches a terminal outcome.
    agent.ctx.clear_runtime_injections();
    state.running_turns.lock().await.remove(&frame_id);

    // Close the persist channel and wait for the task to flush (abort + join on
    // timeout so a late INSERT cannot race replace/compaction). last_seq then
    // comes from durable MAX(seq), never from the in-memory message count.
    let compaction_strategy = output.take_last_compaction_strategy();
    drop(output);
    // Drain the live coalescer before the direct Done/Error emit below so the
    // final buffered deltas cannot arrive after the turn boundary.
    if tokio::time::timeout(std::time::Duration::from_secs(5), live_event_handle)
        .await
        .is_err()
    {
        tracing::warn!("live event coalescer did not finish cleanly");
    }
    if tokio::time::timeout(std::time::Duration::from_secs(5), ui_event_handle)
        .await
        .is_err()
    {
        tracing::warn!("UI event persistence did not finish cleanly");
    }
    match wisp_store::join_or_abort_persist(persist_handle, std::time::Duration::from_secs(5)).await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!("incremental persist failed: {error}"),
        Err(error) => tracing::warn!("persist task did not finish cleanly: {error}"),
    }
    if let Err(error) = rt.sync_last_seq_from_store(&state.store, &frame_id).await {
        tracing::warn!("{error}");
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), prov_handle).await;
    // The epoch must not open until the persist task has finished or been
    // aborted and joined — a late INSERT would land inside the new epoch's
    // seq range. Rows appended during this turn stay in the old epoch, where
    // their visual MessageBoundary anchors still resolve; the compacted
    // working set becomes the new head epoch on top of them.
    if agent.ctx.compaction_revision() != compaction_revision {
        // `kept_from_index` indexes the list the compaction started from. That
        // list is row-aligned with the old head epoch only for a single
        // compaction; after two in one turn it indexes an already compacted
        // list, so the (UI-only) tail origin is left unknown.
        let single_compaction = agent.ctx.compaction_revision() == compaction_revision + 1;
        match persist_compaction_epoch(
            &state.store,
            &frame_id,
            &agent.ctx,
            compaction_strategy.as_deref().unwrap_or("auto"),
            single_compaction,
        )
        .await
        {
            Ok(_) => {
                if let Err(error) = rt.sync_last_seq_from_store(&state.store, &frame_id).await {
                    tracing::warn!("{error}");
                }
            }
            Err(error) => {
                result = Err(anyhow::anyhow!(
                    "automatic compact: persisting the compacted context failed: {error}"
                ));
            }
        }
    }
    // Resume is already mid-turn. A normal send is mid-turn once the loop
    // accepted the user message (ctx grew past turn_start via on_message).
    // The UI uses this marker so it keeps the optimistic user bubble instead of
    // rolling the draft back; the visual Error card stays prefix-free.
    let turn_started = resume || agent.ctx.messages.len() > turn_start;
    drop(guard);
    // After the persist flush so the seen snapshot covers the final messages.
    mark_seen_if_viewed(state, &frame_id).await;

    match result {
        Ok(outcome) => {
            persist_and_emit_terminal_event(
                state,
                &app,
                &frame_id,
                AgentEvent::Done {
                    frame_id: frame_id.clone(),
                    stop_reason: outcome.stop_reason().map(str::to_string),
                    effective_max_iter: Some(max_iter),
                },
            )
            .await;
            emit_browser_tab_cleanup(state, &app, &browser_turn_id, &ap.id).await;
            Ok(frame_id)
        }
        Err(e) => {
            let message = wisp_llm::annotate_transport_error(
                &format!("{e}"),
                llm_proxy().as_deref(),
                &wisp_llm::ambient_proxy_env(),
            );
            persist_and_emit_terminal_event(
                state,
                &app,
                &frame_id,
                AgentEvent::Error {
                    frame_id: frame_id.clone(),
                    message: message.clone(),
                    effective_max_iter: Some(max_iter),
                },
            )
            .await;
            emit_browser_tab_cleanup(state, &app, &browser_turn_id, &ap.id).await;
            Err(client_turn_error(turn_started, &message))
        }
    }
}

async fn emit_browser_tab_cleanup(
    state: &AppState,
    app: &AppHandle,
    turn_id: &str,
    project_id: &str,
) {
    if let browser_bridge::TabCleanupAction::Prompt(prompt) =
        state.browser_bridge.complete_turn(turn_id).await
    {
        emit_to_session_surfaces_filtered(
            app,
            &prompt.frame_id,
            Some(project_id),
            "browser-tab-cleanup",
            &prompt,
            false,
        );
    }
}

/// Invoke-facing failure string for `send_message`. The live Error event carries
/// the plain message; only the Promise rejection uses the control prefix so the
/// UI can preserve a started turn's user row without painting `[turn-started]`
/// into the transcript.
pub(crate) fn client_turn_error(turn_started: bool, message: &str) -> String {
    if turn_started {
        format!("[turn-started] {message}")
    } else {
        message.to_string()
    }
}

/// Queue (#433): reconcile cut-ins, then drain ordinary follow-ups FIFO. Each
/// acquires the workflow lock and runs as a fresh turn with the item's
/// *current* text, so edits made while it waited take effect. The
/// `draining` flag is cleared under the `queued` lock so a concurrent enqueue
/// can never leave an item stranded with no driver.
async fn queued_workflow_guard(rt: &SessionRuntime) -> tokio::sync::OwnedMutexGuard<()> {
    loop {
        let guard = rt.workflow.clone().lock_owned().await;
        // A replacement reserves priority before cancelling the current
        // workflow. Yield even if the driver was already a mutex waiter.
        if rt.replacing.load(Ordering::SeqCst) == 0 {
            return guard;
        }
        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

pub(crate) fn spawn_queue_driver(
    app: AppHandle,
    rt: Arc<SessionRuntime>,
    session_id: String,
    window_label: String,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            let guard = queued_workflow_guard(&rt).await;
            let Some(item) = take_next_queued_turn(&rt) else {
                break;
            };
            emit_queued_turn_state(&app, &session_id, item.id, "started");
            let state = app.state::<AppState>();
            if let Err(error) = send_message_inner(
                state.inner(),
                app.clone(),
                &window_label,
                Some(session_id.clone()),
                item.message,
                Some(item.attachments),
                Some(item.references),
                Some(false),
                None,
                None,
                None,
                None,
                Some(guard),
                TurnOrigin::Queued(item.id),
            )
            .await
            {
                emit_queued_turn_state(&app, &session_id, item.id, "failed");
                tracing::warn!("queued turn failed: {error}");
            }
        }
    });
}

fn emit_queued_turn_state(app: &AppHandle, session_id: &str, id: u64, state: &str) {
    emit_to_session_surfaces(
        app,
        session_id,
        None,
        "queued-turn-state",
        &wisp_dto::QueuedTurnStateEvent {
            session_id: session_id.to_string(),
            id,
            state: state.to_string(),
        },
    );
}

/// Queue (#433): park a follow-up behind the running turn instead of sending
/// now. Fast, non-blocking — the driver runs it once the session frees up.
#[tauri::command]
pub(crate) async fn enqueue_turn(
    state: State<'_, AppState>,
    app: AppHandle,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
    id: u64,
    message: String,
    attachments: Option<Vec<String>>,
    references: Option<Vec<ComposerReferenceArg>>,
) -> Result<(), String> {
    if session_id.is_empty() {
        return Err("queue requires a session id".into());
    }
    let (project, scope) =
        exploration_commands::working_project_for_frame(&state, &session_id).await?;
    let _project_activity = state.begin_project_activity(&project.id)?;
    let _project_write_locked = exploration_commands::conversation_project_write_locked(
        &state.store,
        &scope,
        Some(&session_id),
    )
    .await?;
    let rt = {
        let mut sessions = state.sessions.lock().await;
        sessions
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(SessionRuntime::new()))
            .clone()
    };
    let spawn = {
        let mut q = rt.queued.lock().unwrap();
        q.push(QueuedItem {
            id,
            message,
            attachments: attachments.unwrap_or_default(),
            references: references.unwrap_or_default(),
        });
        // Claim the driver slot atomically with the push: the driver only clears
        // `draining` while holding this same lock on an empty queue.
        !rt.draining.swap(true, Ordering::SeqCst)
    };
    emit_queued_turn_state(&app, &session_id, id, "queued");
    if spawn {
        spawn_queue_driver(app, rt, session_id, window.label().to_string());
    }
    Ok(())
}

/// Queue (#433): edit / cancel / cut-in a parked follow-up by id.
/// - `edit`   → replace the item's text (runs with the latest when it drains).
/// - `cancel` → drop it from the queue.
/// - `cutin`  → offer it to the current loop, retaining its payload for a
///   priority handoff if that loop has already ended (or has not started yet).
pub(crate) fn begin_queued_cutin(rt: &SessionRuntime, id: u64) -> Option<u64> {
    // All transfers use the same lock order: queued → cut-ins → guidance.
    let mut queued = rt.queued.lock().unwrap();
    let index = queued.iter().position(|item| item.id == id)?;
    let item = queued.remove(index);
    let mut cutins = rt.queued_cutins.lock().unwrap();
    let guidance_id = rt
        .guidance_seq
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    rt.pending_guidance
        .lock()
        .unwrap()
        .push((guidance_id, item.message.clone()));
    cutins.push((guidance_id, item));
    Some(guidance_id)
}

/// Reorder within the queue (#433): swap the item with its neighbour (`up`
/// toward the front), clamped at both ends. FIFO order is the Vec order,
/// which the driver drains front-first.
pub(crate) fn swap_queued_toward(q: &mut Vec<QueuedItem>, id: u64, up: bool) {
    if let Some(i) = q.iter().position(|it| it.id == id) {
        let target = if up {
            i.checked_sub(1)
        } else {
            (i + 1 < q.len()).then_some(i + 1)
        };
        if let Some(j) = target {
            q.swap(i, j);
        }
    }
}

/// Park exactly one user-authored follow-up. A second distinct draft is refused.
/// Repeating the same id does not add another item. The queue driver drains
/// this with `take_next_queued_turn` and stops when that returns nothing.
pub(crate) fn queue_one_follow_up(
    turn_running: bool,
    rt: &SessionRuntime,
    id: u64,
    message: &str,
    attachments: &[String],
) -> Result<String, String> {
    if !turn_running {
        return Err("Queue a follow-up only while a turn is running".into());
    }
    let paths = attachments
        .iter()
        .filter(|path| !path.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let text = wisp_dto::native_conversations::message_with_attachments(message, &paths);
    if text.trim().is_empty() {
        return Err("A follow-up needs text".into());
    }
    let mut queued = rt.queued.lock().unwrap();
    if let Some(existing) = queued.iter().find(|item| item.id == id) {
        return Ok(existing.message.clone());
    }
    if !queued.is_empty() {
        return Err("Only one follow-up can wait".into());
    }
    let cutins = rt.queued_cutins.lock().unwrap();
    if !cutins.is_empty() {
        return Err("Only one follow-up can wait".into());
    }
    drop(cutins);
    queued.push(QueuedItem {
        id,
        message: text.clone(),
        attachments: paths,
        references: Vec::new(),
    });
    Ok(text)
}

/// Called only by the workflow-lock owner, before it starts any queued turn.
/// Reconcile offered guidance here, not in a later mutex waiter: the FIFO
/// driver may already be ahead of the cut-in command in the lock's wait list.
pub(crate) fn take_next_queued_turn(rt: &SessionRuntime) -> Option<QueuedItem> {
    let mut queued = rt.queued.lock().unwrap();
    let mut cutins = rt.queued_cutins.lock().unwrap();
    let mut pending = rt.pending_guidance.lock().unwrap();
    let mut unconsumed = Vec::new();
    for (guidance_id, item) in cutins.drain(..) {
        if let Some(index) = pending.iter().position(|(id, _)| *id == guidance_id) {
            pending.remove(index);
            unconsumed.push(item);
        }
    }
    queued.splice(0..0, unconsumed);
    if queued.is_empty() {
        rt.draining.store(false, Ordering::SeqCst);
        None
    } else {
        Some(queued.remove(0))
    }
}

#[tauri::command]
pub(crate) async fn queued_turn_action(
    state: State<'_, AppState>,
    app: AppHandle,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
    id: u64,
    action: String,
    message: Option<String>,
) -> Result<(), String> {
    let rt = {
        let sessions = state.sessions.lock().await;
        match sessions.get(&session_id) {
            Some(rt) => rt.clone(),
            None => return Ok(()),
        }
    };
    match action.as_str() {
        "edit" => {
            if let Some(text) = message {
                let mut q = rt.queued.lock().unwrap();
                if let Some(item) = q.iter_mut().find(|it| it.id == id) {
                    item.message = text;
                }
            }
        }
        "cancel" => {
            let removed = {
                let mut queued = rt.queued.lock().unwrap();
                let before = queued.len();
                queued.retain(|it| it.id != id);
                queued.len() != before
            };
            if removed {
                emit_queued_turn_state(&app, &session_id, id, "cancelled");
            }
        }
        "cutin" => {
            if begin_queued_cutin(&rt, id).is_some() {
                emit_queued_turn_state(&app, &session_id, id, "cutin_pending");
                // A running_turns snapshot can be false during prompt setup or
                // persistence. The loop/driver handoff works in both windows.
                let spawn = {
                    let _queued = rt.queued.lock().unwrap();
                    !rt.draining.swap(true, Ordering::SeqCst)
                };
                if spawn {
                    spawn_queue_driver(app, rt, session_id, window.label().to_string());
                }
            }
        }
        // Reorder within the queue (#433): swap with the neighbour, clamped at
        // the ends. FIFO order is the Vec order, which the driver drains front-first.
        "move_up" | "move_down" => {
            let mut q = rt.queued.lock().unwrap();
            swap_queued_toward(&mut q, id, action == "move_up");
        }
        // Interrupt-and-replace from a queued row: jump the item to the front so
        // the caller's `stop_agent` hands the freed session straight to it.
        "move_front" => {
            let mut q = rt.queued.lock().unwrap();
            if let Some(i) = q.iter().position(|it| it.id == id) {
                let item = q.remove(i);
                q.insert(0, item);
            }
        }
        other => return Err(format!("unknown queued action: {other}")),
    }
    Ok(())
}

pub(crate) fn message_uses_resource_bindings(message: &Message) -> bool {
    message.role == wisp_llm::Role::Assistant
        || (message.role == wisp_llm::Role::Tool
            && message.tool_name.as_deref() == Some("attempt_completion"))
}

#[tauri::command]
pub(crate) async fn stop_agent(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(id) = session_id.as_deref().filter(|s| !s.is_empty()) {
        state.mcp_app_tool_bridges.cancel_for_frame(id);
    } else {
        state.mcp_app_tool_bridges.cancel_all();
    }
    // Cancel only the named session's turn; other conversations keep running.
    let targets: Vec<(String, Arc<SessionRuntime>)> =
        match session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => state
                .sessions
                .lock()
                .await
                .get(id)
                .cloned()
                .map(|runtime| (id.to_string(), runtime))
                .into_iter()
                .collect(),
            None => state
                .sessions
                .lock()
                .await
                .iter()
                .map(|(id, runtime)| (id.clone(), runtime.clone()))
                .collect(),
        };
    for (id, rt) in targets {
        rt.cancel.store(true, Ordering::Relaxed);
        // Wake an agent suspended on the async approval receiver. The loop
        // observes the cancel flag after the denied tool result and exits
        // instead of leaving the Stop button waiting forever.
        approval_commands::cancel_pending_confirmation(&state, &id);
    }
    if let Some(id) = session_id.as_deref().filter(|id| !id.is_empty()) {
        acp::cancel_frame(&state, id).await;
    } else {
        let ids = state
            .acp_sessions
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            acp::cancel_frame(&state, &id).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn compact_command_accepts_an_optional_instruction_without_matching_longer_names() {
        assert_eq!(
            parse_manual_compact_command("/compact"),
            Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::PruneOnly,
                instruction: None,
            })
        );
        assert_eq!(
            parse_manual_compact_command("  /compact   --semantic  "),
            Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::Semantic,
                instruction: None,
            })
        );
        assert_eq!(
            parse_manual_compact_command(
                "  /compact   --semantic  preserve QC thresholds and blockers  "
            ),
            Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::Semantic,
                instruction: Some("preserve QC thresholds and blockers".into()),
            })
        );
        assert_eq!(
            parse_manual_compact_command("  /compact   preserve QC thresholds and blockers  "),
            Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::Semantic,
                instruction: Some("preserve QC thresholds and blockers".into()),
            })
        );
        assert_eq!(
            parse_manual_compact_command("/compact   "),
            Some(ManualCompactCommand {
                intent: wisp_core::CompactIntent::PruneOnly,
                instruction: None,
            })
        );
        assert_eq!(parse_manual_compact_command("/compact2"), None);
        assert_eq!(parse_manual_compact_command("/compact --semantic2"), None);
        assert_eq!(parse_manual_compact_command("send /compact now"), None);
    }

    #[test]
    fn one_user_follow_up_is_sent_and_the_queue_stops() {
        let rt = SessionRuntime::new();
        assert!(queue_one_follow_up(false, &rt, 1, "继续", &[]).is_err());
        assert!(take_next_queued_turn(&rt).is_none());
        let parked = queue_one_follow_up(
            true,
            &rt,
            7,
            "  继续检查对照  ",
            &["uploads/notes.csv".into()],
        )
        .unwrap();
        assert_eq!(parked, "继续检查对照\n\nUploaded files: uploads/notes.csv");
        assert!(queue_one_follow_up(true, &rt, 8, "另一条", &[]).is_err());
        let replay = queue_one_follow_up(
            true,
            &rt,
            7,
            "  继续检查对照  ",
            &["uploads/notes.csv".into()],
        )
        .unwrap();
        assert_eq!(replay, parked);
        let next = take_next_queued_turn(&rt).unwrap();
        assert_eq!(next.id, 7);
        assert_eq!(next.message, parked);
        assert_eq!(next.attachments, vec!["uploads/notes.csv".to_string()]);
        assert!(take_next_queued_turn(&rt).is_none());
        assert!(take_next_queued_turn(&rt).is_none());
    }

    #[test]
    fn replacement_only_supersedes_identical_payloads_including_cutins() {
        let rt = SessionRuntime::new();
        let item = QueuedItem {
            id: 1,
            message: "same".into(),
            attachments: vec![],
            references: vec![],
        };
        let mut attachment = item.clone();
        attachment.id = 2;
        attachment.attachments.push("uploads/a.png".into());
        let mut context = item.clone();
        context.id = 3;
        context.message.push_str("\n\nProject context: keep");
        let mut reference = item.clone();
        reference.id = 4;
        reference.references.push(ComposerReferenceArg::Artifact {
            id: "report".into(),
        });
        rt.queued
            .lock()
            .unwrap()
            .extend([item.clone(), attachment, context, reference]);
        let mut cutin = item;
        cutin.id = 5;
        rt.queued.lock().unwrap().push(cutin);
        begin_queued_cutin(&rt, 5).unwrap();
        assert_eq!(
            supersede_duplicate_queued(&rt, "same", &[], &[]),
            vec![1, 5]
        );
        assert_eq!(
            rt.queued
                .lock()
                .unwrap()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert!(rt.pending_guidance.lock().unwrap().is_empty());
        assert!(rt.queued_cutins.lock().unwrap().is_empty());
    }

    #[test]
    fn replacement_reservation_is_released_on_drop() {
        let rt = Arc::new(SessionRuntime::new());
        let first = ReplacementReservation::new(rt.clone());
        let second = ReplacementReservation::new(rt.clone());
        assert_eq!(rt.replacing.load(Ordering::SeqCst), 2);
        drop(first);
        assert_eq!(rt.replacing.load(Ordering::SeqCst), 1);
        drop(second);
        assert_eq!(rt.replacing.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn replacement_precedes_an_already_waiting_queue_driver() {
        let rt = Arc::new(SessionRuntime::new());
        let active = rt.workflow.clone().lock_owned().await;
        let queued = queued_workflow_guard(&rt);
        tokio::pin!(queued);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut queued)
                .await
                .is_err()
        );
        let reservation = ReplacementReservation::new(rt.clone());
        drop(active);
        // Poll the FIFO waiter so it must explicitly yield to the replacement.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut queued)
                .await
                .is_err()
        );
        let replacement = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            rt.workflow.clone().lock_owned(),
        )
        .await
        .unwrap();
        drop(reservation);
        drop(replacement);
        let _queued = tokio::time::timeout(std::time::Duration::from_secs(1), queued)
            .await
            .unwrap();
    }

    #[test]
    fn queued_turn_origin_carries_the_backend_id() {
        assert_eq!(TurnOrigin::Queued(42).queue_id(), Some(42));
        assert_eq!(TurnOrigin::Desktop.queue_id(), None);
    }
}

#[cfg(test)]
mod context_epoch_tests {
    use super::*;
    use wisp_llm::{Completion, LlmError, Provider};

    struct SummaryProvider(&'static str);

    #[async_trait::async_trait]
    impl Provider for SummaryProvider {
        fn name(&self) -> &str {
            "fake"
        }
        fn model(&self) -> &str {
            "fake-summary"
        }
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[wisp_llm::ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            Ok(Completion {
                content: self.0.to_string(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[wisp_llm::ToolSchema],
            _sink: &mut dyn wisp_llm::StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }

    struct NoSummaryProvider;

    #[async_trait::async_trait]
    impl Provider for NoSummaryProvider {
        fn name(&self) -> &str {
            "fake"
        }
        fn model(&self) -> &str {
            "fake-none"
        }
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[wisp_llm::ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            Err(LlmError::Incomplete)
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[wisp_llm::ToolSchema],
            _sink: &mut dyn wisp_llm::StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }

    async fn store_with_frame() -> Store {
        let tmp =
            std::env::temp_dir().join(format!("wisp_agent_turn_epoch_{}.sqlite", Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
    }

    /// Persist `messages` as the frame's epoch-0 rows and load them into a
    /// context the way `send_message` builds the agent.
    async fn seeded_context(
        store: &Store,
        messages: Vec<Message>,
        max_context: usize,
    ) -> wisp_core::ContextManager {
        for (index, message) in messages.iter().enumerate() {
            store
                .append_message("f", index as i64 + 1, message)
                .await
                .unwrap();
        }
        let mut ctx = wisp_core::ContextManager::new(max_context);
        ctx.messages = store.load_messages("f").await.unwrap();
        ctx
    }

    fn long_turns(count: usize) -> Vec<Message> {
        let mut messages = vec![Message::system("sys")];
        for turn in 0..count {
            messages.push(Message::user(format!(
                "question {turn} {}",
                "u".repeat(1_400)
            )));
            messages.push(Message::assistant(format!(
                "answer {turn} {}",
                "a".repeat(1_400)
            )));
        }
        messages
    }

    fn archive(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("wisp-agent-turn-epoch-{}", std::process::id()))
            .join(name)
    }

    #[tokio::test]
    async fn semantic_compaction_opens_an_epoch_and_keeps_old_rows_and_undo() {
        let store = store_with_frame().await;
        let mut ctx = seeded_context(&store, long_turns(12), 10_000).await;
        let original_rows = store.load_messages_with_seq("f").await.unwrap();
        // A file change recorded against the third user turn (seq 6).
        store
            .save_turn_file_undo(
                "f",
                6,
                "notes.md",
                true,
                None,
                Some("a"),
                Some("b"),
                true,
                None,
            )
            .await
            .unwrap();

        ctx.compact(
            &SummaryProvider("Objective\nkeep going"),
            &archive("semantic.json"),
        )
        .await
        .unwrap();
        let outcome = ctx.last_compaction().unwrap().clone();
        assert_eq!(outcome.kind, wisp_core::CompactionKind::Semantic);

        let epoch = persist_compaction_epoch(&store, "f", &ctx, "manual", true)
            .await
            .unwrap();
        assert_eq!(epoch, 1);

        // Old rows are frozen, not rewritten.
        let epoch0 = store.load_messages_in_epoch("f", 0).await.unwrap();
        assert_eq!(epoch0.len(), original_rows.len());
        for ((seq, row), (original_seq, original)) in epoch0.iter().zip(&original_rows) {
            assert_eq!(seq, original_seq);
            assert_eq!(row.content.as_text(), original.content.as_text());
        }
        // The head epoch is exactly the compacted in-memory context.
        let head = store.load_messages_with_seq("f").await.unwrap();
        assert_eq!(head.len(), ctx.messages.len());
        for ((_, row), message) in head.iter().zip(&ctx.messages) {
            assert_eq!(row.content.as_text(), message.content.as_text());
        }
        assert_eq!(head[0].0, original_rows.len() as i64 + 1);

        let record = store.context_epoch("f", 1).await.unwrap().unwrap();
        assert_eq!(record.kind, "semantic");
        assert_eq!(record.strategy, "manual");
        assert_eq!(record.parent_epoch, 0);
        assert_eq!(
            (record.before_tokens, record.after_tokens),
            (outcome.before as i64, outcome.after as i64)
        );
        assert!(record
            .archive_ref
            .as_deref()
            .unwrap()
            .ends_with("semantic.json"));
        // Checkpoint follows the system row; the retained tail's origin is the
        // epoch-0 row whose content reappears right after the checkpoint.
        assert_eq!(record.checkpoint_seq, Some(head[0].0 + 1));
        let kept_seq = record.first_kept_seq.expect("tail origin mapped");
        let (_, kept_row) = original_rows
            .iter()
            .find(|(seq, _)| *seq == kept_seq)
            .unwrap();
        assert_eq!(kept_row.content.as_text(), head[2].1.content.as_text());
        assert!(kept_row.content.as_text().starts_with("question 1"));

        // Seq-anchored undo rows survive the compaction.
        assert_eq!(store.list_turn_file_undo("f", 6).await.unwrap().len(), 1);
        assert_eq!(store.resolve_message_epoch("f", 6).await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn prune_only_compaction_opens_an_epoch_without_checkpoint() {
        let store = store_with_frame().await;
        let mut messages = vec![Message::system("sys")];
        for turn in 0..12 {
            messages.push(Message::user(format!("question {turn}")));
            messages.push(Message::assistant(format!("answer {turn}")));
            messages.push(Message::tool(
                format!("call{turn}"),
                "shell",
                format!("tool-output-{turn} {}", "x".repeat(50)),
            ));
        }
        let mut ctx = seeded_context(&store, messages, 1_000_000).await;
        ctx.compact(&NoSummaryProvider, &archive("prune.json"))
            .await
            .unwrap();

        let epoch = persist_compaction_epoch(&store, "f", &ctx, "auto", true)
            .await
            .unwrap();
        let record = store.context_epoch("f", epoch).await.unwrap().unwrap();
        assert_eq!(record.kind, "prune_only");
        assert_eq!(record.strategy, "auto");
        assert_eq!(record.checkpoint_seq, None);
        assert_eq!(record.first_kept_seq, None);
        assert_eq!(
            store.load_messages("f").await.unwrap().len(),
            ctx.messages.len()
        );
        assert_eq!(
            store.load_messages_in_epoch("f", 0).await.unwrap().len(),
            37
        );
    }

    #[tokio::test]
    async fn unaligned_persist_leaves_the_tail_origin_unknown_but_links_the_event() {
        let store = store_with_frame().await;
        let mut ctx = seeded_context(&store, long_turns(12), 10_000).await;
        ctx.compact(
            &SummaryProvider("Objective\nkeep going"),
            &archive("unaligned.json"),
        )
        .await
        .unwrap();
        // The turn already persisted its Compaction flag (mid-turn path).
        let mut seq = store.next_session_ui_event_seq("f").await.unwrap();
        let event_seq = seq;
        append_ui_event(
            &store,
            "f",
            &mut seq,
            AgentEvent::Compaction {
                frame_id: "f".into(),
                before: 10,
                after: 5,
                strategy: "auto".into(),
                epoch: None,
            },
        )
        .await;
        append_ui_event(
            &store,
            "f",
            &mut seq,
            AgentEvent::Compaction {
                frame_id: "f".into(),
                before: 1,
                after: 3,
                strategy: "auto_continue".into(),
                epoch: None,
            },
        )
        .await;

        let epoch = persist_compaction_epoch(&store, "f", &ctx, "auto", false)
            .await
            .unwrap();
        let record = store.context_epoch("f", epoch).await.unwrap().unwrap();
        assert_eq!(record.first_kept_seq, None);
        assert_eq!(record.ui_event_seq, Some(event_seq));
    }

    #[tokio::test]
    async fn consecutive_compactions_form_a_parent_epoch_chain() {
        let store = store_with_frame().await;
        let mut ctx = seeded_context(&store, long_turns(12), 10_000).await;
        let epoch0_len = store.load_messages_in_epoch("f", 0).await.unwrap().len();
        ctx.compact(
            &SummaryProvider("Objective\nfirst fold"),
            &archive("chain-1.json"),
        )
        .await
        .unwrap();
        let first = persist_compaction_epoch(&store, "f", &ctx, "manual", true)
            .await
            .unwrap();
        ctx.compact(
            &SummaryProvider("Objective\nsecond fold"),
            &archive("chain-2.json"),
        )
        .await
        .unwrap();
        let second = persist_compaction_epoch(&store, "f", &ctx, "auto", true)
            .await
            .unwrap();
        assert_eq!((first, second), (1, 2));
        let first_record = store.context_epoch("f", 1).await.unwrap().unwrap();
        let second_record = store.context_epoch("f", 2).await.unwrap().unwrap();
        assert_eq!(first_record.parent_epoch, 0);
        assert_eq!(second_record.parent_epoch, 1);
        assert_eq!(second_record.strategy, "auto");
        assert_eq!(
            store.load_messages_in_epoch("f", 0).await.unwrap().len(),
            epoch0_len
        );
        assert_eq!(store.frame_head_epoch("f").await.unwrap(), 2);
    }

    async fn persist_visual_turn(store: &Store, event_seq: &mut i64, message_seq: i64, text: &str) {
        store
            .append_session_ui_event(
                "f",
                *event_seq,
                &format!(r#"{{"kind":"User","frame_id":"f","text":"{text}"}}"#),
            )
            .await
            .unwrap();
        *event_seq += 1;
        store
            .append_session_ui_event(
                "f",
                *event_seq,
                &format!(r#"{{"kind":"MessageBoundary","frame_id":"f","seq":{message_seq}}}"#),
            )
            .await
            .unwrap();
        *event_seq += 1;
        store
            .append_session_ui_event(
                "f",
                *event_seq,
                &format!(r#"{{"kind":"Text","frame_id":"f","delta":"a {text}"}}"#),
            )
            .await
            .unwrap();
        *event_seq += 1;
        store
            .append_session_ui_event(
                "f",
                *event_seq,
                &format!(
                    r#"{{"kind":"MessageBoundary","frame_id":"f","seq":{}}}"#,
                    message_seq + 1
                ),
            )
            .await
            .unwrap();
        *event_seq += 1;
    }

    #[tokio::test]
    async fn resolve_visual_keep_uses_pre_compact_anchors() {
        let store = store_with_frame().await;
        let mut ctx = seeded_context(&store, long_turns(4), 10_000).await;
        let mut event_seq = 1i64;
        // system + (user, assistant) * 4 → user seqs 2,4,6,8
        persist_visual_turn(&store, &mut event_seq, 2, "q0").await;
        persist_visual_turn(&store, &mut event_seq, 4, "q1").await;
        persist_visual_turn(&store, &mut event_seq, 6, "q2").await;
        persist_visual_turn(&store, &mut event_seq, 8, "q3").await;
        ctx.compact(
            &SummaryProvider("Objective\nkeep going"),
            &archive("rewind.json"),
        )
        .await
        .unwrap();
        persist_compaction_epoch(&store, "f", &ctx, "manual", true)
            .await
            .unwrap();

        let before = crate::session_commands::resolve_visual_keep(&store, "f", 1, false)
            .await
            .unwrap();
        assert_eq!(before, (0, 3));
        let after = crate::session_commands::resolve_visual_keep(&store, "f", 1, true)
            .await
            .unwrap();
        assert_eq!(after, (0, 5));

        store.rewind_to_seq("f", before.0, before.1).await.unwrap();
        let head = store.load_messages("f").await.unwrap();
        assert!(head
            .iter()
            .any(|m| m.content.as_text().starts_with("question 0")));
        assert!(head
            .iter()
            .all(|m| !m.content.as_text().starts_with("question 1")));
        assert!(!head
            .iter()
            .any(wisp_core::ContextManager::is_summary_checkpoint));
    }

    #[test]
    fn head_keep_seq_maps_row_counts_onto_durable_seqs() {
        let rows = vec![
            (7, Message::system("sys")),
            (8, Message::user("q")),
            (9, Message::assistant("a")),
        ];
        assert_eq!(crate::session_commands::head_keep_seq(&rows, 0), 6);
        assert_eq!(crate::session_commands::head_keep_seq(&rows, 1), 7);
        assert_eq!(crate::session_commands::head_keep_seq(&rows, 3), 9);
        assert_eq!(crate::session_commands::head_keep_seq(&rows, 9), 9);
        assert_eq!(crate::session_commands::head_keep_seq(&[], 0), 0);
    }
}
