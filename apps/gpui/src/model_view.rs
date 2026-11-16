//! What the model is sent, as its dialog words it. The host reads the
//! conversation's saved working set; this only says what is in it.
use crate::i18n::tr;
use wisp_server::desktop::{
    ContextToolDetail, ContextUsage, ModelRowKind, ModelView, ModelViewRow, SentContext, TokenUsage,
};

use crate::controls::{compact, context_percent};

/// A line longer than this is cut in its row, so the row opens.
const ONE_LINE: usize = 90;
/// Characters to a token, for an estimate that says it is one.
const CHARS_PER_TOKEN: usize = 4;

fn tokens(chars: usize) -> String {
    // Anything that is sent costs at least a token.
    compact(chars.div_ceil(CHARS_PER_TOKEN).max(1) as u64)
}

/// The line under the title: how much is sent, and whether it is the
/// whole conversation.
pub fn summary(view: &ModelView) -> String {
    if view.rows.is_empty() {
        return tr("Nothing has been sent in this conversation yet.").into();
    }
    let size = crate::i18n::fill(
        tr("{} messages, about {} tokens by length."),
        &[&view.rows.len(), &tokens(view.chars)],
    );
    if view.compacted {
        crate::i18n::fill(
            tr(
                "{} Older turns have been replaced by a summary: the transcript still shows them, the model no longer reads them.",
            ),
            &[&size],
        )
    } else {
        crate::i18n::fill(
            tr("{} Everything in the transcript is still sent."),
            &[&size],
        )
    }
}

/// Who a row is from, as its label.
pub fn label(row: &ModelViewRow) -> String {
    match row.kind {
        ModelRowKind::System => tr("Instructions").into(),
        ModelRowKind::Summary => tr("Summary").into(),
        ModelRowKind::User => tr("You").into(),
        ModelRowKind::Assistant => "Wisp".into(),
        ModelRowKind::Tool => match row.tool.as_deref().filter(|name| !name.is_empty()) {
            Some(name) => name.to_owned(),
            None => tr("Tool").into(),
        },
    }
}

/// A row's one line: the first line of what was said, or which tools were
/// asked for when nothing was said.
pub fn line(row: &ModelViewRow) -> String {
    let first = row
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    match (first.is_empty(), row.calls.is_empty()) {
        (false, true) => first.to_owned(),
        (false, false) => format!("{first} → {}", row.calls.join(", ")),
        (true, false) => crate::i18n::fill(tr("Asks for {}"), &[&row.calls.join(", ")]),
        (true, true) => String::new(),
    }
}

/// A row's share: about how many tokens, and how much of the whole.
/// The working set by kind, as the WebView's context usage names them, in
/// its order: each kind that holds anything, with its estimated tokens and
/// its share of them all.
pub fn by_kind(view: &ModelView) -> Vec<(&'static str, usize, usize)> {
    let split = &view.breakdown;
    let kinds = [
        (tr("System prompt"), split.system_prompt),
        (tr("Rules"), split.rules),
        (tr("Skills"), split.skills),
        (tr("Sub-agent definitions"), split.subagent_definitions),
        (tr("Conversation"), split.conversation),
    ];
    let all: usize = kinds.iter().map(|(_, tokens)| tokens).sum();
    kinds
        .into_iter()
        .filter(|(_, tokens)| *tokens > 0)
        .map(|(name, tokens)| (name, tokens, (tokens * 100 + all / 2) / all.max(1)))
        .collect()
}

/// A kind of what the latest request was made of, as the WebView's
/// context usage names and orders them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    SystemPrompt,
    ToolDefinitions,
    Rules,
    Skills,
    DynamicTools,
    SubagentDefinitions,
    Conversation,
}

