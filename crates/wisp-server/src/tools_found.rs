//! Which of the command-line tools research work leans on are installed
//! here, by looking along the search path the app was started with.

use std::path::{Path, PathBuf};

/// The tools looked for, with what each is for.
pub(crate) const TOOLS: [(&str, &str); 8] = [
    ("python3", "Python"),
    ("Rscript", "R"),
    ("uv", "Python environments"),
    ("pixi", "Project environments"),
    ("conda", "Conda environments"),
    ("git", "Version control"),
    ("ssh", "Remote hosts"),
    ("node", "JavaScript"),
];

/// The executables the local environment names, as the WebView keeps them
/// on the `local` execution context: its key, what it is, and the program
/// looked for on the search path when no path is saved.
pub(crate) const PATHS: [(&str, &str, &str); 8] = [
    ("python_executable", "Python", "python3"),
    ("rscript_executable", "Rscript", "Rscript"),
    ("uv_executable", "uv", "uv"),
    ("node_executable", "Node.js", "node"),
    ("npm_executable", "npm", "npm"),
    ("sci_executable", "SCIMaster CLI (sci)", "sci"),
    ("pixi_executable", "pixi", "pixi"),
    ("cua_driver_executable", "Computer-use driver", "cua-driver"),
];

/// The first place along `path` where `name` is found, trying each of
/// `extensions` (one empty entry on systems that have none). `exists` says
/// whether a file is there, so this is tested without a file system.
pub(crate) fn find(
    name: &str,
    path: &[PathBuf],
    extensions: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    path.iter()
        .flat_map(|folder| {
            extensions
                .iter()
                .map(move |extension| folder.join(format!("{name}{extension}")))
        })
        .find(|candidate| exists(candidate))
}

/// What this computer's own search path and extensions are.
pub(crate) fn search_path() -> (Vec<PathBuf>, Vec<String>) {
    let path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let extensions = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    (path, extensions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_is_found_in_the_first_folder_that_has_it() {
        let path = [PathBuf::from("/opt/a"), PathBuf::from("/usr/bin")];
        let plain = [String::new()];
        let here = |wanted: &'static [&'static str]| {
            move |candidate: &Path| wanted.iter().any(|w| candidate == Path::new(w))
        };
        assert_eq!(
            find("git", &path, &plain, here(&["/usr/bin/git"])),
            Some(PathBuf::from("/usr/bin/git"))
        );
        // The search path's order decides between two installs.
        assert_eq!(
            find("git", &path, &plain, here(&["/usr/bin/git", "/opt/a/git"])),
            Some(PathBuf::from("/opt/a/git"))
        );
        assert_eq!(find("pixi", &path, &plain, here(&["/usr/bin/git"])), None);
        assert_eq!(find("git", &[], &plain, here(&["/usr/bin/git"])), None);
        // Where programs carry an extension, each is tried in turn.
        let windows = [".EXE".to_string(), ".CMD".to_string()];
        assert_eq!(
            find("uv", &path, &windows, here(&["/usr/bin/uv.CMD"])),
            Some(PathBuf::from("/usr/bin/uv.CMD"))
        );
        // Every tool that is looked for says what it is for.
        assert!(TOOLS
            .iter()
            .all(|(name, what)| !name.is_empty() && !what.is_empty()));
    }
}
