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

/// A quote's source as it may stand between backticks in what the model
/// reads: on one line, with no backtick that would close it early.
pub fn quote_source_safe(source: &str) -> String {
    source
        .replace(['\r', '\n'], " ")
        .replace('`', "\\`")
        .trim()
        .to_string()
}

/// The line in front of quoted turns of another conversation, named by
/// `label` (its title and turn range), as the model reads it.
pub fn quoted_turns_heading(label: &str) -> String {
    format!(
        "Quoted turns from an earlier conversation `{}`:\n",
        quote_source_safe(label)
    )
}

/// The line in front of an experience distilled from turns of the
/// conversation `label` names, as the model reads it.
pub fn distilled_experience_heading(label: &str) -> String {
    format!(
        "Reusable experience distilled from conversation `{}`:\n",
        quote_source_safe(label)
    )
}

/// `text` as a Markdown quote, each of its lines behind `> `.
pub fn blockquote_lines(text: &str) -> String {
    let mut out = String::new();
    for line in text.trim().lines() {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_turns_are_quoted_under_their_source() {
        assert_eq!(
            format!(
                "{}{}",
                quoted_turns_heading("a`b\nc · turns 3–5"),
                blockquote_lines("\nUser:\nq\n\nAssistant:\na\n")
            ),
            "Quoted turns from an earlier conversation `a\\`b c · turns 3–5`:\n\
             > User:\n> q\n> \n> Assistant:\n> a\n"
        );
        assert_eq!(
            distilled_experience_heading(" x · turn 1 "),
            "Reusable experience distilled from conversation `x · turn 1`:\n"
        );
        assert_eq!(blockquote_lines("  "), "");
    }

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
