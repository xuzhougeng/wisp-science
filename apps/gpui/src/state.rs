//! Selection generations reject stale async results, even after A -> B -> A.
//! No graphics context is needed to exercise this boundary.
use std::collections::HashMap;
use wisp_dto::native_conversations::{SCHEMA, Snapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub generation: u64,
    pub project: Option<String>,
    pub session: Option<String>,
}

#[derive(Default)]
pub struct WorkspaceState {
    generation: u64,
    pub project: Option<String>,
    pub session: Option<String>,
    pub snapshot: Option<Snapshot>,
    drafts: HashMap<String, String>,
}

impl WorkspaceState {
    pub fn selection(&self) -> Selection {
        Selection {
            generation: self.generation,
            project: self.project.clone(),
            session: self.session.clone(),
        }
    }

    pub fn select(&mut self, project: Option<String>, session: Option<String>) {
        self.generation += 1;
        self.project = project;
        self.session = session;
        self.snapshot = None;
    }

    pub fn current(&self, selection: &Selection) -> bool {
        &self.selection() == selection
    }

    pub fn accept(&mut self, selection: &Selection, snapshot: Snapshot) -> bool {
        if !self.current(selection)
            || snapshot.schema != SCHEMA
            || Some(&snapshot.project_id) != selection.project.as_ref()
            || Some(&snapshot.session_id) != selection.session.as_ref()
        {
            return false;
        }
        if let Some(previous) = &self.snapshot {
            if previous.epoch == snapshot.epoch && previous.sequence >= snapshot.sequence {
                return false;
            }
        }
        self.snapshot = Some(snapshot);
        true
    }

    pub fn save_draft(&mut self, text: String) {
        if let Some(session) = &self.session {
            self.drafts.insert(session.clone(), text);
        }
    }

    pub fn draft(&self) -> String {
        self.session
            .as_ref()
            .and_then(|id| self.drafts.get(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn can_send(&self, text: &str, pending: bool) -> bool {
        !pending
            && !text.trim().is_empty()
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| !s.running && !s.read_only)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(project: &str, session: &str, sequence: u64) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "schema": SCHEMA, "epoch": "host-1", "sequence": sequence,
            "project_id": project, "session_id": session, "items": [],
            "next_before_seq": null, "running": false, "stopping": false,
            "read_only": false, "model_id": "offline", "request_id": null,
            "error": null, "approvals": []
        }))
        .unwrap()
    }

    #[test]
    fn rejects_out_of_order_wrong_session_and_previous_visit_responses() {
        let mut state = WorkspaceState::default();
        state.select(Some("p".into()), Some("a".into()));
        let first = state.selection();
        assert!(state.accept(&first, snapshot("p", "a", 2)));
        assert!(!state.accept(&first, snapshot("p", "a", 1)));
        assert!(!state.accept(&first, snapshot("p", "b", 3)));
        assert!(!state.accept(&first, snapshot("other", "a", 3)));
        state.select(Some("p".into()), Some("b".into()));
        state.select(Some("p".into()), Some("a".into()));
        assert!(!state.accept(&first, snapshot("p", "a", 4)));
        let current = state.selection();
        assert!(state.accept(&current, snapshot("p", "a", 5)));
        let mut restarted = snapshot("p", "a", 0);
        restarted.epoch = "host-2".into();
        assert!(state.accept(&current, restarted));
    }

    #[test]
    fn drafts_survive_navigation_and_sending_requires_a_writable_idle_session() {
        let mut state = WorkspaceState::default();
        state.select(Some("p".into()), Some("a".into()));
        state.save_draft("中文\nexperiment 🧪".into());
        state.select(Some("p".into()), Some("b".into()));
        assert!(state.draft().is_empty());
        state.select(Some("p".into()), Some("a".into()));
        assert_eq!(state.draft(), "中文\nexperiment 🧪");
        assert!(!state.can_send("hello", false));
        state.accept(&state.selection(), snapshot("p", "a", 1));
        assert!(state.can_send("hello", false));
        assert!(!state.can_send(" \n", false));
        assert!(!state.can_send("hello", true));
        state.snapshot.as_mut().unwrap().running = true;
        assert!(!state.can_send("hello", false));
        state.snapshot.as_mut().unwrap().running = false;
        state.snapshot.as_mut().unwrap().read_only = true;
        assert!(!state.can_send("hello", false));
    }
}
