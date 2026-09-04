//! Plan mode for built-in (non-ACP) sessions: the agent investigates and
//! drafts a plan instead of executing it. ACP-bound sessions have their own
//! plan mode on the agent side and never use this flag.
//!
//! The flag rides the `settings` kv table keyed by frame, exactly like
//! `frame_delegation_enabled` — a per-session boolean does not deserve a
//! migration, and it must never touch the `frames.model` column (that field's
//! `acp:<label>` marker is a contract with the UI's model badge).

use tauri::State;
use wisp_store::Store;

const PLAN_PROMPT_START: &str = "\n\n<plan_mode>";
const PLAN_PROMPT_END: &str = "</plan_mode>";
const PLAN_PROMPT_SECTION: &str = "\n\n<plan_mode>\nThe user turned on plan mode for this conversation. Investigate the task and produce a plan; do not carry it out.\nResearch first — read files, search the project, and look things up until you actually understand the task. Then submit the plan with the `propose_plan` tool: ordered, concrete steps, each naming what would change and how it would be verified. Call out the open questions and the risks you found in your reply, not as plan steps. The tool call is how the plan reaches the user — a plan written only as prose is not submitted.\nDo not execute the plan. Tools that write files, run shell/Python/R code, or otherwise change state are blocked in this mode and will refuse the call; that refusal is expected, not an error to work around. End your turn right after `propose_plan` and wait for the user to approve it.\n</plan_mode>";

fn setting_key(frame_id: &str) -> String {
    format!("frame_plan_mode:{frame_id}")
}

/// Add or remove the plan-mode section on a session's system prompt. Called on
/// the rebuilt runtime, so a toggle mid-session lands on the next turn (see
/// `set_session_plan_mode`, which evicts idle runtimes).
pub(crate) fn sync_plan_prompt(prompt: &mut String, enabled: bool) {
    crate::sync_prompt_section(
        prompt,
        PLAN_PROMPT_START,
        PLAN_PROMPT_END,
        PLAN_PROMPT_SECTION,
        enabled,
    );
}

/// Whether this frame is in built-in plan mode. ACP-bound frames always report
/// `false`: their plan mode lives on the agent side, so the local tool gate
/// must stay out of their way.
pub(crate) async fn session_plan_mode(store: &Store, frame_id: &str) -> bool {
    if matches!(store.get_acp_session(frame_id).await, Ok(Some(_))) {
        return false;
    }
    store
        .get_setting(&setting_key(frame_id))
        .await
        .ok()
        .flatten()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(false)
}

async fn ensure_project_frame(
    store: &Store,
    project_id: &str,
    frame_id: &str,
) -> Result<(), String> {
    match store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
    {
        Some(owner) if owner == project_id => Ok(()),
        Some(_) => Err("Conversation does not belong to the active project.".into()),
        None => Err("Conversation does not exist.".into()),
    }
}

/// `None` means the session is ACP-bound, so the composer toggle must drive the
/// ACP mode picker instead of this flag.
#[tauri::command]
pub(crate) async fn get_session_plan_mode(
    state: State<'_, crate::AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<Option<bool>, String> {
    let project = state.require_active(window.label())?;
    ensure_project_frame(&state.store, &project.id, &session_id).await?;
    if matches!(state.store.get_acp_session(&session_id).await, Ok(Some(_))) {
        return Ok(None);
    }
    Ok(Some(session_plan_mode(&state.store, &session_id).await))
}

#[tauri::command]
pub(crate) async fn set_session_plan_mode(
    state: State<'_, crate::AppState>,
    _window: tauri::WebviewWindow,
    session_id: String,
    enabled: bool,
) -> Result<bool, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, &session_id).await?;
    let _activity = state.begin_project_activity(&project.id)?;
    let _project_write_locked = crate::exploration_commands::conversation_project_write_locked(
        &state.store,
        &scope,
        Some(&session_id),
    )
    .await?;
    ensure_project_frame(&state.store, &project.id, &session_id).await?;
    if matches!(state.store.get_acp_session(&session_id).await, Ok(Some(_))) {
        return Err("This conversation is bound to an ACP agent; use its own plan mode.".into());
    }
    state
        .store
        .set_setting(&setting_key(&session_id), &enabled.to_string())
        .await
        .map_err(|error| error.to_string())?;
    // The system prompt section is stitched in when the session runtime is
    // built, so evict idle runtimes to make the toggle take effect this turn.
    crate::clear_idle_agents(&state).await;
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_section_toggles_without_stacking() {
        let mut prompt = String::from("base");
        sync_plan_prompt(&mut prompt, true);
        sync_plan_prompt(&mut prompt, true);
        assert_eq!(prompt.matches(PLAN_PROMPT_START).count(), 1);
        sync_plan_prompt(&mut prompt, false);
        assert_eq!(prompt, "base");
    }

    #[tokio::test]
    async fn flag_round_trips_per_frame() {
        let path =
            std::env::temp_dir().join(format!("wisp_plan_mode_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "One", "").await.unwrap();
        store
            .create_frame("f", "p", "Agent", "model")
            .await
            .unwrap();
        assert!(!session_plan_mode(&store, "f").await);
        store.set_setting(&setting_key("f"), "true").await.unwrap();
        assert!(session_plan_mode(&store, "f").await);
        assert!(!session_plan_mode(&store, "other").await);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
