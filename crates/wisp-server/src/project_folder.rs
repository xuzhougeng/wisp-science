//! Deleting a project's folder together with the project.
//!
//! This is the one place the host deletes a folder it did not make, for
//! good, so what it may touch is decided here and nowhere else: only the
//! folder registered for the project, never one so broad that more than a
//! project would go with it.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

/// What deleting the folder of a project would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// There is no folder any more: only the records go.
    Missing,
    /// The registered path is a link. The link goes; what it points at is
    /// someone else's and stays.
    Link(PathBuf),
    /// The folder, by its real path, with everything in it.
    Folder(PathBuf),
}

/// Folders that are never deleted with a project, nor anything above them.
pub(crate) struct Protected {
    /// The user's home folder.
    pub home: Option<PathBuf>,
    /// Where Wisp keeps its own data.
    pub data: PathBuf,
    /// The folders of the other projects, with their names.
    pub others: Vec<(String, PathBuf)>,
}

/// Decides what deleting `folder` would delete, or refuses.
pub(crate) fn target(folder: &Path, protected: &Protected) -> Result<Target> {
    ensure!(
        !folder.as_os_str().is_empty(),
        "The project has no folder on record; nothing was deleted"
    );
    let found = match std::fs::symlink_metadata(folder) {
        Ok(found) => found,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Target::Missing),
        Err(error) => bail!("Could not look at {}: {error}", folder.display()),
    };
    if found.file_type().is_symlink() {
        return Ok(Target::Link(folder.to_path_buf()));
    }
    ensure!(found.is_dir(), "{} is not a folder", folder.display());
    let real = dunce::canonicalize(folder)
        .with_context(|| format!("Could not look at {}", folder.display()))?;
    let depth = real
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .count();
    ensure!(
        depth >= 2,
        "{} is too close to the top of the disk to be deleted with a project",
        real.display()
    );
    ensure!(
        !is_mount_point(&real),
        "{} is a whole disk or volume; it is not deleted with a project",
        real.display()
    );
    // A folder that is not there cannot be inside this one.
    let within =
        |inner: &Path| dunce::canonicalize(inner).is_ok_and(|inner| inner.starts_with(&real));
    if protected.home.as_deref().is_some_and(within) {
        bail!(
            "{} holds your home folder; it is not deleted with a project",
            real.display()
        );
    }
    ensure!(
        !within(&protected.data),
        "{} holds Wisp's own data; it is not deleted with a project",
        real.display()
    );
    if let Some((name, _)) = protected.others.iter().find(|(_, other)| within(other)) {
        bail!(
            "{} also holds the project \"{name}\"; remove that project first",
            real.display()
        );
    }
    Ok(Target::Folder(real))
}

#[cfg(unix)]
fn is_mount_point(folder: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let device = |path: &Path| std::fs::metadata(path).map(|found| found.dev()).ok();
    match folder.parent() {
        Some(parent) => device(folder) != device(parent),
        None => true,
    }
}

#[cfg(not(unix))]
fn is_mount_point(folder: &Path) -> bool {
    // A drive's root has no parent; folders below it are on that drive.
    folder.parent().is_none()
}

/// How much is in a folder, for the question before it is deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Size {
    pub files: u64,
    pub bytes: u64,
    /// The count stopped early: there is at least this much.
    pub more: bool,
}

/// Counts the files under `folder` without following links, up to `limit`
/// entries, so the question is not held up by a very large folder.
pub(crate) fn measure(folder: &Path, limit: u64) -> Size {
    let mut size = Size::default();
    let mut seen = 0;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > limit {
                size.more = true;
                return size;
            }
            let Ok(found) = entry.metadata() else {
                continue;
            };
            if found.is_dir() {
                pending.push(entry.path());
            } else {
                size.files += 1;
                size.bytes += found.len();
            }
        }
    }
    size
}

