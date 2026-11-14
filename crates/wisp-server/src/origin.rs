//! Where a file came from: which tool call wrote it, with what code, in
//! which conversation, and what it read to do so.
//!
//! Each tool call that reports what it did is kept in the store's
//! execution log when its turn ends, with paths written the way the Files
//! tab names them. Looking a file up reads that log back. The table is the
//! one the WebView desktop fills and reads.

use std::path::{Component, Path};

use wisp_core::provenance::ProvenanceRecord;
use wisp_store::{ExecLog, Store};

/// As much of a call's code and of its output as is kept.
const SOURCE_LIMIT: usize = 64 * 1024;
const OUTPUT_LIMIT: usize = 16 * 1024;

/// The call that last wrote a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOrigin {
    /// The conversation it was made in.
    pub session: String,
    pub title: String,
    pub tool: String,
    pub language: String,
    /// The code or command that was run.
    pub source: String,
    /// Project files the call read, as far as it said.
    pub read: Vec<String>,
}

/// A path a tool reported, as the project names it: relative to the
/// project's folder with `/` between the parts. A path outside the
/// project, or one that climbs out of it, is not the project's to name.
pub(crate) fn relative(root: &Path, path: &str) -> Option<String> {
    let path = Path::new(path.trim());
    let inside = match path.is_absolute() {
        true => path.strip_prefix(root).ok()?,
        false => path,
    };
    let mut parts = Vec::new();
    for part in inside.components() {
        match part {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn clipped(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[… cut]", &text[..end])
}

/// Keeps what a turn's tool calls reported. A call with no code to show
/// and nothing written is not worth a row.
pub(crate) async fn record(
    store: &Store,
    frame: &str,
    root: &Path,
    records: Vec<ProvenanceRecord>,
) -> anyhow::Result<()> {
    // Reported paths are compared with the folder's real path.
    let root = tokio::fs::canonicalize(root)
        .await
        .unwrap_or_else(|_| root.to_path_buf());
    let mut cell = store.next_cell_index(frame).await?;
    for record in records {
        let named = |paths: &[String]| -> Vec<String> {
            let mut named: Vec<String> = paths
                .iter()
                .filter_map(|path| relative(&root, path))
                .collect();
            named.sort();
            named.dedup();
            named
        };
        let files_written = named(&record.files_written);
        if record.source.trim().is_empty() && files_written.is_empty() {
            continue;
        }
        store
            .insert_execution_log(&ExecLog {
                id: uuid::Uuid::new_v4().to_string(),
                frame_id: frame.to_owned(),
                cell_index: cell,
                tool: record.tool.clone(),
                language: record.language.clone(),
                source: clipped(&record.source, SOURCE_LIMIT),
                stdout: clipped(&record.output, OUTPUT_LIMIT),
                stderr: String::new(),
                exit_status: if record.success { "ok" } else { "error" }.into(),
                wall_s: None,
                files_written,
                files_read: named(&record.files_read),
                env_hash: None,
            })
            .await?;
        cell += 1;
    }
    Ok(())
}

/// The call that last wrote `path` in one of `frames`, looked for in that
/// order: the conversation that is open first, then the project's others.
pub(crate) async fn find(
    store: &Store,
    frames: &[String],
    path: &str,
) -> anyhow::Result<Option<FileOrigin>> {
    for frame in frames {
        // A file a failed call left behind is still that call's doing.
        if let Some(found) = store.find_provenance_by_path(frame, path).await? {
            return Ok(Some(FileOrigin {
                session: frame.clone(),
                title: store.session_title(frame).await?.unwrap_or_default(),
                tool: found.tool,
                language: found.language,
                source: found.source,
                read: found.files_read,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_path_is_named_the_way_the_project_names_it() {
        let root = Path::new("/work/lab");
        assert_eq!(
            relative(root, "/work/lab/results/counts.csv").as_deref(),
            Some("results/counts.csv")
        );
        assert_eq!(
            relative(root, "./results/counts.csv").as_deref(),
            Some("results/counts.csv")
        );
        assert_eq!(
            relative(root, " figs/a.png ").as_deref(),
            Some("figs/a.png")
        );
        // Outside the project, climbing out of it, or no path at all.
        assert_eq!(relative(root, "/etc/passwd"), None);
        assert_eq!(relative(root, "../other/x.csv"), None);
        assert_eq!(relative(root, "results/../../x"), None);
        assert_eq!(relative(root, "/work/lab"), None);
        assert_eq!(relative(root, ""), None);
    }

    #[test]
    fn long_code_is_cut_on_a_character() {
        assert_eq!(clipped("short", 10), "short");
        let cut = clipped(&"é".repeat(10), 5);
        assert_eq!(cut, "éé\n[… cut]");
    }
}
