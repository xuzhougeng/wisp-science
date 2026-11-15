//! The example conversations that ship with Wisp: recorded sessions in the
//! bundled `seed/` folder (`manifest_*.json`, with figures and tables in a
//! paired `assets_*.tar.gz`). The WebView's example project shows them read
//! only and copies one into a project to be continued; this reads them the
//! same way.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{bail, ensure, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wisp_llm::Message;

/// A recorded turn is never repeated more than this when it is expanded.
const MAX_REPEAT: usize = 200;
/// Nor its text more than this inside one message.
const MAX_PAD: usize = 64;
/// A title is the request's first sentence, cut to this.
const TITLE_CHARS: usize = 70;

/// One example, as a list names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DemoInfo {
    /// The manifest's file stem, e.g. `manifest_esr1_03_rnaseq`.
    pub id: String,
    pub title: String,
}

/// Who a row of an example is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DemoRole {
    You,
    Wisp,
    /// The model's reasoning before what it said.
    Thinking,
    /// The plan as it stood at that point.
    Plan,
    /// A tool's result.
    Tool,
}

/// One row of an example, as it was recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DemoRow {
    pub role: DemoRole,
    pub text: String,
    /// The tool, on a tool's row.
    pub tool: Option<String>,
    /// Whether the tool succeeded, when that was recorded.
    pub ok: Option<bool>,
    /// The model that said it, on Wisp's rows.
    pub model: Option<String>,
}

/// One example, whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Demo {
    pub id: String,
    pub title: String,
    pub rows: Vec<DemoRow>,
}

/// An example with the files it puts in a project it is copied into.
pub(crate) struct Manifest {
    pub demo: Demo,
    /// Text files by their project path, written where nothing is.
    pub files: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Item {
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    model_name: Option<String>,
}

/// A recorded turn kept short in the file: its text repeated `pad` times
/// in one message, and that message `repeat` times, so a long-context
/// example stays small on disk.
#[derive(Deserialize)]
struct SeedTurn {
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default = "one")]
    repeat: usize,
    #[serde(default = "one")]
    pad: usize,
}

fn one() -> usize {
    1
}

/// Whether `id` names a manifest and nothing else: no folder, no dots.
fn valid_id(id: &str) -> bool {
    id.strip_prefix("manifest_").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

/// Placeholders for figures the recording made are said in words: the
/// figure itself is in the example's archive, under its own name.
fn clean(text: &str) -> String {
    static PICTURE: OnceLock<Regex> = OnceLock::new();
    static ARTIFACT: OnceLock<Regex> = OnceLock::new();
    let picture = PICTURE
        .get_or_init(|| Regex::new(r"!\[([^\]]*)\]\(\{\{artifact:[^}]+\}\}\)").expect("regex"));
    let artifact = ARTIFACT.get_or_init(|| Regex::new(r"\{\{artifact:[^}]+\}\}").expect("regex"));
    let text = picture.replace_all(text, "[$1 (figure)]");
    artifact.replace_all(&text, "(artifact)").into_owned()
}

fn request(value: &Value) -> Option<&str> {
    value
        .pointer("/root_frame/input_data/request")
        .and_then(Value::as_str)
}

/// The request's first sentence, short.
fn title(value: &Value, id: &str) -> String {
    let fallback = || id.trim_start_matches("manifest_").to_owned();
    let Some(request) = request(value) else {
        return fallback();
    };
    let first = request.split('.').next().unwrap_or(request).trim();
    match first.is_empty() {
        true => fallback(),
        false => first.chars().take(TITLE_CHARS).collect(),
    }
}

fn read(dir: &Path, id: &str) -> Result<Value> {
    ensure!(valid_id(id), "There is no example called {id}");
    let path = dir.join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("There is no example called {id}"))?;
    serde_json::from_str(&text).with_context(|| format!("The example {id} could not be read"))
}