/// Deletes what `target` names. Deleting a folder cannot be undone.
pub(crate) async fn delete(target: Target) -> Result<()> {
    let (path, done) = match target {
        Target::Missing => return Ok(()),
        Target::Folder(path) => {
            let done = tokio::fs::remove_dir_all(&path).await;
            (path, done)
        }
        Target::Link(path) => {
            // A link to a folder is a directory entry on Windows.
            let done = if cfg!(windows) {
                tokio::fs::remove_dir(&path).await
            } else {
                tokio::fs::remove_file(&path).await
            };
            (path, done)
        }
    };
    done.with_context(|| format!("Could not delete {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protected(temp: &Path) -> Protected {
        Protected {
            home: Some(temp.join("home")),
            data: temp.join("home/data"),
            others: vec![("Other".into(), temp.join("labs/other"))],
        }
    }

    fn refused(folder: &Path, protected: &Protected) -> String {
        target(folder, protected).unwrap_err().to_string()
    }

    #[test]
    fn only_a_projects_own_folder_may_go() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        for folder in [
            "home/data",
            "labs/other",
            "labs/mine/results",
            "home/work/p1",
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join("labs/mine/notes.txt"), "x").unwrap();
        let protected = protected(&root);

        // Its own folder, by its real path.
        assert_eq!(
            target(&root.join("labs/mine/."), &protected).unwrap(),
            Target::Folder(root.join("labs/mine"))
        );
        // A project inside the home folder is fine; the home folder is not,
        // nor anything above it.
        assert_eq!(
            target(&root.join("home/work/p1"), &protected).unwrap(),
            Target::Folder(root.join("home/work/p1"))
        );
        assert!(refused(&root.join("home"), &protected).contains("home folder"));
        assert!(refused(&root, &protected).contains("home folder"));
        // Wisp's own data, with the home folder out of the way.
        let no_home = Protected {
            home: None,
            ..self::protected(&root)
        };
        assert!(refused(&root.join("home/data"), &no_home).contains("Wisp's own data"));
        assert!(refused(&root.join("home"), &no_home).contains("Wisp's own data"));
        // A folder that also holds another project.
        assert!(refused(&root.join("labs"), &protected).contains("\"Other\""));
        // The top of a disk, however it is spelt.
        assert!(refused(Path::new("/"), &protected).contains("top of the disk"));
        // A file is not a folder, and no path is no folder.
        assert!(refused(&root.join("labs/mine/notes.txt"), &protected).contains("not a folder"));
        assert!(refused(Path::new(""), &protected).contains("no folder on record"));
        // Already gone: nothing to delete, and that is not an error.
        assert_eq!(
            target(&root.join("labs/gone"), &protected).unwrap(),
            Target::Missing
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_is_removed_and_what_it_points_at_is_kept() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        std::fs::create_dir_all(root.join("shared/data")).unwrap();
        std::fs::write(root.join("shared/data/reads.fq"), "ACGT").unwrap();
        std::os::unix::fs::symlink(root.join("shared"), root.join("link")).unwrap();
        let found = target(&root.join("link"), &protected(&root)).unwrap();
        assert_eq!(found, Target::Link(root.join("link")));
        delete(found).await.unwrap();
        assert!(!root.join("link").exists());
        assert!(root.join("shared/data/reads.fq").exists());
    }

    #[tokio::test]
    async fn a_folder_is_measured_and_then_deleted_whole() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let folder = root.join("labs/mine");
        std::fs::create_dir_all(folder.join("results/deep")).unwrap();
        std::fs::write(folder.join("a.txt"), "12345").unwrap();
        std::fs::write(folder.join("results/deep/b.txt"), "123").unwrap();
        assert_eq!(
            measure(&folder, 1_000),
            Size {
                files: 2,
                bytes: 8,
                more: false
            }
        );
        // A count that stops early says so.
        assert!(measure(&folder, 2).more);
        assert_eq!(measure(&root.join("gone"), 10), Size::default());

        let found = target(&folder, &protected(&root)).unwrap();
        delete(found).await.unwrap();
        assert!(!folder.exists());
        // Its neighbours are untouched.
        assert!(root.join("labs").exists());
        delete(Target::Missing).await.unwrap();
    }
}
