//! Local executable discovery and Python environment locations.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A uv-created virtualenv that hosts the Wisp kernel worker.
pub struct PythonEnv {
    pub venv: PathBuf,
}

impl PythonEnv {
    /// The app-managed environment location without creating or modifying it.
    pub fn managed(app_data: &Path) -> Self {
        Self {
            venv: app_data.join("python").join(".venv"),
        }
    }

    /// Locate `uv` on PATH (or via `UV_PATH` env).
    pub fn find_uv() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("UV_PATH") {
            return Some(PathBuf::from(p));
        }
        find_local_program("uv")
    }

    /// Locate `node` on PATH.
    pub fn find_node() -> Option<PathBuf> {
        find_local_program("node")
    }

    /// Locate `npm` on PATH.
    pub fn find_npm() -> Option<PathBuf> {
        find_local_program("npm")
    }

    /// Locate `sci` (scimaster-cli) on PATH.
    pub fn find_sci() -> Option<PathBuf> {
        find_local_program("sci")
    }

    /// Locate `pixi` on PATH (or via `PIXI_PATH` env).
    pub fn find_pixi() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("PIXI_PATH") {
            return Some(PathBuf::from(p));
        }
        find_local_program("pixi")
    }

    /// Python interpreter inside the venv (`Scripts\python.exe` on Windows).
    pub fn python(&self) -> PathBuf {
        if cfg!(target_os = "windows") {
            self.venv.join("Scripts").join("python.exe")
        } else {
            self.venv.join("bin").join("python")
        }
    }

    /// Discover an existing interpreter without running Python or a package manager.
    /// Retain the previous app environment when it exists, then try PATH.
    pub fn find_python(app_data: &Path) -> Option<PathBuf> {
        Self::find_python_with(app_data, find_local_program)
    }

    fn find_python_with(
        app_data: &Path,
        lookup: impl Fn(&str) -> Option<PathBuf>,
    ) -> Option<PathBuf> {
        let managed = Self::managed(app_data).python();
        if managed.is_file() {
            return Some(managed);
        }
        ["python3", "python"]
            .into_iter()
            .find_map(|name| lookup(name).filter(|path| usable_python_path(path)))
    }
}

/// GUI applications often inherit a smaller PATH than a terminal. Include
/// common user installs without spawning a login shell or editing host PATH.
fn find_local_program(name: &str) -> Option<PathBuf> {
    if let Ok(paths) = which::which_all(name) {
        if let Some(path) = paths
            .into_iter()
            .find(|path| !name.starts_with("python") || usable_python_path(path))
        {
            return Some(path);
        }
    }
    let mut directories = Vec::new();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let home = PathBuf::from(home);
        directories.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".pixi/bin"),
        ]);
        for prefix in ["miniconda3", "anaconda3", "miniforge3", "mambaforge"] {
            let prefix = home.join(prefix);
            directories.push(if cfg!(windows) {
                prefix
            } else {
                prefix.join("bin")
            });
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            directories.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            directories.push(PathBuf::from(program_files).join("nodejs"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let root = PathBuf::from(local).join("Programs/Python");
            let mut installs: Vec<_> = std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect();
            installs.sort();
            directories.extend(installs.into_iter().rev());
        }
    }
    #[cfg(not(target_os = "windows"))]
    directories
        .extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/conda/bin"].map(PathBuf::from));
    directories.into_iter().find_map(|directory| {
        // which checks executability and PATHEXT without launching the program.
        which::which_in(name, Some(directory.as_os_str()), &directory)
            .ok()
            .filter(|path| !name.starts_with("python") || usable_python_path(path))
    })
}

// Windows App Execution Aliases can open the Store instead of Python. On
// macOS the system python3 stub can request Xcode installation when launched.
fn usable_python_path(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    !normalized.contains("/microsoft/windowsapps/")
        && !(cfg!(target_os = "macos") && normalized == "/usr/bin/python3")
}

