//! Memory Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

const AUTO_FAILURE_ANALYSIS_SETTING: &str = "auto_failure_analysis";
const GLOBAL_MEMORY_CONTEXT_CAP: usize = 6_000;
const CONFIRMED_MEMORY_CAP: usize = 4_000;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub(super) struct AutoFailureAnalysisSettings {
    enabled: bool,
    failure_rate_threshold: u8,
    minimum_failures: u16,
}

impl Default for AutoFailureAnalysisSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_rate_threshold: 30,
            minimum_failures: 2,
        }
    }
}

impl AutoFailureAnalysisSettings {
    fn normalized(mut self) -> Self {
        self.failure_rate_threshold = self.failure_rate_threshold.clamp(1, 100);
        self.minimum_failures = self.minimum_failures.clamp(1, 100);
        self
    }

    fn should_analyze(self, snapshot: &turn_memory::TurnSnapshot) -> bool {
        self.enabled
            && snapshot.failed_tool_calls >= self.minimum_failures as usize
            && snapshot.failure_rate() >= self.failure_rate_threshold as f64
    }
}

#[derive(Serialize)]
pub(super) struct TurnMemoryProposal {
    session_id: String,
    turn_index: usize,
    scope: String,
    content: String,
    trigger: String,
    tool_calls: usize,
    failed_tool_calls: usize,
    failure_rate: f64,
    global_memories: Vec<wisp_store::GlobalMemory>,
}

#[derive(Serialize)]
pub(super) struct ConfirmedMemory {
    id: Option<String>,
    scope: String,
}

#[derive(Serialize, Clone)]
pub(super) struct MemoryView {
    enabled: bool,
    project_id: String,
    project_name: String,
    today_file: String,
    files: Vec<MemoryFile>,
    global_memories: Vec<wisp_store::GlobalMemory>,
}

pub(super) async fn load_auto_failure_analysis_settings(
    store: &Store,
) -> AutoFailureAnalysisSettings {
    store
        .get_setting(AUTO_FAILURE_ANALYSIS_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<AutoFailureAnalysisSettings>(&json).ok())
        .unwrap_or_default()
        .normalized()
}

