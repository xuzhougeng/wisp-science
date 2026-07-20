//! Windows-aware command safety + path sandbox.
//!
//! Replaces mangopi-cli's POSIX `_check_command_safety` / `_validate_file_path`
//! with PowerShell + cmd semantics. The dangerous-command list is intentionally
//! pattern-based and conservative: anything matching asks for confirmation
//! rather than silently running.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Reason category for a flagged command. Order matters only for messaging.
#[derive(Debug, Clone, Copy)]
pub enum Danger {
    Delete,
    Disk,
    Perms,
    Privilege,
    Process,
    Env,
    History,
    DownloadExec,
    Registry,
}

struct Rule {
    re: Regex,
    danger: Danger,
}

fn rules() -> &'static [Rule] {
    static R: OnceLock<Vec<Rule>> = OnceLock::new();
    R.get_or_init(|| {
        let mk = |pat: &str, d: Danger| Rule {
            re: Regex::new(pat).expect("danger regex"),
            danger: d,
        };
        vec![
            // Deletion
            mk(r"(?i)\bremove-item\b.*-r(ecurse)?", Danger::Delete),
            mk(r"(?i)\brmdir\b.*/s\b", Danger::Delete),
            mk(r"(?i)\brd\b.*/s\b", Danger::Delete),
            mk(r"(?i)\bdel\b.*/[sf]", Danger::Delete),
            mk(r"(?i)\berase\b.*/[sf]", Danger::Delete),
            mk(r"(?i)\brm\b\s+(-rf|--recursive|--force)", Danger::Delete),
            mk(r"(?i)\bunlink\b", Danger::Delete),
            // Disk / partition
            mk(r"(?i)\bformat\b", Danger::Disk),
            mk(r"(?i)\bdiskpart\b", Danger::Disk),
            mk(r"(?i)\bcd\b\s+[a-z]:\\", Danger::Disk), // naive drive switch — warn only on destructive below
            mk(r"(?i)\b(icacls|takeown|cacls)\b", Danger::Perms),
            mk(r"(?i)\bicacls\b.*everyone", Danger::Perms),
            // Privilege
            mk(r"(?i)\brunas\b", Danger::Privilege),
            mk(
                r"(?i)\bnet\b+(localgroup|user)\b.*administrators",
                Danger::Privilege,
            ),
            mk(r"(?i)\bsudo\b", Danger::Privilege),
            // Process control
            mk(r"(?i)\btaskkill\b.*/f\b", Danger::Process),
            mk(r"(?i)\bstop-process\b.*-force", Danger::Process),
            mk(r"(?i)\bkill\s+-9", Danger::Process),
            // Env / system config
            mk(r"(?i)\bset-executionpolicy\b", Danger::Env),
            mk(r"(?i)\bsetx\b\s+(path|windir|systemroot)", Danger::Env),
            mk(r"(?i)\[environment\]::setenvironmentvariable", Danger::Env),
            // History / log clearing
            mk(r"(?i)\bclear-history\b", Danger::History),
            mk(r"(?i)\bwevtutil\b\s+cl\b", Danger::History),
            mk(r"(?i)\bremove-item\b.*eventlog", Danger::History),
            // Download-and-execute patterns
            mk(r"(?i)\biex\b|invoke-expression", Danger::DownloadExec),
            mk(r"(?i)\birm\b.*\|\s*iex", Danger::DownloadExec),
            mk(r"(?i)\biwr\b.*\|\s*iex", Danger::DownloadExec),
            mk(
                r"(?i)\b(curl|wget|iwr|irm)\b.*\|\s*(sh|bash|cmd)",
                Danger::DownloadExec,
            ),
            mk(
                r"(?i)-enc(odedcommand)?\s+[A-Za-z0-9+/=]{40,}",
                Danger::DownloadExec,
            ),
            // Registry
            mk(r"(?i)\breg\b\s+(delete|add)\b", Danger::Registry),
        ]
    })
}