/// Locate `Rscript`: PATH first, then well-known install locations, so an R
/// installed outside PATH (e.g. `D:\R-4.5.2` on Windows or a conda base env)
/// is still found (issue #651). Context-specific interpreter paths are
/// resolved by the host from persisted execution-context configuration.
pub fn find_rscript() -> Option<PathBuf> {
    if let Ok(path) = which::which("Rscript") {
        return Some(path);
    }
    rscript_common_install_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

/// Resolve the `Rscript` binary that should actually be spawned.
///
/// On Windows `<R>\bin\Rscript.exe` is an architecture shim that re-launches
/// `<R>\bin\x64\Rscript.exe` through `cmd.exe`. Launching the real binary keeps
/// the interpreter as our own direct child, so its pid, exit status, and
/// termination are all observable instead of belonging to the shim (#941).
/// Anything that is not that shim is returned unchanged.
pub fn direct_rscript(configured: &Path) -> PathBuf {
    if !cfg!(target_os = "windows") {
        return configured.to_path_buf();
    }
    configured
        .to_str()
        .and_then(|path| windows_direct_rscript(path, |candidate| candidate.is_file()))
        .map(PathBuf::from)
        .unwrap_or_else(|| configured.to_path_buf())
}

/// Insert the architecture directory into `...\bin\Rscript.exe`, or `None` when
/// no such binary exists. Splits on the separator itself rather than going
/// through `Path`, so Windows layouts stay unit-testable on any host.
fn windows_direct_rscript(configured: &str, exists: impl Fn(&Path) -> bool) -> Option<String> {
    let separator = configured.rfind(['\\', '/'])?;
    let (directory, name) = configured.split_at(separator + 1);
    let separator = &configured[separator..separator + 1];
    // `bin\x64\Rscript.exe` is already the real binary; only `bin\` is a shim,
    // so a nested `x64\x64` candidate never exists and leaves it untouched.
    ["x64", "i386"]
        .into_iter()
        .map(|arch| format!("{directory}{arch}{separator}{name}"))
        .find(|candidate| exists(Path::new(candidate)))
}

/// Environment a child needs to run an interpreter that lives inside a
/// conda-style prefix (conda, mamba, or pixi), or empty when it does not.
///
/// Only the child's `PATH` is set; the host environment is never modified. On
/// Windows an interpreter's shared libraries live in the prefix rather than
/// beside the executable, so without this a conda-forge `Rscript.exe` exits
/// immediately with `STATUS_DLL_NOT_FOUND` (`0xC0000135`), or picks up a
/// mismatched DLL from an unrelated `PATH` entry and faults (#941).
pub fn conda_prefix_envs(interpreter: &Path) -> Vec<(String, String)> {
    let Some(prefix) = conda_prefix(interpreter) else {
        return Vec::new();
    };
    let entries = prefix_path_entries(&prefix, cfg!(target_os = "windows"));
    let path = prepend_path(&entries, std::env::var_os("PATH").as_deref());
    vec![("PATH".into(), path.to_string_lossy().into_owned())]
}

/// Nearest ancestor of `interpreter` that is a conda-style environment prefix.
/// `conda-meta` is written by conda, mamba, and pixi alike, so it identifies a
/// prefix exactly instead of guessing from directory names.
fn conda_prefix(interpreter: &Path) -> Option<PathBuf> {
    interpreter
        .ancestors()
        .skip(1)
        .find(|directory| directory.join("conda-meta").is_dir())
        .map(Path::to_path_buf)
}

/// Directories a conda-style prefix contributes to `PATH`, in the order conda's
/// own activation uses them. The Windows entries are spelled out rather than
/// joined through `Path`, so the layout stays unit-testable on any host.
fn prefix_path_entries(prefix: &Path, windows: bool) -> Vec<PathBuf> {
    if !windows {
        return vec![prefix.join("bin")];
    }
    let prefix = prefix.to_string_lossy();
    std::iter::once(prefix.to_string())
        .chain(
            [
                r"Library\mingw-w64\bin",
                r"Library\usr\bin",
                r"Library\bin",
                "Scripts",
                "bin",
            ]
            .into_iter()
            .map(|entry| format!(r"{prefix}\{entry}")),
        )
        .map(PathBuf::from)
        .collect()
}

fn prepend_path(entries: &[PathBuf], current: Option<&OsStr>) -> OsString {
    let inherited = current.filter(|value| !value.is_empty());
    let mut path = OsString::new();
    for entry in entries {
        if !path.is_empty() {
            path.push(path_separator());
        }
        path.push(entry);
    }
    if let Some(inherited) = inherited {
        if !path.is_empty() {
            path.push(path_separator());
        }
        path.push(inherited);
    }
    path
}

fn path_separator() -> &'static str {
    if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    }
}

