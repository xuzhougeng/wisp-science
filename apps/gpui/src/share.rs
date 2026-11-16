//! A conversation as one HTML page to pass on: your messages and its
//! answers, styled with the WebView's share stylesheet, readable in any
//! browser without Wisp. Answers are Markdown made safe: raw HTML is shown
//! as text and links that could run code are dropped.
use crate::i18n::tr;
use wisp_dto::native_conversations::ShareRow;

/// The WebView's `ui/src/styles/share-export.css`; a test keeps the copy
/// equal to it while the WebView sources exist.
const STYLE: &str = include_str!("../assets/share-export.css");
/// What a page may carry, as the WebView limits it.
const LIMIT: usize = 16 * 1024 * 1024;
/// The dark palette the WebView puts over the stylesheet.
const DARK: &str = ":root{color-scheme:dark;--bg-app:#191817;--bg-elev:#242220;--bg-sunken:#201e1c;--text:#e9e6df;--text-muted:#b2aea3;--text-faint:#979287;--border:#38342f}";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// An answer's Markdown as HTML. With GitHub's extensions and nothing
/// dangerous allowed; if it cannot be read, it is shown as it was written.
fn answer(text: &str) -> String {
    markdown::to_html_with_options(text, &markdown::Options::gfm())
        .unwrap_or_else(|_| format!("<pre>{}</pre>", escape(text)))
}

/// The page, in the window's language and light or dark as the window is.
pub fn html(title: &str, rows: &[ShareRow], dark: bool) -> Result<String, String> {
    if rows.is_empty() {
        return Err(tr("This conversation has nothing to share yet.").into());
    }
    if rows.iter().map(|row| row.text.len()).sum::<usize>() > LIMIT {
        return Err(tr("This conversation is too long to share as one page.").into());
    }
    let title = match title.trim() {
        "" => "Wisp Science",
        title => title,
    };
    let language = match crate::i18n::current() {
        crate::i18n::Locale::Zh => "zh",
        _ => "en",
    };
    let mut page = format!(
        "<!doctype html><html lang=\"{language}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{STYLE}{}</style></head><body><main class=\"share-page\">\
         <header class=\"share-head\"><h1>{}</h1></header><div class=\"thread\">",
        escape(title),
        if dark { DARK } else { "" },
        escape(title),
    );
    for row in rows {
        let (class, label, body) = match row.role.as_str() {
            "user" => (
                "user",
                tr("You"),
                format!(
                    "<div class=\"user-bubble\"><div class=\"body\">{}</div></div>",
                    escape(&row.text)
                ),
            ),
            "assistant" => (
                "assistant",
                "Wisp Science",
                format!(
                    "<div class=\"assistant-wrap\"><div class=\"body md\">{}</div></div>",
                    answer(&row.text)
                ),
            ),
            // Only what was said is shared.
            _ => continue,
        };
        page.push_str(&format!(
            "<article class=\"msg {class}\"><div class=\"role\">{}</div>{body}</article>",
            escape(label)
        ));
    }
    page.push_str(&format!(
        "</div><footer class=\"share-foot\">{}</footer></main></body></html>",
        escape(tr("Shared from Wisp Science"))
    ));
    Ok(page)
}

/// What was said in a conversation, each message ticked to go into its
/// shared page or not, as the WebView's share dialog lists them: one row per
/// message, your messages and the answers alike, every one ticked at first.
#[derive(Clone, Debug, Default)]
pub struct ShareChoice {
    rows: Vec<ShareRow>,
    picked: Vec<bool>,
}

impl ShareChoice {
    /// The whole conversation, as sharing was before there was a choice.
    pub fn new(rows: Vec<ShareRow>) -> Self {
        let picked = vec![true; rows.len()];
        Self { rows, picked }
    }

    pub fn rows(&self) -> &[ShareRow] {
        &self.rows
    }

    pub fn is_picked(&self, index: usize) -> bool {
        self.picked.get(index).copied().unwrap_or(false)
    }

    /// Ticks the message at `index`, or takes its tick away.
    pub fn toggle(&mut self, index: usize) {
        if let Some(picked) = self.picked.get_mut(index) {
            *picked = !*picked;
        }
    }

    /// Select all, or Clear selection.
    pub fn set_all(&mut self, picked: bool) {
        self.picked.fill(picked);
    }

    pub fn count(&self) -> usize {
        self.picked.iter().filter(|picked| **picked).count()
    }

    /// The ticked messages, in the order they were said: what the page shows.
    pub fn chosen(&self) -> Vec<ShareRow> {
        self.rows
            .iter()
            .zip(&self.picked)
            .filter(|(_, picked)| **picked)
            .map(|(row, _)| row.clone())
            .collect()
    }

    /// How many are ticked, of how many.
    pub fn count_line(&self) -> String {
        crate::i18n::fill(tr("{} of {} selected"), &[&self.count(), &self.rows.len()])
    }
}

