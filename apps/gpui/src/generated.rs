//! The files a turn produced, under the answer that closes it, as the
//! WebView's "Generated · N" lists them: which answer each recorded file
//! belongs to, and the folders they are in. The host says which tool call
//! each turn's files come after; the files go under the last answer of the
//! turn that call is in.

use std::collections::{BTreeMap, HashMap};

use wisp_dto::native_conversations::Item;
use wisp_server::desktop::ReplyArtifact;

/// The files under one answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// What its folds are kept under while the conversation is open: the
    /// first tool call its files came after, which does not move when more
    /// of the transcript is loaded above it.
    pub key: String,
    pub files: Vec<File>,
}

/// One file under an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub artifact: String,
    pub name: String,
    /// Where it is in the project.
    pub path: Option<String>,
    /// A later answer of the conversation wrote it again.
    pub superseded: bool,
}

/// The answer the files recorded after `call` go under: the last answer
/// of the turn the call is in, the one `inline::written` draws a turn's
/// pictures under. None while the turn has not answered yet, or when the
/// call is not among the loaded rows.
pub fn answer_of(rows: &[Item], call: &str) -> Option<usize> {
    let at = rows
        .iter()
        .rposition(|row| row.role == "tool" && row.call_id.as_deref() == Some(call))?;
    rows.iter()
        .enumerate()
        .skip(at + 1)
        .take_while(|(_, row)| row.role != "user")
        .filter(|(_, row)| row.role == "assistant" && !row.text.trim().is_empty())
        .map(|(index, _)| index)
        .last()
}

/// The files of each answer, by row. A file recorded twice for one answer
/// (a turn that was resumed) is listed once, as recorded last.
pub fn by_answer(rows: &[Item], artifacts: &[ReplyArtifact]) -> HashMap<usize, Reply> {
    let mut answers: HashMap<usize, Reply> = HashMap::new();
    // Each call is looked for once.
    let mut found: HashMap<&str, Option<usize>> = HashMap::new();
    for artifact in artifacts {
        let answer = *found
            .entry(artifact.call_id.as_str())
            .or_insert_with(|| answer_of(rows, &artifact.call_id));
        let Some(answer) = answer else {
            continue;
        };
        let reply = answers.entry(answer).or_insert_with(|| Reply {
            key: artifact.call_id.clone(),
            files: Vec::new(),
        });
        let file = File {
            artifact: artifact.artifact.clone(),
            name: artifact.name.clone(),
            path: artifact.path.clone(),
            superseded: artifact.superseded,
        };
        match reply
            .files
            .iter_mut()
            .find(|kept| kept.artifact == file.artifact)
        {
            Some(kept) => *kept = file,
            None => reply.files.push(file),
        }
    }
    answers
}

/// The pictures among an answer's files to draw under it: project
/// pictures no later answer wrote again, in order.
pub fn pictures(reply: &Reply) -> Vec<String> {
    reply
        .files
        .iter()
        .filter(|file| !file.superseded)
        .filter_map(|file| file.path.clone())
        .filter(|path| crate::favorites::is_image(path))
        .collect()
}

/// A folder of an answer's files: the folders in it by name, and its own
/// files as indexes into the answer's list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub folders: BTreeMap<String, Folder>,
    pub files: Vec<usize>,
}

impl Folder {
    /// How many files are in it and in its folders.
    pub fn count(&self) -> usize {
        self.files.len() + self.folders.values().map(Folder::count).sum::<usize>()
    }
}

/// The answer's files under their folders. A file with no folder, or that
/// is not a project file, is at the top.
pub fn tree(files: &[File]) -> Folder {
    let mut top = Folder::default();
    for (index, file) in files.iter().enumerate() {
        let mut folder = &mut top;
        if let Some(path) = &file.path {
            let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
            parts.pop();
            for part in parts {
                folder = folder.folders.entry(part.to_owned()).or_default();
            }
        }
        folder.files.push(index);
    }
    top
}

/// One line of the opened list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A folder: its path from the top (what its fold is kept under), its
    /// name, how many files it holds in all, how deep it is and whether it
    /// is open.
    Folder {
        path: String,
        name: String,
        count: usize,
        depth: usize,
        open: bool,
    },
    /// A file, as an index into the answer's list, and how deep it is.
    File { index: usize, depth: usize },
}

/// The lines to show of `folder`: its folders first, by name, each followed
/// by what it holds when `open` says it is open, then its files.
pub fn lines(folder: &Folder, open: &dyn Fn(&str) -> bool) -> Vec<Line> {
    let mut lines = Vec::new();
    push_lines(folder, "", 0, open, &mut lines);
    lines
}