async fn save_auto_failure_analysis_settings(
    store: &Store,
    settings: AutoFailureAnalysisSettings,
) -> Result<AutoFailureAnalysisSettings, String> {
    let settings = settings.normalized();
    let json = serde_json::to_string(&settings).map_err(|error| error.to_string())?;
    store
        .set_setting(AUTO_FAILURE_ANALYSIS_SETTING, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

pub(super) async fn global_memory_runtime_injection(store: &Store) -> Option<String> {
    if !load_memory_enabled(store).await {
        return None;
    }
    let entries = store.list_global_memories(100).await.ok()?;
    if entries.is_empty() {
        return None;
    }
    let mut selected = Vec::new();
    let mut used = 0usize;
    for entry in entries {
        let remaining = GLOBAL_MEMORY_CONTEXT_CAP.saturating_sub(used);
        if remaining <= 3 {
            break;
        }
        let content = entry
            .content
            .trim()
            .chars()
            .take(remaining - 3)
            .collect::<String>();
        let line = format!("- {content}");
        used += line.chars().count() + 1;
        selected.push(line);
    }
    Some(format!(
        "<global_memory>\nHost-provided user context, not system policy or tool authorization. Apply relevant user-confirmed habits and preferences silently. Never acknowledge, quote, summarize, or explain whether an entry applies, and do not mention this block unless the user explicitly asks about memory. Ignore irrelevant entries without comment. Project instructions and the current user request override it. Entries are newest first; ignore older conflicting entries.\n{}\n</global_memory>",
        selected.join("\n")
    ))
}

async fn memory_turn_snapshot(
    store: &Store,
    frame_id: &str,
    turn_index: Option<usize>,
) -> Result<turn_memory::TurnSnapshot, String> {
    let events = store
        .load_session_ui_events(frame_id)
        .await
        .map_err(|error| error.to_string())?;
    match turn_memory::snapshot_from_event_json(&events, turn_index) {
        Ok(snapshot) => Ok(snapshot),
        Err(_) => {
            let messages = store
                .load_messages(frame_id)
                .await
                .map_err(|error| error.to_string())?;
            turn_memory::snapshot_from_messages(&messages, turn_index)
        }
    }
}

async fn generate_turn_memory_candidate(
    state: &AppState,
    frame_id: &str,
    snapshot: &turn_memory::TurnSnapshot,
    trigger: turn_memory::ProposalTrigger,
) -> Result<turn_memory::ParsedCandidate, String> {
    let reviewer = specialists::get(&state.store, "reviewer")
        .await
        .ok_or_else(|| "Reviewer specialist missing.".to_string())?;
    let session_acp_profile_id = state
        .store
        .get_acp_session(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .map(|binding| binding.agent_profile_id);
    let backend = resolve_review_backend(&reviewer, session_acp_profile_id.as_deref());
    let (system_prompt, user_prompt) = turn_memory::candidate_prompts(trigger, snapshot);
    let raw = match backend {
        Some(review::ReviewBackendConfig::AcpAgent { profile_id }) => {
            if profile_id.trim().is_empty() {
                return Err("Reviewer ACP Agent is not configured.".into());
            }
            let project_id = state
                .store
                .frame_project_id(frame_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Session project was not found.".to_string())?;
            let project = project_commands::load_active_project(state, &project_id)
                .await?
                .0;
            let label = acp::profile_label(&state.store, &profile_id)
                .await
                .ok_or_else(|| "The Reviewer ACP Agent profile no longer exists.".to_string())?;
            log_dev_llm_dispatch(
                frame_id,
                "memory_analyst_acp",
                &profile_id,
                &label,
                &label,
                false,
            );
            acp::acp_read_only_once(
                state,
                &project.root,
                &profile_id,
                &format!("{system_prompt}\n\n{user_prompt}"),
                None,
            )
            .await?
        }
        backend => {
            let mut analyst = reviewer;
            if let Some(review::ReviewBackendConfig::HttpModel { profile_id }) = backend {
                analyst.model_id = profile_id;
            }
            let (provider, api_url, model, api_key, max_tokens, reasoning_effort, service_tier) =
                specialists::specialist_llm(&state.store, &analyst).await;
            let cfg = build_provider_config(
                &provider,
                &api_url,
                &api_key,
                &model,
                max_tokens,
                &reasoning_effort,
                &service_tier,
            )?;
            let llm = wisp_llm::build(cfg);
            let selected_profile = if analyst.model_id.trim().is_empty() {
                "active"
            } else {
                analyst.model_id.as_str()
            };
            log_dev_llm_dispatch(
                frame_id,
                "memory_analyst_http",
                selected_profile,
                &model,
                llm.model(),
                false,
            );
            llm.complete(
                &[Message::system(system_prompt), Message::user(user_prompt)],
                &[],
            )
            .await
            .map_err(|error| error.to_string())?
            .content
        }
    };
    turn_memory::parse_candidate(&raw)
}

fn bounded_confirmed_memory(content: &str) -> Result<String, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("Memory content cannot be empty.".into());
    }
    if content.chars().count() > CONFIRMED_MEMORY_CAP {
        return Err(format!(
            "Memory content cannot exceed {CONFIRMED_MEMORY_CAP} characters."
        ));
    }
    Ok(content.to_string())
}

/// Resolve which project's `.wisp/memory` to open. `None` / empty uses the
/// window's active project; otherwise load that project's workspace without
/// switching the active chat project.
async fn resolve_memory_target(
    state: &AppState,
    window_label: &str,
    project_id: Option<String>,
) -> Result<(String, String, MemoryManager), String> {
    let ap = state.require_active(window_label)?;
    let requested = project_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    let id = requested.unwrap_or_else(|| ap.id.clone());

    let (name, root) = if id == ap.id {
        let name = state
            .store
            .get_project(&ap.id)
            .await
            .ok()
            .flatten()
            .map(|(name, _)| name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| ap.id.clone());
        (name, ap.root.clone())
    } else {
        let (name, workspace) = state
            .store
            .get_project(&id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("project not found: {id}"))?;
        let workspace = workspace.trim().to_string();
        if workspace.is_empty() {
            return Err(format!("project {id} has no workspace directory"));
        }
        let name = if name.trim().is_empty() {
            id.clone()
        } else {
            name
        };
        (name, PathBuf::from(workspace))
    };

    Ok((id, name, MemoryManager::at(&root)))
}

async fn build_memory_view(
    state: &AppState,
    window_label: &str,
    enabled: bool,
    project_id: Option<String>,
) -> Result<MemoryView, String> {
    let (project_id, project_name, memory) =
        resolve_memory_target(state, window_label, project_id).await?;
    let global_memories = state
        .store
        .list_global_memories(100)
        .await
        .map_err(|error| error.to_string())?;
    Ok(MemoryView {
        enabled,
        project_id,
        project_name,
        today_file: chrono::Local::now().format("%Y-%m-%d.md").to_string(),
        files: list_memory_files(&memory),
        global_memories,
    })
}

async fn require_writable_memory_target(
    state: &AppState,
    window_label: &str,
    project_id: &str,
) -> Result<wisp_store::StateScope, String> {
    let (_, active_scope) =
        exploration_commands::working_project_for_active_frame(state, window_label).await?;
    let scope = if active_scope.project_id() == project_id {
        active_scope
    } else {
        wisp_store::StateScope::mainline(project_id.to_string())
    };
    exploration_commands::require_writable_scope(&state.store, &scope).await?;
    Ok(scope)
}

#[tauri::command]
pub(super) async fn get_memory_view(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    project_id: Option<String>,
) -> Result<MemoryView, String> {
    let enabled = load_memory_enabled(&state.store).await;
    build_memory_view(&state, window.label(), enabled, project_id).await
}

#[tauri::command]
pub(super) async fn set_memory_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    enabled: bool,
    project_id: Option<String>,
) -> Result<MemoryView, String> {
    save_memory_enabled(&state.store, enabled).await?;
    clear_idle_agents(&state).await;
    build_memory_view(&state, window.label(), enabled, project_id).await
}

#[tauri::command]
pub(super) async fn read_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    project_id: Option<String>,
) -> Result<String, String> {
    let (_, _, memory) = resolve_memory_target(&state, window.label(), project_id).await?;
    let path = memory_file_path(&memory, &name)?;
    if !path.is_file() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn write_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    content: String,
    project_id: Option<String>,
) -> Result<Vec<MemoryFile>, String> {
    let target_project_id = project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| state.active(window.label()).id);
    let _project_activity = state.begin_project_activity(&target_project_id)?;
    let (id, _, memory) = resolve_memory_target(&state, window.label(), project_id).await?;
    let scope = require_writable_memory_target(&state, window.label(), &id).await?;
    let path = memory_file_path(&memory, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("{e}"))?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(list_memory_files(&memory))
}

