//! Deterministic, symlink-safe workspace traversal shared by project transfer
//! and persistent exploration snapshots.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) const MAX_WORKSPACE_ENTRIES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceNodeKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceNode {
    pub path: PathBuf,
    pub relative_path: String,
    pub kind: WorkspaceNodeKind,
    pub size_bytes: u64,
    pub mode: Option<u32>,
    pub modified_unix_millis: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceScanOptions {
    pub excluded_roots: Vec<PathBuf>,
    pub excluded_relative_prefixes: Vec<String>,
    pub excluded_directory_names: Vec<String>,
    pub use_ignore_files: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    pub max_entries: usize,
}

impl Default for WorkspaceScanOptions {
    fn default() -> Self {
        Self {
            excluded_roots: Vec::new(),
            excluded_relative_prefixes: Vec::new(),
            excluded_directory_names: Vec::new(),
            use_ignore_files: false,
            cancel: None,
            max_entries: MAX_WORKSPACE_ENTRIES,
        }
    }
}

pub(crate) fn scan_workspace(
    root: &Path,
    options: &WorkspaceScanOptions,
) -> Result<Vec<WorkspaceNode>, String> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|error| {
        format!(
            "project directory does not exist: {} ({error})",
            root.display()
        )
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(format!(
            "project directory must be a real directory: {}",
            root.display()
        ));
    }

    let excluded_roots = options
        .excluded_roots
        .iter()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect::<Vec<_>>();
    let mut excluded_prefixes = options
        .excluded_relative_prefixes
        .iter()
        .map(|prefix| root.join(prefix.trim_matches('/')))
        .collect::<Vec<_>>();
    // Live persistence is snapshotted by the store, never copied by a file
    // walker (including WAL/SHM and crash-recovery files).
    excluded_prefixes.extend(
        [
            ".wisp/project.sqlite",
            ".wisp/project.sqlite-wal",
            ".wisp/project.sqlite-shm",
            ".wisp/project.sqlite-journal",
            ".wisp/project.json",
            ".wisp/storage-migration.json",
        ]
        .into_iter()
        .map(|path| root.join(path)),
    );
    let excluded_directory_names = options
        .excluded_directory_names
        .iter()
        .map(OsString::from)
        .collect::<HashSet<_>>();

    let mut builder = ignore::WalkBuilder::new(root);
    builder.standard_filters(false).follow_links(false);
    if options.use_ignore_files {
        builder
            .hidden(false)
            .parents(false)
            .ignore(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(false)
            .require_git(false)
            .add_custom_ignore_filename(".wispignore");
    }
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        let path = entry.path();
        if path
            .parent()
            .is_some_and(|parent| parent.file_name().is_some_and(|name| name == ".wisp"))
            && ["project-migration-", "storage-write-"]
                .iter()
                .any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix))
        {
            return false;
        }
        if excluded_roots
            .iter()
            .any(|excluded| path == excluded || path.starts_with(excluded))
            || excluded_prefixes
                .iter()
                .any(|excluded| path == excluded || path.starts_with(excluded))
        {
            return false;
        }
        !entry.file_type().is_some_and(|kind| kind.is_dir())
            || !excluded_directory_names.contains(entry.file_name())
    });

    let mut nodes = Vec::new();
    for entry in builder.build() {
        if options
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err("workspace scan cancelled".into());
        }
        let entry = entry.map_err(|error| format!("cannot scan workspace: {error}"))?;
        if entry.depth() == 0 {
            continue;
        }
        if nodes.len() >= options.max_entries {
            return Err(format!(
                "workspace contains more than {} entries",
                options.max_entries
            ));
        }
        let path = entry.path().to_path_buf();
        let relative_path = portable_relative(root, &path)?;
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let kind = if metadata.file_type().is_symlink() {
            WorkspaceNodeKind::Symlink
        } else if metadata.is_file() {
            WorkspaceNodeKind::File
        } else if metadata.is_dir() {
            WorkspaceNodeKind::Directory
        } else {
            WorkspaceNodeKind::Other
        };
        nodes.push(WorkspaceNode {
            path: path.clone(),
            relative_path,
            kind,
            size_bytes: metadata.len(),
            mode: file_mode(&metadata),
            modified_unix_millis: modified_unix_millis(&metadata),
        });
    }
    nodes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(nodes)
}

