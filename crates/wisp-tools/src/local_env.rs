//! Environment for code Wisp launches on this machine: the Code requests proxy
//! plus the Wisp-managed toolchain ahead of the inherited `PATH`. The host
//! process environment is never modified.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::RwLock;

static MANAGED_PATH: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Directories of tools Wisp installed itself, most preferred first. An empty
/// list leaves `PATH` to the child's inherited environment.
pub fn set_managed_path(directories: Vec<PathBuf>) {
    *MANAGED_PATH.write().unwrap() = directories;
}

/// `PATH` for a child process, or nothing when no managed tools exist.
pub fn managed_path_env() -> Vec<(String, String)> {
    let directories = MANAGED_PATH.read().unwrap().clone();
    prepend_path(&directories, std::env::var_os("PATH").as_deref())
        .map(|path| vec![("PATH".to_owned(), path)])
        .unwrap_or_default()
}

/// Everything a locally launched shell command, Run, or hook should inherit.
pub fn command_env() -> Vec<(String, String)> {
    let mut envs = crate::network::command_proxy_env();
    envs.extend(managed_path_env());
    envs
}

fn prepend_path(directories: &[PathBuf], inherited: Option<&OsStr>) -> Option<String> {
    if directories.is_empty() {
        return None;
    }
    let inherited = inherited
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .filter(|entry| !entry.as_os_str().is_empty());
    let joined = std::env::join_paths(directories.iter().cloned().chain(inherited)).ok()?;
    Some(joined.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_directories_come_first_and_keep_the_inherited_path() {
        let managed = [
            PathBuf::from("/wisp/python/.venv/bin"),
            PathBuf::from("/wisp/toolchain/bin"),
        ];
        let inherited = std::env::join_paths(["/usr/bin", "/bin"]).unwrap();
        let path = prepend_path(&managed, Some(&inherited)).unwrap();
        let entries = std::env::split_paths(&path).collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                PathBuf::from("/wisp/python/.venv/bin"),
                PathBuf::from("/wisp/toolchain/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]
        );
        assert_eq!(
            std::env::split_paths(&prepend_path(&managed, None).unwrap()).count(),
            2
        );
    }

    #[test]
    fn nothing_installed_leaves_path_alone() {
        assert_eq!(prepend_path(&[], Some(OsStr::new("/usr/bin"))), None);
    }
}
