//! Looking through what a finished run left on its SSH host, one folder at
//! a time, and choosing files and folders there to bring into the project
//! or to delete. Nothing of the listing is kept. No graphics.
use std::collections::BTreeMap;

use crate::i18n::tr;
use wisp_dto::WorkspaceEntry;

/// Which deletion is being asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deletion {
    /// The files and folders chosen.
    Chosen,
    /// The whole workspace, with whatever was never brought back.
    Whole,
}

pub struct RunReview {
    pub run: String,
    pub title: String,
    /// The folder shown, relative to the workspace; empty at its top.
    pub path: String,
    pub entries: Vec<WorkspaceEntry>,
    /// More entries than were read are there.
    pub truncated: bool,
    /// The chosen paths, each saying whether it is a folder.
    pub picked: BTreeMap<String, bool>,
    pub busy: bool,
    pub problem: Option<String>,
    pub note: Option<String>,
    pub asking: Option<Deletion>,
    /// Only the latest listing may fill the dialog.
    pub request: u64,
}

impl RunReview {
    pub fn new(run: String, title: String) -> Self {
        Self {
            run,
            title,
            path: String::new(),
            entries: Vec::new(),
            truncated: false,
            picked: BTreeMap::new(),
            busy: false,
            problem: None,
            note: None,
            asking: None,
            request: 0,
        }
    }

    /// Chooses an entry, or leaves it when it was chosen.
    pub fn toggle(&mut self, entry: &WorkspaceEntry) {
        if self.picked.remove(&entry.path).is_none() {
            self.picked.insert(entry.path.clone(), is_folder(entry));
        }
    }

    /// The chosen files and the chosen folders.
    pub fn chosen(&self) -> (Vec<String>, Vec<String>) {
        let (folders, files): (Vec<_>, Vec<_>) =
            self.picked.iter().partition(|(_, folder)| **folder);
        let paths = |found: Vec<(&String, &bool)>| {
            found
                .into_iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        };
        (paths(files), paths(folders))
    }

    /// Goes into `path`, or up to it; what is chosen stays chosen.
    pub fn go(&mut self, path: String) {
        self.path = path;
        self.entries.clear();
        self.truncated = false;
    }
}

pub fn is_folder(entry: &WorkspaceEntry) -> bool {
    entry.kind == "dir"
}

/// The folder above `path`; the top has none.
pub fn parent(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    Some(
        path.rsplit_once('/')
            .map(|(above, _)| above.to_owned())
            .unwrap_or_default(),
    )
}

/// The last part of an entry's path, which is how it is listed.
pub fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// How large an entry is, and how many files a folder holds.
pub fn detail(entry: &WorkspaceEntry) -> String {
    let size = crate::files::size_label(entry.size_bytes);
    if !is_folder(entry) {
        return size;
    }
    match entry.file_count.unwrap_or(0) {
        1 => format!("{size} · {}", tr("1 file")),
        count => format!("{size} · {}", crate::i18n::fill(tr("{} files"), &[&count])),
    }
}

/// Where the dialog is, from the workspace's top.
pub fn location(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &str, size: u64, files: Option<u64>) -> WorkspaceEntry {
        WorkspaceEntry {
            path: path.into(),
            kind: kind.into(),
            size_bytes: size,
            file_count: files,
        }
    }

    #[test]
    fn files_and_folders_are_chosen_across_folders_and_handed_over_apart() {
        let mut review = RunReview::new("run-1".into(), "Align".into());
        let table = entry("results/table.csv", "file", 2048, None);
        let plots = entry("results/plots", "dir", 4096, Some(3));
        review.toggle(&table);
        review.toggle(&plots);
        // Going elsewhere keeps what was chosen.
        review.go("logs".into());
        assert!(review.entries.is_empty() && review.path == "logs");
        review.toggle(&entry("logs/run.log", "file", 10, None));
        assert_eq!(
            review.chosen(),
            (
                vec!["logs/run.log".to_owned(), "results/table.csv".to_owned()],
                vec!["results/plots".to_owned()]
            )
        );
        // A second press leaves it.
        review.toggle(&table);
        assert_eq!(review.chosen().0, ["logs/run.log"]);
    }

    #[test]
    fn a_folder_has_its_parent_its_name_and_what_it_holds() {
        assert_eq!(parent(""), None);
        assert_eq!(parent("results").as_deref(), Some(""));
        assert_eq!(parent("results/plots").as_deref(), Some("results"));
        assert_eq!(name("results/plots/a.png"), "a.png");
        assert_eq!(name("top.txt"), "top.txt");
        assert_eq!(location(""), "/");
        assert_eq!(location("results/plots"), "/results/plots");
        assert_eq!(detail(&entry("a.csv", "file", 2048, None)), "2.0 KB");
        assert_eq!(
            detail(&entry("plots", "dir", 2048, Some(3))),
            "2.0 KB · 3 files"
        );
        assert_eq!(
            detail(&entry("plots", "dir", 2048, Some(1))),
            "2.0 KB · 1 file"
        );
    }
}
