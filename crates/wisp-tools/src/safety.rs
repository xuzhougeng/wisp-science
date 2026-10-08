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
            mk(
                r"(?i)\brm\b(?:\s+[^\s;&|]+)*?\s+(?:-[a-z]*[rf][a-z]*|--recursive|--force|--no-preserve-root)\b",
                Danger::Delete,
            ),
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
            // Canonicalize failed: either the target doesn't exist yet (a new
            // write) or it is a symlink whose target doesn't resolve. A
            // dangling link must be rejected here — the parent+file fallback
            // below can't see where the link points, and a follow-on write
            // would create the link's target, possibly outside the root.
            if abs
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(format!(
                    "path '{path}' is a symlink whose target cannot be resolved"
                ));
            }
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

/// Overwrite `path` without following a symlink (Unix) or reparse point
/// (Windows) in the final component. `validate_file_path` already resolves
/// links at check time; this closes the check-to-write window in which the
/// validated path could be swapped for a link pointing outside the root.
pub fn write_no_follow(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = open_no_follow(path)?;
    file.set_len(0)?;
    file.write_all(content)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| {
            // O_NOFOLLOW reports a symlink final component as ELOOP (Linux,
            // macOS) or EMLINK (some BSDs); surface a clearer message.
            if path
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                std::io::Error::other(format!(
                    "refusing to write through symlink '{}'",
                    path.display()
                ))
            } else {
                e
            }
        })
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    // Not in windows-sys' enabled feature set; values are ABI-stable.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    // The flag opened the reparse point itself rather than its target, so
    // checking the handle (not the path) is race-free.
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::other(format!(
            "refusing to write through reparse point '{}'",
            path.display()
        )));
    }
    Ok(file)
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

    /// Unique per-test sandbox so parallel tests never share a directory.
    fn unique_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wisp_safety_{tag}_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recursive_rm_spellings_all_trip_the_delete_gate() {
        for cmd in [
            "rm -rf x",
            "rm -r x",
            "rm -fr x",
            "rm -R x",
            "rm -r -f x",
            "rm -rv x",
            "rm x -rf",
            "rm --recursive x",
            "rm --force x",
            "cd a && rm -r b",
        ] {
            assert!(
                matches!(check_command_safety(cmd), Some(Danger::Delete)),
                "{cmd}"
            );
        }
        for cmd in ["rm x", "rm -i x", "ls -r x", "rm x && grep -r y z"] {
            assert!(check_command_safety(cmd).is_none(), "{cmd}");
        }
    }

    #[test]
    fn delegated_glob_patterns_cannot_escape_the_project() {
        assert!(validate_relative_pattern("src/**/*.rs").is_ok());
        assert!(validate_relative_pattern("../**/*").is_err());
        assert!(validate_relative_pattern(&std::env::temp_dir().to_string_lossy()).is_err());
    }

    #[test]
    fn validate_file_path_enforces_the_sandbox_boundary() {
        let root = unique_dir("vfp");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("inside.txt"), "in").unwrap();
        std::fs::write(root.join("sub/nested.txt"), "in").unwrap();
        let outside = unique_dir("vfp_outside");
        std::fs::write(outside.join("secret.txt"), "out").unwrap();
        let root_real = dunce::canonicalize(&root).unwrap();

        struct Case {
            name: &'static str,
            path: String,
            expect_ok: bool,
        }
        let cases = vec![
            Case {
                name: "relative path inside root accepted",
                path: "inside.txt".into(),
                expect_ok: true,
            },
            Case {
                name: "absolute path inside root accepted",
                path: root.join("sub/nested.txt").to_string_lossy().into_owned(),
                expect_ok: true,
            },
            Case {
                name: "../ traversal escape rejected",
                path: format!(
                    "../{}/secret.txt",
                    outside.file_name().unwrap().to_string_lossy()
                ),
                expect_ok: false,
            },
            Case {
                name: "absolute path outside root rejected",
                path: outside.join("secret.txt").to_string_lossy().into_owned(),
                expect_ok: false,
            },
            Case {
                name: "new file with an existing parent accepted (write target)",
                path: "sub/not_yet_written.txt".into(),
                expect_ok: true,
            },
            Case {
                name: "new file whose parent does not exist rejected",
                path: "missing_dir/new.txt".into(),
                expect_ok: false,
            },
            Case {
                name: "directory target rejected (write/edit are file-only)",
                path: "sub".into(),
                expect_ok: false,
            },
        ];
        for case in cases {
            let got = validate_file_path(&root, &case.path);
            assert_eq!(got.is_ok(), case.expect_ok, "{}: {:?}", case.name, got);
            if let Ok(resolved) = got {
                assert!(
                    resolved.starts_with(&root_real),
                    "{}: resolved path {resolved:?} must stay under root",
                    case.name
                );
            }
        }

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn resolve_under_root_allows_directories_but_not_escapes() {
        let root = unique_dir("rur");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("inside.txt"), "in").unwrap();
        let outside = unique_dir("rur_outside");
        std::fs::write(outside.join("secret.txt"), "out").unwrap();

        assert!(resolve_under_root(&root, "inside.txt").is_ok());
        assert!(resolve_under_root(&root, "sub").is_ok(), "dirs are allowed");
        assert!(
            resolve_under_root(&root, &root.join("sub").to_string_lossy()).is_ok(),
            "absolute path inside root accepted"
        );
        assert!(
            resolve_under_root(&root, &outside.join("secret.txt").to_string_lossy()).is_err(),
            "absolute path outside root rejected"
        );
        assert!(
            resolve_under_root(
                &root,
                &format!(
                    "../{}/secret.txt",
                    outside.file_name().unwrap().to_string_lossy()
                )
            )
            .is_err(),
            "../ traversal escape rejected"
        );
        assert!(
            resolve_under_root(&root, "does_not_exist.txt").is_err(),
            "nonexistent paths are rejected (list/read need an existing target)"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_rejected_instead_of_using_the_parent_fallback() {
        let root = unique_dir("dangling");
        let outside = unique_dir("dangling_outside");
        // Link target does not exist, so canonicalize fails and the old
        // parent+file fallback would have let the path through.
        std::os::unix::fs::symlink(outside.join("not_yet.txt"), root.join("dangling.txt")).unwrap();
        std::os::unix::fs::symlink(root.join("also_missing.txt"), root.join("dangling_in.txt"))
            .unwrap();

        for name in ["dangling.txt", "dangling_in.txt"] {
            let got = validate_file_path(&root, name);
            assert!(
                got.as_ref().is_err_and(|e| e.contains("symlink")),
                "{name}: dangling links must be rejected: {got:?}"
            );
        }

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_no_follow_refuses_symlinks_but_overwrites_regular_files() {
        let root = unique_dir("nofollow");
        let outside = unique_dir("nofollow_outside");
        std::fs::write(root.join("plain.txt"), "long original contents").unwrap();
        std::os::unix::fs::symlink(outside.join("target.txt"), root.join("link.txt")).unwrap();

        // Simulates the path being swapped for a link after validation.
        let err = write_no_follow(&root.join("link.txt"), b"payload").unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(
            !outside.join("target.txt").exists(),
            "no-follow write must not create the link target"
        );

        write_no_follow(&root.join("plain.txt"), b"short").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("plain.txt")).unwrap(),
            "short",
            "overwrite must truncate previous longer contents"
        );
        write_no_follow(&root.join("created.txt"), b"new file").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("created.txt")).unwrap(),
            "new file"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_pointing_outside_the_root_is_rejected() {
        let root = unique_dir("symlink");
        let outside = unique_dir("symlink_outside");
        std::fs::write(outside.join("secret.txt"), "out").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("sneaky.txt")).unwrap();

        let via_validate = validate_file_path(&root, "sneaky.txt");
        assert!(
            via_validate.is_err(),
            "validate_file_path must resolve symlinks: {via_validate:?}"
        );
        let via_resolve = resolve_under_root(&root, "sneaky.txt");
        assert!(
            via_resolve.is_err(),
            "resolve_under_root must resolve symlinks: {via_resolve:?}"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
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
