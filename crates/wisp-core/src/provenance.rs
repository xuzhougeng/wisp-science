//! Best-effort artifact provenance: snapshot the workspace around a producing
//! tool call and diff to learn which files it wrote and read.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const MAX_UNDO_TEXT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_UNDO_CAPTURE_BYTES: u64 = 32 * 1024 * 1024;

/// Reported to `Output::provenance` after a producing tool writes ≥1 file.
#[derive(Debug, Clone, Default)]
pub struct ProvenanceRecord {
    pub tool: String,
    pub language: String,
    pub source: String,
    pub output: String,
    pub success: bool,
    pub files_written: Vec<String>,
    pub files_read: Vec<String>,
    pub file_changes: Vec<ProvenanceFileChange>,
}

/// A bounded preimage for undoing one local text-file change.
#[derive(Debug, Clone, Default)]
pub struct ProvenanceFileChange {
    pub path: String,
    pub before_exists: bool,
    pub before_text: Option<String>,
    pub before_checksum: Option<String>,
    pub after_checksum: Option<String>,
    pub reversible: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TextPreimage {
    pub text: String,
    pub checksum: String,
}

pub(crate) const SKIP_DIRS: &[&str] = &[
    ".git",
    ".venv",
    "node_modules",
    ".wisp",
    "uploads",
    "__pycache__",
];
// ponytail: recursive mtime scan, capped + heavy dirs skipped. Swap for an fs-notify
// watcher only if this shows up in a profile.
const MAX_ENTRIES: usize = 20_000;

pub fn is_producing(tool: &str) -> bool {
    matches!(
        tool,
        "python" | "r" | "shell" | "write" | "edit" | "generate_image" | "generate_video"
    )
}

pub fn language_of(tool: &str) -> String {
    match tool {
        "python" => "python",
        "r" => "r",
        "shell" => "bash",
        _ => "text",
    }
    .to_string()
}

pub fn source_of(tool: &str, args: &serde_json::Value) -> String {
    let key = match tool {
        "python" | "r" => "code",
        "write" | "edit" | "generate_image" | "generate_video" => "path",
        _ => "cmd",
    };
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Recursive path→mtime map of the workspace, skipping heavy dirs, capped.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    snapshot_capped(root, MAX_ENTRIES)
}

/// Parallel conversations of one project run producing tools concurrently
/// against the same workspace root, so the pre/post mtime diff alone would
/// credit every overlapping call with every file another session wrote (#911).
/// Each producing call registers a window here and, on completion, learns
/// which other windows overlapped it. Windows carry an opaque conversation
/// scope: windows of the same scope (one conversation and its subagents) are
/// not foreign to each other, so a parent turn and its parallel subagents do
/// not suppress each other's attribution.
#[derive(Default)]
struct RootWindows {
    next_id: u64,
    /// window id → every producing call still in flight.
    active: HashMap<u64, ActiveWindow>,
    /// Windows that already ended but may still overlap an active window.
    ended: Vec<EndedWindow>,
}

struct ActiveWindow {
    started: SystemTime,
    scope: Option<String>,
}

struct EndedWindow {
    started: SystemTime,
    ended: SystemTime,
    scope: Option<String>,
}

static WINDOWS: OnceLock<Mutex<HashMap<String, RootWindows>>> = OnceLock::new();

fn windows() -> &'static Mutex<HashMap<String, RootWindows>> {
    WINDOWS.get_or_init(Default::default)
}

/// One producing tool call's registration in the per-root window registry.
/// Dropping without `finish` still deregisters, so a cancelled call cannot
/// poison attribution for the rest of the process lifetime.
pub struct ProducingWindow {
    root_key: String,
    id: u64,
    started: SystemTime,
    scope: Option<String>,
    closed: bool,
}

/// A closed producing window: its own span plus the spans of every foreign
/// (different-scope) window that overlapped it.
pub struct FinishedWindow {
    pub started: SystemTime,
    pub ended: SystemTime,
    pub foreign: Vec<(SystemTime, SystemTime)>,
}

/// Register a producing tool call against `root` before its pre-snapshot.
/// `scope` identifies the conversation tree this call belongs to (a root frame
/// id); calls without a scope treat every other window as foreign.
pub fn begin_window(root: &Path, scope: Option<&str>) -> ProducingWindow {
    let root_key = path_identity(root);
    let started = SystemTime::now();
    let scope = scope.map(str::to_owned);
    let mut map = windows()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = map.entry(root_key.clone()).or_default();
    let id = entry.next_id;
    entry.next_id += 1;
    entry.active.insert(
        id,
        ActiveWindow {
            started,
            scope: scope.clone(),
        },
    );
    ProducingWindow {
        root_key,
        id,
        started,
        scope,
        closed: false,
    }
}

