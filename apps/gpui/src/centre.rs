//! Project files opened in the centre beside a conversation, as the
//! WebView's centre tabs (`center_files` and `center_file` in
//! `ui/src/main.rs`). Each conversation has its own documents, in the order
//! they were opened, and the centre shows either the conversation or one of
//! them. They are kept per conversation while the window is open, as the
//! WebView keeps them (`center_tabs_by_session`), and not saved: a new
//! window starts with none.
//!
//! The split (`center_split`) puts the conversation beside the document
//! shown. It is one switch for the window, and the conversation's width,
//! once the divider is dragged, is kept until the window closes, as the
//! WebView keeps `center_chat_w`. The widths are the WebView's own
//! (`CENTER_CHAT_MIN_WIDTH`, `CENTER_DOCUMENT_MIN_WIDTH`,
//! `PANE_RESIZER_WIDTH` and the 38% in `.center.split`). Holds no graphics
//! state and is exercised without a window.
use std::collections::HashMap;

/// The conversation's column in the split is never narrower than this.
pub const CHAT_MIN: f32 = 320.;
/// Dragging the divider leaves the document at least this wide.
pub const DOCUMENT_MIN: f32 = 240.;
/// The divider between the document and the conversation.
pub const DIVIDER: f32 = 5.;
/// The conversation's share of the centre until the divider is dragged.
pub const CHAT_SHARE: f32 = 0.38;
/// The narrowest the centre goes while it is split: the side panel gives
/// way before the two do.
pub const SPLIT_MIN: f32 = CHAT_MIN + DIVIDER + DOCUMENT_MIN;

/// One document's tab: a project file, by its project path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub path: String,
}

impl Document {
    /// What its tab says: the file's name.
    pub fn name(&self) -> &str {
        crate::files::file_name(&self.path)
    }
}

/// One conversation's documents and which of them, if any, is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Documents {
    tabs: Vec<Document>,
    /// `None` shows the conversation.
    shown: Option<String>,
}

impl Documents {
    pub fn tabs(&self) -> &[Document] {
        &self.tabs
    }

    /// The document shown in place of the conversation, if one is.
    pub fn shown(&self) -> Option<&str> {
        self.shown.as_deref()
    }

    /// Shows `path`, with a tab of its own after the others the first
    /// time; a file already open goes to its tab and gets no second one.
    pub fn open(&mut self, path: &str) {
        if !self.tabs.iter().any(|tab| tab.path == path) {
            self.tabs.push(Document {
                path: path.to_owned(),
            });
        }
        self.shown = Some(path.to_owned());
    }

    /// Shows the document at `path` when it has a tab, or the conversation
    /// for `None`. Returns whether that changed what is shown.
    pub fn show(&mut self, path: Option<&str>) -> bool {
        if path.is_some_and(|path| !self.tabs.iter().any(|tab| tab.path == path)) {
            return false;
        }
        let changed = self.shown.as_deref() != path;
        self.shown = path.map(str::to_owned);
        changed
    }

    /// Closes the tab of `path`. The conversation comes back when it was
    /// the one shown, as in the WebView. Returns whether it was shown.
    pub fn close(&mut self, path: &str) -> bool {
        self.tabs.retain(|tab| tab.path != path);
        let shown = self.shown.as_deref() == Some(path);
        if shown {
            self.shown = None;
        }
        shown
    }

    /// Closes the tabs to the right of `path`. When the one shown was among
    /// them, `path` is shown instead. Returns whether what is shown changed.
    pub fn close_right(&mut self, path: &str) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.path == path) else {
            return false;
        };
        self.tabs.truncate(index + 1);
        let kept = self
            .shown
            .as_deref()
            .is_none_or(|shown| self.tabs.iter().any(|tab| tab.path == shown));
        if !kept {
            self.shown = Some(path.to_owned());
        }
        !kept
    }

    /// Closes every tab, and the conversation is shown.
    pub fn close_all(&mut self) {
        self.tabs.clear();
        self.shown = None;
    }

    /// Follows a file or folder of the project that was renamed: the tabs
    /// of the file, or of the files under the folder, take the new path.
    pub fn renamed(&mut self, from: &str, to: &str) {
        let moved = |path: &str| -> Option<String> {
            if path == from {
                return Some(to.to_owned());
            }
            path.strip_prefix(from)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(|rest| format!("{to}/{rest}"))
        };
        for tab in &mut self.tabs {
            if let Some(path) = moved(&tab.path) {
                tab.path = path;
            }
        }
        // A file renamed onto one already open keeps one tab.
        let mut seen = std::collections::HashSet::new();
        self.tabs.retain(|tab| seen.insert(tab.path.clone()));
        if let Some(path) = self.shown.as_deref().and_then(moved) {
            self.shown = Some(path);
        }
    }
}

