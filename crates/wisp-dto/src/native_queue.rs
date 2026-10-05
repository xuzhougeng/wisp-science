use serde::{Deserialize, Serialize};

/// Decimal strings retain every bit of WebView/native queue IDs across clients.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueItem {
    pub id: String,
    pub digest: String,
    pub state: String,
    pub message: String,
    pub attachments: Vec<String>,
    pub references: Vec<super::ComposerReferenceArg>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueOutcome {
    pub id: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueSnapshot {
    pub items: Vec<QueueItem>,
    pub outcomes: Vec<QueueOutcome>,
    pub can_cut_in: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueueActionRequest {
    pub session_id: String,
    pub id: String,
    pub digest: String,
    pub action: QueueAction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueueAction {
    Edit { message: String },
    Cancel,
    CutIn,
    MoveUp,
    MoveDown,
    Replace,
}

impl QueueActionRequest {
    pub fn queue_id(&self) -> Result<u64, &'static str> {
        let id = self.id.parse::<u64>().map_err(|_| "Invalid queue ID")?;
        if id.to_string() != self.id {
            return Err("Invalid queue ID");
        }
        if self.digest.is_empty() {
            return Err("A queue payload digest is required");
        }
        if let QueueAction::Edit { message } = &self.action {
            if message.len() > 131_072 {
                return Err("Queued message exceeds 128 KiB");
            }
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_ids_remain_exact_above_javascript_integer_precision() {
        let request: QueueActionRequest = serde_json::from_str(r#"{"session_id":"s","id":"18446744073709551615","digest":"d","action":{"kind":"replace"}}"#).unwrap();
        assert_eq!(request.queue_id().unwrap(), u64::MAX);
        assert!(serde_json::from_str::<QueueActionRequest>(r#"{"session_id":"s","id":18446744073709551615,"digest":"d","action":{"kind":"replace"}}"#).is_err());
        let mut invalid = request;
        invalid.id = "01".into();
        assert!(invalid.queue_id().is_err());
    }
}
