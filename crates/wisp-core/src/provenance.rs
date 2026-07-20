//! Best-effort artifact provenance: snapshot the workspace around a producing
//! tool call and diff to learn which files it wrote and read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
}

const SKIP_DIRS: &[&str] = &[
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
    matches!(tool, "python" | "r" | "shell" | "write" | "edit")
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
        "write" | "edit" => "path",
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
    let mut read = Vec::new();
    for p in before.keys() {
        let r = rel(p);
        if !r.is_empty() && !wset.contains(&r) && source.contains(&r) {
            read.push(r);
        }
    }
    read.sort();
    (written, read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_written_and_read_and_skips_git() {
        let tmp = std::env::temp_dir().join("wisp_prov_snap_test");
        std::fs::remove_dir_all(&tmp).ok();
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
        for tool in ["write", "edit"] {
            assert!(is_producing(tool));
            assert_eq!(
                source_of(tool, &serde_json::json!({"path": "results/table.csv"})),
                "results/table.csv"
            );
        }
    }

    #[test]
    fn snapshot_caps_entries_inside_a_wide_directory() {
        let tmp = std::env::temp_dir().join("wisp_prov_wide_test");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(tmp.join(name), b"x").unwrap();
        }

        assert_eq!(snapshot_capped(&tmp, 2).len(), 2);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