/// The examples in `dir`, in their file order, which is the order the
/// research they record went in.
pub(crate) fn list(dir: Option<&Path>) -> Vec<DemoInfo> {
    let Some(entries) = dir.and_then(|dir| std::fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut found: Vec<DemoInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_owned();
            if !valid_id(&id) {
                return None;
            }
            let value = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())?;
            Some(DemoInfo {
                title: title(&value, &id),
                id,
            })
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

fn row(role: DemoRole, text: &str) -> DemoRow {
    DemoRow {
        role,
        text: clean(text),
        tool: None,
        ok: None,
        model: None,
    }
}

fn role(name: &str) -> Option<DemoRole> {
    Some(match name {
        "user" => DemoRole::You,
        "assistant" => DemoRole::Wisp,
        "reasoning" => DemoRole::Thinking,
        "plan" => DemoRole::Plan,
        "tool" => DemoRole::Tool,
        _ => return None,
    })
}

/// One example from `dir`, and the files it brings.
pub(crate) fn load(dir: &Path, id: &str) -> Result<Manifest> {
    let value = read(dir, id)?;
    let output = |pointer: &str| value.pointer(&format!("/root_frame/output_data/{pointer}"));
    let mut rows = Vec::new();
    if let Some(turns) = output("context_seed").and_then(|v| Vec::<SeedTurn>::deserialize(v).ok()) {
        for turn in turns {
            let Some(role) = role(&turn.role) else {
                continue;
            };
            let text = turn.text.repeat(turn.pad.clamp(1, MAX_PAD));
            for _ in 0..turn.repeat.clamp(1, MAX_REPEAT) {
                rows.push(row(role, &text));
            }
        }
    }
    let items = output("items")
        .and_then(|v| Vec::<Item>::deserialize(v).ok())
        .unwrap_or_default();
    for item in items {
        let Some(role) = role(&item.role) else {
            continue;
        };
        rows.push(DemoRow {
            tool: item.tool_name.filter(|_| role == DemoRole::Tool),
            ok: item.ok,
            model: item.model_name.filter(|_| role == DemoRole::Wisp),
            ..row(role, &item.text)
        });
    }
    // An older recording has only the request and the answer.
    if rows.is_empty() {
        let text = |pointer| output(pointer).and_then(Value::as_str).unwrap_or("");
        rows.push(row(DemoRole::You, request(&value).unwrap_or("")));
        if !text("thinking").trim().is_empty() {
            rows.push(row(DemoRole::Thinking, text("thinking")));
        }
        rows.push(row(DemoRole::Wisp, text("response")));
    }
    let files = output("workspace_files")
        .and_then(|v| BTreeMap::<String, String>::deserialize(v).ok())
        .unwrap_or_default();
    Ok(Manifest {
        demo: Demo {
            id: id.to_owned(),
            title: title(&value, id),
            rows,
        },
        files,
    })
}

/// The messages a copied example is continued from. Thinking goes with the
/// answer after it; plans were the app's, not the model's, and are left
/// out, as the WebView leaves them.
pub(crate) fn messages(rows: &[DemoRow]) -> Vec<Message> {
    let mut out = Vec::new();
    let mut thinking = None;
    for (index, row) in rows.iter().enumerate() {
        match row.role {
            DemoRole::Thinking => thinking = Some(row.text.clone()),
            DemoRole::You => {
                thinking = None;
                out.push(Message::user(&row.text));
            }
            DemoRole::Wisp => {
                let mut message = Message::assistant(&row.text);
                message.reasoning = thinking.take();
                message.model_name = row.model.clone();
                out.push(message);
            }
            DemoRole::Tool => out.push(Message::tool(
                format!("demo-tool-{index}"),
                row.tool.as_deref().unwrap_or("tool"),
                &row.text,
            )),
            DemoRole::Plan => {}
        }
    }
    out
}

/// `relative` under `root`, refused when it would leave it.
fn inside(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => out.push(name),
            _ => bail!("The example names a file outside the project: {relative}"),
        }
    }
    ensure!(
        !out.as_os_str().is_empty(),
        "The example names a file without a name"
    );
    Ok(root.join(out))
}