#[tauri::command]
pub(super) async fn delete_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    project_id: Option<String>,
) -> Result<Vec<MemoryFile>, String> {
    let target_project_id = project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| state.active(window.label()).id);
    let _project_activity = state.begin_project_activity(&target_project_id)?;
    let (id, _, memory) = resolve_memory_target(&state, window.label(), project_id).await?;
    let scope = require_writable_memory_target(&state, window.label(), &id).await?;
    let path = memory_file_path(&memory, &name)?;
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("{e}"))?;
        state
            .store
            .bump_state_generation(&scope)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(list_memory_files(&memory))
}

#[tauri::command]
pub(super) async fn clear_memory(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    project_id: Option<String>,
) -> Result<Vec<MemoryFile>, String> {
    let target_project_id = project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| state.active(window.label()).id);
    let _project_activity = state.begin_project_activity(&target_project_id)?;
    let (id, _, memory) = resolve_memory_target(&state, window.label(), project_id).await?;
    let scope = require_writable_memory_target(&state, window.label(), &id).await?;
    let Ok(rd) = std::fs::read_dir(memory.dir()) else {
        return Ok(vec![]);
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let _ = std::fs::remove_file(path);
        }
    }
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(list_memory_files(&memory))
}

#[tauri::command]
pub(super) async fn get_auto_failure_analysis_settings(
    state: State<'_, AppState>,
) -> Result<AutoFailureAnalysisSettings, String> {
    Ok(load_auto_failure_analysis_settings(&state.store).await)
}

#[tauri::command]
pub(super) async fn set_auto_failure_analysis_settings(
    state: State<'_, AppState>,
    settings: AutoFailureAnalysisSettings,
) -> Result<AutoFailureAnalysisSettings, String> {
    save_auto_failure_analysis_settings(&state.store, settings).await
}

