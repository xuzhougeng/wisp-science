//! A conversation as a Markdown document, for the researcher to keep or
//! pass on outside Wisp.

use wisp_dto::native_conversations::{Item, ShareRow};

/// How much of one tool's input or output the document carries.
const TOOL_LIMIT: usize = 4_000;

/// A fence longer than any run of backticks in `text`, so the text cannot
/// end the block early.
fn fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat(longest.max(2) + 1)
}

fn clipped(text: &str) -> (&str, bool) {
    if text.len() <= TOOL_LIMIT {
        return (text, false);
    }
    let mut end = TOOL_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn block(out: &mut String, text: &str, language: &str) {
    let (text, cut) = clipped(text.trim_end());
    let fence = fence(text);
    out.push_str(&format!("{fence}{language}\n{text}\n{fence}\n"));
    if cut {
        out.push_str("\n*(cut here; the full text is in Wisp)*\n");
    }
}

/// The conversation `title`, with `rows` in order, as Markdown. What the
/// user and the model said is kept whole; a tool call is named with what
/// it was given and what it returned, each cut to a few thousand bytes.
pub(crate) fn markdown(title: &str, project: &str, rows: &[Item]) -> String {
    let title = match title.trim() {
        "" => "Conversation",
        title => title,
    };
    let mut out = format!("# {title}\n\nProject: {project}\n");
    for row in rows {
        // A turn's answer delivered through `attempt_completion` reads as
        // Wisp's answer, not as a tool call.
        if row.is_completion_answer() {
            out.push_str("\n## Wisp\n\n");
            out.push_str(&crate::citations::file_citations_as_links(row.text.trim()));
            out.push('\n');
            continue;
        }
        match row.role.as_str() {
            "user" => {
                out.push_str("\n## You\n\n");
                let text = row.text.trim();
                out.push_str(if text.is_empty() { "*(no text)*" } else { text });
                out.push('\n');
            }
            "assistant" => {
                if row.text.trim().is_empty() {
                    continue;
                }
                out.push_str("\n## Wisp\n\n");
                // Codex's file citations as links (`citations`).
                out.push_str(&crate::citations::file_citations_as_links(row.text.trim()));
                out.push('\n');
            }
            "tool" => {
                let name = row.tool_name.as_deref().unwrap_or("tool");
                let outcome = match row.ok {
                    Some(true) => " — done",
                    Some(false) => " — failed",
                    None => "",
                };
                out.push_str(&format!("\n**Tool `{name}`{outcome}**\n\n"));
                if let Some(input) = row
                    .input
                    .as_deref()
                    .filter(|input| !input.trim().is_empty())
                {
                    block(&mut out, input, "json");
                }
                if !row.text.trim().is_empty() {
                    out.push('\n');
                    block(&mut out, &row.text, "");
                }
            }
            _ => {}
        }
    }
    out
}

/// What was said, for a page to share: each message and each answer with
/// text, oldest first, without the tools between them.
pub(crate) fn share_rows(rows: &[Item]) -> Vec<ShareRow> {
    rows.iter()
        .filter(|row| {
            matches!(row.role.as_str(), "user" | "assistant") || row.is_completion_answer()
        })
        .filter(|row| !row.text.trim().is_empty())
        .map(|row| ShareRow {
            // The answer a turn delivered through its completion is Wisp's.
            role: if row.is_completion_answer() {
                "assistant".to_owned()
            } else {
                row.role.clone()
            },
            text: crate::citations::file_citations_as_links(row.text.trim()).into_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::item;

    #[test]
    fn a_completions_answer_is_wisps_answer_in_the_document_and_the_page() {
        let mut done = item("tool", "| a | b |\n|---|---|\n| 1 | 2 |");
        done.tool_name = Some("attempt_completion".into());
        done.status = Some("complete".into());
        let mut failed = item(
            "tool",
            "attempt_completion error: 'result' must contain text",
        );
        failed.tool_name = Some("attempt_completion".into());
        failed.status = Some("complete".into());
        let rows = [item("user", "Count them"), done, failed];
        let page = share_rows(&rows);
        assert_eq!(page.len(), 2);
        assert_eq!(page[1].role, "assistant");
        assert!(page[1].text.starts_with("| a | b |"));
        let document = markdown("t", "p", &rows);
        assert!(document.contains("## Wisp\n\n| a | b |"));
        assert!(document.contains("**Tool `attempt_completion`"));
    }

    #[test]
    fn codexs_file_citations_are_links_in_the_document_and_the_page() {
        let said = r#"Wrote :codex-file-citation{path="E:/r/panel_index.csv" purpose="output"}"#;
        let rows = [item("user", "Go"), item("assistant", said)];
        let link = "Wrote [panel_index.csv](<file:///E:/r/panel_index.csv>)";
        assert!(markdown("t", "p", &rows).contains(link));
        assert_eq!(share_rows(&rows)[1].text, link);
    }

    #[test]
    fn a_shared_page_has_what_was_said_and_no_tools() {
        let mut call = item("tool", "secret output");
        call.tool_name = Some("shell".into());
        let rows = share_rows(&[
            item("user", " Align the reads "),
            item("assistant", ""),
            call,
            item("assistant", "Mapped: **98%**."),
            item("user", "  "),
        ]);
        let said: Vec<_> = rows
            .iter()
            .map(|row| (row.role.as_str(), row.text.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("user", "Align the reads"),
                ("assistant", "Mapped: **98%**.")
            ]
        );
    }

    #[test]
    fn a_conversation_reads_as_a_document_and_tools_cannot_break_it() {
        let mut call = item("tool", "a ``` fence in the output\nand more");
        call.tool_name = Some("shell".into());
        call.input = Some("{\"cmd\":\"ls\"}".into());
        call.ok = Some(false);
        let mut long = item("tool", &"x".repeat(TOOL_LIMIT + 10));
        long.tool_name = Some("read".into());
        let rows = [
            item("user", " Align the reads \n"),
            item("assistant", ""),
            call,
            item("assistant", "The mapping rate is **98%**."),
            long,
            item("user", ""),
        ];
        let text = markdown("  Alignment ", "Screens", &rows);
        assert!(text.starts_with("# Alignment\n\nProject: Screens\n\n## You\n\nAlign the reads\n"));
        // An empty answer is not a section; an empty message says so.
        assert_eq!(text.matches("## Wisp").count(), 1);
        assert!(text.ends_with("## You\n\n*(no text)*\n"));
        // The tool is named with how it ended, and its output sits in a
        // fence longer than the one inside it.
        assert!(text.contains("**Tool `shell` — failed**"));
        assert!(text.contains("```json\n{\"cmd\":\"ls\"}\n```"));
        assert!(text.contains("````\na ``` fence in the output\nand more\n````"));
        // Long output is cut and says so; what people said never is.
        assert!(text.contains("**Tool `read`**"));
        assert!(text.contains("*(cut here; the full text is in Wisp)*"));
        assert!(!text.contains(&"x".repeat(TOOL_LIMIT + 1)));
        assert!(markdown("", "P", &[]).starts_with("# Conversation\n"));
    }
}