fn push_lines(
    folder: &Folder,
    parent: &str,
    depth: usize,
    open: &dyn Fn(&str) -> bool,
    lines: &mut Vec<Line>,
) {
    for (name, child) in &folder.folders {
        let path = match parent {
            "" => name.clone(),
            parent => format!("{parent}/{name}"),
        };
        let is_open = open(&path);
        lines.push(Line::Folder {
            path: path.clone(),
            name: name.clone(),
            count: child.count(),
            depth,
            open: is_open,
        });
        if is_open {
            push_lines(child, &path, depth + 1, open, lines);
        }
    }
    lines.extend(
        folder
            .files
            .iter()
            .map(|&index| Line::File { index, depth }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, text: &str, call: Option<&str>) -> Item {
        serde_json::from_value(serde_json::json!({
            "role": role, "text": text, "tool_name": call.map(|_| "python"),
            "input": null, "ok": null, "status": null, "call_id": call
        }))
        .unwrap()
    }

    fn recorded(call: &str, artifact: &str, path: &str, superseded: bool) -> ReplyArtifact {
        ReplyArtifact {
            call_id: call.into(),
            artifact: artifact.into(),
            version: format!("{call}:{artifact}"),
            name: path.rsplit('/').next().unwrap().into(),
            kind: "image/png".into(),
            path: Some(path.into()),
            superseded,
        }
    }

    fn file(path: &str) -> File {
        File {
            artifact: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            path: Some(path.into()),
            superseded: false,
        }
    }

    #[test]
    fn a_turns_files_go_under_its_last_answer() {
        let rows = [
            row("user", "plot it", None),
            row("tool", "", Some("c1")),
            row("assistant", "Writing the second one.", None),
            row("tool", "", Some("c2")),
            row("assistant", "Done.", None),
            row("user", "again", None),
            row("tool", "", Some("c3")),
            row("assistant", "Redone.", None),
            row("user", "and now?", None),
            row("tool", "", Some("c4")),
        ];
        // From any call of the turn, the turn's last answer.
        assert_eq!(answer_of(&rows, "c1"), Some(4));
        assert_eq!(answer_of(&rows, "c2"), Some(4));
        assert_eq!(answer_of(&rows, "c3"), Some(7));
        // Not answered yet, or a call that is not loaded.
        assert_eq!(answer_of(&rows, "c4"), None);
        assert_eq!(answer_of(&rows, "gone"), None);

        let artifacts = [
            recorded("c2", "a", "figs/a.png", true),
            recorded("c2", "t", "tables/t.csv", false),
            recorded("c3", "a", "figs/a.png", false),
            recorded("gone", "x", "x.png", false),
        ];
        let answers = by_answer(&rows, &artifacts);
        assert_eq!(answers.len(), 2);
        let first = &answers[&4];
        assert_eq!(first.key, "c2");
        assert_eq!(
            first
                .files
                .iter()
                .map(|file| (file.name.as_str(), file.superseded))
                .collect::<Vec<_>>(),
            [("a.png", true), ("t.csv", false)]
        );
        assert_eq!(answers[&7].files.len(), 1);
        assert!(!answers[&7].files[0].superseded);
    }

    #[test]
    fn a_file_recorded_twice_for_one_answer_is_listed_once() {
        let rows = [
            row("user", "plot it", None),
            row("tool", "", Some("c1")),
            row("tool", "", Some("c2")),
            row("assistant", "Done.", None),
        ];
        // A resumed turn recorded figs/a.png after each of its calls.
        let answers = by_answer(
            &rows,
            &[
                recorded("c1", "a", "figs/a.png", true),
                recorded("c2", "a", "figs/a.png", false),
            ],
        );
        let reply = &answers[&3];
        // Its folds stay with the first call, which later loading keeps.
        assert_eq!(reply.key, "c1");
        assert_eq!(reply.files.len(), 1);
        assert!(!reply.files[0].superseded);
    }

    #[test]
    fn folders_count_what_they_hold_and_keep_same_names_apart() {
        let files = [
            file("results/run1/plot.png"),
            file("results/run2/plot.png"),
            file("results/summary.csv"),
            file("notes.md"),
            File {
                path: None,
                ..file("inline")
            },
        ];
        let top = tree(&files);
        assert_eq!(top.count(), 5);
        // Files with no folder, and those that are not project files, at the top.
        assert_eq!(top.files, [3, 4]);
        let results = &top.folders["results"];
        assert_eq!(results.count(), 3);
        assert_eq!(results.files, [2]);
        assert_eq!(results.folders["run1"].files, [0]);
        assert_eq!(results.folders["run2"].files, [1]);

        // Folders start shut: one line for the folder with its count, then
        // the top's files.
        assert_eq!(
            lines(&top, &|_| false),
            [
                Line::Folder {
                    path: "results".into(),
                    name: "results".into(),
                    count: 3,
                    depth: 0,
                    open: false,
                },
                Line::File { index: 3, depth: 0 },
                Line::File { index: 4, depth: 0 },
            ]
        );
        // Opened, a folder shows its folders and then its files, one deeper.
        let open = |path: &str| matches!(path, "results" | "results/run2");
        assert_eq!(
            lines(&top, &open),
            [
                Line::Folder {
                    path: "results".into(),
                    name: "results".into(),
                    count: 3,
                    depth: 0,
                    open: true,
                },
                Line::Folder {
                    path: "results/run1".into(),
                    name: "run1".into(),
                    count: 1,
                    depth: 1,
                    open: false,
                },
                Line::Folder {
                    path: "results/run2".into(),
                    name: "run2".into(),
                    count: 1,
                    depth: 1,
                    open: true,
                },
                Line::File { index: 1, depth: 2 },
                Line::File { index: 2, depth: 1 },
                Line::File { index: 3, depth: 0 },
                Line::File { index: 4, depth: 0 },
            ]
        );
    }

    #[test]
    fn only_current_project_pictures_are_drawn() {
        let reply = Reply {
            key: "c1".into(),
            files: vec![
                file("figs/a.png"),
                File {
                    superseded: true,
                    ..file("figs/old.png")
                },
                file("tables/t.csv"),
                File {
                    path: None,
                    ..file("b.png")
                },
                file("figs/c.svg"),
            ],
        };
        assert_eq!(pictures(&reply), ["figs/a.png", "figs/c.svg"]);
    }
}
