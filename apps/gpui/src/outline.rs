//! The outline of the open conversation: what the user asked, in order, as
//! places to jump to. The part of the conversation that is loaded is
//! listed; the rest can be loaded from the outline.
use wisp_dto::native_conversations::Item;

/// How much of a message's first line an entry shows.
const WIDTH: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The transcript row to jump to.
    pub row: usize,
    /// Which of the user's messages it is, from 1.
    pub number: usize,
    pub title: String,
}

/// One entry for each of the user's messages in `rows`. A message that is
/// only attached files is named after them.
pub fn entries(rows: &[Item]) -> Vec<Entry> {
    rows.iter()
        .enumerate()
        .filter(|(_, item)| item.role == "user")
        .enumerate()
        .map(|(number, (row, item))| Entry {
            row,
            number: number + 1,
            title: title(item),
        })
        .collect()
}

/// The page to ask for after the one before `asked`: the host's next one,
/// as long as it is further back, so a walk through the pages ends.
pub fn next_page(asked: i64, next: Option<i64>) -> Option<i64> {
    next.filter(|next| *next < asked)
}

/// Earlier pages, newest first as they were asked for, put in front of the
/// rows already loaded, oldest first.
pub fn stitch(pages: Vec<Vec<Item>>, loaded: Vec<Item>) -> Vec<Item> {
    let mut rows: Vec<Item> = pages.into_iter().rev().flatten().collect();
    rows.extend(loaded);
    rows
}

fn title(item: &Item) -> String {
    let text = crate::compose::user_text(item);
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.is_empty() {
        return match item.attachments.len() {
            0 => crate::i18n::tr("(empty message)").into(),
            n => crate::files::count(n),
        };
    }
    if line.chars().count() <= WIDTH {
        line.to_owned()
    } else {
        let cut: String = line.chars().take(WIDTH - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn earlier_pages_go_in_front_oldest_first_and_the_walk_ends() {
        let texts = |rows: &[Item]| rows.iter().map(|row| row.text.clone()).collect::<Vec<_>>();
        let rows = stitch(
            vec![
                vec![row("user", "3", &[]), row("assistant", "c", &[])],
                vec![row("user", "1", &[]), row("user", "2", &[])],
            ],
            vec![row("user", "4", &[])],
        );
        assert_eq!(texts(&rows), ["1", "2", "3", "c", "4"]);
        assert_eq!(next_page(40, Some(20)), Some(20));
        assert_eq!(next_page(40, None), None);
        // A host that answers with the same place again does not loop.
        assert_eq!(next_page(40, Some(40)), None);
        assert_eq!(next_page(40, Some(41)), None);
    }

    fn row(role: &str, text: &str, attachments: &[&str]) -> Item {
        serde_json::from_value(json!({
            "role": role, "text": text, "tool_name": null, "input": null,
            "ok": null, "status": null, "attachments": attachments
        }))
        .unwrap()
    }

    #[test]
    fn the_outline_lists_what_the_user_asked_in_order() {
        let long = "word ".repeat(40);
        let rows = [
            row("user", "\n  Align the reads\nagainst GRCh38", &[]),
            row("assistant", "Done.", &[]),
            row("tool", "{}", &[]),
            row("user", &long, &[]),
            row("user", "Uploaded files: a.csv, b.csv", &["a.csv", "b.csv"]),
            row("user", "   ", &[]),
        ];
        let outline = entries(&rows);
        assert_eq!(
            outline
                .iter()
                .map(|e| (e.row, e.number))
                .collect::<Vec<_>>(),
            [(0, 1), (3, 2), (4, 3), (5, 4)]
        );
        // The first line that says something, without its indent.
        assert_eq!(outline[0].title, "Align the reads");
        // A long line is cut to fit and says so.
        assert_eq!(outline[1].title.chars().count(), WIDTH);
        assert!(outline[1].title.ends_with('…'));
        // Files alone are named as files; nothing at all is said to be so.
        assert_eq!(outline[2].title, "2 files");
        assert_eq!(outline[3].title, "(empty message)");
        assert!(entries(&[row("assistant", "hi", &[])]).is_empty());
    }
}