#[tauri::command]
pub(super) async fn propose_turn_memory(
    state: State<'_, AppState>,
    session_id: String,
    turn_index: Option<usize>,
    automatic: Option<bool>,
) -> Result<Option<TurnMemoryProposal>, String> {
    let frame_id = session_id.trim();
    if frame_id.is_empty() {
        return Err("No session was selected for memory.".into());
    }
    if !load_memory_enabled(&state.store).await {
        return if automatic.unwrap_or(false) {
            Ok(None)
        } else {
            Err("Memory is turned off.".into())
        };
    }
    if state.running_turns.lock().await.contains(frame_id) {
        return Err("Wait for the turn to finish before creating a memory.".into());
    }
    let project_id = state
        .store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session project was not found.".to_string())?;
    let _project_activity = state.begin_project_activity(&project_id)?;
    let snapshot = memory_turn_snapshot(&state.store, frame_id, turn_index).await?;
    let trigger = if automatic.unwrap_or(false) {
        let settings = load_auto_failure_analysis_settings(&state.store).await;
        if settings.should_analyze(&snapshot) {
            turn_memory::ProposalTrigger::ToolFailures
        } else if turn_memory::explicit_memory_intent(&snapshot.user_text) {
            turn_memory::ProposalTrigger::Explicit
        } else {
            return Ok(None);
        }
    } else {
        turn_memory::ProposalTrigger::Manual
    };
    let candidate = generate_turn_memory_candidate(&state, frame_id, &snapshot, trigger).await?;
    let trigger = match trigger {
        turn_memory::ProposalTrigger::Manual => "manual",
        turn_memory::ProposalTrigger::Explicit => "explicit",
        turn_memory::ProposalTrigger::ToolFailures => "tool_failures",
    };
    // Keep the displayed percentage stable without changing the threshold math.
    let failure_rate = (snapshot.failure_rate() * 10.0).round() / 10.0;
    let global_memories = state
        .store
        .list_global_memories(100)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(TurnMemoryProposal {
        session_id: frame_id.to_string(),
        turn_index: snapshot.turn_index,
        scope: candidate.scope,
        content: candidate.content,
        trigger: trigger.into(),
        tool_calls: snapshot.tool_calls,
        failed_tool_calls: snapshot.failed_tool_calls,
        failure_rate,
        global_memories,
    }))
}

#[tauri::command]
pub(super) async fn confirm_turn_memory(
    state: State<'_, AppState>,
    session_id: String,
    turn_index: usize,
    scope: String,
    content: String,
    replace_id: Option<String>,
) -> Result<ConfirmedMemory, String> {
    if !load_memory_enabled(&state.store).await {
        return Err("Memory is turned off.".into());
    }
    let content = bounded_confirmed_memory(&content)?;
    let frame_id = session_id.trim();
    let project_id = state
        .store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session project was not found.".to_string())?;
    let _project_activity = state.begin_project_activity(&project_id)?;
    if scope.eq_ignore_ascii_case("global") {
        let now = chrono::Utc::now().timestamp();
        if let Some(replace_id) = replace_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let updated = state
                .store
                .update_global_memory(replace_id, &content, now)
                .await
                .map_err(|error| error.to_string())?;
            if !updated {
                return Err("The global memory selected for replacement no longer exists.".into());
            }
            return Ok(ConfirmedMemory {
                id: Some(replace_id.to_string()),
                scope: "global".into(),
            });
        }
        let memory = wisp_store::GlobalMemory {
            id: Uuid::new_v4().to_string(),
            content,
            source_frame_id: Some(frame_id.to_string()),
            source_turn_index: Some(turn_index as i64),
            created_at: now,
            updated_at: now,
        };
        state
            .store
            .insert_global_memory(&memory)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(ConfirmedMemory {
            id: Some(memory.id),
            scope: "global".into(),
        });
    }

    let (project, frame_scope) =
        exploration_commands::working_project_for_frame(&state, frame_id).await?;
    exploration_commands::require_writable_scope(&state.store, &frame_scope).await?;
    let short_frame = frame_id.chars().take(8).collect::<String>();
    project
        .memory
        .append(&format!(
            "[TURN-MEMORY · session {short_frame} · turn {}]\n{content}",
            turn_index + 1
        ))
        .map_err(|error| error.to_string())?;
    state
        .store
        .bump_state_generation(&frame_scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ConfirmedMemory {
        id: None,
        scope: "project".into(),
    })
}

