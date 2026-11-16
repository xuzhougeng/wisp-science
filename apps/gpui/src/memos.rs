//! Wording and small rules for the project's memos: the researcher's own
//! notes, which reach a conversation only by being put into a message. No
//! graphics state.
use wisp_server::desktop::ProjectMemo;

const HEADING: usize = 80;
const PREVIEW: usize = 220;

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(limit - 1).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

/// What a memo is called in the list: its title, or failing that its first
/// line.
pub fn heading(memo: &ProjectMemo) -> String {
    let title = memo.title.trim();
    let line = if title.is_empty() {
        memo.body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
    } else {
        title
    };
    clip(line, HEADING)
}

/// The start of a memo's text under its heading, on one line. Empty when
/// the heading already says all of it.
pub fn preview(memo: &ProjectMemo) -> String {
    let mut lines = memo
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if memo.title.trim().is_empty() {
        // The first line is the heading.
        lines.next();
    }
    clip(&lines.collect::<Vec<_>>().join(" "), PREVIEW)
}

/// The message box after a memo is put into it: after what is already
/// written, apart from it by an empty line.
pub fn insert(draft: &str, body: &str) -> String {
    let draft = draft.trim_end();
    if draft.is_empty() {
        body.to_owned()
    } else {
        format!("{draft}\n\n{body}")
    }
}

/// Whether the editor holds something worth saving. The host has the last
/// word on lengths.
pub fn can_save(body: &str) -> bool {
    !body.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(title: &str, body: &str) -> ProjectMemo {
        ProjectMemo {
            id: "m".into(),
            project_id: "p".into(),
            title: title.into(),
            body: body.into(),
            pinned: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_memo_is_called_by_its_title_or_its_first_line() {
        let titled = memo("Server", "ssh lab-gpu\n-p 2222");
        assert_eq!(heading(&titled), "Server");
        assert_eq!(preview(&titled), "ssh lab-gpu -p 2222");

        let untitled = memo("  ", "\n  ask about controls  \n\nthen plot\n");
        assert_eq!(heading(&untitled), "ask about controls");
        assert_eq!(preview(&untitled), "then plot");

        // One line and no title: the heading is all there is.
        let short = memo("", "call Li");
        assert_eq!(heading(&short), "call Li");
        assert_eq!(preview(&short), "");
    }

    #[test]
    fn long_text_is_cut_on_characters_not_bytes() {
        let long = memo(&"基因".repeat(60), &"序列 ".repeat(200));
        let name = heading(&long);
        assert_eq!(name.chars().count(), HEADING);
        assert!(name.ends_with('…'));
        let start = preview(&long);
        assert!(start.chars().count() <= PREVIEW);
        assert!(start.ends_with('…') && !start.ends_with(" …"));
    }

    #[test]
    fn a_memo_goes_after_what_is_already_written() {
        assert_eq!(insert("", "note"), "note");
        assert_eq!(insert("  \n", "note"), "note");
        assert_eq!(
            insert("Compare A and B.\n", "note\nline"),
            "Compare A and B.\n\nnote\nline"
        );
    }

    #[test]
    fn an_empty_memo_cannot_be_saved() {
        assert!(!can_save(" \n "));
        assert!(can_save(" x "));
    }
}