impl ProducingWindow {
    /// Close this window after the post-snapshot and return its span together
    /// with the span of every foreign producing window (still active or
    /// already ended) that overlapped it. Still-active windows are clamped to
    /// now, which covers every mtime this call can have observed.
    pub fn finish(mut self) -> FinishedWindow {
        self.close()
    }

    fn is_foreign(&self, other: Option<&str>) -> bool {
        match (self.scope.as_deref(), other) {
            (Some(mine), Some(theirs)) => mine != theirs,
            _ => true,
        }
    }

    fn close(&mut self) -> FinishedWindow {
        let ended_at = SystemTime::now();
        if self.closed {
            return FinishedWindow {
                started: self.started,
                ended: ended_at,
                foreign: Vec::new(),
            };
        }
        self.closed = true;
        let mut map = windows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = map.get_mut(&self.root_key) else {
            return FinishedWindow {
                started: self.started,
                ended: ended_at,
                foreign: Vec::new(),
            };
        };
        entry.active.remove(&self.id);
        let mut foreign: Vec<(SystemTime, SystemTime)> = entry
            .active
            .values()
            .filter(|window| self.is_foreign(window.scope.as_deref()))
            .map(|window| (window.started, ended_at))
            .collect();
        foreign.extend(
            entry
                .ended
                .iter()
                .filter(|window| {
                    window.ended >= self.started && self.is_foreign(window.scope.as_deref())
                })
                .map(|window| (window.started, window.ended)),
        );
        if entry.active.is_empty() {
            map.remove(&self.root_key);
        } else {
            entry.ended.push(EndedWindow {
                started: self.started,
                ended: ended_at,
                scope: self.scope.clone(),
            });
            let oldest_active = entry
                .active
                .values()
                .map(|window| window.started)
                .min()
                .unwrap_or(ended_at);
            entry.ended.retain(|window| window.ended >= oldest_active);
        }
        FinishedWindow {
            started: self.started,
            ended: ended_at,
            foreign,
        }
    }
}