/// The share dialog's state: which conversation, under what title, the
/// messages chosen, and whether the page is being saved.
#[derive(Clone, Debug)]
pub struct Sharing {
    pub session: String,
    pub title: String,
    pub choice: ShareChoice,
    pub saving: bool,
}

/// Who said a message, as its row and the page name them.
pub fn speaker(row: &ShareRow) -> &'static str {
    match row.role.as_str() {
        "user" => tr("You"),
        _ => "Wisp Science",
    }
}

/// The start of a message on one line, for its row in the share dialog.
pub fn preview(row: &ShareRow) -> String {
    const LIMIT: usize = 160;
    let line = row.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= LIMIT {
        return line;
    }
    let mut short: String = line.chars().take(LIMIT - 1).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

/// The file name a page is offered under: the conversation's export name
/// as HTML.
pub fn file_name(title: &str) -> String {
    let name = crate::organise::export_name(title);
    match name.strip_suffix(".md") {
        Some(stem) => format!("{stem}.html"),
        None => format!("{name}.html"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, text: &str) -> ShareRow {
        ShareRow {
            role: role.into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_page_shows_what_was_said_and_runs_nothing() {
        let page = html(
            "Reads <today>",
            &[
                row("user", "Is <b>this</b> right?"),
                row(
                    "assistant",
                    "**98%** <script>alert(1)</script> [bad](javascript:alert(1)) [ok](https://example.org)\n\n| a | b |\n|---|---|\n| 1 | 2 |",
                ),
                row("tool", "secret"),
            ],
            false,
        )
        .unwrap();
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<title>Reads &lt;today&gt;</title>"));
        assert!(page.contains("Is &lt;b&gt;this&lt;/b&gt; right?"));
        assert!(page.contains("<strong>98%</strong>"));
        assert!(page.contains("<table>"));
        assert!(page.contains("href=\"https://example.org\""));
        assert!(!page.contains("<script>"));
        assert!(!page.contains("javascript:"));
        assert!(!page.contains("secret"));
        assert!(page.contains("share-page"));
        assert!(!page.contains("color-scheme:dark"));
        assert!(
            html("", &[row("user", "hi")], true)
                .unwrap()
                .contains("color-scheme:dark")
        );
        assert!(html("", &[], false).is_err());
    }

    #[test]
    fn every_message_is_shared_until_one_is_unticked() {
        let mut choice = ShareChoice::new(vec![
            row("user", "Align the reads"),
            row("assistant", "Mapped: **98%**."),
            row("user", "And the duplicates?"),
            row("assistant", "12% duplicates."),
        ]);
        // At first the page is the whole conversation, as it was before.
        assert_eq!(choice.count(), 4);
        assert_eq!(choice.chosen().len(), 4);
        assert_eq!(choice.count_line(), "4 of 4 selected");
        // A single message goes, its question or answer stays.
        choice.toggle(1);
        assert!(!choice.is_picked(1) && choice.is_picked(0));
        let said: Vec<_> = choice.chosen().into_iter().map(|row| row.text).collect();
        assert_eq!(
            said,
            ["Align the reads", "And the duplicates?", "12% duplicates."]
        );
        let page = html("Reads", &choice.chosen(), false).unwrap();
        assert!(!page.contains("98%") && page.contains("12% duplicates."));
        // A row that is not there changes nothing.
        choice.toggle(9);
        assert!(!choice.is_picked(9));
        assert_eq!(choice.count_line(), "3 of 4 selected");
        // Clear selection leaves nothing to share; Select all brings all back.
        choice.set_all(false);
        assert_eq!(choice.count(), 0);
        assert!(html("Reads", &choice.chosen(), false).is_err());
        choice.set_all(true);
        assert_eq!(choice.chosen().len(), choice.rows().len());
        choice.toggle(1);
        choice.toggle(1);
        assert!(choice.is_picked(1));
    }

    #[test]
    fn a_row_names_who_spoke_and_begins_the_message_on_one_line() {
        assert_eq!(speaker(&row("user", "x")), "You");
        assert_eq!(speaker(&row("assistant", "x")), "Wisp Science");
        assert_eq!(preview(&row("user", "  one\n\ntwo  ")), "one two");
        let long = preview(&row("assistant", &"序列 ".repeat(200)));
        assert!(long.chars().count() <= 160);
        assert!(long.ends_with('…') && !long.ends_with(" …"));
    }

    #[test]
    fn a_page_is_offered_as_an_html_file() {
        assert_eq!(file_name("Read: counts"), "Read- counts.html");
        assert_eq!(file_name(""), "conversation.html");
    }

    /// While the WebView sources exist, both frontends share one look.
    #[test]
    fn the_stylesheet_is_the_webviews() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/src/styles/share-export.css");
        let Ok(source) = std::fs::read_to_string(source) else {
            return;
        };
        assert_eq!(STYLE, source);
    }
}
