//! What the model is sent: a conversation's saved working set, as rows.
//!
//! The transcript shows everything that was said. The model is sent less
//! once a conversation has been compacted (a summary stands in for the
//! older turns) and more than the transcript shows (the instructions it
//! works under). This reads the working set as it is saved, so a part of
//! the instructions that is refreshed when a turn starts shows as it was
//! for the last turn.

use wisp_llm::{Message, Role};

/// As much of one message as a row carries; the size is of the whole.
const TEXT_LIMIT: usize = 16_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelView {
    pub rows: Vec<ModelViewRow>,
    /// Characters of every message together, tool calls included.
    pub chars: usize,
    /// Older turns have been replaced by a summary.
    pub compacted: bool,
    /// The working set's estimated tokens by kind, as the WebView's
    /// context usage splits them. Tool definitions are made when a turn
    /// starts and are not saved, so they are not counted here.
    pub breakdown: wisp_core::ContextUsage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelViewRow {
    pub kind: ModelRowKind,
    /// The tool, for a tool's result.
    pub tool: Option<String>,
    /// The tools an assistant message asked for.
    pub calls: Vec<String>,
    /// The message's text, cut when very long.
    pub text: String,
    /// How long the whole message is, in characters.
    pub chars: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelRowKind {
    /// The instructions the model works under.
    System,
    /// The summary that stands in for compacted turns.
    Summary,
    User,
    Assistant,
    Tool,
}

/// Splits the saved working set into the kinds the WebView counts: the
/// system prompt, rules and sub-agent definitions in it, skills given to
/// the model, and the conversation.
pub(crate) fn breakdown(messages: &[Message]) -> wisp_core::ContextUsage {
    let mut context = wisp_core::ContextManager::new(usize::MAX / 4);
    context.messages = messages.to_vec();
    context.context_usage(&[], &[])
}

pub(crate) fn fold(messages: &[Message]) -> ModelView {
    let mut view = ModelView {
        breakdown: breakdown(messages),
        ..ModelView::default()
    };
    for message in messages {
        let text = message.content.as_text();
        let calls: Vec<String> = message
            .tool_calls
            .iter()
            .map(|call| call.function.name.clone())
            .collect();
        let chars = text.chars().count()
            + message
                .tool_calls
                .iter()
                .map(|call| {
                    call.function.name.chars().count() + call.function.arguments.chars().count()
                })
                .sum::<usize>();
        // A message with nothing in it is not sent as anything to read.
        if chars == 0 {
            continue;
        }
        let kind = match message.role {
            Role::System => ModelRowKind::System,
            Role::User if wisp_store::is_compaction_checkpoint(&text) => ModelRowKind::Summary,
            Role::User => ModelRowKind::User,
            Role::Assistant => ModelRowKind::Assistant,
            Role::Tool => ModelRowKind::Tool,
        };
        view.compacted |= kind == ModelRowKind::Summary;
        view.chars += chars;
        view.rows.push(ModelViewRow {
            kind,
            tool: (message.role == Role::Tool)
                .then(|| message.tool_name.clone())
                .flatten(),
            calls,
            text: clipped(&text),
            chars,
        });
    }
    view
}

fn clipped(text: &str) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(TEXT_LIMIT).collect();
    match chars.count() {
        0 => kept,
        more => format!("{kept}\n[… {more} more characters]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::{FunctionCall, ToolCall};

    #[test]
    fn the_working_set_is_split_by_kind_without_tool_definitions() {
        let messages = [
            Message::system("You are Wisp. ".repeat(40)),
            Message::user("count the reads ".repeat(20)),
            Message::assistant("Counted."),
        ];
        let split = breakdown(&messages);
        assert!(split.system_prompt > 0);
        assert!(split.conversation > 0);
        // Made when a turn starts, so not in what is saved.
        assert_eq!(split.tool_definitions, 0);
        assert_eq!(split.mcp_dynamic_tools, 0);
        assert_eq!(fold(&messages).breakdown, split);
        assert_eq!(breakdown(&[]), wisp_core::ContextUsage::default());
    }

    #[test]
    fn the_working_set_is_rows_with_their_sizes() {
        let mut calling = Message::assistant("Counting.");
        calling.tool_calls = vec![ToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "shell".into(),
                arguments: r#"{"cmd":"wc"}"#.into(),
            },
        }];
        let mut silent = Message::assistant("");
        silent.tool_calls = calling.tool_calls.clone();
        let messages = [
            Message::system("You are Wisp."),
            Message::user("count the reads"),
            calling,
            Message::tool("c1", "shell", "400 reads.fq"),
            // Said nothing and asked for nothing: not a row.
            Message::assistant(""),
            // Asked for a tool without saying anything: still sent.
            silent,
            Message::tool("c1", "read", "é".repeat(TEXT_LIMIT + 5)),
        ];
        let view = fold(&messages);
        let kinds: Vec<_> = view.rows.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            [
                ModelRowKind::System,
                ModelRowKind::User,
                ModelRowKind::Assistant,
                ModelRowKind::Tool,
                ModelRowKind::Assistant,
                ModelRowKind::Tool,
            ]
        );
        assert!(!view.compacted);
        assert_eq!(view.rows[0].text, "You are Wisp.");
        // An assistant message counts what it said and what it asked for.
        assert_eq!(view.rows[2].calls, ["shell"]);
        assert_eq!(view.rows[2].chars, 9 + 5 + 12);
        assert_eq!(view.rows[3].tool.as_deref(), Some("shell"));
        assert_eq!(view.rows[4].text, "");
        assert_eq!(view.rows[4].chars, 5 + 12);
        // A very long result is cut in the row and whole in the count.
        let long = &view.rows[5];
        assert_eq!(long.chars, TEXT_LIMIT + 5);
        assert!(long.text.ends_with("\n[… 5 more characters]"));
        assert_eq!(
            view.chars,
            view.rows.iter().map(|row| row.chars).sum::<usize>()
        );
        assert_eq!(fold(&[]), ModelView::default());
    }

    #[test]
    fn a_compacted_conversation_shows_the_summary_that_stands_in() {
        let view = fold(&[
            Message::system("You are Wisp."),
            Message::user("[context summary checkpoint]\nThe reads were counted: 100."),
            Message::user("and the adapters?"),
        ]);
        let kinds: Vec<_> = view.rows.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            [
                ModelRowKind::System,
                ModelRowKind::Summary,
                ModelRowKind::User
            ]
        );
        assert!(view.compacted);
    }
}