/// Writes the example's text files and unpacks its archive into `root`.
/// Nothing that is there is written over. Returns how many files were
/// written.
pub(crate) fn write_files(
    dir: &Path,
    id: &str,
    root: &Path,
    files: &BTreeMap<String, String>,
) -> Result<usize> {
    ensure!(valid_id(id), "There is no example called {id}");
    let mut written = 0;
    for (relative, text) in files {
        let path = inside(root, relative)?;
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not make {}", parent.display()))?;
        }
        std::fs::write(&path, text)
            .with_context(|| format!("Could not write {}", path.display()))?;
        written += 1;
    }
    let archive = dir.join(format!(
        "assets_{}.tar.gz",
        id.trim_start_matches("manifest_")
    ));
    if !archive.is_file() {
        return Ok(written);
    }
    let file = std::fs::File::open(&archive)
        .with_context(|| format!("Could not open {}", archive.display()))?;
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in entries
        .entries()
        .context("The example's files could not be read")?
    {
        let mut entry = entry.context("The example's files could not be read")?;
        // Only files, each by its own name at the top of the project, as
        // the recording refers to them.
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry
            .path()
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_owned()));
        let Some(name) = name else {
            continue;
        };
        let path = root.join(name);
        if path.exists() {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .context("The example's files could not be read")?;
        std::fs::write(&path, bytes)
            .with_context(|| format!("Could not write {}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> PathBuf {
        wisp_paths::seed_dir().expect("the checkout has its seed folder")
    }

    #[test]
    fn the_bundled_examples_are_listed_in_order_with_titles() {
        let found = list(Some(&seed()));
        let ids: Vec<&str> = found.iter().map(|demo| demo.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "manifest_esr1_01_datasets",
                "manifest_esr1_02_samples",
                "manifest_esr1_03_rnaseq",
                "manifest_esr1_04_downstream",
                "manifest_esr1_05_hypotheses",
                "manifest_memory_01_long_context",
            ]
        );
        assert!(found
            .iter()
            .all(|demo| !demo.title.is_empty() && demo.title.chars().count() <= TITLE_CHARS));
        assert!(list(None).is_empty());
        assert!(list(Some(Path::new("/no/such/folder"))).is_empty());
    }

    #[test]
    fn an_example_reads_as_rows_with_its_tools_and_no_placeholders() {
        let rnaseq = load(&seed(), "manifest_esr1_03_rnaseq").unwrap().demo;
        assert_eq!(rnaseq.rows[0].role, DemoRole::You);
        assert!(rnaseq.rows[0].text.contains("GSE153250"));
        assert!(rnaseq
            .rows
            .iter()
            .any(|row| row.role == DemoRole::Tool && row.tool.as_deref() == Some("monitor_run")));
        assert!(rnaseq.rows.iter().any(|row| row.role == DemoRole::Plan));
        assert!(rnaseq
            .rows
            .iter()
            .filter(|row| row.role == DemoRole::Wisp)
            .all(|row| row.tool.is_none()));
        assert!(rnaseq
            .rows
            .iter()
            .all(|row| !row.text.contains("{{artifact:")));

        // The long-context example is expanded from its short seed.
        let memory = load(&seed(), "manifest_memory_01_long_context").unwrap();
        assert!(memory.demo.rows.len() > 100);
        assert!(memory.demo.rows[1].text.contains("FDR_CUTOFF=0.05"));
        assert!(memory.files.contains_key("AGENTS.md"));

        // Only a manifest's own name is read.
        for id in [
            "../Cargo",
            "manifest_../x",
            "manifest_",
            "notes",
            "manifest_a/b",
        ] {
            assert!(load(&seed(), id).is_err(), "{id}");
        }
    }

    #[test]
    fn placeholders_for_figures_are_said_in_words() {
        assert_eq!(
            clean("See ![Volcano]({{artifact:abc}}) and {{artifact:def}}."),
            "See [Volcano (figure)] and (artifact)."
        );
    }

    #[test]
    fn rows_become_messages_with_thinking_on_the_answer() {
        let rows = vec![
            row(DemoRole::You, "count reads"),
            row(DemoRole::Thinking, "use wc"),
            row(DemoRole::Plan, "[ ] count"),
            DemoRow {
                model: Some("m".into()),
                ..row(DemoRole::Wisp, "Counting.")
            },
            DemoRow {
                tool: Some("shell".into()),
                ok: Some(true),
                ..row(DemoRole::Tool, "400")
            },
            row(DemoRole::Wisp, "400 reads."),
        ];
        let out = messages(&rows);
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].reasoning.as_deref(), Some("use wc"));
        assert_eq!(out[1].model_name.as_deref(), Some("m"));
        assert_eq!(out[2].tool_name.as_deref(), Some("shell"));
        assert_eq!(out[2].tool_call_id.as_deref(), Some("demo-tool-4"));
        assert_eq!(out[3].reasoning, None);
    }

    #[test]
    fn files_are_written_inside_the_project_and_never_over_anything() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("GSE153250_counts_matrix.tsv"), "mine").unwrap();
        let files = BTreeMap::from([
            ("AGENTS.md".to_owned(), "rules".to_owned()),
            (".wisp/memory/note.md".to_owned(), "note".to_owned()),
        ]);
        let written = write_files(&seed(), "manifest_esr1_03_rnaseq", root, &files).unwrap();
        assert!(written >= 3, "{written}");
        assert_eq!(
            std::fs::read_to_string(root.join(".wisp/memory/note.md")).unwrap(),
            "note"
        );
        assert!(root.join("GSE153250_sample_groups.txt").is_file());
        // What was there stays as it was.
        assert_eq!(
            std::fs::read_to_string(root.join("GSE153250_counts_matrix.tsv")).unwrap(),
            "mine"
        );
        // Again, nothing is new.
        assert_eq!(
            write_files(&seed(), "manifest_esr1_03_rnaseq", root, &files).unwrap(),
            0
        );
        for unsafe_path in ["../out.md", "/etc/x", ""] {
            let files = BTreeMap::from([(unsafe_path.to_owned(), "x".to_owned())]);
            assert!(write_files(&seed(), "manifest_esr1_01_datasets", root, &files).is_err());
        }
    }
}
