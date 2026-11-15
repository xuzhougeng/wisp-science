use anyhow::{ensure, Result};
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use wisp_dto::{native_files as dto, DirEntry, FileContent};

pub(crate) async fn resolve(root: &Path, path: &str) -> Result<PathBuf> {
    ensure!(
        !path.contains('\\') && !Path::new(path).is_absolute(),
        "Use a project-relative path"
    );
    ensure!(
        Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "Invalid project path"
    );
    ensure!(
        !Path::new(path)
            .components()
            .any(|c| c.as_os_str() == ".wisp" || c.as_os_str() == ".git"),
        "Internal project metadata is not available remotely"
    );
    let root = tokio::fs::canonicalize(root).await?;
    let resolved = tokio::fs::canonicalize(root.join(path)).await?;
    ensure!(
        resolved.starts_with(&root),
        "Path leaves the project workspace"
    );
    let relative = resolved.strip_prefix(&root)?;
    ensure!(
        !relative
            .components()
            .any(|c| c.as_os_str() == ".wisp" || c.as_os_str() == ".git"),
        "Internal project metadata is not available remotely"
    );
    Ok(resolved)
}

/// Where things the user deletes from the file browser go, inside the
/// project's own metadata folder.
const TRASH: &str = ".wisp/trash";

/// A name for one file or folder: a single path component that is not
/// Wisp's or git's own folder.
fn entry_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give it a name");
    ensure!(
        name.len() <= 255
            && !name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\'),
        "A name is one line of at most 255 bytes without slashes"
    );
    ensure!(name != "." && name != "..", "That name is not allowed");
    ensure!(
        name != ".wisp" && name != ".git",
        "That name is kept for the project's own records"
    );
    Ok(name)
}

/// The entry `path` names, without following a link in its last part: the
/// folder it is in must be inside the project, and the entry is then
/// whatever is there, a link included.
async fn entry(root: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Name a file or a folder"))?;
    let name = entry_name(name)?;
    let parent = relative
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or_default();
    Ok(resolve(root, parent).await?.join(name))
}