impl Drop for ProducingWindow {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Widen other windows when checking whether an mtime falls inside them, so
/// coarse filesystem timestamp truncation cannot leak a neighbor's write past
/// the interval check.
const OVERLAP_MTIME_SLACK: Duration = Duration::from_secs(2);

/// Drop written paths whose authorship is ambiguous because a foreign
/// producing tool call overlapped this one. A path is kept only when this
/// call's source names it, or when the file's mtime falls inside this call's
/// own window and outside every foreign one. Requiring the own window rejects
/// backdated timestamps (e.g. tar extraction preserving old mtimes), which
/// would otherwise be credited to an unrelated bystander call (#911).
/// Wrong attribution is worse than missing attribution.
pub fn retain_unambiguous_writes(
    written: &mut Vec<String>,
    after: &BTreeMap<PathBuf, SystemTime>,
    root: &Path,
    source: &str,
    window: &FinishedWindow,
) {
    if window.foreign.is_empty() {
        return;
    }
    let own_start = window
        .started
        .checked_sub(OVERLAP_MTIME_SLACK)
        .unwrap_or(UNIX_EPOCH);
    let own_end = window
        .ended
        .checked_add(OVERLAP_MTIME_SLACK)
        .unwrap_or(window.ended);
    written.retain(|relative| {
        let path = root.join(relative);
        if source_mentions_path(source, root, &path) {
            return true;
        }
        let Some(mtime) = after.get(&path) else {
            return false;
        };
        if *mtime < own_start || *mtime > own_end {
            return false;
        }
        window.foreign.iter().all(|(start, end)| {
            let start = start.checked_sub(OVERLAP_MTIME_SLACK).unwrap_or(UNIX_EPOCH);
            let end = end.checked_add(OVERLAP_MTIME_SLACK).unwrap_or(*end);
            *mtime < start || *mtime > end
        })
    });
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn path_identity(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        path.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn relative_identity(path: &str) -> String {
    path_identity(Path::new(path))
}

/// True when [`snapshot`] would never have reported `relative`, because one
/// of its parent directories is in `SKIP_DIRS`. Applies the snapshot's own
/// rule — a directory is skipped by name at any depth — so the two cannot
/// drift.
fn is_snapshot_skipped(relative: &str) -> bool {
    match relative.rsplit_once('/') {
        Some((parents, _)) => parents.split('/').any(|dir| SKIP_DIRS.contains(&dir)),
        None => false,
    }
}

/// Union interpreter-reported writes into the diff-derived list.
///
/// A report bypasses the workspace snapshot, so it must be confined by the
/// snapshot's exclusions first: otherwise a cell that merely imports a
/// project-local module credits itself with the `__pycache__` bytecode the
/// import wrote, and the record names paths the diff could never produce.
pub fn union_reported_writes(paths: &mut Vec<String>, reported: &[String]) {
    let kept: Vec<String> = reported
        .iter()
        .filter(|path| !is_snapshot_skipped(path))
        .cloned()
        .collect();
    union_paths_by_identity(paths, &kept);
}

/// Union `extra` into `paths`, keeping the spelling already present when
/// two spellings resolve to one file (`out\b.txt` vs `out/b.txt`; case
/// variants on Windows). Every merge of written paths goes through here so
/// the equality rule cannot drift between call sites.
pub fn union_paths_by_identity(paths: &mut Vec<String>, extra: &[String]) {
    if extra.is_empty() {
        return;
    }
    let mut seen: HashSet<String> = paths.iter().map(|p| relative_identity(p)).collect();
    let mut inserted = false;
    for path in extra {
        if seen.insert(relative_identity(path)) {
            paths.push(path.clone());
            inserted = true;
        }
    }
    // Only re-sort when something was added: a report that covers nothing
    // new must leave the record byte-identical to the pre-report behavior.
    if inserted {
        paths.sort();
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    path_identity(left) == path_identity(right)
}

pub fn is_text_path(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "markdown"
            | "rmd"
            | "qmd"
            | "txt"
            | "csv"
            | "tsv"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "rs"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hpp"
            | "hxx"
            | "cs"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "swift"
            | "py"
            | "pyi"
            | "r"
            | "rb"
            | "php"
            | "lua"
            | "pl"
            | "pm"
            | "scala"
            | "clj"
            | "cljs"
            | "ex"
            | "exs"
            | "erl"
            | "hrl"
            | "fs"
            | "fsx"
            | "vb"
            | "vue"
            | "svelte"
            | "astro"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "bat"
            | "cmd"
            | "tex"
            | "bib"
            | "log"
            | "ini"
            | "cfg"
            | "conf"
            | "sql"
            | "ipynb"
    )
}

/// Fold backslashes into forward slashes and collapse runs of slashes, so an
/// escaped Windows separator in program text (`out\\b.txt` → `out//b.txt`)
/// still lines up with the normalized relative path (`out/b.txt`).
fn normalize_path_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_was_slash = false;
    for ch in text.chars() {
        let ch = if ch == '\\' { '/' } else { ch };
        if ch == '/' && previous_was_slash {
            continue;
        }
        previous_was_slash = ch == '/';
        out.push(ch);
    }
    #[cfg(windows)]
    {
        out.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        out
    }
}

fn is_path_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')
}

/// Whole-token occurrence of a normalized path inside normalized program
/// text. Boundary characters on both sides must not be path characters, so
/// `results.csv` is not "named" by a source that only writes
/// `final_results.csv` (#911). An explicit `./` prefix still counts.
fn mentions_path_token(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(found) = haystack[from..].find(needle) {
        let start = from + found;
        let end = start + needle.len();
        let prefix = &haystack[..start];
        let before_ok = match prefix.chars().next_back() {
            None => true,
            Some('/') => {
                prefix.ends_with("./")
                    && prefix[..prefix.len() - 2]
                        .chars()
                        .next_back()
                        .is_none_or(|ch| !is_path_char(ch))
            }
            Some(ch) => !is_path_char(ch),
        };
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_path_char(ch));
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn source_mentions_path(source: &str, root: &Path, path: &Path) -> bool {
    let source = normalize_path_text(source);
    let absolute = normalize_path_text(&path.to_string_lossy());
    let relative = normalize_path_text(&path.strip_prefix(root).unwrap_or(path).to_string_lossy());
    mentions_path_token(&source, &absolute) || mentions_path_token(&source, &relative)
}

/// Capture only bounded text files explicitly named by a producing tool.
///
/// New files need no preimage. Existing files whose destination is computed
/// dynamically remain visible in the undo preview but are not overwritten.
pub fn capture_text_preimages(
    before: &BTreeMap<PathBuf, SystemTime>,
    root: &Path,
    source: &str,
) -> BTreeMap<PathBuf, TextPreimage> {
    let mut out = BTreeMap::new();
    let mut captured = 0_u64;
    for path in before.keys() {
        if !is_text_path(path) || !source_mentions_path(source, root, path) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        let len = metadata.len();
        if len > MAX_UNDO_TEXT_BYTES
            || captured
                .checked_add(len)
                .is_none_or(|total| total > MAX_UNDO_CAPTURE_BYTES)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        captured += len;
        out.insert(
            path.clone(),
            TextPreimage {
                checksum: sha256_hex(text.as_bytes()),
                text,
            },
        );
    }
    out
}

fn read_current_text(path: &Path) -> Result<(String, String), &'static str> {
    let metadata = std::fs::metadata(path).map_err(|_| "file is no longer readable")?;
    if metadata.len() > MAX_UNDO_TEXT_BYTES {
        return Err("text file exceeds the 10 MiB undo limit");
    }
    let bytes = std::fs::read(path).map_err(|_| "file is no longer readable")?;
    let text = String::from_utf8(bytes).map_err(|_| "binary files are not supported yet")?;
    let checksum = sha256_hex(text.as_bytes());
    Ok((text, checksum))
}

/// Turn the provenance diff into bounded undo entries.
pub fn undo_file_changes(
    before: &BTreeMap<PathBuf, SystemTime>,
    root: &Path,
    written: &[String],
    preimages: &BTreeMap<PathBuf, TextPreimage>,
) -> Vec<ProvenanceFileChange> {
    written
        .iter()
        .map(|relative| {
            let requested_path = root.join(relative);
            let before_path = before
                .keys()
                .find(|path| same_path(path, &requested_path))
                .cloned();
            let before_exists = before_path.is_some();
            let path = before_path.as_deref().unwrap_or(&requested_path);
            if !is_text_path(path) {
                return ProvenanceFileChange {
                    path: relative.clone(),
                    before_exists,
                    reason: Some("binary files are not supported yet".into()),
                    ..Default::default()
                };
            }
            let (_, after_checksum) = match read_current_text(path) {
                Ok(current) => current,
                Err(reason) => {
                    return ProvenanceFileChange {
                        path: relative.clone(),
                        before_exists,
                        reason: Some(reason.into()),
                        ..Default::default()
                    };
                }
            };
            if !before_exists {
                return ProvenanceFileChange {
                    path: relative.clone(),
                    before_exists: false,
                    after_checksum: Some(after_checksum),
                    reversible: true,
                    ..Default::default()
                };
            }
            let Some(preimage) = preimages.get(path) else {
                return ProvenanceFileChange {
                    path: relative.clone(),
                    before_exists: true,
                    after_checksum: Some(after_checksum),
                    reason: Some("pre-change text was not captured safely".into()),
                    ..Default::default()
                };
            };
            ProvenanceFileChange {
                path: relative.clone(),
                before_exists: true,
                before_text: Some(preimage.text.clone()),
                before_checksum: Some(preimage.checksum.clone()),
                after_checksum: Some(after_checksum),
                reversible: true,
                reason: None,
            }
        })
        .collect()
}

/// Add writes that an mtime-only workspace scan can miss.
///
/// Explicit file tools name their destination, while captured text preimages
/// let shell/Python/R calls prove a content change without trusting timestamp
/// granularity.
pub fn augment_written_paths(
    tool: &str,
    root: &Path,
    source: &str,
    success: bool,
    preimages: &BTreeMap<PathBuf, TextPreimage>,
    written: &mut Vec<String>,
) {
    let relative = |path: &Path| {
        path.strip_prefix(root)
            .ok()
            .filter(|path| {
                !path.as_os_str().is_empty()
                    && path
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
            })
            .map(|path| path.to_string_lossy().replace('\\', "/"))
    };
    for (path, preimage) in preimages {
        if read_current_text(path).is_ok_and(|(_, checksum)| checksum != preimage.checksum) {
            if let Some(path) = relative(path) {
                written.push(path);
            }
        }
    }
    if success && matches!(tool, "write" | "edit" | "generate_image" | "generate_video") {
        let source_path = Path::new(source);
        let path = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            root.join(source_path)
        };
        if path.is_file() && !preimages.keys().any(|before| same_path(before, &path)) {
            if let Some(path) = relative(&path) {
                written.push(path);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    written.retain(|path| seen.insert(path_identity(&root.join(path))));
    written.sort();
}

fn snapshot_capped(root: &Path, max_entries: usize) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0;
    'walk: while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if visited >= max_entries {
                break 'walk;
            }
            visited += 1;
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !SKIP_DIRS.contains(&name) {
                    stack.push(p);
                }
            } else if ft.is_file() {
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                out.insert(p, mtime);
            }
        }
    }
    out
}