impl Sent {
    pub const ALL: [Sent; 7] = [
        Sent::SystemPrompt,
        Sent::ToolDefinitions,
        Sent::Rules,
        Sent::Skills,
        Sent::DynamicTools,
        Sent::SubagentDefinitions,
        Sent::Conversation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sent::SystemPrompt => tr("System prompt"),
            Sent::ToolDefinitions => tr("Tool definitions"),
            Sent::Rules => tr("Rules"),
            Sent::Skills => tr("Skills"),
            Sent::DynamicTools => tr("MCP & dynamic tools"),
            Sent::SubagentDefinitions => tr("Sub-agent definitions"),
            Sent::Conversation => tr("Conversation"),
        }
    }

    fn tokens(self, usage: &ContextUsage) -> usize {
        match self {
            Sent::SystemPrompt => usage.system_prompt,
            Sent::ToolDefinitions => usage.tool_definitions,
            Sent::Rules => usage.rules,
            Sent::Skills => usage.skills,
            Sent::DynamicTools => usage.mcp_dynamic_tools,
            Sent::SubagentDefinitions => usage.subagent_definitions,
            Sent::Conversation => usage.conversation,
        }
    }
}

/// What one kind of the latest request holds, as its row opens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holds<'a> {
    /// Tools, one by one, with what the model reads about each.
    Tools(&'a [ContextToolDetail]),
    Text(&'a str),
    /// The conversation is the rows below; nothing else to open.
    Nothing,
}

/// What `kind` holds in the latest request.
pub fn holds(sent: &SentContext, kind: Sent) -> Holds<'_> {
    let details = &sent.details;
    match kind {
        Sent::SystemPrompt => Holds::Text(&details.system_prompt),
        Sent::ToolDefinitions => Holds::Tools(&details.tool_definitions),
        Sent::Rules => Holds::Text(&details.rules),
        Sent::Skills => Holds::Text(&details.skills),
        Sent::DynamicTools => Holds::Tools(&details.mcp_dynamic_tools),
        Sent::SubagentDefinitions => Holds::Tools(&details.subagent_definitions),
        Sent::Conversation => Holds::Nothing,
    }
}

/// Whether a kind's row opens: it holds text or tools to read.
pub fn sent_opens(sent: &SentContext, kind: Sent) -> bool {
    match holds(sent, kind) {
        Holds::Tools(tools) => !tools.is_empty(),
        Holds::Text(text) => !text.trim().is_empty(),
        Holds::Nothing => false,
    }
}

/// The latest request kind by kind, every kind in the WebView's order,
/// with its tokens and its share of the context window (of the request,
/// when the window is not known or is smaller), as the WebView's bar
/// measures them.
pub fn sent_kinds(sent: &SentContext) -> Vec<(Sent, usize, usize)> {
    let all = sent.max_context.max(sent.usage.total()).max(1);
    Sent::ALL
        .into_iter()
        .map(|kind| {
            let tokens = kind.tokens(&sent.usage);
            (kind, tokens, (tokens * 100 + all / 2) / all)
        })
        .collect()
}

/// The line over the kinds: how big the latest request was, and how full
/// it made the context window, as the composer's percentage counts it.
pub fn sent_line(sent: &SentContext) -> String {
    let used = sent.usage.total();
    let full = TokenUsage {
        context_tokens: used,
        max_context: sent.max_context,
        ..Default::default()
    };
    match context_percent(&full) {
        Some(percent) => crate::i18n::fill(
            tr("The latest request to the model: about {} of {} tokens, {}% of its context."),
            &[
                &compact(used as u64),
                &compact(sent.max_context as u64),
                &percent,
            ],
        ),
        None => crate::i18n::fill(
            tr("The latest request to the model: about {} tokens."),
            &[&compact(used as u64)],
        ),
    }
}

/// As much of a kind's text as it opens to, as a row's is cut on the host.
const OPENED_LIMIT: usize = 16_000;

/// A kind's text as it opens: whole, or cut with how much more there is.
pub fn opened_text(text: &str) -> String {
    let mut chars = text.trim_end().chars();
    let kept: String = chars.by_ref().take(OPENED_LIMIT).collect();
    match chars.count() {
        0 => kept,
        more => format!(
            "{kept}\n{}",
            crate::i18n::fill(tr("[… {} more characters]"), &[&more])
        ),
    }
}

