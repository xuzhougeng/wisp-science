use anyhow::{ensure, Result};
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use wisp_dto::{native_files as dto, DirEntry, FileContent};

async fn resolve(root: &Path, path: &str) -> Result<PathBuf> {
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
            entries.push(DirEntry {
                name,
                is_dir: kind.is_dir(),
                size: metadata.len(),
                modified_unix_millis: None,
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
}