/// Diff two snapshots → (files_written, files_read), both workspace-relative.
/// written = new or mtime-advanced files; read = pre-existing files (not also written)
/// whose relative path appears literally in `source`.
pub fn diff(
    before: &BTreeMap<PathBuf, SystemTime>,
    after: &BTreeMap<PathBuf, SystemTime>,
    root: &Path,
    source: &str,
) -> (Vec<String>, Vec<String>) {
    let rel = |p: &Path| -> String {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut written = Vec::new();
    for (p, mt) in after {
        match before.get(p) {
            None => written.push(rel(p)),
            Some(old) if mt > old => written.push(rel(p)),
            _ => {}
        }
    }
    written.sort();
    let wset: std::collections::HashSet<&String> = written.iter().collect();
    let normalized_source = normalize_path_text(source);
    let mut read = Vec::new();
    for p in before.keys() {
        let r = rel(p);
        if !r.is_empty()
            && !wset.contains(&r)
            && mentions_path_token(&normalized_source, &normalize_path_text(&r))
        {
            read.push(r);
        }
    }
    read.sort();
    (written, read)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test directory so parallel test runs never collide.
    fn unique_tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wisp_prov_{tag}_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_written_and_read_and_skips_git() {
        let tmp = unique_tmp("snap");
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join("data.csv"), b"x").unwrap();
        std::fs::write(tmp.join(".git/HEAD"), b"x").unwrap();
        let before = snapshot(&tmp);
        assert!(
            !before.keys().any(|p| p.ends_with("HEAD")),
            ".git must be skipped"
        );
        std::fs::write(tmp.join("out.png"), b"y").unwrap();
        let after = snapshot(&tmp);
        let (w, r) = diff(
            &before,
            &after,
            &tmp,
            "df=read_csv('data.csv'); savefig('out.png')",
        );
        assert!(w.contains(&"out.png".to_string()));
        assert!(r.contains(&"data.csv".to_string()));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn r_is_a_producing_language_with_code_source() {
        assert!(is_producing("r"));
        assert_eq!(language_of("r"), "r");
        assert_eq!(
            source_of("r", &serde_json::json!({"code": "png('plot.png')"})),
            "png('plot.png')"
        );
    }

    #[test]
    fn file_mutation_tools_are_producing_with_path_source() {
        for tool in ["write", "edit", "generate_image", "generate_video"] {
            assert!(is_producing(tool));
            assert_eq!(
                source_of(tool, &serde_json::json!({"path": "results/table.csv"})),
                "results/table.csv"
            );
        }
    }

    #[test]
    fn snapshot_caps_entries_inside_a_wide_directory() {
        let tmp = unique_tmp("wide");
        for name in ["a", "b", "c"] {
            std::fs::write(tmp.join(name), b"x").unwrap();
        }

        assert_eq!(snapshot_capped(&tmp, 2).len(), 2);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn undo_captures_existing_and_new_markdown_but_not_word() {
        let tmp = unique_tmp("undo_text");
        std::fs::write(tmp.join("notes.md"), "before\n").unwrap();
        let before = snapshot(&tmp);
        let preimages = capture_text_preimages(&before, &tmp, "edit notes.md");

        std::fs::write(tmp.join("notes.md"), "after\n").unwrap();
        std::fs::write(tmp.join("summary.md"), "new\n").unwrap();
        std::fs::write(tmp.join("paper.docx"), b"PK\x03\x04").unwrap();
        let written = vec!["notes.md".into(), "summary.md".into(), "paper.docx".into()];
        let changes = undo_file_changes(&before, &tmp, &written, &preimages);

        assert!(changes[0].reversible);
        assert_eq!(changes[0].before_text.as_deref(), Some("before\n"));
        assert!(changes[1].reversible);
        assert!(!changes[1].before_exists);
        assert!(!changes[2].reversible);
        assert!(changes[2].reason.as_deref().unwrap().contains("binary"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn existing_text_with_a_dynamic_destination_is_not_guessed() {
        let tmp = unique_tmp("undo_dynamic");
        std::fs::write(tmp.join("notes.md"), "before\n").unwrap();
        let before = snapshot(&tmp);
        let preimages = capture_text_preimages(&before, &tmp, "run generated destination");
        std::fs::write(tmp.join("notes.md"), "after\n").unwrap();
        let changes = undo_file_changes(&before, &tmp, &["notes.md".into()], &preimages);

        assert!(!changes[0].reversible);
        assert!(changes[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("not captured"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn overlapping_producing_windows_see_each_other() {
        let root = unique_tmp("windows_overlap");
        let first = begin_window(&root, Some("session-a"));
        let second = begin_window(&root, Some("session-b"));

        assert_eq!(second.finish().foreign.len(), 1);
        // The ended second window still overlapped the first one.
        assert_eq!(first.finish().foreign.len(), 1);

        // With everything closed, a later window starts clean.
        assert!(begin_window(&root, None).finish().foreign.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn windows_on_different_roots_never_overlap() {
        let root_a = unique_tmp("windows_root_a");
        let root_b = unique_tmp("windows_root_b");
        let a = begin_window(&root_a, None);
        let b = begin_window(&root_b, None);
        assert!(a.finish().foreign.is_empty());
        assert!(b.finish().foreign.is_empty());
        std::fs::remove_dir_all(&root_a).ok();
        std::fs::remove_dir_all(&root_b).ok();
    }

    #[test]
    fn dropping_a_window_without_finish_deregisters_it() {
        let root = unique_tmp("windows_drop");
        let abandoned = begin_window(&root, None);
        let survivor = begin_window(&root, None);
        drop(abandoned);
        assert_eq!(survivor.finish().foreign.len(), 1);
        assert!(begin_window(&root, None).finish().foreign.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// One conversation's parallel subagents share a scope, so their mutual
    /// overlap must not suppress each other's attribution (#911 case 5).
    /// Scopeless windows keep the safe default: everyone is foreign.
    #[test]
    fn same_scope_windows_are_not_foreign() {
        let root = unique_tmp("windows_scope");
        let parent = begin_window(&root, Some("conversation-1"));
        let sibling = begin_window(&root, Some("conversation-1"));
        let stranger = begin_window(&root, Some("conversation-2"));
        let scopeless = begin_window(&root, None);

        assert_eq!(sibling.finish().foreign.len(), 2);
        assert_eq!(parent.finish().foreign.len(), 2);
        assert_eq!(stranger.finish().foreign.len(), 3);
        assert_eq!(scopeless.finish().foreign.len(), 3);
        std::fs::remove_dir_all(&root).ok();
    }

    fn window_over(mtime: SystemTime, foreign: Vec<(SystemTime, SystemTime)>) -> FinishedWindow {
        FinishedWindow {
            started: mtime - Duration::from_secs(1),
            ended: mtime + Duration::from_secs(1),
            foreign,
        }
    }

    #[test]
    fn concurrent_overlap_keeps_only_provable_writes() {
        let tmp = unique_tmp("overlap_filter");
        std::fs::write(tmp.join("named.txt"), b"mine").unwrap();
        std::fs::write(tmp.join("foreign.png"), b"theirs").unwrap();
        std::fs::write(tmp.join("old_plot.png"), b"mine too").unwrap();
        let after = snapshot(&tmp);
        let mtime = after[&tmp.join("foreign.png")];

        // No foreign overlap: everything survives.
        let mut written = vec![
            "foreign.png".to_string(),
            "named.txt".to_string(),
            "old_plot.png".to_string(),
        ];
        retain_unambiguous_writes(
            &mut written,
            &after,
            &tmp,
            "open('named.txt','w')",
            &window_over(mtime, Vec::new()),
        );
        assert_eq!(written.len(), 3);

        // A foreign call was in flight when these files changed: only the
        // source-named file survives.
        let inside = window_over(
            mtime,
            vec![(
                mtime - Duration::from_secs(1),
                mtime + Duration::from_secs(1),
            )],
        );
        let mut written = vec!["foreign.png".to_string(), "named.txt".to_string()];
        retain_unambiguous_writes(&mut written, &after, &tmp, "open('named.txt','w')", &inside);
        assert_eq!(written, vec!["named.txt".to_string()]);

        // A change inside this call's own window but outside every foreign
        // window stays attributed.
        let outside = window_over(
            mtime,
            vec![(
                mtime + Duration::from_secs(60),
                mtime + Duration::from_secs(120),
            )],
        );
        let mut written = vec!["old_plot.png".to_string()];
        retain_unambiguous_writes(&mut written, &after, &tmp, "print('hi')", &outside);
        assert_eq!(written, vec!["old_plot.png".to_string()]);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// #911 case 3: files extracted with backdated mtimes fall outside every
    /// window, so a bystander call whose diff picked them up must not claim
    /// them — the mtime cannot prove this call wrote them.
    #[test]
    fn backdated_mtimes_are_not_credited_during_overlap() {
        let tmp = unique_tmp("overlap_backdated");
        std::fs::write(tmp.join("old_fig.png"), b"extracted").unwrap();
        let mut after = snapshot(&tmp);
        let now = SystemTime::now();
        let day_ago = now - Duration::from_secs(86_400);
        after.insert(tmp.join("old_fig.png"), day_ago);

        let window = window_over(
            now,
            vec![(now - Duration::from_secs(30), now + Duration::from_secs(30))],
        );
        let mut written = vec!["old_fig.png".to_string()];
        retain_unambiguous_writes(&mut written, &after, &tmp, "time.sleep(60)", &window);
        assert!(
            written.is_empty(),
            "a backdated mtime proves nothing about this call"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// #911 case 2: a Windows path escaped in program text
    /// (`open("out\\b_named.txt", ...)`) still counts as naming the file.
    #[test]
    fn escaped_backslash_paths_count_as_named() {
        let tmp = unique_tmp("overlap_backslash");
        std::fs::create_dir_all(tmp.join("out")).unwrap();
        std::fs::write(tmp.join("out/b_named.txt"), b"backslash").unwrap();
        let after = snapshot(&tmp);
        let mtime = after[&tmp.join("out/b_named.txt")];

        let window = window_over(mtime, vec![(mtime, mtime + Duration::from_secs(60))]);
        let mut written = vec!["out/b_named.txt".to_string()];
        retain_unambiguous_writes(
            &mut written,
            &after,
            &tmp,
            r#"open("out\\b_named.txt", "w").write("backslash")"#,
            &window,
        );
        assert_eq!(written, vec!["out/b_named.txt".to_string()]);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// #911 case 4: naming `final_results.csv` must not also claim
    /// `results.csv` written by a concurrent foreign call.
    #[test]
    fn substring_of_a_named_path_is_not_named() {
        let tmp = unique_tmp("overlap_substring");
        std::fs::write(tmp.join("final_results.csv"), b"A").unwrap();
        std::fs::write(tmp.join("results.csv"), b"B").unwrap();
        let after = snapshot(&tmp);
        let mtime = after[&tmp.join("results.csv")];

        let window = window_over(
            mtime,
            vec![(
                mtime - Duration::from_secs(1),
                mtime + Duration::from_secs(1),
            )],
        );
        let mut written = vec!["final_results.csv".to_string(), "results.csv".to_string()];
        retain_unambiguous_writes(
            &mut written,
            &after,
            &tmp,
            "open('final_results.csv','w').write('A')",
            &window,
        );
        assert_eq!(written, vec!["final_results.csv".to_string()]);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// #937: a computed-name path dropped by foreign overlap is restored
    /// when the kernel reports it; the literal-named sibling stays too.
    #[test]
    fn kernel_reported_computed_name_survives_foreign_overlap() {
        let tmp = unique_tmp("937_repro");
        std::fs::write(tmp.join("fig_1.png"), b"computed").unwrap();
        std::fs::write(tmp.join("fig_2.png"), b"literal").unwrap();
        let after = snapshot(&tmp);
        let mtime = after[&tmp.join("fig_1.png")];
        let window = window_over(
            mtime,
            vec![(
                mtime - Duration::from_secs(1),
                mtime + Duration::from_secs(1),
            )],
        );
        let mut written = vec!["fig_1.png".to_string(), "fig_2.png".to_string()];
        retain_unambiguous_writes(
            &mut written,
            &after,
            &tmp,
            "open('fig_2.png','w'); name = f'fig_{1}.png'; open(name,'w')",
            &window,
        );
        assert_eq!(written, vec!["fig_2.png".to_string()]);
        union_reported_writes(&mut written, &["fig_1.png".to_string()]);
        assert_eq!(
            written,
            vec!["fig_1.png".to_string(), "fig_2.png".to_string()]
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Reports must not weaken #935 for paths they do not cover.
    #[test]
    fn unreported_ambiguous_path_stays_dropped() {
        let tmp = unique_tmp("937_control");
        std::fs::write(tmp.join("fig_1.png"), b"computed").unwrap();
        std::fs::write(tmp.join("fig_2.png"), b"literal").unwrap();
        let after = snapshot(&tmp);
        let mtime = after[&tmp.join("fig_1.png")];
        let window = window_over(
            mtime,
            vec![(
                mtime - Duration::from_secs(1),
                mtime + Duration::from_secs(1),
            )],
        );
        let mut written = vec!["fig_1.png".to_string(), "fig_2.png".to_string()];
        retain_unambiguous_writes(
            &mut written,
            &after,
            &tmp,
            "open('fig_2.png','w'); name = f'fig_{1}.png'; open(name,'w')",
            &window,
        );
        assert_eq!(written, vec!["fig_2.png".to_string()]);
        union_reported_writes(&mut written, &[]);
        assert_eq!(written, vec!["fig_2.png".to_string()]);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A report bypasses the snapshot, so every directory the snapshot skips
    /// must be skipped here too. `__pycache__` is the one that fires in
    /// ordinary use: importing a project-local module writes bytecode the
    /// diff would never have shown.
    #[test]
    fn reported_writes_under_skipped_dirs_never_enter_the_record() {
        let mut written = vec!["out/fig.png".to_string()];
        union_reported_writes(
            &mut written,
            &[
                "__pycache__/helper.cpython-312.pyc".to_string(),
                "src/__pycache__/helper.cpython-312.pyc".to_string(),
                ".venv/lib/python3.12/site-packages/pkg/mod.py".to_string(),
                ".git/index".to_string(),
                ".wisp/tool-output/spill.txt".to_string(),
                "uploads/raw.csv".to_string(),
                "node_modules/x/index.js".to_string(),
                "results/table.csv".to_string(),
            ],
        );
        assert_eq!(
            written,
            vec!["out/fig.png".to_string(), "results/table.csv".to_string()]
        );
    }

    /// Only parent directories gate inclusion. A file whose own name matches
    /// a skipped directory is a normal project file.
    #[test]
    fn reported_leaf_named_like_a_skipped_dir_is_kept() {
        let mut written = Vec::new();
        union_reported_writes(
            &mut written,
            &["uploads".to_string(), "notes/.git".to_string()],
        );
        assert_eq!(
            written,
            vec!["notes/.git".to_string(), "uploads".to_string()]
        );
    }

    #[test]
    fn union_keeps_diff_spelling_for_the_same_identity() {
        let mut written = vec!["out/b.txt".to_string()];
        union_paths_by_identity(&mut written, &[r"out\b.txt".to_string()]);
        assert_eq!(written, vec!["out/b.txt".to_string()]);
    }

    #[test]
    fn all_duplicate_report_leaves_written_byte_identical() {
        let mut written = vec![
            "c.txt".to_string(),
            "a.txt".to_string(),
            r"b\d.txt".to_string(),
        ];
        let before = written.clone();
        union_paths_by_identity(&mut written, &["a.txt".to_string(), "b/d.txt".to_string()]);
        assert_eq!(written, before);
    }

    #[test]
    fn empty_report_leaves_written_byte_identical() {
        let mut written = vec![
            "a.txt".to_string(),
            "c.txt".to_string(),
            "b.txt".to_string(),
        ];
        let before = written.clone();
        union_paths_by_identity(&mut written, &[]);
        assert_eq!(written, before);
    }

    #[test]
    fn path_token_matching_respects_boundaries() {
        assert!(mentions_path_token("open('results.csv')", "results.csv"));
        assert!(mentions_path_token("save ./results.csv now", "results.csv"));
        assert!(mentions_path_token("open(\"out/b.txt\")", "out/b.txt"));
        assert!(!mentions_path_token(
            "open('final_results.csv')",
            "results.csv"
        ));
        assert!(!mentions_path_token("results.csv.bak", "results.csv"));
        assert!(!mentions_path_token("myout/b.txt", "out/b.txt"));
        assert!(!mentions_path_token("nested/out/b.txt", "out/b.txt"));
        assert!(!mentions_path_token("anything", ""));
        // Escaped separators normalize to the same token.
        assert!(mentions_path_token(
            &normalize_path_text(r#"open("out\\b.txt")"#),
            &normalize_path_text("out/b.txt")
        ));
    }

    #[test]
    fn changed_preimages_do_not_depend_on_mtime_resolution() {
        let tmp = unique_tmp("undo_timestamp");
        std::fs::write(tmp.join("notes.md"), "before\n").unwrap();
        let before = snapshot(&tmp);
        let preimages = capture_text_preimages(&before, &tmp, "notes.md");
        std::fs::write(tmp.join("notes.md"), "after\n").unwrap();

        let mut written = Vec::new();
        augment_written_paths("shell", &tmp, "notes.md", true, &preimages, &mut written);
        assert_eq!(written, vec!["notes.md"]);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
