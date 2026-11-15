//! The notebook of the open conversation: the code in it, in order, as
//! cells. It is a reading of the transcript, not something saved on its
//! own: code the model ran with `python`, `r` or `shell`, with what came
//! back, and code it only showed in an answer.
use crate::i18n::tr;
use wisp_dto::native_conversations::Item;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// From 1, in transcript order.
    pub number: usize,
    pub language: String,
    pub source: String,
    /// What the tool returned; empty for code that was only shown.
    pub output: String,
    /// The code was run by a tool, not only written in an answer.
    pub ran: bool,
    /// The transcript row it comes from.
    pub row: usize,
}

/// Fenced code blocks of a Markdown text: language and body.
fn fences(text: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut open: Option<(String, String, Vec<&str>)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        match &mut open {
            None => {
                let ticks = trimmed.chars().take_while(|c| *c == '`').count();
                if ticks >= 3 {
                    let language = trimmed[ticks..].trim().to_lowercase();
                    open = Some(("`".repeat(ticks), language, Vec::new()));
                }
            }
            Some((fence, language, lines)) => {
                // A block ends at a line of at least as many backticks.
                if trimmed.starts_with(fence.as_str())
                    && trimmed.trim_matches('`').trim().is_empty()
                {
                    blocks.push((std::mem::take(language), lines.join("\n")));
                    open = None;
                } else {
                    lines.push(line);
                }
            }
        }
    }
    blocks
}

/// The code a tool call ran, read from its arguments.
fn tool_source(tool: &str, input: &str) -> Option<(&'static str, String)> {
    let arguments: serde_json::Value = serde_json::from_str(input).ok()?;
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let (language, source) = match tool {
        "python" => (
            "python",
            text("code")
                .map(str::to_owned)
                .or_else(|| text("script_path").map(|path| format!("# runs the script {path}"))),
        ),
        "r" => (
            "r",
            text("code")
                .map(str::to_owned)
                .or_else(|| text("script_path").map(|path| format!("# runs the script {path}"))),
        ),
        "shell" => ("bash", text("cmd").map(str::to_owned)),
        _ => return None,
    };
    Some((language, source?))
}

/// The cells of a transcript. Data shown in a fence (CSV, TSV, FASTA) is
/// not code. Code that an answer shows and a tool then runs is one cell,
/// the run; a run that is repeated on purpose stays as often as it ran.
pub fn cells(rows: &[Item]) -> Vec<Cell> {
    let mut found: Vec<Cell> = Vec::new();
    for (row, item) in rows.iter().enumerate() {
        match item.role.as_str() {
            "assistant" => {
                for (language, source) in fences(&item.text) {
                    if matches!(language.as_str(), "csv" | "tsv" | "fasta" | "fa")
                        || source.trim().is_empty()
                    {
                        continue;
                    }
                    found.push(Cell {
                        number: 0,
                        language: if language.is_empty() {
                            "text".into()
                        } else {
                            language
                        },
                        source,
                        output: String::new(),
                        ran: false,
                        row,
                    });
                }
            }
            "tool" => {
                let tool = item.tool_name.as_deref().unwrap_or_default();
                let input = item.input.as_deref().unwrap_or_default();
                if let Some((language, source)) = tool_source(tool, input) {
                    found.push(Cell {
                        number: 0,
                        language: language.into(),
                        source,
                        output: item.text.clone(),
                        ran: true,
                        row,
                    });
                }
            }
            _ => {}
        }
    }
    let ran: std::collections::HashSet<String> = found
        .iter()
        .filter(|cell| cell.ran)
        .map(|cell| cell.source.trim().to_owned())
        .collect();
    found.retain(|cell| cell.ran || !ran.contains(cell.source.trim()));
    for (index, cell) in found.iter_mut().enumerate() {
        cell.number = index + 1;
    }
    found
}