/// Candidate `Rscript` paths in well-known install locations, most preferred
/// first. Kept separate from `find_rscript` so the ordering stays testable
/// without touching the host filesystem layout.
fn rscript_common_install_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "windows")]
    {
        // C:\Program Files\R\R-<version>\bin\Rscript.exe — newest version first.
        let program_files = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let mut names: Vec<String> = std::fs::read_dir(program_files.join("R"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.path().is_dir())
                    .filter_map(|entry| Some(entry.file_name().into_string().ok()?))
                    .collect()
            })
            .unwrap_or_default();
        sort_r_install_dirs_newest_first(&mut names);
        candidates.extend(names.into_iter().map(|name| {
            program_files
                .join("R")
                .join(name)
                .join("bin")
                .join("Rscript.exe")
        }));
    }
    #[cfg(not(target_os = "windows"))]
    {
        for path in [
            "/usr/local/bin/Rscript",
            "/opt/homebrew/bin/Rscript",
            "/usr/bin/Rscript",
        ] {
            candidates.push(PathBuf::from(path));
        }
        if let Some(home) = std::env::var_os("HOME") {
            for dir in ["miniconda3", "anaconda3", "miniforge3", "mambaforge"] {
                candidates.push(Path::new(&home).join(dir).join("bin").join("Rscript"));
            }
        }
        candidates.push(PathBuf::from("/opt/conda/bin/Rscript"));
    }
    candidates
}

/// Order `R-x.y.z` install directory names newest-first. Pure string parsing
/// so it can be unit-tested on any host.
#[cfg(any(target_os = "windows", test))]
fn sort_r_install_dirs_newest_first(names: &mut [String]) {
    names.sort_by_key(|name| std::cmp::Reverse(r_install_version_key(name)));
}

