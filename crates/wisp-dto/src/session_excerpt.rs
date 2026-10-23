use serde::{Deserialize, Serialize};

/// A span of completed turns from one conversation, rendered as quotable text
/// for another conversation's composer. The source conversation is not
/// changed: the excerpt travels as a quote attached to the next message.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionExcerpt {
    pub session_id: String,
    /// Sidebar title of the source conversation; empty when it has none.
    pub title: String,
    /// Visual user-turn indexes (0-based, inclusive) the excerpt covers.
    pub start_user_index: usize,
    pub end_user_index: usize,
    /// Markdown excerpt: `User:` / `Assistant:` blocks, tool protocol removed.
    pub text: String,
    /// True when part of the excerpt was cut to fit the quote size limit.
    #[serde(default)]
    pub truncated: bool,
}

/// An editable, unsaved lesson derived from a selected conversation range.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionExperience {
    pub content: String,
    /// Evidence was shortened before generation; the draft cannot cover omissions.
    pub truncated: bool,
}

impl SessionExcerpt {
    /// Number of user turns the excerpt spans.
    pub fn turn_count(&self) -> usize {
        self.end_user_index.saturating_sub(self.start_user_index) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::SessionExcerpt;

    #[test]
    fn turn_count_is_inclusive() {
        let excerpt = SessionExcerpt {
            start_user_index: 2,
            end_user_index: 4,
            ..SessionExcerpt::default()
        };
        assert_eq!(excerpt.turn_count(), 3);
        assert_eq!(SessionExcerpt::default().turn_count(), 1);
    }
}
