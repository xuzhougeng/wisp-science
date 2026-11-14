//! A conversation cut down to what a side request to the model needs: who
//! said what, with long messages and tool results shortened. Used by the
//! requests that read a conversation without being part of it.

use wisp_llm::{Message, Role};

/// How many characters are shown of one message, of one tool result, and
/// of the whole excerpt.
pub(crate) struct Limits {
    pub message: usize,
    pub tool: usize,
    pub total: usize,
}

/// `messages` as labelled blocks, oldest first. When that is still longer
/// than `limits.total` the oldest part goes, because what is asked about a
/// conversation is mostly about where it ended.
pub(crate) fn transcript(messages: &[Message], limits: &Limits) -> String {
    let mut blocks = Vec::new();
    for message in messages {
        let text = message.content.as_text();
        let (label, limit) = match message.role {
            Role::System => continue,
            Role::User => ("USER".to_owned(), limits.message),
            Role::Assistant => ("ASSISTANT".to_owned(), limits.message),
            Role::Tool => match message.tool_name.as_deref() {
                // What the model said through its completion tool is said
                // to the user like any answer.
                Some("attempt_completion") => ("ASSISTANT".to_owned(), limits.message),
                name => (format!("TOOL {}", name.unwrap_or("tool")), limits.tool),
            },
        };
        if !text.trim().is_empty() {
            blocks.push(format!("[{label}]\n{}", clipped(text.trim(), limit)));
        }
    }
    let whole = blocks.join("\n\n");
    let extra = whole.chars().count().saturating_sub(limits.total);
    if extra == 0 {
        return whole;
    }
    let tail: String = whole.chars().skip(extra).collect();
    format!("[Earlier part left out]\n…{tail}")
}

fn clipped(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        message: 200,
        tool: 60,
        total: 2_400,
    };

    #[test]
    fn an_excerpt_is_short_and_keeps_the_end() {
        let messages = [
            Message::system("You are Wisp."),
            Message::user("count the reads"),
            // A call with nothing said beside it is not a block.
            Message::assistant(""),
            Message::tool("c1", "shell", "y".repeat(LIMITS.tool + 50)),
            Message::tool("c2", "attempt_completion", "There are 100."),
            Message::assistant("Anything else?"),
        ];
        let text = transcript(&messages, &LIMITS);
        assert!(!text.contains("You are Wisp."));
        assert!(text.starts_with("[USER]\ncount the reads\n\n[TOOL shell]\n"));
        assert!(text.contains(&format!("{}…\n\n", "y".repeat(LIMITS.tool))));
        assert!(text.contains("[ASSISTANT]\nThere are 100.\n\n"));
        assert!(text.ends_with("[ASSISTANT]\nAnything else?"));
        assert_eq!(transcript(&[], &LIMITS), "");

        // Too long in all: the oldest part goes and the end is whole.
        let long: Vec<_> = (0..20)
            .map(|turn| Message::user(format!("{turn} {}", "é".repeat(LIMITS.message))))
            .chain([Message::assistant("The last answer.")])
            .collect();
        let text = transcript(&long, &LIMITS);
        assert!(text.starts_with("[Earlier part left out]\n…"));
        assert!(text.ends_with("[ASSISTANT]\nThe last answer."));
        assert!(text.chars().count() < LIMITS.total + 40);
    }
}