#[cfg(any(target_os = "windows", test))]
fn r_install_version_key(name: &str) -> (u64, u64, u64) {
    let version = name.strip_prefix("R-").unwrap_or(name);
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Path to the kernel worker bundled with the app (`python/kernel_worker.py`).
pub fn bundled_worker_path() -> Option<PathBuf> {
    wisp_paths::kernel_worker_path()
}

/// Path to the R worker bundled with the app (`r/kernel_worker.R`).
pub fn bundled_r_worker_path() -> Option<PathBuf> {
    wisp_paths::r_kernel_worker_path()
}

/// Path to the mock MCP server bundled with the app.
pub fn bundled_mock_mcp_path() -> Option<PathBuf> {
    wisp_paths::python_dir()
        .map(|d| d.join("mock_mcp_server.py"))
        .filter(|p| p.is_file())
}

/// Resolve a script path, remapping known names to bundled resources when missing.
pub fn resolve_bundled_script(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_file() {
        return p;
    }
    match p.file_name().and_then(|n| n.to_str()) {
        Some("kernel_worker.py") => bundled_worker_path().unwrap_or(p),
        Some("kernel_worker.R") => bundled_r_worker_path().unwrap_or(p),
        Some("mock_mcp_server.py") => bundled_mock_mcp_path().unwrap_or(p),
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locating_managed_python_does_not_create_an_environment() {
        let root = std::env::temp_dir().join(format!("wisp-env-{}", uuid::Uuid::new_v4()));
        let env = PythonEnv::managed(&root);
        assert!(env.python().starts_with(&root));
        assert!(!root.exists());
    }

    #[test]
    fn python_detection_reuses_existing_environment_without_executing_it() {
        let root = std::env::temp_dir().join(format!("wisp-detect-{}", uuid::Uuid::new_v4()));
        assert!(PythonEnv::find_python_with(&root, |_| None).is_none());
        assert!(!root.exists());
        let interpreter = PythonEnv::managed(&root).python();
        std::fs::create_dir_all(interpreter.parent().unwrap()).unwrap();
        // Not an executable: discovery must not attempt to run it.
        std::fs::write(&interpreter, b"fixture, not Python").unwrap();
        assert_eq!(
            PythonEnv::find_python_with(&root, |_| panic!("existing environment wins")),
            Some(interpreter)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_python_store_aliases_and_apple_developer_stubs() {
        assert!(!usable_python_path(Path::new(
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\python.exe"
        )));
        assert!(usable_python_path(Path::new(r"C:\Python312\python.exe")));
        assert!(usable_python_path(Path::new("/opt/homebrew/bin/python3")));
    }

    #[test]
    fn r_install_dirs_sort_newest_version_first() {
        let mut names = vec![
            "R-4.3.2".to_string(),
            "R-4.10.0".to_string(),
            "R-3.6.3".to_string(),
            "R-4.5.2".to_string(),
            "unrelated".to_string(),
        ];
        sort_r_install_dirs_newest_first(&mut names);
        assert_eq!(
            names,
            ["R-4.10.0", "R-4.5.2", "R-4.3.2", "R-3.6.3", "unrelated"]
        );
    }

    /// `bin\Rscript.exe` only ever launches the architecture subdirectory, so
    /// resolving it up front keeps the interpreter as our direct child.
    #[test]
    fn windows_rscript_shim_resolves_to_the_architecture_binary() {
        let shim = r"C:\Program Files\R\R-4.5.3\bin\Rscript.exe";
        let x64 = r"C:\Program Files\R\R-4.5.3\bin\x64\Rscript.exe";
        assert_eq!(
            windows_direct_rscript(shim, |path| path == Path::new(x64)).as_deref(),
            Some(x64)
        );

        // A 32-bit-only install exposes i386 instead.
        let i386 = r"C:\Program Files\R\R-4.5.3\bin\i386\Rscript.exe";
        assert_eq!(
            windows_direct_rscript(shim, |path| path == Path::new(i386)).as_deref(),
            Some(i386)
        );

        // Pixi and conda prefixes nest R under lib\R and behave the same way.
        let pixi = r"E:\plot\.pixi\envs\default\lib\R\bin\Rscript.exe";
        assert_eq!(
            windows_direct_rscript(pixi, |_| true).as_deref(),
            Some(r"E:\plot\.pixi\envs\default\lib\R\bin\x64\Rscript.exe")
        );
    }

    #[test]
    fn windows_rscript_resolution_declines_when_there_is_nothing_to_rewrite() {
        // Already the architecture binary: no nested x64\x64 exists.
        let x64 = r"E:\plot\.pixi\envs\default\lib\R\bin\x64\Rscript.exe";
        assert_eq!(
            windows_direct_rscript(x64, |path| path == Path::new(x64)),
            None
        );

        // No architecture subdirectory at all: keep what the user configured.
        assert_eq!(
            windows_direct_rscript(r"D:\R\bin\Rscript.exe", |_| false),
            None
        );

        // A bare command has no directory to rewrite.
        assert_eq!(windows_direct_rscript("Rscript", |_| true), None);
    }

    /// A pixi env is a conda prefix, and `conda-meta` is what says so. Walking
    /// up from the interpreter is what lets a saved
    /// `.pixi/envs/default/lib/R/bin/x64/Rscript.exe` find its own DLLs.
    #[test]
    fn conda_prefix_is_found_from_a_nested_interpreter_path() {
        let root = std::env::temp_dir().join(format!("wisp-conda-{}", uuid::Uuid::new_v4()));
        let prefix = root.join(".pixi").join("envs").join("default");
        let bin = prefix.join("lib").join("R").join("bin").join("x64");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        let interpreter = bin.join("Rscript.exe");

        assert_eq!(conda_prefix(&interpreter), Some(prefix.clone()));
        // The prefix directory itself is never its own parent match.
        assert_eq!(conda_prefix(&prefix), None);

        // A plain system install is not a prefix and must be left alone.
        let system = root.join("R-4.5.3").join("bin").join("x64");
        std::fs::create_dir_all(&system).unwrap();
        assert_eq!(conda_prefix(&system.join("Rscript.exe")), None);
        assert!(conda_prefix_envs(&system.join("Rscript.exe")).is_empty());

        let envs = conda_prefix_envs(&interpreter);
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].0, "PATH");
        let expected = prefix_path_entries(&prefix, cfg!(target_os = "windows"))[0]
            .to_string_lossy()
            .into_owned();
        assert!(envs[0].1.starts_with(&expected), "{}", envs[0].1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Windows resolves an executable's shared libraries through PATH, and a
    /// conda-forge prefix keeps them under `Library`. Missing these is what
    /// makes a pixi `Rscript.exe` exit with 0xC0000135.
    #[test]
    fn windows_prefix_contributes_the_library_directories_conda_activates() {
        let prefix = Path::new(r"E:\plot\.pixi\envs\default");
        let entries = prefix_path_entries(prefix, true)
            .iter()
            .map(|entry| entry.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                r"E:\plot\.pixi\envs\default",
                r"E:\plot\.pixi\envs\default\Library\mingw-w64\bin",
                r"E:\plot\.pixi\envs\default\Library\usr\bin",
                r"E:\plot\.pixi\envs\default\Library\bin",
                r"E:\plot\.pixi\envs\default\Scripts",
                r"E:\plot\.pixi\envs\default\bin",
            ]
        );
        assert_eq!(
            prefix_path_entries(Path::new("/opt/conda/envs/research"), false),
            [PathBuf::from("/opt/conda/envs/research/bin")]
        );
    }

    /// The prefix must win over whatever the host happens to have on PATH: a
    /// mismatched DLL found first is the 0xC0000005 variant of the same bug.
    #[test]
    fn prefix_entries_are_prepended_and_never_drop_the_inherited_path() {
        let entries = [PathBuf::from("/opt/conda/bin"), PathBuf::from("/opt/extra")];
        let separator = path_separator();
        assert_eq!(
            prepend_path(&entries, Some(OsStr::new("/usr/bin"))),
            OsString::from(format!(
                "/opt/conda/bin{separator}/opt/extra{separator}/usr/bin"
            ))
        );
        assert_eq!(
            prepend_path(&entries, None),
            OsString::from(format!("/opt/conda/bin{separator}/opt/extra"))
        );
        assert_eq!(
            prepend_path(&entries, Some(OsStr::new(""))),
            OsString::from(format!("/opt/conda/bin{separator}/opt/extra"))
        );
        assert_eq!(prepend_path(&[], Some(OsStr::new("/usr/bin"))), "/usr/bin");
    }

    #[test]
    fn rscript_candidates_are_absolute_and_prefer_standard_locations() {
        let candidates = rscript_common_install_candidates();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|path| path.is_absolute()));
        #[cfg(not(target_os = "windows"))]
        assert_eq!(candidates[0], PathBuf::from("/usr/local/bin/Rscript"));
    }
}