fn portable_relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "workspace entry escaped the project root".to_string())?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str().ok_or_else(|| {
                "project paths must be valid Unicode to move between operating systems".to_string()
            })?),
            _ => return Err("workspace contains a non-portable path".into()),
        }
    }
    Ok(parts.join("/"))
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

fn modified_unix_millis(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_database_sidecars_are_excluded_but_project_artifacts_remain() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".wisp/artifacts")).unwrap();
        for name in [
            "project.sqlite",
            "project.sqlite-wal",
            "project.sqlite-shm",
            "project.sqlite-journal",
            "project.json",
            "storage-migration.json",
            "project-migration-incomplete.sqlite",
        ] {
            std::fs::write(root.path().join(".wisp").join(name), b"private persistence").unwrap();
        }
        std::fs::write(root.path().join(".wisp/artifacts/plot.svg"), b"figure").unwrap();
        let nodes = scan_workspace(root.path(), &WorkspaceScanOptions::default()).unwrap();
        let files: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == WorkspaceNodeKind::File)
            .map(|n| n.relative_path.as_str())
            .collect();
        assert_eq!(files, vec![".wisp/artifacts/plot.svg"]);
    }

    #[test]
    fn scan_is_sorted_and_does_not_follow_symlinks() {
        let root =
            std::env::temp_dir().join(format!("wisp_workspace_scan_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("z.txt"), b"z").unwrap();
        std::fs::write(root.join("nested/a.txt"), b"a").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("nested"), root.join("link")).unwrap();

        let nodes = scan_workspace(&root, &WorkspaceScanOptions::default()).unwrap();
        let paths = nodes
            .iter()
            .map(|node| node.relative_path.as_str())
            .collect::<Vec<_>>();
        #[cfg(unix)]
        assert_eq!(paths, vec!["link", "nested", "nested/a.txt", "z.txt"]);
        #[cfg(not(unix))]
        assert_eq!(paths, vec!["nested", "nested/a.txt", "z.txt"]);
        #[cfg(unix)]
        assert_eq!(nodes[0].kind, WorkspaceNodeKind::Symlink);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_enforces_limits_and_relative_prefix_exclusions() {
        let root = std::env::temp_dir().join(format!(
            "wisp_workspace_scan_limit_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join(".git/objects/blob"), b"hidden").unwrap();
        std::fs::write(root.join("visible"), b"visible").unwrap();

        let nodes = scan_workspace(
            &root,
            &WorkspaceScanOptions {
                excluded_relative_prefixes: vec![".git".into()],
                ..WorkspaceScanOptions::default()
            },
        )
        .unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].relative_path, "visible");

        let error = scan_workspace(
            &root,
            &WorkspaceScanOptions {
                max_entries: 1,
                ..WorkspaceScanOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("more than 1 entries"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_honors_gitignore_wispignore_and_generated_directory_exclusions() {
        let root = std::env::temp_dir().join(format!(
            "wisp_workspace_scan_ignore_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("private")).unwrap();
        std::fs::create_dir_all(root.join("nested/.pixi/cache")).unwrap();
        std::fs::write(root.join(".gitignore"), b"ignored.txt\n*.tmp\n!keep.tmp\n").unwrap();
        std::fs::write(root.join(".wispignore"), b"private/\n").unwrap();
        std::fs::write(root.join("ignored.txt"), b"ignored").unwrap();
        std::fs::write(root.join("drop.tmp"), b"ignored").unwrap();
        std::fs::write(root.join("keep.tmp"), b"kept").unwrap();
        std::fs::write(root.join("private/secret.txt"), b"ignored").unwrap();
        std::fs::write(root.join("nested/.pixi/cache/package"), b"ignored").unwrap();
        std::fs::write(root.join("visible.txt"), b"visible").unwrap();

        let nodes = scan_workspace(
            &root,
            &WorkspaceScanOptions {
                excluded_directory_names: vec![".pixi".into()],
                use_ignore_files: true,
                ..WorkspaceScanOptions::default()
            },
        )
        .unwrap();
        let paths = nodes
            .iter()
            .map(|node| node.relative_path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&".gitignore"));
        assert!(paths.contains(&".wispignore"));
        assert!(paths.contains(&"keep.tmp"));
        assert!(paths.contains(&"visible.txt"));
        assert!(!paths.contains(&"ignored.txt"));
        assert!(!paths.contains(&"drop.tmp"));
        assert!(!paths.iter().any(|path| path.starts_with("private")));
        assert!(!paths.iter().any(|path| path.contains(".pixi")));
        let _ = std::fs::remove_dir_all(root);
    }
}
