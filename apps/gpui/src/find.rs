//! Finding text in the open conversation: which rows of the transcript
//! contain what was typed, and which of them is the current one. Only the
//! part of the conversation that is loaded is searched.
use wisp_dto::native_conversations::Item;

#[derive(Default)]
pub struct Find {
    query: String,
    /// Row indices that match, in transcript order, each with the row's
    /// own number in the saved conversation when it has one.
    hits: Vec<(usize, Option<i64>)>,
    /// Which of `hits` is the current one.
    at: usize,
}

fn matches(item: &Item, needle: &str) -> bool {
    // A kept review's row holds its report as data, not words to find, and
    // so does the line where the conversation was compacted.
    if item.role == "review" || item.role == "compaction" {
        return false;
    }
    item.text.to_lowercase().contains(needle)
        || item
            .tool_name
            .as_deref()
            .is_some_and(|name| name.to_lowercase().contains(needle))
}

impl Find {
    /// Looks for `query` in `rows`, ignoring case. A new search starts at
    /// the newest match, which is where the reader is; searching again for
    /// the same words (the transcript grew at either end) keeps the current
    /// message if it still matches.
    pub fn search(&mut self, rows: &[Item], query: &str) {
        let query = query.trim();
        let kept = (query == self.query)
            .then(|| self.hits.get(self.at).copied())
            .flatten();
        self.query = query.to_owned();
        let needle = query.to_lowercase();
        self.hits = if needle.is_empty() {
            Vec::new()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(_, item)| matches(item, &needle))
                .map(|(index, item)| (index, item.seq))
                .collect()
        };
        // A saved message is the same message wherever it now stands; one
        // that is still being written has only its place.
        let same = |hit: &(usize, Option<i64>)| match kept {
            Some((_, Some(seq))) => hit.1 == Some(seq),
            Some((row, None)) => hit.0 == row && hit.1.is_none(),
            None => false,
        };
        self.at = self
            .hits
            .iter()
            .position(same)
            .unwrap_or(self.hits.len().saturating_sub(1));
    }

    /// Searches again for the same words after the transcript changed.
    pub fn refresh(&mut self, rows: &[Item]) {
        let query = self.query.clone();
        self.search(rows, &query);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The row to show, if anything matches.
    pub fn current(&self) -> Option<usize> {
        self.hits.get(self.at).map(|hit| hit.0)
    }

    pub fn is_hit(&self, row: usize) -> bool {
        self.hits.binary_search_by_key(&row, |hit| hit.0).is_ok()
    }

    /// Moves to the match before or after the current one and wraps at
    /// the ends. Returns the row to show.
    pub fn step(&mut self, older: bool) -> Option<usize> {
        let count = self.hits.len();
        if count == 0 {
            return None;
        }
        self.at = if older {
            (self.at + count - 1) % count
        } else {
            (self.at + 1) % count
        };
        self.current()
    }

    /// What the bar says beside the field.
    pub fn label(&self) -> String {
        match (self.query.is_empty(), self.hits.len()) {
            (true, _) => String::new(),
            (false, 0) => "No matches".into(),
            (false, count) => format!("{} of {count}", self.at + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row; `seq` is its number in the saved conversation, if saved.
    fn row(role: &str, text: &str, tool: Option<&str>, seq: Option<i64>) -> Item {
        serde_json::from_value(serde_json::json!({
            "role": role, "text": text, "tool_name": tool, "input": null,
            "ok": null, "status": null, "seq": seq
        }))
        .unwrap()
    }

    #[test]
    fn a_kept_reviews_report_is_not_searched() {
        let mut find = Find::default();
        let rows = vec![
            row("assistant", "The reads were mapped.", None, Some(1)),
            row("review", "{\"summary\":\"reads checked\"}", None, Some(1)),
            row(
                "compaction",
                "{\"strategy\":\"manual\",\"checkpoint\":\"reads were mapped\"}",
                None,
                Some(4),
            ),
        ];
        find.search(&rows, "reads");
        assert_eq!(find.hits.len(), 1);
        find.search(&rows, "summary");
        assert!(find.hits.is_empty());
        find.search(&rows, "manual");
        assert!(find.hits.is_empty());
    }

    fn rows() -> Vec<Item> {
        vec![
            row("user", "Align the reads against GRCh38", None, Some(10)),
            row(
                "tool",
                "{\"stdout\":\"mapped 98%\"}",
                Some("shell"),
                Some(11),
            ),
            row("assistant", "The mapping rate is 98%.", None, Some(12)),
            row("user", "now count reads per gene", None, Some(13)),
            row(
                "assistant",
                "Counting READS with featureCounts.",
                None,
                None,
            ),
        ]
    }

    #[test]
    fn a_search_starts_at_the_newest_match_and_steps_around() {
        let rows = rows();
        let mut find = Find::default();
        assert_eq!((find.current(), find.label()), (None, String::new()));

        find.search(&rows, "  Reads ");
        // Case does not matter; the newest match is the current one.
        assert_eq!(find.current(), Some(4));
        assert_eq!(find.label(), "3 of 3");
        assert!(find.is_hit(0) && find.is_hit(3) && !find.is_hit(2));
        // Older, older, and around to the newest again.
        assert_eq!(find.step(true), Some(3));
        assert_eq!(find.step(true), Some(0));
        assert_eq!(find.label(), "1 of 3");
        assert_eq!(find.step(true), Some(4));
        assert_eq!(find.step(false), Some(0));

        // What a tool printed is found, and so is a tool by its name.
        find.search(&rows, "98%");
        assert_eq!(find.label(), "2 of 2");
        find.search(&rows, "SHELL");
        assert_eq!(find.current(), Some(1));

        find.search(&rows, "nothing like this");
        assert_eq!((find.current(), find.step(true)), (None, None));
        assert_eq!(find.label(), "No matches");
        find.search(&rows, "   ");
        assert_eq!(find.label(), "");
        assert!(!find.is_hit(0));
    }

    #[test]
    fn the_place_is_kept_while_the_conversation_grows() {
        let mut rows = rows();
        let mut find = Find::default();
        find.search(&rows, "reads");
        find.step(true);
        assert_eq!(find.current(), Some(3));
        // A turn adds a row that matches too: the reader stays where they are.
        rows.push(row("assistant", "All reads are counted.", None, None));
        find.refresh(&rows);
        assert_eq!(find.current(), Some(3));
        assert_eq!(find.label(), "2 of 4");
        // Earlier messages are loaded above: the same message, a new index.
        rows.insert(
            0,
            row("user", "an older message about reads", None, Some(9)),
        );
        find.refresh(&rows);
        assert_eq!(find.current(), Some(4));
        assert_eq!(find.label(), "3 of 5");
        // The message that was current is gone (a rewind): start at the
        // newest that is left.
        rows.truncate(2);
        find.refresh(&rows);
        assert_eq!(find.current(), Some(1));
        assert_eq!(find.label(), "2 of 2");
        // A message still being written is kept by its place.
        rows.push(row("assistant", "reads, streaming", None, None));
        rows.push(row("assistant", "more reads", None, None));
        find.refresh(&rows);
        find.step(false);
        assert_eq!(find.current(), Some(2));
        rows.push(row("assistant", "and more reads", None, None));
        find.refresh(&rows);
        assert_eq!(find.current(), Some(2));

        find.clear();
        assert_eq!((find.current(), find.label()), (None, String::new()));
    }
}