/// The documents of every conversation the window has opened some in, and
/// the split.
#[derive(Clone, Debug, Default)]
pub struct Centre {
    conversations: HashMap<String, Documents>,
    /// The conversation sits beside the document shown, instead of the
    /// document taking its place.
    pub split: bool,
    /// The conversation's width in the split once its divider was dragged;
    /// `None` is its share of the centre.
    pub chat_width: Option<f32>,
}

impl Centre {
    /// The documents of `session`, if it has any.
    pub fn of(&self, session: &str) -> Option<&Documents> {
        self.conversations.get(session)
    }

    pub fn of_mut(&mut self, session: &str) -> &mut Documents {
        self.conversations.entry(session.to_owned()).or_default()
    }

    /// The document shown in the centre of `session`, if one is.
    pub fn shown(&self, session: Option<&str>) -> Option<&str> {
        self.of(session?)?.shown()
    }

    /// Whether `session` is drawn split: the switch is on and a document
    /// is shown, as the WebView's `center_split_on`.
    pub fn split_shown(&self, session: Option<&str>) -> bool {
        self.split && self.shown(session).is_some()
    }

    /// Follows a renamed file or folder in every conversation's tabs.
    pub fn renamed(&mut self, from: &str, to: &str) {
        for documents in self.conversations.values_mut() {
            documents.renamed(from, to);
        }
    }
}

/// The conversation's width when the divider is dragged to `pointer` in a
/// centre `width` wide that starts at `left`: what lies right of the
/// divider, never under `CHAT_MIN` and never so wide that the document
/// gets less than `DOCUMENT_MIN`. Where the centre has room for neither,
/// the conversation keeps `CHAT_MIN`.
pub fn dragged_chat_width(left: f32, width: f32, pointer: f32) -> f32 {
    let most = (width - DOCUMENT_MIN - DIVIDER).max(CHAT_MIN);
    (left + width - pointer - DIVIDER / 2.).clamp(CHAT_MIN, most)
}