/// A tool's line under its kind: the first thing its description says.
pub fn tool_line(tool: &ContextToolDetail) -> &str {
    tool.description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

pub fn share(row: &ModelViewRow, view: &ModelView) -> String {
    let percent = if view.chars == 0 {
        0
    } else {
        (row.chars * 100 + view.chars / 2) / view.chars
    };
    match percent {
        0 => format!("≈{}", tokens(row.chars)),
        percent => format!("≈{} · {percent}%", tokens(row.chars)),
    }
}

/// Whether opening the row shows more than its line does.
pub fn opens(row: &ModelViewRow) -> bool {
    let text = row.text.trim();
    let first = text.lines().next().unwrap_or_default().trim();
    // Nothing said, or one short line that the row already shows whole.
    !text.is_empty() && (text != first || first.chars().count() > ONE_LINE)
}

/// Whether the working set shown can be put back to what it was before
/// its compaction: it is compacted, and the compaction is named.
pub fn can_undo(view: &ModelView) -> bool {
    view.compacted && view.epoch > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_named_compaction_can_be_undone() {
        let mut view = ModelView::default();
        assert!(!can_undo(&view));
        view.compacted = true;
        assert!(!can_undo(&view));
        view.epoch = 2;
        assert!(can_undo(&view));
        view.compacted = false;
        assert!(!can_undo(&view));
    }

    fn row(kind: ModelRowKind, text: &str, chars: usize) -> ModelViewRow {
        ModelViewRow {
            kind,
            tool: None,
            calls: vec![],
            text: text.into(),
            chars,
        }
    }

    #[test]
    fn the_summary_says_how_much_is_sent_and_whether_it_is_everything() {
        let mut view = ModelView::default();
        assert_eq!(
            summary(&view),
            "Nothing has been sent in this conversation yet."
        );
        view.rows = vec![
            row(ModelRowKind::System, "You are Wisp.", 8_000),
            row(ModelRowKind::User, "hello", 5),
        ];
        view.chars = 8_005;
        assert_eq!(
            summary(&view),
            "2 messages, about 2.0k tokens by length. Everything in the transcript is still sent."
        );
        view.compacted = true;
        assert!(summary(&view).contains("replaced by a summary"));
    }

    #[test]
    fn the_working_set_is_listed_by_kind_with_shares() {
        let view = ModelView {
            rows: vec![],
            chars: 0,
            compacted: false,
            breakdown: wisp_server::desktop::ContextUsage {
                system_prompt: 300,
                skills: 100,
                conversation: 600,
                ..Default::default()
            },
            epoch: 0,
            sent: None,
        };
        assert_eq!(
            by_kind(&view),
            [
                ("System prompt", 300, 30),
                ("Skills", 100, 10),
                ("Conversation", 600, 60)
            ]
        );
        let empty = ModelView {
            breakdown: Default::default(),
            ..view
        };
        assert!(by_kind(&empty).is_empty());
    }

    fn tool(name: &str, description: &str) -> ContextToolDetail {
        ContextToolDetail {
            name: name.into(),
            description: description.into(),
        }
    }

    fn sent() -> SentContext {
        SentContext {
            usage: ContextUsage {
                system_prompt: 2_000,
                tool_definitions: 6_000,
                mcp_dynamic_tools: 1_000,
                conversation: 11_000,
                ..Default::default()
            },
            details: wisp_server::desktop::ContextUsageDetails {
                system_prompt: "You are Wisp.".into(),
                tool_definitions: vec![
                    tool("read", "\n  Read a file.\nMore about reading."),
                    tool("shell", "Run a command."),
                ],
                mcp_dynamic_tools: vec![tool("pubmed_search", "Search PubMed.")],
                ..Default::default()
            },
            max_context: 200_000,
        }
    }

    #[test]
    fn the_latest_request_is_every_kind_with_its_share_of_the_window() {
        let sent = sent();
        let kinds = sent_kinds(&sent);
        assert_eq!(
            kinds.iter().map(|(kind, ..)| *kind).collect::<Vec<_>>(),
            Sent::ALL
        );
        assert_eq!(kinds[0], (Sent::SystemPrompt, 2_000, 1));
        assert_eq!(kinds[1], (Sent::ToolDefinitions, 6_000, 3));
        // A kind that holds nothing is still listed, as the WebView lists it.
        assert_eq!(kinds[2], (Sent::Rules, 0, 0));
        assert_eq!(kinds[6], (Sent::Conversation, 11_000, 6));
        assert_eq!(
            sent_line(&sent),
            "The latest request to the model: about 20.0k of 200.0k tokens, 10% of its context."
        );
        // A window that is not known: the request is the whole.
        let unknown = SentContext {
            max_context: 0,
            ..sent.clone()
        };
        assert_eq!(sent_kinds(&unknown)[6], (Sent::Conversation, 11_000, 55));
        assert_eq!(
            sent_line(&unknown),
            "The latest request to the model: about 20.0k tokens."
        );
    }

    #[test]
    fn each_kind_opens_to_its_tools_or_its_text() {
        let sent = sent();
        let Holds::Tools(built_in) = holds(&sent, Sent::ToolDefinitions) else {
            panic!("tools");
        };
        assert_eq!(
            built_in
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["read", "shell"]
        );
        assert_eq!(tool_line(&built_in[0]), "Read a file.");
        assert_eq!(
            holds(&sent, Sent::DynamicTools),
            Holds::Tools(&sent.details.mcp_dynamic_tools)
        );
        assert_eq!(
            holds(&sent, Sent::SystemPrompt),
            Holds::Text("You are Wisp.")
        );
        assert_eq!(holds(&sent, Sent::Conversation), Holds::Nothing);
        assert!(sent_opens(&sent, Sent::ToolDefinitions));
        assert!(sent_opens(&sent, Sent::SystemPrompt));
        // Nothing to read in it, or the rows below are what it holds.
        assert!(!sent_opens(&sent, Sent::Rules));
        assert!(!sent_opens(&sent, Sent::SubagentDefinitions));
        assert!(!sent_opens(&sent, Sent::Conversation));
        assert_eq!(tool_line(&tool("x", "")), "");
        // A long text opens cut, saying how much is left out.
        assert_eq!(opened_text("rules\n"), "rules");
        let long = "é".repeat(OPENED_LIMIT + 3);
        let opened = opened_text(&long);
        assert!(opened.ends_with("\n[… 3 more characters]"), "{opened}");
        assert_eq!(
            opened.chars().count(),
            OPENED_LIMIT + "\n[… 3 more characters]".chars().count()
        );
    }

    #[test]
    fn a_row_is_labelled_summarised_and_weighed() {
        let mut tool = row(ModelRowKind::Tool, "\n  400 reads.fq\nmore", 3_000);
        tool.tool = Some("shell".into());
        assert_eq!(label(&tool), "shell");
        assert_eq!(line(&tool), "400 reads.fq");
        assert!(opens(&tool));
        let view = ModelView {
            rows: vec![],
            chars: 12_000,
            compacted: false,
            breakdown: Default::default(),
            epoch: 0,
            sent: None,
        };
        assert_eq!(share(&tool, &view), "≈750 · 25%");

        let hello = row(ModelRowKind::User, "hello", 5);
        assert_eq!(label(&hello), "You");
        // Too small to be a share of anything, but never nothing.
        assert_eq!(share(&hello, &view), "≈2");
        assert!(!opens(&hello));
        // One line too long for a row is opened to be read.
        assert!(opens(&row(ModelRowKind::User, &"long ".repeat(40), 200)));

        let mut asking = row(ModelRowKind::Assistant, "", 40);
        asking.calls = vec!["shell".into(), "read".into()];
        assert_eq!(label(&asking), "Wisp");
        assert_eq!(line(&asking), "Asks for shell, read");
        // Nothing was said: there is nothing more to open.
        assert!(!opens(&asking));
        asking.text = "Counting.".into();
        assert_eq!(line(&asking), "Counting. → shell, read");

        assert_eq!(label(&row(ModelRowKind::System, "x", 1)), "Instructions");
        assert_eq!(label(&row(ModelRowKind::Summary, "x", 1)), "Summary");
        assert_eq!(label(&row(ModelRowKind::Tool, "x", 1)), "Tool");
        // An empty view has no shares to work out.
        assert_eq!(share(&hello, &ModelView::default()), "≈2");
    }
}