#[tauri::command]
pub(super) async fn create_global_memory(
    state: State<'_, AppState>,
    content: String,
) -> Result<wisp_store::GlobalMemory, String> {
    let content = bounded_confirmed_memory(&content)?;
    let now = chrono::Utc::now().timestamp();
    let memory = wisp_store::GlobalMemory {
        id: Uuid::new_v4().to_string(),
        content,
        source_frame_id: None,
        source_turn_index: None,
        created_at: now,
        updated_at: now,
    };
    state
        .store
        .insert_global_memory(&memory)
        .await
        .map_err(|error| error.to_string())?;
    Ok(memory)
}

#[tauri::command]
pub(super) async fn update_global_memory(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("No global memory was selected.".into());
    }
    let content = bounded_confirmed_memory(&content)?;
    let updated = state
        .store
        .update_global_memory(id, &content, chrono::Utc::now().timestamp())
        .await
        .map_err(|error| error.to_string())?;
    if updated {
        Ok(())
    } else {
        Err("The global memory no longer exists.".into())
    }
}

#[tauri::command]
pub(super) async fn delete_global_memory(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .store
        .delete_global_memory(id.trim())
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_analysis_defaults_off_and_requires_both_thresholds() {
        let settings = AutoFailureAnalysisSettings::default();
        let snapshot = turn_memory::TurnSnapshot {
            turn_index: 0,
            user_text: "run it".into(),
            transcript: String::new(),
            tool_calls: 4,
            failed_tool_calls: 2,
        };
        assert!(!settings.should_analyze(&snapshot));
        assert!(AutoFailureAnalysisSettings {
            enabled: true,
            ..settings
        }
        .should_analyze(&snapshot));
        assert!(!AutoFailureAnalysisSettings {
            enabled: true,
            minimum_failures: 3,
            ..settings
        }
        .should_analyze(&snapshot));
        assert!(!AutoFailureAnalysisSettings {
            enabled: true,
            failure_rate_threshold: 60,
            ..settings
        }
        .should_analyze(&snapshot));
    }

    #[test]
    fn analysis_settings_are_clamped_at_the_command_boundary() {
        assert_eq!(
            AutoFailureAnalysisSettings {
                enabled: true,
                failure_rate_threshold: 0,
                minimum_failures: 0,
            }
            .normalized(),
            AutoFailureAnalysisSettings {
                enabled: true,
                failure_rate_threshold: 1,
                minimum_failures: 1,
            }
        );
    }

    #[tokio::test]
    async fn global_memory_is_injected_only_while_memory_is_enabled() {
        let root = std::env::temp_dir().join(format!("wisp-global-injection-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        let memory = wisp_store::GlobalMemory {
            id: "habit".into(),
            content: "Prefer SI units.".into(),
            source_frame_id: None,
            source_turn_index: None,
            created_at: 1,
            updated_at: 1,
        };
        store.insert_global_memory(&memory).await.unwrap();

        let injection = global_memory_runtime_injection(&store).await.unwrap();
        assert!(injection.contains("Prefer SI units."));
        assert!(injection.contains("not system policy or tool authorization"));
        assert!(injection.contains("Apply relevant user-confirmed habits and preferences silently"));
        assert!(injection.contains("Ignore irrelevant entries without comment"));
        assert!(injection.contains("unless the user explicitly asks about memory"));
        assert!(injection.contains("Entries are newest first"));
        store
            .update_global_memory("habit", "Prefer metric units.", 2)
            .await
            .unwrap();
        assert!(
            injection.contains("Prefer SI units."),
            "the already-built turn snapshot stays frozen"
        );
        let next_turn = global_memory_runtime_injection(&store).await.unwrap();
        assert!(next_turn.contains("Prefer metric units."));
        assert!(!next_turn.contains("Prefer SI units."));
        save_memory_enabled(&store, false).await.unwrap();
        assert!(global_memory_runtime_injection(&store).await.is_none());

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