/// What a cell's output shows in the panel: at most `lines` lines and
/// `bytes` bytes of it, and whether more was left out.
pub fn output_preview(output: &str, lines: usize, bytes: usize) -> (String, bool) {
    let trimmed = output.trim_end();
    let mut shown = String::new();
    let mut cut = false;
    for (index, line) in trimmed.lines().enumerate() {
        if index >= lines || shown.len() + line.len() > bytes {
            cut = true;
            break;
        }
        if index > 0 {
            shown.push('\n');
        }
        shown.push_str(line);
    }
    (shown, cut)
}

/// One line saying what the notebook holds.
pub fn summary(cells: &[Cell]) -> String {
    let ran = cells.iter().filter(|cell| cell.ran).count();
    match (cells.len(), ran) {
        (0, _) => tr("No code in this conversation yet.").into(),
        (1, 1) => tr("1 cell, run").into(),
        (1, _) => tr("1 cell, shown but not run").into(),
        (n, ran) if ran == n => crate::i18n::fill(tr("{} cells, all run"), &[&n]),
        (n, ran) => crate::i18n::fill(tr("{} cells, {} run"), &[&n, &ran]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(role: &str, tool: Option<&str>, input: Option<serde_json::Value>, text: &str) -> Item {
        serde_json::from_value(json!({
            "role": role, "text": text, "tool_name": tool,
            "input": input.map(|value| value.to_string()), "ok": null, "status": null
        }))
        .unwrap()
    }

    #[test]
    fn code_that_ran_and_code_that_was_shown_become_cells_in_order() {
        let rows = [
            row("user", None, None, "```python\nnot the model's\n```"),
            row(
                "assistant",
                None,
                None,
                "Plan:\n```python\nimport pandas as pd\n```\nand the data:\n```csv\na,b\n1,2\n```\n\
                 then\n````\nplain ``` inside\n````",
            ),
            row(
                "tool",
                Some("python"),
                Some(json!({"code": "import pandas as pd\n", "context_id": "local"})),
                "ok",
            ),
            row(
                "tool",
                Some("shell"),
                Some(json!({"cmd": "ls -la"})),
                "a.txt\nb.txt\n",
            ),
            row(
                "tool",
                Some("r"),
                Some(json!({"script_path": "fit.R"})),
                "done",
            ),
            row(
                "tool",
                Some("read"),
                Some(json!({"path": "a.txt"})),
                "contents",
            ),
            row(
                "tool",
                Some("shell"),
                Some(json!({"cmd": "ls -la"})),
                "a.txt\n",
            ),
            row("tool", Some("python"), None, "no arguments"),
        ];
        let cells = cells(&rows);
        let seen: Vec<_> = cells
            .iter()
            .map(|cell| {
                (
                    cell.number,
                    cell.language.as_str(),
                    cell.source.as_str(),
                    cell.ran,
                    cell.row,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                // The fence the model showed and then ran is the run alone;
                // the data fence is not code; an unnamed fence is text.
                (1, "text", "plain ``` inside", false, 1),
                (2, "python", "import pandas as pd", true, 2),
                (3, "bash", "ls -la", true, 3),
                (4, "r", "# runs the script fit.R", true, 4),
                // Run again on purpose: it is there again.
                (5, "bash", "ls -la", true, 6),
            ]
        );
        assert_eq!(cells[2].output, "a.txt\nb.txt\n");
        assert_eq!(cells[0].output, "");
        assert_eq!(summary(&cells), "5 cells, 4 run");
        assert_eq!(summary(&[]), "No code in this conversation yet.");
        assert_eq!(summary(&cells[1..2]), "1 cell, run");
        assert_eq!(summary(&cells[..1]), "1 cell, shown but not run");
        assert_eq!(summary(&cells[1..]), "4 cells, all run");
    }

    #[test]
    fn long_output_is_cut_and_says_so() {
        assert_eq!(
            output_preview("a\nb\n\n", 5, 100),
            ("a\nb".to_string(), false)
        );
        assert_eq!(
            output_preview("a\nb\nc\nd", 2, 100),
            ("a\nb".to_string(), true)
        );
        // A line that would go past the limit is left out whole.
        assert_eq!(
            output_preview("short\na much longer line", 5, 10),
            ("short".to_string(), true)
        );
        assert_eq!(output_preview("", 5, 100), (String::new(), false));
    }
}
