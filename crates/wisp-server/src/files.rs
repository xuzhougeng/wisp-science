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
}
