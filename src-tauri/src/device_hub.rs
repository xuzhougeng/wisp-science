//! Backend-owned physical-pet state.
//!
//! Unlike the desktop pet WebView, this reducer lives for the lifetime of the
//! Tauri process. The LAN bridge can therefore answer polls even when no
//! workspace window is currently mounted.

use crate::AgentEvent;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PetState {
    #[default]
    Idle,
    Working,
    Review,
    NeedsUser,
    Done,
    Failed,
}

impl PetState {
    fn priority(self) -> u8 {
        match self {
            Self::NeedsUser => 5,
            Self::Failed => 4,
            Self::Review => 3,
            Self::Working => 2,
            Self::Done => 1,
            Self::Idle => 0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Agent is idle",
            Self::Working => "Agent is working",
            Self::Review => "Review in progress",
            Self::NeedsUser => "Needs your attention",
            Self::Done => "Agent is done",
            Self::Failed => "Agent failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetStateSnapshot {
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub state: PetState,
    pub project: &'static str,
    pub label: &'static str,
    pub session_id: Option<String>,
    pub seq: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
struct SessionActivity {
    base: PetState,
    needs_user: bool,
    project_id: Option<String>,
    changed_seq: u64,
}

impl Default for SessionActivity {
    fn default() -> Self {
        Self {
            base: PetState::Idle,
            needs_user: false,
            project_id: None,
            changed_seq: 0,
        }
    }
}

impl SessionActivity {
    fn displayed(&self) -> PetState {
        if self.needs_user {
            PetState::NeedsUser
        } else {
            self.base
        }
    }
}

struct HubState {
    sessions: HashMap<String, SessionActivity>,
    seq: u64,
    updated_at: i64,
}

impl Default for HubState {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            seq: 0,
            updated_at: chrono::Utc::now().timestamp(),
        }
    }
}

#[derive(Default)]
pub struct DeviceHub {
    inner: Mutex<HubState>,
}

impl DeviceHub {
    pub fn snapshot(&self) -> PetStateSnapshot {
        let state = self.inner.lock().unwrap();
        snapshot_from(&state)
    }

    pub fn mark_working(&self, session_id: &str, project_id: Option<&str>) {
        self.set_base(session_id, project_id, PetState::Working);
    }

    pub fn mark_needs_user(&self, session_id: &str, project_id: Option<&str>) {
        self.change(session_id, project_id, |activity| {
            if activity.needs_user {
                false
            } else {
                activity.needs_user = true;
                true
            }
        });
    }

    pub fn resolve_needs_user(&self, session_id: &str) {
        self.change(session_id, None, |activity| {
            if activity.needs_user {
                activity.needs_user = false;
                true
            } else {
                false
            }
        });
    }

    pub fn apply_agent_event(&self, event: &AgentEvent, project_id: Option<&str>) {
        let (frame_id, state) = match event {
            AgentEvent::User { frame_id, .. }
            | AgentEvent::Text { frame_id, .. }
            | AgentEvent::Reasoning { frame_id, .. }
            | AgentEvent::ToolCall { frame_id, .. }
            | AgentEvent::Stdout { frame_id, .. }
            | AgentEvent::CompactionStarted { frame_id, .. }
            | AgentEvent::CorrectionStarted { frame_id, .. } => {
                (frame_id.as_str(), PetState::Working)
            }
            AgentEvent::ToolResult {
                frame_id,
                ok: false,
                ..
            }
            | AgentEvent::Error { frame_id, .. }
            | AgentEvent::ReviewFailed { frame_id, .. } => (frame_id.as_str(), PetState::Failed),
            AgentEvent::ToolResult {
                frame_id, ok: true, ..
            } => (frame_id.as_str(), PetState::Working),
            AgentEvent::ReviewStarted { frame_id } | AgentEvent::Review { frame_id, .. } => {
                (frame_id.as_str(), PetState::Review)
            }
            AgentEvent::Done { frame_id, .. } => (frame_id.as_str(), PetState::Done),
            AgentEvent::DelegationCompleted {
                frame_id, status, ..
            } => (
                frame_id.as_str(),
                if status == "succeeded" {
                    PetState::Done
                } else {
                    PetState::Failed
                },
            ),
            AgentEvent::MessageBoundary { .. }
            | AgentEvent::Resources { .. }
            | AgentEvent::ToolCallDraft { .. }
            | AgentEvent::ToolCallDraftClear { .. }
            | AgentEvent::ToolPresentation { .. }
            | AgentEvent::Usage { .. }
            | AgentEvent::Compaction { .. }
            | AgentEvent::ContextWarning { .. }
            | AgentEvent::Diff { .. }
            | AgentEvent::FileChanged { .. } => return,
        };
        self.set_base(frame_id, project_id, state);
    }