pub fn check_command_safety(command: &str) -> Option<Danger> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return None;
    }
    for r in rules() {
        if r.re.is_match(cmd) {
            return Some(r.danger);
        }
    }
    None
}

impl Danger {
    pub fn label(&self) -> &'static str {
        match self {
            Danger::Delete => "File / directory deletion",
            Danger::Disk => "Disk formatting or partition",
            Danger::Perms => "Permission change",
            Danger::Privilege => "Privilege escalation",
            Danger::Process => "Dangerous process control",
            Danger::Env => "Environment or system config change",
            Danger::History => "History / log clearing",
            Danger::DownloadExec => "Download-and-execute pattern",
            Danger::Registry => "Registry modification",
        }
    }
}

/// Resolve `path` and ensure it lives under `root`. Returns an error message
/// string when the path escapes the sandbox or names a directory (write/edit
/// are file-only, matching mangopi's rule).
pub fn validate_file_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    // dunce strips the `\\?\` prefix canonicalize adds on Windows, so string
    // starts_with checks below behave.
    let real = match dunce::canonicalize(&abs) {
        Ok(r) => r,
        Err(_) => {
            // Target doesn't exist yet (write). Canonicalize the parent and
            // append the file name, then verify the parent is under root.
            let parent = abs.parent().unwrap_or(Path::new(""));
            let file = abs.file_name().map(PathBuf::from).unwrap_or_default();
            let parent_real = dunce::canonicalize(parent)
                .map_err(|e| format!("path '{path}' parent not resolvable: {e}"))?;
            parent_real.join(file)
        }
    };
    let root_real = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !real.starts_with(&root_real) {
        return Err(format!("path '{}' is outside project root", path));
    }
    if real.is_dir() {
        return Err(format!("path '{}' is a directory, not a file", path));
    }
    Ok(real)
}

/// Resolve `path` under `root`, allowing directories (for `list_dir`).
pub fn resolve_under_root(root: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    let real = dunce::canonicalize(&abs).map_err(|e| format!("path '{path}' not found: {e}"))?;
    let root_real = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !real.starts_with(&root_real) {
        return Err(format!("path '{path}' is outside project root"));
    }
    Ok(real)
}

pub fn validate_relative_pattern(pattern: &str) -> Result<(), String> {
    use std::path::Component;
    let path = Path::new(pattern);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("pattern '{pattern}' escapes the project root"));
    }
    Ok(())
}

/// Whether a command looks like a heavy directory traversal whose output we
/// should filter (mangopi's `_is_directory_heavy`), Windows flavor.
pub fn is_directory_heavy(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    [
        "get-childitem -r",
        "tree ",
        "dir /s",
        "ls -r",
        "find ",
        "rg ",
        "fd ",
        "du ",
    ]
    .iter()
    .any(|k| c.contains(k))
}

/// Directories to drop from heavy directory listings.
pub const FILTERED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".idea",
    ".vscode",
    ".mypy_cache",
    ".pytest_cache",
    ".cache",
    "target",
    "vendor",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegated_glob_patterns_cannot_escape_the_project() {
        assert!(validate_relative_pattern("src/**/*.rs").is_ok());
        assert!(validate_relative_pattern("../**/*").is_err());
        assert!(validate_relative_pattern(&std::env::temp_dir().to_string_lossy()).is_err());
    }
}

/// Drop lines that name a filtered directory, then cap to `max_lines`.
pub fn filter_directory_output(lines: &[String], max_lines: usize) -> Vec<String> {
    let mut kept: Vec<String> = lines
        .iter()
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            !FILTERED_DIRS.iter().any(|d| {
                lower.contains(&format!("/{d}/"))
                    || lower.contains(&format!("/{d}\\"))
                    || lower.contains(&format!("\\{d}\\"))
                    || lower.trim_start_matches('.').starts_with(d)
            })
        })
        .cloned()
        .collect();
    if kept.len() > max_lines {
        let n = kept.len() - max_lines;
        kept.truncate(max_lines);
        kept.push(String::new());
        kept.push(format!("... truncated {n} lines ..."));
    }
    kept
}
