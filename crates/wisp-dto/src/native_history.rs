//! Explicit, durable turn identities for native historical message actions.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TurnIdentity {
    pub user_index: usize,
    pub user_seq: i64,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoryState {
    pub revision: String,
    pub can_branch: bool,
    pub reviewing: bool,
    pub turns: Vec<TurnIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    pub session_id: String,
    pub target: TurnIdentity,
    pub revision: String,
    pub action: HistoryAction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryAction {
    Branch {
        checkpoint: Checkpoint,
    },
    Rewind,
    UndoPreview,
    Undo,
    Review,
    ProposeMemory,
    ConfirmMemory {
        scope: MemoryScope,
        content: String,
        replace_id: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Checkpoint {
    BeforeUser,
    AfterResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Project,
    Global,
}

impl HistoryRequest {
    pub fn validate(
        &self,
        state: &HistoryState,
        running: bool,
        acp: bool,
    ) -> Result<(), &'static str> {
        if state.turns.get(self.target.user_index) != Some(&self.target) {
            return Err("This historical turn changed; reload it before continuing");
        }
        let latest = self.target.user_index + 1 == state.turns.len();
        match &self.action {
            HistoryAction::Branch { .. } if !state.can_branch => {
                Err("This conversation cannot be branched")
            }
            HistoryAction::Branch { .. }
            | HistoryAction::ProposeMemory
            | HistoryAction::ConfirmMemory { .. }
                if running && latest =>
            {
                Err("Only completed historical turns are available while running")
            }
            HistoryAction::Rewind
            | HistoryAction::Undo
            | HistoryAction::UndoPreview
            | HistoryAction::Review
                if running || state.reviewing =>
            {
                Err("Wait for this conversation to finish before continuing")
            }
            HistoryAction::Rewind | HistoryAction::Undo | HistoryAction::UndoPreview if acp => {
                Err("ACP sessions cannot be rewound or undone in protocol v1")
            }
            HistoryAction::Rewind | HistoryAction::Undo | HistoryAction::UndoPreview
                if self.revision != state.revision =>
            {
                Err("The conversation changed; inspect it again before confirming")
            }
            HistoryAction::Undo | HistoryAction::UndoPreview if !latest => {
                Err("Only the latest completed turn can be undone")
            }
            HistoryAction::ConfirmMemory {
                scope,
                content,
                replace_id,
            } => {
                if content.trim().is_empty() || content.len() > 16_384 {
                    return Err("Memory content must be nonempty and at most 16384 bytes");
                }
                if *scope == MemoryScope::Project && replace_id.is_some() {
                    return Err("Only a global memory can replace another memory");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_actions_preserve_running_and_checkpoint_boundaries() {
        let state = HistoryState {
            revision: "r".into(),
            can_branch: true,
            reviewing: false,
            turns: (0..3)
                .map(|user_index| TurnIdentity {
                    user_index,
                    user_seq: user_index as i64 * 2 + 1,
                    digest: format!("turn-{user_index}"),
                })
                .collect(),
        };
        let mut request = HistoryRequest {
            session_id: "s".into(),
            target: state.turns[1].clone(),
            revision: "r".into(),
            action: HistoryAction::Branch {
                checkpoint: Checkpoint::AfterResponse,
            },
        };
        assert!(request.validate(&state, true, false).is_ok());
        request.action = HistoryAction::ProposeMemory;
        assert!(request.validate(&state, true, false).is_ok());
        request.target = state.turns[2].clone();
        assert!(request.validate(&state, true, false).is_err());
        request.action = HistoryAction::Undo;
        assert!(request.validate(&state, false, false).is_ok());
        assert!(request.validate(&state, true, false).is_err());
        assert!(request.validate(&state, false, true).is_err());
        request.revision = "stale".into();
        assert!(request.validate(&state, false, false).is_err());
        request.action = HistoryAction::ProposeMemory;
        request.target.digest = "replaced".into();
        assert!(request.validate(&state, false, false).is_err());
    }
    #[test]
    fn malformed_history_actions_do_not_fall_back_to_latest() {
        assert!(serde_json::from_value::<HistoryRequest>(serde_json::json!({
            "session_id":"s", "revision":"r", "action":{"kind":"rewind"}
        }))
        .is_err());
        assert!(serde_json::from_value::<HistoryAction>(serde_json::json!({
            "kind":"branch", "checkpoint":"latest"
        }))
        .is_err());
    }
}