    /// Clear a terminal physical-pet notification without touching the Agent.
    pub fn acknowledge(&self, requested_session: Option<&str>) -> bool {
        let selected = {
            let state = self.inner.lock().unwrap();
            requested_session
                .map(str::to_string)
                .or_else(|| selected_session(&state).map(|(session_id, _)| session_id.to_string()))
        };
        let Some(session_id) = selected else {
            return false;
        };
        let mut acknowledged = false;
        self.change(&session_id, None, |activity| {
            if matches!(activity.displayed(), PetState::Done | PetState::Failed) {
                activity.base = PetState::Idle;
                activity.needs_user = false;
                acknowledged = true;
                true
            } else {
                false
            }
        });
        acknowledged
    }

    fn set_base(&self, session_id: &str, project_id: Option<&str>, next: PetState) {
        self.change(session_id, project_id, |activity| {
            let changed = activity.base != next || activity.needs_user;
            activity.base = next;
            // This mirrors the desktop pet: once the Agent resumes or reaches a
            // terminal/review state, a prior permission pause is no longer live.
            activity.needs_user = false;
            changed
        });
    }

    fn change(
        &self,
        session_id: &str,
        project_id: Option<&str>,
        mutate: impl FnOnce(&mut SessionActivity) -> bool,
    ) {
        if session_id.trim().is_empty() {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        let mut activity = state.sessions.remove(session_id).unwrap_or_default();
        let project_changed =
            project_id.is_some_and(|project_id| activity.project_id.as_deref() != Some(project_id));
        if let Some(project_id) = project_id {
            activity.project_id = Some(project_id.to_string());
        }
        let changed = mutate(&mut activity) || project_changed;
        if changed {
            state.seq = state.seq.saturating_add(1);
            state.updated_at = chrono::Utc::now().timestamp();
            activity.changed_seq = state.seq;
        }
        state.sessions.insert(session_id.to_string(), activity);
    }
}

fn selected_session(state: &HubState) -> Option<(&str, &SessionActivity)> {
    state
        .sessions
        .iter()
        .max_by_key(|(_, activity)| (activity.displayed().priority(), activity.changed_seq))
        .map(|(session_id, activity)| (session_id.as_str(), activity))
}

fn snapshot_from(state: &HubState) -> PetStateSnapshot {
    let (session_id, pet_state) = selected_session(state)
        .map(|(session_id, activity)| (Some(session_id.to_string()), activity.displayed()))
        .unwrap_or((None, PetState::Idle));
    PetStateSnapshot {
        message_type: "pet_state",
        state: pet_state,
        project: "Wisp Science",
        label: pet_state.label(),
        session_id,
        seq: state.seq,
        updated_at: state.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(frame_id: &str) -> AgentEvent {
        AgentEvent::Text {
            frame_id: frame_id.into(),
            delta: "working".into(),
        }
    }

    #[test]
    fn reducer_covers_major_agent_transitions() {
        let hub = DeviceHub::default();
        assert_eq!(hub.snapshot().state, PetState::Idle);

        hub.mark_working("a", Some("project-a"));
        let working_events = [
            AgentEvent::User {
                frame_id: "a".into(),
                text: "prompt".into(),
            },
            text("a"),
            AgentEvent::Reasoning {
                frame_id: "a".into(),
                delta: "thinking".into(),
            },
            AgentEvent::ToolCall {
                frame_id: "a".into(),
                name: "read_file".into(),
                preview: "README.md".into(),
            },
            AgentEvent::Stdout {
                frame_id: "a".into(),
                chunk: "progress".into(),
            },
            AgentEvent::ToolResult {
                frame_id: "a".into(),
                name: "read_file".into(),
                ok: true,
                content: "ok".into(),
                duration_ms: 1,
            },
            AgentEvent::CorrectionStarted {
                frame_id: "a".into(),
                model: "reviewer".into(),
            },
        ];
        for event in working_events {
            hub.apply_agent_event(&event, Some("project-a"));
            assert_eq!(hub.snapshot().state, PetState::Working);
        }
        hub.apply_agent_event(
            &AgentEvent::ReviewStarted {
                frame_id: "a".into(),
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Review);
        hub.mark_needs_user("a", None);
        assert_eq!(hub.snapshot().state, PetState::NeedsUser);
        hub.resolve_needs_user("a");
        assert_eq!(hub.snapshot().state, PetState::Review);
        hub.apply_agent_event(
            &AgentEvent::Done {
                frame_id: "a".into(),
                stop_reason: None,
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Done);
        hub.apply_agent_event(
            &AgentEvent::ToolResult {
                frame_id: "a".into(),
                name: "shell".into(),
                ok: false,
                content: "failed".into(),
                duration_ms: 1,
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Failed);
        hub.apply_agent_event(
            &AgentEvent::ReviewFailed {
                frame_id: "a".into(),
                message: "review failed".into(),
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Failed);
        hub.apply_agent_event(
            &AgentEvent::Error {
                frame_id: "a".into(),
                message: "failed".into(),
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Failed);
        hub.apply_agent_event(
            &AgentEvent::DelegationCompleted {
                frame_id: "a".into(),
                workflow_id: "workflow".into(),
                status: "succeeded".into(),
                result: "{}".into(),
                auto_resume: false,
            },
            None,
        );
        assert_eq!(hub.snapshot().state, PetState::Done);
    }

    #[test]
    fn parallel_sessions_use_the_documented_priority() {
        let hub = DeviceHub::default();
        hub.apply_agent_event(&text("working"), None);
        hub.apply_agent_event(
            &AgentEvent::Done {
                frame_id: "done".into(),
                stop_reason: None,
            },
            None,
        );
        assert_eq!(hub.snapshot().session_id.as_deref(), Some("working"));

        hub.apply_agent_event(
            &AgentEvent::ReviewStarted {
                frame_id: "review".into(),
            },
            None,
        );
        assert_eq!(hub.snapshot().session_id.as_deref(), Some("review"));
        hub.apply_agent_event(
            &AgentEvent::Error {
                frame_id: "failed".into(),
                message: "no".into(),
            },
            None,
        );
        assert_eq!(hub.snapshot().session_id.as_deref(), Some("failed"));
        hub.mark_needs_user("needs-user", None);
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.state, PetState::NeedsUser);
        assert_eq!(snapshot.session_id.as_deref(), Some("needs-user"));
    }

    #[test]
    fn sequence_only_moves_forward_when_state_changes() {
        let hub = DeviceHub::default();
        let initial = hub.snapshot().seq;
        hub.apply_agent_event(&text("a"), None);
        let working = hub.snapshot().seq;
        hub.apply_agent_event(&text("a"), None);
        let repeated = hub.snapshot().seq;
        hub.mark_needs_user("a", None);
        let waiting = hub.snapshot().seq;
        hub.resolve_needs_user("a");
        let resolved = hub.snapshot().seq;

        assert!(working > initial);
        assert_eq!(repeated, working);
        assert!(waiting > repeated);
        assert!(resolved > waiting);
    }

    #[test]
    fn resolving_one_permission_does_not_clear_another_session() {
        let hub = DeviceHub::default();
        hub.mark_working("a", None);
        hub.mark_working("b", None);
        hub.mark_needs_user("a", None);
        hub.mark_needs_user("b", None);
        hub.resolve_needs_user("b");
        assert_eq!(hub.snapshot().session_id.as_deref(), Some("a"));
        assert_eq!(hub.snapshot().state, PetState::NeedsUser);
    }
}