async fn relative(root: &Path, path: &Path) -> Result<String> {
    let root = tokio::fs::canonicalize(root).await?;
    Ok(path
        .strip_prefix(&root)?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

/// Makes an empty folder or file named `name` in the project folder
/// `parent`, and returns its project-relative path. Nothing that exists is
/// replaced.
pub async fn create(root: &Path, parent: &str, name: &str, folder: bool) -> Result<String> {
    let parent = resolve(root, parent).await?;
    ensure!(
        tokio::fs::metadata(&parent).await?.is_dir(),
        "That is not a folder"
    );
    let target = parent.join(entry_name(name)?);
    ensure!(
        tokio::fs::symlink_metadata(&target).await.is_err(),
        "Something with that name is already there"
    );
    if folder {
        tokio::fs::create_dir(&target).await?;
    } else {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .await?;
    }
    relative(root, &target).await
}

/// Gives the file or folder at `path` a new name in the same folder, and
/// returns its new project-relative path. Nothing that exists is replaced.
pub async fn rename(root: &Path, path: &str, name: &str) -> Result<String> {
    let source = entry(root, path).await?;
    ensure!(
        tokio::fs::symlink_metadata(&source).await.is_ok(),
        "That file is no longer there"
    );
    let target = source
        .parent()
        .ok_or_else(|| anyhow::anyhow!("The project folder itself cannot be renamed"))?
        .join(entry_name(name)?);
    if target != source {
        ensure!(
            tokio::fs::symlink_metadata(&target).await.is_err(),
            "Something with that name is already there"
        );
        tokio::fs::rename(&source, &target).await?;
    }
    relative(root, &target).await
}

/// Moves the file or folder at `path` out of the way, into the project's
/// own `.wisp/trash`, and returns where it went, relative to the project.
/// Nothing is erased: the user empties that folder themselves.
pub async fn trash(root: &Path, path: &str, now: u64) -> Result<String> {
    let source = entry(root, path).await?;
    ensure!(
        tokio::fs::symlink_metadata(&source).await.is_ok(),
        "That file is no longer there"
    );
    let root = tokio::fs::canonicalize(root).await?;
    let name = source.file_name().unwrap_or_default().to_owned();
    // One folder per deletion, so two files of one name never collide.
    let mut slot = 0;
    let folder = loop {
        let folder = root.join(TRASH).join(format!("{now}-{slot}"));
        if tokio::fs::symlink_metadata(&folder).await.is_err() {
            break folder;
        }
        slot += 1;
    };
    tokio::fs::create_dir_all(&folder).await?;
    let target = folder.join(name);
    tokio::fs::rename(&source, &target).await?;
    Ok(target
        .strip_prefix(&root)?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

/// A file a message may carry: project-relative, inside the project, and a
/// regular file. The same confinement as the file browser.
pub async fn attachable(root: &Path, path: &str) -> Result<()> {
    let resolved = resolve(root, path)
        .await
        .map_err(|_| anyhow::anyhow!("Attached file '{path}' is not in this project"))?;
    ensure!(
        tokio::fs::metadata(&resolved).await?.is_file(),
        "Attached file '{path}' is not a regular file"
    );
    Ok(())
}

/// Copies files chosen outside the project into its `uploads` folder and
/// returns their project-relative paths. A file that is already inside the
/// project is referenced where it is. Existing uploads are never overwritten.
pub async fn import(root: &Path, sources: &[PathBuf]) -> Result<Vec<String>> {
    const LIMIT: u64 = 100 * 1024 * 1024;
    let root = tokio::fs::canonicalize(root).await?;
    let mut imported = Vec::new();
    for source in sources {
        let source = tokio::fs::canonicalize(source).await?;
        let metadata = tokio::fs::metadata(&source).await?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("File names must be UTF-8"))?
            .to_owned();
        ensure!(metadata.is_file(), "'{name}' is not a regular file");
        ensure!(
            metadata.len() <= LIMIT,
            "'{name}' is larger than 100 MB; reference it from the project folder instead"
        );
        if let Ok(inside) = source.strip_prefix(&root) {
            let relative = inside
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if attachable(&root, &relative).await.is_ok() {
                imported.push(relative);
                continue;
            }
        }
        let uploads = root.join("uploads");
        tokio::fs::create_dir_all(&uploads).await?;
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
            _ => (name.as_str(), String::new()),
        };
        let mut target = name.clone();
        let mut copy = 1;
        while tokio::fs::try_exists(uploads.join(&target)).await? {
            copy += 1;
            target = format!("{stem}-{copy}{extension}");
        }
        tokio::fs::copy(&source, uploads.join(&target)).await?;
        imported.push(format!("uploads/{target}"));
    }
    Ok(imported)
}

/// Keeps pasted content as `uploads/<name>`, numbered when that name is
/// taken, and gives its path for a message to carry.
pub async fn import_bytes(root: &Path, name: &str, bytes: &[u8]) -> Result<String> {
    const LIMIT: usize = 100 * 1024 * 1024;
    ensure!(!bytes.is_empty(), "There is nothing to keep");
    ensure!(
        bytes.len() <= LIMIT,
        "What was pasted is larger than 100 MB; save it in the project folder instead"
    );
    ensure!(
        !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.'),
        "'{name}' is not a file name"
    );
    let uploads = root.join("uploads");
    tokio::fs::create_dir_all(&uploads).await?;
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let mut target = name.to_owned();
    let mut copy = 1;
    while tokio::fs::try_exists(uploads.join(&target)).await? {
        copy += 1;
        target = format!("{stem}-{copy}{extension}");
    }
    tokio::fs::write(uploads.join(&target), bytes).await?;
    Ok(format!("uploads/{target}"))
}

/// Folders that hold tooling rather than research files. They stay
/// browsable; a name search just does not descend into them.
fn searchable(name: &str) -> bool {
    !name.starts_with('.') && !matches!(name, "node_modules" | "__pycache__")
}

/// Project-relative paths of files whose name contains `query`, ignoring
/// case. The walk is breadth-first so files near the root come first, never
/// follows links, and stops after `budget` entries so a huge tree cannot
/// stall the caller. Names that start with the query lead.
pub async fn search(root: &Path, query: &str, limit: usize, budget: usize) -> Result<Vec<String>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let root = tokio::fs::canonicalize(root).await?;
    tokio::task::spawn_blocking(move || {
        let mut found: Vec<(bool, String)> = Vec::new();
        let mut queue = std::collections::VecDeque::from([(root.clone(), String::new())]);
        let mut visited = 0usize;
        'walk: while let Some((directory, prefix)) = queue.pop_front() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                visited += 1;
                if visited > budget {
                    break 'walk;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                // `file_type` does not follow links, so links are skipped.
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                if kind.is_dir() {
                    if searchable(&name) {
                        queue.push_back((entry.path(), path));
                    }
                } else if kind.is_file() {
                    let lower = name.to_lowercase();
                    if lower.contains(&needle) {
                        found.push((!lower.starts_with(&needle), path));
                    }
                }
            }
            if found.len() >= limit * 4 {
                break;
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.truncate(limit);
        found.into_iter().map(|(_, path)| path).collect()
    })
    .await
    .map_err(Into::into)
}

pub async fn read(
    project: &str,
    root: &str,
    command: &str,
    args: dto::Request,
) -> Result<serde_json::Value> {
    ensure!(
        args.context_id.as_deref().is_none_or(|c| c == "local") && args.paths.is_empty(),
        "Only the local project workspace is available"
    );
    let path = args.path.as_deref().unwrap_or("");
    let resolved = resolve(Path::new(root), path).await?;
    if command.ends_with("_directory") {
        let mut entries = Vec::new();
        let mut directory = tokio::fs::read_dir(&resolved).await?;
        while let Some(entry) = directory.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches!(name.as_str(), ".wisp" | ".git") {
                continue;
            }
            // Do not expose link targets or special files through the file browser.
            let kind = entry.file_type().await?;
            if !kind.is_file() && !kind.is_dir() {
                continue;
            }
            ensure!(
                entries.len() < 2000,
                "Directory is too large; open a smaller subdirectory"
            );
            let metadata = entry.metadata().await?;
            // When it last changed, so a client can list the newest first.
            let modified_unix_millis = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_millis() as u64);
            entries.push(DirEntry {
                name,
                is_dir: kind.is_dir(),
                size: metadata.len(),
                modified_unix_millis,
            });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        return Ok(serde_json::to_value(dto::Directory {
            schema: dto::SCHEMA.into(),
            project_id: project.into(),
            session_id: args.session_id,
            context_id: "local".into(),
            path: path.into(),
            entries,
        })?);
    }
    ensure!(
        tokio::fs::metadata(&resolved).await?.is_file(),
        "Select a regular file"
    );
    let file = tokio::fs::File::open(resolved).await?;
    let metadata = file.metadata().await?;
    ensure!(metadata.is_file(), "Select a regular file");
    let mut bytes = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut bytes).await?;
    let text = (!bytes.contains(&0)).then(|| String::from_utf8_lossy(&bytes).into_owned());
    Ok(serde_json::to_value(dto::Preview {
        schema: dto::SCHEMA.into(),
        project_id: project.into(),
        session_id: args.session_id,
        context_id: "local".into(),
        requested_path: path.into(),
        content: FileContent {
            path: path.into(),
            mime: "text/plain".into(),
            text,
            base64: None,
            truncated: metadata.len() > bytes.len() as u64,
            total_bytes: Some(metadata.len()),
        },
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_listing_says_when_each_entry_last_changed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("counts.csv"), "a,b\n").unwrap();
        std::fs::create_dir(tmp.path().join("figs")).unwrap();
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let request = serde_json::from_value(serde_json::json!({"path": ""})).unwrap();
        let listed = read(
            "lab",
            &tmp.path().to_string_lossy(),
            "native_project_file_directory",
            request,
        )
        .await
        .unwrap();
        let listed: dto::Directory = serde_json::from_value(listed).unwrap();
        assert_eq!(listed.entries.len(), 2);
        for entry in &listed.entries {
            let modified = entry.modified_unix_millis.expect("a time");
            // Written a moment ago, give or take the file system's clock.
            assert!(modified.abs_diff(before) < 60_000, "{}", entry.name);
        }
    }

    #[tokio::test]
    async fn file_browser_confines_paths_and_hides_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("answer.txt"), "result")
            .await
            .unwrap();
        assert!(resolve(tmp.path(), "answer.txt").await.is_ok());
        for path in [
            "../outside",
            "/etc/passwd",
            "..\\outside",
            ".wisp/wisp.sqlite",
        ] {
            assert!(resolve(tmp.path(), path).await.is_err(), "{path}");
        }
        #[cfg(unix)]
        {
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();
            assert!(resolve(tmp.path(), "link").await.is_err());
        }
    }

    #[tokio::test]
    async fn name_search_is_bounded_ranked_and_skips_tooling_and_links() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in ["data/raw", ".git", ".venv/lib", "node_modules/pkg", "notes"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "data/raw/Counts_matrix.csv",
            "data/summary-counts.tsv",
            "counts.md",
            "notes/plan.md",
            ".git/counts",
            ".venv/lib/counts.py",
            "node_modules/pkg/counts.js",
        ] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("counts-secret.txt"), "x").unwrap();
            std::os::unix::fs::symlink(outside.path(), root.join("linked")).unwrap();
        }
        // Prefix matches lead; within a rank, files nearer the root come first.
        assert_eq!(
            search(root, " COUNTS ", 10, 10_000).await.unwrap(),
            [
                "counts.md",
                "data/raw/Counts_matrix.csv",
                "data/summary-counts.tsv"
            ]
        );
        assert_eq!(
            search(root, "counts", 1, 10_000).await.unwrap(),
            ["counts.md"]
        );
        assert!(search(root, "", 10, 10_000).await.unwrap().is_empty());
        assert!(search(root, "absent", 10, 10_000).await.unwrap().is_empty());
        // A tiny budget ends the walk early instead of scanning everything.
        assert!(search(root, "matrix", 10, 3).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn entries_are_made_renamed_and_trashed_inside_the_project_only() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        assert_eq!(create(root, "", "data", true).await.unwrap(), "data");
        assert_eq!(
            create(root, "data", " notes.md ", false).await.unwrap(),
            "data/notes.md"
        );
        assert_eq!(std::fs::read(root.join("data/notes.md")).unwrap(), b"");
        // Nothing that exists is replaced, and a name is one plain name.
        std::fs::write(root.join("data/notes.md"), "kept").unwrap();
        assert!(create(root, "data", "notes.md", false).await.is_err());
        assert!(create(root, "", "data", true).await.is_err());
        for name in ["", "  ", "a/b", "a\\b", "..", ".wisp", ".git", "two\nlines"] {
            assert!(create(root, "", name, true).await.is_err(), "{name:?}");
        }
        // Only inside the project, in a folder that is there, and never in
        // the project's own records.
        for parent in ["..", "/tmp", ".wisp", "missing", "data/notes.md"] {
            assert!(create(root, parent, "x", true).await.is_err(), "{parent}");
        }
        assert_eq!(std::fs::read(root.join("data/notes.md")).unwrap(), b"kept");

        // Renaming keeps the content and the folder, and replaces nothing.
        assert_eq!(
            rename(root, "data/notes.md", "plan.md").await.unwrap(),
            "data/plan.md"
        );
        assert!(!root.join("data/notes.md").exists());
        assert_eq!(std::fs::read(root.join("data/plan.md")).unwrap(), b"kept");
        create(root, "data", "other.md", false).await.unwrap();
        assert!(rename(root, "data/plan.md", "other.md").await.is_err());
        assert_eq!(std::fs::read(root.join("data/plan.md")).unwrap(), b"kept");
        // The same name again changes nothing.
        assert_eq!(
            rename(root, "data/plan.md", "plan.md").await.unwrap(),
            "data/plan.md"
        );
        for (path, name) in [
            ("data/missing.md", "x"),
            ("../outside", "x"),
            (".wisp", "x"),
            ("data/plan.md", "a/b"),
            ("", "x"),
        ] {
            assert!(rename(root, path, name).await.is_err(), "{path} -> {name}");
        }

        // Deleting moves the entry into the project's trash; nothing is erased.
        assert_eq!(
            trash(root, "data/plan.md", 1700).await.unwrap(),
            ".wisp/trash/1700-0/plan.md"
        );
        assert!(!root.join("data/plan.md").exists());
        assert_eq!(
            std::fs::read(root.join(".wisp/trash/1700-0/plan.md")).unwrap(),
            b"kept"
        );
        // The same name in the same second gets a place of its own.
        create(root, "data", "plan.md", false).await.unwrap();
        assert_eq!(
            trash(root, "data/plan.md", 1700).await.unwrap(),
            ".wisp/trash/1700-1/plan.md"
        );
        // A folder goes whole.
        assert_eq!(
            trash(root, "data", 1701).await.unwrap(),
            ".wisp/trash/1701-0/data"
        );
        assert!(root.join(".wisp/trash/1701-0/data/other.md").exists());
        assert!(!root.join("data").exists());
        for path in ["missing", "", "..", ".wisp", ".wisp/trash"] {
            assert!(trash(root, path, 1).await.is_err(), "{path}");
        }

        // A link is what is renamed or moved, never what it points at.
        #[cfg(unix)]
        {
            let elsewhere = tempfile::tempdir().unwrap();
            let outside = elsewhere.path().join("raw.fastq");
            std::fs::write(&outside, "reads").unwrap();
            std::os::unix::fs::symlink(&outside, root.join("raw")).unwrap();
            assert_eq!(rename(root, "raw", "reads").await.unwrap(), "reads");
            assert!(trash(root, "reads", 1702).await.is_ok());
            assert!(!root.join("reads").exists());
            assert_eq!(std::fs::read(&outside).unwrap(), b"reads");
        }
    }

    #[tokio::test]
    async fn pasted_content_is_kept_in_uploads_under_a_free_name() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let first = import_bytes(root, "pasted.png", b"one").await.unwrap();
        assert_eq!(first, "uploads/pasted.png");
        let second = import_bytes(root, "pasted.png", b"two").await.unwrap();
        assert_eq!(second, "uploads/pasted-2.png");
        assert_eq!(std::fs::read(root.join(&second)).unwrap(), b"two");
        // Nothing, a path or a hidden name is not kept.
        for (name, bytes) in [
            ("empty.png", &b""[..]),
            ("../out.png", b"x"),
            ("a/b.png", b"x"),
            (".hidden", b"x"),
        ] {
            assert!(import_bytes(root, name, bytes).await.is_err(), "{name}");
        }
    }

    #[tokio::test]
    async fn imports_copy_outside_files_and_reference_inside_ones() {
        let project = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/counts.csv"), "inside").unwrap();
        let outside = elsewhere.path().join("reads.fastq");
        std::fs::write(&outside, "first").unwrap();

        let paths = import(root, &[root.join("data/counts.csv"), outside.clone()])
            .await
            .unwrap();
        assert_eq!(paths, ["data/counts.csv", "uploads/reads.fastq"]);
        assert_eq!(
            std::fs::read_to_string(root.join("uploads/reads.fastq")).unwrap(),
            "first"
        );
        // A second upload of the same name never replaces the first.
        std::fs::write(&outside, "second").unwrap();
        assert_eq!(
            import(root, &[outside.clone()]).await.unwrap(),
            ["uploads/reads-2.fastq"]
        );
        assert_eq!(
            std::fs::read_to_string(root.join("uploads/reads.fastq")).unwrap(),
            "first"
        );
        assert!(import(root, &[elsewhere.path().to_owned()]).await.is_err());
        assert!(import(root, &[elsewhere.path().join("missing")])
            .await
            .is_err());

        for path in ["data/counts.csv", "uploads/reads-2.fastq"] {
            attachable(root, path).await.unwrap();
        }
        for path in ["data", "../reads.fastq", "missing.txt", ".git/config"] {
            assert!(attachable(root, path).await.is_err(), "{path}");
        }
    }
}
