//! Project-local content-addressed file snapshots.
//!
//! Message resources, Run lineage, Publication Freeze, and Capsule Builder all
//! use this path so an ArtifactVersion's checksum and storage semantics mean
//! the same thing everywhere.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_SNAPSHOT_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotPolicy {
    Always,
    UpTo(u64),
    Reference,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedFile {
    pub checksum: String,
    pub size_bytes: u64,
    pub storage_path: String,
    pub materialization: wisp_store::ArtifactMaterialization,
}

pub fn capture_file(
    project_root: &Path,
    source: &Path,
    policy: SnapshotPolicy,
) -> Result<CapturedFile, String> {
    reject_project_symlinks(project_root, source)?;
    let metadata = std::fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("'{}' is not a regular file", source.display()));
    }

    let root = dunce::canonicalize(project_root).map_err(|error| error.to_string())?;
    let source = dunce::canonicalize(source).map_err(|error| error.to_string())?;
    if !source.starts_with(&root) {
        return Err(format!(
            "artifact path '{}' is outside project root",
            source.display()
        ));
    }

    let temp_dir = secure_directory(&root, &[".wisp", "artifacts", "tmp"])?;
    let mut temp_path = None;
    let should_start_copy = !matches!(policy, SnapshotPolicy::Reference);
    let mut output = if should_start_copy {
        std::fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
        let path = temp_dir.join(uuid::Uuid::new_v4().to_string());
        let file = File::create(&path).map_err(|error| error.to_string())?;
        temp_path = Some(path);
        Some(BufWriter::new(file))
    } else {
        None
    };

    let result = (|| {
        let mut input = BufReader::new(File::open(&source).map_err(|error| error.to_string())?);
        let mut digest = Sha256::new();
        let mut size_bytes = 0_u64;
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            size_bytes = size_bytes.saturating_add(read as u64);
            digest.update(&buffer[..read]);
            if matches!(policy, SnapshotPolicy::UpTo(limit) if size_bytes > limit) {
                output = None;
            }
            if let Some(output) = output.as_mut() {
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| error.to_string())?;
            }
        }
        if let Some(output) = output.as_mut() {
            output.flush().map_err(|error| error.to_string())?;
            output
                .get_ref()
                .sync_all()
                .map_err(|error| error.to_string())?;
        }

        let checksum = hex::encode(digest.finalize());
        if output.is_some() {
            let destination_dir =
                secure_directory(&root, &[".wisp", "artifacts", "sha256", &checksum[..2]])?;
            let destination = destination_dir.join(snapshot_filename(&checksum, &source));
            let temp = temp_path
                .as_ref()
                .ok_or_else(|| "snapshot temporary file is missing".to_string())?;
            if destination.exists() {
                verify_blob(&destination, &checksum, size_bytes)?;
                std::fs::remove_file(temp).map_err(|error| error.to_string())?;
            } else {
                match std::fs::rename(temp, &destination) {
                    Ok(()) => {}
                    Err(_) if destination.exists() => {
                        verify_blob(&destination, &checksum, size_bytes)?;
                        std::fs::remove_file(temp).map_err(|error| error.to_string())?;
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(CapturedFile {
                checksum,
                size_bytes,
                storage_path: relative_path(&root, &destination),
                materialization: wisp_store::ArtifactMaterialization::Snapshot,
            })
        } else {
            if let Some(temp) = temp_path.as_ref().filter(|path| path.exists()) {
                std::fs::remove_file(temp).map_err(|error| error.to_string())?;
            }
            Ok(CapturedFile {
                checksum,
                size_bytes,
                storage_path: relative_path(&root, &source),
                materialization: wisp_store::ArtifactMaterialization::Reference,
            })
        }
    })();

    if result.is_err() {
        if let Some(temp) = temp_path.filter(|path| path.exists()) {
            let _ = std::fs::remove_file(temp);
        }
    }
    result
}

fn secure_directory(root: &Path, components: &[&str]) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("artifact snapshot storage does not follow symlinks".into())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "artifact snapshot directory '{}' is not a directory",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.to_string()),
                }
                let metadata =
                    std::fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("artifact snapshot storage does not follow symlinks".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(current)
}

fn verify_blob(path: &Path, expected_checksum: &str, expected_size: u64) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("content-addressed snapshot target is not a regular file".into());
    }
    let mut input = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        digest.update(&buffer[..read]);
    }
    if size != expected_size || hex::encode(digest.finalize()) != expected_checksum {
        return Err("content-addressed snapshot checksum collision or corruption".into());
    }
    Ok(())
}

fn reject_project_symlinks(project_root: &Path, source: &Path) -> Result<(), String> {
    // macOS exposes the temporary directory through `/var` while canonical
    // input paths use `/private/var`. Compare against the physical root so a
    // path that is genuinely inside the project is not rejected.
    let logical_root = project_root;
    let project_root = logical_root
        .canonicalize()
        .unwrap_or_else(|_| logical_root.to_path_buf());
    let source = if source.is_absolute() {
        source
            .strip_prefix(logical_root)
            .map(|relative| project_root.join(relative))
            .unwrap_or_else(|_| source.to_path_buf())
    } else {
        project_root.join(source)
    };
    let relative = source
        .strip_prefix(&project_root)
        .map_err(|_| "artifact path is outside project root".to_string())?;
    let mut current = project_root;
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::Normal(part) => current.push(part),
            _ => return Err("artifact path contains an unsafe component".into()),
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("artifact snapshots do not follow symlinks".into());
        }
    }
    Ok(())
}

fn snapshot_filename(checksum: &str, source: &Path) -> String {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    format!("{checksum}{extension}")
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_snapshots_and_references_without_changing_identity() {
        let root = std::env::temp_dir().join(format!("wisp_snapshot_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("results")).unwrap();
        let source = root.join("results/table.tsv");
        std::fs::write(&source, b"a\tb\n1\t2\n").unwrap();

        let copied = capture_file(&root, &source, SnapshotPolicy::Always).unwrap();
        assert_eq!(
            copied.materialization,
            wisp_store::ArtifactMaterialization::Snapshot
        );
        assert_eq!(copied.size_bytes, 8);
        assert_eq!(
            std::fs::read(root.join(&copied.storage_path)).unwrap(),
            b"a\tb\n1\t2\n"
        );

        let referenced = capture_file(&root, &source, SnapshotPolicy::UpTo(1)).unwrap();
        assert_eq!(
            referenced.materialization,
            wisp_store::ArtifactMaterialization::Reference
        );
        assert_eq!(referenced.storage_path, "results/table.tsv");
        assert_eq!(referenced.checksum, copied.checksum);
        assert!(!root
            .join(".wisp/artifacts/tmp")
            .read_dir()
            .unwrap()
            .any(|_| true));

        std::fs::write(root.join(&copied.storage_path), b"corrupt").unwrap();
        let error = capture_file(&root, &source, SnapshotPolicy::Always).unwrap_err();
        assert!(error.contains("corruption"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_sources() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("wisp_snapshot_link_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.txt"), b"secret").unwrap();
        symlink(root.join("real.txt"), root.join("link.txt")).unwrap();

        let error =
            capture_file(&root, &root.join("link.txt"), SnapshotPolicy::Always).unwrap_err();
        assert!(error.contains("symlink"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_snapshot_storage() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("wisp_snapshot_store_link_{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("wisp_snapshot_outside_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("source.txt"), b"result").unwrap();
        symlink(&outside, root.join(".wisp")).unwrap();

        let error =
            capture_file(&root, &root.join("source.txt"), SnapshotPolicy::Always).unwrap_err();
        assert!(error.contains("does not follow symlinks"));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }
}