/// The document's and the conversation's widths in a split centre `width`
/// wide, as the WebView's grid lays them out: the conversation takes its
/// dragged width or its share, at least `CHAT_MIN`, and the document the
/// rest, down to nothing. So at narrow widths the document gives way
/// first, and the conversation keeps a column to read in.
pub fn split_widths(width: f32, chat_width: Option<f32>) -> (f32, f32) {
    let chat = chat_width.unwrap_or(width * CHAT_SHARE).max(CHAT_MIN);
    ((width - DIVIDER - chat).max(0.), chat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(documents: &Documents) -> Vec<&str> {
        documents
            .tabs()
            .iter()
            .map(|tab| tab.path.as_str())
            .collect()
    }

    #[test]
    fn a_file_opened_twice_has_one_tab_and_is_shown() {
        let mut documents = Documents::default();
        assert_eq!(documents.shown(), None);
        documents.open("analysis/fit.R");
        documents.open("results/qc.csv");
        assert_eq!(paths(&documents), ["analysis/fit.R", "results/qc.csv"]);
        assert_eq!(documents.shown(), Some("results/qc.csv"));
        // An open file is gone to, not opened again.
        documents.open("analysis/fit.R");
        assert_eq!(paths(&documents), ["analysis/fit.R", "results/qc.csv"]);
        assert_eq!(documents.shown(), Some("analysis/fit.R"));
        assert_eq!(documents.tabs()[0].name(), "fit.R");
    }

    #[test]
    fn the_conversation_or_an_open_tab_is_shown() {
        let mut documents = Documents::default();
        documents.open("a.md");
        documents.open("b.md");
        assert!(documents.show(None));
        assert_eq!(documents.shown(), None);
        assert!(!documents.show(None));
        assert!(documents.show(Some("a.md")));
        // A file without a tab is not shown.
        assert!(!documents.show(Some("c.md")));
        assert_eq!(documents.shown(), Some("a.md"));
    }

    #[test]
    fn closing_the_tab_shown_brings_the_conversation_back() {
        let mut documents = Documents::default();
        for path in ["a.md", "b.md", "c.md"] {
            documents.open(path);
        }
        // Another tab closes without changing what is shown.
        assert!(!documents.close("a.md"));
        assert_eq!(documents.shown(), Some("c.md"));
        assert!(documents.close("c.md"));
        assert_eq!(documents.shown(), None);
        assert_eq!(paths(&documents), ["b.md"]);
        assert!(!documents.close("gone.md"));
    }

    #[test]
    fn closing_to_the_right_keeps_the_tab_asked_about() {
        let mut documents = Documents::default();
        for path in ["a.md", "b.md", "c.md", "d.md"] {
            documents.open(path);
        }
        documents.show(Some("a.md"));
        // The one shown stays shown.
        assert!(!documents.close_right("c.md"));
        assert_eq!(paths(&documents), ["a.md", "b.md", "c.md"]);
        assert_eq!(documents.shown(), Some("a.md"));
        // The one shown was to the right: the tab asked about is shown.
        documents.show(Some("c.md"));
        assert!(documents.close_right("a.md"));
        assert_eq!(paths(&documents), ["a.md"]);
        assert_eq!(documents.shown(), Some("a.md"));
        // With the conversation shown it stays shown.
        documents.open("e.md");
        documents.show(None);
        assert!(!documents.close_right("a.md"));
        assert_eq!(documents.shown(), None);
        assert!(!documents.close_right("gone.md"));
        documents.open("f.md");
        documents.close_all();
        assert!(documents.tabs().is_empty());
        assert_eq!(documents.shown(), None);
    }

    #[test]
    fn a_renamed_file_or_folder_keeps_its_tabs() {
        let mut documents = Documents::default();
        for path in ["data/raw/a.csv", "data/rawer.csv", "data/b.csv", "notes.md"] {
            documents.open(path);
        }
        documents.show(Some("data/raw/a.csv"));
        documents.renamed("data/raw", "data/clean");
        // A sibling whose name only starts the same is not under the folder.
        assert_eq!(
            paths(&documents),
            [
                "data/clean/a.csv",
                "data/rawer.csv",
                "data/b.csv",
                "notes.md"
            ]
        );
        assert_eq!(documents.shown(), Some("data/clean/a.csv"));
        documents.renamed("notes.md", "README.md");
        assert_eq!(documents.tabs()[3].path, "README.md");
        // Renamed onto a file already open, it keeps one tab.
        documents.renamed("data/b.csv", "README.md");
        assert_eq!(
            paths(&documents),
            ["data/clean/a.csv", "data/rawer.csv", "README.md"]
        );
    }

    #[test]
    fn each_conversation_keeps_its_own_documents() {
        let mut centre = Centre::default();
        assert_eq!(centre.shown(Some("s1")), None);
        assert_eq!(centre.shown(None), None);
        centre.of_mut("s1").open("fit.R");
        centre.of_mut("s2").open("qc.csv");
        centre.of_mut("s2").show(None);
        // Coming back to a conversation brings back its tabs and the one
        // it showed; another conversation's are not there.
        assert_eq!(centre.shown(Some("s1")), Some("fit.R"));
        assert_eq!(centre.shown(Some("s2")), None);
        assert_eq!(centre.of("s2").unwrap().tabs().len(), 1);
        assert!(centre.of("s3").is_none());
        // The split is the window's, drawn only where a document is shown.
        centre.split = true;
        assert!(centre.split_shown(Some("s1")));
        assert!(!centre.split_shown(Some("s2")));
        assert!(!centre.split_shown(None));
        centre.renamed("fit.R", "model.R");
        assert_eq!(centre.shown(Some("s1")), Some("model.R"));
    }

    #[test]
    fn the_conversation_keeps_a_column_to_read_in() {
        // Until it is dragged: its share, the document the rest.
        let (document, chat) = split_widths(1200., None);
        assert_eq!(chat, 1200. * CHAT_SHARE);
        assert_eq!(document, 1200. - DIVIDER - chat);
        // Narrower, the conversation holds its least width and the
        // document gives way, below its own least width once the centre
        // is narrower than both.
        assert_eq!(
            split_widths(700., None),
            (700. - DIVIDER - CHAT_MIN, CHAT_MIN)
        );
        assert_eq!(split_widths(SPLIT_MIN, None).0, DOCUMENT_MIN);
        assert_eq!(split_widths(300., None), (0., CHAT_MIN));
        // A dragged width is kept as it is.
        assert_eq!(split_widths(1200., Some(500.)), (695., 500.));
    }

    #[test]
    fn the_divider_drags_between_the_two_least_widths() {
        // A centre from x = 100, 1000 wide: its right edge is at 1100.
        let at = |pointer: f32| dragged_chat_width(100., 1000., pointer);
        assert_eq!(at(700.), 1100. - 700. - DIVIDER / 2.);
        // Dragged right, the conversation stops at its least width.
        assert_eq!(at(1050.), CHAT_MIN);
        // Dragged left, the document keeps its least width.
        assert_eq!(at(150.), 1000. - DOCUMENT_MIN - DIVIDER);
        // A centre too narrow for both leaves the conversation its column.
        assert_eq!(dragged_chat_width(0., 400., 10.), CHAT_MIN);
    }
}
