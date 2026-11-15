//! What the model is sent, as its dialog words it. The host reads the
//! conversation's saved working set; this only says what is in it.
use crate::i18n::tr;
use wisp_server::desktop::{ModelRowKind, ModelView, ModelViewRow};

use crate::controls::compact;

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

#[cfg(test)]
mod tests {
    use super::*;

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
